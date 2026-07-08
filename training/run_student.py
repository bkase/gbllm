"""Train the ternary MoE STUDENT via Off->Hard QAT + teacher distillation (bd-wfghf).

Loads the dense fp32 teacher checkpoint (artifacts/teacher_d512/), builds a
ternary top-1 8-expert MoE student at the PROVEN-DEPLOYABLE d192 geometry
(d_model=192, d_ff=384, n_blocks=6, state_slots=192, n_experts=8, vocab=1024),
and trains it against

    total = CE + distill_w * T^2 * softCE(teacher/T, student/T) + aux_w * router_aux

under the Off->Hard QAT schedule (gbtrain/qat_schedule.py): QAT hooks OFF for
warmup_frac of steps, then Hard (ternary weights + Int8 activation fake-quant),
cosine hard-phase LR decay to 0.1x, distill-weight ramp 0.5 -> 0.65. The hardened
deployable student (int8 ternary {-1,0,+1} + per-row Q8.8 scales + fp32
embedding/router + MT4 decay raws) is exported to artifacts/student_moe_d192x8/.

`--steps` defaults to a SMOKE value (fast, tiny footprint, safe alongside the
teacher). Pass the multi-hour value only once the teacher run is finished and
owns the GPU no longer. The exact recommended launch command is printed at the
end of every run.

    # smoke (default): quick correctness/export check
    uv run python run_student.py
    # full run (AFTER the teacher finishes): see the printed command

DEPLOYABILITY NOTE: d_model is pinned to 192 because the ROM runtime caps
d_model <= 255 (u8 lane loop) and, more bindingly, the Game Boy's 8 KiB WRAM
(the proven d192 deploy already uses 7208/8192 bytes) — activation/state/residual
buffers scale ~linearly with d_model, so ~192 is the max deployable activation
width. Capacity comes from MANY EXPERTS at a deployable width, not from widening
d_model: experts live in ROM (one active per token) so they cost no
activation-WRAM. The d512 teacher is fp and undeployed (teachers have no device
constraint); distilling a d192 student from it is the intended setup. The V=1024
tied head needs on-device logit paging at deploy (separately beaded) — a
deploy-side concern, not a training concern.
"""

from __future__ import annotations

import argparse
import time
from pathlib import Path

import mlx.core as mx

from gbtrain.data import Dataset
from gbtrain.export import export_hardened, load_hardened
from gbtrain.model import GBModel, ModelConfig
from gbtrain.qat_schedule import QATScheduleConfig
from gbtrain.tokenizer import BPEModel
from gbtrain.train import (
    TrainConfig,
    byte_len_table,
    eval_bits_per_raw_byte,
    load_checkpoint,
    train,
)

# --- student topology (proven-deployable d192 geometry, 8 experts) ---------
STUDENT = dict(
    d_model=192, d_ff=384, n_blocks=6, state_slots=192, n_experts=8, vocab=1024
)

# Recommended multi-hour full-training config (printed, not the smoke default).
FULL_STEPS = 24000
FULL_SEQ_LEN = 256
FULL_LANES = 64


def _flat(tree):
    out = []
    if isinstance(tree, dict):
        for v in tree.values():
            out += _flat(v)
    elif isinstance(tree, list):
        for v in tree:
            out += _flat(v)
    else:
        out.append(("", tree))
    return out


DEPLOY_OUT = "artifacts/student_moe_d192x8"


def full_command(args) -> str:
    # Always target the canonical deployable out dir (not a smoke scratch dir).
    return (
        "nohup uv run python run_student.py "
        f"--steps {FULL_STEPS} --seq-len {FULL_SEQ_LEN} --lanes {FULL_LANES} "
        f"--warmup-frac {args.warmup_frac} --lr-peak {args.lr_peak} "
        f"--out {DEPLOY_OUT} --teacher-dir {args.teacher_dir} "
        "--eval-every 500 --ckpt-every 1000 "
        "> artifacts/student_moe_d192x8.log 2>&1 &"
    )


def main() -> None:
    ap = argparse.ArgumentParser(description="Train the ternary MoE student (QAT + distill)")
    ap.add_argument("--steps", type=int, default=40, help="SMOKE default; set to ~20000 for the full run")
    ap.add_argument("--seq-len", type=int, default=32)
    ap.add_argument("--lanes", type=int, default=4)
    ap.add_argument("--warmup-frac", type=float, default=0.4)
    ap.add_argument("--lr-peak", type=float, default=2e-3)
    ap.add_argument("--lr-warmup-steps", type=int, default=200)
    ap.add_argument("--distill-temperature", type=float, default=2.0)
    ap.add_argument("--distill-start", type=float, default=0.5)
    ap.add_argument("--distill-end", type=float, default=0.65)
    ap.add_argument("--aux-weight", type=float, default=1.0)
    ap.add_argument("--eval-every", type=int, default=20)
    ap.add_argument("--eval-batches", type=int, default=8)
    ap.add_argument("--ckpt-every", type=int, default=10_000_000)  # smoke: only final
    ap.add_argument("--log-every", type=int, default=10)
    ap.add_argument("--dataset", default="artifacts/ds_ts_1024")
    ap.add_argument("--vocab-json", default="artifacts/tinystories_bpe_1024.json")
    ap.add_argument("--teacher-dir", default="artifacts/teacher_d512")
    ap.add_argument("--out", default="artifacts/student_moe_d192x8")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--no-distill", action="store_true", help="ablate the teacher (control)")
    args = ap.parse_args()

    mx.random.seed(args.seed)
    smoke = args.steps <= 200
    print(
        f"[student] mlx device={mx.default_device()} start={time.ctime()} "
        f"mode={'SMOKE' if smoke else 'FULL'} steps={args.steps}",
        flush=True,
    )

    ds = Dataset.load(args.dataset)
    bpe = BPEModel.load(args.vocab_json)
    byte_lens = byte_len_table(bpe)
    assert ds.vocab_size == STUDENT["vocab"], f"vocab mismatch {ds.vocab_size}"

    # --- teacher (frozen, fp32, QAT OFF) ---
    teacher = None
    if not args.no_distill:
        teacher = load_checkpoint(args.teacher_dir)
        tstep = "?"
        tcfg_path = Path(args.teacher_dir) / "config.json"
        if tcfg_path.exists():
            import json

            tstep = json.loads(tcfg_path.read_text()).get("step", "?")
        print(
            f"[student] loaded teacher {args.teacher_dir} (step {tstep}) "
            f"cfg={teacher.cfg.to_dict()}",
            flush=True,
        )

    # --- student (ternary MoE; QAT flags flip via the schedule) ---
    scfg = ModelConfig(**STUDENT, qat_weights=False, qat_acts=False)
    student = GBModel(scfg)
    mx.eval(student.parameters())
    nparams = sum(v.size for _, v in _flat(student.parameters()))
    print(f"[student] params={nparams:,} cfg={scfg.to_dict()}", flush=True)

    qsched = QATScheduleConfig(
        total_steps=args.steps,
        warmup_frac=args.warmup_frac,
        lr_peak=args.lr_peak,
        lr_warmup_steps=min(args.lr_warmup_steps, max(1, args.steps // 5)),
        hard_lr_floor_mult=0.1,
        distill_weight_start=args.distill_start,
        distill_weight_end=args.distill_end,
    )
    print(f"[student] qat_schedule={qsched.to_dict()}", flush=True)

    tcfg = TrainConfig(
        seq_len=args.seq_len,
        lanes=args.lanes,
        steps=args.steps,
        lr_peak=args.lr_peak,
        aux_weight=args.aux_weight,
        eval_every=args.eval_every,
        eval_batches=args.eval_batches,
        ckpt_every=args.ckpt_every,
        log_every=args.log_every,
        ckpt_dir=args.out,
        seed=args.seed,
        distill=teacher is not None,
        distill_temperature=args.distill_temperature,
    )

    t0 = time.time()
    train(student, ds.train, ds.val, byte_lens, tcfg, teacher=teacher, qat_schedule=qsched)
    dt = time.time() - t0

    # --- hardness must be ON at the end of a real run; export the deployable form ---
    student.set_qat(True, True)  # ensure hardened math for the final eval/export
    bpb, ce = eval_bits_per_raw_byte(
        student, ds.val, byte_lens, args.seq_len, args.lanes, args.eval_batches
    )
    print(f"[student] final (Hard QAT) val_ce {ce:.4f} nats  bits/raw-byte {bpb:.4f}", flush=True)

    out = export_hardened(
        student,
        args.out,
        meta={
            "steps": args.steps,
            "smoke": smoke,
            "final_bits_per_raw_byte": bpb,
            "final_val_ce_nats": ce,
            "distill": teacher is not None,
            "qat_schedule": qsched.to_dict(),
            "train_seconds": dt,
        },
    )
    print(f"[student] hardened export -> {out}  (train {dt:.1f}s)", flush=True)

    # verify the export round-trips (deploy math == student Hard QAT math)
    reloaded = load_hardened(out)
    bpb2, ce2 = eval_bits_per_raw_byte(
        reloaded, ds.val, byte_lens, args.seq_len, args.lanes, args.eval_batches
    )
    ok = abs(bpb - bpb2) < 1e-4 and abs(ce - ce2) < 1e-4
    print(
        f"[student] export round-trip {'OK' if ok else 'MISMATCH'}: "
        f"reload val_ce {ce2:.4f} bits/raw-byte {bpb2:.4f}",
        flush=True,
    )

    print("\n" + "=" * 78, flush=True)
    print("LAUNCH THE FULL STUDENT RUN (only after the teacher finishes / frees the GPU):", flush=True)
    print(full_command(args), flush=True)
    print(
        "  aux lambdas: lambda_zrouter=1e-3, lambda_balance=1e-2 (Switch/ST-MoE "
        "defaults in ModelConfig; the aux no longer swamps the CE).",
        flush=True,
    )
    print(
        "  throughput: top-1 MoE now uses window-batched SPARSE dispatch "
        f"(~1x FFN work). Measured ~6 steps/s (fp) / ~5 steps/s (hard QAT) at "
        f"lanes={FULL_LANES} seq_len={FULL_SEQ_LEN} on Device(gpu,0); teacher "
        "forward is ~27% of the step. Expected wall-clock for "
        f"{FULL_STEPS} steps: ~1.2-1.5 h (no teacher-logit caching needed).",
        flush=True,
    )
    print("=" * 78, flush=True)
    print(f"[student] done {time.ctime()}", flush=True)


if __name__ == "__main__":
    main()

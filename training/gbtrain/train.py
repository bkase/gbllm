"""Truncated-BPTT training loop (MLX/Metal, fp32) for the GB LLM teacher.

Processes the packed token stream in truncation windows of length `seq_len`
over `lanes` independent streams, unrolling the LinearState recurrence inside a
window and backprop-through-time across all T steps. At each window boundary
the carried recurrent state is DETACHED (mx.stop_gradient) and passed as the
initial state of the next window, so gradients never cross the truncation
boundary but the state VALUE persists (standard truncated-BPTT detached carry,
state_model_ref.rs:522-534).

Eval reports bits/raw-byte on val: sum of -log2 p(target) over the stream
divided by total RAW bytes (bytes/token accounted via the BPE id->bytes table),
so it is comparable to the KN-5 / charset byte-level baselines.

Export is definition-of-done: checkpoints (weights + config) are written to
disk periodically and at the end. An 8h run was once lost by not exporting.
"""

from __future__ import annotations

import json
import math
import time
from dataclasses import dataclass, field
from pathlib import Path

import mlx.core as mx
import mlx.nn as nn
import mlx.optimizers as optim
import numpy as np

from .data import iter_bptt_batches
from .model import MT4_DECAY_RAWS, GBModel, ModelConfig
from .qat_schedule import QATScheduleConfig
from .qat_schedule import distill_weight_at as sched_distill_weight_at
from .qat_schedule import lr_at as sched_lr_at
from .qat_schedule import qat_flags_at as sched_qat_flags_at
from .tokenizer import BPEModel

LN2 = math.log(2.0)


# --- byte-length table for bits/raw-byte ----------------------------------
def byte_len_table(bpe: BPEModel) -> np.ndarray:
    """[V] int32 raw-byte length of every token id (for bits/raw-byte)."""
    return np.array(
        [len(bpe.token_bytes(i)) for i in range(bpe.vocab_size)], dtype=np.int32
    )


# --- losses ----------------------------------------------------------------
def cross_entropy(logits: mx.array, targets: mx.array) -> mx.array:
    """Mean token cross-entropy in nats. logits [B,T,V], targets [B,T]."""
    lse = mx.logsumexp(logits, axis=-1)  # [B,T]
    tgt = mx.take_along_axis(logits, targets[..., None], axis=-1)[..., 0]  # [B,T]
    return mx.mean(lse - tgt)


def distill_loss(
    student_logits: mx.array, teacher_logits: mx.array, temperature: float
) -> mx.array:
    """Soft cross-entropy (KL) distillation term (stub for the student stage):
    KL(softmax(teacher/T) || softmax(student/T)) * T^2, mean over tokens."""
    t = temperature
    tp = mx.softmax(teacher_logits / t, axis=-1)
    s_logp = student_logits / t - mx.logsumexp(student_logits / t, axis=-1, keepdims=True)
    t_logp = teacher_logits / t - mx.logsumexp(teacher_logits / t, axis=-1, keepdims=True)
    return mx.mean(mx.sum(tp * (t_logp - s_logp), axis=-1)) * (t * t)


# --- config ----------------------------------------------------------------
@dataclass
class TrainConfig:
    seq_len: int = 256
    lanes: int = 32  # batch = independent streams
    steps: int = 20000
    lr_peak: float = 3e-3
    lr_min: float = 3e-4
    warmup_steps: int = 400
    weight_decay: float = 0.01
    grad_clip: float = 1.0
    aux_weight: float = 1.0  # scales the router aux (MoE only)
    ternary_calib_delta: float = 0.7  # TWN threshold fraction at the Off->Hard flip
    eval_every: int = 500
    eval_batches: int = 40  # val windows per eval
    ckpt_every: int = 2000
    log_every: int = 20
    ckpt_dir: str = "artifacts/ckpt"
    seed: int = 0
    # distillation (student stage; unused for the teacher)
    distill: bool = False
    distill_temperature: float = 2.0
    distill_weight: float = 1.0


def lr_at(step: int, cfg: TrainConfig) -> float:
    """Linear warmup to lr_peak over warmup_steps, then cosine decay to lr_min
    across the remaining steps."""
    if step < cfg.warmup_steps:
        return cfg.lr_peak * (step + 1) / cfg.warmup_steps
    prog = (step - cfg.warmup_steps) / max(1, cfg.steps - cfg.warmup_steps)
    prog = min(1.0, prog)
    cos = 0.5 * (1.0 + math.cos(math.pi * prog))
    return cfg.lr_min + (cfg.lr_peak - cfg.lr_min) * cos


# --- checkpoint (export is definition-of-done) -----------------------------
def save_checkpoint(model: GBModel, out_dir: str | Path, step: int, meta: dict | None = None):
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    model.save_weights(str(out / "weights.safetensors"))
    payload = {
        "config": model.cfg.to_dict(),
        "step": step,
        "decay_raws": list(MT4_DECAY_RAWS),
    }
    if meta:
        payload["meta"] = meta
    (out / "config.json").write_text(json.dumps(payload, indent=2))
    return out


def load_checkpoint(out_dir: str | Path) -> GBModel:
    out = Path(out_dir)
    payload = json.loads((out / "config.json").read_text())
    cfg = ModelConfig.from_dict(payload["config"])
    model = GBModel(cfg)
    model.load_weights(str(out / "weights.safetensors"))
    mx.eval(model.parameters())
    return model


# --- eval ------------------------------------------------------------------
def eval_bits_per_raw_byte(
    model: GBModel,
    tokens: np.ndarray,
    byte_lens: np.ndarray,
    seq_len: int,
    lanes: int,
    max_batches: int | None = None,
) -> tuple[float, float]:
    """Returns (bits_per_raw_byte, mean_token_ce_nats) over the stream.

    Carries detached state across windows within each lane. Deterministic (no
    dropout / jitter; teacher has none anyway)."""
    h = model.init_state(lanes)
    blt = mx.array(byte_lens)
    total_bits = mx.array(0.0)
    total_bytes = mx.array(0.0)
    total_ce = mx.array(0.0)
    total_tok = 0
    for i, (x, y) in enumerate(iter_bptt_batches(tokens, seq_len, lanes)):
        if max_batches is not None and i >= max_batches:
            break
        xb = mx.array(x)
        yb = mx.array(y)
        logits, h, _ = model(xb, h)
        h = mx.stop_gradient(h)
        lse = mx.logsumexp(logits, axis=-1)
        tgt = mx.take_along_axis(logits, yb[..., None], axis=-1)[..., 0]
        nats = lse - tgt  # [B,T] per-token -log p in nats
        bits = nats / LN2
        raw_bytes = blt[yb]  # [B,T]
        total_bits = total_bits + mx.sum(bits)
        total_bytes = total_bytes + mx.sum(raw_bytes.astype(mx.float32))
        total_ce = total_ce + mx.sum(nats)
        total_tok += xb.size
        mx.eval(h, total_bits, total_bytes, total_ce)
    bpb = float(total_bits) / max(1.0, float(total_bytes))
    ce = float(total_ce) / max(1, total_tok)
    return bpb, ce


# --- training loop ---------------------------------------------------------
def train(
    model: GBModel,
    train_tokens: np.ndarray,
    val_tokens: np.ndarray,
    byte_lens: np.ndarray,
    cfg: TrainConfig,
    teacher: GBModel | None = None,
    qat_schedule: QATScheduleConfig | None = None,
):
    """Truncated-BPTT training. If `qat_schedule` is given (student stage), the
    QAT hooks, LR, and distillation weight follow the Off->Hard schedule instead
    of `cfg`'s static LR / distill_weight; otherwise `cfg`'s cosine LR and static
    distill_weight are used (teacher stage)."""
    mx.random.seed(cfg.seed)
    opt = optim.AdamW(learning_rate=cfg.lr_peak, weight_decay=cfg.weight_decay)

    def loss_fn(model, xb, yb, h, teacher_logits, distill_weight):
        logits, new_h, aux = model(xb, h)
        loss = cross_entropy(logits, yb)
        if teacher_logits is not None:
            loss = loss + distill_weight * distill_loss(
                logits, teacher_logits, cfg.distill_temperature
            )
        loss = loss + cfg.aux_weight * aux
        return loss, mx.stop_gradient(new_h)

    loss_and_grad = nn.value_and_grad(model, loss_fn)

    h = model.init_state(cfg.lanes)
    teacher_h = teacher.init_state(cfg.lanes) if teacher is not None else None
    step = 0
    t0 = time.time()
    window_iter = iter_bptt_batches(train_tokens, cfg.seq_len, cfg.lanes)
    running = 0.0
    running_n = 0
    # When a QAT schedule drives the run, initialise the hooks to the step-0
    # phase and track the current flags so we only re-flip (and re-eval) at the
    # Off->Hard boundary, not every step.
    qat_state: tuple[bool, bool] | None = None
    if qat_schedule is not None:
        qat_state = sched_qat_flags_at(0, qat_schedule)
        model.set_qat(*qat_state)
        print(
            f"[qat] schedule on: hard_start={qat_schedule.hard_start_step} "
            f"init flags(weights,acts)={qat_state}",
            flush=True,
        )
    while step < cfg.steps:
        try:
            x, y = next(window_iter)
        except StopIteration:
            # wrap the stream, reset state at the true sequence boundary
            window_iter = iter_bptt_batches(train_tokens, cfg.seq_len, cfg.lanes)
            h = model.init_state(cfg.lanes)
            if teacher is not None:
                teacher_h = teacher.init_state(cfg.lanes)
            continue
        xb = mx.array(x)
        yb = mx.array(y)
        teacher_logits = None
        if teacher is not None:
            tl, teacher_h, _ = teacher(xb, teacher_h)
            teacher_h = mx.stop_gradient(teacher_h)
            teacher_logits = mx.stop_gradient(tl)

        if qat_schedule is not None:
            new_flags = sched_qat_flags_at(step, qat_schedule)
            if new_flags != qat_state:
                # TWN-calibrate ternary (threshold, scale) from the fp latent
                # weights at the moment the WEIGHT fake-quant turns ON, so the
                # hard phase starts magnitude-matched (avoids the activation
                # blow-up of +-1 rows against 1/sqrt(cols) latents).
                if new_flags[0] and not qat_state[0]:
                    model.calibrate_ternary_scales(cfg.ternary_calib_delta)
                model.set_qat(*new_flags)
                qat_state = new_flags
                print(
                    f"[qat] step {step}: flip flags(weights,acts)={new_flags} "
                    f"(Hard phase begins; ternary TWN-calibrated)",
                    flush=True,
                )
            opt.learning_rate = sched_lr_at(step, qat_schedule)
            distill_weight = sched_distill_weight_at(step, qat_schedule)
        else:
            opt.learning_rate = lr_at(step, cfg)
            distill_weight = cfg.distill_weight
        (loss, new_h), grads = loss_and_grad(
            model, xb, yb, h, teacher_logits, distill_weight
        )
        if cfg.grad_clip is not None:
            grads, _ = optim.clip_grad_norm(grads, cfg.grad_clip)
        opt.update(model, grads)
        h = new_h  # already detached in loss_fn
        mx.eval(model.parameters(), opt.state, loss, h)

        running += float(loss)
        running_n += 1
        step += 1

        if step % cfg.log_every == 0:
            dt = time.time() - t0
            sps = step / dt
            print(
                f"step {step:6d}  loss {running / running_n:.4f}  "
                f"lr {opt.learning_rate.item():.2e}  {sps:.2f} steps/s",
                flush=True,
            )
            running = 0.0
            running_n = 0

        if step % cfg.eval_every == 0 or step == cfg.steps:
            bpb, ce = eval_bits_per_raw_byte(
                model, val_tokens, byte_lens, cfg.seq_len, cfg.lanes, cfg.eval_batches
            )
            print(
                f"[eval] step {step:6d}  val_ce {ce:.4f} nats  "
                f"bits/raw-byte {bpb:.4f}",
                flush=True,
            )

        if step % cfg.ckpt_every == 0 or step == cfg.steps:
            save_checkpoint(model, cfg.ckpt_dir, step, meta={"lr": opt.learning_rate.item()})
            print(f"[ckpt] wrote {cfg.ckpt_dir} @ step {step}", flush=True)

    return model

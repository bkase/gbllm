"""Student-stage tests: Off->Hard QAT schedule, hardened ternary export,
distillation, and MoE load-balance anti-collapse. Fast/tiny by construction."""

from __future__ import annotations

import tempfile

import mlx.core as mx
import mlx.nn as nn
import mlx.optimizers as optim
import numpy as np
import pytest

from gbtrain import export as X
from gbtrain import qat_schedule as Q
from gbtrain import train as T
from gbtrain.model import GBModel, ModelConfig, q8_8_ste, ternarize_ste


# ---------------------------------------------------------------------------
# fixtures / helpers
# ---------------------------------------------------------------------------
def _repeat_stream(period=8, reps=600):
    pat = np.arange(1, period + 1, dtype=np.uint16)
    return np.tile(pat, reps)


def _two_mode_stream(reps=400):
    """Two interleaved token 'modes' -> the router should spread experts across
    them instead of collapsing onto one (the charset-80 collapse guard)."""
    mode_a = np.array([1, 2, 3, 4, 5, 6, 7, 8], dtype=np.uint16)
    mode_b = np.array([9, 10, 11, 12, 13, 14, 15, 8], dtype=np.uint16)
    block = np.concatenate([np.tile(mode_a, 4), np.tile(mode_b, 4)])
    return np.tile(block, reps)


def _tiny_moe_cfg(**kw):
    base = dict(d_model=32, d_ff=64, n_blocks=2, state_slots=8, n_experts=8, vocab=16)
    base.update(kw)
    return ModelConfig(**base)


def _trace_expert_counts(model: GBModel, ids: np.ndarray) -> np.ndarray:
    """Per-expert argmax token counts summed over all blocks, using the REAL
    residual stream (embedding -> state block -> each block's router)."""
    ids_mx = mx.array(ids.astype(np.int32))
    B, Tn = ids_mx.shape
    h = model.init_state(B)
    counts = np.zeros(model.cfg.n_experts, dtype=np.int64)
    for t in range(Tn):
        x = model.embedding[ids_mx[:, t]]
        x, h = model.state_block(x, h)
        h = mx.stop_gradient(h)
        for blk in model.blocks:
            hid = x @ blk.input_projection.T + blk.input_bias
            raw = hid @ blk.expert_projection.T + blk.expert_bias
            idx = np.array(mx.argmax(raw, axis=-1)).reshape(-1)
            for i in idx:
                counts[int(i)] += 1
            x, _ = blk(x)
    return counts


# ---------------------------------------------------------------------------
# QAT schedule unit tests
# ---------------------------------------------------------------------------
def test_schedule_off_then_hard_boundary():
    cfg = Q.QATScheduleConfig(total_steps=100, warmup_frac=0.4)
    assert cfg.hard_start_step == 40
    assert Q.qat_flags_at(0, cfg) == (False, False)
    assert Q.qat_flags_at(39, cfg) == (False, False)
    assert Q.qat_flags_at(40, cfg) == (True, True)  # both flip together
    assert Q.qat_flags_at(99, cfg) == (True, True)


def test_schedule_lr_two_phase_cosine_to_floor():
    cfg = Q.QATScheduleConfig(
        total_steps=100, warmup_frac=0.4, lr_peak=2e-3,
        lr_warmup_steps=10, hard_lr_floor_mult=0.1,
    )
    assert Q.lr_at(0, cfg) == pytest.approx(2e-3 * 1 / 10)
    assert Q.lr_at(9, cfg) == pytest.approx(2e-3)  # end of warmup
    assert Q.lr_at(30, cfg) == pytest.approx(2e-3)  # OFF-phase hold at peak
    assert Q.lr_at(40, cfg) == pytest.approx(2e-3)  # hard start: cosine==peak
    floor = 0.1 * 2e-3
    assert Q.lr_at(99, cfg) == pytest.approx(floor, rel=1e-2)  # decays to floor
    # monotone non-increasing across the hard phase
    hard = [Q.lr_at(s, cfg) for s in range(40, 100)]
    assert all(hard[i] >= hard[i + 1] - 1e-12 for i in range(len(hard) - 1))


def test_schedule_distill_weight_ramp():
    cfg = Q.QATScheduleConfig(
        total_steps=100, warmup_frac=0.4,
        distill_weight_start=0.5, distill_weight_end=0.65,
    )
    assert Q.distill_weight_at(0, cfg) == pytest.approx(0.5)  # OFF phase: held
    assert Q.distill_weight_at(39, cfg) == pytest.approx(0.5)
    assert Q.distill_weight_at(40, cfg) == pytest.approx(0.5)  # hard start
    assert Q.distill_weight_at(99, cfg) == pytest.approx(0.65, rel=2e-2)  # ramps to end
    assert Q.distill_weight_at(70, cfg) == pytest.approx(0.5 + 0.15 * (30 / 60), abs=1e-3)


def test_schedule_rejects_bad_warmup_frac():
    with pytest.raises(ValueError):
        Q.QATScheduleConfig(total_steps=10, warmup_frac=1.5)


# ---------------------------------------------------------------------------
# TWN calibration at the Off->Hard flip: ternary*scale must MATCH the fp latent
# magnitude (no +-1-vs-1/sqrt(cols) blow-up) and produce genuine zeros.
# ---------------------------------------------------------------------------
def test_ternary_calibration_matches_magnitude_and_sparsifies():
    mx.random.seed(4)
    cfg = _tiny_moe_cfg()
    m = GBModel(cfg)
    mx.eval(m.parameters())

    # pick a projection; its fp latent is ~1/sqrt(cols) small
    proj = m.blocks[0].experts[0].up
    latent = np.array(proj.weight)
    proj.calibrate(delta_frac=0.7)
    # effective weight under QAT: tern*scale
    proj.set_qat(True)
    eff = np.array(proj.effective_weight())

    # magnitude match: mean|eff| within 2x of mean|latent| (NOT ~sqrt(cols) off)
    r = np.mean(np.abs(eff)) / max(1e-9, np.mean(np.abs(latent)))
    assert 0.5 < r < 2.0, f"calibrated magnitude mismatch ratio={r}"
    # genuine ternary sparsity: some entries pruned to exactly 0
    assert np.count_nonzero(eff == 0.0) > 0, "TWN threshold produced no zeros"
    # per-row scale is positive and on/above the Q8.8 tick
    assert np.all(np.array(proj.scale) >= 1.0 / 256.0 - 1e-9)


# ---------------------------------------------------------------------------
# (a) a QAT-hard student trains (loss drops on a tiny config)
# ---------------------------------------------------------------------------
def test_qat_hard_student_trains_loss_drops():
    mx.random.seed(0)
    cfg = _tiny_moe_cfg()
    m = GBModel(cfg)
    toks = _repeat_stream()
    blt = np.ones(cfg.vocab, dtype=np.int32)

    # baseline CE in Hard-QAT mode BEFORE training
    m.set_qat(True, True)
    _, ce0 = T.eval_bits_per_raw_byte(m, toks, blt, 16, 2, 6)
    m.set_qat(False, False)  # let the schedule drive from step 0

    qsched = Q.QATScheduleConfig(
        total_steps=200, warmup_frac=0.3, lr_peak=3e-3,
        lr_warmup_steps=20, hard_lr_floor_mult=0.1,
    )
    tc = T.TrainConfig(
        seq_len=16, lanes=2, steps=200, aux_weight=1.0,
        eval_every=10_000, ckpt_every=10_000, log_every=10_000,
        ckpt_dir=tempfile.mkdtemp(),
    )
    m = T.train(m, toks, toks, blt, tc, qat_schedule=qsched)

    # after training, QAT flags must be Hard (schedule ended in hard phase)
    assert m.cfg.qat_weights and m.cfg.qat_acts
    _, ce1 = T.eval_bits_per_raw_byte(m, toks, blt, 16, 2, 6)
    assert ce1 < 0.6 * ce0, f"hard-QAT student did not learn: ce {ce0:.3f} -> {ce1:.3f}"


# ---------------------------------------------------------------------------
# (b) after the hardness flip, exported weights are ACTUALLY ternary + Q8.8
# ---------------------------------------------------------------------------
def test_hardened_export_is_ternary_with_q8_8_scales():
    mx.random.seed(1)
    cfg = _tiny_moe_cfg()
    m = GBModel(cfg)
    m.set_qat(True, True)
    # push a positive threshold so some weights ternarize to exactly 0 (proves
    # the {-1,0,+1} set, not just {-1,+1}).
    for _, proj in X._ternary_projections(m):
        proj.threshold = mx.ones((proj.weight.shape[0], 1)) * 0.02
        proj.scale = mx.ones((proj.weight.shape[0],)) * 1.5  # off the trivial 256 raw
    mx.eval(m.parameters())

    d = tempfile.mkdtemp()
    X.export_hardened(m, d)
    tensors = mx.load(str(f"{d}/hardened.safetensors"))
    import json

    manifest = json.loads(open(f"{d}/manifest.json").read())

    saw_zero = False
    for name, proj in X._ternary_projections(m):
        w = np.array(tensors[f"{name}.w_tern"])
        raw = np.array(tensors[f"{name}.scale_raw"])
        assert set(np.unique(w).tolist()).issubset({-1, 0, 1}), f"{name} not ternary"
        assert tensors[f"{name}.w_tern"].dtype == mx.int8
        assert tensors[f"{name}.scale_raw"].dtype == mx.uint16
        saw_zero = saw_zero or (0 in np.unique(w).tolist())
        # exact dequant == the student's STE effective weight (bit-faithful)
        eff = np.array(proj.effective_weight())
        dq = w.astype(np.float32) * (raw.astype(np.float32) / 256.0)[:, None]
        assert np.array_equal(eff, dq), f"{name} dequant != STE effective weight"
        # Q8.8 grid: scale_raw == round(scale*256)
        exp_raw = np.clip(np.round(np.array(proj.scale) * 256.0), 0, 65535)
        assert np.array_equal(raw.astype(np.float64), exp_raw)
    assert saw_zero, "threshold>0 should have produced some ternary zeros"

    # manifest lists every per-expert ternary projection + scale
    names = {p["name"] for p in manifest["ternary_projections"]}
    for bi in range(cfg.n_blocks):
        for ei in range(cfg.n_experts):
            assert f"blocks.{bi}.experts.{ei}.up" in names
            assert f"blocks.{bi}.experts.{ei}.down" in names
    assert "state_block.state_in" in names and "state_block.state_out" in names
    for p in manifest["ternary_projections"]:
        assert p["scale_grid"] == "Q8.8 (scale_raw/256)"
        assert set(p["w_tern_values"]).issubset({-1, 0, 1})


# ---------------------------------------------------------------------------
# (c) distillation decreases student<->teacher KL vs no-distill
# ---------------------------------------------------------------------------
def _train_on_fixed_batch(student, teacher, ids, tgt, steps, distill_w, temp=2.0):
    opt = optim.AdamW(learning_rate=3e-3)
    h0 = student.init_state(ids.shape[0])
    th = teacher.init_state(ids.shape[0])
    tl, _, _ = teacher(ids, th)
    teacher_logits = mx.stop_gradient(tl)

    def loss_fn(model):
        logits, _, aux = model(ids, h0)
        loss = T.cross_entropy(logits, tgt)
        if distill_w > 0:
            loss = loss + distill_w * T.distill_loss(logits, teacher_logits, temp)
        return loss + aux

    lg = nn.value_and_grad(student, loss_fn)
    for _ in range(steps):
        loss, grads = lg(student)
        opt.update(student, grads)
        mx.eval(student.parameters(), opt.state, loss)
    return student, teacher_logits


def test_distillation_reduces_student_teacher_kl():
    mx.random.seed(7)
    tcfg = _tiny_moe_cfg(vocab=16)
    teacher = GBModel(tcfg)  # frozen random 'teacher'
    mx.eval(teacher.parameters())

    ids = mx.array(np.tile(np.arange(1, 9), (2, 1)).astype(np.int32))  # [2,8]
    tgt = mx.array(np.tile(np.arange(2, 10), (2, 1)).astype(np.int32))

    # two students from an identical init
    mx.random.seed(11)
    s_distill = GBModel(_tiny_moe_cfg(vocab=16))
    s_distill.set_qat(True, True)
    mx.eval(s_distill.parameters())
    mx.random.seed(11)
    s_plain = GBModel(_tiny_moe_cfg(vocab=16))
    s_plain.set_qat(True, True)
    mx.eval(s_plain.parameters())

    s_distill, tlog = _train_on_fixed_batch(s_distill, teacher, ids, tgt, steps=120, distill_w=1.0)
    s_plain, _ = _train_on_fixed_batch(s_plain, teacher, ids, tgt, steps=120, distill_w=0.0)

    def kl_to_teacher(student):
        h = student.init_state(ids.shape[0])
        sl, _, _ = student(ids, h)
        return float(T.distill_loss(sl, tlog, temperature=2.0))

    kl_distill = kl_to_teacher(s_distill)
    kl_plain = kl_to_teacher(s_plain)
    assert kl_distill < kl_plain, f"distill did not reduce KL: {kl_distill} !< {kl_plain}"


# ---------------------------------------------------------------------------
# (d) MoE load-balance keeps expert utilization spread (anti-collapse)
# ---------------------------------------------------------------------------
def test_moe_load_balance_no_collapse_all_experts_used():
    mx.random.seed(3)
    cfg = _tiny_moe_cfg(n_experts=8)
    m = GBModel(cfg)
    toks = _two_mode_stream()
    blt = np.ones(cfg.vocab, dtype=np.int32)

    qsched = Q.QATScheduleConfig(
        total_steps=250, warmup_frac=0.3, lr_peak=3e-3, lr_warmup_steps=20,
    )
    # The default lambda_balance was corrected 1.0 -> 1e-2 (Switch/ST-MoE scale;
    # the old 1.0 made the aux ~1000x too strong and dominated the CE). At the
    # corrected weight, aux_weight is the test's collapse-pressure knob: the
    # EFFECTIVE balance weight here is aux_weight * lambda_balance = 50 * 1e-2 =
    # 0.5. This is deliberately cranked well above the ~5e-2 real-training level
    # to force full expert recruitment on this toy 2-mode task in only 250
    # steps. It remains a REAL guard: with aux OFF (aux_weight=0) this same
    # setup collapses to 6/8 experts (two die), so the assertion below fails
    # without the load-balance mechanism actually working.
    tc = T.TrainConfig(
        seq_len=16, lanes=4, steps=250,
        aux_weight=50.0,  # * default lambda_balance 1e-2 -> effective 0.5
        eval_every=10_000, ckpt_every=10_000, log_every=10_000,
        ckpt_dir=tempfile.mkdtemp(),
    )
    m = T.train(m, toks, toks, blt, tc, qat_schedule=qsched)

    probe = _two_mode_stream(reps=6).reshape(4, -1)[:, :64]
    counts = _trace_expert_counts(m, probe)
    used = int(np.count_nonzero(counts))
    total = int(counts.sum())
    max_frac = counts.max() / max(1, total)
    assert used == cfg.n_experts, f"expert collapse: only {used}/8 used, counts={counts.tolist()}"
    assert max_frac < 0.9, f"one expert dominates ({max_frac:.2f}), counts={counts.tolist()}"


# ---------------------------------------------------------------------------
# (e) hardened export round-trips (save->reload->same eval) + manifest
# ---------------------------------------------------------------------------
def test_hardened_export_roundtrip_same_eval():
    mx.random.seed(5)
    cfg = _tiny_moe_cfg()
    m = GBModel(cfg)
    m.set_qat(True, True)
    # give scales/threshold non-trivial values so the round-trip is meaningful
    for _, proj in X._ternary_projections(m):
        proj.threshold = mx.ones((proj.weight.shape[0], 1)) * 0.015
        proj.scale = mx.ones((proj.weight.shape[0],)) * 1.25
    mx.eval(m.parameters())

    toks = _repeat_stream(reps=120)
    blt = np.ones(cfg.vocab, dtype=np.int32) * 2

    b0, c0 = T.eval_bits_per_raw_byte(m, toks, blt, 16, 2, 6)
    d = tempfile.mkdtemp()
    X.export_hardened(m, d, meta={"smoke": True})
    reloaded = X.load_hardened(d)
    b1, c1 = T.eval_bits_per_raw_byte(reloaded, toks, blt, 16, 2, 6)
    assert b0 == b1 and c0 == c1, f"round-trip drift: {(b0, c0)} != {(b1, c1)}"

    # reloaded config matches topology; acts stay quantized, weights pre-hardened
    assert reloaded.cfg.n_experts == cfg.n_experts
    assert reloaded.cfg.qat_acts and not reloaded.cfg.qat_weights

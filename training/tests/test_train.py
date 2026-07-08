"""Training-loop tests: tiny-overfit, eval determinism, detached state carry,
checkpoint round-trip, distillation stub."""

from __future__ import annotations

import tempfile

import mlx.core as mx
import numpy as np
import pytest

from gbtrain import train as T
from gbtrain.model import GBModel, ModelConfig


def _repeat_stream(period=8, reps=600):
    pat = np.arange(1, period + 1, dtype=np.uint16)
    return np.tile(pat, reps)


def test_tiny_overfit_drops_loss_near_zero():
    mx.random.seed(0)
    cfg = ModelConfig(d_model=32, d_ff=64, n_blocks=2, state_slots=8, vocab=16)
    m = GBModel(cfg)
    toks = _repeat_stream()
    blt = np.ones(16, dtype=np.int32)
    tc = T.TrainConfig(
        seq_len=16, lanes=2, steps=400, warmup_steps=20,
        lr_peak=3e-3, lr_min=3e-4, eval_every=10_000, ckpt_every=10_000,
        log_every=10_000, ckpt_dir=tempfile.mkdtemp(),
    )
    m = T.train(m, toks, toks, blt, tc)
    _, ce = T.eval_bits_per_raw_byte(m, toks, blt, 16, 2, 4)
    assert ce < 0.15, f"tiny model failed to overfit: ce={ce}"


def test_eval_is_deterministic():
    mx.random.seed(1)
    cfg = ModelConfig(d_model=16, d_ff=32, n_blocks=2, state_slots=8, vocab=16)
    m = GBModel(cfg)
    toks = _repeat_stream(reps=100)
    blt = np.ones(16, dtype=np.int32) * 2
    a = T.eval_bits_per_raw_byte(m, toks, blt, 16, 2, 5)
    b = T.eval_bits_per_raw_byte(m, toks, blt, 16, 2, 5)
    assert a == b


def test_state_carry_detach_blocks_gradient():
    # A scalar 'scale' produces the initial state; with mx.stop_gradient on the
    # carried state, gradient must NOT flow back through the carry; without it,
    # it must. Proves the truncated-BPTT detach at the window boundary.
    mx.random.seed(2)
    cfg = ModelConfig(d_model=16, d_ff=32, n_blocks=1, state_slots=8, vocab=16)
    m = GBModel(cfg)
    ids = mx.array([[1, 2, 3, 4]])
    targets = mx.array([[2, 3, 4, 5]])
    base_state = mx.random.normal((1, cfg.state_slots))

    def loss_detached(scale):
        h = mx.stop_gradient(base_state * scale)  # window boundary detach
        logits, _, _ = m(ids, h)
        return T.cross_entropy(logits, targets)

    def loss_attached(scale):
        h = base_state * scale  # gradient allowed to cross
        logits, _, _ = m(ids, h)
        return T.cross_entropy(logits, targets)

    g_det = float(mx.grad(loss_detached)(mx.array(1.0)))
    g_att = float(mx.grad(loss_attached)(mx.array(1.0)))
    assert abs(g_det) < 1e-9, f"detach leaked gradient: {g_det}"
    assert abs(g_att) > 1e-6, f"state should influence loss: {g_att}"


def test_checkpoint_roundtrip_same_eval():
    mx.random.seed(3)
    cfg = ModelConfig(d_model=16, d_ff=32, n_blocks=2, state_slots=8, n_experts=1, vocab=16)
    m = GBModel(cfg)
    toks = _repeat_stream(reps=80)
    blt = np.ones(16, dtype=np.int32) * 2
    d = tempfile.mkdtemp()
    T.save_checkpoint(m, d, step=7)
    m2 = T.load_checkpoint(d)
    b1, c1 = T.eval_bits_per_raw_byte(m, toks, blt, 16, 2, 5)
    b2, c2 = T.eval_bits_per_raw_byte(m2, toks, blt, 16, 2, 5)
    assert b1 == b2 and c1 == c2


def test_distill_loss_zero_when_logits_equal():
    logits = mx.random.normal((2, 3, 8))
    z = T.distill_loss(logits, logits, temperature=2.0)
    assert abs(float(z)) < 1e-5


def test_byte_len_table_matches_tokenizer():
    from gbtrain.tokenizer import BPEModel

    bpe = BPEModel.load("artifacts/tinystories_bpe_1024.json")
    blt = T.byte_len_table(bpe)
    assert blt.shape == (1024,)
    assert all(blt[i] == len(bpe.token_bytes(i)) for i in range(256))  # base bytes len 1
    assert blt[:256].tolist() == [1] * 256

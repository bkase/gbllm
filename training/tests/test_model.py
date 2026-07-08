"""Model forward-shape, dtype-discipline, and QAT-hook wiring tests."""

from __future__ import annotations

import mlx.core as mx
import numpy as np
import pytest

from gbtrain.model import (
    GBModel,
    LinearStateBlock,
    ModelConfig,
    act_fake_quant,
    rms_norm_clip,
)


def _tiny(**kw) -> ModelConfig:
    base = dict(d_model=16, d_ff=32, n_blocks=2, state_slots=8, n_experts=1, vocab=32)
    base.update(kw)
    return ModelConfig(**base)


def test_dense_forward_shapes_and_dtype():
    mx.random.seed(0)
    m = GBModel(_tiny())
    ids = mx.array([[1, 2, 3, 4], [5, 6, 7, 0]])
    logits, h, aux = m(ids, m.init_state(2))
    mx.eval(logits, h, aux)
    assert logits.shape == (2, 4, 32)
    assert h.shape == (2, 8)
    assert logits.dtype == mx.float32 and h.dtype == mx.float32
    assert float(aux) == 0.0  # dense has no router aux


def test_moe_forward_shapes_and_aux_finite():
    mx.random.seed(0)
    m = GBModel(_tiny(n_experts=4))
    ids = mx.array([[1, 2, 3, 4], [5, 6, 7, 0]])
    logits, h, aux = m(ids, m.init_state(2))
    mx.eval(logits, aux)
    assert logits.shape == (2, 4, 32)
    assert logits.dtype == mx.float32
    assert np.isfinite(float(aux)) and float(aux) > 0.0  # z_loss + balance


def test_router_rank_default():
    assert ModelConfig(n_experts=1).resolved_rank() == 1
    assert ModelConfig(n_experts=4).resolved_rank() == 1
    assert ModelConfig(n_experts=8).resolved_rank() == 2  # clamp(ceil(8/4),1,8)


def test_qat_toggle_changes_output_but_keeps_fp32():
    # Same weights, QAT on vs off must differ (fake-quant is active), and both
    # stay fp32 end-to-end.
    mx.random.seed(3)
    m_fp = GBModel(_tiny())
    m_q = GBModel(_tiny(qat_weights=True, qat_acts=True))
    # copy fp weights into the qat model so only the hooks differ
    m_q.update(m_fp.parameters())
    ids = mx.array([[1, 2, 3, 4]])
    l_fp, _, _ = m_fp(ids, m_fp.init_state(1))
    l_q, _, _ = m_q(ids, m_q.init_state(1))
    mx.eval(l_fp, l_q)
    assert l_fp.dtype == mx.float32 and l_q.dtype == mx.float32
    assert not np.allclose(np.array(l_fp), np.array(l_q))


def test_state_block_matches_manual_reference():
    # One-token LinearState step reproduced by hand (teacher, no act quant).
    mx.random.seed(1)
    cfg = _tiny()
    blk = LinearStateBlock(cfg)
    x = mx.random.normal((1, cfg.d_model))
    h = mx.zeros((1, cfg.state_slots))
    x_out, h_out = blk(x, h)
    # manual
    normed = rms_norm_clip(x)
    delta = normed @ blk.state_in.weight.T
    decay = mx.array(blk._decay_np)
    h_ref = h * decay + delta
    y = h_ref @ blk.state_out.weight.T
    x_ref = x + y
    mx.eval(x_out, h_out, x_ref, h_ref)
    assert np.allclose(np.array(x_out), np.array(x_ref), atol=1e-5)
    assert np.allclose(np.array(h_out), np.array(h_ref), atol=1e-5)


def test_head_is_tied_to_embedding():
    # final logits[v] = normed . E[v]; the head reuses E (no separate param).
    mx.random.seed(2)
    m = GBModel(_tiny())
    names = {k for k, _ in _flat(m.parameters())}
    assert not any("head" in n for n in names)  # no separate head weight
    assert any("embedding" in n for n in names)


def _flat(tree, prefix=""):
    out = []
    if isinstance(tree, dict):
        for k, v in tree.items():
            out += _flat(v, f"{prefix}{k}.")
    elif isinstance(tree, list):
        for i, v in enumerate(tree):
            out += _flat(v, f"{prefix}{i}.")
    else:
        out.append((prefix.rstrip("."), tree))
    return out

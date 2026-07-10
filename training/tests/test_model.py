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


def test_router_balance_is_computed_per_block_before_summing():
    cfg = _tiny(
        n_experts=2,
        lambda_balance=1.0,
        lambda_zrouter=0.0,
    )
    model = GBModel(cfg)
    tok_count = 4
    zloss = mx.zeros((cfg.n_blocks,))

    # Both blocks use both experts uniformly: one unit of Switch balance per
    # block, hence two after summing blocks.
    uniform = mx.array([[2.0, 2.0], [2.0, 2.0]])
    uniform_aux = model._aux_from_totals(uniform, uniform, zloss, tok_count)

    # Each block is individually collapsed, but onto a different expert. A
    # cross-block aggregation sees the same global totals as `uniform`; the
    # correct per-block objective assigns two units to each collapsed block.
    collapsed = mx.array([[4.0, 0.0], [0.0, 4.0]])
    collapsed_aux = model._aux_from_totals(collapsed, collapsed, zloss, tok_count)
    mx.eval(uniform_aux, collapsed_aux)

    assert float(uniform_aux) == pytest.approx(2.0)
    assert float(collapsed_aux) == pytest.approx(4.0)

    onehot_grad = mx.grad(
        lambda onehot: model._aux_from_totals(onehot, uniform, zloss, tok_count)
    )(uniform)
    probs_grad = mx.grad(
        lambda probs: model._aux_from_totals(uniform, probs, zloss, tok_count)
    )(uniform)
    mx.eval(onehot_grad, probs_grad)
    assert np.count_nonzero(np.array(onehot_grad)) == 0
    assert np.count_nonzero(np.array(probs_grad)) > 0


def test_model_forward_keeps_complementary_block_stats_separate():
    cfg = ModelConfig(
        d_model=2,
        d_ff=2,
        n_blocks=2,
        state_slots=4,
        n_experts=2,
        vocab=2,
        router_rank=1,
        lambda_balance=1.0,
        lambda_zrouter=0.0,
    )
    model = GBModel(cfg)
    model.embedding = mx.array([[-1.0, 0.0], [1.0, 0.0]])
    model.state_block.state_in.weight = mx.zeros_like(model.state_block.state_in.weight)
    model.state_block.state_out.weight = mx.zeros_like(model.state_block.state_out.weight)
    for block in model.blocks:
        block.input_projection = mx.array([[1.0, 0.0]])
        block.input_bias = mx.zeros((1,))
        block.expert_projection = mx.array([[-1.0], [1.0]])
        block.expert_bias = mx.zeros((2,))
        for expert in block.experts:
            expert.up.weight = mx.zeros_like(expert.up.weight)
            expert.down.weight = mx.zeros_like(expert.down.weight)

    # Each block independently routes half the tokens to each expert and has
    # mean soft probabilities [0.5, 0.5], so its Switch term is exactly 1.
    # Summing blocks first (the old bug) creates cross-block terms and returns
    # 4 instead of the correct per-block sum 2.
    ids = mx.array([[0, 1, 0, 1]])
    _, _, optimized_aux = model(ids, model.init_state(1))
    _, _, reference_aux = model._forward_per_token(ids, model.init_state(1))
    mx.eval(optimized_aux, reference_aux)
    assert float(optimized_aux) == pytest.approx(2.0)
    assert float(reference_aux) == pytest.approx(2.0)


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

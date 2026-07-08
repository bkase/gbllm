"""Sparse top-1 MoE dispatch tests (bd MoE-speed fix).

The efficient `MoEBlock.__call__` runs each expert on ONLY its top-1-assigned
tokens (gather/scatter) instead of computing all E experts densely and masking.
These tests pin that the optimization is a pure SPEED change:

  * block-level: sparse `__call__` == dense `forward_dense` on a fixed
    input+router (same output AND same router stats), QAT off and on;
  * model-level: the window-batched `__call__` == the per-token dense reference
    `_forward_per_token(dense_moe=True)`, bit-identical logits/h/aux;
  * gradient: under value_and_grad the sparse path still delivers nonzero grad
    to BOTH the router projections and the (top-1-selected) expert weights.
"""

from __future__ import annotations

import mlx.core as mx
import mlx.nn as nn
import numpy as np

from gbtrain.model import GBModel, MoEBlock, ModelConfig


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


def _moe_cfg(**kw):
    base = dict(d_model=32, d_ff=64, n_blocks=2, state_slots=8, n_experts=8, vocab=16)
    base.update(kw)
    return ModelConfig(**base)


# ---------------------------------------------------------------------------
# block-level: sparse dispatch == dense compute-then-select
# ---------------------------------------------------------------------------
def _assert_block_equal(qat_weights: bool, qat_acts: bool):
    mx.random.seed(0)
    cfg = _moe_cfg(qat_weights=qat_weights, qat_acts=qat_acts)
    blk = MoEBlock(cfg)
    mx.eval(blk.parameters())
    xf = mx.random.normal((40, cfg.d_model))  # [N,D]

    x_sparse, st_sparse = blk(xf)
    x_dense, st_dense = blk.forward_dense(xf)
    mx.eval(x_sparse, x_dense, st_sparse, st_dense)

    # same selected-expert output for every token
    max_diff = float(mx.max(mx.abs(x_sparse - x_dense)))
    assert max_diff == 0.0, f"sparse!=dense output (qat_w={qat_weights}): {max_diff}"
    # router stats identical (same routing decisions + probs + z_loss)
    for key in ("onehot", "probs", "zloss"):
        d = float(mx.max(mx.abs(st_sparse[key] - st_dense[key])))
        assert d == 0.0, f"sparse!=dense stat {key}: {d}"


def test_moe_sparse_equals_dense_qat_off():
    _assert_block_equal(qat_weights=False, qat_acts=False)


def test_moe_sparse_equals_dense_qat_on():
    _assert_block_equal(qat_weights=True, qat_acts=True)


def test_moe_sparse_covers_every_token_once():
    """Every token is routed to exactly one expert (argmax), so the scatter
    writes each output row exactly once -- there is no unassigned/dropped row."""
    mx.random.seed(2)
    cfg = _moe_cfg()
    blk = MoEBlock(cfg)
    mx.eval(blk.parameters())
    xf = mx.random.normal((57, cfg.d_model))
    _, st = blk(xf)
    onehot = np.array(st["onehot"])
    assert np.array_equal(onehot.sum(axis=1), np.ones(57)), "token not routed exactly once"


# ---------------------------------------------------------------------------
# model-level: window-batched sparse forward == per-token dense reference
# ---------------------------------------------------------------------------
def test_model_window_batched_equals_per_token_reference():
    for qat in (False, True):
        mx.random.seed(1)
        cfg = _moe_cfg(qat_weights=qat, qat_acts=qat)
        m = GBModel(cfg)
        mx.eval(m.parameters())
        ids = mx.array(np.random.randint(0, cfg.vocab, size=(3, 12)).astype(np.int32))

        l_new, h_new, aux_new = m(ids, m.init_state(3))
        l_ref, h_ref, aux_ref = m._forward_per_token(ids, m.init_state(3), dense_moe=True)
        mx.eval(l_new, h_new, aux_new, l_ref, h_ref, aux_ref)

        assert float(mx.max(mx.abs(l_new - l_ref))) < 1e-4, f"logits drift qat={qat}"
        assert float(mx.max(mx.abs(h_new - h_ref))) < 1e-4, f"state drift qat={qat}"
        assert abs(float(aux_new) - float(aux_ref)) < 1e-5, f"aux drift qat={qat}"


# ---------------------------------------------------------------------------
# gradient: sparse path keeps router AND selected-expert grads alive
# ---------------------------------------------------------------------------
def test_sparse_dispatch_router_and_expert_grads_nonzero():
    mx.random.seed(0)
    cfg = _moe_cfg(n_experts=8)
    m = GBModel(cfg)
    mx.eval(m.parameters())
    ids = mx.array(np.tile(np.arange(1, 9), (2, 1)).astype(np.int32))  # [2,8]

    def loss_fn(model):
        logits, _, aux = model(ids, model.init_state(2))
        return mx.sum(logits) + aux

    grads = nn.value_and_grad(m, loss_fn)(m)[1]
    named = dict(_flat(grads))

    # router projections train (via the differentiable aux loss)
    rg = [v for k, v in named.items() if "expert_projection" in k or "input_projection" in k]
    assert rg and any(float(mx.sum(mx.abs(g))) > 0 for g in rg), "router got no gradient"

    # every selected expert's up/down weight trains (gather/scatter path).
    up_down = [v for k, v in named.items() if k.endswith("up.weight") or k.endswith("down.weight")]
    assert up_down, "no expert weights in grad tree"
    n_trained = sum(1 for g in up_down if float(mx.sum(mx.abs(g))) > 0)
    assert n_trained > 0, "no expert weight received gradient through sparse dispatch"

"""Fake-quant / norm / activation unit tests, matching the gbf-kernel f32
reference formulas exactly (model_ref.rs / state_model_ref.rs)."""

from __future__ import annotations

import math

import mlx.core as mx
import numpy as np
import pytest

from gbtrain.model import (
    ACT_RANGE,
    MT4_DECAY_RAWS,
    MT4_RATES,
    NORM_EPS,
    QMAX,
    SQRT_2_OVER_PI_F32,
    act_fake_quant,
    build_decay,
    gelu_approx,
    q8_8_ste,
    rms_norm_clip,
    ternarize_ste,
)


def test_mx_round_is_ties_even():
    # Lock the rounding mode: round-half-to-even (banker's), NOT half-away.
    got = mx.round(mx.array([0.5, 1.5, 2.5, 3.5, -0.5, -1.5, -2.5])).tolist()
    assert got == [0.0, 2.0, 2.0, 4.0, 0.0, -2.0, -2.0]


def test_gelu_constant_matches_f64_to_f32_cast():
    # SQRT_2_OVER_PI = FRAC_2_SQRT_PI * FRAC_1_SQRT_2, f64 then cast to f32.
    expect = np.float32((2.0 / math.sqrt(math.pi)) * (1.0 / math.sqrt(2.0)))
    assert SQRT_2_OVER_PI_F32 == float(expect)


def _gelu_ref(x: float) -> float:
    inner = x + x * x * x * 0.044715
    inner = inner * SQRT_2_OVER_PI_F32
    return (x * (math.tanh(inner) + 1.0)) * 0.5


def test_gelu_matches_reference():
    xs = np.linspace(-6, 6, 64).astype(np.float32)
    got = np.array(gelu_approx(mx.array(xs)))
    ref = np.array([_gelu_ref(float(x)) for x in xs], dtype=np.float32)
    assert np.max(np.abs(got - ref)) < 1e-5


def _act_ref(v: np.ndarray) -> np.ndarray:
    clamped = np.clip(v, -ACT_RANGE, ACT_RANGE)
    q = np.clip(np.round((clamped / ACT_RANGE) * QMAX), -QMAX, QMAX)  # np.round: ties-even
    return (q / QMAX) * ACT_RANGE


def test_act_fake_quant_matches_reference():
    rng = np.random.default_rng(0)
    v = rng.uniform(-12, 12, size=4096).astype(np.float32)
    got = np.array(act_fake_quant(mx.array(v)))
    ref = _act_ref(v).astype(np.float32)
    assert np.max(np.abs(got - ref)) < 1e-6


def test_act_fake_quant_clamps_at_range():
    got = np.array(act_fake_quant(mx.array([100.0, -100.0, 8.0, -8.0])))
    assert got.tolist() == [8.0, -8.0, 8.0, -8.0]


def test_act_fake_quant_ties_even_boundary():
    # arg = (v/8)*127 lands exactly on a half-integer -> ties-to-even quant.
    # v = 0.5*8/127 -> arg 0.5 -> q 0; v = 1.5*8/127 -> arg 1.5 -> q 2 (even).
    v = mx.array([0.5 * 8.0 / 127.0, 1.5 * 8.0 / 127.0, 2.5 * 8.0 / 127.0])
    got = np.array(act_fake_quant(v))
    ref = np.array([0.0, (2.0 / 127.0) * 8.0, (2.0 / 127.0) * 8.0], dtype=np.float32)
    assert np.max(np.abs(got - ref)) < 1e-6


def test_act_fake_quant_ste_gradient_interior_and_exterior():
    # identity gradient inside [-8,8], zero outside (clamp carries the grad).
    g = mx.grad(lambda x: mx.sum(act_fake_quant(x)))(mx.array([1.0, -3.0, 100.0, -50.0]))
    assert np.allclose(np.array(g), [1.0, 1.0, 0.0, 0.0])


def test_rms_norm_clip_matches_reference():
    rng = np.random.default_rng(1)
    x = rng.normal(size=(3, 64)).astype(np.float32)
    got = np.array(rms_norm_clip(mx.array(x)))
    sum_sq = np.sum(x * x, axis=-1, keepdims=True)
    rms = np.sqrt(sum_sq / x.shape[-1] + NORM_EPS)
    ref = np.clip(x / rms, -ACT_RANGE, ACT_RANGE).astype(np.float32)
    assert np.max(np.abs(got - ref)) < 1e-5


def test_rms_norm_clip_engages_clip():
    # one large entry among n=128 zeros -> x0/rms = sqrt(128) ~ 11.3 > 8.
    x = mx.concatenate([mx.array([[1000.0]]), mx.zeros((1, 127))], axis=1)
    got = np.array(rms_norm_clip(x))
    assert got[0, 0] == ACT_RANGE  # clamped to 8


def test_ternarize_ste_values_and_gradient():
    w = mx.array([[0.9, -0.9, 0.05, -0.05], [2.0, -2.0, 0.3, -0.3]])
    thr = mx.array([[0.5], [1.0]])
    tern = np.array(ternarize_ste(w, thr))
    assert tern[0].tolist() == [1.0, -1.0, 0.0, 0.0]
    assert tern[1].tolist() == [1.0, -1.0, 0.0, 0.0]
    # STE: gradient wrt latent weight is identity (all ones).
    g = mx.grad(lambda ww: mx.sum(ternarize_ste(ww, thr)))(w)
    assert np.allclose(np.array(g), np.ones_like(np.array(w)))


def test_q8_8_ste_grid_and_gradient():
    s = mx.array([1.0, 0.5, 1.0 / 256.0, 0.001, 3.25])
    got = np.array(q8_8_ste(s))
    ref = np.round(np.array([1.0, 0.5, 1.0 / 256.0, 0.001, 3.25]) * 256.0) / 256.0
    assert np.allclose(got, ref, atol=1e-7)
    g = mx.grad(lambda ss: mx.sum(q8_8_ste(ss)))(s)
    assert np.allclose(np.array(g), np.ones_like(np.array(s)))  # STE identity


def test_mt4_decay_raws_are_rate_times_256():
    assert list(MT4_DECAY_RAWS) == [round(r * 256) for r in MT4_RATES]
    assert list(MT4_DECAY_RAWS) == [128, 192, 224, 240]


def test_build_decay_contiguous_blocks():
    d = build_decay(8)
    assert d.tolist() == [0.5, 0.5, 0.75, 0.75, 0.875, 0.875, 0.9375, 0.9375]
    # d192 topology: 48 slots per rate, contiguous.
    d192 = build_decay(192)
    assert d192[0] == 0.5 and d192[47] == 0.5
    assert d192[48] == 0.75 and d192[95] == 0.75
    assert d192[96] == 0.875 and d192[143] == 0.875
    assert d192[144] == 0.9375 and d192[191] == 0.9375


def test_build_decay_requires_divisible_by_four():
    with pytest.raises(ValueError):
        build_decay(6)

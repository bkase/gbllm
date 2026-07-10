"""Config-driven MLX (Apple Metal) fp32 reference model for the Game Boy LLM.

Bit-faithful to the trainer's hard-ternary f32 semantics in gbf-kernel
(`state_model_ref.rs::f32_state_forward`, `model_ref.rs::f32_forward`). The
teacher runs full-precision (QAT hooks OFF); the same module tree, with the
weight-ternary and Int8-activation fake-quant hooks toggled ON, is the QAT
student that later lowers into the Rust deploy pipeline unchanged.

Everything executes in mx.float32 end-to-end (no fp16/bf16 promotion), because
the fake-quant rounding boundaries and the RMS reciprocal-sqrt must match the
ndarray f32 reference. Train-time rounding is round-ties-even (banker's), which
MLX's `mx.round` provides on Metal (verified).

Module order per token (state_model_ref.rs:448-499):
  x = E[prev_id]                                     # residual stream, f32
  (1) LinearState MT4 block: updates recurrent state, residual-adds y
  (2) for b in 0..n_blocks: pre-norm residual FFN block (dense, or top-1 MoE)
  (3) final rms_norm_clip -> normed
  (4) tied head logits[v] = normed . E[v]            # E reused, no bias

Activation Int8 fake-quant attaches at EXACTLY four sites (QAT on):
  (1) after rms_norm_clip on the state-block input, before W_in
  (2) on the state out-projection output y, after W_out, before residual add
  (3) after rms_norm_clip on each FFN prenorm, before W_up
  (4) after gelu on the FFN hidden vector, before W_down
NOT quantized: the residual stream x, the router path, the down-projection
output delta, the final normed head input.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

import mlx.core as mx
import mlx.nn as nn
import numpy as np

# --- global constants pinned from gbf-kernel/src/model_ref.rs --------------
ACT_RANGE = 8.0
QMAX = 127.0
NORM_EPS = 1.0e-5
# SQRT_2_OVER_PI = FRAC_2_SQRT_PI * FRAC_1_SQRT_2, computed in f64 then cast to
# f32 at the multiply, exactly as gelu_approx_f32 (model_ref.rs:221-231).
_SQRT_2_OVER_PI_F64 = (2.0 / math.sqrt(math.pi)) * (1.0 / math.sqrt(2.0))
SQRT_2_OVER_PI_F32 = float(np.float32(_SQRT_2_OVER_PI_F64))
# MT4 decay rates and their Q8.8 raws {128,192,224,240} = round(rate*256).
MT4_RATES = (0.5, 0.75, 0.875, 0.9375)
MT4_DECAY_RAWS = (128, 192, 224, 240)


# --- quant / norm / activation primitives (exact spec formulas) ------------
def rms_norm_clip(x: mx.array) -> mx.array:
    """rms_norm_clip over the last axis (model_ref.rs:242-254 /
    state_model_ref.rs:419-429). Architectural (always on, both teacher and
    student): sum_sq (fp32) -> mean_sq -> rms=sqrt(mean_sq+eps) -> clamp x/rms
    to [-ACT_RANGE, ACT_RANGE]. n = last-axis width."""
    n = x.shape[-1]
    sum_sq = mx.sum(x * x, axis=-1, keepdims=True)
    mean_sq = sum_sq / n
    rms = mx.sqrt(mean_sq + NORM_EPS)
    return mx.clip(x / rms, -ACT_RANGE, ACT_RANGE)


def act_fake_quant(v: mx.array) -> mx.array:
    """Int8 signed activation fake-quant on fixed range [-8,8], round-ties-even
    (f32_act_fake_quant, model_ref.rs:260-268). Straight-through: forward
    applies the quant; backward is identity on the clamp interior and zero
    outside (the clamp carries that gradient, the round is STE'd)."""
    clamped = mx.clip(v, -ACT_RANGE, ACT_RANGE)
    q = mx.clip(mx.round((clamped / ACT_RANGE) * QMAX), -QMAX, QMAX)
    quant = (q / QMAX) * ACT_RANGE
    return clamped + mx.stop_gradient(quant - clamped)


def gelu_approx(x: mx.array) -> mx.array:
    """Tanh-approx GELU (gelu_approx_f32, model_ref.rs:227-231):
    inner = x + 0.044715*x^3; g = 0.5*x*(tanh(inner*SQRT_2_OVER_PI)+1)."""
    inner = x + x * x * x * 0.044715
    inner = inner * SQRT_2_OVER_PI_F32
    return (x * (mx.tanh(inner) + 1.0)) * 0.5


def ternarize_ste(weight: mx.array, threshold: mx.array) -> mx.array:
    """Per-row threshold ternarize to {-1,0,+1} with a straight-through
    estimator. `threshold` broadcasts against `weight` (scalar or [rows,1])."""
    tern = (weight > threshold).astype(mx.float32) - (weight < -threshold).astype(
        mx.float32
    )
    return weight + mx.stop_gradient(tern - weight)


def q8_8_ste(scale: mx.array) -> mx.array:
    """Quantize a positive per-row scale onto the Q8.8 grid (scale_raw u16 /
    256, model_ref.rs:274-275) with a straight-through estimator."""
    raw = mx.clip(mx.round(scale * 256.0), 0.0, 65535.0)
    quant = raw / 256.0
    return scale + mx.stop_gradient(quant - scale)


def build_decay(state_slots: int) -> np.ndarray:
    """MT4 per-slot decay [S], CONTIGUOUS blocks (sequence.rs:275-282):
    slots_per_rate = S/4; decay[s] = MT4_RATES[s // slots_per_rate]."""
    if state_slots % 4 != 0:
        raise ValueError(f"state_slots must be divisible by 4, got {state_slots}")
    spr = state_slots // 4
    decay = np.empty(state_slots, dtype=np.float32)
    for s in range(state_slots):
        decay[s] = MT4_RATES[s // spr]
    return decay


# --- config ----------------------------------------------------------------
@dataclass
class ModelConfig:
    d_model: int = 512
    d_ff: int = 1024
    n_blocks: int = 6
    state_slots: int = 512
    n_experts: int = 1  # 1 == dense FFN; >1 == top-1 MoE
    vocab: int = 1024
    router_rank: int | None = None  # None -> clamp(ceil(n_experts/4),1,8)
    # QAT toggles (OFF for the teacher, wired for the student).
    qat_weights: bool = False  # per-row ternary weight fake-quant (STE)
    qat_acts: bool = False  # Int8 activation fake-quant at the 4 sites
    # aux-loss weights: standard Switch/ST-MoE training values. These are the
    # EFFECTIVE training weights on the differentiable aux terms, NOT the Rust
    # CONFIG defaults (router.rs pins all 1.0, but those are unweighted term
    # magnitudes, not the loss multipliers). z_loss = mean_tok logsumexp(raw)^2
    # is ~O(logE^2)~O(tens) per block and would otherwise dwarf the CE (~1-7
    # nats); the balance term ~O(1). Switch/ST-MoE use lambda_z~1e-3,
    # lambda_balance~1e-2 so the aux nudges routing without swamping the LM loss.
    lambda_zrouter: float = 1e-3
    lambda_balance: float = 1e-2

    def resolved_rank(self) -> int:
        if self.router_rank is not None:
            return self.router_rank
        return max(1, min(8, math.ceil(self.n_experts / 4)))

    def to_dict(self) -> dict:
        return {
            "d_model": self.d_model,
            "d_ff": self.d_ff,
            "n_blocks": self.n_blocks,
            "state_slots": self.state_slots,
            "n_experts": self.n_experts,
            "vocab": self.vocab,
            "router_rank": self.router_rank,
            "qat_weights": self.qat_weights,
            "qat_acts": self.qat_acts,
            "lambda_zrouter": self.lambda_zrouter,
            "lambda_balance": self.lambda_balance,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "ModelConfig":
        known = {f: d[f] for f in d if f in cls.__dataclass_fields__}
        return cls(**known)


# --- ternary projection ----------------------------------------------------
class TernaryLinear(nn.Module):
    """Ternary-QAT-capable linear projection: out_r = sum_c w_rc*scale_r*x_c.

    Holds latent fp32 weight [rows,cols], a per-row latent scale [rows] and a
    per-row ternarize threshold [rows,1]. QAT off (teacher): a plain fp32
    matvec using the latent weight directly. QAT on (student): STE-ternarize the
    latent to {-1,0,+1}, STE-quantize the row scale to Q8.8, then matvec."""

    def __init__(self, rows: int, cols: int, qat: bool = False):
        super().__init__()
        std = 1.0 / math.sqrt(cols)
        self.weight = mx.random.normal((rows, cols)) * std
        self.scale = mx.ones((rows,))
        self.threshold = mx.zeros((rows, 1))
        self._qat = qat

    def set_qat(self, weights: bool) -> None:
        """Runtime toggle of the per-row ternary weight fake-quant (used by the
        Off->Hard QAT schedule; the flag is a plain attribute read every
        forward, so flipping it mid-training takes effect immediately)."""
        self._qat = weights

    def calibrate(self, delta_frac: float = 0.7) -> None:
        """TWN-style per-row calibration of (threshold, scale) from the current
        fp latent weight (Li & Liu, Ternary Weight Networks). Called at the
        Off->Hard flip so the ternary approximation `tern*scale` starts MATCHED
        to the trained fp magnitude instead of blowing up: without it, ternary
        rows are +-1 (magnitude ~1) while the fp latent is ~1/sqrt(cols), a huge
        activation-amplifying mismatch.

        Per output row r: threshold_r = delta_frac * mean_c |w_rc|; the optimal
        scale for the ternary that keeps entries with |w|>threshold is
        scale_r = mean over the kept entries of |w_rc|. Result: real ternary
        sparsity (genuine zeros) AND magnitude-matched rows."""
        absw = mx.abs(self.weight)  # [rows, cols]
        row_mean = mx.mean(absw, axis=1, keepdims=True)  # [rows, 1]
        thr = delta_frac * row_mean  # [rows, 1]
        mask = (absw > thr).astype(mx.float32)
        nnz = mx.sum(mask, axis=1)  # [rows]
        scale = mx.sum(absw * mask, axis=1) / mx.maximum(nnz, 1.0)  # [rows]
        self.threshold = thr
        self.scale = mx.maximum(scale, 1.0 / 256.0)  # keep >= 1 Q8.8 tick
        mx.eval(self.threshold, self.scale)

    def effective_weight(self) -> mx.array:
        if not self._qat:
            return self.weight
        tern = ternarize_ste(self.weight, self.threshold)
        sc = q8_8_ste(self.scale)
        return tern * sc[:, None]

    def __call__(self, x: mx.array) -> mx.array:
        # x [..., cols] -> [..., rows]
        return x @ self.effective_weight().T


# --- FFN expert ------------------------------------------------------------
class FFNExpert(nn.Module):
    """Pre-norm residual FFN (mirrors the f32 FFN loop, model_ref.rs:292-305):
    normed=rms_norm_clip(x); [aq]; hidden=up(normed); hidden=[aq](gelu(hidden));
    delta=down(hidden). Returns the delta only (residual added by the caller)."""

    def __init__(self, cfg: ModelConfig):
        super().__init__()
        self.up = TernaryLinear(cfg.d_ff, cfg.d_model, qat=cfg.qat_weights)
        self.down = TernaryLinear(cfg.d_model, cfg.d_ff, qat=cfg.qat_weights)
        self._qat_acts = cfg.qat_acts

    def set_qat(self, weights: bool, acts: bool) -> None:
        self.up.set_qat(weights)
        self.down.set_qat(weights)
        self._qat_acts = acts

    def delta(self, x: mx.array) -> mx.array:
        normed = rms_norm_clip(x)
        if self._qat_acts:  # site (3)
            normed = act_fake_quant(normed)
        hidden = self.up(normed)
        hidden = gelu_approx(hidden)
        if self._qat_acts:  # site (4): gelu THEN quant
            hidden = act_fake_quant(hidden)
        return self.down(hidden)


class MoEBlock(nn.Module):
    """Dense FFN (n_experts==1) or top-1 MoE over n_experts experts.

    Router (router.rs:575-627) runs on the pre-norm residual x (raw [D] input,
    no internal norm -- see spec open-question, pinned to the reference):
      hidden = input_projection[rank,D] @ x + input_bias -> [rank]
      raw = expert_projection[E,rank] @ hidden + expert_bias -> [E]
      probs = softmax(raw); expert_index = argmax(raw)  (lowest-index tiebreak)
    Hard top-1: only the selected expert's delta contributes. Router
    projections are full-precision (router.rs matvec is plain f32)."""

    def __init__(self, cfg: ModelConfig):
        super().__init__()
        self.n_experts = cfg.n_experts
        self.experts = [FFNExpert(cfg) for _ in range(cfg.n_experts)]
        if cfg.n_experts > 1:
            rank = cfg.resolved_rank()
            self.rank = rank
            self.input_projection = mx.random.normal((rank, cfg.d_model)) * (
                1.0 / math.sqrt(cfg.d_model)
            )
            self.input_bias = mx.zeros((rank,))
            self.expert_projection = mx.random.normal((cfg.n_experts, rank)) * (
                1.0 / math.sqrt(rank)
            )
            self.expert_bias = mx.zeros((cfg.n_experts,))

    def set_qat(self, weights: bool, acts: bool) -> None:
        for e in self.experts:
            e.set_qat(weights, acts)

    def _route(self, xf: mx.array):
        """Router forward on a flat [N,D] token batch -> (probs [N,E],
        idx [N] int, onehot [N,E], zloss [N]). Router projections are
        full-precision (router.rs matvec is plain f32)."""
        hid = xf @ self.input_projection.T + self.input_bias  # [N,rank]
        raw = hid @ self.expert_projection.T + self.expert_bias  # [N,E]
        probs = mx.softmax(raw, axis=-1)
        idx = mx.argmax(raw, axis=-1)  # lowest-index tiebreak
        onehot = (idx[:, None] == mx.arange(self.n_experts)[None, :]).astype(mx.float32)
        zloss = mx.logsumexp(raw, axis=-1) ** 2  # z_loss uses RAW logits [N]
        return probs, idx, onehot, zloss

    def __call__(self, x: mx.array):
        """Efficient top-1 MoE. x [..., D] (any leading batch, typically the
        flattened [N=B*T, D] window). Returns (x_out, stats); stats is None for
        the dense (n_experts==1) path, else a dict with per-token 'onehot' [N,E],
        'probs' [N,E], 'zloss' [N] for the window-level aux aggregation.

        SPARSE DISPATCH: hard top-1 selects one expert per token (argmax,
        lowest-index tiebreak). Instead of computing ALL experts densely and
        masking (~E x FFN work), each expert runs on ONLY its assigned tokens
        (gather -> expert.delta -> scatter), so total FFN work is ~1x. The
        argmax dispatch indices carry no gradient (argmax is non-differentiable),
        so materializing them to host to build the per-expert row groups changes
        no gradient: the router still trains via the differentiable aux loss
        (probs/raw), and expert weights train through the gather/scatter path.
        Semantics are BIT-IDENTICAL to the dense compute-then-select form
        (each token's expert output is independent of the other tokens); see
        `forward_dense` and the equality test."""
        if self.n_experts == 1:
            return x + self.experts[0].delta(x), None
        lead = x.shape[:-1]
        d = x.shape[-1]
        xf = x.reshape(-1, d)  # [N,D]
        probs, idx, onehot, zloss = self._route(xf)
        # argmax indices carry no gradient -> host-materialize once to build the
        # per-expert token groups (one sync per block, not per token).
        idx_np = np.asarray(idx)
        out = mx.zeros_like(xf)
        for e in range(self.n_experts):
            rows = np.nonzero(idx_np == e)[0]
            if rows.size == 0:
                continue
            r = mx.array(rows)
            sub = mx.take(xf, r, axis=0)  # [k,D] gather assigned tokens
            out[r] = self.experts[e].delta(sub)  # scatter expert delta back
        x_out = (xf + out).reshape(*lead, d)
        stats = {"onehot": onehot, "probs": probs, "zloss": zloss}
        return x_out, stats

    def forward_dense(self, x: mx.array):
        """Reference dense-compute-then-select forward (pre-optimization form):
        compute ALL experts on every token, then hard top-1 select. Kept as the
        numerical reference for the sparse-dispatch equality test. Same
        (x_out, stats) contract as `__call__`."""
        if self.n_experts == 1:
            return x + self.experts[0].delta(x), None
        lead = x.shape[:-1]
        d = x.shape[-1]
        xf = x.reshape(-1, d)  # [N,D]
        probs, idx, onehot, zloss = self._route(xf)
        deltas = mx.stack([e.delta(xf) for e in self.experts], axis=1)  # [N,E,D]
        sel_delta = mx.sum(onehot[:, :, None] * deltas, axis=1)  # [N,D]
        x_out = (xf + sel_delta).reshape(*lead, d)
        stats = {"onehot": onehot, "probs": probs, "zloss": zloss}
        return x_out, stats


class LinearStateBlock(nn.Module):
    """LinearState MT4 recurrent block (f32_state_forward, state_model_ref
    .rs:454-469): normed=rms_norm_clip(x); [aq]; delta=W_in@normed; per slot
    h[s]=h[s]*decay[s]+delta[s]; y=W_out@h; x += [aq](y)."""

    def __init__(self, cfg: ModelConfig):
        super().__init__()
        self.state_in = TernaryLinear(cfg.state_slots, cfg.d_model, qat=cfg.qat_weights)
        self.state_out = TernaryLinear(cfg.d_model, cfg.state_slots, qat=cfg.qat_weights)
        self._qat_acts = cfg.qat_acts

        # decay is a fixed MT4 constant -> stored as numpy so nn.Module does not
        # register it as a trainable parameter.
        self._decay_np = build_decay(cfg.state_slots)

    def set_qat(self, weights: bool, acts: bool) -> None:
        self.state_in.set_qat(weights)
        self.state_out.set_qat(weights)
        self._qat_acts = acts

    def __call__(self, x: mx.array, h: mx.array):
        decay = mx.array(self._decay_np)
        normed = rms_norm_clip(x)
        if self._qat_acts:  # site (1)
            normed = act_fake_quant(normed)
        delta = self.state_in(normed)  # [B,S]
        h = h * decay + delta
        y = self.state_out(h)  # [B,D]
        if self._qat_acts:  # site (2)
            y = act_fake_quant(y)
        return x + y, h

    def forward_sequence(self, x_all: mx.array, h: mx.array):
        """Whole-window form of `__call__`: x_all [B,T,D], initial state h [B,S]
        -> (x_out_all [B,T,D], final h [B,S]). Bit-identical to calling
        `__call__` for t=0..T-1 (each output row is an independent dot product,
        so batching the state_in/state_out matvecs over T does not change any
        value), but it hoists the two ternary matvecs and both activation
        fake-quants out of the per-token Python loop; only the cheap elementwise
        MT4 recurrence stays sequential. site (1)/(2) act-quant order preserved."""
        B, T, D = x_all.shape
        decay = mx.array(self._decay_np)  # [S]
        normed = rms_norm_clip(x_all)  # [B,T,D]
        if self._qat_acts:  # site (1): before W_in
            normed = act_fake_quant(normed)
        delta_all = self.state_in(normed)  # [B,T,S] (batched over T)
        hs = []
        ht = h
        for t in range(T):  # elementwise recurrence: h_t = h_{t-1}*decay + delta_t
            ht = ht * decay + delta_all[:, t]
            hs.append(ht)
        H = mx.stack(hs, axis=1)  # [B,T,S]
        y_all = self.state_out(H)  # [B,T,D] (batched over T)
        if self._qat_acts:  # site (2): on y, after W_out, before residual add
            y_all = act_fake_quant(y_all)
        return x_all + y_all, ht


# --- full model ------------------------------------------------------------
class GBModel(nn.Module):
    def __init__(self, cfg: ModelConfig):
        super().__init__()
        self.cfg = cfg
        std = 1.0 / math.sqrt(cfg.d_model)
        self.embedding = mx.random.normal((cfg.vocab, cfg.d_model)) * std
        self.state_block = LinearStateBlock(cfg)
        self.blocks = [MoEBlock(cfg) for _ in range(cfg.n_blocks)]

    def set_qat(self, weights: bool, acts: bool) -> None:
        """Flip every QAT hook in the module tree at once. `weights` toggles the
        per-row ternary weight fake-quant on all ternary projections; `acts`
        toggles the Int8 activation fake-quant at the four sites. Also records
        the current mode on cfg so a subsequent checkpoint/export sees it."""
        self.state_block.set_qat(weights, acts)
        for blk in self.blocks:
            blk.set_qat(weights, acts)
        self.cfg.qat_weights = weights
        self.cfg.qat_acts = acts

    def calibrate_ternary_scales(self, delta_frac: float = 0.7) -> None:
        """TWN-calibrate (threshold, scale) of EVERY ternary projection from the
        current fp latent weights. Call once at the Off->Hard flip so the hard
        phase starts from a magnitude-matched, genuinely-sparse ternary
        approximation (see TernaryLinear.calibrate)."""
        self.state_block.state_in.calibrate(delta_frac)
        self.state_block.state_out.calibrate(delta_frac)
        for blk in self.blocks:
            for expert in blk.experts:
                expert.up.calibrate(delta_frac)
                expert.down.calibrate(delta_frac)

    def init_state(self, batch: int) -> mx.array:
        """Zeroed recurrent state [B,S] at stream start."""
        return mx.zeros((batch, self.cfg.state_slots))

    def _aux_from_totals(self, block_onehot, block_probs, block_zloss, tok_count):
        """Window-level router aux from per-block token totals.

        ``block_onehot`` and ``block_probs`` are ``[n_blocks, n_experts]``.
        The Switch balance term is computed independently for each block and
        then summed; aggregating expert totals before multiplying would let
        different blocks collapse onto complementary experts while appearing
        globally balanced. ``block_zloss`` is ``[n_blocks]`` and remains a sum
        of the per-block token means.
        """
        cfg = self.cfg
        # Hard top-1 assignments are stop-gradient dispatch provenance. Router
        # learning for this term flows through the soft probabilities only.
        f = mx.stop_gradient(block_onehot / tok_count)  # [L,E]
        p = block_probs / tok_count  # [L,E]
        balance = cfg.n_experts * mx.sum(f * p)
        zrouter = mx.sum(block_zloss) / tok_count
        return cfg.lambda_balance * balance + cfg.lambda_zrouter * zrouter

    def __call__(self, ids: mx.array, h: mx.array):
        """ids [B,T] int -> (logits [B,T,V], new_h [B,S], aux scalar).

        The LinearState recurrence is sequential over t (not parallelizable) but
        the pre-norm FFN/MoE blocks are token-independent (they never feed the
        recurrent state h), so the forward runs in two stages: (1) the state
        block over the whole window collecting the post-state residual x_all
        [B,T,D]; (2) the blocks on the FLATTENED [N=B*T, D] token batch, so each
        block's top-1 sparse dispatch amortizes over all N tokens at once (one
        router dispatch per block, not per token). Bit-identical to the
        per-token reference (`_forward_per_token`); see the equality test. `h`
        is the initial recurrent state; the caller detaches it across windows."""
        B, T = ids.shape
        cfg = self.cfg
        embed_all = self.embedding[ids]  # [B,T,D]
        x_all, h = self.state_block.forward_sequence(embed_all, h)  # [B,T,D], [B,S]

        xf = x_all.reshape(B * T, cfg.d_model)  # [N,D]
        block_onehot = []
        block_probs = []
        block_zloss = []
        for blk in self.blocks:
            xf, stats = blk(xf)
            if stats is not None:
                block_onehot.append(mx.sum(stats["onehot"], axis=0))  # [E]
                block_probs.append(mx.sum(stats["probs"], axis=0))  # [E]
                block_zloss.append(mx.sum(stats["zloss"]))  # scalar

        normed = rms_norm_clip(xf)  # [N,D]
        logits = (normed @ self.embedding.T).reshape(B, T, cfg.vocab)  # [B,T,V]

        if not block_onehot:
            aux = mx.array(0.0)
        else:
            aux = self._aux_from_totals(
                mx.stack(block_onehot),
                mx.stack(block_probs),
                mx.stack(block_zloss),
                B * T,
            )
        return logits, h, aux

    def _forward_per_token(self, ids: mx.array, h: mx.array, dense_moe: bool = False):
        """Sequential per-token reference forward (pre-optimization form): loop
        t, run the state block and every block on the single-token [B,D] slice.
        `dense_moe=True` routes each MoE block through `forward_dense` (compute
        all experts then select). Kept ONLY as the numerical reference for the
        sparse-dispatch / window-batched equality tests -- NOT used in training.
        """
        B, T = ids.shape
        cfg = self.cfg
        block_onehot = [None] * len(self.blocks)
        block_probs = [None] * len(self.blocks)
        block_zloss = [None] * len(self.blocks)
        tok_count = 0
        logits_steps = []
        for t in range(T):
            x = self.embedding[ids[:, t]]  # [B,D]
            x, h = self.state_block(x, h)
            for block_index, blk in enumerate(self.blocks):
                x, stats = blk.forward_dense(x) if dense_moe else blk(x)
                if stats is not None:
                    oh = mx.sum(stats["onehot"], axis=0)
                    pr = mx.sum(stats["probs"], axis=0)
                    zl = mx.sum(stats["zloss"])
                    if block_onehot[block_index] is None:
                        block_onehot[block_index] = oh
                        block_probs[block_index] = pr
                        block_zloss[block_index] = zl
                    else:
                        block_onehot[block_index] = block_onehot[block_index] + oh
                        block_probs[block_index] = block_probs[block_index] + pr
                        block_zloss[block_index] = block_zloss[block_index] + zl
            normed = rms_norm_clip(x)
            logits_steps.append(normed @ self.embedding.T)  # [B,V]
            tok_count += B
        logits = mx.stack(logits_steps, axis=1)  # [B,T,V]
        if not block_onehot or block_onehot[0] is None:
            aux = mx.array(0.0)
        else:
            aux = self._aux_from_totals(
                mx.stack(block_onehot),
                mx.stack(block_probs),
                mx.stack(block_zloss),
                tok_count,
            )
        return logits, h, aux

"""Off->Hard QAT schedule for the ternary MoE student (charset-80 winning recipe).

The student trains in two phases across `total_steps`:

  * OFF phase   -- the first `warmup_frac` (default 0.4) of steps run with BOTH
    QAT hooks OFF (full-precision latent weights, no activation fake-quant). This
    lets the fp latent weights and the router settle before quantization noise is
    injected. LR linearly warms up to `lr_peak` over `lr_warmup_steps`, then holds
    at `lr_peak`. Distillation weight is held at `distill_weight_start` (0.5).

  * HARD phase  -- the remaining steps run in Hard QAT (ternary weights + Int8
    activation fake-quant ON simultaneously; there is no intermediate soft phase
    in this recipe). LR cosine-decays from `lr_peak` down to
    `hard_lr_floor_mult * lr_peak` (default 0.1x) across the hard phase. The
    distillation weight ramps linearly `distill_weight_start` -> `distill_weight_end`
    (0.5 -> 0.65) across the hard phase, leaning harder on the teacher as the
    student is progressively quantized.

Everything is config-driven and pure/deterministic in `step`, so the training
loop can query (qat_weights, qat_acts, lr, distill_weight) at every step without
carrying hidden state. `warmup_frac == 0` degenerates to pure-Hard-from-step-0;
`warmup_frac == 1` degenerates to a never-hardened full-precision run (useful as
a no-QAT control in tests).
"""

from __future__ import annotations

import math
from dataclasses import dataclass


@dataclass
class QATScheduleConfig:
    total_steps: int
    warmup_frac: float = 0.4  # fraction of steps with QAT hooks OFF
    lr_peak: float = 2e-3
    lr_warmup_steps: int = 200  # linear LR ramp at the very start of the OFF phase
    hard_lr_floor_mult: float = 0.1  # hard-phase cosine decays to this * lr_peak
    distill_weight_start: float = 0.5
    distill_weight_end: float = 0.65

    def __post_init__(self) -> None:
        if not (0.0 <= self.warmup_frac <= 1.0):
            raise ValueError(f"warmup_frac must be in [0,1], got {self.warmup_frac}")
        if self.total_steps < 1:
            raise ValueError(f"total_steps must be >= 1, got {self.total_steps}")
        if self.hard_lr_floor_mult <= 0.0:
            raise ValueError("hard_lr_floor_mult must be > 0")

    @property
    def hard_start_step(self) -> int:
        """First step (0-indexed) at which the Hard QAT hooks are ON."""
        return int(round(self.warmup_frac * self.total_steps))

    def to_dict(self) -> dict:
        return {
            "total_steps": self.total_steps,
            "warmup_frac": self.warmup_frac,
            "lr_peak": self.lr_peak,
            "lr_warmup_steps": self.lr_warmup_steps,
            "hard_lr_floor_mult": self.hard_lr_floor_mult,
            "distill_weight_start": self.distill_weight_start,
            "distill_weight_end": self.distill_weight_end,
            "hard_start_step": self.hard_start_step,
        }


def is_hard(step: int, cfg: QATScheduleConfig) -> bool:
    """True once the Hard QAT phase has begun at `step`."""
    return step >= cfg.hard_start_step


def qat_flags_at(step: int, cfg: QATScheduleConfig) -> tuple[bool, bool]:
    """(qat_weights, qat_acts) at `step`. Both flip together at hard_start_step;
    Hard means ternary weights AND Int8 activation fake-quant simultaneously."""
    hard = is_hard(step, cfg)
    return hard, hard


def lr_at(step: int, cfg: QATScheduleConfig) -> float:
    """Two-phase LR: linear warmup -> hold at peak (OFF phase), then cosine decay
    to hard_lr_floor_mult*lr_peak across the HARD phase."""
    hard_start = cfg.hard_start_step
    if step < hard_start:
        # OFF phase: linear warmup to peak, then hold.
        if cfg.lr_warmup_steps > 0 and step < cfg.lr_warmup_steps:
            return cfg.lr_peak * (step + 1) / cfg.lr_warmup_steps
        return cfg.lr_peak
    # HARD phase: cosine from lr_peak down to floor.
    floor = cfg.hard_lr_floor_mult * cfg.lr_peak
    hard_len = max(1, cfg.total_steps - hard_start)
    prog = min(1.0, (step - hard_start) / hard_len)
    cos = 0.5 * (1.0 + math.cos(math.pi * prog))
    return floor + (cfg.lr_peak - floor) * cos


def distill_weight_at(step: int, cfg: QATScheduleConfig) -> float:
    """Distillation weight: held at start during the OFF phase, then ramps
    linearly start->end across the HARD phase."""
    hard_start = cfg.hard_start_step
    if step < hard_start:
        return cfg.distill_weight_start
    hard_len = max(1, cfg.total_steps - hard_start)
    prog = min(1.0, (step - hard_start) / hard_len)
    return cfg.distill_weight_start + (
        cfg.distill_weight_end - cfg.distill_weight_start
    ) * prog

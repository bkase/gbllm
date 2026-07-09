"""Autoregressive text generation from a trained GBModel (subword MoE student).

The model is stateful (LinearState MT4 recurrence), so generation carries the
recurrent state ``h`` across tokens exactly as training does: state starts at
zero (stream start), the prompt warms it up, then each sampled token advances it
by one step. This is the tangible "does the model actually write?" harness.

Sampling is done on the host with numpy (logits are tiny: [vocab]), so it is
fully deterministic given a seed and adds no GPU work -- generation can run on
the CPU device beside a live GPU training job. Greedy (temperature==0), or
temperature + top-k nucleus-free top-k sampling.
"""

from __future__ import annotations

from dataclasses import dataclass

import mlx.core as mx
import numpy as np

from .tokenizer import BPEModel


@dataclass
class SampleConfig:
    max_new_tokens: int = 200
    temperature: float = 0.8
    top_k: int = 40  # 0 disables top-k
    seed: int = 0


def _sample_next(logits: np.ndarray, cfg: SampleConfig, rng: np.random.Generator) -> int:
    """Sample one token id from a [vocab] logit vector (numpy, deterministic)."""
    if cfg.temperature <= 0.0:
        return int(np.argmax(logits))
    z = logits.astype(np.float64) / cfg.temperature
    if cfg.top_k and 0 < cfg.top_k < z.shape[-1]:
        # keep only the top-k logits; mask the rest to -inf
        kth = np.partition(z, -cfg.top_k)[-cfg.top_k]
        z = np.where(z < kth, -np.inf, z)
    z -= z.max()  # stabilize
    p = np.exp(z)
    p /= p.sum()
    return int(rng.choice(p.shape[-1], p=p))


def generate(
    model,
    tokenizer: BPEModel,
    prompt: str,
    cfg: SampleConfig | None = None,
) -> str:
    """Generate a continuation of ``prompt``. Returns the full decoded text
    (prompt + continuation)."""
    cfg = cfg or SampleConfig()
    rng = np.random.default_rng(cfg.seed)

    ids = tokenizer.encode(prompt)
    if not ids:
        ids = tokenizer.encode(" ")  # non-empty seed so the state has input
    out_ids = list(ids)

    h = model.init_state(1)
    seq = mx.array([ids])  # [1, T]
    logits, h, _ = model(seq, h)
    mx.eval(logits, h)
    last = np.array(logits[0, -1])  # [vocab]

    for _ in range(cfg.max_new_tokens):
        nxt = _sample_next(last, cfg, rng)
        out_ids.append(nxt)
        step = mx.array([[nxt]])
        logits, h, _ = model(step, h)
        mx.eval(logits, h)
        last = np.array(logits[0, -1])

    return tokenizer.decode(out_ids)


def load_model(ckpt_dir: str):
    """Load a GBModel from either a hardened export (deployable math) or a raw
    trainable checkpoint, autodetected by the manifest/config on disk."""
    import json
    from pathlib import Path

    d = Path(ckpt_dir)
    manifest = d / "manifest.json"
    if manifest.exists():
        fmt = json.loads(manifest.read_text()).get("format", "")
        if fmt.startswith("gbllm_student_hardened"):
            from .export import load_hardened

            return load_hardened(d)
    from .train import load_checkpoint

    return load_checkpoint(d)

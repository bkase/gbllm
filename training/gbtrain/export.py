"""Hardened deployable export for the ternary MoE student.

The QAT student trains with fp32 latent weights; the deployed math is
``out_r = sum_c w_rc * (scale_raw_r / 256) * x_c`` with ``w_rc in {-1,0,+1}``
(model_ref.rs:270-281). This module materializes that hardened form:

  * every ternary projection (state_in, state_out, and every expert up/down)
    exports an int8 ternary weight matrix ``w_tern`` (values in {-1,0,+1}) plus a
    per-output-row ``scale_raw`` (u16, Q8.8 grid = scale_raw/256);
  * the embedding E stays fp32 (tied head reuses it);
  * router input/expert projections + biases stay fp32 (router.rs matvec is plain
    f32; the reference does not ternarize the router);
  * per-slot MT4 decay raws {128,192,224,240} are recorded in the manifest.

The hardening is EXACT with respect to the QAT-on student forward: the STE
forward value of ``ternarize_ste(w, thr)`` is exactly the ternary set and
``q8_8_ste(scale)`` is exactly ``round(scale*256)/256``. So the exported
``w_tern * scale_raw/256`` equals the effective weight the student used, and a
reloaded model reproduces the student's eval bit-for-bit (see
``load_hardened`` / test_student round-trip).

Layout on disk (``out_dir``):
  hardened.safetensors  -- all tensors (see key scheme below)
  manifest.json         -- topology block + per-projection ternary+scale listing
"""

from __future__ import annotations

import json
from pathlib import Path

import mlx.core as mx
import numpy as np

from .model import (
    MT4_DECAY_RAWS,
    GBModel,
    ModelConfig,
    q8_8_ste,
    ternarize_ste,
)

MANIFEST_FORMAT = "gbllm_student_hardened.v1"


def _ternary_projections(model: GBModel) -> list[tuple[str, object]]:
    """(dotted_name, TernaryLinear) for every ternary projection, in a stable
    deploy order: state block first, then blocks x experts x {up,down}."""
    out: list[tuple[str, object]] = []
    out.append(("state_block.state_in", model.state_block.state_in))
    out.append(("state_block.state_out", model.state_block.state_out))
    for bi, blk in enumerate(model.blocks):
        for ei, expert in enumerate(blk.experts):
            out.append((f"blocks.{bi}.experts.{ei}.up", expert.up))
            out.append((f"blocks.{bi}.experts.{ei}.down", expert.down))
    return out


def harden_projection(proj) -> tuple[mx.array, mx.array]:
    """Materialize a TernaryLinear into (w_tern int8 [rows,cols] in {-1,0,+1},
    scale_raw uint16 [rows] on the Q8.8 grid). Uses the same STE forward values
    the student trained with, so the dequant is exact."""
    tern = ternarize_ste(proj.weight, proj.threshold)  # STE fwd == pure ternary
    w_tern = mx.round(tern).astype(mx.int8)  # {-1,0,+1}
    raw = mx.clip(mx.round(proj.scale * 256.0), 0.0, 65535.0).astype(mx.uint16)
    return w_tern, raw


def export_hardened(model: GBModel, out_dir: str | Path, meta: dict | None = None) -> Path:
    """Write the hardened deployable student to ``out_dir``. Returns the path."""
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    cfg = model.cfg

    tensors: dict[str, mx.array] = {}
    tensors["embedding"] = model.embedding.astype(mx.float32)

    projections_manifest: list[dict] = []
    for name, proj in _ternary_projections(model):
        w_tern, raw = harden_projection(proj)
        mx.eval(w_tern, raw)
        tensors[f"{name}.w_tern"] = w_tern
        tensors[f"{name}.scale_raw"] = raw
        rows, cols = int(proj.weight.shape[0]), int(proj.weight.shape[1])
        vals = np.unique(np.array(w_tern)).tolist()
        nnz = int(np.count_nonzero(np.array(w_tern)))
        projections_manifest.append(
            {
                "name": name,
                "shape": [rows, cols],
                "w_tern_dtype": "int8",
                "w_tern_values": vals,  # subset of [-1,0,1]
                "scale_raw_dtype": "uint16",
                "scale_grid": "Q8.8 (scale_raw/256)",
                "density": nnz / max(1, rows * cols),
            }
        )

    # Router (fp32) + biases, per block.
    router_manifest: list[dict] = []
    if cfg.n_experts > 1:
        for bi, blk in enumerate(model.blocks):
            tensors[f"blocks.{bi}.router.input_projection"] = blk.input_projection.astype(mx.float32)
            tensors[f"blocks.{bi}.router.input_bias"] = blk.input_bias.astype(mx.float32)
            tensors[f"blocks.{bi}.router.expert_projection"] = blk.expert_projection.astype(mx.float32)
            tensors[f"blocks.{bi}.router.expert_bias"] = blk.expert_bias.astype(mx.float32)
            router_manifest.append(
                {
                    "block": bi,
                    "rank": int(blk.rank),
                    "dtype": "float32",
                    "tensors": [
                        f"blocks.{bi}.router.input_projection",
                        f"blocks.{bi}.router.input_bias",
                        f"blocks.{bi}.router.expert_projection",
                        f"blocks.{bi}.router.expert_bias",
                    ],
                }
            )

    mx.eval(tensors)
    mx.save_safetensors(str(out / "hardened.safetensors"), tensors)

    manifest = {
        "format": MANIFEST_FORMAT,
        "topology": {
            "d_model": cfg.d_model,
            "d_ff": cfg.d_ff,
            "n_blocks": cfg.n_blocks,
            "state_slots": cfg.state_slots,
            "n_experts": cfg.n_experts,
            "vocab": cfg.vocab,
            "router_rank": cfg.resolved_rank() if cfg.n_experts > 1 else None,
        },
        "embedding": {"name": "embedding", "dtype": "float32", "shape": list(model.embedding.shape)},
        "decay_raws": list(MT4_DECAY_RAWS),
        "act_fake_quant": {"range": 8.0, "qmax": 127, "grid": "Int8 symmetric [-8,8]"},
        "ternary_projections": projections_manifest,
        "routers": router_manifest,
    }
    if meta:
        manifest["meta"] = meta
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return out


def load_hardened(out_dir: str | Path) -> GBModel:
    """Reload a hardened export into a GBModel whose forward reproduces the
    student's deployed math bit-for-bit.

    The reload folds each ``w_tern * scale_raw/256`` back into the plain fp32
    ``TernaryLinear.weight`` and turns the WEIGHT fake-quant OFF (the weights are
    already hardened, so re-ternarizing would be a no-op at best and lossy at
    worst), while keeping the ACTIVATION fake-quant ON (Int8 acts are part of the
    deployed math). Effective weight with qat_weights=False is exactly the
    dequantized hardened matrix."""
    out = Path(out_dir)
    manifest = json.loads((out / "manifest.json").read_text())
    if manifest.get("format") != MANIFEST_FORMAT:
        raise ValueError(f"unexpected manifest format: {manifest.get('format')}")
    topo = manifest["topology"]
    cfg = ModelConfig(
        d_model=topo["d_model"],
        d_ff=topo["d_ff"],
        n_blocks=topo["n_blocks"],
        state_slots=topo["state_slots"],
        n_experts=topo["n_experts"],
        vocab=topo["vocab"],
        router_rank=topo.get("router_rank"),
        qat_weights=False,  # weights already hardened into .weight
        qat_acts=True,  # Int8 activation fake-quant is part of deployed math
    )
    model = GBModel(cfg)
    tensors = mx.load(str(out / "hardened.safetensors"))

    model.embedding = tensors["embedding"].astype(mx.float32)
    for name, proj in _ternary_projections(model):
        w_tern = tensors[f"{name}.w_tern"].astype(mx.float32)
        raw = tensors[f"{name}.scale_raw"].astype(mx.float32)
        proj.weight = w_tern * (raw / 256.0)[:, None]  # dequant fold
        proj.scale = mx.ones((proj.weight.shape[0],))
        proj.threshold = mx.zeros((proj.weight.shape[0], 1))
        proj._qat = False

    if cfg.n_experts > 1:
        for bi, blk in enumerate(model.blocks):
            blk.input_projection = tensors[f"blocks.{bi}.router.input_projection"].astype(mx.float32)
            blk.input_bias = tensors[f"blocks.{bi}.router.input_bias"].astype(mx.float32)
            blk.expert_projection = tensors[f"blocks.{bi}.router.expert_projection"].astype(mx.float32)
            blk.expert_bias = tensors[f"blocks.{bi}.router.expert_bias"].astype(mx.float32)

    mx.eval(model.parameters())
    return model

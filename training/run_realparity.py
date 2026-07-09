"""Build a real-geometry MLX<->Rust parity fixture from a trained student.

Hardens a checkpoint (or uses an existing hardened export), bridges it to the
Rust-consumable f_s8_moe_state_checkpoint_export.v2 layout, runs the MLX f32
forward on the CPU device to produce golden.json, and drops both into an output
dir laid out as `<out>/ckpt` + `<out>/golden.json` -- exactly what the Rust
`rust_forward_matches_mlx_golden_external` test consumes via MOE_PARITY_DIR.

Usage (after training finishes):
  uv run python run_realparity.py --ckpt artifacts/student_moe_d192x8 \
      --out /tmp/moe_parity_real --tokens 24
  MOE_PARITY_DIR=/tmp/moe_parity_real cargo test -p gbf-bench --test moe_parity \
      -- --ignored --nocapture rust_forward_matches_mlx_golden_external
"""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path

import mlx.core as mx

mx.set_default_device(mx.cpu)  # never contend with a GPU training job
import numpy as np

from gbtrain.bridge import bridge_hardened_export
from gbtrain.export import export_hardened, load_hardened
from gbtrain.train import load_checkpoint


def _load_or_harden(ckpt: Path, scratch: Path):
    """Return (hardened_dir, model). If ckpt is already a hardened export use it;
    else harden the trainable checkpoint into scratch."""
    manifest = ckpt / "manifest.json"
    if manifest.exists() and json.loads(manifest.read_text()).get("format", "").startswith(
        "gbllm_student_hardened"
    ):
        return ckpt, load_hardened(ckpt)
    model = load_checkpoint(ckpt)
    hardened = scratch / "hardened"
    export_hardened(model, hardened)
    return hardened, load_hardened(hardened)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ckpt", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--tokens", type=int, default=24)
    ap.add_argument("--seed", type=int, default=0xC0FFEE)
    args = ap.parse_args()

    out = Path(args.out)
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    scratch = out / "_scratch"
    scratch.mkdir()

    hardened, model = _load_or_harden(Path(args.ckpt), scratch)
    cfg = model.cfg

    rng = np.random.default_rng(args.seed)
    B, T = 1, args.tokens
    ids_np = rng.integers(0, cfg.vocab, size=(B, T)).astype(np.int32)
    ids = mx.array(ids_np)
    h = model.init_state(B)
    logits, _, _ = model(ids, h)
    mx.eval(logits)
    logits_np = np.array(logits).astype(np.float32)
    argmax_np = np.array(mx.argmax(logits, axis=-1)).astype(np.int32)

    bridge_hardened_export(hardened, out / "ckpt")
    golden = {
        "note": f"real-geometry MLX cpu f32 forward of {args.ckpt}",
        "topology": {
            "d_model": cfg.d_model, "d_ff": cfg.d_ff, "n_blocks": cfg.n_blocks,
            "state_slots": cfg.state_slots, "n_experts": cfg.n_experts,
            "vocab": cfg.vocab, "router_rank": cfg.resolved_rank(),
        },
        "B": B, "T": T,
        "ids": ids_np.reshape(-1).tolist(),
        "logits_shape": [B, T, cfg.vocab],
        "logits": [round(float(v), 6) for v in logits_np.reshape(-1)],
        "argmax": argmax_np.reshape(-1).tolist(),
        "logit_abs_max": float(np.abs(logits_np).max()),
    }
    (out / "golden.json").write_text(json.dumps(golden))
    shutil.rmtree(scratch)
    print(f"wrote {out}/ckpt + golden.json  (d{cfg.d_model} V{cfg.vocab} "
          f"{cfg.n_experts}exp {cfg.n_blocks}blk, T={T})")
    print(f"run: MOE_PARITY_DIR={out} cargo test -p gbf-bench --test moe_parity "
          f"-- --ignored --nocapture rust_forward_matches_mlx_golden_external")


if __name__ == "__main__":
    main()

"""Export bridge: MLX hardened student -> Rust deploy artifact (bd-3mzda).

This is the seam between GPU training and the deterministic Rust deploy
pipeline. It reads a ``gbllm_student_hardened.v1`` directory (produced by
``gbtrain.export.export_hardened`` -- manifest.json + hardened.safetensors) and
re-serializes it into the tensor-file layout the Rust compiler consumes
(``gbf-codegen/src/import_state_checkpoint.rs``), one ``.bin`` per tensor plus
a self-describing ``manifest.json`` with a per-tensor sha256.

**Schema.** The dense arm matches ``f_s5_state_checkpoint_export.v1`` exactly.
The MoE arm emits ``f_s8_moe_state_checkpoint_export.v2``. The ``.v2`` differs
from the s8 experiment's ``.v1`` in the router: our student uses the *real
deployed* low-rank router (``gbf-model/src/qat/router.rs`` ``Top1RouterQat``:
``hidden = Win @ x + bin`` -> ``raw = Wout @ hidden + bout`` -> ``argmax(raw)``,
lowest-index tiebreak, on the RAW pre-norm residual, all f32), NOT s8's
simplified flat ``[n_experts, d_model]`` router. The bridge carries all four
router tensors per block so the Rust side can reproduce dispatch exactly.

**Byte formats** (all little-endian, row-major), matching the Rust readers:
  * ternary weights  -> ``tensors/{base}.ternary.i8.bin``      (i8, values {-1,0,1})
  * per-row scales   -> ``tensors/{base}.scales.q8_8_u16le.bin`` (u16 Q8.8, f32 = raw/256)
  * embedding/router -> ``tensors/{name}.f32.bin``             (f32)
  * state decay      -> ``tensors/state_decay.q8_8_u16le.bin``  (u16 Q8.8)

**Name mapping** (MLX hardened key -> Rust base):
  * ``state_block.state_in``            -> ``state_input_to_state``
  * ``state_block.state_out``           -> ``state_state_to_output``
  * ``blocks.{b}.experts.{e}.up``       -> ``block{b}_expert{e}_up``
  * ``blocks.{b}.experts.{e}.down``     -> ``block{b}_expert{e}_down``
  * ``blocks.{b}.router.input_projection`` -> ``block{b}_router_input_projection`` (+ input_bias, expert_projection, expert_bias)

The bridge is pure numpy + stdlib (no MLX / no GPU), so it can run alongside a
live training job without touching the Metal device.
"""

from __future__ import annotations

import hashlib
import json
import struct
from pathlib import Path

import numpy as np

BRIDGE_SCHEMA_MOE = "f_s8_moe_state_checkpoint_export.v2"
BRIDGE_SCHEMA_DENSE = "f_s5_state_checkpoint_export.v1"
HARDENED_FORMAT = "gbllm_student_hardened.v1"

# safetensors dtype string -> (numpy dtype, itemsize)
_ST_DTYPE = {
    "F32": np.dtype("<f4"),
    "F16": np.dtype("<f2"),
    "I8": np.dtype("i1"),
    "U8": np.dtype("u1"),
    "I16": np.dtype("<i2"),
    "U16": np.dtype("<u2"),
    "I32": np.dtype("<i4"),
    "U32": np.dtype("<u4"),
    "I64": np.dtype("<i8"),
}


def _read_safetensors(path: Path) -> dict[str, np.ndarray]:
    """Minimal numpy-only safetensors reader (no MLX, no GPU). Returns
    name -> ndarray with the on-disk dtype and shape."""
    raw = path.read_bytes()
    (header_len,) = struct.unpack_from("<Q", raw, 0)
    header = json.loads(raw[8 : 8 + header_len].decode("utf-8"))
    base = 8 + header_len
    out: dict[str, np.ndarray] = {}
    for name, meta in header.items():
        if name == "__metadata__":
            continue
        dt = _ST_DTYPE[meta["dtype"]]
        lo, hi = meta["data_offsets"]
        buf = raw[base + lo : base + hi]
        arr = np.frombuffer(buf, dtype=dt)
        out[name] = arr.reshape(meta["shape"]) if meta["shape"] else arr.reshape(())
    return out


def _sha256_hex(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def _write_tensor(
    export_dir: Path,
    index: list[dict],
    name: str,
    rel_file: str,
    role: str,
    dtype_str: str,
    shape: list[int],
    payload: bytes,
) -> None:
    dest = export_dir / rel_file
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(payload)
    index.append(
        {
            "name": name,
            "role": role,
            "dtype": dtype_str,
            "shape": shape,
            "layout": "row_major",
            "file": rel_file,
            "sha256": _sha256_hex(payload),
        }
    )


def _f32_bytes(arr: np.ndarray) -> bytes:
    return np.ascontiguousarray(arr, dtype="<f4").tobytes()


def _ternary_i8_bytes(arr: np.ndarray) -> bytes:
    """int8 {-1,0,1} row-major; -1 serializes as 0xFF (two's complement),
    matching the Rust ``v.as_i8() as u8`` convention."""
    a = np.ascontiguousarray(arr).astype(np.int8)
    if not np.all(np.isin(a, (-1, 0, 1))):
        bad = np.unique(a[~np.isin(a, (-1, 0, 1))])
        raise ValueError(f"ternary weights must be in {{-1,0,1}}; found {bad.tolist()}")
    return a.tobytes()


def _u16_le_bytes(arr: np.ndarray) -> bytes:
    a = np.ascontiguousarray(arr).astype(np.int64)
    if np.any(a < 0) or np.any(a > 0xFFFF):
        raise ValueError("Q8.8 raw scale out of u16 range")
    return a.astype("<u2").tobytes()


def bridge_hardened_export(src_dir: str | Path, out_dir: str | Path) -> Path:
    """Convert a hardened MLX export at ``src_dir`` into a Rust-consumable
    checkpoint at ``out_dir``. Returns ``out_dir``."""
    src = Path(src_dir)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)

    man = json.loads((src / "manifest.json").read_text())
    if man.get("format") != HARDENED_FORMAT:
        raise ValueError(f"unexpected hardened format: {man.get('format')!r}")
    topo = man["topology"]
    tensors = _read_safetensors(src / "hardened.safetensors")

    d_model = topo["d_model"]
    d_ff = topo["d_ff"]
    n_blocks = topo["n_blocks"]
    state_slots = topo["state_slots"]
    n_experts = topo["n_experts"]
    vocab = topo["vocab"]
    router_rank = topo.get("router_rank")
    is_moe = n_experts > 1

    index: list[dict] = []

    # Embedding (tied head).
    emb = tensors["embedding"]
    if list(emb.shape) != [vocab, d_model]:
        raise ValueError(f"embedding shape {emb.shape} != [{vocab},{d_model}]")
    _write_tensor(
        out, index, "embedding", "tensors/embedding.f32.bin",
        "token_embedding_and_tied_head", "f32_le", [vocab, d_model], _f32_bytes(emb),
    )

    def emit_ternary(mlx_base: str, rust_base: str, role: str, rows: int, cols: int) -> None:
        w = tensors[f"{mlx_base}.w_tern"]
        s = tensors[f"{mlx_base}.scale_raw"]
        if list(w.shape) != [rows, cols]:
            raise ValueError(f"{mlx_base} shape {w.shape} != [{rows},{cols}]")
        if list(s.shape) != [rows]:
            raise ValueError(f"{mlx_base} scale shape {s.shape} != [{rows}]")
        _write_tensor(
            out, index, f"{rust_base}.ternary", f"tensors/{rust_base}.ternary.i8.bin",
            f"{role}_ternary_weights", "i8 (values in {-1,0,1})", [rows, cols],
            _ternary_i8_bytes(w),
        )
        _write_tensor(
            out, index, f"{rust_base}.scales", f"tensors/{rust_base}.scales.q8_8_u16le.bin",
            f"{role}_per_output_row_scale", "u16_le (Q8.8; f32 = raw/256)", [rows],
            _u16_le_bytes(s),
        )

    # State block.
    emit_ternary("state_block.state_in", "state_input_to_state", "state_in_projection", state_slots, d_model)
    emit_ternary("state_block.state_out", "state_state_to_output", "state_out_projection", d_model, state_slots)

    # Per-slot decay (Q8.8), from the hardened manifest's decay_raws.
    decay_raws = man["decay_raws"]
    per_band = state_slots // len(decay_raws)
    decay_full = np.repeat(np.asarray(decay_raws, dtype=np.int64), per_band)
    if decay_full.shape[0] != state_slots:
        raise ValueError(f"decay expansion {decay_full.shape[0]} != state_slots {state_slots}")
    _write_tensor(
        out, index, "state_decay", "tensors/state_decay.q8_8_u16le.bin",
        "linear_state_per_slot_decay", "u16_le (Q8.8; f32 = raw/256)", [state_slots],
        _u16_le_bytes(decay_full),
    )

    # FFN blocks: MoE (low-rank router + experts) or dense.
    layers: list[dict] = []
    for bi in range(n_blocks):
        if is_moe:
            router_names = {}
            for tname, rust_suffix, shape in (
                ("input_projection", "router_input_projection", [router_rank, d_model]),
                ("input_bias", "router_input_bias", [router_rank]),
                ("expert_projection", "router_expert_projection", [n_experts, router_rank]),
                ("expert_bias", "router_expert_bias", [n_experts]),
            ):
                arr = tensors[f"blocks.{bi}.router.{tname}"]
                if list(arr.shape) != shape:
                    raise ValueError(f"block{bi} router.{tname} shape {arr.shape} != {shape}")
                rust_name = f"block{bi}_{rust_suffix}"
                _write_tensor(
                    out, index, rust_name, f"tensors/{rust_name}.f32.bin",
                    f"top1_lowrank_router_{tname}_fp", "f32_le", shape, _f32_bytes(arr),
                )
                router_names[tname] = rust_name
            experts = []
            for ei in range(n_experts):
                emit_ternary(f"blocks.{bi}.experts.{ei}.up", f"block{bi}_expert{ei}_up", "ffn_up", d_ff, d_model)
                emit_ternary(f"blocks.{bi}.experts.{ei}.down", f"block{bi}_expert{ei}_down", "ffn_down", d_model, d_ff)
                experts.append(
                    {
                        "up_ternary": f"block{bi}_expert{ei}_up.ternary",
                        "up_scales": f"block{bi}_expert{ei}_up.scales",
                        "down_ternary": f"block{bi}_expert{ei}_down.ternary",
                        "down_scales": f"block{bi}_expert{ei}_down.scales",
                    }
                )
            layers.append(
                {
                    "index": bi,
                    "kind": "prenorm_residual_top1_moe_ffn",
                    "n_experts": n_experts,
                    "router_rank": router_rank,
                    "router": router_names,
                    "experts": experts,
                    "up_shape": [d_ff, d_model],
                    "down_shape": [d_model, d_ff],
                }
            )
        else:
            emit_ternary(f"blocks.{bi}.experts.0.up", f"block{bi}_up", "ffn_up", d_ff, d_model)
            emit_ternary(f"blocks.{bi}.experts.0.down", f"block{bi}_down", "ffn_down", d_model, d_ff)
            layers.append(
                {
                    "index": bi,
                    "kind": "prenorm_residual_ffn",
                    "up_ternary": f"block{bi}_up.ternary",
                    "up_scales": f"block{bi}_up.scales",
                    "down_ternary": f"block{bi}_down.ternary",
                    "down_scales": f"block{bi}_down.scales",
                    "up_shape": [d_ff, d_model],
                    "down_shape": [d_model, d_ff],
                }
            )

    schema = BRIDGE_SCHEMA_MOE if is_moe else BRIDGE_SCHEMA_DENSE
    family = (
        "linear_state_multi_timescale_then_top1_moe_ffn"
        if is_moe
        else "linear_state_multi_timescale_then_dense_ffn"
    )
    manifest = {
        "schema": schema,
        "source_format": HARDENED_FORMAT,
        "source_meta": man.get("meta", {}),
        "topology": {
            "family": family,
            "moe": is_moe,
            "n_experts_per_block": n_experts,
            "d_model": d_model,
            "d_ff": d_ff,
            "n_blocks": n_blocks,
            "vocab": vocab,
            "tied_head": True,
            "sequence_state_kind": "linear_state_multi_timescale",
            "sequence_state_params": {
                "state_slots": state_slots,
                "decay_policy": "MultiTimescale",
                "decay_raws_by_band": decay_raws,
                "band_layout": "state_slots partitioned into len(decay_raws) equal contiguous bands; slot s uses decay_raws[s // (state_slots/len)]",
            },
        },
        "router_semantics": {
            "kind": "top1_lowrank_router.rs_Top1RouterQat",
            "note": (
                "hidden = input_projection @ x + input_bias; "
                "raw = expert_projection @ hidden + expert_bias; "
                "expert = argmax(raw) (lowest-index tiebreak); "
                "input x is the RAW pre-norm residual (NOT the fake-quant activation); "
                "all router weights are f32 (router is not ternarized). "
                "This is the deployed gbf-model router, NOT s8's simplified flat [E,d_model] router."
            ),
        }
        if is_moe
        else None,
        "numeric_convention": {
            "weight_encoding": "Ternary {-1,0,+1}",
            "weight_scale": "per_output_row Q8.8 (u16 raw, f32 = raw/256)",
            "embedding_dtype": "f32_le",
            "activation_fake_quant": man.get("act_fake_quant"),
            "block_forward": "x' = x + Down(gelu(Up(actq(rms_norm(x))))); logits = rms_norm(x_final) @ embedding^T",
        },
        "layers": layers,
        "tensors": index,
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    return out

"""Tests for the MLX hardened -> Rust artifact export bridge (bd-3mzda).

These prove the bridge is a faithful, byte-exact re-serialization of the
hardened student: every ternary/scale/router/embedding tensor reaches disk in
the little-endian, row-major layout the Rust loader expects, with a matching
per-tensor sha256, and the MoE router carries the full low-rank
``router.rs`` form (not s8's flat router).
"""

from __future__ import annotations

import hashlib
import json
import struct
from pathlib import Path

import numpy as np
import pytest

from gbtrain.bridge import (
    BRIDGE_SCHEMA_DENSE,
    BRIDGE_SCHEMA_MOE,
    _read_safetensors,
    bridge_hardened_export,
)
from gbtrain.export import export_hardened
from gbtrain.model import GBModel, ModelConfig


def _tiny_hardened(tmp_path: Path, n_experts: int, vocab: int = 48) -> Path:
    """Build a tiny model, harden it, return the hardened dir."""
    cfg = ModelConfig(
        d_model=16,
        d_ff=32,
        n_blocks=2,
        state_slots=16,
        n_experts=n_experts,
        vocab=vocab,
        qat_weights=True,
        qat_acts=True,
    )
    model = GBModel(cfg)
    import mlx.core as mx

    mx.eval(model.parameters())
    hardened = tmp_path / "hardened"
    export_hardened(model, hardened)
    return hardened


def _read_bin(export: Path, rel: str) -> bytes:
    return (export / rel).read_bytes()


def test_moe_bridge_produces_expected_schema_and_files(tmp_path: Path) -> None:
    hardened = _tiny_hardened(tmp_path, n_experts=4)
    out = bridge_hardened_export(hardened, tmp_path / "rust_ckpt")
    man = json.loads((out / "manifest.json").read_text())

    assert man["schema"] == BRIDGE_SCHEMA_MOE
    assert man["topology"]["moe"] is True
    assert man["topology"]["n_experts_per_block"] == 4
    assert man["router_semantics"]["kind"].startswith("top1_lowrank")

    # Every tensor listed exists, and its sha256 matches the file bytes.
    for t in man["tensors"]:
        payload = _read_bin(out, t["file"])
        assert hashlib.sha256(payload).hexdigest() == t["sha256"], t["name"]

    # Expected per-block router + expert tensor names are present.
    names = {t["name"] for t in man["tensors"]}
    for bi in range(2):
        assert f"block{bi}_router_input_projection" in names
        assert f"block{bi}_router_input_bias" in names
        assert f"block{bi}_router_expert_projection" in names
        assert f"block{bi}_router_expert_bias" in names
        for ei in range(4):
            assert f"block{bi}_expert{ei}_up.ternary" in names
            assert f"block{bi}_expert{ei}_up.scales" in names
            assert f"block{bi}_expert{ei}_down.ternary" in names
            assert f"block{bi}_expert{ei}_down.scales" in names


def test_dense_bridge_uses_s5_schema(tmp_path: Path) -> None:
    hardened = _tiny_hardened(tmp_path, n_experts=1)
    out = bridge_hardened_export(hardened, tmp_path / "rust_ckpt")
    man = json.loads((out / "manifest.json").read_text())
    assert man["schema"] == BRIDGE_SCHEMA_DENSE
    assert man["topology"]["moe"] is False
    assert man["router_semantics"] is None
    names = {t["name"] for t in man["tensors"]}
    assert "block0_up.ternary" in names
    assert "block0_down.scales" in names
    assert not any("router" in n for n in names)


def test_bridged_tensors_byte_match_hardened_source(tmp_path: Path) -> None:
    """The bridge must not alter tensor values -- ternary i8, u16 scales, and
    f32 embedding/router bytes must round-trip from the hardened safetensors."""
    hardened = _tiny_hardened(tmp_path, n_experts=4)
    src = _read_safetensors(hardened / "hardened.safetensors")
    out = bridge_hardened_export(hardened, tmp_path / "rust_ckpt")

    # Embedding f32 bytes identical.
    emb_disk = np.frombuffer(_read_bin(out, "tensors/embedding.f32.bin"), dtype="<f4")
    np.testing.assert_array_equal(emb_disk, np.ascontiguousarray(src["embedding"], dtype="<f4").ravel())

    # A ternary matrix: i8 with -1 -> 0xFF.
    w = np.ascontiguousarray(src["state_block.state_in.w_tern"]).astype(np.int8)
    disk = np.frombuffer(_read_bin(out, "tensors/state_input_to_state.ternary.i8.bin"), dtype=np.int8)
    np.testing.assert_array_equal(disk, w.ravel())
    assert set(np.unique(disk).tolist()) <= {-1, 0, 1}

    # Scales: u16 LE.
    s = np.ascontiguousarray(src["state_block.state_in.scale_raw"]).astype(np.uint16)
    disk_s = np.frombuffer(_read_bin(out, "tensors/state_input_to_state.scales.q8_8_u16le.bin"), dtype="<u2")
    np.testing.assert_array_equal(disk_s, s.ravel())

    # Router low-rank tensor bytes identical.
    rip = np.ascontiguousarray(src["blocks.0.router.input_projection"], dtype="<f4").ravel()
    disk_r = np.frombuffer(_read_bin(out, "tensors/block0_router_input_projection.f32.bin"), dtype="<f4")
    np.testing.assert_array_equal(disk_r, rip)


def test_decay_expands_to_per_slot(tmp_path: Path) -> None:
    hardened = _tiny_hardened(tmp_path, n_experts=4)
    man_h = json.loads((hardened / "manifest.json").read_text())
    raws = man_h["decay_raws"]
    slots = man_h["topology"]["state_slots"]
    out = bridge_hardened_export(hardened, tmp_path / "rust_ckpt")
    decay = np.frombuffer(_read_bin(out, "tensors/state_decay.q8_8_u16le.bin"), dtype="<u2")
    assert decay.shape[0] == slots
    # 4 equal contiguous bands.
    per = slots // len(raws)
    for band, raw in enumerate(raws):
        assert np.all(decay[band * per : (band + 1) * per] == raw)


def test_safetensors_reader_matches_shapes(tmp_path: Path) -> None:
    hardened = _tiny_hardened(tmp_path, n_experts=4)
    t = _read_safetensors(hardened / "hardened.safetensors")
    assert t["embedding"].shape == (48, 16)
    assert t["state_block.state_in.w_tern"].dtype == np.int8
    assert t["state_block.state_in.scale_raw"].dtype == np.uint16


def test_rejects_wrong_source_format(tmp_path: Path) -> None:
    bad = tmp_path / "bad"
    bad.mkdir()
    (bad / "manifest.json").write_text(json.dumps({"format": "not_hardened"}))
    with pytest.raises(ValueError, match="unexpected hardened format"):
        bridge_hardened_export(bad, tmp_path / "out")

#!/usr/bin/env python3
"""Generate the deterministic F-A5 M0 8x8 mono font asset.

The output uses Game Boy 2bpp tile format. Each row writes the same bit pattern
into both planes so the glyph renders as a simple monochrome tile.
"""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "gbf-runtime" / "assets" / "font_8x8.bin"
FONT_TILE_COUNT = 128
BYTES_PER_TILE = 16


def glyph_rows(glyph: int) -> list[int]:
    rows: list[int] = []
    for row in range(8):
        # Deterministic, compact bring-up pattern: visible ASCII gets a boxy
        # non-empty tile keyed by the glyph code; control bytes and space stay blank.
        if glyph < 0x20 or glyph == 0x20:
            bits = 0
        elif row == 0 or row == 7:
            bits = 0b0111_1110
        else:
            interior = (glyph << row) & 0b0011_1100
            bits = 0b0100_0010 | interior
        rows.append(bits & 0xFF)
    return rows


def build() -> bytes:
    out = bytearray()
    for glyph in range(FONT_TILE_COUNT):
        for row in glyph_rows(glyph):
            out.append(row)
            out.append(row)
    assert len(out) == FONT_TILE_COUNT * BYTES_PER_TILE
    return bytes(out)


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(build())


if __name__ == "__main__":
    main()

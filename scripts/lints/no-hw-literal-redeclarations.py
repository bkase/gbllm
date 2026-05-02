#!/usr/bin/env python3
"""Conservative F-A2 hardware-literal redeclaration audit.

This is a promotion-ready helper for the ignored gbf-hw single-source smoke
test. It scans Rust sources outside gbf-hw for hardware-address and header
literals that should normally be imported from gbf-hw.
"""

from __future__ import annotations

import argparse
from pathlib import Path


NEEDLES = (
    "0xC000",
    "0xFF80",
    "0xA000",
    "0xFFFF",
    "0x4000",
    "0x0A",
    "0x0040",
    "0x0048",
    "0x0050",
    "0x0058",
    "0x0060",
    "0xFF00",
    "0xFF0F",
    "0xFF04",
    "0xFF05",
    "0xFF06",
    "0xFF07",
    "0xFF40",
    "0xFF41",
    "0xFF42",
    "0xFF43",
    "0xFF44",
    "0xFF45",
    "0xFF46",
    "0xFF47",
    "0xFF48",
    "0xFF49",
    "0xFF4A",
    "0xFF4B",
    "0x0148",
    "0x0149",
    "17556",
    "1140",
    "0xCE",
)

ALLOWLIST = {
    "gbf-asm/src/isa.rs": "Instruction/addressing tests contain literal operands, not canonical memory-map declarations.",
    "gbf-asm/src/layout.rs": "F-A1 layout owns half-open placement constants by design.",
    "gbf-asm/src/listing.rs": "Listing tests contain illustrative operands.",
    "gbf-asm/src/relax.rs": "Relaxation tests use ROMX fixture addresses.",
    "gbf-asm/src/rom.rs": "F-A1 ROM tests exercise header offsets and ROMX fixture addresses.",
    "gbf-asm/src/section.rs": "Section tests use arbitrary byte fixtures that overlap the Nintendo logo prefix.",
    "gbf-asm/src/symbols.rs": "Symbol tests use fixture addresses.",
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()

    root = args.root.resolve()
    violations: list[str] = []
    for path in sorted(root.glob("gbf-*/src/**/*.rs")):
        rel = path.relative_to(root).as_posix()
        if rel.startswith("gbf-hw/") or rel in ALLOWLIST:
            continue
        for line_no, line in enumerate(production_lines(path), start=1):
            if any(needle in line for needle in NEEDLES):
                violations.append(f"{rel}:{line_no}: {line.strip()}")

    if violations:
        print("hardware literals must be sourced from gbf-hw:")
        print("\n".join(violations))
        return 1
    return 0


def production_lines(path: Path) -> list[str]:
    lines = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.lstrip().startswith("#[cfg(test)]"):
            break
        lines.append(line)
    return lines


if __name__ == "__main__":
    raise SystemExit(main())

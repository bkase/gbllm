#!/usr/bin/env python3
"""Apply the F-S4 §D2 book-level train/val split.

Per-book independence: split assignment for any retained id is a pure
function of (split_seed_bytes, book_id), so adding/removing other ids
never changes anyone else's bucket.

Inputs:
  corpus/gutenberg/book_ids.json  (the §D1 selection)

Outputs:
  corpus/gutenberg/splits.json    {train: [...ids...], val: [...ids...]}
  corpus/gutenberg/splits.log     summary counters
"""
from __future__ import annotations

import hashlib
import json
import struct
import sys
from pathlib import Path

SEED_LABEL = b"gbf:s4:book-split:2026-05-09"
SPLIT_PREFIX = b"gbf:s4:book-split:v1"
TRAIN_FRACTION = 0.90


def split_seed_bytes() -> bytes:
    return hashlib.sha256(SEED_LABEL).digest()[:16]


def split_u(seed16: bytes, book_id: int) -> float:
    h = hashlib.sha256(SPLIT_PREFIX + seed16 + struct.pack("<I", book_id)).digest()
    # high 53 bits of digest as f64 in [0, 1)
    top64 = int.from_bytes(h[:8], "big")
    x = top64 >> 11
    return x / (1 << 53)


def main() -> int:
    here = Path(__file__).resolve()
    repo = here.parents[3]
    book_ids_path = repo / "corpus/gutenberg/book_ids.json"
    out_path = repo / "corpus/gutenberg/splits.json"
    log_path = repo / "corpus/gutenberg/splits.log"

    selection = json.loads(book_ids_path.read_text())
    book_ids: list[int] = selection["book_ids"]
    seed16 = split_seed_bytes()

    train, val = [], []
    for bid in book_ids:
        u = split_u(seed16, bid)
        (train if u < TRAIN_FRACTION else val).append(bid)
    train.sort()
    val.sort()

    splits = {
        "schema_hint": "F-S4 §D2 book-level split output",
        "seed_label_ascii": SEED_LABEL.decode("ascii"),
        "split_seed_bytes_hex": seed16.hex(),
        "train_fraction": TRAIN_FRACTION,
        "train_count": len(train),
        "val_count": len(val),
        "train": train,
        "val": val,
        "self_hash_sha256": hashlib.sha256(
            (",".join(str(i) for i in train) + "|" + ",".join(str(i) for i in val)).encode("ascii")
        ).hexdigest(),
    }
    out_path.write_text(json.dumps(splits, indent=2) + "\n")

    lines = [
        f"book_ids_total       {len(book_ids)}",
        f"train_count          {len(train)}",
        f"val_count            {len(val)}",
        f"train_fraction_seen  {len(train) / len(book_ids):.4f}",
        f"split_seed_hex       {seed16.hex()}",
        f"self_hash_sha256     {splits['self_hash_sha256']}",
    ]
    log_path.write_text("\n".join(lines) + "\n")
    for line in lines:
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())

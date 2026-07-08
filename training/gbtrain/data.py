"""TinyStories -> packed token dataset + deterministic BPTT batching (MLX).

Tokenizes raw text with the byte-level BPE artifact into a flat ``uint16``
token stream (vocab <= 65536), saved alongside a small meta file. Batching
mirrors the truncated-BPTT-with-detached-state design the LinearState model
uses: the stream is split into ``lanes`` contiguous segments and iterated in
fixed-size ``seq_len`` windows, so each lane carries its own recurrent state
across steps. Deterministic given the inputs; no hidden shuffling.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from .tokenizer import BPEModel, _read_capped_text


def tokenize_text(model: BPEModel, text: str) -> np.ndarray:
    ids = model.encode(text)
    assert model.vocab_size <= 65536, "uint16 stream requires vocab <= 65536"
    return np.asarray(ids, dtype=np.uint16)


def tokenize_file(model: BPEModel, path: str | Path, byte_cap: int | None) -> np.ndarray:
    # boundary-safe read: a byte cap never slices a multibyte UTF-8 char
    text = _read_capped_text(path, byte_cap)
    return tokenize_text(model, text)


@dataclass
class Dataset:
    train: np.ndarray  # uint16
    val: np.ndarray  # uint16
    vocab_size: int

    def save(self, out_dir: str | Path) -> None:
        out = Path(out_dir)
        out.mkdir(parents=True, exist_ok=True)
        np.save(out / "train.npy", self.train)
        np.save(out / "val.npy", self.val)
        (out / "meta.json").write_text(
            json.dumps(
                {
                    "vocab_size": self.vocab_size,
                    "train_tokens": int(self.train.size),
                    "val_tokens": int(self.val.size),
                    "dtype": "uint16",
                },
                indent=2,
            )
        )

    @classmethod
    def load(cls, out_dir: str | Path) -> "Dataset":
        out = Path(out_dir)
        meta = json.loads((out / "meta.json").read_text())
        return cls(
            train=np.load(out / "train.npy"),
            val=np.load(out / "val.npy"),
            vocab_size=meta["vocab_size"],
        )


def build_dataset(
    model: BPEModel,
    train_path: str | Path,
    val_path: str | Path,
    train_byte_cap: int | None,
    val_byte_cap: int | None,
) -> Dataset:
    return Dataset(
        train=tokenize_file(model, train_path, train_byte_cap),
        val=tokenize_file(model, val_path, val_byte_cap),
        vocab_size=model.vocab_size,
    )


def iter_bptt_batches(tokens: np.ndarray, seq_len: int, lanes: int):
    """Yield ``(x, y)`` int arrays of shape ``(lanes, seq_len)``.

    The stream is reshaped into ``lanes`` contiguous rows; consecutive yields
    advance every lane by ``seq_len`` with the +1-shifted targets, so a model
    that carries recurrent state between yields sees each lane as one long
    continuation. Deterministic; drops the ragged tail.
    """
    if seq_len < 1 or lanes < 1:
        raise ValueError("seq_len and lanes must be >= 1")
    per_lane = tokens.size // lanes
    if per_lane < seq_len + 1:
        raise ValueError(
            f"stream too short: {tokens.size} tokens, {lanes} lanes -> "
            f"{per_lane}/lane < seq_len+1={seq_len + 1}"
        )
    grid = tokens[: lanes * per_lane].reshape(lanes, per_lane).astype(np.int32)
    n_steps = (per_lane - 1) // seq_len
    for t in range(n_steps):
        lo = t * seq_len
        x = grid[:, lo : lo + seq_len]
        y = grid[:, lo + 1 : lo + seq_len + 1]
        yield x, y


def _main() -> None:
    import argparse
    import time

    ap = argparse.ArgumentParser(description="Build a packed token dataset")
    ap.add_argument("--vocab", required=True)
    ap.add_argument("--train", required=True)
    ap.add_argument("--val", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--train-mb", type=float, default=200.0)
    ap.add_argument("--val-mb", type=float, default=0.0, help="0 = whole val file")
    args = ap.parse_args()

    model = BPEModel.load(args.vocab)
    tcap = None if args.train_mb == 0 else int(args.train_mb * 1024 * 1024)
    vcap = None if args.val_mb == 0 else int(args.val_mb * 1024 * 1024)
    t0 = time.time()
    ds = build_dataset(model, args.train, args.val, tcap, vcap)
    ds.save(args.out)
    print(
        f"train {ds.train.size:,} tok  val {ds.val.size:,} tok  "
        f"vocab {ds.vocab_size}  in {time.time() - t0:.1f}s -> {args.out}"
    )


if __name__ == "__main__":
    _main()

"""Byte-level BPE tokenizer for the Game Boy LLM subword vocabulary.

Design goals (foundation for the subword rewrite, epic bd-wfghf):

* **Lossless & total.** Byte-level: every token decodes to raw bytes and
  ``decode(encode(text)) == text`` for *any* input. The pre-tokenizer regex is
  **total** — every character lands in exactly one chunk (verified by a fuzz
  test), so nothing can be silently dropped (an earlier pattern dropped ``_``).
* **Deterministic & specified.** Training and encoding are pure functions of
  the input. The canonical encode algorithm is: *repeatedly find the pair with
  the minimum merge rank; merge the leftmost occurrence; repeat until no pair
  has a rank.* Ties on pair frequency during training break by the pair value.
* **Cross-language parity by conformance vectors, not regex strings.** The
  same vocabulary is reimplemented in Rust for on-device use. Rust's ``regex``
  crate has no look-around, so we (a) keep the pattern look-around-free and
  (b) ship a frozen ``text -> token-ids`` conformance corpus that BOTH the
  Python and Rust encoders must reproduce byte-exactly. The pattern string in
  the artifact is informative; the conformance vectors are authoritative.
* **Portable artifact.** ``to_json`` emits vocab + ordered merges + explicit
  id->bytes + ``max_token_len`` (for the on-device render buffer).
"""

from __future__ import annotations

import json
import re
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

# Total, look-around-free pre-tokenizer (Python ``re`` and Rust ``regex`` both
# compile this identically — no ``(?!...)`` look-ahead). Alternatives, in
# order: optional-leading-space letter run, digit run, punctuation/symbol run
# (excludes ``_`` because ``_`` is a ``\w`` char), underscore run, whitespace
# run, and a DOTALL catch-all ``.`` guaranteeing totality for any leftover
# code point. ``(?s)`` makes ``.`` match newlines too.
_PATTERN = r"(?s) ?[^\W\d_]+| ?\d+| ?[^\s\w]+| ?_+|\s+|."
_RE = re.compile(_PATTERN)

BASE_VOCAB = 256


def pretokenize(text: str, pattern: str = _PATTERN) -> list[str]:
    """Split text into pre-token chunks. Total: ``''.join(...) == text``."""
    return re.compile(pattern).findall(text)


@dataclass
class BPEModel:
    """A trained byte-level BPE model.

    ``merges`` is the ordered list of ``(a, b)`` token-id pairs; merge ``i``
    creates token id ``BASE_VOCAB + i``. ``vocab`` maps every token id to its
    raw ``bytes``.
    """

    merges: list[tuple[int, int]]
    pattern: str = _PATTERN

    def __post_init__(self) -> None:
        self._re = re.compile(self.pattern)
        self.vocab: dict[int, bytes] = {i: bytes([i]) for i in range(BASE_VOCAB)}
        for i, (a, b) in enumerate(self.merges):
            if a >= BASE_VOCAB + i or b >= BASE_VOCAB + i:
                raise ValueError(f"merge {i} references undefined id: ({a},{b})")
            self.vocab[BASE_VOCAB + i] = self.vocab[a] + self.vocab[b]
        self._rank: dict[tuple[int, int], int] = {
            pair: i for i, pair in enumerate(self.merges)
        }

    @property
    def vocab_size(self) -> int:
        return BASE_VOCAB + len(self.merges)

    @property
    def max_token_len(self) -> int:
        return max((len(b) for b in self.vocab.values()), default=1)

    # -- encode / decode --------------------------------------------------

    def _merge_chunk(self, ids: list[int]) -> list[int]:
        """Canonical BPE: lowest-rank pair first, leftmost occurrence."""
        if len(ids) < 2:
            return ids
        while True:
            best_rank = None
            best_pos = -1
            for pos in range(len(ids) - 1):
                rank = self._rank.get((ids[pos], ids[pos + 1]))
                if rank is not None and (best_rank is None or rank < best_rank):
                    best_rank = rank
                    best_pos = pos
            if best_rank is None:
                return ids
            new_id = BASE_VOCAB + best_rank
            ids = ids[:best_pos] + [new_id] + ids[best_pos + 2 :]

    def encode(self, text: str) -> list[int]:
        out: list[int] = []
        for chunk in self._re.findall(text):
            out.extend(self._merge_chunk(list(chunk.encode("utf-8"))))
        return out

    def decode(self, ids: list[int]) -> str:
        """Decode ids to text. ``errors='replace'`` is only lossless on
        complete, encoder-produced streams; on-device rendering must buffer a
        trailing incomplete multibyte char rather than emit U+FFFD."""
        buf = b"".join(self.vocab[i] for i in ids)
        return buf.decode("utf-8", errors="replace")

    def token_bytes(self, token_id: int) -> bytes:
        return self.vocab[token_id]

    # -- serialization ----------------------------------------------------

    def to_json(self) -> str:
        return json.dumps(
            {
                "format": "gbllm_bpe.v2",
                "base_vocab": BASE_VOCAB,
                "vocab_size": self.vocab_size,
                "max_token_len": self.max_token_len,
                "pattern": self.pattern,
                "pattern_note": "informative; parity is gated by conformance vectors, not this string",
                "merges": [[a, b] for a, b in self.merges],
                "id_bytes_hex": {
                    str(i): self.vocab[i].hex() for i in range(self.vocab_size)
                },
            },
            separators=(",", ":"),
        )

    def save(self, path: str | Path) -> None:
        Path(path).write_text(self.to_json())

    @classmethod
    def from_json(cls, text: str) -> "BPEModel":
        obj = json.loads(text)
        merges = [(a, b) for a, b in obj["merges"]]
        model = cls(merges=merges, pattern=obj["pattern"])
        # strict integrity: base_vocab, vocab_size, and a COMPLETE id->bytes map
        if obj.get("base_vocab") != BASE_VOCAB:
            raise ValueError(f"base_vocab mismatch: {obj.get('base_vocab')} != {BASE_VOCAB}")
        if obj.get("vocab_size") != model.vocab_size:
            raise ValueError(
                f"vocab_size mismatch: {obj.get('vocab_size')} != {model.vocab_size}"
            )
        table = obj.get("id_bytes_hex")
        if table is None:
            raise ValueError("artifact missing required id_bytes_hex table")
        if len(table) != model.vocab_size:
            raise ValueError(
                f"id_bytes_hex has {len(table)} entries, expected {model.vocab_size}"
            )
        for i in range(model.vocab_size):
            expect = bytes.fromhex(table[str(i)])
            if model.vocab[i] != expect:
                raise ValueError(
                    f"vocab mismatch at id {i}: artifact {expect!r} != replay {model.vocab[i]!r}"
                )
        return model

    @classmethod
    def load(cls, path: str | Path) -> "BPEModel":
        return cls.from_json(Path(path).read_text())


def train_bpe(text: str, vocab_size: int, pattern: str = _PATTERN) -> BPEModel:
    """Train byte-level BPE to ``vocab_size`` total tokens (>= 256).

    Operates on unique pre-token chunks weighted by frequency; ties on pair
    frequency break by the pair value for determinism.
    """
    if vocab_size < BASE_VOCAB:
        raise ValueError(f"vocab_size must be >= {BASE_VOCAB}")
    rx = re.compile(pattern)
    words: Counter[tuple[int, ...]] = Counter()
    for chunk in rx.findall(text):
        words[tuple(chunk.encode("utf-8"))] += 1

    seqs: list[list] = [[list(ids), freq] for ids, freq in words.items()]

    merges: list[tuple[int, int]] = []
    for step in range(vocab_size - BASE_VOCAB):
        pair_counts: Counter[tuple[int, int]] = Counter()
        for ids, freq in seqs:
            for a, b in zip(ids, ids[1:]):
                pair_counts[(a, b)] += freq
        if not pair_counts:
            break
        best = max(pair_counts.items(), key=lambda kv: (kv[1], kv[0]))[0]
        new_id = BASE_VOCAB + step
        merges.append(best)
        a, b = best
        for entry in seqs:
            ids = entry[0]
            if len(ids) < 2:
                continue
            merged: list[int] = []
            i = 0
            while i < len(ids):
                if i < len(ids) - 1 and ids[i] == a and ids[i + 1] == b:
                    merged.append(new_id)
                    i += 2
                else:
                    merged.append(ids[i])
                    i += 1
            entry[0] = merged

    return BPEModel(merges=merges)


# -- CLI ------------------------------------------------------------------


def _read_capped_text(path: str | Path, byte_cap: int | None) -> str:
    """Read up to ``byte_cap`` bytes, trimmed back to a UTF-8 char boundary so
    a capped slice never cuts a multibyte char (which would inject U+FFFD)."""
    raw = Path(path).read_bytes()
    if byte_cap is not None and byte_cap > 0 and byte_cap < len(raw):
        end = byte_cap
        # back off any UTF-8 continuation bytes (0x80..0xBF) plus their lead
        while end > 0 and (raw[end] & 0xC0) == 0x80:
            end -= 1
        raw = raw[:end]
    return raw.decode("utf-8", errors="replace")


def _main() -> None:
    import argparse

    ap = argparse.ArgumentParser(description="Train a byte-level BPE vocabulary")
    ap.add_argument("--input", required=True)
    ap.add_argument("--vocab-size", type=int, default=1024)
    ap.add_argument("--sample-mb", type=float, default=40.0)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    n = None if args.sample_mb == 0 else int(args.sample_mb * 1024 * 1024)
    text = _read_capped_text(args.input, n)
    model = train_bpe(text, args.vocab_size)
    model.save(args.out)
    sample = text[:200_000]
    ids = model.encode(sample)
    cpt = len(sample.encode("utf-8")) / max(1, len(ids))
    print(
        f"trained vocab_size={model.vocab_size} merges={len(model.merges)} "
        f"max_token_len={model.max_token_len} -> {cpt:.2f} bytes/token on a "
        f"200k-char sample; wrote {args.out}"
    )


if __name__ == "__main__":
    _main()

"""Frozen text -> token-ids conformance vectors.

The subword vocabulary is reimplemented in Rust for on-device use. Rust's
``regex`` crate has no look-around and its Unicode tables may differ, so we do
NOT trust the pattern string to be portable. Instead this module freezes a set
of ``(text, ids)`` vectors that BOTH the Python encoder (here) and the Rust
encoder must reproduce byte-exactly. Any divergence fails a test on each side,
turning a silent inference-corruption bug into a loud one.
"""

from __future__ import annotations

import json
from pathlib import Path

from .tokenizer import BPEModel, pretokenize

# Diverse, adversarial-ish coverage: the classes the pre-tokenizer must keep
# total and the encode cases a naive Rust port is most likely to get wrong.
CONFORMANCE_TEXTS: list[str] = [
    "",
    "a",
    "the machines dreamed of electric sheep",
    "Once upon a time, there was a little robot.",
    "snake_case and __init__ and a_b_c",  # underscore (the dropped-char bug)
    "CamelCase mixedWith123numbers",
    "digits 0 1 2 3 42 1000 3.14",
    "punctuation!?; (parens) [brackets] {braces} 'quotes' \"double\"",
    "   leading   and   trailing   spaces   ",  # whitespace-run splitting
    "tabs\tand\nnewlines\r\nand\tmore",
    "don't can't it's I'll we've they're",  # contractions
    "unicode: café résumé naïve — em-dash “quotes”",
    "emoji \U0001f916\U0001f3ae survive as bytes",
    "path/like/thing.ext and http://x.y/z?q=1",
    "MiXeD_case/with.every123 kind! of\tboundary\n",
]


def build_vectors(model: BPEModel) -> list[dict]:
    return [{"text": t, "ids": model.encode(t)} for t in CONFORMANCE_TEXTS]


def save_vectors(model: BPEModel, path: str | Path) -> None:
    Path(path).write_text(
        json.dumps(
            {
                "format": "gbllm_bpe_conformance.v1",
                "vocab_size": model.vocab_size,
                "vectors": build_vectors(model),
            },
            ensure_ascii=True,
            indent=1,
        )
    )


def verify_vectors(model: BPEModel, path: str | Path) -> None:
    """Raise if the model does not reproduce every frozen vector, or if any
    text is not total under the pre-tokenizer (a dropped char)."""
    obj = json.loads(Path(path).read_text())
    for v in obj["vectors"]:
        text = v["text"]
        joined = "".join(pretokenize(text))
        if joined != text:
            raise ValueError(f"pretokenizer not total on {text!r}: dropped chars")
        ids = model.encode(text)
        if ids != v["ids"]:
            raise ValueError(f"encode mismatch on {text!r}: {ids} != {v['ids']}")
        if model.decode(ids) != text:
            raise ValueError(f"round-trip mismatch on {text!r}")


def _main() -> None:
    import argparse

    ap = argparse.ArgumentParser(description="Emit/verify BPE conformance vectors")
    ap.add_argument("--vocab", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    model = BPEModel.load(args.vocab)
    save_vectors(model, args.out)
    verify_vectors(model, args.out)
    print(f"wrote + verified {len(CONFORMANCE_TEXTS)} conformance vectors -> {args.out}")


if __name__ == "__main__":
    _main()

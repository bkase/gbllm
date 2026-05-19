#!/usr/bin/env python3
"""Contamination preview, direction 2: GB_train ∋ TS_val.

Mirrors scripts/corpus/gutenberg/contamination_preview.py but with the
roles swapped:
  - sample 100 random 200-char windows from TinyStories val
  - search verbatim presence in concatenated Gutenberg train bodies

This is the second §D6 closure-gated direction. Like direction 1, it is
a *smoke test*, not the production check.
"""
from __future__ import annotations

import hashlib
import json
import mmap
import random
import sys
from pathlib import Path

WINDOW_CHARS = 200
N_SAMPLES = 100
SEED = 0xC0DEBEEF


def sample_tinystories_windows(repo: Path) -> list[tuple[int, str]]:
    """Sample N_SAMPLES (offset, window) tuples deterministically from
    TinyStories val (single 22 MB file)."""
    ts_val = repo / "corpus/tinystories/raw/TinyStoriesV2-GPT4-valid.txt"
    text = ts_val.read_text(encoding="utf-8", errors="replace")
    rng = random.Random(SEED)
    samples: list[tuple[int, str]] = []
    attempts = 0
    while len(samples) < N_SAMPLES and attempts < N_SAMPLES * 4:
        attempts += 1
        if len(text) < WINDOW_CHARS + 100:
            break
        start = rng.randint(50, len(text) - WINDOW_CHARS - 50)
        window = text[start : start + WINDOW_CHARS]
        if len(window.split()) < 20:
            continue
        samples.append((start, window))
    return samples


def gutenberg_train_haystack_path(repo: Path) -> Path:
    """Concatenate gutenberg train bodies into one temp file so mmap.find
    has a single haystack to scan. The output is gitignored under
    corpus/gutenberg/."""
    out = repo / "corpus/gutenberg/gutenberg_train_concatenated.bin"
    splits = json.loads((repo / "corpus/gutenberg/splits.json").read_text())
    train_ids = splits["train"]
    if not out.exists():
        print(f"  building concatenated train haystack ({len(train_ids)} books) ...", file=sys.stderr)
        with out.open("wb") as f:
            for bid in train_ids:
                body = repo / "corpus/gutenberg/bodies" / str(bid) / "body.txt"
                if body.exists():
                    f.write(body.read_bytes())
                    f.write(b"\n\x00\n")  # boundary marker so 200-char windows don't cross books
        print(f"  wrote {out} ({out.stat().st_size:,} bytes)", file=sys.stderr)
    return out


def main() -> int:
    repo = Path(__file__).resolve().parents[3]
    haystack = gutenberg_train_haystack_path(repo)

    print(f"sampling {N_SAMPLES} windows of {WINDOW_CHARS} chars from TinyStories val ...",
          file=sys.stderr)
    samples = sample_tinystories_windows(repo)

    print(f"mmap.find against gutenberg train ({haystack.stat().st_size:,} bytes) ...",
          file=sys.stderr)
    hits: list[dict] = []
    with open(haystack, "rb") as f:
        with mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ) as m:
            for i, (off, window) in enumerate(samples):
                if m.find(window.encode("utf-8")) >= 0:
                    hits.append({
                        "ts_val_offset": off,
                        "window_first_80": window[:80].replace("\n", " "),
                    })
                if (i + 1) % 25 == 0:
                    print(f"  ... {i + 1}/{len(samples)} patterns; {len(hits)} hits", file=sys.stderr, flush=True)

    docs = repo / "docs/experiments/S4-contamination-preview-dir2.md"
    md = [
        "# F-S4 Cross-Corpus Contamination Preview — Direction 2 (smoke test)",
        "",
        "Mirror of `S4-contamination-preview.md` with the §D6 directions",
        "swapped. Samples 200-character windows from **TinyStories val** and",
        "looks for verbatim appearance in **Gutenberg train** (concatenated",
        "stripped bodies, book boundary markers preserved so windows do not",
        "span books).",
        "",
        "## Setup",
        "",
        f"- TinyStories val source: `corpus/tinystories/raw/TinyStoriesV2-GPT4-valid.txt` (22 MB)",
        f"- Gutenberg train haystack: `{haystack.relative_to(repo)}` ({haystack.stat().st_size:,} bytes)",
        f"- Sampler seed: 0x{SEED:X}",
        f"- Sample count: {len(samples)}",
        f"- Tool: mmap.find (single C-level memmem per pattern)",
        "",
        "## Result",
        "",
        f"- **Windows with verbatim match in Gutenberg train: {len(hits)}**",
        f"- **Match rate: {100 * len(hits) / max(1, len(samples)):.3f}%**",
        "",
    ]
    if not hits:
        md.extend([
            "## Interpretation",
            "",
            "**Clean.** No 200-char window from sampled TinyStories val text",
            "appears verbatim in Gutenberg train. Combined with direction 1's",
            "Clean result, both §D6 closure-gated directions look healthy at",
            "smoke-test granularity. F-S4.08's production check should",
            "produce `Clean` or at worst `Warn`.",
            "",
        ])
    else:
        md.extend([
            "## Interpretation",
            "",
            "⚠️ **One or more TinyStories val windows appear in Gutenberg train.**",
            "Investigate before F-S4.08 — this is the less-likely direction",
            "(Gutenberg is public-domain pre-modern text; TinyStories was",
            "generated by GPT-4) so any hit deserves scrutiny.",
            "",
            "### First few hits",
            "",
            "| ts_val_offset | first 80 chars |",
            "|---:|---|",
        ])
        for h in hits[:10]:
            esc = h['window_first_80'].replace('|', '\\|')
            md.append(f"| {h['ts_val_offset']} | `{esc}…` |")
        md.append("")
    md.append(
        f"_Generated by `scripts/corpus/gutenberg/contamination_preview_dir2.py` "
        f"with seed 0x{SEED:X}._"
    )
    docs.write_text("\n".join(md) + "\n")
    print(f"wrote {docs}")
    print(f"summary: {len(hits)} / {len(samples)} windows matched in gutenberg train")
    return 0


if __name__ == "__main__":
    sys.exit(main())

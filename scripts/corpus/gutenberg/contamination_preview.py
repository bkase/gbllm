#!/usr/bin/env python3
"""Fast char-level contamination smoke test between Gutenberg val and
TinyStories train. NOT a substitute for F-S4.08 (which is exact
13-token-id windows with sha256 indexing + byte-window disambiguation).

This is a *smell test* — if it finds zero literal 200-char passages
from Gutenberg val appearing verbatim in TinyStories train, that's a
strong prior that F-S4.08's production check will return Clean. If it
finds matches, F-S4.08 implementer should be cautious about whether
the contamination threshold is right.

Inputs:
  corpus/gutenberg/splits.json
  corpus/gutenberg/bodies/{val_id}/body.txt
  corpus/tinystories/raw/TinyStoriesV2-GPT4-train.txt

Output:
  docs/experiments/S4-contamination-preview.md
"""
from __future__ import annotations

import hashlib
import json
import random
import subprocess
import sys
from pathlib import Path

WINDOW_CHARS = 200
# N=100 keeps the mmap.find scan under ~3 minutes wall time (each find is
# O(haystack_size) ≈ 2s on a 2.1 GB TinyStories train file). A bigger N
# wouldn't change the smoke signal materially — if literal overlap exists
# at any meaningful density, 100 random 200-char windows already have a
# substantial chance of catching it.
N_SAMPLES = 100
SEED = 0xC0DEBEEF  # frozen so this run is reproducible


def sample_windows(repo: Path) -> list[tuple[int, int, str]]:
    """Sample N_SAMPLES (book_id, char_offset, window_text) tuples
    deterministically from Gutenberg val bodies."""
    splits = json.loads((repo / "corpus/gutenberg/splits.json").read_text())
    val_ids: list[int] = splits["val"]

    rng = random.Random(SEED)
    samples: list[tuple[int, int, str]] = []
    attempts = 0
    while len(samples) < N_SAMPLES and attempts < N_SAMPLES * 4:
        attempts += 1
        bid = rng.choice(val_ids)
        body_path = repo / "corpus/gutenberg/bodies" / str(bid) / "body.txt"
        if not body_path.exists():
            continue
        body = body_path.read_text(encoding="utf-8")
        if len(body) < WINDOW_CHARS + 100:
            continue
        # Avoid first / last 50 chars to dodge any residual banner artifacts
        start = rng.randint(50, len(body) - WINDOW_CHARS - 50)
        window = body[start : start + WINDOW_CHARS]
        # Reject windows that are mostly whitespace / page-break artifacts
        if len(window.split()) < 20:
            continue
        samples.append((bid, start, window))
    return samples


def grep_patterns(patterns: list[str], haystack_path: Path) -> dict[str, int]:
    """mmap-based fixed-string substring search. For each pattern, calls
    bytes.find against the mmap'd haystack. Each find is a single C-level
    memmem call — fast and predictable, no subprocess pipe deadlock risk.

    Returns map: pattern_str -> hit_present (1 if found, 0 if not).
    We only care about presence/absence for the smoke test.
    """
    import mmap
    flat_bytes = [p.replace("\n", " ").encode("utf-8") for p in patterns]
    hits: dict[str, int] = {}
    with open(haystack_path, "rb") as f:
        with mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ) as m:
            for i, pat_b in enumerate(flat_bytes):
                if m.find(pat_b) >= 0:
                    hits[patterns[i].replace("\n", " ")] = 1
                if (i + 1) % 100 == 0:
                    print(f"  ... {i + 1}/{len(flat_bytes)} patterns checked, {len(hits)} hits so far",
                          file=sys.stderr, flush=True)
    return hits


def main() -> int:
    repo = Path(__file__).resolve().parents[3]
    tinystories_path = repo / "corpus/tinystories/raw/TinyStoriesV2-GPT4-train.txt"
    if not tinystories_path.exists():
        print(f"ERROR: TinyStories train not found at {tinystories_path}", file=sys.stderr)
        return 2

    print(f"sampling {N_SAMPLES} windows of {WINDOW_CHARS} chars from Gutenberg val ...", file=sys.stderr)
    samples = sample_windows(repo)
    if len(samples) < N_SAMPLES:
        print(f"WARNING: only {len(samples)} windows sampled", file=sys.stderr)

    flat_windows = [w for (_, _, w) in samples]
    print(f"grepping {len(flat_windows)} windows against TinyStories train ({tinystories_path.stat().st_size:,} bytes) ...", file=sys.stderr)
    hits = grep_patterns(flat_windows, tinystories_path)

    # Build a hit map back to (book_id, offset)
    hit_records: list[dict] = []
    for (bid, off, window) in samples:
        key = window.replace("\n", " ")
        if key in hits:
            hit_records.append({
                "book_id": bid,
                "char_offset_in_body": off,
                "window_first_80": window[:80].replace("\n", " "),
                "tinystories_match_count": hits[key],
            })

    docs_path = repo / "docs/experiments/S4-contamination-preview.md"
    md = [
        "# F-S4 Cross-Corpus Contamination Preview (smoke test)",
        "",
        f"**This is NOT the §D6 production check.** It's a fast char-level smoke",
        f"test: sample N={N_SAMPLES} random {WINDOW_CHARS}-character windows from",
        "Gutenberg val bodies and look for verbatim substring appearance in",
        "TinyStories train. Purpose: derisk F-S4.08 (bd-2p3n) by exposing any",
        "obvious literal overlap early.",
        "",
        "The real check (F-S4.08) operates on charset_v1-tokenized 13-token-id",
        "windows with sha256_high_u64 indexing + exact byte-window",
        "disambiguation. The thresholds for Clean/Warn/HardFail (§D6 lines",
        f"444-452) are 0.10% / 0.05% of unique val 13-grams — much finer-grained",
        "than this preview.",
        "",
        "## Setup",
        "",
        f"- Gutenberg val source: `corpus/gutenberg/bodies/{{val_id}}/body.txt` (155 books)",
        f"- TinyStories train source: `{tinystories_path.relative_to(repo)}` "
        f"({tinystories_path.stat().st_size:,} bytes)",
        f"- Sampler seed: 0x{SEED:X} (frozen for reproducibility)",
        f"- Window size: {WINDOW_CHARS} characters (> the §D6 13-token window;",
        "  any match here would also be a real-overlap candidate)",
        f"- Sample count: {len(samples)} windows actually sampled",
        f"- Tool: `grep -F -f <patterns>` (fgrep multi-pattern, fixed-string)",
        "",
        "## Result",
        "",
        f"- **Windows with verbatim match in TinyStories train: {len(hit_records)}**",
        f"- **Match rate: {100 * len(hit_records) / max(1, len(samples)):.3f}%**",
        "",
    ]
    if not hit_records:
        md.extend([
            "## Interpretation",
            "",
            "**Clean smoke result.** No 200-char window from any sampled Gutenberg",
            "val passage appears verbatim in TinyStories train. This is a strong",
            "prior that F-S4.08's production check will return `Clean` (or at",
            "worst `Warn` — the production check is more sensitive but also",
            "looks at much shorter, charset-normalized windows).",
            "",
            "An F-S4.08 implementer should still run the real check; this",
            "preview cannot detect:",
            "",
            "- Overlap below 200 chars but above 13 charset_v1 tokens.",
            "- Overlap masked by whitespace / capitalization / NFC differences.",
            "- Overlap in non-val Gutenberg splits.",
            "",
            "But a clean smoke is much more likely to be followed by a clean",
            "real check than a noisy one.",
        ])
    else:
        md.extend([
            "## Interpretation",
            "",
            "⚠️ **One or more 200-char Gutenberg val passages appear verbatim in",
            "TinyStories train.** F-S4.08's production check may return `Warn` or",
            "`HardFail`. Investigate before claiming F-S4.08 — the contamination",
            "might be intentional (e.g., a public-domain quotation appears in",
            "both corpora) but the §D6 threshold is unforgiving.",
            "",
            "## First few hits",
            "",
            "| book_id | char_offset | first 80 chars of window | match_count |",
            "|---|---:|---|---:|",
        ])
        for rec in hit_records[:10]:
            escaped = rec['window_first_80'].replace('|', '\\|')
            md.append(
                f"| {rec['book_id']} | {rec['char_offset_in_body']} | "
                f"`{escaped}…` | "
                f"{rec['tinystories_match_count']} |"
            )

    md.append("")
    md.append(
        f"_Generated by `scripts/corpus/gutenberg/contamination_preview.py` "
        f"with seed 0x{SEED:X}._"
    )
    docs_path.write_text("\n".join(md) + "\n")
    print(f"wrote {docs_path}", file=sys.stderr)
    print(f"summary: {len(hit_records)} / {len(samples)} windows had a verbatim match")
    return 0


if __name__ == "__main__":
    sys.exit(main())

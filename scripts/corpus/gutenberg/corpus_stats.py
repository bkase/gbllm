#!/usr/bin/env python3
"""Compute summary stats over the F-S4 harvested + stripped Gutenberg corpus.

This is a *reference* pass that gives F-S4.05/09 implementers expected
values to check against. It deliberately uses whitespace tokenization
(NOT charset_v1) — the real numbers from build-corpus will differ in
exact token counts but should bound to within a small constant of these.

Inputs:
  corpus/gutenberg/book_ids.json
  corpus/gutenberg/splits.json
  corpus/gutenberg/bodies/{id}/body.txt
  corpus/gutenberg/bodies/{id}/body_record.json

Outputs:
  corpus/gutenberg/corpus_stats.json  (machine-readable, gitignored)
  docs/experiments/S4-harvest-report.md  (human-readable, committed)
"""
from __future__ import annotations

import json
import re
import statistics
import sys
from pathlib import Path

NON_ASCII_RE = re.compile(r"[^\x00-\x7f]")
WHITESPACE_RE = re.compile(r"\S+")


def main() -> int:
    here = Path(__file__).resolve()
    repo = here.parents[3]
    corpus_dir = repo / "corpus/gutenberg"

    selection = json.loads((corpus_dir / "book_ids.json").read_text())
    book_ids: list[int] = selection["book_ids"]
    splits = json.loads((corpus_dir / "splits.json").read_text())
    train_ids = set(splits["train"])
    val_ids = set(splits["val"])

    per_book: list[dict] = []
    for bid in book_ids:
        body_path = corpus_dir / "bodies" / str(bid) / "body.txt"
        record_path = corpus_dir / "bodies" / str(bid) / "body_record.json"
        if not body_path.exists() or not record_path.exists():
            per_book.append({"book_id": bid, "retained": False})
            continue
        body = body_path.read_text(encoding="utf-8")
        body_bytes = len(body.encode("utf-8"))
        non_ascii_chars = len(NON_ASCII_RE.findall(body))
        tokens = WHITESPACE_RE.findall(body)
        token_count = len(tokens)
        rec = json.loads(record_path.read_text())
        per_book.append({
            "book_id": bid,
            "retained": True,
            "split": "train" if bid in train_ids else ("val" if bid in val_ids else None),
            "body_bytes": body_bytes,
            "body_chars": len(body),
            "token_count_ws": token_count,
            "non_ascii_chars": non_ascii_chars,
            "non_ascii_fraction": non_ascii_chars / max(1, len(body)),
            "body_sha256": rec["body_sha256"],
        })

    retained = [b for b in per_book if b["retained"]]
    train = [b for b in retained if b["split"] == "train"]
    val = [b for b in retained if b["split"] == "val"]

    def stats(values: list[int]) -> dict:
        if not values:
            return {"count": 0}
        return {
            "count": len(values),
            "min": min(values),
            "max": max(values),
            "mean": round(statistics.mean(values), 2),
            "median": int(statistics.median(values)),
            "p10": int(statistics.quantiles(values, n=10)[0]) if len(values) >= 10 else None,
            "p90": int(statistics.quantiles(values, n=10)[8]) if len(values) >= 10 else None,
            "sum": sum(values),
        }

    summary = {
        "book_ids_total": len(book_ids),
        "retained_book_count": len(retained),
        "dropped_book_count": len(book_ids) - len(retained),
        "retained_floor": 1350,
        "retained_floor_passed": len(retained) >= 1350,
        "split": {
            "train_count": len(train),
            "val_count": len(val),
            "train_fraction": round(len(train) / max(1, len(retained)), 4),
            "target_train_fraction": 0.90,
        },
        "body_bytes": {
            "all": stats([b["body_bytes"] for b in retained]),
            "train": stats([b["body_bytes"] for b in train]),
            "val": stats([b["body_bytes"] for b in val]),
        },
        "token_count_ws": {
            "all": stats([b["token_count_ws"] for b in retained]),
            "train": stats([b["token_count_ws"] for b in train]),
            "val": stats([b["token_count_ws"] for b in val]),
        },
        "non_ascii_fraction": {
            "all": stats([round(b["non_ascii_fraction"], 4) for b in retained]),
        },
        "notes": [
            "token_count_ws is whitespace tokenization, NOT charset_v1. The real",
            "F-G2 tokenizer will produce different per-book counts; this is a",
            "rough magnitude check, not a determinism oracle.",
            "non_ascii_fraction is on the NFC-normalized body. Books with high",
            "non-ASCII (>0.05) are candidates for charset_v1 unmappable-rate",
            "investigation (§D5).",
        ],
    }

    out_path = corpus_dir / "corpus_stats.json"
    out_path.write_text(json.dumps({"summary": summary, "per_book": per_book}, indent=2) + "\n")
    print(f"wrote {out_path}")

    # Human-readable report
    docs_path = repo / "docs/experiments/S4-harvest-report.md"
    docs_path.parent.mkdir(parents=True, exist_ok=True)
    body_b = summary["body_bytes"]
    tok = summary["token_count_ws"]
    nascii = summary["non_ascii_fraction"]["all"]
    md = [
        "# F-S4 Gutenberg Harvest Report",
        "",
        "Reference statistics over the F-S4 harvested + §D3-stripped Gutenberg",
        "corpus. Generated by `scripts/corpus/gutenberg/corpus_stats.py` against",
        "the local 1500-book harvest under `corpus/gutenberg/` (gitignored).",
        "",
        "These numbers are a **magnitude check**, not a determinism oracle —",
        "the real F-G2 charset_v1 tokenizer will produce different exact token",
        "counts. Use them when implementing F-S4.05 / F-S4.09 to confirm the",
        "build-corpus pipeline is in the right ballpark.",
        "",
        "## Headline",
        "",
        f"- **Book IDs selected (§D1):** {summary['book_ids_total']}",
        f"- **Retained after §D3 strip:** {summary['retained_book_count']} "
        f"({100 * summary['retained_book_count'] / summary['book_ids_total']:.1f}% retention)",
        f"- **Retention floor (§D1 line 216):** {summary['retained_floor']} — "
        f"{'PASSED ✓' if summary['retained_floor_passed'] else 'FAILED ✗'}",
        f"- **Train/val split (§D2):** "
        f"{summary['split']['train_count']} train / {summary['split']['val_count']} val "
        f"(train_fraction = {summary['split']['train_fraction']}, target 0.90)",
        "",
        "## Body size distribution (UTF-8 bytes)",
        "",
        "| Split | Count | Min | p10 | Median | Mean | p90 | Max | Total |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for split_name, stats_obj in [
        ("All retained", body_b["all"]),
        ("Train", body_b["train"]),
        ("Val", body_b["val"]),
    ]:
        md.append(
            f"| {split_name} | {stats_obj['count']} | {stats_obj['min']:,} | "
            f"{stats_obj.get('p10', '—')} | {stats_obj['median']:,} | "
            f"{stats_obj['mean']:,.0f} | {stats_obj.get('p90', '—')} | "
            f"{stats_obj['max']:,} | {stats_obj['sum']:,} |"
        )
    md.extend([
        "",
        "## Whitespace-token count (approximate)",
        "",
        "| Split | Count | Min | Median | Mean | Max | Total |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ])
    for split_name, stats_obj in [
        ("All retained", tok["all"]),
        ("Train", tok["train"]),
        ("Val", tok["val"]),
    ]:
        md.append(
            f"| {split_name} | {stats_obj['count']} | {stats_obj['min']:,} | "
            f"{stats_obj['median']:,} | {stats_obj['mean']:,.0f} | "
            f"{stats_obj['max']:,} | {stats_obj['sum']:,} |"
        )
    md.extend([
        "",
        "## Non-ASCII character fraction (per-book)",
        "",
        f"- Min: {nascii['min']:.4f}",
        f"- Median: {nascii['median']:.4f}" if nascii.get('median') is not None else "",
        f"- Mean: {nascii['mean']:.4f}",
        f"- Max: {nascii['max']:.4f}",
        "",
        "Books above 0.05 are candidates for charset_v1 unmappable-rate",
        "investigation when F-S4.07 (§D5) lands.",
        "",
        "## Caveats",
        "",
        "- Token counts are whitespace tokenization, not charset_v1.",
        "- The non_ascii_fraction is over the NFC-normalized body, not over",
        "  charset_v1-mapped token ids.",
        "- Source bytes (565 MB) and body bytes (526 MB) live in the gitignored",
        "  `corpus/gutenberg/` directory; full per-book stats are in",
        "  `corpus/gutenberg/corpus_stats.json` (also gitignored).",
        "",
    ])
    docs_path.write_text("\n".join(md) + "\n")
    print(f"wrote {docs_path}")

    # Also print the summary to stdout for the commit transcript.
    print()
    print(f"retention: {summary['retained_book_count']}/{summary['book_ids_total']} "
          f"(floor {summary['retained_floor']}, passed={summary['retained_floor_passed']})")
    print(f"train_fraction_observed: {summary['split']['train_fraction']}")
    print(f"total_body_bytes:       {body_b['all']['sum']:,}")
    print(f"total_ws_tokens:        {tok['all']['sum']:,}")
    print(f"non_ascii_fraction.max: {nascii['max']:.4f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

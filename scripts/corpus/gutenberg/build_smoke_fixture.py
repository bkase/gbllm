#!/usr/bin/env python3
"""Build the F-S4.18 tiny in-repo Gutenberg smoke fixture.

Selects 8-10 books from the local 1500-book harvest that:
  - are UTF-8 plaintext (preference class 1)
  - survived the §D3 strip pass (have a body)
  - span a range of source-blob sizes so build-corpus tests exercise both
    small and medium books
  - between them surface at least three distinct header/footer banner
    variants (the F-S4.06 golden corpus is a separate, larger set; this
    smoke set only needs enough variety to keep build-corpus honest)

Staged to fixtures/corpora/gutenberg_smoke/:
  - one .bin per book, content-addressed by source_blob_sha256
  - fixtures/corpora/gutenberg_smoke.toml with sha256 pins matching
    the gutenberg.toml schema's [[sources]] table shape

This is a stage-only script. It does NOT emit `expected.toml` or run
build-corpus — those land in F-S4.05 once the Rust implementation exists.

Inputs:
  corpus/gutenberg/book_ids.json
  corpus/gutenberg/sources/{id}/source_record.json + .bin
  corpus/gutenberg/bodies/{id}/body_record.json (used as a 'survived
    strip' liveness check)
"""
from __future__ import annotations

import hashlib
import json
import re
import shutil
import sys
from pathlib import Path

HEADER_VARIANT_RE = re.compile(
    rb"\*{3}\s*START OF (THIS |THE )?PROJECT GUTENBERG EBOOK",
    re.IGNORECASE,
)


def banner_variant(blob: bytes) -> str:
    """Tag the header-banner variant a book uses, for variant-coverage."""
    m = HEADER_VARIANT_RE.search(blob)
    if not m:
        return "no_match"  # unlikely if body survived strip
    sub = (m.group(1) or b"").strip().upper().decode("ascii", "replace")
    if sub == "THIS":
        return "THIS"
    if sub == "THE":
        return "THE"
    return "BARE"  # "*** START OF PROJECT GUTENBERG EBOOK" with no THIS/THE


def main() -> int:
    here = Path(__file__).resolve()
    repo = here.parents[3]
    corpus_dir = repo / "corpus/gutenberg"
    out_dir = repo / "fixtures/corpora/gutenberg_smoke"
    pin_path = repo / "fixtures/corpora/gutenberg_smoke.toml"

    selection = json.loads((corpus_dir / "book_ids.json").read_text())
    book_ids: list[int] = selection["book_ids"]

    # Walk in book_id order, collect candidates that survived strip + are pref 1.
    candidates: list[dict] = []
    for bid in book_ids:
        rec_path = corpus_dir / "sources" / str(bid) / "source_record.json"
        body_path = corpus_dir / "bodies" / str(bid) / "body_record.json"
        if not rec_path.exists() or not body_path.exists():
            continue
        rec = json.loads(rec_path.read_text())
        if rec.get("preference_class") != 1:
            continue
        blob_path = corpus_dir / "sources" / str(bid) / rec["blob_filename"]
        blob = blob_path.read_bytes()
        variant = banner_variant(blob)
        candidates.append({
            "book_id": bid,
            "size": len(blob),
            "variant": variant,
            "rec": rec,
            "blob_path": blob_path,
        })

    # Selection strategy: pick 8 books across the size distribution.
    # 4 smallest exercise the fast path; 4 sampled from the size distribution
    # exercise medium bodies. Header/footer variant coverage is intentionally
    # NOT a goal here — that responsibility lives in F-S4.06's golden corpus,
    # which can include us-ascii (preference_class=2) books we need to re-encode.
    # F-S4.18 is a UTF-8-clean smoke fixture for build-corpus + KN baseline.
    BUDGET = 5 * 1024 * 1024
    TARGET_BOOKS = 8

    by_size = sorted(candidates, key=lambda c: c["size"])
    if len(by_size) < TARGET_BOOKS:
        print(f"WARNING: only {len(by_size)} candidate books", file=sys.stderr)
        TARGET_BOOKS = len(by_size)

    # Take 4 smallest + 4 evenly-spaced through the rest of the size distribution.
    small_half = by_size[: TARGET_BOOKS // 2]
    rest = by_size[TARGET_BOOKS // 2 :]
    if rest and TARGET_BOOKS // 2 > 0:
        stride = max(1, len(rest) // (TARGET_BOOKS // 2))
        sampled = [rest[i * stride] for i in range(TARGET_BOOKS // 2) if i * stride < len(rest)]
    else:
        sampled = []
    selected = small_half + sampled

    # Drop the largest until under budget.
    selected.sort(key=lambda c: c["size"])
    out = []
    total = 0
    for c in selected:
        if total + c["size"] > BUDGET:
            continue
        out.append(c)
        total += c["size"]
    out.sort(key=lambda c: c["book_id"])

    if len(out) < 6:
        print(f"WARNING: only {len(out)} books fit under 5 MB", file=sys.stderr)

    # Stage files
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)
    for c in out:
        dest = out_dir / f"{c['book_id']}.{c['rec']['source_blob_sha256'][:16]}.bin"
        shutil.copy2(c["blob_path"], dest)
        c["staged_path"] = str(dest.relative_to(repo))

    # Emit pin TOML
    lines = [
        "# fixtures/corpora/gutenberg_smoke.toml",
        "# F-S4.18 tiny in-repo Gutenberg smoke fixture: 8-10 hash-pinned",
        "# public-domain books sampled from the F-S4.04 harvest. Exists so",
        "# F-S4.05 build-corpus and the corpus-side test beads (F-S4.06/07/08/09)",
        "# can run in CI without the 1.3 GB full harvest.",
        "#",
        "# Emitted by scripts/corpus/gutenberg/build_smoke_fixture.py.",
        "",
        'schema = "gutenberg_smoke_fixture.v1"',
        'source_name = "Project Gutenberg"',
        f"book_count = {len(out)}",
        f"total_bytes = {total}",
        "",
        "[header_variant_coverage]",
    ]
    coverage: dict[str, int] = {}
    for c in out:
        coverage[c["variant"]] = coverage.get(c["variant"], 0) + 1
    for v in ("THIS", "THE", "BARE"):
        lines.append(f"{v} = {coverage.get(v, 0)}")
    lines.append("")

    for c in out:
        rec = c["rec"]
        lines.extend([
            "[[sources]]",
            f"book_id = {c['book_id']}",
            f'source_landing_url = "{rec["source_landing_url"]}"',
            f'rdf_resource_url = "{rec["rdf_resource_url"]}"',
            f'source_blob_sha256 = "{rec["source_blob_sha256"]}"',
            f"source_blob_size_bytes = {rec['source_blob_size_bytes']}",
            f'media_type = "{rec["media_type"]}"',
            f'charset = "{rec.get("charset", "utf-8")}"',
            f"preference_class = {rec['preference_class']}",
            f'header_variant = "{c["variant"]}"',
            f'local_blob_path = "{c["staged_path"]}"',
            "",
        ])

    pin_path.write_text("\n".join(lines))

    print(f"wrote {pin_path}")
    print(f"  staged_dir              {out_dir.relative_to(repo)}")
    print(f"  book_count              {len(out)}")
    print(f"  total_bytes             {total} ({total / 1024 / 1024:.2f} MB)")
    print(f"  header_variant_coverage {coverage}")
    print(f"  smoke_pin_sha256        {hashlib.sha256(pin_path.read_bytes()).hexdigest()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

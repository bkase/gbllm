#!/usr/bin/env python3
"""Emit fixtures/corpora/gutenberg.toml from the local harvest.

Per F-S4 §D1 lines 933-935: this fixture file records "catalog, selected
raw-file, mirror/cache, and sha256 pins used to construct the manifest."
It is the INPUT-side pin set. The OUTPUT (`gutenberg_manifest.v1`,
including train/val sha256s, drop_counts, etc.) is produced later by
`gbf s4 build-corpus` and is out of scope here.

What this script pins:
  - catalog_snapshot_{url, sha256, observed_at_utc, last_modified_utc}
  - selection_filter (verbatim D1 JSON) + its sha256
  - rank_selection (rank prefix + target slice + actual self-hash)
  - book_ids (sorted ascending; 1500)
  - sources (one per book_id; same order; mirrors GutenbergSourceRecord)
  - header_regex_pattern / footer_regex_pattern (D3, verbatim)
  - split_seed_u128 + fractions (D2)
  - dedup_policy (constant from §D1 line 875)
  - retained_book_count_min (1350)

What it does not pin (deferred to the build-corpus output manifest):
  - train_sha256, val_sha256, train_byte_length, val_byte_length
  - drop_count_*, unmappable_rate_corpus
  - manifest_self_hash
  - normalization_spec_self_hash (charset_v1 — owned by S3/F-G2)
"""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

# §D1 selection filter — verbatim from the RFC.
SELECTION_FILTER = {
    "languages_canonical": ["en"],
    "pg_rights": "Public domain in the USA.",
    "has_plain_text": True,
}

# §D3 header / footer marker regexes — verbatim from the RFC.
HEADER_REGEX_PATTERN = (
    r"(?im-s)\A(?s:.*?)^[ \t]*\*{3}[ \t]*START OF (?:THIS |THE )?"
    r"PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*\n"
)
FOOTER_REGEX_PATTERN = (
    r"(?im-s)\n[ \t]*\*{3}[ \t]*END OF (?:THIS |THE )?"
    r"PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})(?s:.*)\z"
)

DEDUP_POLICY_KIND = "exact_post_strip_charset_body_sha"
DEDUP_POLICY_NOTES = (
    "Two retained books with identical post_charset_body_sha256 (i.e. "
    "identical body token-id streams excluding <bos>/<eos>) are treated "
    "as duplicates; only the lowest book_id is retained. Raw "
    "source_blob_sha256 is reported but is not the dedup key, because "
    "Gutenberg boilerplate divergence (release notes, edition metadata) "
    "can mask body-identical duplicates."
)


def canonical_json(obj) -> str:
    """Stable, compact JSON: sorted keys, no extra whitespace."""
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def toml_escape(s: str) -> str:
    return (
        s.replace("\\", "\\\\")
        .replace("\"", "\\\"")
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
    )


def toml_str(s: str) -> str:
    return f"\"{toml_escape(s)}\""


def toml_array_of_u32(values: list[int]) -> str:
    if not values:
        return "[]"
    parts = []
    line = "["
    for i, v in enumerate(values):
        token = f"{v}"
        if i < len(values) - 1:
            token += ", "
        if len(line) + len(token) > 100:
            parts.append(line)
            line = "  " + token
        else:
            line += token
    parts.append(line + "]")
    return "\n".join(parts)


def main() -> int:
    here = Path(__file__).resolve()
    repo = here.parents[3]
    corpus_dir = repo / "corpus/gutenberg"
    out_path = repo / "fixtures/corpora/gutenberg.toml"

    catalog_prov = json.loads((corpus_dir / "catalog_snapshot.provenance.json").read_text())
    selection = json.loads((corpus_dir / "book_ids.json").read_text())
    splits = json.loads((corpus_dir / "splits.json").read_text())
    book_ids: list[int] = selection["book_ids"]

    # Load source records in book_ids order.
    sources: list[dict] = []
    for bid in book_ids:
        rec_path = corpus_dir / "sources" / str(bid) / "source_record.json"
        sources.append(json.loads(rec_path.read_text()))

    selection_filter_canonical = canonical_json(SELECTION_FILTER)
    selection_filter_sha256 = hashlib.sha256(
        selection_filter_canonical.encode("utf-8")
    ).hexdigest()

    # Build the TOML by hand (no toml writer in stdlib) so the layout is
    # readable and the [[sources]] tables stay grouped.
    lines: list[str] = []
    add = lines.append

    add("# fixtures/corpora/gutenberg.toml")
    add("# F-S4 fixture pin file. Records the network-derived inputs to")
    add("# `gbf s4 build-corpus`. See history/rfcs/F-S4-gutenberg-promotion.md.")
    add("#")
    add("# Emitted by scripts/corpus/gutenberg/emit_fixture_toml.py from")
    add("# the corpus/gutenberg/ harvest.")
    add("")
    add("schema = \"gutenberg_fixture.v1\"")
    add("source_name = \"Project Gutenberg\"")
    add("")

    add("[catalog_snapshot]")
    add(f"url = {toml_str(catalog_prov['catalog_snapshot_url'])}")
    add(f"sha256 = {toml_str(catalog_prov['catalog_snapshot_sha256'])}")
    add(f"size_bytes = {catalog_prov['catalog_snapshot_size_bytes']}")
    add(f"observed_at_utc = {toml_str(catalog_prov['catalog_snapshot_observed_at_utc'])}")
    add(f"last_modified_utc = {toml_str(catalog_prov['catalog_snapshot_last_modified_utc'])}")
    add(f"local_path = {toml_str(catalog_prov['local_path'])}")
    add("")

    add("[selection_filter]")
    add("# Verbatim §D1 filter, applied to the RDF catalog snapshot.")
    add(f"canonical_json = {toml_str(selection_filter_canonical)}")
    add(f"sha256 = {toml_str(selection_filter_sha256)}")
    add("")

    add("[rank_selection]")
    add("# §D1 deterministic top-1500 selection by rank_key.")
    add(f"rank_prefix_ascii = {toml_str(selection['rank_prefix_ascii'])}")
    add(f"target_slice = {selection['target_slice']}")
    add(f"candidates_total = {selection['candidates_total']}")
    add(f"book_ids_self_hash_sha256 = {toml_str(selection['book_ids_self_hash_sha256'])}")
    add(f"book_count = {len(book_ids)}")
    add("")

    add("[split]")
    add("# §D2 per-book split.")
    add(f"seed_label_ascii = {toml_str(splits['seed_label_ascii'])}")
    add(f"split_seed_u128_hex = {toml_str(splits['split_seed_bytes_hex'])}")
    add(f"train_fraction = {splits['train_fraction']}")
    add("val_fraction = 0.10")
    add("test_fraction = 0.00")
    add(f"observed_train_count = {splits['train_count']}")
    add(f"observed_val_count = {splits['val_count']}")
    add(f"splits_self_hash_sha256 = {toml_str(splits['self_hash_sha256'])}")
    add("")

    add("[markers]")
    add("# §D3 verbatim header / footer regex patterns.")
    add(f"header_regex_pattern = {toml_str(HEADER_REGEX_PATTERN)}")
    add(f"footer_regex_pattern = {toml_str(FOOTER_REGEX_PATTERN)}")
    add("")

    add("[dedup_policy]")
    add(f"kind = {toml_str(DEDUP_POLICY_KIND)}")
    add(f"notes = {toml_str(DEDUP_POLICY_NOTES)}")
    add("")

    add("[guards]")
    add("# §D1 / §D5 hard floors and ceilings.")
    add("retained_book_count_min = 1350")
    add("unmappable_rate_corpus_max = 0.005")
    add("marker_missing_drop_max_fraction = 0.05")
    add("")

    add("[book_ids]")
    add("# §D1: 1500 sorted ascending. The book_ids_self_hash_sha256 above")
    add("# is the canonical pin; this array is a convenience for reviewers.")
    add(f"values = {toml_array_of_u32(book_ids)}")
    add("")

    add("# §D1: one [[sources]] table per book_id, same order. Mirrors the")
    add("# GutenbergSourceRecord schema. RFC requires source_landing_url")
    add("# is recorded but NEVER fetched at replay time; replay reads from")
    add("# the content-addressed mirror keyed by source_blob_sha256.")
    add("")
    for rec in sources:
        bid = rec["book_id"]
        sha = rec["source_blob_sha256"]
        add("[[sources]]")
        add(f"book_id = {bid}")
        add(f"source_landing_url = {toml_str(rec['source_landing_url'])}")
        add(f"rdf_resource_url = {toml_str(rec['rdf_resource_url'])}")
        if rec.get("mirror_fetch_url") is not None:
            add(f"mirror_fetch_url = {toml_str(rec['mirror_fetch_url'])}")
        add(f"source_blob_sha256 = {toml_str(sha)}")
        add(f"source_blob_size_bytes = {rec['source_blob_size_bytes']}")
        add(f"media_type = {toml_str(rec['media_type'])}")
        if rec.get("charset"):
            add(f"charset = {toml_str(rec['charset'])}")
        if rec.get("extent_declared") is not None:
            add(f"extent_declared = {rec['extent_declared']}")
        add(f"preference_class = {rec['preference_class']}")
        add(f"fetch_namespace_kind = {toml_str(rec['fetch_namespace_kind'])}")
        add(f"fetch_namespace_id = {toml_str(rec['fetch_namespace_id'])}")
        add(f"local_blob_path = {toml_str('corpus/gutenberg/sources/' + str(bid) + '/' + rec['blob_filename'])}")
        add("")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(lines))

    # Print a brief summary.
    size = out_path.stat().st_size
    fixture_sha256 = hashlib.sha256(out_path.read_bytes()).hexdigest()
    print(f"wrote {out_path}")
    print(f"  size_bytes              {size}")
    print(f"  book_ids                {len(book_ids)}")
    print(f"  sources                 {len(sources)}")
    print(f"  selection_filter_sha256 {selection_filter_sha256}")
    print(f"  fixture_file_sha256     {fixture_sha256}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Apply the F-S4 §D1 filter to the pinned Gutenberg RDF catalog and select
the 1500 book ids deterministically.

Inputs:
  corpus/gutenberg/rdf-files.tar.bz2  (the pinned catalog snapshot)

Outputs (under corpus/gutenberg/):
  candidates.jsonl   one line per book passing the filter, with plaintext
                     resource metadata kept for the fetch step
  book_ids.json      the deterministic 1500-id selection (sorted ascending)
  selection.log      summary counters

Filter (verbatim from RFC F-S4 §D1):
  languages_canonical == ["en"]              (the RDF language set, lowercased
                                              and sorted, is exactly ["en"])
  pg_rights           == "Public domain in the USA."
  has_plain_text      == True                (>= 1 pgterms:file whose declared
                                              dcterms:format media type starts
                                              with "text/plain")

Selection (verbatim from RFC F-S4 §D1):
  rank_key(id) = (sha256("gbf:s4:gutenberg-select:v1" || le_u32(id)), id)
  book_ids     = first 1500 ids by rank_key, then sorted ascending
"""
from __future__ import annotations

import hashlib
import io
import json
import struct
import sys
import tarfile
import time
from pathlib import Path
from xml.etree import ElementTree as ET

NS = {
    "rdf": "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "dcterms": "http://purl.org/dc/terms/",
    "pgterms": "http://www.gutenberg.org/2009/pgterms/",
    "dcam": "http://purl.org/dc/dcam/",
}

RANK_PREFIX = b"gbf:s4:gutenberg-select:v1"
TARGET_SLICE = 1500


def book_id_from_about(about: str) -> int | None:
    # rdf:about looks like "ebooks/1342".
    if not about.startswith("ebooks/"):
        return None
    try:
        return int(about.split("/", 1)[1])
    except (ValueError, IndexError):
        return None


def canonical_languages(ebook: ET.Element) -> list[str]:
    """Return the RDF language set for an ebook, lowercased and sorted."""
    langs: set[str] = set()
    for lang_el in ebook.findall("dcterms:language", NS):
        for value_el in lang_el.iter(f'{{{NS["rdf"]}}}value'):
            text = (value_el.text or "").strip()
            if text:
                langs.add(text.lower())
    return sorted(langs)


def rights_literal(ebook: ET.Element) -> str | None:
    rights_el = ebook.find("dcterms:rights", NS)
    if rights_el is None or rights_el.text is None:
        return None
    return rights_el.text.strip()


def plaintext_resources(ebook: ET.Element) -> list[dict]:
    """Collect pgterms:file entries whose declared media type starts with
    text/plain. Returns dicts with url, media_type, charset, extent."""
    results: list[dict] = []
    for has_fmt in ebook.findall("dcterms:hasFormat", NS):
        for f in has_fmt.findall("pgterms:file", NS):
            url = f.get(f'{{{NS["rdf"]}}}about', "")
            extent_el = f.find("dcterms:extent", NS)
            extent = None
            if extent_el is not None and extent_el.text:
                try:
                    extent = int(extent_el.text.strip())
                except ValueError:
                    pass
            # All declared media types for this file (usually one).
            for fmt_el in f.findall("dcterms:format", NS):
                for value_el in fmt_el.iter(f'{{{NS["rdf"]}}}value'):
                    raw = (value_el.text or "").strip()
                    if not raw:
                        continue
                    low = raw.lower()
                    if low.startswith("text/plain"):
                        charset = None
                        for part in (p.strip() for p in raw.split(";")[1:]):
                            if part.lower().startswith("charset="):
                                charset = part.split("=", 1)[1].strip()
                        results.append({
                            "url": url,
                            "media_type": raw,
                            "charset": charset,
                            "extent": extent,
                        })
    return results


def rank_key(book_id: int) -> tuple[bytes, int]:
    return (
        hashlib.sha256(RANK_PREFIX + struct.pack("<I", book_id)).digest(),
        book_id,
    )


def parse_rdf(blob: bytes) -> dict | None:
    """Parse one RDF blob. Return a candidate record or None if no ebook node."""
    try:
        root = ET.fromstring(blob)
    except ET.ParseError:
        return None
    ebook = root.find("pgterms:ebook", NS)
    if ebook is None:
        return None
    about = ebook.get(f'{{{NS["rdf"]}}}about', "")
    bid = book_id_from_about(about)
    if bid is None:
        return None
    return {
        "id": bid,
        "rights": rights_literal(ebook),
        "languages_canonical": canonical_languages(ebook),
        "plaintext_resources": plaintext_resources(ebook),
    }


def main() -> int:
    here = Path(__file__).resolve()
    repo = here.parents[3]
    catalog = repo / "corpus/gutenberg/rdf-files.tar.bz2"
    out_dir = repo / "corpus/gutenberg"
    out_dir.mkdir(parents=True, exist_ok=True)

    candidates_path = out_dir / "candidates.jsonl"
    book_ids_path = out_dir / "book_ids.json"
    log_path = out_dir / "selection.log"

    counters = {
        "rdf_entries_total": 0,
        "rdf_parse_failures": 0,
        "no_ebook_node": 0,
        "filter_drops_languages": 0,
        "filter_drops_rights": 0,
        "filter_drops_no_plain_text": 0,
        "candidates_kept": 0,
    }
    rights_filter = "Public domain in the USA."

    start = time.time()
    with tarfile.open(catalog, "r:bz2") as tar, candidates_path.open("w") as out:
        for member in tar:
            if not member.isfile() or not member.name.endswith(".rdf"):
                continue
            counters["rdf_entries_total"] += 1
            fh = tar.extractfile(member)
            if fh is None:
                counters["rdf_parse_failures"] += 1
                continue
            blob = fh.read()
            rec = parse_rdf(blob)
            if rec is None:
                if blob.strip():
                    counters["rdf_parse_failures"] += 1
                else:
                    counters["no_ebook_node"] += 1
                continue
            if rec["languages_canonical"] != ["en"]:
                counters["filter_drops_languages"] += 1
                continue
            if rec["rights"] != rights_filter:
                counters["filter_drops_rights"] += 1
                continue
            if not rec["plaintext_resources"]:
                counters["filter_drops_no_plain_text"] += 1
                continue
            counters["candidates_kept"] += 1
            out.write(json.dumps(rec, separators=(",", ":")) + "\n")
            if counters["rdf_entries_total"] % 5000 == 0:
                elapsed = time.time() - start
                print(
                    f"  scanned {counters['rdf_entries_total']:>6d}  "
                    f"kept {counters['candidates_kept']:>5d}  "
                    f"({elapsed:5.1f}s)",
                    file=sys.stderr,
                )

    elapsed = time.time() - start
    print(f"filter pass done in {elapsed:.1f}s", file=sys.stderr)

    # Re-read candidates and apply the deterministic rank-key selection.
    candidate_ids: list[int] = []
    with candidates_path.open() as f:
        for line in f:
            rec = json.loads(line)
            candidate_ids.append(rec["id"])

    if len(candidate_ids) < TARGET_SLICE:
        print(
            f"ERROR: only {len(candidate_ids)} candidates; need >= {TARGET_SLICE}",
            file=sys.stderr,
        )
        return 2

    ranked = sorted(candidate_ids, key=rank_key)[:TARGET_SLICE]
    book_ids = sorted(ranked)

    selection = {
        "schema_hint": "F-S4 §D1 selection output",
        "rank_prefix_ascii": RANK_PREFIX.decode("ascii"),
        "target_slice": TARGET_SLICE,
        "candidates_total": len(candidate_ids),
        "book_ids": book_ids,
        "book_ids_self_hash_sha256": hashlib.sha256(
            ",".join(str(i) for i in book_ids).encode("ascii")
        ).hexdigest(),
    }
    book_ids_path.write_text(json.dumps(selection, indent=2) + "\n")

    summary_lines = [
        f"catalog_path                   {catalog}",
        f"target_slice                   {TARGET_SLICE}",
        f"rdf_entries_total              {counters['rdf_entries_total']}",
        f"rdf_parse_failures             {counters['rdf_parse_failures']}",
        f"no_ebook_node                  {counters['no_ebook_node']}",
        f"filter_drops_languages         {counters['filter_drops_languages']}",
        f"filter_drops_rights            {counters['filter_drops_rights']}",
        f"filter_drops_no_plain_text     {counters['filter_drops_no_plain_text']}",
        f"candidates_kept                {counters['candidates_kept']}",
        f"book_ids_selected              {len(book_ids)}",
        f"book_ids_self_hash_sha256      {selection['book_ids_self_hash_sha256']}",
        f"elapsed_seconds                {elapsed:.1f}",
    ]
    log_path.write_text("\n".join(summary_lines) + "\n")
    for line in summary_lines:
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())

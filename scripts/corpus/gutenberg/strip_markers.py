#!/usr/bin/env python3
"""Apply F-S4 §D3 header/footer marker stripping to each fetched source blob.

Per §D3, the pipeline on each source blob is:
  1. format-decode into a UTF-8 byte string
     (here: the source is already declared text/plain; charset=utf-8, so
      decoding is just bytes.decode("utf-8") with strict errors)
  2. validate UTF-8                          -> reason "invalid_utf8" on fail
  3. strip leading UTF-8 BOM if present
  4. normalize CRLF/CR -> LF
  5. apply Unicode NFC
  6. apply header_regex / footer_regex with the matching contract:
       - first header match
       - last footer match
       - header_match.end <= footer_match.start
     If any check fails or body is empty -> reason "gutenberg_marker_missing"

We also enforce the §D3 5%-of-book_ids cap on marker-missing drops; exceeding
the cap aborts and the bodies directory is not finalized.

Inputs:
  corpus/gutenberg/sources/{id}/source_record.json + blob

Outputs:
  corpus/gutenberg/bodies/{id}/body.txt
  corpus/gutenberg/bodies/{id}/body_record.json     per-book provenance
  corpus/gutenberg/strip.log                         summary counters
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
import unicodedata
from pathlib import Path

# Markers are applied to the normalized (LF-only, NFC) Unicode text.
HEADER_RE = re.compile(
    r"\A(?s:.*?)^[ \t]*\*{3}[ \t]*START OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*\n",
    re.IGNORECASE | re.MULTILINE,
)
FOOTER_RE = re.compile(
    r"\n[ \t]*\*{3}[ \t]*END OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})(?s:.*)\Z",
    re.IGNORECASE | re.MULTILINE,
)

DROP_REASON_INVALID_UTF8 = "invalid_utf8"
DROP_REASON_NO_MARKERS = "gutenberg_marker_missing"
DROP_REASON_SOURCE_DECODE = "source_decode_failed"

MARKER_DROP_CAP_FRACTION = 0.05  # §D3


def normalize_text(blob: bytes, charset: str | None) -> tuple[str, str | None]:
    """Decode + normalize per §D3. Returns (text, drop_reason_or_None)."""
    try:
        text = blob.decode(charset or "utf-8")
    except UnicodeDecodeError:
        return "", DROP_REASON_SOURCE_DECODE
    # Validate it's representable as UTF-8 (round-trip).
    try:
        text.encode("utf-8")
    except UnicodeEncodeError:
        return "", DROP_REASON_INVALID_UTF8
    # Strip leading BOM
    if text.startswith("﻿"):
        text = text[1:]
    # Normalize line endings CRLF/CR -> LF
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    # NFC
    text = unicodedata.normalize("NFC", text)
    return text, None


def strip_markers(text: str) -> tuple[str | None, str | None]:
    """Return (body, drop_reason_or_None)."""
    h = HEADER_RE.search(text)
    if h is None:
        return None, DROP_REASON_NO_MARKERS
    # find last footer
    last = None
    for m in FOOTER_RE.finditer(text):
        last = m
    if last is None:
        return None, DROP_REASON_NO_MARKERS
    if h.end() > last.start():
        return None, DROP_REASON_NO_MARKERS
    body = text[h.end() : last.start()]
    if not body.strip():
        return None, DROP_REASON_NO_MARKERS
    return body, None


def main() -> int:
    here = Path(__file__).resolve()
    repo = here.parents[3]
    book_ids_path = repo / "corpus/gutenberg/book_ids.json"
    sources_dir = repo / "corpus/gutenberg/sources"
    bodies_dir = repo / "corpus/gutenberg/bodies"
    log_path = repo / "corpus/gutenberg/strip.log"
    drops_path = repo / "corpus/gutenberg/strip_drops.jsonl"
    bodies_dir.mkdir(parents=True, exist_ok=True)

    book_ids: list[int] = json.loads(book_ids_path.read_text())["book_ids"]

    counters = {
        "total": 0,
        "kept": 0,
        DROP_REASON_INVALID_UTF8: 0,
        DROP_REASON_NO_MARKERS: 0,
        DROP_REASON_SOURCE_DECODE: 0,
        "missing_source_record": 0,
    }

    with drops_path.open("w") as drops:
        for bid in book_ids:
            counters["total"] += 1
            rec_path = sources_dir / str(bid) / "source_record.json"
            if not rec_path.exists():
                counters["missing_source_record"] += 1
                drops.write(json.dumps({"id": bid, "reason": "missing_source_record"}) + "\n")
                continue
            rec = json.loads(rec_path.read_text())
            blob_path = sources_dir / str(bid) / rec["blob_filename"]
            blob = blob_path.read_bytes()
            text, decode_reason = normalize_text(blob, rec.get("charset"))
            if decode_reason is not None:
                counters[decode_reason] += 1
                drops.write(json.dumps({"id": bid, "reason": decode_reason}) + "\n")
                continue
            body, strip_reason = strip_markers(text)
            if strip_reason is not None:
                counters[strip_reason] += 1
                drops.write(json.dumps({"id": bid, "reason": strip_reason}) + "\n")
                continue
            book_dir = bodies_dir / str(bid)
            book_dir.mkdir(parents=True, exist_ok=True)
            body_bytes = body.encode("utf-8")
            body_path = book_dir / "body.txt"
            body_path.write_bytes(body_bytes)
            body_record = {
                "schema_hint": "F-S4 §D3 body record (draft)",
                "book_id": bid,
                "source_blob_sha256": rec["source_blob_sha256"],
                "normalized_text_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                "body_sha256": hashlib.sha256(body_bytes).hexdigest(),
                "body_bytes": len(body_bytes),
            }
            (book_dir / "body_record.json").write_text(json.dumps(body_record, indent=2) + "\n")
            counters["kept"] += 1

    marker_drops = counters[DROP_REASON_NO_MARKERS]
    cap = int(len(book_ids) * MARKER_DROP_CAP_FRACTION)
    cap_breached = marker_drops > cap

    lines = [
        f"book_ids_total          {counters['total']}",
        f"kept                    {counters['kept']}",
        f"drop_marker_missing     {marker_drops}",
        f"drop_invalid_utf8       {counters[DROP_REASON_INVALID_UTF8]}",
        f"drop_source_decode      {counters[DROP_REASON_SOURCE_DECODE]}",
        f"missing_source_record   {counters['missing_source_record']}",
        f"marker_drop_cap         {cap}        (5% of book_ids)",
        f"marker_drop_cap_breached {cap_breached}",
    ]
    log_path.write_text("\n".join(lines) + "\n")
    for line in lines:
        print(line)
    return 2 if cap_breached else 0


if __name__ == "__main__":
    sys.exit(main())

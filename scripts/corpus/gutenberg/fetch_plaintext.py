#!/usr/bin/env python3
"""Fetch the plaintext source blob for each selected Gutenberg book id.

Inputs:
  corpus/gutenberg/candidates.jsonl   (from select_book_ids.py)
  corpus/gutenberg/book_ids.json      (the 1500-id selection)

Outputs (under corpus/gutenberg/sources/):
  {id}/{sha256}.txt                   the raw fetched source blob, content-
                                      addressed so a re-run that hits the
                                      same bytes is a no-op
  {id}/source_record.json             provenance: chosen URL, media type,
                                      sha256, fetch namespace, etc.
  fetch.log                           one structured line per book (jsonl)

Selection per book (F-S4 §D1 preference order, simplified to v1 plaintext
discovery — zip/gzip handling is deferred to the format-decoder bead since
Gutenberg RDFs only expose text/plain resources directly):

  1. text/plain; charset=utf-8       (uncompressed UTF-8 plaintext)
  2. text/plain; charset=*           (other charset; decode + re-encode later)
  3. text/plain (no charset)         (assume us-ascii; decode + re-encode)

Within each class, ties resolved by ascending canonical URL.

Network:
  We do not touch gutenberg.org main-site deep links beyond the canonical
  URLs the RDF already records. Each fetch:
    - sets a polite User-Agent that identifies us,
    - retries with backoff,
    - records fetch_namespace_kind = "official_robot_harvest" since we are
      pulling from the canonical text-resource URLs the RDF declares.

This script is restartable: it skips a book if the per-book directory
already contains a source_record.json whose recorded sha256 matches the
on-disk blob.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

USER_AGENT = "gbllm-fixture-builder/0.1 (research; brandernan@gmail.com)"
FETCH_NAMESPACE_KIND = "official_robot_harvest"
FETCH_NAMESPACE_ID = "https://www.gutenberg.org/"


def preference_class(res: dict) -> int:
    mt = (res.get("media_type") or "").lower()
    charset = (res.get("charset") or "").lower()
    if "charset=utf-8" in mt or charset == "utf-8":
        return 1
    if "charset=" in mt or charset:
        return 2
    return 3


def choose_resource(plaintext_resources: list[dict]) -> dict | None:
    if not plaintext_resources:
        return None
    return min(
        plaintext_resources,
        key=lambda r: (preference_class(r), r.get("url") or ""),
    )


def fetch_bytes(url: str, *, timeout: float = 60.0) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read()


def fetch_with_retries(url: str, *, attempts: int = 5) -> bytes:
    backoff = 2.0
    last: Exception | None = None
    for i in range(attempts):
        try:
            return fetch_bytes(url)
        except (urllib.error.URLError, TimeoutError) as e:
            last = e
            time.sleep(backoff)
            backoff = min(backoff * 2, 30.0)
    raise RuntimeError(f"fetch failed after {attempts} attempts: {url} ({last})")


def process_book(rec: dict, sources_dir: Path) -> dict:
    bid = rec["id"]
    chosen = choose_resource(rec.get("plaintext_resources") or [])
    if chosen is None:
        return {"id": bid, "status": "no_plaintext_resource"}
    book_dir = sources_dir / str(bid)
    record_path = book_dir / "source_record.json"
    if record_path.exists():
        try:
            existing = json.loads(record_path.read_text())
            blob_path = book_dir / existing["blob_filename"]
            if (
                blob_path.exists()
                and hashlib.sha256(blob_path.read_bytes()).hexdigest()
                == existing["source_blob_sha256"]
            ):
                return {"id": bid, "status": "cached", "sha256": existing["source_blob_sha256"]}
        except (KeyError, json.JSONDecodeError, OSError):
            pass
    book_dir.mkdir(parents=True, exist_ok=True)
    url = chosen["url"]
    try:
        blob = fetch_with_retries(url)
    except Exception as e:
        return {"id": bid, "status": "fetch_failed", "url": url, "error": str(e)}
    sha = hashlib.sha256(blob).hexdigest()
    blob_filename = f"{sha}.bin"
    (book_dir / blob_filename).write_bytes(blob)
    source_record = {
        "schema_hint": "F-S4 GutenbergSourceRecord (draft)",
        "book_id": bid,
        "source_landing_url": f"https://www.gutenberg.org/ebooks/{bid}",
        "rdf_resource_url": url,
        "mirror_fetch_url": url,
        "source_blob_sha256": sha,
        "source_blob_size_bytes": len(blob),
        "media_type": chosen.get("media_type"),
        "charset": chosen.get("charset"),
        "extent_declared": chosen.get("extent"),
        "preference_class": preference_class(chosen),
        "fetch_namespace_kind": FETCH_NAMESPACE_KIND,
        "fetch_namespace_id": FETCH_NAMESPACE_ID,
        "blob_filename": blob_filename,
        "fetched_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    record_path.write_text(json.dumps(source_record, indent=2) + "\n")
    return {"id": bid, "status": "fetched", "sha256": sha, "bytes": len(blob)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=4, help="concurrent fetches")
    ap.add_argument(
        "--limit", type=int, default=0, help="if > 0, only fetch first N selected ids"
    )
    args = ap.parse_args()

    here = Path(__file__).resolve()
    repo = here.parents[3]
    candidates_path = repo / "corpus/gutenberg/candidates.jsonl"
    book_ids_path = repo / "corpus/gutenberg/book_ids.json"
    sources_dir = repo / "corpus/gutenberg/sources"
    fetch_log_path = repo / "corpus/gutenberg/fetch.log"
    sources_dir.mkdir(parents=True, exist_ok=True)

    selection = json.loads(book_ids_path.read_text())
    wanted_ids: set[int] = set(selection["book_ids"])
    if args.limit > 0:
        wanted_ids = set(sorted(wanted_ids)[: args.limit])

    todo: list[dict] = []
    with candidates_path.open() as f:
        for line in f:
            rec = json.loads(line)
            if rec["id"] in wanted_ids:
                todo.append(rec)

    print(f"fetching {len(todo)} books with {args.workers} workers", file=sys.stderr)
    started = time.time()
    counters = {"fetched": 0, "cached": 0, "fetch_failed": 0, "no_plaintext_resource": 0}
    with fetch_log_path.open("a") as logf, ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {pool.submit(process_book, rec, sources_dir): rec["id"] for rec in todo}
        for i, fut in enumerate(as_completed(futures), 1):
            result = fut.result()
            counters[result["status"]] = counters.get(result["status"], 0) + 1
            logf.write(json.dumps(result, separators=(",", ":")) + "\n")
            if i % 50 == 0:
                elapsed = time.time() - started
                print(
                    f"  {i:>5d}/{len(todo)}  "
                    f"fetched={counters['fetched']}  cached={counters['cached']}  "
                    f"failed={counters['fetch_failed']}  ({elapsed:5.1f}s)",
                    file=sys.stderr,
                )

    print("done. counters:", counters, file=sys.stderr)
    return 0 if counters.get("fetch_failed", 0) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

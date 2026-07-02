#!/usr/bin/env python3
"""Hydrate source blobs referenced by fixtures/corpora/gutenberg.toml.

The S4 build-corpus path is intentionally network-disabled: it reads the
content-addressed `local_blob_path` entries already pinned in
`fixtures/corpora/gutenberg.toml`. This helper is the explicit operator step
for recreating that local cache from the fixture's `mirror_fetch_url` fields.

It is restartable and fail-closed:
  - existing blobs are accepted only when sha256 and optional size match;
  - fetched bytes are written only after matching the pinned sha256/size;
  - each blob is written through a temporary sibling and atomically replaced.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

USER_AGENT = "gbllm-gutenberg-fixture-hydrator/0.1 (research; brandernan@gmail.com)"
DEFAULT_FIXTURE = "fixtures/corpora/gutenberg.toml"
DEFAULT_LOG = "corpus/gutenberg/hydrate_fixture_blobs.log"
UTF8_BOM = b"\xef\xbb\xbf"


def parse_sources(toml_text: str) -> list[dict[str, str]]:
    records: list[dict[str, str]] = []
    current: dict[str, str] | None = None
    field_re = re.compile(r"^(\w+)\s*=\s*(.*)$")
    for raw in toml_text.splitlines():
        line = raw.strip()
        if line == "[[sources]]":
            if current is not None:
                records.append(current)
            current = {}
            continue
        if line.startswith("["):
            if current is not None:
                records.append(current)
                current = None
            continue
        if current is None:
            continue
        match = field_re.match(line)
        if match is None:
            continue
        key, value = match.group(1), match.group(2).strip()
        if value.startswith('"') and value.endswith('"'):
            value = value[1:-1]
        current[key] = value
    if current is not None:
        records.append(current)
    return records


def sha256_bytes(blob: bytes) -> str:
    return hashlib.sha256(blob).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expected_size(record: dict[str, str]) -> int | None:
    raw = record.get("source_blob_size_bytes")
    if raw is None or raw == "":
        return None
    return int(raw)


def validate_record(record: dict[str, str]) -> tuple[bool, str | None]:
    for key in ("book_id", "source_blob_sha256", "local_blob_path"):
        if not record.get(key):
            return False, f"missing {key}"
    if not re.fullmatch(r"[0-9a-f]{64}", record["source_blob_sha256"]):
        return False, "source_blob_sha256 must be lowercase hex sha256"
    if not record.get("mirror_fetch_url"):
        return False, "missing mirror_fetch_url"
    local = Path(record["local_blob_path"])
    if local.is_absolute() or ".." in local.parts:
        return False, f"unsafe local_blob_path {record['local_blob_path']!r}"
    return True, None


def fetch_url(url: str, timeout: float) -> bytes:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme == "file":
        return Path(urllib.request.url2pathname(parsed.path)).read_bytes()
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read()


def fetch_with_retries(url: str, attempts: int, timeout: float) -> bytes:
    delay = 2.0
    last_error: Exception | None = None
    for _ in range(attempts):
        try:
            return fetch_url(url, timeout)
        except (OSError, TimeoutError, urllib.error.URLError) as error:
            last_error = error
            time.sleep(delay)
            delay = min(delay * 2.0, 30.0)
    raise RuntimeError(f"fetch failed after {attempts} attempts: {url} ({last_error})")


def source_urls(record: dict[str, str], *, include_alternates: bool) -> list[str]:
    urls: list[str] = []

    def add(url: str | None) -> None:
        if url and url not in urls:
            urls.append(url)

    book_id = record.get("book_id")
    add(record.get("mirror_fetch_url"))
    if include_alternates and book_id and book_id.isdigit():
        add(f"https://www.gutenberg.org/cache/epub/{book_id}/pg{book_id}.txt")
        add(f"https://www.gutenberg.org/cache/epub/{book_id}/pg{book_id}-0.txt")
        add(f"https://www.gutenberg.org/files/{book_id}/{book_id}-0.txt")
        add(f"https://www.gutenberg.org/files/{book_id}/{book_id}.txt")
    return urls


def atomic_write(path: Path, blob: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    temp.write_bytes(blob)
    os.replace(temp, path)


def pinned_fetch_candidates(blob: bytes) -> list[tuple[str, bytes]]:
    candidates = [("fetched", blob)]
    if not blob.startswith(UTF8_BOM):
        candidates.append(("fetched_repaired_utf8_bom", UTF8_BOM + blob))
    return candidates


def matching_pinned_blob(
    blob: bytes,
    *,
    pinned_sha: str,
    pinned_size: int | None,
    alternate_url: bool,
) -> tuple[str, bytes, str] | None:
    for candidate_status, candidate_blob in pinned_fetch_candidates(blob):
        observed_sha = sha256_bytes(candidate_blob)
        observed_size = len(candidate_blob)
        if observed_sha == pinned_sha and (pinned_size is None or observed_size == pinned_size):
            if alternate_url:
                candidate_status = candidate_status.replace("fetched", "fetched_alt", 1)
            return candidate_status, candidate_blob, observed_sha
    return None


def process_record(
    repo: Path,
    record: dict[str, str],
    *,
    attempts: int,
    timeout: float,
    dry_run: bool,
    include_alternates: bool,
) -> dict[str, Any]:
    valid, reason = validate_record(record)
    book_id = record.get("book_id")
    if not valid:
        return {"book_id": book_id, "status": "invalid_record", "reason": reason}

    local_path = repo / record["local_blob_path"]
    pinned_sha = record["source_blob_sha256"]
    pinned_size = expected_size(record)

    if local_path.exists():
        observed_size = local_path.stat().st_size
        observed_sha = sha256_file(local_path)
        if observed_sha == pinned_sha and (pinned_size is None or observed_size == pinned_size):
            return {
                "book_id": book_id,
                "status": "cached",
                "bytes": observed_size,
                "sha256": observed_sha,
            }
        return {
            "book_id": book_id,
            "status": "existing_mismatch",
            "local_blob_path": record["local_blob_path"],
            "expected_sha256": pinned_sha,
            "observed_sha256": observed_sha,
            "expected_bytes": pinned_size,
            "observed_bytes": observed_size,
        }

    if dry_run:
        return {
            "book_id": book_id,
            "status": "missing",
            "local_blob_path": record["local_blob_path"],
            "mirror_fetch_url": record["mirror_fetch_url"],
        }

    failures: list[dict[str, Any]] = []
    mismatches: list[dict[str, Any]] = []
    chosen_status = None
    chosen_blob = None
    chosen_sha = None
    chosen_url = None
    urls = source_urls(record, include_alternates=include_alternates)
    for index, url in enumerate(urls):
        try:
            blob = fetch_with_retries(url, attempts, timeout)
        except Exception as error:  # noqa: BLE001 - command-line diagnostic path.
            failures.append({"url": url, "error": str(error)})
            continue
        match = matching_pinned_blob(
            blob,
            pinned_sha=pinned_sha,
            pinned_size=pinned_size,
            alternate_url=index > 0,
        )
        if match is not None:
            chosen_status, chosen_blob, chosen_sha = match
            chosen_url = url
            break
        mismatches.append(
            {
                "url": url,
                "observed_sha256": sha256_bytes(blob),
                "observed_bytes": len(blob),
            }
        )

    if chosen_blob is None or chosen_status is None or chosen_sha is None:
        status = "fetch_failed" if failures and not mismatches else "fetched_mismatch"
        return {
            "book_id": book_id,
            "status": status,
            "mirror_fetch_url": record["mirror_fetch_url"],
            "expected_sha256": pinned_sha,
            "expected_bytes": pinned_size,
            "candidate_failures": failures[:5],
            "candidate_mismatches": mismatches[:5],
        }

    atomic_write(local_path, chosen_blob)
    return {
        "book_id": book_id,
        "status": chosen_status,
        "bytes": len(chosen_blob),
        "sha256": chosen_sha,
        "source_url": chosen_url,
        "local_blob_path": record["local_blob_path"],
    }


def selected_sources(
    records: list[dict[str, str]],
    limit: int,
    book_ids: set[str],
) -> list[dict[str, str]]:
    if book_ids:
        records = [record for record in records if record.get("book_id") in book_ids]
    if limit <= 0:
        return records
    return records[:limit]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", default=DEFAULT_FIXTURE)
    parser.add_argument("--log", default=DEFAULT_LOG)
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument(
        "--book-id",
        action="append",
        default=[],
        help="hydrate only this book id; may be repeated",
    )
    parser.add_argument("--attempts", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument(
        "--url-mode",
        choices=("mirror", "all"),
        default="all",
        help="fetch only mirror_fetch_url, or also try common Gutenberg path variants",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    repo = Path(__file__).resolve().parents[3]
    fixture_path = (repo / args.fixture).resolve()
    records = selected_sources(
        parse_sources(fixture_path.read_text()),
        args.limit,
        {str(book_id) for book_id in args.book_id},
    )
    log_path = repo / args.log
    log_path.parent.mkdir(parents=True, exist_ok=True)

    counters: dict[str, int] = {}
    started = time.time()
    print(
        f"hydrating {len(records)} fixture blobs "
        f"workers={args.workers} url_mode={args.url_mode} dry_run={args.dry_run}",
        file=sys.stderr,
    )
    with log_path.open("a", encoding="utf-8") as log:
        pool = ThreadPoolExecutor(max_workers=max(1, args.workers))
        try:
            futures = [
                pool.submit(
                    process_record,
                    repo,
                    record,
                    attempts=args.attempts,
                    timeout=args.timeout,
                    dry_run=args.dry_run,
                    include_alternates=args.url_mode == "all",
                )
                for record in records
            ]
            for index, future in enumerate(as_completed(futures), start=1):
                result = future.result()
                status = str(result["status"])
                counters[status] = counters.get(status, 0) + 1
                log.write(json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n")
                if index % 25 == 0 or index == len(records):
                    elapsed = time.time() - started
                    print(
                        f"  {index:>5d}/{len(records)} "
                        + " ".join(f"{key}={value}" for key, value in sorted(counters.items()))
                        + f" elapsed={elapsed:.1f}s",
                        file=sys.stderr,
                    )
        except KeyboardInterrupt:
            pool.shutdown(wait=False, cancel_futures=True)
            print("interrupted; completed cache entries remain valid", file=sys.stderr)
            return 130
        else:
            pool.shutdown(wait=True)

    bad_statuses = {
        "invalid_record",
        "existing_mismatch",
        "fetch_failed",
        "fetched_mismatch",
    }
    print("done:", json.dumps(counters, sort_keys=True), file=sys.stderr)
    return 1 if any(counters.get(status, 0) for status in bad_statuses) else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Download or verify the S1 TinyStories raw-byte corpus files.

The script intentionally has no third-party dependencies. It reads
fixtures/corpora/tinystories.toml, writes corpus/tinystories/raw/, and verifies
byte length plus sha256 for every split. Use --verify-only to validate an
existing local corpus without touching the network.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import os
from pathlib import Path
import sys
import tempfile
from urllib.request import urlopen

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None


CHUNK_SIZE = 1024 * 1024


class VerificationError(Exception):
    pass


def workspace_root() -> Path:
    return Path(__file__).resolve().parents[1]


def load_manifest(path: Path) -> dict:
    if tomllib is not None:
        with path.open("rb") as handle:
            return tomllib.load(handle)
    return load_manifest_fallback(path)


def load_manifest_fallback(path: Path) -> dict:
    manifest: dict = {}
    current = manifest
    array_key = None
    array_values: list[str] = []
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if array_key is not None:
            if line.endswith("]"):
                current[array_key] = array_values
                array_key = None
                array_values = []
                continue
            array_values.append(ast.literal_eval(line.rstrip(",")))
            continue
        if line.startswith("[") and line.endswith("]"):
            current = manifest
            for part in line.strip("[]").split("."):
                current = current.setdefault(part, {})
            continue
        if "=" not in line:
            continue
        key, value = [part.strip() for part in line.split("=", 1)]
        if value.startswith("[") and not value.endswith("]"):
            array_key = key
            array_values = []
            continue
        if value.startswith('"'):
            current[key] = ast.literal_eval(value)
        elif value.startswith("["):
            current[key] = ast.literal_eval(value)
        else:
            current[key] = int(value)
    return manifest


def split_records(manifest: dict, selected_role: str) -> list[dict]:
    splits = manifest["splits"]
    records = [splits["train"], splits["validation"]]
    if selected_role == "all":
        return records
    return [split for split in records if split["role"] == selected_role]


def hash_file(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    length = 0
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(CHUNK_SIZE), b""):
            length += len(chunk)
            digest.update(chunk)
    return length, f"sha256:{digest.hexdigest()}"


def verify(path: Path, split: dict) -> None:
    length, sha256 = hash_file(path)
    expected_len = split["byte_length"]
    expected_sha = split["sha256"]
    if length != expected_len:
        raise VerificationError(f"{path}: expected {expected_len} bytes, observed {length}")
    if sha256 != expected_sha:
        raise VerificationError(f"{path}: expected {expected_sha}, observed {sha256}")


def download(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent
    )
    os.close(fd)
    tmp_path = Path(tmp_name)
    try:
        with urlopen(url, timeout=60) as response, tmp_path.open("wb") as output:
            while True:
                chunk = response.read(CHUNK_SIZE)
                if not chunk:
                    break
                output.write(chunk)
        tmp_path.replace(destination)
    except Exception:
        tmp_path.unlink(missing_ok=True)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--manifest",
        type=Path,
        default=workspace_root() / "fixtures" / "corpora" / "tinystories.toml",
    )
    parser.add_argument(
        "--raw-root",
        type=Path,
        help="Override the raw corpus destination. Defaults to manifest raw_root.",
    )
    parser.add_argument(
        "--verify-only",
        action="store_true",
        help="Validate existing files without downloading missing or invalid files.",
    )
    parser.add_argument(
        "--split",
        choices=["all", "train", "validation"],
        default="all",
        help="Split to download or verify. Scheduled S1 O-metric-4 uses validation only.",
    )
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    root = args.raw_root or (args.manifest.parent / manifest["raw_root"])

    for split in split_records(manifest, args.split):
        path = root / split["local_filename"]
        if not path.exists():
            if args.verify_only:
                raise SystemExit(f"{path}: missing")
            print(f"download {split['role']} -> {path}", file=sys.stderr)
            download(split["url"], path)
        try:
            verify(path, split)
        except VerificationError as error:
            if args.verify_only:
                raise SystemExit(str(error)) from error
            print(str(error), file=sys.stderr)
            print(f"redownload {split['role']} -> {path}", file=sys.stderr)
            download(split["url"], path)
            verify(path, split)
        print(f"ok {split['role']} {path}", file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

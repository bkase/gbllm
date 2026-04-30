#!/usr/bin/env python3
"""Check that a review packet names every file in the reviewed diff."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


CHANGED_FILE_HEADING_RE = re.compile(r"^#{1,6}\s+Changed File Disposition\s*$")
HEADING_RE = re.compile(r"^#{1,6}\s+")
CODE_SPAN_RE = re.compile(r"`([^`]+)`")


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    packet = (repo_root / args.packet).resolve()

    changed_files = sorted(load_changed_files(args, repo_root))
    packet_files = sorted(extract_changed_file_table(packet))

    failures: list[str] = []
    if not changed_files:
        failures.append("changed-file source produced no files; check --pr/--base/--head input")
    if not packet_files:
        failures.append(
            f"{display_path(packet, repo_root)} has no non-header entries in Changed File Disposition"
        )

    missing = sorted(set(changed_files) - set(packet_files))
    extra = sorted(set(packet_files) - set(changed_files))
    duplicates = sorted(find_duplicates(packet_files))

    if missing:
        failures.append("packet is missing changed files: " + ", ".join(missing))
    if extra and not args.allow_extra:
        failures.append("packet lists files not in the diff: " + ", ".join(extra))
    if duplicates:
        failures.append("packet lists files more than once: " + ", ".join(duplicates))

    if failures:
        print("review packet check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        print(
            "\nFix: update the packet's Changed File Disposition table so it matches "
            "the PR diff. Put base-context-only files in a separate section.",
            file=sys.stderr,
        )
        return 1

    print(
        f"review packet check passed: {len(changed_files)} changed files matched "
        f"{display_path(packet, repo_root)}"
    )
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--packet", required=True, type=Path, help="Review packet markdown file")
    parser.add_argument(
        "--changed-files",
        type=Path,
        help="Line-delimited file containing changed paths to compare against",
    )
    parser.add_argument("--pr", help="GitHub PR number passed to `gh pr diff --name-only`")
    parser.add_argument("--base", help="Base revision for `git diff --name-only base...head`")
    parser.add_argument("--head", default="HEAD", help="Head revision for --base mode")
    parser.add_argument(
        "--allow-extra",
        action="store_true",
        help="Allow packet table entries that are not in the diff",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path.cwd(),
        help="Repository root; defaults to the current directory",
    )
    args = parser.parse_args()

    sources = sum(bool(value) for value in (args.changed_files, args.pr, args.base))
    if sources != 1:
        parser.error("choose exactly one changed-file source: --changed-files, --pr, or --base")
    return args


def load_changed_files(args: argparse.Namespace, repo_root: Path) -> set[str]:
    if args.changed_files:
        path = (repo_root / args.changed_files).resolve()
        return normalize_lines(path.read_text(encoding="utf-8").splitlines())

    if args.pr:
        output = run(["gh", "pr", "diff", str(args.pr), "--name-only"], repo_root)
        return normalize_lines(output.splitlines())

    output = run(["git", "diff", "--name-only", f"{args.base}...{args.head}"], repo_root)
    return normalize_lines(output.splitlines())


def extract_changed_file_table(packet: Path) -> list[str]:
    lines = packet.read_text(encoding="utf-8").splitlines()
    start = None
    for index, line in enumerate(lines):
        if CHANGED_FILE_HEADING_RE.match(line.strip()):
            start = index + 1
            break

    if start is None:
        raise SystemExit(
            f"review packet check failed:\n- {packet} is missing a "
            "'Changed File Disposition' heading"
        )

    entries: list[str] = []
    for line in lines[start:]:
        stripped = line.strip()
        if HEADING_RE.match(stripped):
            break
        if not stripped.startswith("|"):
            continue

        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        if not cells:
            continue
        first = cells[0]
        if not first or first.lower() == "file" or set(first) <= {"-", ":", " "}:
            continue

        path = extract_path_cell(first)
        if path:
            entries.append(path)

    return entries


def extract_path_cell(cell: str) -> str | None:
    match = CODE_SPAN_RE.search(cell)
    path = match.group(1) if match else cell
    path = path.strip()
    if not path or path.lower() in {"n/a", "none"}:
        return None
    return normalize_path(path)


def normalize_lines(lines: list[str]) -> set[str]:
    return {normalize_path(line) for line in lines if line.strip()}


def normalize_path(path: str) -> str:
    return path.strip().removeprefix("./")


def find_duplicates(values: list[str]) -> set[str]:
    seen: set[str] = set()
    duplicates: set[str] = set()
    for value in values:
        if value in seen:
            duplicates.add(value)
        seen.add(value)
    return duplicates


def display_path(path: Path, repo_root: Path) -> str:
    try:
        return str(path.relative_to(repo_root))
    except ValueError:
        return str(path)


def run(cmd: list[str], cwd: Path) -> str:
    try:
        return subprocess.check_output(cmd, cwd=cwd, text=True, stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as exc:
        print(f"command failed: {' '.join(cmd)}", file=sys.stderr)
        if exc.stderr:
            print(exc.stderr, file=sys.stderr, end="")
        raise SystemExit(exc.returncode) from exc


if __name__ == "__main__":
    raise SystemExit(main())

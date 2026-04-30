#!/usr/bin/env python3
"""Check that a PR only changes allowed bead issue records."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


ISSUES_PATH = ".beads/issues.jsonl"
WORKTREE = "WORKTREE"
INDEX = "INDEX"


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    allowlist = parse_allowlist(args.allow, args.allow_file, repo_root)

    base = load_issues(args.base, repo_root, args.issues_path)
    head = load_issues(args.head, repo_root, args.issues_path)
    changed = changed_issue_ids(base, head)
    unexpected = sorted(changed - allowlist)

    if unexpected:
        print("beads PR scope check failed:", file=sys.stderr)
        print(
            "- changed bead issue ids outside allowlist: " + ", ".join(unexpected),
            file=sys.stderr,
        )
        print(
            "- allowed issue ids: "
            + (", ".join(sorted(allowlist)) if allowlist else "(none)"),
            file=sys.stderr,
        )
        print(
            "\nFix: resolve .beads/issues.jsonl from the branch that owns those issues, "
            "or add the intentional issue ids to --allow.",
            file=sys.stderr,
        )
        return 1

    changed_text = ", ".join(sorted(changed)) if changed else "(none)"
    print(f"beads PR scope check passed: changed issue ids {changed_text}")
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base",
        default="origin/main",
        help=f"Base git revision containing {ISSUES_PATH}; default: origin/main",
    )
    parser.add_argument(
        "--head",
        default=WORKTREE,
        help=f"Head source: git revision, {WORKTREE}, or {INDEX}; default: {WORKTREE}",
    )
    parser.add_argument(
        "--allow",
        action="append",
        default=[],
        help="Allowed issue id or comma-separated issue ids; may be repeated",
    )
    parser.add_argument("--allow-file", type=Path, help="Line-delimited allowed issue ids")
    parser.add_argument(
        "--issues-path",
        default=ISSUES_PATH,
        help=f"Path to issue JSONL within the repo; default: {ISSUES_PATH}",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path.cwd(),
        help="Repository root; defaults to the current directory",
    )
    return parser.parse_args()


def parse_allowlist(raw_values: list[str], allow_file: Path | None, repo_root: Path) -> set[str]:
    allowed: set[str] = set()
    for raw in raw_values:
        allowed.update(parse_id_list(raw.split(",")))

    if allow_file is not None:
        path = (repo_root / allow_file).resolve()
        allowed.update(parse_id_list(path.read_text(encoding="utf-8").splitlines()))

    return allowed


def parse_id_list(values: list[str]) -> set[str]:
    issue_ids: set[str] = set()
    for value in values:
        stripped = value.strip()
        if stripped and not stripped.startswith("#"):
            issue_ids.add(stripped)
    return issue_ids


def load_issues(source: str, repo_root: Path, issues_path: str) -> dict[str, dict[str, Any]]:
    text = load_issues_text(source, repo_root, issues_path)
    issues: dict[str, dict[str, Any]] = {}
    for line_number, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if not stripped:
            continue
        try:
            issue = json.loads(stripped)
        except json.JSONDecodeError as exc:
            raise SystemExit(
                f"failed to parse {issues_path} from {source} at line {line_number}: {exc}"
            ) from exc
        issue_id = issue.get("id")
        if not isinstance(issue_id, str):
            raise SystemExit(f"{issues_path} from {source} line {line_number} has no string id")
        issues[issue_id] = issue
    return issues


def load_issues_text(source: str, repo_root: Path, issues_path: str) -> str:
    if source == WORKTREE:
        return (repo_root / issues_path).read_text(encoding="utf-8")
    if source == INDEX:
        return run(["git", "show", f":{issues_path}"], repo_root)
    return run(["git", "show", f"{source}:{issues_path}"], repo_root)


def changed_issue_ids(
    base: dict[str, dict[str, Any]],
    head: dict[str, dict[str, Any]],
) -> set[str]:
    issue_ids = set(base) | set(head)
    return {issue_id for issue_id in issue_ids if base.get(issue_id) != head.get(issue_id)}


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

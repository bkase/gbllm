#!/usr/bin/env bash
set -euo pipefail

python3 - "$@" <<'PY'
from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


RFC_DEFAULT = "history/rfcs/F-S7-moe-beats-dense.md"
OUTPUT_DEFAULT = "fixtures/preregistration/s7.toml"
START_HEADING_DEFAULT = "# 1. Hypothesis algebra"
END_HEADING_DEFAULT = "# 2. Authority rules"
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
PASS_VERSION_RE = re.compile(r"^(?:\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?|s7-[a-z0-9][a-z0-9._-]*)$")
PLACEHOLDER_PASS_VERSION_RE = re.compile(
    r"(?:draft|pre[-_ ]?implementation|estimate|placeholder|fixture|self[-_ ]?test)",
    re.IGNORECASE,
)


class PinError(Exception):
    pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Emit fixtures/preregistration/s7.toml for the committed F-S7 "
            "prediction block. Refuses to emit if the current RFC prediction "
            "section differs from predictions_commit."
        )
    )
    parser.add_argument("--rfc-path", default=RFC_DEFAULT)
    parser.add_argument("--output", default=OUTPUT_DEFAULT, help="pin output path, or '-' for stdout")
    parser.add_argument(
        "--pass-version",
        help="final pass_version_S7 pin id; required unless --check-ready is used",
    )
    parser.add_argument("--predictions-commit", default="HEAD")
    parser.add_argument("--rfc-revision", default="HEAD")
    parser.add_argument("--first-result-commit", default="")
    parser.add_argument("--start-heading", default=START_HEADING_DEFAULT)
    parser.add_argument("--end-heading", default=END_HEADING_DEFAULT)
    parser.add_argument(
        "--check-ready",
        action="store_true",
        help="validate that the current RFC prediction block can be pinned, without writing s7.toml",
    )
    args = parser.parse_args(sys.argv[1:])

    try:
        repo = git_root()
        rfc_path = normalize_repo_path(args.rfc_path, "rfc_path")
        output = args.output
        predictions_commit = resolve_commit(repo, args.predictions_commit, "predictions_commit")
        rfc_revision = resolve_commit(repo, args.rfc_revision, "rfc_revision")
        first_result_commit = normalize_first_result_commit(
            repo, args.first_result_commit, predictions_commit
        )
        head_commit = resolve_commit(repo, "HEAD", "HEAD")
        require_ancestor(repo, predictions_commit, head_commit, "predictions_commit")
        require_ancestor(repo, rfc_revision, head_commit, "rfc_revision")

        current_text = (repo / rfc_path).read_text(encoding="utf-8")
        start, end, current_section = extract_heading_section(
            current_text,
            args.start_heading,
            args.end_heading,
            "current worktree",
        )
        committed_text = git(repo, ["show", f"{predictions_commit}:{rfc_path}"], text=True)
        committed_start, committed_end, committed_section = extract_heading_section(
            committed_text,
            args.start_heading,
            args.end_heading,
            "predictions_commit",
        )
        if (committed_start, committed_end) != (start, end):
            raise PinError(
                "current RFC prediction heading line range differs from predictions_commit\n"
                f"  rfc_path={rfc_path}\n"
                f"  current_line_range={start}..{end}\n"
                f"  predictions_commit_line_range={committed_start}..{committed_end}\n"
                f"  predictions_commit={predictions_commit}\n"
                "  pin schema records line numbers; commit the current RFC "
                "prediction block before emitting s7.toml\n"
                "offending_diff_hunk:\n"
                f"{diff_hunk(committed_section, current_section, 'predictions_commit', 'current_worktree')}"
            )
        if committed_section != current_section:
            raise PinError(
                "current RFC predictions section differs from predictions_commit\n"
                f"  rfc_path={rfc_path}\n"
                f"  current_line_range={start}..{end}\n"
                f"  predictions_commit_line_range={committed_start}..{committed_end}\n"
                f"  predictions_commit={predictions_commit}\n"
                "offending_diff_hunk:\n"
                f"{diff_hunk(committed_section, current_section, 'predictions_commit', 'current_worktree')}"
            )

        digest = predictions_hash(rfc_path, start, end, current_section)
        if args.check_ready:
            print(
                json.dumps(
                    {
                        "script": "s7_preregistration_pin",
                        "ready": True,
                        "rfc_path": str(rfc_path),
                        "line_range": f"{start}..{end}",
                        "predictions_commit": predictions_commit,
                        "predictions_section_hash": digest,
                        "rfc_revision": rfc_revision,
                    },
                    sort_keys=True,
                    separators=(",", ":"),
                )
            )
            return 0

        pass_version = validate_pass_version(args.pass_version)
        pin_text = render_pin(
            rfc_path,
            start,
            end,
            predictions_commit,
            digest,
            pass_version,
            rfc_revision,
            first_result_commit,
        )
        if output == "-":
            print(pin_text, end="")
        else:
            output_path = normalize_repo_path(output, "--output")
            full_output = repo / output_path
            full_output.parent.mkdir(parents=True, exist_ok=True)
            full_output.write_text(pin_text, encoding="utf-8")
            print(f"wrote {output_path}")
        return 0
    except Exception as error:
        print(error, file=sys.stderr)
        return 1


def validate_pass_version(value: Any) -> str:
    if not isinstance(value, str) or not value.strip():
        raise PinError("pass_version_S7 must be a non-empty final pin id")
    version = value.strip()
    if PLACEHOLDER_PASS_VERSION_RE.search(version):
        raise PinError("pass_version_S7 must be finalized, not a draft/fixture/self-test placeholder")
    if not PASS_VERSION_RE.fullmatch(version):
        raise PinError("pass_version_S7 must be semver or an s7-* final pin id")
    return version


def normalize_first_result_commit(repo: Path, value: str, predictions_commit: str) -> str:
    if value == "":
        return ""
    commit = resolve_commit(repo, value, "first_result_commit")
    if commit == predictions_commit or not git_is_ancestor(repo, predictions_commit, commit):
        raise PinError("predictions_commit must be a strict ancestor of first_result_commit")
    return commit


def resolve_commit(repo: Path, value: str, field: str) -> str:
    try:
        commit = git(repo, ["rev-parse", "--verify", f"{value}^{{commit}}"], text=True).strip()
    except PinError as error:
        raise PinError(f"{field} does not name an existing commit: {value}") from error
    if not COMMIT_RE.fullmatch(commit):
        raise PinError(f"{field} must resolve to a lowercase 40-character git commit id")
    return commit


def require_ancestor(repo: Path, ancestor: str, descendant: str, field: str) -> None:
    if not git_is_ancestor(repo, ancestor, descendant):
        if field == "predictions_commit":
            raise PinError("predictions_commit must be an ancestor of HEAD/current checkout")
        if field == "rfc_revision":
            raise PinError("rfc_revision must be an ancestor of HEAD/current checkout")
        raise PinError(f"{field} must be an ancestor of HEAD/current checkout")


def git_is_ancestor(repo: Path, ancestor: str, descendant: str) -> bool:
    completed = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=repo,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return completed.returncode == 0


def extract_heading_section(
    text: str,
    start_heading: str,
    end_heading: str,
    source: str,
) -> tuple[int, int, str]:
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    start = None
    for index, line in enumerate(lines, 1):
        if line == start_heading:
            start = index
            break
    if start is None:
        raise PinError(f"{source} missing start heading {start_heading!r}")
    for index in range(start + 1, len(lines) + 1):
        if lines[index - 1] == end_heading:
            end = index - 1
            break
    else:
        raise PinError(f"{source} missing end heading {end_heading!r}")
    while end >= start and not lines[end - 1].strip():
        end -= 1
    return start, end, "\n".join(lines[start - 1 : end]).strip()


def predictions_hash(path: Path, start: int, end: int, section: str) -> str:
    payload = {
        "path": str(path),
        "start_line": start,
        "end_line": end,
        "section": section.strip(),
    }
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def render_pin(
    rfc_path: Path,
    start: int,
    end: int,
    predictions_commit: str,
    predictions_section_hash: str,
    pass_version: str,
    rfc_revision: str,
    first_result_commit: str,
) -> str:
    fields = [
        'schema = "s7_preregistration.v1"',
        f'rfc_path = "{rfc_path}"',
        f"predictions_line_start = {start}",
        f"predictions_line_end = {end}",
        f'predictions_commit = "{predictions_commit}"',
        f'predictions_section_hash = "{predictions_section_hash}"',
        f'pass_version_S7 = "{pass_version}"',
        f'rfc_revision = "{rfc_revision}"',
        "",
        "# TOML has no null value. Empty string is the preregistration null sentinel",
        "# until the first results-bearing S7 PR records the earliest result commit.",
        f'first_result_commit = "{first_result_commit}"',
        "",
    ]
    return "\n".join(fields)


def diff_hunk(expected: str, observed: str, expected_label: str, observed_label: str) -> str:
    diff = difflib.unified_diff(
        expected.splitlines(),
        observed.splitlines(),
        fromfile=expected_label,
        tofile=observed_label,
        lineterm="",
    )
    return "\n".join(list(diff)[:80]) or "(no textual diff; check line endings)"


def git_root() -> Path:
    return Path(git(Path.cwd(), ["rev-parse", "--show-toplevel"], text=True).strip())


def git(repo: Path, args: list[str], text: bool) -> Any:
    completed = subprocess.run(
        ["git", *args],
        cwd=repo,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=text,
        check=False,
    )
    if completed.returncode != 0:
        stderr = completed.stderr if text else completed.stderr.decode("utf-8", errors="replace")
        raise PinError(f"git {' '.join(args)} failed: {stderr.strip()}")
    return completed.stdout


def normalize_repo_path(value: Any, field: str) -> Path:
    if not isinstance(value, str) or not value:
        raise PinError(f"{field} must be a non-empty repo-relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts:
        raise PinError(f"{field} must be repo-relative and stay within the repo")
    return path


if __name__ == "__main__":
    sys.exit(main())
PY

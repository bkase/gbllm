#!/usr/bin/env bash
set -euo pipefail

python3 - "$@" <<'PY'
import argparse
import json
import hashlib
import re
import subprocess
import sys
from pathlib import Path

REPORT_DEFAULT = "docs/experiments/S1-report.md"
ARTIFACT_DEFAULT = "experiments/S1"
RESULT_HASH_RE = re.compile(
    rb'"(?:checkpoint|score|negative|ablation|baseline)_self_hash"\s*:\s*"sha256:[0-9a-f]{64}"'
)
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
PREDICTIONS_MARKER = "## Pre-registered predictions\n\n"
OBSERVED_MARKER = "\n## Observed\n"


class PreregError(Exception):
    pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Verify F-S1 preregistration ordering and predictions hash."
    )
    parser.add_argument(
        "--report",
        default=REPORT_DEFAULT,
        help=f"report path relative to the git root (default: {REPORT_DEFAULT})",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit a single JSON status object on stdout",
    )
    parser.add_argument(
        "--artifact-dir",
        action="append",
        dest="artifact_dirs",
        default=None,
        help=(
            "repo-relative S1 result artifact directory to scan for "
            "first_result_commit; repeat or comma-separate for multiple "
            "directories (default: experiments/S1)"
        ),
    )
    args = parser.parse_args(sys.argv[1:])

    try:
        repo = git(["rev-parse", "--show-toplevel"], text=True).strip()
        report_path = args.report
        artifact_dirs = normalize_artifact_dirs(args.artifact_dirs)
        current = Path(repo, report_path).read_text(encoding="utf-8")
        front, body = parse_report(current)
        section = predictions_section(body)
        front_hash = expect_hash(front.get("predictions_section_hash"), "predictions_section_hash")
        current_hash = predictions_hash(section)
        if current_hash != front_hash:
            raise PreregError(
                "predictions_section_hash mismatch in current report\n"
                f"  expected_from_body={current_hash}\n"
                f"  observed_front_matter={front_hash}"
            )
        ok("predictions_section_hash matches current report")

        predictions_commit = optional_commit(front.get("predictions_commit"), "predictions_commit")
        first_result_commit = optional_commit(front.get("first_result_commit"), "first_result_commit")

        if predictions_commit is not None:
            ensure_commit_exists(predictions_commit, "predictions_commit")
            pred_report = git_show_text(predictions_commit, report_path)
            _, pred_body = parse_report(pred_report)
            pred_hash = predictions_hash(predictions_section(pred_body))
            if pred_hash != front_hash:
                raise PreregError(
                    "predictions_section_hash does not match predictions_commit section\n"
                    f"  predictions_commit={predictions_commit}\n"
                    f"  expected_from_predictions_commit={pred_hash}\n"
                    f"  observed_front_matter={front_hash}"
                )
            ok("predictions_section_hash matches predictions_commit")
        else:
            ok("predictions_commit is not set yet; pre-result mode")

        earliest_result = earliest_result_commit(report_path, artifact_dirs)
        if first_result_commit is None:
            if earliest_result is not None:
                raise PreregError(
                    "first_result_commit is null but result artifacts already exist in history\n"
                    f"  earliest_result_commit={earliest_result}"
                )
            ok("no result commit recorded yet")
        else:
            ensure_commit_exists(first_result_commit, "first_result_commit")
            if earliest_result is None:
                raise PreregError(
                    "first_result_commit is set but no S1 result artifact commit was found\n"
                    f"  first_result_commit={first_result_commit}"
                )
            if earliest_result != first_result_commit:
                raise PreregError(
                    "first_result_commit is not the earliest S1 result artifact commit\n"
                    f"  expected_earliest_result_commit={earliest_result}\n"
                    f"  observed_front_matter_first_result_commit={first_result_commit}"
                )
            if predictions_commit is None:
                raise PreregError("predictions_commit is required once first_result_commit is set")
            if predictions_commit == first_result_commit:
                raise PreregError(
                    "predictions_commit must be strictly before first_result_commit\n"
                    f"  predictions_commit={predictions_commit}\n"
                    f"  first_result_commit={first_result_commit}"
                )
            if subprocess.run(
                ["git", "merge-base", "--is-ancestor", predictions_commit, first_result_commit],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            ).returncode != 0:
                raise PreregError(
                    "predictions commit not an ancestor of first result\n"
                    f"  predictions_commit={predictions_commit}\n"
                    f"  first_result_commit={first_result_commit}"
                )
            ok("predictions_commit is a strict ancestor of first_result_commit")
            ok("first_result_commit is the earliest result artifact commit")

        ok("preregistration check passed")
        if args.json:
            print(json.dumps({"result": "PASS", "predictions_section_hash": front_hash}, sort_keys=True))
        return 0
    except (OSError, subprocess.CalledProcessError, PreregError, json.JSONDecodeError) as error:
        fail(str(error))
        if args.json:
            print(json.dumps({"result": "FAIL", "diagnostic": str(error)}, sort_keys=True))
        return 1


def parse_report(text: str):
    if not text.startswith("---\n"):
        raise PreregError("report must start with front-matter marker")
    try:
        front_raw, body = text[4:].split("\n---\n", 1)
    except ValueError as error:
        raise PreregError("report is missing closing front-matter marker") from error
    front = json.loads(front_raw)
    if not isinstance(front, dict):
        raise PreregError("report front matter must be a JSON object")
    return front, body


def predictions_section(body: str) -> str:
    start = body.find(PREDICTIONS_MARKER)
    if start < 0:
        raise PreregError("missing ## Pre-registered predictions section")
    start += len(PREDICTIONS_MARKER)
    end = body.find(OBSERVED_MARKER, start)
    if end < 0:
        raise PreregError("missing ## Observed section after predictions")
    return body[start:end].strip()


def predictions_hash(section: str) -> str:
    # Keep this byte-for-byte aligned with
    # gbf_experiments::s1::report::predictions_section_hash:
    # sha256(S1CanonicalJson::to_vec(markdown.trim())).
    # The section is stripped at both boundaries before hashing. For a markdown
    # string, S1CanonicalJson is JSON string canonicalization: no insignificant
    # whitespace around the JSON value, escaped as needed, and UTF-8 encoded
    # with ensure_ascii=false.
    canonical = json.dumps(
        section.strip(),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(canonical).hexdigest()


def earliest_result_commit(report_path: str, artifact_dirs: list[str]):
    paths = result_scan_paths(report_path, artifact_dirs)
    commits = git(
        ["rev-list", "--reverse", "HEAD", "--", *paths],
        text=True,
    ).splitlines()
    for commit in commits:
        for path in tracked_files_at(commit, report_path, artifact_dirs):
            payload = git_show_bytes(commit, path)
            if RESULT_HASH_RE.search(payload):
                return commit
    return None


def result_scan_paths(report_path: str, artifact_dirs: list[str]):
    paths = [*artifact_dirs, report_path]
    return list(dict.fromkeys(paths))


def tracked_files_at(commit: str, report_path: str, artifact_dirs: list[str]):
    paths = git(
        [
            "ls-tree",
            "-r",
            "--name-only",
            commit,
            "--",
            *result_scan_paths(report_path, artifact_dirs),
        ],
        text=True,
    ).splitlines()
    return [
        path
        for path in paths
        if path == report_path
        or any(
            path == artifact_dir or path.startswith(f"{artifact_dir}/")
            for artifact_dir in artifact_dirs
        )
    ]


def normalize_artifact_dirs(values) -> list[str]:
    if values is None:
        values = [ARTIFACT_DEFAULT]
    result = []
    for value in values:
        for raw in value.split(","):
            path = normalize_repo_path(raw, "--artifact-dir")
            if path == ".":
                raise PreregError("--artifact-dir must not be the repository root")
            if path not in result:
                result.append(path)
    return result


def normalize_repo_path(value: str, field: str) -> str:
    raw = value.strip().replace("\\", "/")
    while raw.startswith("./"):
        raw = raw[2:]
    raw = raw.rstrip("/")
    if not raw:
        raise PreregError(f"{field} cannot be empty")
    path = Path(raw)
    if path.is_absolute() or ".." in path.parts:
        raise PreregError(f"{field} must be a repo-relative path without '..'")
    return raw


def expect_hash(value, field: str) -> str:
    if not isinstance(value, str) or not re.match(r"^sha256:[0-9a-f]{64}$", value):
        raise PreregError(f"{field} must be a sha256 hash string")
    return value


def optional_commit(value, field: str):
    if value is None:
        return None
    if not isinstance(value, str) or not COMMIT_RE.match(value):
        raise PreregError(f"{field} must be null or a 40-character lowercase git commit id")
    return value


def ensure_commit_exists(commit: str, field: str) -> None:
    subprocess.run(
        ["git", "cat-file", "-e", f"{commit}^{{commit}}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=True,
    )


def git(args, *, text: bool):
    return subprocess.check_output(["git", *args], text=text)


def git_show_text(commit: str, path: str) -> str:
    try:
        return git(["show", f"{commit}:{path}"], text=True)
    except subprocess.CalledProcessError as error:
        raise PreregError(f"could not read {path} at {commit}") from error


def git_show_bytes(commit: str, path: str) -> bytes:
    return subprocess.check_output(["git", "show", f"{commit}:{path}"])


def ok(message: str) -> None:
    print(f"[PREREG OK] {message}", file=sys.stderr)


def fail(message: str) -> None:
    print(f"[PREREG FAIL] {message}", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
PY

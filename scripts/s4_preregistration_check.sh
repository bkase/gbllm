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

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback.
    tomllib = None


PIN_DEFAULT = "fixtures/preregistration/s4.toml"
OUTPUT_DEFAULT = "/tmp/s4-preregistration.json"
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
RESULT_HASH_RE = re.compile(
    rb'"(?:gutenberg_manifest_self_hash|checkpoint_self_hash|run_log_self_hash|'
    rb'score_self_hash|fp_reference_self_hash|oracle_agreement_self_hash|'
    rb'corpus_quality_self_hash|corpus_progression_self_hash|'
    rb'contamination_self_hash|promotion_gate_self_hash|'
    rb'baseline_gutenberg_self_hash)"\s*:\s*"sha256:[0-9a-f]{64}"'
)


class PreregError(Exception):
    def __init__(self, stage: int, detail: str, extra: dict[str, Any] | None = None):
        super().__init__(detail)
        self.stage = stage
        self.detail = detail
        self.extra = extra or {}


events: list[dict[str, Any]] = []


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Verify F-S4 preregistration ordering and predictions hash. "
            "The hash covers the RFC predictions line range recorded in "
            "fixtures/preregistration/s4.toml using the S1CanonicalJson-style "
            "canonical JSON encoder."
        )
    )
    parser.add_argument("--pin", default=PIN_DEFAULT)
    parser.add_argument("--output", "--report-path", dest="output", default=OUTPUT_DEFAULT)
    parser.add_argument("--dry-run", action="store_true", help="validate without requiring result artifacts")
    parser.add_argument(
        "--artifact-path",
        action="append",
        default=None,
        help=(
            "repo-relative S4 result artifact path or directory to scan; "
            "repeat for multiple paths (default: experiments/S4 and "
            "docs/experiments/S4-report.md)"
        ),
    )
    parser.add_argument("--json", action="store_true", help="emit final JSON on stdout")
    args = parser.parse_args(sys.argv[1:])

    repo = git_root()
    output_path = Path(args.output)
    try:
        pin_path = normalize_repo_path(repo, args.pin, "--pin")
        pin = load_pin(repo / pin_path)
        rfc_path = normalize_repo_path(repo, pin["rfc_path"], "rfc_path")
        artifact_paths = [
            normalize_repo_path(repo, path, "--artifact-path")
            for path in (args.artifact_path or ["experiments/S4", "docs/experiments/S4-report.md"])
        ]

        emit(
            {
                "event": "s4_prereg_check_started",
                "pin": str(pin_path),
                "rfc": str(rfc_path),
                "line_range": line_range(pin),
            }
        )
        stage_start(1, "validate frozen predictions block")
        current_section = extract_line_range((repo / rfc_path).read_text(encoding="utf-8"), pin)
        current_hash = predictions_hash(rfc_path, pin, current_section)
        expected_hash = expect_hash(pin["predictions_section_hash"], "predictions_section_hash")
        if current_hash != expected_hash:
            expected_section = section_from_commit(repo, pin["predictions_commit"], rfc_path, pin)
            hunk = diff_hunk(
                rfc_path,
                pin,
                expected_section,
                current_section,
                "predictions_commit",
                "current_worktree",
            )
            raise PreregError(
                1,
                (
                    "predictions_section_hash mismatch in current RFC\n"
                    f"  line_range={line_range(pin)}\n"
                    f"  expected_from_pin={expected_hash}\n"
                    f"  observed_from_current={current_hash}\n"
                    "offending_diff_hunk:\n"
                    f"{hunk}"
                ),
                {"expected": expected_hash, "observed": current_hash, "line_range": line_range(pin)},
            )

        predictions_commit = required_commit(pin["predictions_commit"], "predictions_commit")
        rfc_revision = required_commit(pin["rfc_revision"], "rfc_revision")
        ensure_commit_exists(repo, predictions_commit, "predictions_commit")
        ensure_commit_exists(repo, rfc_revision, "rfc_revision")
        committed_section = section_from_commit(repo, predictions_commit, rfc_path, pin)
        committed_hash = predictions_hash(rfc_path, pin, committed_section)
        if committed_hash != expected_hash:
            hunk = diff_hunk(
                rfc_path,
                pin,
                current_section,
                committed_section,
                "current_worktree",
                "predictions_commit",
            )
            raise PreregError(
                1,
                (
                    "predictions_section_hash does not match predictions_commit section\n"
                    f"  line_range={line_range(pin)}\n"
                    f"  expected_from_pin={expected_hash}\n"
                    f"  observed_from_predictions_commit={committed_hash}\n"
                    "offending_diff_hunk:\n"
                    f"{hunk}"
                ),
                {"expected": expected_hash, "observed": committed_hash, "line_range": line_range(pin)},
            )
        stage_done("predictions_hash", 1, True, {"predictions_section_hash": expected_hash})

        stage_start(2, "validate result ordering")
        first_result_commit = normalize_optional_commit(pin.get("first_result_commit"))
        if first_result_commit is None:
            current_result = first_result_path_in_worktree(repo, artifact_paths)
            if current_result is not None:
                raise PreregError(
                    2,
                    (
                        "first_result_commit is unset but S4 result evidence exists in the worktree\n"
                        f"  result_path={current_result}"
                    ),
                    {"result_path": current_result},
                )
            found = earliest_result_commit(repo, predictions_commit, artifact_paths)
            if found is not None:
                raise PreregError(
                    2,
                    (
                        "first_result_commit is unset but S4 result evidence exists\n"
                        f"  earliest_result_commit={found[0]}\n"
                        f"  result_path={found[1]}"
                    ),
                    {"earliest_result_commit": found[0], "result_path": found[1]},
                )
            stage_done(
                "pre_result_scan",
                2,
                True,
                {"first_result_commit": None, "artifact_paths": [str(path) for path in artifact_paths]},
            )
        else:
            ensure_commit_exists(repo, first_result_commit, "first_result_commit")
            if predictions_commit == first_result_commit or not git_is_ancestor(
                repo, predictions_commit, first_result_commit
            ):
                raise PreregError(
                    2,
                    "predictions_commit must be a strict ancestor of first_result_commit",
                    {"predictions_commit": predictions_commit, "first_result_commit": first_result_commit},
                )
            found = earliest_result_commit(repo, predictions_commit, artifact_paths)
            if found is None:
                raise PreregError(2, "first_result_commit is set but no S4 result evidence was found")
            if found[0] != first_result_commit:
                raise PreregError(
                    2,
                    (
                        "first_result_commit is not the earliest S4 result artifact commit\n"
                        f"  expected_earliest_result_commit={found[0]}\n"
                        f"  observed_front_matter_first_result_commit={first_result_commit}\n"
                        f"  result_path={found[1]}"
                    ),
                    {
                        "expected_earliest_result_commit": found[0],
                        "observed_first_result_commit": first_result_commit,
                        "result_path": found[1],
                    },
                )
            stage_done(
                "first_result_ordering",
                2,
                True,
                {"first_result_commit": first_result_commit, "result_path": found[1]},
            )

        stage_start(3, "validate pin history")
        if first_result_commit is not None:
            offenders = [
                commit
                for commit in commits_touching_path(repo, pin_path)
                if not git_is_ancestor(repo, commit, first_result_commit)
            ]
            if offenders:
                raise PreregError(
                    3,
                    (
                        "commit touching fixtures/preregistration/s4.toml is not an ancestor of "
                        f"first_result_commit\n  offending_commits={','.join(offenders)}"
                    ),
                    {"offending_commits": offenders, "first_result_commit": first_result_commit},
                )
        stage_done("pin_history", 3, True, {"first_result_commit": first_result_commit})
        return finish(True, 0, output_path, args.json, dry_run=args.dry_run)
    except PreregError as error:
        return finish(False, 1, output_path, args.json, error, dry_run=args.dry_run)
    except Exception as error:
        wrapped = PreregError(len(events) + 1, str(error))
        return finish(False, 1, output_path, args.json, wrapped, dry_run=args.dry_run)


def load_pin(path: Path) -> dict[str, Any]:
    data = tomllib.loads(path.read_text(encoding="utf-8")) if tomllib is not None else parse_string_toml(path)
    required = {
        "schema",
        "rfc_path",
        "predictions_line_start",
        "predictions_line_end",
        "predictions_commit",
        "predictions_section_hash",
        "pass_version_S4",
        "rfc_revision",
        "first_result_commit",
    }
    missing = sorted(required.difference(data))
    if missing:
        raise PreregError(1, f"pin is missing required fields: {', '.join(missing)}")
    if data["schema"] != "s4_preregistration.v1":
        raise PreregError(1, "pin schema must be s4_preregistration.v1")
    if not isinstance(data["predictions_line_start"], int) or not isinstance(data["predictions_line_end"], int):
        raise PreregError(1, "predictions_line_start/end must be integers")
    if data["predictions_line_start"] < 1 or data["predictions_line_end"] < data["predictions_line_start"]:
        raise PreregError(1, "predictions line range is invalid")
    return data


def parse_string_toml(path: Path) -> dict[str, Any]:
    data: dict[str, Any] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.split("#", 1)[0].strip()
        if not stripped:
            continue
        key, raw = stripped.split("=", 1)
        data[key.strip()] = json.loads(raw.strip())
    return data


def predictions_hash(path: Path, pin: dict[str, Any], section: str) -> str:
    payload = {
        "path": str(path),
        "start_line": pin["predictions_line_start"],
        "end_line": pin["predictions_line_end"],
        "section": section.strip(),
    }
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def extract_line_range(text: str, pin: dict[str, Any]) -> str:
    normalized = text.replace("\r\n", "\n").replace("\r", "\n")
    lines = normalized.split("\n")
    start = pin["predictions_line_start"]
    end = pin["predictions_line_end"]
    if end > len(lines):
        raise PreregError(1, f"predictions line range {line_range(pin)} exceeds RFC length {len(lines)}")
    return "\n".join(lines[start - 1 : end]).strip()


def section_from_commit(repo: Path, commit: str, path: Path, pin: dict[str, Any]) -> str:
    text = git(repo, ["show", f"{commit}:{path}"], text=True)
    return extract_line_range(text, pin)


def diff_hunk(
    path: Path,
    pin: dict[str, Any],
    expected: str,
    observed: str,
    expected_label: str,
    observed_label: str,
) -> str:
    expected_lines = expected.splitlines()
    observed_lines = observed.splitlines()
    diff = list(
        difflib.unified_diff(
            expected_lines,
            observed_lines,
            fromfile=f"{expected_label}:{path}:{pin['predictions_line_start']}",
            tofile=f"{observed_label}:{path}:{pin['predictions_line_start']}",
            lineterm="",
        )
    )
    return "\n".join(diff[:80]) if diff else "(no textual diff; check line endings or canonical fields)"


def earliest_result_commit(repo: Path, predictions_commit: str, paths: list[Path]) -> tuple[str, str] | None:
    existing_args = [str(path) for path in paths]
    commits = git(
        repo,
        ["rev-list", "--reverse", f"{predictions_commit}..HEAD", "--", *existing_args],
        text=True,
    ).splitlines()
    for commit in commits:
        found_path = first_result_path_at_commit(repo, commit, paths)
        if found_path is not None:
            return commit, found_path
    return None


def first_result_path_at_commit(repo: Path, commit: str, paths: list[Path]) -> str | None:
    for path in iter_files_at_commit(repo, commit, paths):
        blob = git(repo, ["show", f"{commit}:{path}"], text=False)
        if RESULT_HASH_RE.search(blob):
            return str(path)
    return None


def first_result_path_in_worktree(repo: Path, paths: list[Path]) -> str | None:
    for path in paths:
        full = repo / path
        candidates: list[Path]
        if full.is_dir():
            candidates = sorted(candidate for candidate in full.rglob("*") if candidate.is_file())
        elif full.is_file():
            candidates = [full]
        else:
            candidates = []
        for candidate in candidates:
            try:
                if RESULT_HASH_RE.search(candidate.read_bytes()):
                    return str(candidate.relative_to(repo))
            except OSError:
                continue
    return None


def iter_files_at_commit(repo: Path, commit: str, paths: list[Path]) -> list[Path]:
    files: list[Path] = []
    for path in paths:
        completed = subprocess.run(
            ["git", "ls-tree", "-r", "--name-only", commit, "--", str(path)],
            cwd=repo,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        if completed.returncode == 0:
            files.extend(Path(line) for line in completed.stdout.splitlines() if line)
    return sorted(set(files))


def commits_touching_path(repo: Path, path: Path) -> list[str]:
    return git(repo, ["log", "--format=%H", "--", str(path)], text=True).splitlines()


def expect_hash(value: Any, field: str) -> str:
    if not isinstance(value, str) or not HASH_RE.fullmatch(value):
        raise PreregError(1, f"{field} must be sha256:<64 lowercase hex>")
    return value


def required_commit(value: Any, field: str) -> str:
    if not isinstance(value, str) or not COMMIT_RE.fullmatch(value):
        raise PreregError(1, f"{field} must be a lowercase 40-character git commit id")
    return value


def normalize_optional_commit(value: Any) -> str | None:
    if value in ("", None):
        return None
    return required_commit(value, "first_result_commit")


def ensure_commit_exists(repo: Path, commit: str, field: str) -> None:
    completed = subprocess.run(
        ["git", "cat-file", "-e", f"{commit}^{{commit}}"],
        cwd=repo,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if completed.returncode != 0:
        raise PreregError(1, f"{field} does not name an existing commit: {commit}")


def git_is_ancestor(repo: Path, ancestor: str, descendant: str) -> bool:
    completed = subprocess.run(
        ["git", "merge-base", "--is-ancestor", ancestor, descendant],
        cwd=repo,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return completed.returncode == 0


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
        raise PreregError(1, f"git {' '.join(args)} failed: {stderr.strip()}")
    return completed.stdout


def normalize_repo_path(repo: Path, value: Any, field: str) -> Path:
    if not isinstance(value, str) or not value:
        raise PreregError(1, f"{field} must be a non-empty repo-relative path")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts:
        raise PreregError(1, f"{field} must be repo-relative and stay within the repo")
    return path


def line_range(pin: dict[str, Any]) -> str:
    return f"{pin['predictions_line_start']}..{pin['predictions_line_end']}"


def stage_start(stage: int, description: str) -> None:
    emit({"event": "s4_prereg_stage_started", "stage": stage, "description": description})


def stage_done(name: str, stage: int, passed: bool, detail: dict[str, Any]) -> None:
    emit({"event": "s4_prereg_stage_done", "stage": stage, "name": name, "passed": passed, "detail": detail})


def emit(payload: dict[str, Any]) -> None:
    events.append(payload)
    print(json.dumps(payload, sort_keys=True, separators=(",", ":")), file=sys.stderr)


def finish(
    passed: bool,
    exit_code: int,
    output_path: Path,
    emit_json: bool,
    error: PreregError | None = None,
    dry_run: bool = False,
) -> int:
    summary = {
        "script": "s4_preregistration_check",
        "passed": passed,
        "exit_code": exit_code,
        "events": events,
        "dry_run": dry_run,
    }
    if error is not None:
        summary["error"] = {"stage": error.stage, "detail": error.detail, **error.extra}
        emit({"event": "s4_prereg_check_failed", "stage": error.stage, "detail": error.detail})
        print(error.detail, file=sys.stderr)
    else:
        emit({"event": "s4_prereg_check_passed"})
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(summary, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
    if emit_json:
        print(json.dumps(summary, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
PY

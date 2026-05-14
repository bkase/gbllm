#!/usr/bin/env bash
set -euo pipefail

REPORT_DEFAULT="docs/experiments/S2-report.md"
has_report_arg=0
for arg in "$@"; do
  if [[ "$arg" == "--report" || "$arg" == --report=* ]]; then
    has_report_arg=1
    break
  fi
done

if [[ "$has_report_arg" -eq 0 && ! -f "$REPORT_DEFAULT" ]]; then
  script_path="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/s2-prereg-fixture.XXXXXX")"
  cleanup() {
    rm -rf "$tmpdir"
  }
  trap cleanup EXIT

  python3 - "$tmpdir" <<'PY'
import hashlib
import json
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
predictions = "H2 ternary-full gap remains <= 0.5 bpc."


def run(args):
    subprocess.run(args, cwd=root, check=True, stdout=subprocess.DEVNULL)


def predictions_hash(section: str) -> str:
    canonical = json.dumps(
        section.strip(),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(canonical).hexdigest()


def write(path: str, contents: str) -> None:
    out = root / path
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(contents, encoding="utf-8")


def write_report(predictions_commit=None, first_result_commit=None) -> None:
    front_matter = json.dumps(
        {
            "schema": "s2_report.v1",
            "predictions_section_hash": pred_hash,
            "predictions_commit": predictions_commit,
            "first_result_commit": first_result_commit,
            "report_self_hash": None,
        },
        sort_keys=True,
        separators=(",", ":"),
    )
    write(
        "docs/experiments/S2-report.md",
        (
            "---\n"
            f"{front_matter}\n"
            "---\n"
            "# S2 Report\n\n"
            "## Pre-registered predictions\n\n"
            f"{predictions}\n\n"
            "## Observed\n\n"
            "Pending fixture evidence for the preregistration script self-test.\n"
        ),
    )


def commit(message: str) -> str:
    run(["git", "add", "."])
    run(["git", "commit", "-m", message])
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()


run(["git", "init"])
run(["git", "config", "user.email", "s2@example.invalid"])
run(["git", "config", "user.name", "S2 Prereg Fixture"])
pred_hash = predictions_hash(predictions)
write_report()
predictions_commit = commit("pre-register S2 predictions")
write(
    "experiments/S2/schema-template.json",
    '{"score_self_hash":null,"completion":{"kind":"NotReached"}}\n',
)
commit("add S2 schema template")
write(
    "experiments/S2/result.json",
    '{"score_self_hash":"sha256:1212121212121212121212121212121212121212121212121212121212121212"}\n',
)
first_result_commit = commit("add first S2 result")
write_report(predictions_commit, first_result_commit)
commit("finalize S2 report")
PY

  (
    cd "$tmpdir"
    "$script_path" --report "$REPORT_DEFAULT" --artifact-dir experiments/S2 "$@"
  )
  exit $?
fi

# This checker intentionally has no --dry-run mode: O1 preregistration is a
# git-history scan over the current report and S2 artifact paths. Every
# invocation reads live repository history and therefore emits dry_run=false,
# evidence_mode=live, and live_evidence=true in its structured output.

python3 - "$@" <<'PY'
import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

REPORT_DEFAULT = "docs/experiments/S2-report.md"
ARTIFACT_DEFAULT = "experiments/S2"
OUTPUT_DEFAULT = "/tmp/s2-prereg.json"
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
HASH_RE = r"sha256:[0-9a-f]{64}"
RESULT_HASH_RE = re.compile(
    (
        rb'"(?:report_self_hash|ablation_self_hash|oracle_re_run_self_hash|'
        rb'loss_grad_flow_self_hash|linearstate_smoke_self_hash|'
        rb'phase_transition_integ_self_hash|falsification_s2_suite_hash|'
        rb'phase_log_self_hash|score_self_hash|distill_log_self_hash|'
        rb'final_checkpoint)"\s*:\s*"sha256:[0-9a-f]{64}"'
    )
)
COMPLETED_RE = re.compile(rb'"completion"\s*:\s*\{\s*"kind"\s*:\s*"Completed"\s*\}')
VERDICT_RE = re.compile(
    rb'("s2_outcome"\s*:\s*"(?:Pass|Fail)-|'
    rb'"decision"\s*:\s*\{\s*"kind"\s*:\s*"(?:ProceedToS3|Investigate|Halt)|'
    rb'"status"\s*:\s*"(?:Confirmed|Refuted)")'
)
PREDICTIONS_MARKER = "## Pre-registered predictions\n\n"
OBSERVED_MARKER = "\n## Observed\n"


class PreregError(Exception):
    def __init__(self, stage: int, detail: str):
        super().__init__(detail)
        self.stage = stage
        self.detail = detail


events = []


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Verify F-S2 O1 preregistration ordering and predictions hash. "
            "The checker reads predictions_commit and first_result_commit from "
            "the current s2_report.v1 JSON front matter, then scans only the "
            "--report path plus --artifact-dir paths for Step 3 result evidence. "
            "There is intentionally no --dry-run mode because this is a "
            "git-history/preregistration scan; structured output always marks "
            "dry_run=false, evidence_mode=live, and live_evidence=true."
        )
    )
    parser.add_argument(
        "--report",
        default=REPORT_DEFAULT,
        help=(
            "report path relative to the git root; also the source of "
            f"predictions_commit and first_result_commit (default: {REPORT_DEFAULT})"
        ),
    )
    parser.add_argument(
        "--artifact-dir",
        action="append",
        dest="artifact_dirs",
        default=None,
        help=(
            "repo-relative S2 result artifact directory to scan; repeat or "
            "comma-separate for multiple directories (default: experiments/S2)"
        ),
    )
    parser.add_argument(
        "--output",
        "--report-path",
        dest="output",
        default=OUTPUT_DEFAULT,
        help=f"structured JSON output path (default: {OUTPUT_DEFAULT})",
    )
    parser.add_argument("--json", action="store_true", help="emit final JSON on stdout")
    args = parser.parse_args(sys.argv[1:])

    output_path = Path(args.output)
    try:
        repo = Path(git(["rev-parse", "--show-toplevel"], text=True).strip())
        report_path = normalize_repo_path(args.report, "--report")
        artifact_dirs = normalize_artifact_dirs(args.artifact_dirs)
        current_report = Path(repo, report_path).read_text(encoding="utf-8")
        current_front, current_body = parse_report(current_report)

        stage_start(1, "extract section + hash")
        front_hash = expect_hash(current_front.get("predictions_section_hash"), "predictions_section_hash")
        current_hash = predictions_hash(predictions_section(current_body))
        hash_compare(front_hash, current_hash)
        if current_hash != front_hash:
            raise PreregError(
                1,
                "predictions_section_hash mismatch in current report",
            )

        predictions_commit = required_commit(current_front.get("predictions_commit"), "predictions_commit")
        first_result_commit = required_commit(current_front.get("first_result_commit"), "first_result_commit")
        ensure_commit_exists(predictions_commit, "predictions_commit")
        ensure_commit_exists(first_result_commit, "first_result_commit")
        pred_front, pred_body = parse_report(git_show_text(predictions_commit, report_path))
        pred_hash = predictions_hash(predictions_section(pred_body))
        hash_compare(front_hash, pred_hash)
        if pred_hash != front_hash:
            raise PreregError(
                1,
                "predictions_section_hash does not match predictions_commit section",
            )

        stage_start(2, "strict ancestor check")
        strict_ancestor = predictions_commit != first_result_commit and subprocess.run(
            ["git", "merge-base", "--is-ancestor", predictions_commit, first_result_commit],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        ).returncode == 0
        emit(
            {
                "event": "prereg_ancestor_check",
                "predictions_commit": predictions_commit,
                "first_result_commit": first_result_commit,
                "is_strict_ancestor": strict_ancestor,
            }
        )
        if not strict_ancestor:
            raise PreregError(
                2,
                "predictions_commit must be a strict ancestor of first_result_commit",
            )

        stage_start(3, "non-null result scan")
        earliest = earliest_result_commit(predictions_commit, report_path, artifact_dirs)
        if earliest != first_result_commit:
            raise PreregError(
                3,
                (
                    "first_result_commit is not the earliest non-null S2 result commit "
                    f"(expected {earliest}, observed {first_result_commit})"
                ),
            )

        return finish(
            True,
            0,
            output_path,
            {
                "predictions_section_hash": front_hash,
                "predictions_commit": predictions_commit,
                "first_result_commit": first_result_commit,
                "earliest_result_commit": earliest,
            },
            args.json,
        )
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError, PreregError) as error:
        stage = error.stage if isinstance(error, PreregError) else 0
        detail = error.detail if isinstance(error, PreregError) else str(error)
        emit({"event": "prereg_violation", "stage": stage, "detail": detail})
        return finish(False, 1, output_path, {"diagnostic": detail}, args.json)


def emit(payload: dict) -> None:
    events.append(payload)
    print(json.dumps(payload, sort_keys=True, separators=(",", ":")), file=sys.stderr)


def stage_start(stage: int, description: str) -> None:
    emit({"event": "prereg_stage_start", "stage": stage, "description": description})


def hash_compare(expected: str, observed: str) -> None:
    emit(
        {
            "event": "prereg_section_hash_compare",
            "expected": expected,
            "observed": observed,
            "matches": expected == observed,
        }
    )


def finish(passed: bool, code: int, output_path: Path, payload: dict, json_stdout: bool) -> int:
    result = {
        "script": "s2_preregistration_check",
        "passed": passed,
        "exit_code": code,
        "dry_run": False,
        "evidence_mode": "live",
        "live_evidence": True,
        "evidence_source": "git-history-report-and-artifact-scan",
        "events": events,
        **payload,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    emit({"event": "prereg_done", "passed": passed, "exit_code": code})
    if json_stdout:
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return code


def parse_report(text: str):
    if not text.startswith("---\n"):
        raise PreregError(1, "report must start with front-matter marker")
    try:
        front_raw, body = text[4:].split("\n---\n", 1)
    except ValueError as error:
        raise PreregError(1, "report is missing closing front-matter marker") from error
    front = json.loads(front_raw)
    if not isinstance(front, dict):
        raise PreregError(1, "report front matter must be a JSON object")
    return front, body


def predictions_section(body: str) -> str:
    start = body.find(PREDICTIONS_MARKER)
    if start < 0:
        raise PreregError(1, "missing ## Pre-registered predictions section")
    start += len(PREDICTIONS_MARKER)
    end = body.find(OBSERVED_MARKER, start)
    if end < 0:
        raise PreregError(1, "missing ## Observed section after predictions")
    return body[start:end].strip()


def predictions_hash(section: str) -> str:
    canonical = json.dumps(
        section.strip(),
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(canonical).hexdigest()


def earliest_result_commit(predictions_commit: str, report_path: str, artifact_dirs: list[str]):
    paths = result_scan_paths(report_path, artifact_dirs)
    commits = git(["rev-list", "--reverse", f"{predictions_commit}..HEAD", "--", *paths], text=True).splitlines()
    earliest = None
    for commit in commits:
        introduces = commit_introduces_result(commit, report_path, artifact_dirs)
        emit(
            {
                "event": "prereg_commit_scanned",
                "commit": commit,
                "introduces_non_null_result": introduces,
            }
        )
        if introduces and earliest is None:
            earliest = commit
    return earliest


def commit_introduces_result(commit: str, report_path: str, artifact_dirs: list[str]) -> bool:
    if not commit_has_result(commit, report_path, artifact_dirs):
        return False
    parent = subprocess.run(
        ["git", "rev-parse", "--verify", f"{commit}^"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        check=False,
    )
    if parent.returncode != 0:
        return True
    return not commit_has_result(parent.stdout.strip(), report_path, artifact_dirs)


def commit_has_result(commit: str, report_path: str, artifact_dirs: list[str]) -> bool:
    for path in tracked_files_at(commit, report_path, artifact_dirs):
        payload = git_show_bytes(commit, path)
        if RESULT_HASH_RE.search(payload) or COMPLETED_RE.search(payload) or VERDICT_RE.search(payload):
            return True
    return False


def tracked_files_at(commit: str, report_path: str, artifact_dirs: list[str]) -> list[str]:
    paths = git(
        ["ls-tree", "-r", "--name-only", commit, "--", *result_scan_paths(report_path, artifact_dirs)],
        text=True,
    ).splitlines()
    return [
        path
        for path in paths
        if path == report_path
        or any(path == artifact_dir or path.startswith(f"{artifact_dir}/") for artifact_dir in artifact_dirs)
    ]


def result_scan_paths(report_path: str, artifact_dirs: list[str]) -> list[str]:
    return list(dict.fromkeys([*artifact_dirs, report_path]))


def normalize_artifact_dirs(values) -> list[str]:
    if values is None:
        values = [ARTIFACT_DEFAULT]
    result = []
    for value in values:
        for raw in value.split(","):
            path = normalize_repo_path(raw, "--artifact-dir")
            if path == ".":
                raise PreregError(0, "--artifact-dir must not be the repository root")
            if path not in result:
                result.append(path)
    return result


def normalize_repo_path(value: str, field: str) -> str:
    raw = value.strip().replace("\\", "/")
    while raw.startswith("./"):
        raw = raw[2:]
    raw = raw.rstrip("/")
    if not raw:
        raise PreregError(0, f"{field} cannot be empty")
    path = Path(raw)
    if path.is_absolute() or ".." in path.parts:
        raise PreregError(0, f"{field} must be a repo-relative path without '..'")
    return raw


def expect_hash(value, field: str) -> str:
    if not isinstance(value, str) or not re.match(f"^{HASH_RE}$", value):
        raise PreregError(1, f"{field} must be a sha256 hash string")
    return value


def required_commit(value, field: str) -> str:
    if not isinstance(value, str) or not COMMIT_RE.match(value):
        raise PreregError(1, f"{field} must be a 40-character lowercase git commit id")
    return value


def ensure_commit_exists(commit: str, field: str) -> None:
    try:
        subprocess.run(
            ["git", "cat-file", "-e", f"{commit}^{{commit}}"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=True,
        )
    except subprocess.CalledProcessError as error:
        raise PreregError(1, f"{field} does not exist: {commit}") from error


def git(args, *, text: bool):
    return subprocess.check_output(["git", *args], text=text)


def git_show_text(commit: str, path: str) -> str:
    try:
        return git(["show", f"{commit}:{path}"], text=True)
    except subprocess.CalledProcessError as error:
        raise PreregError(1, f"could not read {path} at {commit}") from error


def git_show_bytes(commit: str, path: str) -> bytes:
    return subprocess.check_output(["git", "show", f"{commit}:{path}"])


if __name__ == "__main__":
    raise SystemExit(main())
PY

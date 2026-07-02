#!/usr/bin/env python3
"""Validate the closure-critical shape of docs/experiments/S7-report.md."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import sys
from pathlib import Path


HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
REQUIRED_BODY_HEADINGS = [
    "## Pre-registered predictions",
    "## Observed (per-seed, per-topology table)",
    "## Hypothesis verdicts",
    "## Falsification analysis",
    "## Switch statistics summary",
    "## lambda_switch sweep summary",
    "## Pareto verdict",
    "## Surprises",
    "## Decision",
    "## Reproducibility statement",
]
REQUIRED_HASH_FIELDS = [
    "matched_bytes_self_hash",
    "switch_stats_self_hash",
    "router_collapse_sweep_self_hash",
    "dense_vs_moe_self_hash",
    "frontier_self_hash",
    "burn_grad_smoke_self_hash",
    "oracle_routed_self_hash",
    "emulator_one_token_moe_self_hash",
    "predictions_section_hash",
    "report_self_hash",
]
SWITCH_STATS_MANIFEST_DOMAIN = (
    "gbf-experiments",
    "S7SwitchStatsBundleManifest",
    "s7_switch_stats_bundle_manifest.v1",
    "1",
)
REPORT_MARKDOWN_DOMAIN = (
    "gbf-experiments",
    "S7ReportMarkdown",
    "s7_report.v1",
    "1",
)


class DuplicateKeyError(ValueError):
    pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate F-S7 s7_report.v1 closure front matter and body anchors."
    )
    parser.add_argument(
        "--report",
        default="docs/experiments/S7-report.md",
        help="S7 report path to validate",
    )
    parser.add_argument(
        "--root",
        help="packet root used to cross-check report front-matter hashes against artifact files",
    )
    args = parser.parse_args()

    errors = validate_report(Path(args.report), Path(args.root) if args.root else None)
    if errors:
        print("S7 report closure shape: NEEDS_CHANGES")
        for error in errors:
            print(f" - {error}")
        return 1
    print("S7 report closure shape: ok")
    return 0


def validate_report(path: Path, root: Path | None = None) -> list[str]:
    errors: list[str] = []
    if not path.is_file():
        return [f"report missing: {path}"]

    text = path.read_text(encoding="utf-8")
    try:
        front_matter, body = split_front_matter(text)
    except ValueError as error:
        return [str(error)]

    try:
        scalars, rows = parse_front_matter_subset(front_matter)
    except ValueError as error:
        errors.append(str(error))
        scalars, rows = {}, []

    require_scalar(errors, scalars, "schema", "s7_report.v1")
    decision = scalars.get("decision")
    outcome = scalars.get("s7_outcome")
    if normalize_decision(decision) not in {"ProceedToS8", "ProceedToS8DenseOnly"}:
        errors.append(
            "decision must be ProceedToS8 or ProceedToS8DenseOnly for bd-2v9r closure"
        )
    if (
        normalize_decision(decision) == "ProceedToS8DenseOnly"
        and normalize_outcome(outcome) != "FailParity"
    ):
        errors.append("ProceedToS8DenseOnly is permitted only when s7_outcome is FailParity")

    for field in REQUIRED_HASH_FIELDS:
        require_hash(errors, scalars, field)
    validate_report_self_hash(errors, text, scalars.get("report_self_hash"))
    if normalize_decision(decision) == "ProceedToS8DenseOnly":
        require_hash(errors, scalars, "emulator_one_token_dense_self_hash")
    elif has_non_null(scalars.get("emulator_one_token_dense_self_hash")) and not is_hash(
        scalars.get("emulator_one_token_dense_self_hash", "")
    ):
        errors.append("emulator_one_token_dense_self_hash must be sha256 or null")

    require_commit(errors, scalars, "predictions_commit")
    require_commit(errors, scalars, "first_result_commit")
    rfc_revision = scalars.get("rfc_revision")
    if rfc_revision is None:
        errors.append("missing rfc_revision")
    elif not (COMMIT_RE.match(rfc_revision) or HASH_RE.match(rfc_revision)):
        errors.append("rfc_revision must be a git commit id or sha256 hash")

    validate_rows(errors, rows)
    validate_body(errors, body, normalize_decision(decision))
    if root is not None:
        validate_artifact_references(errors, scalars, rows, root)
    return errors


def split_front_matter(text: str) -> tuple[str, str]:
    if not text.startswith("---\n"):
        raise ValueError("report must start with YAML front matter delimiter")
    marker = "\n---\n"
    end = text.find(marker, 4)
    if end == -1:
        raise ValueError("report front matter closing delimiter not found")
    return text[4:end], text[end + len(marker) :]


def parse_front_matter_subset(
    front_matter: str,
) -> tuple[dict[str, str | None], list[dict[str, str | None]]]:
    """Parse the pinned s7_report.v1 YAML subset emitted by the report producer.

    PyYAML is not part of the repository's default Python environment. Instead
    of silently accepting arbitrary YAML, this parser supports only the simple
    top-level scalar plus per_seed_artifacts list shape that the S7 emitter is
    allowed to produce and rejects richer YAML constructs fail-closed.
    """

    scalars: dict[str, str | None] = {}
    rows: list[dict[str, str | None]] = []
    current: dict[str, str | None] | None = None
    section: str | None = None

    for line_no, raw_line in enumerate(front_matter.splitlines(), start=1):
        if not raw_line.strip() or raw_line.lstrip().startswith("#"):
            continue
        if "\t" in raw_line:
            raise ValueError(f"unsupported tab indentation in report front matter line {line_no}")
        if re.search(r"(^|[:\s])[*&][A-Za-z0-9_-]+", raw_line):
            raise ValueError(f"unsupported YAML anchor/alias in report front matter line {line_no}")

        if not raw_line.startswith(" "):
            section = None
            key, value = parse_key_value(raw_line, line_no)
            if key in scalars:
                raise ValueError(f"duplicate front matter field {key!r}")
            if key == "per_seed_artifacts":
                if value is not None:
                    raise ValueError("per_seed_artifacts must use block list form")
                section = key
            else:
                scalars[key] = value
            continue

        if section != "per_seed_artifacts":
            raise ValueError(f"unexpected nested front matter line {line_no}")
        dash = re.match(r"^\s*-\s*(.*?)\s*$", raw_line)
        if dash:
            if current is not None:
                rows.append(current)
            current = {}
            inline = dash.group(1)
            if inline:
                key, value = parse_key_value(inline, line_no)
                current[key] = value
            continue
        if current is None:
            raise ValueError(f"per_seed_artifacts field before first row in line {line_no}")
        key, value = parse_key_value(raw_line.strip(), line_no)
        if key in current:
            raise ValueError(f"duplicate per_seed_artifacts field {key!r} in line {line_no}")
        current[key] = value

    if current is not None:
        rows.append(current)
    return scalars, rows


def parse_key_value(text: str, line_no: int) -> tuple[str, str | None]:
    match = re.match(r"^([A-Za-z0-9_]+):\s*(.*?)\s*$", text)
    if not match:
        raise ValueError(f"invalid front matter key/value syntax in line {line_no}")
    key = match.group(1)
    raw_value = match.group(2)
    if raw_value in {"|", ">"}:
        raise ValueError(f"unsupported YAML block scalar for {key!r} in line {line_no}")
    if raw_value.startswith(("[", "{")):
        raise ValueError(f"unsupported YAML flow collection for {key!r} in line {line_no}")
    return key, clean_scalar(raw_value)


def clean_scalar(value: str) -> str | None:
    value = value.strip()
    if value in {"", "null", "Null", "NULL", "~"}:
        return None
    if (value.startswith('"') and value.endswith('"')) or (
        value.startswith("'") and value.endswith("'")
    ):
        return value[1:-1]
    return value


def require_scalar(
    errors: list[str], scalars: dict[str, str], field: str, expected: str
) -> None:
    observed = scalars.get(field)
    if observed != expected:
        errors.append(f"{field} must be {expected!r}, observed {observed!r}")


def require_hash(errors: list[str], scalars: dict[str, str], field: str) -> None:
    value = scalars.get(field)
    if value is None:
        errors.append(f"missing {field}")
    elif not is_hash(value):
        errors.append(f"{field} must be a non-null sha256 hash")


def require_commit(errors: list[str], scalars: dict[str, str], field: str) -> None:
    value = scalars.get(field)
    if value is None:
        errors.append(f"missing {field}")
    elif not COMMIT_RE.match(value):
        errors.append(f"{field} must be a 40-hex git commit id")


def validate_rows(errors: list[str], rows: list[dict[str, str]]) -> None:
    if len(rows) != 10:
        errors.append(f"per_seed_artifacts must contain 10 rows, observed {len(rows)}")

    observed: set[tuple[str, str]] = set()
    for row in rows:
        seed = row.get("seed")
        topology = row.get("topology")
        if seed is None or topology is None:
            errors.append(f"per_seed_artifacts row missing seed/topology: {row}")
            continue
        observed.add((topology, seed))
        if row.get("completion") != "Completed":
            errors.append(f"{topology} seed {seed} completion must be Completed")
        for field in ["checkpoint_self_hash", "run_log_self_hash", "score_self_hash"]:
            if not is_hash(row.get(field, "")):
                errors.append(f"{topology} seed {seed} {field} must be a non-null sha256 hash")

    expected = {
        (topology, str(seed))
        for topology in ["MoeTiny", "MoeTinyDenseMatched"]
        for seed in range(5)
    }
    missing = sorted(expected - observed)
    if missing:
        errors.append(f"per_seed_artifacts missing rows: {missing}")


def validate_artifact_references(
    errors: list[str],
    scalars: dict[str, str | None],
    rows: list[dict[str, str | None]],
    root: Path,
) -> None:
    for row in rows:
        seed = row.get("seed")
        topology = row.get("topology")
        if seed is None or topology not in {"MoeTiny", "MoeTinyDenseMatched"}:
            continue
        run_hash = artifact_hash(
            errors,
            root,
            f"experiments/S7/runs/{topology}/seed-{seed}/run-log.json",
            ["run_log_self_hash"],
        )
        compare_report_hash(errors, row.get("run_log_self_hash"), run_hash, "run_log_self_hash")
        score_rel_path = f"experiments/S7/scores/{topology}/seed-{seed}/score.json"
        checkpoint_hash = artifact_hash(
            errors,
            root,
            score_rel_path,
            ["checkpoint_sha"],
        )
        compare_report_hash(
            errors,
            row.get("checkpoint_self_hash"),
            checkpoint_hash,
            "checkpoint_self_hash",
        )
        score_hash = artifact_hash(
            errors,
            root,
            score_rel_path,
            ["score_self_hash"],
        )
        compare_report_hash(errors, row.get("score_self_hash"), score_hash, "score_self_hash")

    top_level_refs = [
        (
            "matched_bytes_self_hash",
            "experiments/S7/dense-vs-moe/comparison.json",
            ["matched_bytes_pin", "matched_bytes_self_hash"],
        ),
        (
            "router_collapse_sweep_self_hash",
            "experiments/S7/router-collapse/seed-0/sweep.json",
            ["sweep_self_hash"],
        ),
        (
            "dense_vs_moe_self_hash",
            "experiments/S7/dense-vs-moe/comparison.json",
            ["comparison_self_hash"],
        ),
        (
            "frontier_self_hash",
            "experiments/S7/frontier/frontier.json",
            ["frontier_self_hash"],
        ),
        (
            "burn_grad_smoke_self_hash",
            "experiments/S7/burn-grad-smoke/expert_block_qat.json",
            ["smoke_self_hash"],
        ),
        (
            "oracle_routed_self_hash",
            "experiments/S7/oracle-routed/seed-0/oracle.json",
            ["oracle_self_hash"],
        ),
        (
            "emulator_one_token_moe_self_hash",
            "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json",
            ["emulator_self_hash"],
        ),
    ]
    for report_field, rel_path, json_path in top_level_refs:
        expected = artifact_hash(errors, root, rel_path, json_path)
        compare_report_hash(errors, scalars.get(report_field), expected, report_field)
    switch_expected = switch_stats_manifest_hash(errors, root)
    compare_report_hash(
        errors,
        scalars.get("switch_stats_self_hash"),
        switch_expected,
        "switch_stats_self_hash",
    )

    dense_hash = scalars.get("emulator_one_token_dense_self_hash")
    if has_non_null(dense_hash):
        expected = artifact_hash(
            errors,
            root,
            "experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json",
            ["emulator_self_hash"],
        )
        compare_report_hash(errors, dense_hash, expected, "emulator_one_token_dense_self_hash")


def switch_stats_manifest_hash(errors: list[str], root: Path) -> str | None:
    entries: list[dict[str, object]] = []
    for seed in range(5):
        bundle_hash = artifact_hash(
            errors,
            root,
            f"experiments/S7/switch-stats/seed-{seed}/switch-stats.json",
            ["bundle_self_hash"],
        )
        if bundle_hash is None:
            return None
        entries.append({"seed": seed, "bundle_self_hash": bundle_hash})
    return domain_json_hash(
        SWITCH_STATS_MANIFEST_DOMAIN,
        {
            "schema": "s7_switch_stats_bundle_manifest.v1",
            "seed_bundle_self_hashes": entries,
        },
    )


def artifact_hash(
    errors: list[str], root: Path, rel_path: str, json_path: list[str]
) -> str | None:
    path = root / rel_path
    if not path.is_file():
        errors.append(f"report artifact reference missing: {rel_path}")
        return None
    try:
        text = path.read_text(encoding="utf-8")
        payload = json.loads(text, object_pairs_hook=reject_duplicate_keys)
    except DuplicateKeyError as error:
        errors.append(f"report artifact reference has duplicate JSON key: {rel_path}: {error}")
        return None
    except json.JSONDecodeError as error:
        errors.append(f"report artifact reference is not JSON: {rel_path}: {error}")
        return None
    try:
        canonical = canonical_json_text(payload)
    except (TypeError, ValueError) as error:
        errors.append(f"report artifact reference has non-canonical JSON value: {rel_path}: {error}")
        return None
    if text not in {canonical, f"{canonical}\n"}:
        errors.append(
            f"report artifact reference must use canonical JSON bytes: {rel_path}"
        )
        return None
    value: object = payload
    for key in json_path:
        if not isinstance(value, dict):
            dotted = ".".join(json_path)
            errors.append(f"report artifact reference {rel_path} missing {dotted}")
            return None
        value = value.get(key)
    if not is_hash(value):
        dotted = ".".join(json_path)
        errors.append(f"report artifact reference {rel_path} {dotted} must be a sha256 hash")
        return None
    return value


def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    out: dict[str, object] = {}
    for key, value in pairs:
        if key in out:
            raise DuplicateKeyError(key)
        out[key] = value
    return out


def canonical_json_text(payload: object) -> str:
    if payload is None:
        return "null"
    if payload is True:
        return "true"
    if payload is False:
        return "false"
    if isinstance(payload, int) and not isinstance(payload, bool):
        return str(payload)
    if isinstance(payload, float):
        return canonical_float_text(payload)
    if isinstance(payload, str):
        return json.dumps(payload, ensure_ascii=False, allow_nan=False)
    if isinstance(payload, list):
        return "[" + ",".join(canonical_json_text(item) for item in payload) + "]"
    if isinstance(payload, dict):
        return (
            "{"
            + ",".join(
                f"{json.dumps(key, ensure_ascii=False, allow_nan=False)}:{canonical_json_text(payload[key])}"
                for key in sorted(payload)
            )
            + "}"
        )
    raise TypeError(f"unsupported JSON value for canonical encoding: {type(payload).__name__}")


def canonical_float_text(value: float) -> str:
    if not math.isfinite(value):
        raise ValueError("non-finite float in canonical JSON payload")
    if value == 0.0:
        return "0.0"
    encoded = repr(value).replace("E", "e")
    if "e" not in encoded:
        return encoded
    mantissa, exponent = encoded.split("e", 1)
    exponent_value = int(exponent)
    if exponent_value < 0 and abs(value) >= 1.0e-5:
        return expand_negative_exponent_decimal(mantissa, exponent_value)
    return f"{mantissa}e{exponent_value}"


def expand_negative_exponent_decimal(mantissa: str, exponent: int) -> str:
    sign = ""
    if mantissa.startswith("-"):
        sign = "-"
        mantissa = mantissa[1:]
    integer_digits = mantissa.find(".")
    if integer_digits == -1:
        integer_digits = len(mantissa)
    digits = mantissa.replace(".", "")
    decimal_at = integer_digits + exponent
    if decimal_at <= 0:
        return f"{sign}0.{'0' * (-decimal_at)}{digits}"
    if decimal_at >= len(digits):
        return f"{sign}{digits}{'0' * (decimal_at - len(digits))}.0"
    return f"{sign}{digits[:decimal_at]}.{digits[decimal_at:]}"


def compare_report_hash(
    errors: list[str], observed: str | None, expected: str | None, field: str
) -> None:
    if expected is None or observed is None or not is_hash(observed):
        return
    if observed != expected:
        errors.append(f"{field} must match artifact self-hash")


def validate_report_self_hash(
    errors: list[str], text: str, observed: str | None
) -> None:
    if observed is None or not is_hash(observed):
        return
    normalized, count = re.subn(
        r'(?m)^report_self_hash:\s*(?:"sha256:[0-9a-f]{64}"|sha256:[0-9a-f]{64}|null)\s*$',
        "report_self_hash: null",
        text,
        count=1,
    )
    if count != 1:
        errors.append("report_self_hash line must be a top-level scalar")
        return
    normalized = re.sub(
        r'(?m)^generated_at:\s*(?:"[^"\n]*"|[^#\n]*)\s*$',
        "generated_at: null",
        normalized,
        count=1,
    )
    expected = domain_bytes_hash(REPORT_MARKDOWN_DOMAIN, normalized.encode("utf-8"))
    if observed != expected:
        errors.append(
            "report_self_hash must match report bytes with generated_at and report_self_hash nulled"
        )


def domain_json_hash(domain: tuple[str, str, str, str], payload: object) -> str:
    canonical = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")
    return domain_bytes_hash(domain, canonical)


def domain_bytes_hash(domain: tuple[str, str, str, str], payload: bytes) -> str:
    crate_name, type_name, schema_id, schema_version = domain
    material = (
        f"gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}".encode("utf-8")
        + b"\0"
        + payload
    )
    return f"sha256:{hashlib.sha256(material).hexdigest()}"


def validate_body(errors: list[str], body: str, front_matter_decision: str | None) -> None:
    for heading in REQUIRED_BODY_HEADINGS:
        if heading not in body:
            errors.append(f"missing body heading: {heading}")
    for index in range(1, 11):
        if not re.search(rf"\bH{index}\b", body):
            errors.append(f"missing explicit H{index} hypothesis verdict")
    if "NotEvaluatedDueToPriorGate" in body:
        errors.append("closure-candidate reports must not use NotEvaluatedDueToPriorGate")
    validate_body_decision(errors, body, front_matter_decision)


def validate_body_decision(
    errors: list[str], body: str, front_matter_decision: str | None
) -> None:
    section = body_section(body, "## Decision")
    observed: set[str] = set()
    if re.search(r"\bProceedToS8(?!DenseOnly)\b|\bProceed-To-S8(?!-DenseOnly)\b", section):
        observed.add("ProceedToS8")
    if re.search(r"\bProceedToS8DenseOnly\b|\bProceed-To-S8-DenseOnly\b", section):
        observed.add("ProceedToS8DenseOnly")
    if not observed:
        errors.append("Decision body must state ProceedToS8 or ProceedToS8DenseOnly")
        return
    if len(observed) > 1:
        errors.append("Decision body must state exactly one closure decision")
        return
    body_decision = next(iter(observed))
    if front_matter_decision is not None and body_decision != front_matter_decision:
        errors.append("Decision body must match front matter decision")


def body_section(body: str, heading: str) -> str:
    lines = body.splitlines()
    in_section = False
    section: list[str] = []
    for line in lines:
        if line == heading:
            in_section = True
            continue
        if in_section and line.startswith("## "):
            break
        if in_section:
            section.append(line)
    return "\n".join(section)


def normalize_decision(value: str | None) -> str | None:
    return {
        "ProceedToS8": "ProceedToS8",
        "Proceed-To-S8": "ProceedToS8",
        "ProceedToS8DenseOnly": "ProceedToS8DenseOnly",
        "Proceed-To-S8-DenseOnly": "ProceedToS8DenseOnly",
    }.get(value or "")


def normalize_outcome(value: str | None) -> str | None:
    if value is None:
        return None
    return value.replace("-", "")


def has_non_null(value: object | None) -> bool:
    return isinstance(value, str) and value.lower() not in {"null", "~", ""}


def is_hash(value: object) -> bool:
    return isinstance(value, str) and bool(HASH_RE.match(value))


if __name__ == "__main__":
    sys.exit(main())

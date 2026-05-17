#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

build_id="${F_B6_F_B7_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
out_dir="${F_B6_F_B7_OUT_DIR:-/tmp/f-b6-f-b7-closure/$build_id}"
check_existing=0

case "${1:-}" in
  "")
    ;;
  --check-existing)
    check_existing=1
    if [[ $# -ne 2 ]]; then
      echo "usage: $0 --check-existing <out-dir>" >&2
      exit 2
    fi
    out_dir="$2"
    ;;
  *)
    echo "usage: $0 [--check-existing <out-dir>]" >&2
    exit 2
    ;;
esac

export F_B6_F_B7_BUILD_ID="$build_id"
export F_B6_F_B7_OUT_DIR="$out_dir"

if [[ "$check_existing" -eq 0 ]]; then
  F_B6_F_B7_SKIP_TAR=1 "$SCRIPT_DIR/closure-pack.sh" >/tmp/f-b6-f-b7-closure-pack-path.txt
  "$SCRIPT_DIR/check-checklist.sh"
fi

python3 - "$REPO_ROOT" "$out_dir" <<'PY'
import json
import os
import pathlib
import re
import sys

repo_root = pathlib.Path(sys.argv[1])
out_dir = pathlib.Path(sys.argv[2])
telemetry_rs = repo_root / "gbf-codegen/tests/support/f_b6_f_b7/telemetry.rs"
fixture_root = pathlib.Path(
    os.environ.get(
        "F_B6_F_B7_FIXTURE_ROOT",
        repo_root / "gbf-codegen/tests/fixtures/f_b6_f_b7",
    )
)

def rust_string_const_array(name):
    text = telemetry_rs.read_text(encoding="utf-8")
    pattern = re.compile(rf"pub const {name}: &\[&str\] = &\[(.*?)\];", re.S)
    match = pattern.search(text)
    if not match:
        raise SystemExit(f"unable to find Rust telemetry const {name}")
    return re.findall(r'"([^"]+)"', match.group(1))

stage4_events = rust_string_const_array("STAGE4_EVENT_NAMES")
stage5_events = rust_string_const_array("STAGE5_EVENT_NAMES")
verify_events = rust_string_const_array("RANGE_CERT_VERIFY_EVENT_NAMES")
common_fields = rust_string_const_array("F_B6_F_B7_COMMON_EVENT_FIELDS")

conditional = {
    "stage4": {
        "stage4.driver.cache_hit": "not emitted by the closure packet cache-miss fixture",
        "stage4.driver.failure_memo": "failure memo path belongs to later driver beads",
        "stage4.driver.audit_parent_rewrap": "audit rewrap path belongs to later driver beads",
    },
    "stage5": {
        "stage5.driver.cache_hit": "not emitted by the closure packet cache-miss fixture",
        "stage5.driver.failure_memo": "failure memo path belongs to later driver beads",
        "stage5.driver.audit_parent_rewrap": "audit rewrap path belongs to later driver beads",
        "range_cert.verifies.single_i16": "mutually exclusive with the chunked_i16 fixture used here",
        "range_cert.verifies.renorm_loop": "mutually exclusive with the chunked_i16 fixture used here",
        "range_cert.verifies.failed": "failure path belongs to tampered/verifier beads",
        "range_cert.renorm_recurrence_verifies": "renorm-loop recurrence path is not exercised by this packet",
    },
    "verify": {
        "range_cert.independent_verify.certified_reduction.single_i16": "mutually exclusive with the chunked_i16 fixture used here",
        "range_cert.independent_verify.certified_reduction.renorm_loop": "mutually exclusive with the chunked_i16 fixture used here",
    },
}
required_events = {
    "stage4": [event for event in stage4_events if event not in conditional["stage4"]],
    "stage5": [event for event in stage5_events if event not in conditional["stage5"]],
    "verify": [event for event in verify_events if event not in conditional["verify"]],
}
allowed_events = {
    "stage4": set(stage4_events),
    "stage5": set(stage5_events),
    "verify": set(verify_events),
}
files = {
    "stage4": out_dir / "stage4-run.ndjson",
    "stage5": out_dir / "stage5-run.ndjson",
    "verify": out_dir / "verify-packet.ndjson",
}

target_by_prefix = {
    "stage4.": "gbf_codegen::s4",
    "stage5.": "gbf_codegen::s5",
    "range_cert.": "gbf_verify::range_cert",
}

ts_re = re.compile(r"^unix:[0-9]+\.[0-9]{9}$")

def target_for_event(event):
    for prefix, target in target_by_prefix.items():
        if event.startswith(prefix):
            return target
    raise SystemExit(f"no target contract for event {event!r}")

def validate_line(path, payload):
    for required in ["ts", "event", "level", "target", "fields", "span"]:
        if required not in payload:
            raise SystemExit(f"{path} line missing {required}")
    event = payload["event"]
    if not isinstance(event, str):
        raise SystemExit(f"{path} event is not a string")
    ts = payload["ts"]
    if not isinstance(ts, str) or not ts_re.match(ts):
        raise SystemExit(f"{path} event {event} has malformed timestamp {ts!r}")
    target = payload["target"]
    expected_target = target_for_event(event)
    if target != expected_target:
        raise SystemExit(
            f"{path} event {event} target {target!r} != expected {expected_target!r}"
        )
    fields = payload["fields"]
    if not isinstance(fields, dict):
        raise SystemExit(f"{path} event {event} fields is not an object")
    for field in common_fields:
        if field not in fields:
            raise SystemExit(f"{path} event {event} missing common field {field}")
    string_fields = [
        "site_id",
        "checkpoint_id",
        "stratum",
        "probe_instance_id",
        "importance_class",
        "build_id",
        "k4_hash",
        "k5_hash",
        "outcome",
        "diag_code",
    ]
    integer_fields = [
        "compact_checkpoint_id",
        "runtime_probe_id",
        "elapsed_ns",
        "event_seq",
    ]
    for field in string_fields:
        if not isinstance(fields[field], str):
            raise SystemExit(
                f"{path} event {event} common field {field} must be a string"
            )
    for field in integer_fields:
        if not isinstance(fields[field], int) or isinstance(fields[field], bool):
            raise SystemExit(
                f"{path} event {event} common field {field} must be an integer"
            )
    if fields["outcome"] not in {"passed", "failed"}:
        raise SystemExit(
            f"{path} event {event} common field outcome has invalid value {fields['outcome']!r}"
        )

def validate_stage5_cert_exclusivity(path, payloads):
    by_fixture = {}
    for payload in payloads:
        event = payload["event"]
        if not event.startswith("range_cert.verifies."):
            continue
        fixture = payload["fields"].get("fixture", "unknown")
        by_fixture.setdefault(fixture, []).append(event)
    for fixture, events in by_fixture.items():
        if len(events) != 1:
            raise SystemExit(
                f"{path} fixture {fixture} emitted mutually exclusive cert events {events}"
            )

def validate_packet_file(key, path):
    observed = set()
    payloads = []
    for line in files[key].read_text(encoding="utf-8").splitlines():
        payload = json.loads(line)
        validate_line(path, payload)
        event = payload["event"]
        if event not in allowed_events[key]:
            raise SystemExit(f"{path} unexpected event {event}")
        observed.add(event)
        payloads.append(payload)
    missing = sorted(set(required_events[key]) - observed)
    if missing:
        raise SystemExit(f"{path} missing required events: {missing}")
    if key == "stage5":
        validate_stage5_cert_exclusivity(path, payloads)
    return payloads

def has_cert_event(payloads, cert_path, event, outcome):
    cert_path = str(cert_path)
    return any(
        payload["event"] == event
        and payload["fields"].get("cert_path") == cert_path
        and payload["fields"].get("outcome") == outcome
        for payload in payloads
    )

def validate_verify_cli_cases(payloads):
    passing_cert = out_dir / "reports/stage5/chunked_i16/certs/range.cert.json"
    expected = {
        passing_cert: (
            "range_cert.independent_verify.certified_reduction.chunked_i16",
            "passed",
        ),
        out_dir / "tampered/malformed_json/range.cert.json": (
            "range_cert.independent_verify.failed.malformed",
            "failed",
        ),
        out_dir / "tampered/report_self_hash_mismatch/range.cert.json": (
            "range_cert.independent_verify.failed.report_self_hash_mismatch",
            "failed",
        ),
        out_dir / "tampered/unsupported_plan_family/range.cert.json": (
            "range_cert.independent_verify.failed.unsupported_plan_family",
            "failed",
        ),
        out_dir / "tampered/cert_lowered_slack/range.cert.json": (
            "range_cert.independent_verify.certified_reduction.chunked_i16",
            "failed",
        ),
        out_dir / "tampered/cert_wrong_plan_family/range.cert.json": (
            "range_cert.independent_verify.certified_reduction.single_i16",
            "failed",
        ),
        out_dir / "tampered/cert_inconsistent_term_count/range.cert.json": (
            "range_cert.independent_verify.certified_reduction.chunked_i16",
            "failed",
        ),
        out_dir / "tampered/cert_failed_witness_mismatch/range.cert.json": (
            "range_cert.independent_verify.failed.witness_mismatch",
            "failed",
        ),
    }
    for cert_path, (event, outcome) in expected.items():
        if not cert_path.is_file():
            raise SystemExit(f"missing verifier CLI cert fixture {cert_path}")
        if not has_cert_event(payloads, cert_path, event, outcome):
            raise SystemExit(
                f"verify-packet missing {outcome} CLI event {event} for {cert_path}"
            )

def canonical_json_bytes(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")

def strip_trailing_ascii_whitespace(data):
    return data.rstrip(b" \t\r\n")

required_input_keys = {
    "stage4": {
        "infer_ir_product",
        "infer_ir_self_hash",
        "quant_graph_self_hash",
        "semantic_checkpoint_schema",
        "semantic_checkpoint_schema_hash",
        "artifact_declared_semantic_checkpoint_schema_hash",
        "probe_registry",
        "probe_registry_hash",
        "metric_registry",
        "metric_registry_hash",
        "trace_event_layout_registry",
        "trace_event_layout_registry_hash",
        "op_policy_projection",
        "audit_parents",
    },
    "stage5": {
        "infer_ir_product",
        "infer_ir_self_hash",
        "quant_graph_self_hash",
        "static_budget_report",
        "static_budget_self_hash",
        "range_policy_projection",
        "audit_parents",
    },
}

def validate_fixture_inputs():
    for stage in ["stage4", "stage5"]:
        stage_dir = fixture_root / "reject" / stage
        if not stage_dir.is_dir():
            raise SystemExit(f"missing fixture stage directory {stage_dir}")
        for path in sorted(stage_dir.glob("*/inputs.json")):
            raw = path.read_bytes()
            payload = json.loads(raw)
            if not isinstance(payload, dict):
                raise SystemExit(f"{path} is not a JSON object")
            if payload.get("fixture_status") == "placeholder":
                code = path.parent.name
                expected = {"code": code, "fixture_status": "placeholder", "stage": stage}
                if payload != expected:
                    raise SystemExit(f"{path} has malformed placeholder contract")
                continue
            keys = set(payload)
            if keys != required_input_keys[stage]:
                missing = sorted(required_input_keys[stage] - keys)
                extra = sorted(keys - required_input_keys[stage])
                raise SystemExit(
                    f"{path} promoted {stage} inputs do not match landed structural contract; "
                    f"missing={missing} extra={extra}"
                )
            if strip_trailing_ascii_whitespace(raw) != canonical_json_bytes(payload):
                raise SystemExit(f"{path} promoted {stage} inputs are not canonical JSON")

packet_payloads = {}
for key, path in files.items():
    if not path.is_file():
        raise SystemExit(f"missing packet telemetry file {path}")
    packet_payloads[key] = validate_packet_file(key, path)

validate_verify_cli_cases(packet_payloads["verify"])

validate_fixture_inputs()

contract = {
    "source_of_truth": str(telemetry_rs.relative_to(repo_root)),
    "common_fields": common_fields,
    "required_events": required_events,
    "conditional_events": conditional,
    "fixture_input_contract": {
        "placeholder": "stage/code/fixture_status marker only; non-executable",
        "promoted": "must match the landed Stage 4/Stage 5 input top-level structures and canonical JSON",
    },
}
(out_dir / "verify-contract.json").write_text(
    json.dumps(contract, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

if [[ "$check_existing" -eq 0 ]]; then
  packet="$out_dir/closure-packet.tar.gz"
  if [[ -e "$packet" ]]; then
    rm -f "$packet"
  fi
  tmp_tar="$(dirname "$out_dir")/$(basename "$out_dir").closure-packet.tar.gz"
  tar -C "$(dirname "$out_dir")" -czf "$tmp_tar" "$(basename "$out_dir")"
  mv "$tmp_tar" "$packet"
  echo "verify packet complete: $packet"
else
  echo "verify packet checks complete: $out_dir"
fi

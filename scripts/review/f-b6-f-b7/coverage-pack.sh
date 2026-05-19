#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

build_id="${F_B6_F_B7_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)-coverage}"
out_dir="${F_B6_F_B7_OUT_DIR:-/tmp/f-b6-f-b7-closure/$build_id}"
fixture_root="gbf-codegen/tests/fixtures/f_b6_f_b7"
mkdir -p "$out_dir"

python3 - "$fixture_root" "$out_dir" "$build_id" <<'PY'
import json
import pathlib
import sys
import time

fixture_root = pathlib.Path(sys.argv[1])
out_dir = pathlib.Path(sys.argv[2])
build_id = sys.argv[3]
stage4 = [
    "OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE",
    "OBSERVATION-CHECKPOINT-NOT-ATTACHABLE",
    "OBSERVATION-CHECKPOINT-AMBIGUOUS",
    "OBSERVATION-PROBE-ID-UNKNOWN",
    "OBSERVATION-REQUIRED-PROBE-DISABLED",
    "OBSERVATION-METRIC-SOURCE-RESERVED-V1",
    "OBSERVATION-METRIC-HISTOGRAM-BUCKET-COUNT-ZERO",
    "OBSERVATION-PROBE-SOURCE-INVALID",
    "OBSERVATION-RESERVED-EFFECT-PROBE",
    "OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED",
    "OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED",
    "OBSERVATION-PROBE-CLASS-CAP-EXCEEDED",
    "OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED",
    "OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT",
    "OBSERVATION-DETERMINISM-MISMATCH",
    "OBSERVATION-SC-HASH-MISMATCH",
]
stage4_evidence = {
    "OBSERVATION-MANDATORY-CHECKPOINT-NOT-FEASIBLE": "semantic_selection_mandatory_not_feasible_fails",
    "OBSERVATION-CHECKPOINT-NOT-ATTACHABLE": "semantic_anchor_binding_missing_anchor_fails",
    "OBSERVATION-CHECKPOINT-AMBIGUOUS": "bind_semantic_observations_v1",
    "OBSERVATION-PROBE-ID-UNKNOWN": "disabled_unknown_probe_rejected",
    "OBSERVATION-REQUIRED-PROBE-DISABLED": "disabled_required_probe_rejected",
    "OBSERVATION-METRIC-SOURCE-RESERVED-V1": "metric_registry_filter_per_slice_reserved_rejected",
    "OBSERVATION-METRIC-HISTOGRAM-BUCKET-COUNT-ZERO": "metric_aggregation_histogram_bucket_count_zero_rejected",
    "OBSERVATION-PROBE-SOURCE-INVALID": "probe_instance_id_collision_rejected_at_canonical_sort",
    "OBSERVATION-RESERVED-EFFECT-PROBE": "effect_class_diagnostic_precedence",
    "OBSERVATION-SEQUENCE-STATE-PROBE-RESERVED": "sequence_state_and_fault_boundary_effect_probe_rejections",
    "OBSERVATION-FAULT-BOUNDARY-PROBE-RESERVED": "sequence_state_and_fault_boundary_effect_probe_rejections",
    "OBSERVATION-PROBE-CLASS-CAP-EXCEEDED": "probe_class_cap_exceeded_for_non_required_classes",
    "OBSERVATION-INVARIANT-MODE-BUDGET-BUSTED": "invariant_budget_check_under_invariant_fails_when_over",
    "OBSERVATION-ENCODING-INVALID-FOR-CHECKPOINT": "encoding_for_invalid_override_fails_without_panicking",
    "OBSERVATION-DETERMINISM-MISMATCH": "op_pre_3a_determinism_class_mismatch_rejected",
    "OBSERVATION-SC-HASH-MISMATCH": "op_pre_2_artifact_declared_hash_mismatch_rejected",
}
stage_origins = {
    "stage4": "ObservationPlanConstruction",
    "stage5": "RangePlanConstruction",
}
stage5 = [
    "RANGE-ACCUMULATOR-DOMAIN-UNSUPPORTED-V1",
    "RANGE-TERM-COUNT-ZERO",
    "RANGE-CEILING-VIOLATED-SINGLE-I16-ONLY",
    "RANGE-CEILING-VIOLATED-NO-RENORM-LOOP",
    "RANGE-NO-PROVEN-PLAN-WITHIN-CEILING",
    "RANGE-SITE-MISSING-FROM-STATIC-BUDGET",
    "RANGE-STATIC-BUDGET-SITE-ORPHANED",
    "RANGE-DUPLICATE-REDUCTION-SITE-ID",
    "RANGE-BITEXACT-REQUIRES-CHUNK-DIVIDES",
    "RANGE-BITEXACT-RENORM-LOOP-RESERVED-V1",
    "RANGE-DETERMINISM-MISMATCH",
    "RANGE-CEILING-OVERRIDE-INVALID-SELECTOR",
    "RANGE-CEILING-OVERRIDE-AMBIGUOUS",
    "RANGE-SITE-FACTS-INCONSISTENT",
    "RANGE-CHUNK-LEN-EXCEEDS-PROFILE-MAX",
    "RANGE-TILE-LEN-BELOW-PROFILE-MIN",
    "RANGE-TILE-LEN-EXCEEDS-PROFILE-MAX",
]
reserved = {
    "OBSERVATION-METRIC-ID-UNKNOWN",
    "OBSERVATION-OPTIONAL-CHECKPOINT-NOT-FEASIBLE",
    "OBSERVATION-WORKLOAD-CHECKPOINT-NOT-FEASIBLE",
    "OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA",
    "OBSERVATION-LOCKED-KNOB-DRIFT",
    "OBSERVATION-COMPARE-DOMAIN-MISMATCH",
    "OBSERVATION-WORKLOAD-DETERMINISM-MISMATCH",
    "OBSERVATION-POLICY-WORKLOAD-DETERMINISM-MISMATCH",
    "RANGE-LOCKED-KNOB-DRIFT",
    "RANGE-CERT-MALFORMED",
    "RANGE-CHUNK-LEN-ZERO",
    "RANGE-TILE-LEN-ZERO",
    "RANGE-BITEXACT-MID-REDUCTION-SATURATION-FORBIDDEN",
    "RANGE-RENORM-STRATEGY-UNSUPPORTED-V1",
    "RANGE-CAPS-INVALID",
    "RANGE-INTEGER-OVERFLOW-DURING-PROOF",
    "RANGE-TILE-LEN-EXCEEDS-U16",
}

def check(stage, codes):
    out = out_dir / f"{stage}_diagnostics_coverage.ndjson"
    expected_origin = stage_origins[stage]
    with out.open("w", encoding="utf-8") as f:
        for seq, code in enumerate(codes, 1):
            directory = fixture_root / "reject" / stage / code
            expected = directory / "expected_diag.json"
            inputs = directory / "inputs.json"
            readme = directory / "README.md"
            if not directory.is_dir() or not expected.is_file() or not inputs.is_file() or not readme.is_file():
                raise SystemExit(f"missing reject fixture files for {stage}/{code}")
            payload = json.loads(expected.read_text(encoding="utf-8"))
            if (
                payload.get("code") != code
                or payload.get("stage") != stage
                or payload.get("origin") != expected_origin
                or payload.get("reserved") is not False
            ):
                raise SystemExit(f"expected_diag mismatch for {stage}/{code}")
            if stage == "stage5":
                if (
                    payload.get("wire_code_kind") != "ReportSemanticInvariantViolated"
                    or payload.get("rfc_code_location") != "evidence.reference"
                    or "producer test" not in payload.get("producer_evidence", "")
                ):
                    raise SystemExit(f"stage5 expected_diag does not pin wire shape/evidence for {code}")
            if stage == "stage4":
                evidence = stage4_evidence[code]
                if evidence not in payload.get("producer_evidence", ""):
                    raise SystemExit(f"stage4 expected_diag does not pin producer evidence for {code}")
            f.write(json.dumps({
                "ts": f"unix:{time.time():.9f}",
                "event": f"{stage}.diagnostic.coverage",
                "level": "INFO",
                "target": "scripts.review.f_b6_f_b7",
                "fields": {
                    "build_id": build_id,
                    "diag_code": code,
                    "origin": expected_origin,
                    "severity": payload["severity"],
                    "fixture": str(directory),
                    "outcome": "passed",
                    "event_seq": seq,
                    "claim_status": "producer_evidence_required_by_closure",
                    "driver_status": "fixture metadata plus Rust producer-evidence map; reserved orphans excluded",
                    "wire_code_kind": payload.get("wire_code_kind", "typed-validation-code"),
                    "rfc_code_location": payload.get("rfc_code_location", "code.kind"),
                    "producer_evidence": payload.get("producer_evidence", "stage4 typed diagnostic producer"),
                },
                "span": None,
            }, sort_keys=True) + "\n")

check("stage4", stage4)
check("stage5", stage5)
for stage in ["stage4", "stage5"]:
    for expected in (fixture_root / "reject" / stage).glob("*/expected_diag.json"):
        payload = json.loads(expected.read_text(encoding="utf-8"))
        if payload.get("code") in reserved:
            if payload.get("reserved") is not True:
                raise SystemExit(f"reserved diagnostic fixture is not marked reserved: {payload['code']}")
        elif payload.get("reserved") is not False:
            raise SystemExit(f"active diagnostic fixture is marked reserved: {payload['code']}")
PY

echo "diagnostic coverage substrate complete: out_dir=$out_dir"

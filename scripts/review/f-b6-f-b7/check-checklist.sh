#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

build_id="${F_B6_F_B7_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)-checklist}"
out_dir="${F_B6_F_B7_OUT_DIR:-/tmp/f-b6-f-b7-closure/$build_id}"
ndjson="$out_dir/closure-checklist.ndjson"
mkdir -p "$out_dir"

python3 - "$out_dir" "$build_id" "$ndjson" <<'PY'
import json
import os
import pathlib
import re
import sys
import time

out_dir = pathlib.Path(sys.argv[1])
build_id = sys.argv[2]
ndjson = pathlib.Path(sys.argv[3])
repo = pathlib.Path.cwd()
required_files = [
    "gbf-codegen/tests/support/f_b6_f_b7/fixtures.rs",
    "gbf-codegen/tests/support/f_b6_f_b7/telemetry.rs",
    "gbf-codegen/tests/coverage_matrix_f_b6_f_b7.rs",
    "gbf-codegen/tests/fixtures/f_b6_f_b7/README.md",
    "gbf-codegen/tests/fixtures/f_b6_f_b7/reject/RESERVED.md",
    "scripts/review/f-b6-f-b7/closure-scope.md",
]
required_scripts = [
    "scripts/review/f-b6-f-b7/stage4-run.sh",
    "scripts/review/f-b6-f-b7/stage5-run.sh",
    "scripts/review/f-b6-f-b7/run-cert-verify.sh",
    "scripts/review/f-b6-f-b7/coverage-pack.sh",
    "scripts/review/f-b6-f-b7/closure-pack.sh",
    "scripts/review/f-b6-f-b7/verify-packet.sh",
    "scripts/review/f-b6-f-b7/check-checklist.sh",
]
checks = []
def text(path):
    return (repo / path).read_text(encoding="utf-8")

for path in required_files:
    checks.append((path, pathlib.Path(path).is_file(), "closure substrate file exists"))
for path in required_scripts:
    p = pathlib.Path(path)
    checks.append((path, p.is_file() and os.access(p, os.X_OK), "closure script exists and is executable"))
checks.append(("stage4-run.ndjson", (out_dir / "stage4-run.ndjson").is_file(), "Stage 4 packet emitted"))
checks.append(("stage5-run.ndjson", (out_dir / "stage5-run.ndjson").is_file(), "Stage 5 packet emitted"))
checks.append(("verify-packet.ndjson", (out_dir / "verify-packet.ndjson").is_file(), "gbf-verify packet emitted"))
checks.append(("stage4_diagnostics_coverage.ndjson", (out_dir / "stage4_diagnostics_coverage.ndjson").is_file(), "Stage 4 diagnostic coverage emitted"))
checks.append(("stage5_diagnostics_coverage.ndjson", (out_dir / "stage5_diagnostics_coverage.ndjson").is_file(), "Stage 5 diagnostic coverage emitted"))

scope_note = text("scripts/review/f-b6-f-b7/closure-scope.md")
checks.append((
    "closure-scope-stage6-deferred",
    "bd-2k0" in scope_note and "No Stage 6 driver" in scope_note,
    "F-B8/Stage 6 executable consumption is deferred to bd-2k0",
))
checks.append((
    "closure-scope-ci-artifacts",
    ".github" in scope_note and "out of scope" in scope_note and "CI artifact" in scope_note,
    "CI artifact attachment and workflow wiring are not claimed by this packet",
))
checks.append((
    "closure-scope-workspace-all-features",
    "gbf-experiments" in scope_note and "S2 feature mutex" in scope_note,
    "workspace all-features limitation is durable packet evidence",
))
checks.append((
    "closure-scope-stale-script-names",
    "cache-replay.sh" in scope_note and "check-§20-conformance.sh" in scope_note and "superseded" in scope_note,
    "historical non-existent script names are explicitly superseded",
))

def contains(path, needle):
    return needle in text(path)

def rg(paths, pattern):
    regex = re.compile(pattern, re.S)
    return any(regex.search(text(path)) for path in paths)

checklist = [
    (
        "18a-01-build-active-schema-id",
        contains("gbf-report/src/report_schemas/observation_plan_v1.rs", "build_active_semantic_checkpoint_schema.v1")
        and contains("scripts/review/f-b6-f-b7/stage4-run.sh", "build_active_semantic_checkpoint_schema.v1"),
        "build-active checkpoint re-emit schema id is present in report schema and packet fixture",
    ),
    (
        "18a-02-pure-cores-return-products-and-bodies",
        rg(["gbf-codegen/src/s4/observation_plan.rs"], r"ObservationPlanCore(Success|Product|Failure)")
        and rg(["gbf-codegen/src/s5/range_plan.rs"], r"RangePlanCore(Success|Product|Failure)"),
        "Stage 4/5 core product/body types exist outside ReportEnvelope emission",
    ),
    (
        "18a-03-registry-snapshots-in-k4",
        rg(["gbf-codegen/src/stage_cache.rs"], r"Stage4CacheKeyMaterial")
        and rg(["gbf-codegen/src/stage_cache.rs"], r"probe_registry_hash")
        and rg(["gbf-codegen/src/stage_cache.rs"], r"metric_registry_hash")
        and rg(["gbf-codegen/src/stage_cache.rs"], r"trace_event_layout_registry_hash"),
        "K4 material records observation registry snapshot hashes",
    ),
    (
        "18a-04-failure-memo-rewraps-audit-parents",
        rg(["gbf-codegen/src/stage_cache.rs"], r"rewrap_stage4_cached_failure")
        and rg(["gbf-codegen/src/stage_cache.rs"], r"rewrap_stage5_cached_failure")
        and rg(["gbf-codegen/src/stage_cache.rs"], r"audit_parent_rewrap"),
        "StageCache has Stage 4/5 cached-failure rewrap paths and telemetry",
    ),
    (
        "18a-05-semantic-stability-is-attachment-based",
        contains("gbf-codegen/src/s4/observation_plan.rs", "anchor_attachment_table_consistent_with_vectors")
        and contains("gbf-codegen/src/s4/observation_plan.rs", "semantic_attachment_map"),
        "semantic stability test is anchored to checkpoint attachments",
    ),
    (
        "18a-06-metric-weights-present",
        contains("gbf-codegen/src/s4/observation_plan.rs", "metric_probe_has_weight_field")
        and contains("gbf-report/src/report_schemas/observation_plan_v1.rs", "per_class_metric_weight_total"),
        "MetricProbe weight and operational-probe metric totals are schema-visible",
    ),
    (
        "18a-07-required-probes-cannot-be-disabled",
        contains("gbf-codegen/src/s4/observation_plan.rs", "disabled_required_probe_rejected"),
        "required probe disable rejection test exists",
    ),
    (
        "18a-08-chunked-cross-sum-uses-term-count",
        rg(["gbf-codegen/src/s5/range_plan.rs"], r"checked_mul_u64\(u64::from\(facts\.term_count\), per_term_abs_max\)")
        and rg(["gbf-verify/src/range_cert/independent.rs"], r"checked_mul_u64\(u64::from\(facts\.term_count\), \*per_term_abs_max\)"),
        "codegen and independent verifier compute cross_chunk_sum_bound from actual term_count",
    ),
    (
        "18a-09-renorm-loop-carries-renorm-spec",
        contains("gbf-report/src/report_schemas/range_plan_v1.rs", "RenormLoop { tile_len: u16, renorm: RenormSpec }")
        and contains("gbf-codegen/src/s5/range_plan.rs", "RenormLoop { tile_len, renorm }"),
        "RangePlan RenormLoop carries RenormSpec",
    ),
    (
        "18a-10-bitexact-renorm-loop-forbidden",
        contains("gbf-codegen/src/s5/range_plan.rs", "choose_tile_len_bitexact_renorm_loop_reserved_v1")
        and contains("gbf-codegen/src/s5/range_plan.rs", "verifies_renorm_loop_proof_bitexact_v1_reserved"),
        "BitExact RenormLoop reserved behavior is tested",
    ),
    (
        "18a-11-proof-scalar-widths-match-verifier",
        contains("gbf-codegen/src/s5/range_plan.rs", "accumulator_certificate_single_i16_proof_round_trip")
        and contains("gbf-codegen/src/s5/range_plan.rs", "reduction_site_facts_per_term_abs_max_q_is_u64")
        and contains("gbf-report/src/report_schemas/range_plan_v1.rs", "cross_chunk_sum_bound: u64")
        and contains("gbf-verify/src/range_cert/independent.rs", "I32_ENVELOPE_U64"),
        "certificate scalar widths are u64 in shared schema and verifier equations",
    ),
    (
        "18a-12-diagnostic-reachability-or-reserved",
        contains("gbf-codegen/tests/coverage_matrix_f_b6_f_b7.rs", "coverage_matrix_fixture_corpus_covers_non_reserved_rfc_codes")
        and contains("gbf-codegen/tests/fixtures/f_b6_f_b7/reject/RESERVED.md", "reserved"),
        "diagnostic coverage matrix and reserved-code documentation exist",
    ),
    (
        "18a-13-landed-code-reconciliation",
        not rg(["gbf-codegen/src/s4/observation_plan.rs"], r"SemanticCheckpointId::[A-Z]")
        and contains("gbf-codegen/src/s4/observation_plan.rs", "SemanticCheckpointKind")
        and contains("gbf-policy/src/compile.rs", "compile_profile_spec:2.0.0")
        and contains("gbf-policy/src/probe.rs", "ProbeImportanceClass")
        and contains("gbf-policy/src/metrics.rs", "MetricId")
        and contains("gbf-policy/src/trace_event_layout.rs", "ABI_TRACE_EVENT_PAYLOAD_BYTES"),
        "§20 landed-code reconciliation guards pass",
    ),
]
checks.extend(checklist)

with ndjson.open("w", encoding="utf-8") as f:
    for seq, (name, ok, evidence) in enumerate(checks, 1):
        f.write(json.dumps({
            "ts": f"unix:{time.time():.9f}",
            "event": "closure.checklist.item",
            "level": "INFO" if ok else "ERROR",
            "target": "scripts.review.f_b6_f_b7",
            "fields": {
                "build_id": build_id,
                "item": name,
                "outcome": "passed" if ok else "failed",
                "event_seq": seq,
                "claim_status": "packet_evidence_closed" if ok else "packet_evidence_failed",
                "evidence": evidence,
                "delegated_semantics": "No F-B6/F-B7 closure predicates are delegated after bd-2phk and bd-3hcq",
            },
            "span": None,
        }, sort_keys=True) + "\n")
failed = [name for name, ok, _evidence in checks if not ok]
if failed:
    raise SystemExit("closure checklist failed: " + ", ".join(failed))
PY

echo "closure checklist complete: out_dir=$out_dir"

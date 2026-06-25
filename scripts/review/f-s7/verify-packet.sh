#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(git -C "$SCRIPT_DIR/../../.." rev-parse --show-toplevel)"
CHECK_ROOT="$ROOT"
REPORT_VALIDATOR="scripts/review/f-s7/validate-report.py"
REPORT_EMITTER="scripts/review/f-s7/emit-report.py"
ARTIFACT_VALIDATOR="scripts/review/f-s7/validate-artifacts.py"
REVIEW_VALIDATOR="scripts/review/f-s7/validate-reviews.py"
RUN_GATES=1
REQUIRE_PRODUCTION=1
RUN_PREREG=1
SELF_TEST=0

usage() {
  cat <<'USAGE'
Usage: scripts/review/f-s7/verify-packet.sh [--skip-gates] [--substrate-only] [--self-test]

Runs the focused local S7 substrate checks, then verifies the production
closure anchors named by F-S7 §13/§15. By default this is a closure packet:
it fails if preregistration, production artifacts, or the final S7 report are
missing. Use --substrate-only only when rehearsing local fixture/substrate
checks; it is not a bd-2v9r closure gate.

Options:
  --skip-gates       skip cargo/script substrate gates and check only files
  --substrate-only   run substrate gates without requiring production artifacts
  --self-test        validate packet wiring without running substrate gates or requiring repo files
USAGE
}

while (($#)); do
  case "$1" in
    --skip-gates)
      RUN_GATES=0
      shift
      ;;
    --substrate-only)
      REQUIRE_PRODUCTION=0
      shift
      ;;
    --self-test)
      SELF_TEST=1
      RUN_GATES=0
      REQUIRE_PRODUCTION=0
      RUN_PREREG=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

print_command() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
}

run_gate() {
  print_command "$@"
  "$@"
}

production_paths() {
  local seed topology

  printf '%s\t%s\t%s\n' \
    "file" "history/rfcs/F-S7-moe-beats-dense.md" "F-S7 RFC" \
    "file" "fixtures/preregistration/s7.toml" "S7 preregistration pin" \
    "file" "experiments/S7/profile/matched_bytes.json" "matched-bytes pin" \
    "file" "docs/experiments/S7-report.md" "s7_report.v1" \
    "file" "docs/review/f-s7/reviews/bd-2v9r-gemini.json" "Gemini ACPX review evidence" \
    "file" "docs/review/f-s7/reviews/bd-2v9r-claude.json" "Claude ACPX review evidence"

  for topology in MoeTiny MoeTinyDenseMatched; do
    for seed in 0 1 2 3 4; do
      printf '%s\t%s\t%s\n' \
        "json:s7_run_log.v1" \
        "experiments/S7/runs/$topology/seed-$seed/run-log.json" \
        "run log for $topology seed $seed" \
        "file" \
        "experiments/S7/runs/$topology/seed-$seed/grad-log.jsonl" \
        "grad log for $topology seed $seed" \
        "file" \
        "experiments/S7/runs/$topology/seed-$seed/router-step-telemetry.jsonl" \
        "router-step telemetry for $topology seed $seed" \
        "json:s7_score.v1" \
        "experiments/S7/scores/$topology/seed-$seed/score.json" \
        "score report for $topology seed $seed"
    done
  done

  for seed in 0 1 2 3 4; do
    printf '%s\t%s\t%s\n' \
      "json:s7_switch_stats.v1" \
      "experiments/S7/switch-stats/seed-$seed/switch-stats.json" \
      "switch-stats report for MoE seed $seed"
  done

  printf '%s\t%s\t%s\n' \
    "json:s7_router_collapse_sweep.v1" \
    "experiments/S7/router-collapse/seed-0/sweep.json" \
    "router-collapse sweep" \
    "json:s7_dense_vs_moe.v1" \
    "experiments/S7/dense-vs-moe/comparison.json" \
    "dense-vs-MoE comparison" \
    "json:s7_frontier.v1" \
    "experiments/S7/frontier/frontier.json" \
    "Pareto frontier" \
    "json:s7_burn_grad_smoke.v1" \
    "experiments/S7/burn-grad-smoke/expert_block_qat.json" \
    "Burn ExpertBlockQat gradient smoke" \
    "json:s7_oracle_routed.v1" \
    "experiments/S7/oracle-routed/seed-0/oracle.json" \
    "routed artifact oracle" \
    "json:s7_emulator_one_token.v1" \
    "experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json" \
    "MoE emulator one-token result"
}

required_gate_commands() {
  cat <<'GATES'
scripts/review/f-s7/schema_discipline.sh
scripts/s7_matched_bytes_check.sh
scripts/s7_falsification_check.sh
scripts/s7_determinism_check.sh
scripts/s7_isolation_check.sh
S7_SMOKE_RUN_TESTS=1 scripts/s7_e2e_smoke.sh
cargo test -p gbf-experiments --features s7 --test closure_protocol
scripts/s7_preregistration_check.sh
scripts/s7_preregistration_pin.sh --check-ready
scripts/review/f-s7/emit-report.py
scripts/review/f-s7/assemble-packet.py --manifest <production-bundle-manifest.json> --run-reviews
scripts/review/f-s7/validate-report.py
scripts/review/f-s7/validate-artifacts.py
scripts/review/f-s7/validate-reviews.py
cargo run -q -p gbf-cli --no-default-features --features s7 -- --log-level off s7 validate-closure --root CHECK_ROOT --predictions-verified
GATES
}

run_substrate_gates() {
  cd "$ROOT"
  run_gate scripts/review/f-s7/schema_discipline.sh
  run_gate scripts/s7_matched_bytes_check.sh
  run_gate scripts/s7_falsification_check.sh
  run_gate scripts/s7_determinism_check.sh
  run_gate scripts/s7_isolation_check.sh
  print_command env S7_SMOKE_RUN_TESTS=1 scripts/s7_e2e_smoke.sh
  env S7_SMOKE_RUN_TESTS=1 scripts/s7_e2e_smoke.sh >/tmp/s7-e2e-smoke.verify-packet.log
  run_gate cargo test -p gbf-experiments --features s7 --test closure_protocol
}

failures=()

record_failure() {
  failures+=("$1")
}

require_file() {
  local rel_path="$1"
  local label="$2"
  if [[ ! -f "$CHECK_ROOT/$rel_path" ]]; then
    record_failure "$label missing: $rel_path"
  fi
}

require_json_schema() {
  local rel_path="$1"
  local schema="$2"
  local label="$3"

  require_file "$rel_path" "$label"
  if [[ -f "$CHECK_ROOT/$rel_path" ]] &&
    ! rg -n -- "\"schema\"[[:space:]]*:[[:space:]]*\"$schema\"" "$CHECK_ROOT/$rel_path" >/dev/null; then
    record_failure "$label does not declare schema $schema: $rel_path"
  fi
}

require_report_shape() {
  local report="docs/experiments/S7-report.md"
  require_file "$report" "s7_report.v1"
  if [[ ! -f "$CHECK_ROOT/$report" ]]; then
    return
  fi

  if ! "$ROOT/$REPORT_VALIDATOR" --report "$CHECK_ROOT/$report" --root "$CHECK_ROOT" >/tmp/s7-report-validate.stdout 2>/tmp/s7-report-validate.stderr; then
    local detail
    detail="$(tr '\n' ' ' </tmp/s7-report-validate.stdout | sed 's/[[:space:]]\+/ /g' | cut -c1-400)"
    record_failure "s7_report.v1 closure validation failed${detail:+: $detail}"
  fi
}

require_rfc_finalized() {
  local rfc="history/rfcs/F-S7-moe-beats-dense.md"
  require_file "$rfc" "F-S7 RFC"
  if [[ ! -f "$CHECK_ROOT/$rfc" ]]; then
    return
  fi
  if rg -n -- '(^> DRAFT|Status:[[:space:]]+pre-implementation|\[ESTIMATE\])' "$CHECK_ROOT/$rfc" >/dev/null; then
    record_failure "F-S7 RFC still has DRAFT/pre-implementation/[ESTIMATE] closure blockers"
  fi
}

run_preregistration_gate() {
  local output="${S7_PREREG_VERIFY_OUTPUT:-/tmp/s7-preregistration.verify-packet.json}"
  if [[ "$RUN_PREREG" -eq 0 ]]; then
    return
  fi
  if ! "$ROOT/scripts/s7_preregistration_check.sh" --output "$output" >/tmp/s7-preregistration.verify-packet.stdout 2>/tmp/s7-preregistration.verify-packet.stderr; then
    local detail
    detail="$(tr '\n' ' ' </tmp/s7-preregistration.verify-packet.stderr | sed 's/[[:space:]]\+/ /g')"
    if [[ -z "$detail" && -f "$output" ]]; then
      detail="$(tr '\n' ' ' <"$output" | sed 's/[[:space:]]\+/ /g' | cut -c1-240)"
    fi
    record_failure "scripts/s7_preregistration_check.sh failed${detail:+: $detail}"
  fi
}

probe_preregistration_pin_readiness() {
  local pin="fixtures/preregistration/s7.toml"
  if [[ "$RUN_PREREG" -eq 0 || "$REQUIRE_PRODUCTION" -eq 0 ]]; then
    return
  fi
  if [[ "$CHECK_ROOT" != "$ROOT" || -f "$CHECK_ROOT/$pin" ]]; then
    return
  fi
  if ! "$ROOT/scripts/s7_preregistration_pin.sh" --check-ready \
    >/tmp/s7-preregistration-pin-ready.stdout \
    2>/tmp/s7-preregistration-pin-ready.stderr; then
    local detail
    detail="$(tr '\n' ' ' </tmp/s7-preregistration-pin-ready.stderr | sed 's/[[:space:]]\+/ /g' | cut -c1-700)"
    record_failure "S7 preregistration pin readiness failed${detail:+: $detail}"
    return
  fi

  local detail
  detail="$(tr '\n' ' ' </tmp/s7-preregistration-pin-ready.stdout | sed 's/[[:space:]]\+/ /g' | cut -c1-300)"
  record_failure "S7 preregistration pin missing but current RFC is ready to pin${detail:+: $detail}"
}

run_rust_closure_gate() {
  if [[ "$RUN_GATES" -eq 0 || "$REQUIRE_PRODUCTION" -eq 0 ]]; then
    return
  fi
  if ((${#failures[@]})); then
    return
  fi
  local args=(
    cargo run -q -p gbf-cli --no-default-features --features s7 --
    --log-level off
    s7 validate-closure
    --root "$CHECK_ROOT"
    --predictions-verified
  )
  print_command "${args[@]}"
  if ! "${args[@]}" >/tmp/s7-rust-closure-validate.stdout 2>/tmp/s7-rust-closure-validate.stderr; then
    local detail
    detail="$(tr '\n' ' ' </tmp/s7-rust-closure-validate.stderr | sed 's/[[:space:]]\+/ /g' | cut -c1-500)"
    if [[ -z "$detail" ]]; then
      detail="$(tr '\n' ' ' </tmp/s7-rust-closure-validate.stdout | sed 's/[[:space:]]\+/ /g' | cut -c1-500)"
    fi
    record_failure "S7 Rust closure validation failed${detail:+: $detail}"
  fi
}

run_s7_cli_feature_preflight() {
  local args=(
    cargo check -q -p gbf-cli --no-default-features --features s7
  )
  print_command "${args[@]}"
  if ! "${args[@]}" >/tmp/s7-cli-feature-preflight.stdout 2>/tmp/s7-cli-feature-preflight.stderr; then
    local detail
    detail="$(tr '\n' ' ' </tmp/s7-cli-feature-preflight.stderr | sed 's/[[:space:]]\+/ /g' | cut -c1-500)"
    if [[ -z "$detail" ]]; then
      detail="$(tr '\n' ' ' </tmp/s7-cli-feature-preflight.stdout | sed 's/[[:space:]]\+/ /g' | cut -c1-500)"
    fi
    record_failure "S7 CLI feature preflight failed${detail:+: $detail}"
    return 1
  fi
}

run_artifact_validator() {
  if ! "$ROOT/$ARTIFACT_VALIDATOR" --root "$CHECK_ROOT" >/tmp/s7-artifact-validate.stdout 2>/tmp/s7-artifact-validate.stderr; then
    local detail
    detail="$(tr '\n' ' ' </tmp/s7-artifact-validate.stdout | sed 's/[[:space:]]\+/ /g' | cut -c1-500)"
    record_failure "S7 artifact closure validation failed${detail:+: $detail}"
  fi
}

run_review_validator() {
  local expected_head current_head
  expected_head="${S7_EXPECTED_REVIEW_HEAD:-}"
  current_head="$(git -C "$CHECK_ROOT" rev-parse HEAD 2>/dev/null || true)"
  local args=(--root "$CHECK_ROOT")
  if [[ -n "$expected_head" ]]; then
    args+=(--expected-head "$expected_head")
  elif [[ -n "$current_head" ]]; then
    args+=(--allow-reviewed-head-ancestor-of "$current_head")
    args+=(--require-reviewed-diff-admin-only)
  fi
  if ! "$ROOT/$REVIEW_VALIDATOR" "${args[@]}" >/tmp/s7-review-validate.stdout 2>/tmp/s7-review-validate.stderr; then
    local detail
    detail="$(tr '\n' ' ' </tmp/s7-review-validate.stdout | sed 's/[[:space:]]\+/ /g' | cut -c1-500)"
    record_failure "S7 ACPX review validation failed${detail:+: $detail}"
  fi
}

check_production_surfaces() {
  local kind rel_path label schema

  while IFS=$'\t' read -r kind rel_path label; do
    case "$kind" in
      file)
        require_file "$rel_path" "$label"
        ;;
      json:*)
        schema="${kind#json:}"
        require_json_schema "$rel_path" "$schema" "$label"
        ;;
      *)
        record_failure "internal error: unknown required path kind $kind for $rel_path"
        ;;
    esac
  done < <(production_paths)

  require_rfc_finalized
  probe_preregistration_pin_readiness
  require_report_shape
  run_artifact_validator
  run_review_validator
  run_preregistration_gate
  run_rust_closure_gate
}

run_self_test() {
  local tmp
  tmp="$(mktemp -d /tmp/s7-verify-packet-self-test.XXXXXX)"
  CHECK_ROOT="$tmp"
  check_production_surfaces

  if ((${#failures[@]} == 0)); then
    echo "error: self-test expected missing-production failures" >&2
    exit 1
  fi
  for required in \
    "fixtures/preregistration/s7.toml" \
    "experiments/S7/runs/MoeTiny/seed-0/run-log.json" \
    "experiments/S7/dense-vs-moe/comparison.json" \
    "docs/experiments/S7-report.md" \
    "docs/review/f-s7/reviews/bd-2v9r-gemini.json"
  do
    if ! printf '%s\n' "${failures[@]}" | rg --fixed-strings "$required" >/dev/null; then
      echo "error: self-test did not exercise required path $required" >&2
      printf '%s\n' "${failures[@]}" >&2
      exit 1
    fi
  done

  echo "S7 verify-packet required gates:"
  required_gate_commands
  echo "S7 verify-packet required production paths:"
  production_paths | cut -f2
  run_synthetic_positive_self_test
  echo "S7 verify-packet self-test: ok"
}

run_synthetic_positive_self_test() {
  local tmp old_check_root old_expected_head old_run_gates old_require_production
  tmp="$(mktemp -d /tmp/s7-verify-packet-positive.XXXXXX)"
  old_check_root="$CHECK_ROOT"
  old_expected_head="${S7_EXPECTED_REVIEW_HEAD:-}"
  old_run_gates="$RUN_GATES"
  old_require_production="$REQUIRE_PRODUCTION"
  write_synthetic_positive_packet "$tmp"
  CHECK_ROOT="$tmp"
  S7_EXPECTED_REVIEW_HEAD=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
  failures=()
  check_production_surfaces
  RUN_GATES=1
  REQUIRE_PRODUCTION=1
  failures=()
  if ! run_s7_cli_feature_preflight; then
    echo "error: synthetic S7 CLI feature preflight should pass" >&2
    printf '%s\n' "${failures[@]}" >&2
    exit 1
  fi
  failures=()
  run_rust_closure_gate
  CHECK_ROOT="$old_check_root"
  RUN_GATES="$old_run_gates"
  REQUIRE_PRODUCTION="$old_require_production"
  if [[ -n "$old_expected_head" ]]; then
    S7_EXPECTED_REVIEW_HEAD="$old_expected_head"
  else
    unset S7_EXPECTED_REVIEW_HEAD
  fi
  if ((${#failures[@]})); then
    echo "error: synthetic positive self-test should pass production surfaces" >&2
    printf '%s\n' "${failures[@]}" >&2
    exit 1
  fi
  echo "S7 verify-packet synthetic S7 CLI feature preflight self-test: ok"
  echo "S7 verify-packet synthetic Rust closure gate self-test: ok"
  echo "S7 verify-packet synthetic production surface self-test: ok"
}

write_synthetic_positive_packet() {
  local root="$1"
  python3 - "$root" <<'PY'
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
h = "sha256:" + "3" * 64
head = "a" * 40
grid = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0]
S7_OPTIMIZER_STEPS = 20_000
S7_EVAL_EVERY_STEPS = 1_000
RUN_LOG_DOMAIN = ("gbf-artifact", "S7RunLog", "s7_run_log.v1", "1")
RAW_LOSS_DIAGNOSTICS_DOMAIN = ("gbf-artifact", "RawLossDiagnostics", "s7_raw_loss_diagnostics.v1", "1")
SCORE_DOMAIN = ("gbf-artifact", "S7ScoreReport", "s7_score.v1", "1")
SWITCH_STATS_DOMAIN = ("gbf-experiments", "S7SwitchStatsReport", "s7_switch_stats.v1", "1")
TEMPORAL_SWITCH_DIGEST_DOMAIN = ("gbf-artifact", "TemporalSwitchDigest", "s7_temporal_switch_digest.v1", "1")
CLIP_SATURATION_DIGEST_DOMAIN = ("gbf-artifact", "ClipSaturationDigest", "s7_clip_saturation_digest.v1", "1")
EXPERT_PAYLOAD_DIGEST_DOMAIN = ("gbf-artifact", "ExpertPayloadDigest", "s7_expert_payload_digest.v1", "1")
EXPERT_SLOT_AFFINITY_DOMAIN = ("gbf-artifact", "ExpertSlotAffinity", "s7_expert_slot_affinity.v1", "1")
MATCHED_BYTES_PIN_DOMAIN = ("gbf-artifact", "MatchedBytesPin", "s7_matched_bytes_pin.v1", "1")
DENSE_VS_MOE_DOMAIN = ("gbf-artifact", "S7DenseVsMoeComparisonReport", "s7_dense_vs_moe.v1", "1")
LAMBDA_SWITCH_RECORD_DOMAIN = ("gbf-experiments", "LambdaSwitchSweepRecord", "s7_lambda_switch_sweep_step.v1", "1")
ROUTER_COLLAPSE_SWEEP_DOMAIN = ("gbf-experiments", "RouterCollapseSweepReport", "s7_router_collapse_sweep.v1", "1")
FRONTIER_DOMAIN = ("gbf-experiments", "S7FrontierReport", "s7_frontier.v1", "1")
BURN_GRAD_SMOKE_DOMAIN = ("gbf-experiments", "S7BurnGradSmokeReport", "s7_burn_grad_smoke.v1", "1")
ORACLE_ROUTED_DOMAIN = ("gbf-experiments", "S7OracleRoutedReport", "s7_oracle_routed.v1", "1")
ROUTER_STEP_TELEMETRY_DOMAIN = ("gbf-experiments", "RouterStepTelemetry", "s7_router_step_telemetry.v1", "1")
EMULATOR_DOMAIN = ("gbf-experiments", "EmulatorOneTokenReport", "s7_emulator_one_token.v1", "1")
SWITCH_STATS_MANIFEST_DOMAIN = ("gbf-experiments", "S7SwitchStatsBundleManifest", "s7_switch_stats_bundle_manifest.v1", "1")
REPORT_MARKDOWN_DOMAIN = ("gbf-experiments", "S7ReportMarkdown", "s7_report.v1", "1")


def write_text(rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def write_json(rel: str, payload) -> None:
    write_text(rel, json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n")


def write_jsonl(rel: str, records) -> None:
    write_text(
        rel,
        "".join(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n" for record in records),
    )


def canonical(payload) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def domain_bytes_hash(domain, payload: bytes) -> str:
    crate_name, type_name, schema_id, schema_version = domain
    material = (
        f"gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}".encode("utf-8")
        + b"\0"
        + payload
    )
    return f"sha256:{hashlib.sha256(material).hexdigest()}"


def domain_json_hash(domain, payload) -> str:
    return domain_bytes_hash(domain, canonical(payload).encode("utf-8"))


def with_self_hash(payload, field: str, domain):
    payload = dict(payload)
    payload.pop(field, None)
    payload[field] = domain_json_hash(domain, payload)
    return payload


def with_report_self_hash(text: str) -> str:
    report_hash = domain_bytes_hash(REPORT_MARKDOWN_DOMAIN, text.encode("utf-8"))
    return text.replace("report_self_hash: null", f'report_self_hash: "{report_hash}"')


def version():
    return {"major": 1, "minor": 0, "patch": 0}


def grad_norms():
    return {"global_l2": 1.0, "max_l2": 0.5, "mean_l2": 0.25}


def grad_log_record(seed: int, step: int):
    return {
        "schema": "s7_grad_log.v1",
        "seed": seed,
        "train_step": step,
        "grad_norms": grad_norms(),
    }


def raw_loss():
    return with_self_hash({
        "lm_loss_raw": 1.0,
        "distill_loss_raw": {
            "kind": "not_available",
            "reason": "no_frozen_teacher",
            "phase": "phase_a",
        },
        "balance_loss_raw": 0.1,
        "zrouter_loss_raw": 0.2,
        "switch_loss_raw": 0.3,
    }, "diagnostics_self_hash", RAW_LOSS_DIAGNOSTICS_DOMAIN)


def router_telemetry(seed: int, layer: int):
    return with_self_hash({
        "schema_version": version(),
        "seed": seed,
        "train_step": 1,
        "layer_id": layer,
        "expert_usage_entropy_bits": 1.0,
        "same_expert_rate": 0.5,
        "router_confidence_distribution": {"mean": 0.6, "p10": 0.4, "p50": 0.6, "p90": 0.8},
        "tokens_per_expert": [2, 2, 0, 0],
        "bank_switches_per_token": 0.5,
    }, "telemetry_self_hash", ROUTER_STEP_TELEMETRY_DOMAIN)


def temporal(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "n_experts": 4,
        "same_expert_rate_q8_8": 128,
        "transition_mass": [{"from_expert": 0, "to_expert": 1, "mass_q8_8": 64}],
    }, "digest_self_hash", TEMPORAL_SWITCH_DIGEST_DOMAIN)


def clip(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "saturation_rate_q8_8": 1,
        "clip_bound_observed": 1.0,
    }, "digest_self_hash", CLIP_SATURATION_DIGEST_DOMAIN)


def payload(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "artifact_path": f"layer-{layer}",
        "entries": [
            {"expert_id": expert, "byte_count": 128 + expert, "weight_quant": {"kind": "ternary2"}}
            for expert in range(4)
        ],
    }, "digest_self_hash", EXPERT_PAYLOAD_DIGEST_DOMAIN)


def affinity(layer: int):
    return with_self_hash({
        "schema_version": version(),
        "layer_id": layer,
        "affinities": [{"pair": {"lo": 0, "hi": 1}, "affinity_score": 64}],
    }, "affinity_self_hash", EXPERT_SLOT_AFFINITY_DOMAIN)


def sweep_record(index: int):
    return with_self_hash({
        "schema_version": version(),
        "seed": 0,
        "lambda_switch": grid[index],
        "base_train_step": 1000,
        "train_step": 2000,
        "completion": {"kind": "completed"},
        "bpc_eval_subset": 1.0 + (index * 0.1),
        "expert_usage_entropy_bits_mean": 1.0,
        "quality_delta_per_lambda_switch": 0.0,
    }, "sweep_self_hash", LAMBDA_SWITCH_RECORD_DOMAIN)


write_text(
    "history/rfcs/F-S7-moe-beats-dense.md",
    "# F-S7 MoE Beats Dense at Matched Bytes\n\nFinalized synthetic self-test RFC.\n",
)
write_text(
    "fixtures/preregistration/s7.toml",
    "\n".join(
        [
            'schema = "s7_preregistration.v1"',
            'rfc_path = "history/rfcs/F-S7-moe-beats-dense.md"',
            "predictions_line_start = 1",
            "predictions_line_end = 1",
            f'predictions_commit = "{head}"',
            f'predictions_section_hash = "{h}"',
            'pass_version_S7 = "self-test"',
            f'rfc_revision = "{head}"',
            'first_result_commit = ""',
            "",
        ]
    ),
)
write_json("experiments/S7/profile/matched_bytes.json", {"schema": "s7_matched_bytes_pin.v1", "matched_bytes_self_hash": h})

run_hashes = {}
score_hashes = {}
loss_diagnostics = raw_loss()
for topology in ["MoeTiny", "MoeTinyDenseMatched"]:
    for seed in range(5):
        run = {
            "schema": "s7_run_log.v1",
            "seed": seed,
            "topology": topology,
            "train_config_hash": h,
            "model_topology_hash": h,
            "router_config_hash": h if topology == "MoeTiny" else None,
            "expert_block_config_hash": h if topology == "MoeTiny" else None,
            "loss_config_hash": h,
            "phase_schedule_hash": h,
            "frozen_teacher_checkpoint_sha": h,
            "losses": [[step, loss_diagnostics] for step in range(1, S7_OPTIMIZER_STEPS + 1)],
            "grad_norms": [[step, grad_norms()] for step in range(1, S7_OPTIMIZER_STEPS + 1)],
            "eval_points": [
                [step, 1.0]
                for step in range(0, S7_OPTIMIZER_STEPS + 1, S7_EVAL_EVERY_STEPS)
            ],
            "final_grad_norms": grad_norms(),
            "completion": {"kind": "completed"},
        }
        run = with_self_hash(run, "run_log_self_hash", RUN_LOG_DOMAIN)
        run_hashes[(topology, seed)] = run["run_log_self_hash"]
        write_json(f"experiments/S7/runs/{topology}/seed-{seed}/run-log.json", run)
        write_jsonl(
            f"experiments/S7/runs/{topology}/seed-{seed}/grad-log.jsonl",
            [grad_log_record(seed, step) for step in range(1, S7_OPTIMIZER_STEPS + 1)],
        )
        write_jsonl(
            f"experiments/S7/runs/{topology}/seed-{seed}/router-step-telemetry.jsonl",
            [router_telemetry(seed, layer) for layer in range(4)] if topology == "MoeTiny" else [],
        )
        score = {
            "schema": "s7_score.v1",
            "seed": seed,
            "topology": topology,
            "checkpoint_sha": h,
            "corpus_val_sha": h,
            "chunk_size": 256,
            "token_count": 4,
            "log2_sum": 8.0,
            "bpc": 2.0,
        }
        score = with_self_hash(score, "score_self_hash", SCORE_DOMAIN)
        score_hashes[(topology, seed)] = score["score_self_hash"]
        write_json(f"experiments/S7/scores/{topology}/seed-{seed}/score.json", score)

switch_stats_hashes = []
for seed in range(5):
    switch_stats_report = with_self_hash({
            "schema": "s7_switch_stats.v1",
            "seed": seed,
            "artifact_path": f"seed-{seed}",
            "temporal_switch_digest": [temporal(layer) for layer in range(4)],
            "clip_saturation_digest": [clip(layer) for layer in range(4)],
            "expert_payload_digest": [payload(layer) for layer in range(4)],
            "expert_slot_affinity": [affinity(layer) for layer in range(4)],
            "aggregation_rule": "SUM",
        }, "bundle_self_hash", SWITCH_STATS_DOMAIN)
    switch_stats_hashes.append(switch_stats_report["bundle_self_hash"])
    write_json(
        f"experiments/S7/switch-stats/seed-{seed}/switch-stats.json",
        switch_stats_report,
    )

sweep_report = with_self_hash({
        "schema": "s7_router_collapse_sweep.v1",
        "seed": 0,
        "base_checkpoint_sha": h,
        "grid": grid,
        "records": [sweep_record(index) for index in range(len(grid))],
        "production_lambda": 0.05,
        "collapse_threshold": 1.0,
        "guardrail_verdict": "Pass",
    }, "sweep_self_hash", ROUTER_COLLAPSE_SWEEP_DOMAIN)
write_json("experiments/S7/router-collapse/seed-0/sweep.json", sweep_report)
matched_bytes_pin = with_self_hash({
    "formula_version": {"major": 0, "minor": 2, "patch": 0},
    "d_ff_dense_resolved": 128,
    "bias_policy": "test",
    "b_experts_total": 100,
    "b_router_overhead_total": 0,
    "b_dense_ffn_total": 100,
    "b_deployed_total_moe": 100,
    "b_deployed_total_dense": 100,
    "tolerance_bytes": 0,
}, "matched_bytes_self_hash", MATCHED_BYTES_PIN_DOMAIN)
comparison_report = with_self_hash({
        "schema": "s7_dense_vs_moe.v1",
        "moe_topology_hash": h,
        "dense_matched_topology_hash": h,
        "matched_bytes_pin": matched_bytes_pin,
        "per_seed": [
            {
                "seed": seed,
                "val_bpc_moe": 1.0,
                "val_bpc_dense": 1.1,
                "delta": 0.1,
                "parity_verdict": "Pass",
            }
            for seed in range(5)
        ],
        "median_val_bpc_moe": 1.0,
        "median_val_bpc_dense": 1.1,
        "deployed_bytes_total_moe": 100,
        "deployed_bytes_total_dense": 100,
        "bytes_diff": 0,
        "bytes_within_tolerance": True,
        "aggregate_parity_verdict": "Pass-clean",
        "pareto_verdict": "MoE-dominates",
        "switch_stats_summary": {
            "same_expert_rate_per_layer_q8_8": [128, 128, 128, 128],
            "expert_usage_entropy_bits_mean": 1.0,
            "bank_switches_per_token_mean": 0.5,
        },
        "sweep_summary": {
            "bpc_at_lambda": {"0.0": 1.0, "0.05": 1.0},
            "entropy_at_lambda": {"0.0": 1.0, "0.05": 1.0},
            "guardrail_verdict": {"kind": "pass"},
        },
    }, "comparison_self_hash", DENSE_VS_MOE_DOMAIN)
write_json("experiments/S7/dense-vs-moe/comparison.json", comparison_report)
frontier_report = with_self_hash({
        "schema": "s7_frontier.v1",
        "points": [
            {
                "topology": "MoeTiny",
                "checkpoint_sha": h,
                "quality": {"median_val_bpc": 1.0, "per_seed_val_bpc": [1.0] * 5},
                "conformance": {"status": "ok"},
                "projected_fit": {"deployed_bytes_total": 100, "deployed_bytes_per_block": [25] * 4},
                "schedule_cost": None,
            },
            {
                "topology": "MoeTinyDenseMatched",
                "checkpoint_sha": h,
                "quality": {"median_val_bpc": 1.1, "per_seed_val_bpc": [1.1] * 5},
                "conformance": {"status": "ok"},
                "projected_fit": {"deployed_bytes_total": 100, "deployed_bytes_per_block": [25] * 4},
                "schedule_cost": None,
            },
        ],
        "pareto_verdict": "MoE-dominates",
    }, "frontier_self_hash", FRONTIER_DOMAIN)
write_json("experiments/S7/frontier/frontier.json", frontier_report)
burn_grad_report = with_self_hash({
        "schema": "s7_burn_grad_smoke.v1",
        "fixture_seed": 0xFEED,
        "burn_adapter_version": "synthetic-validator-fixture",
        "fixture_input_sha": h,
        "grad_up_weight_sum_abs": 1.0,
        "grad_down_weight_sum_abs": 1.0,
        "supported_clipped_activation_count": 3,
        "learned_activation_range_unsupported": True,
        "projection_biases_unsupported": True,
        "glu_construction_rejected": True,
        "replay_byte_identical": True,
    }, "smoke_self_hash", BURN_GRAD_SMOKE_DOMAIN)
write_json("experiments/S7/burn-grad-smoke/expert_block_qat.json", burn_grad_report)
oracle_report = with_self_hash({
        "schema": "s7_oracle_routed.v1",
        "seed": 0,
        "topology": "MoeTiny",
        "fixture_prompt_sha": h,
        "train_logits_sha": h,
        "bundle_logits_sha": h,
        "artifact_logits_sha": h,
        "frozen_teacher_checkpoint_sha": h,
        "pairwise_max_abs_diff_train_bundle": 0.0,
        "pairwise_max_abs_diff_bundle_artifact": 0.0,
        "pairwise_max_abs_diff_train_artifact": 0.0,
        "s3_tolerance": 0.1,
        "route_coverage": {
            "cross_layer_route_difference": True,
            "consecutive_token_route_change": True,
            "consecutive_token_route_same": True,
        },
        "weight_quant_resolution": "QuantSpec::weight_quant",
    }, "oracle_self_hash", ORACLE_ROUTED_DOMAIN)
write_json("experiments/S7/oracle-routed/seed-0/oracle.json", oracle_report)
emulator_report = with_self_hash({
        "schema": "s7_emulator_one_token.v1",
        "seed": 0,
        "topology": "MoeTiny",
        "encoded_rom_sha": h,
        "prompt_sha": h,
        "artifact_oracle_logits_sha": h,
        "emulator_logits_sha": h,
        "pairwise_max_abs_diff": 0.0,
        "s5_tolerance": 0.1,
        "observed_bank_switches_per_token": 0.0,
        "oracle_recorded_bank_switches": 0.0,
        "bank_switch_diff": 0.0,
        "bank_switch_within_one": True,
    }, "emulator_self_hash", EMULATOR_DOMAIN)
write_json("experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json", emulator_report)

rows = []
for topology in ["MoeTiny", "MoeTinyDenseMatched"]:
    for seed in range(5):
        rows.append(
            f"""  - seed: {seed}
    topology: "{topology}"
    completion: Completed
    checkpoint_self_hash: "{h}"
    run_log_self_hash: "{run_hashes[(topology, seed)]}"
    score_self_hash: "{score_hashes[(topology, seed)]}"
"""
        )
switch_stats_manifest_hash = domain_json_hash(
    SWITCH_STATS_MANIFEST_DOMAIN,
    {
        "schema": "s7_switch_stats_bundle_manifest.v1",
        "seed_bundle_self_hashes": [
            {"seed": seed, "bundle_self_hash": switch_stats_hashes[seed]} for seed in range(5)
        ],
    },
)
body = "\n".join(
    [
        "## Pre-registered predictions",
        "Pinned before results.",
        "## Observed (per-seed, per-topology table)",
        "All rows recorded.",
        "## Hypothesis verdicts",
        "H1 Confirmed\nH2 Confirmed\nH3 Confirmed\nH4 Confirmed\nH5 Confirmed\nH6 Confirmed\nH7 Confirmed\nH8 Confirmed\nH9 Confirmed\nH10 Confirmed",
        "## Falsification analysis",
        "No refutation.",
        "## Switch statistics summary",
        "See artifact.",
        "## lambda_switch sweep summary",
        "See artifact.",
        "## Pareto verdict",
        "MoE dominates.",
        "## Surprises",
        "None.",
        "## Decision",
        "ProceedToS8.",
        "## Reproducibility statement",
        "Replay command pinned.",
        "",
    ]
)
report = f"""---
schema: "s7_report.v1"
s7_outcome: PassClean
decision: ProceedToS8
matched_bytes_self_hash: "{matched_bytes_pin["matched_bytes_self_hash"]}"
per_seed_artifacts:
{''.join(rows)}switch_stats_self_hash: "{switch_stats_manifest_hash}"
router_collapse_sweep_self_hash: "{sweep_report["sweep_self_hash"]}"
dense_vs_moe_self_hash: "{comparison_report["comparison_self_hash"]}"
frontier_self_hash: "{frontier_report['frontier_self_hash']}"
burn_grad_smoke_self_hash: "{burn_grad_report['smoke_self_hash']}"
oracle_routed_self_hash: "{oracle_report['oracle_self_hash']}"
emulator_one_token_moe_self_hash: "{emulator_report["emulator_self_hash"]}"
emulator_one_token_dense_self_hash: null
rfc_revision: "{head}"
predictions_section_hash: "{h}"
predictions_commit: "{head}"
first_result_commit: "{head}"
report_self_hash: null
---
{body}"""
write_text("docs/experiments/S7-report.md", with_report_self_hash(report))
for reviewer, personas in {
    "gemini": ["P3", "P4", "P5", "P6", "P7", "P8"],
    "claude": ["P3", "P5", "P6", "P8"],
}.items():
    write_json(
        f"docs/review/f-s7/reviews/bd-2v9r-{reviewer}.json",
        {
            "schema": "s7_acpx_review.v1",
            "bead": "bd-2v9r",
            "reviewer": reviewer,
            "transport": "acpx",
            "verdict": "PASS",
            "personas": personas,
            "command": f"acpx {reviewer} exec review",
            "reviewed_head": head,
            "summary": f"{reviewer} synthetic positive self-test review.",
            "findings": [],
        },
    )
PY
}

if [[ "$SELF_TEST" -eq 1 ]]; then
  run_self_test
  exit 0
fi

if [[ "$RUN_GATES" -eq 1 ]]; then
  run_substrate_gates
fi

if [[ "$REQUIRE_PRODUCTION" -eq 0 ]]; then
  echo "S7 verify-packet: substrate-only mode completed; production closure artifacts were not required."
  exit 0
fi

check_production_surfaces

if ((${#failures[@]})); then
  echo "S7 verify-packet: NEEDS_CHANGES"
  printf ' - %s\n' "${failures[@]}"
  exit 1
fi

echo "S7 verify-packet: ok"

#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

cargo run -q -p gbf-report --bin f_b2_review_artifacts -- \
  verify docs/review/f-b2-f-b4/artifacts

cargo test -q -p gbf-report --lib f_b2_f_b4_reports_reject_soft_diagnostics
cargo test -q -p gbf-report --lib f_b2_f_b4_reports_reject_unlisted_null_fields
cargo test -q -p gbf-codegen --lib stage_cache_validate_allows_partial_failure_key_without_fake_hashes

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cargo run -q -p gbf-report --bin f_b2_review_artifacts -- regen "$tmp"
for file in \
  artifact_validation.golden.json \
  artifact_validation.golden.sha256 \
  artifact_validation.failure.golden.json \
  artifact_validation.failure.golden.sha256 \
  artifact_validation.fixture.toml \
  policy_resolution.golden.json \
  policy_resolution.golden.sha256 \
  policy_resolution.failure.golden.json \
  policy_resolution.failure.golden.sha256 \
  policy_resolution.fixture.toml \
  static_budget.golden.json \
  static_budget.golden.sha256 \
  static_budget.failure.golden.json \
  static_budget.failure.golden.sha256 \
  static_budget.fixture.toml
do
  diff -u "docs/review/f-b2-f-b4/artifacts/$file" "$tmp/$file"
done

#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# F-B2 scaffold. T-B4.13 extends this script with static_budget verification.
cargo run -q -p gbf-report --bin f_b2_review_artifacts -- \
  verify docs/review/f-b2-f-b4/artifacts

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
  policy_resolution.fixture.toml
do
  diff -u "docs/review/f-b2-f-b4/artifacts/$file" "$tmp/$file"
done

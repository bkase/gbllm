#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

cargo test -p gbf-codegen f_b2_resolve_policy_emits_profile_v2_version_and_caps_for_all_canonical_profiles
cargo run -q -p gbf-report --bin f_b2_review_artifacts verify docs/review/f-b2-f-b4/artifacts

policy_golden="docs/review/f-b2-f-b4/artifacts/policy_resolution.golden.json"
rg -q '"compile_profile_spec_version":"2\.0\.0"' "$policy_golden"
rg -q '"range_caps":\{' "$policy_golden"
rg -q '"observation_caps":\{' "$policy_golden"

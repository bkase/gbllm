#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

for profile in bringup default recovery trace; do
  fixture="gbf-policy/fixtures/compile-profiles/${profile}.profile.toml"
  rg -q '^schema_version = "2\.0\.0"$' "$fixture"
  rg -q '^\[range_caps\]$' "$fixture"
  rg -q '^\[observation_caps\]$' "$fixture"
done

cargo test -p gbf-policy compile_profile_spec_fixtures_round_trip
cargo test -p gbf-policy compile_profile_spec_v2

#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

cargo test -p gbf-policy compile_profile_spec_v1
cargo test -p gbf-policy compile_profile_spec_mismatched_schema_version_rejected_with_typed_error

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

cargo test -p gbf-codegen --lib s3::infer_ir::tests::build_infer_ir_core -- --test-threads=1
cargo test -p gbf-codegen --lib s3::infer_ir::tests::run_stage3 -- --test-threads=1

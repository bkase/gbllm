#!/usr/bin/env bash
set -euo pipefail

cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" --test oracle_artifact_real_vs_fallback_parity_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real" --test oracle_artifact_weight_resolution_log_s3

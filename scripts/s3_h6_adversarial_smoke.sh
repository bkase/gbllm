#!/usr/bin/env bash
set -euo pipefail

cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real,s3-oracle-adversarial" --test oracle_quantspec_s3
cargo test -p gbf-experiments --features "s3,s3-phase-d,s3-oracle-real,s3-oracle-adversarial" --test oracle_adversarial_fixture_separating_s3

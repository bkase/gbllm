#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cargo test -p gbf-experiments --no-default-features --features s7 --test matched_bytes_formula

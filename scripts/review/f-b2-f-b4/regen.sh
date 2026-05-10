#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

cargo run -q -p gbf-report --bin f_b2_review_artifacts -- \
  regen docs/review/f-b2-f-b4/artifacts

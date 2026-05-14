#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

packet_dir="${1:-docs/review/f-b5}"
case "$packet_dir" in
  /*) ;;
  *) packet_dir="$(pwd)/$packet_dir" ;;
esac
mkdir -p "$packet_dir/golden"

GBF_REVIEW_F_B5_PACKET_DIR="$packet_dir" \
  cargo test -q -p gbf-codegen --features semantic_equivalence_check --lib \
    review_f_b5_export_packet_goldens -- --test-threads=1

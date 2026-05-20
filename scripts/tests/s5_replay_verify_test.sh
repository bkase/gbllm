#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$(mktemp)"
trap 'rm -f "$out"' EXIT

"${repo_root}/scripts/s5_replay_verify.sh" >"$out"
grep -q "s5 replay fixture verification passed SUBSTRATE_ONLY" "$out"
grep -q "live producer replay is owned by bd-q3zo" "$out"

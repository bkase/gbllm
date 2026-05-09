#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if ! git check-ignore --no-index -q experiments/S1/checkpoints/seed-0/final.safetensors; then
  echo "expected experiments/S1 result artifacts to be ignored before prereg commit" >&2
  exit 1
fi

if git check-ignore -q docs/experiments/S1-report.md; then
  echo "docs/experiments/S1-report.md must remain tracked/stageable" >&2
  exit 1
fi

echo "[S1 GITIGNORE TEST] generated result artifacts ignored; prereg report remains stageable"

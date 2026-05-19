#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

cargo run -q -p gbf-policy --example dump_registries

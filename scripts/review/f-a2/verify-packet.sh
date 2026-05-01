#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$repo_root"

cargo fmt --check --all
cargo test -p gbf-hw
cargo test -p gbf-asm
cargo clippy -p gbf-hw -- -D warnings
scripts/lints/no-hw-literal-redeclarations.py
scripts/review/f-a2/verify-pan-docs-fragments.py
cargo tree -p gbf-hw >/dev/null
git diff --check

printf 'F-A2 review packet verified.\n'

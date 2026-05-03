#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../../.."

required=(
  docs/review/f-a4/README.md
  docs/review/f-a4/claim-to-gate.md
  docs/review/f-a4/architecture.md
  docs/review/f-a4/correctness-dossier.md
  docs/review/f-a4/test-coverage.md
  docs/review/f-a4/diagrams.md
  docs/review/f-a4/reproducibility.md
  docs/review/f-a4/benchmark.md
  docs/review/f-a4/dependency-report.md
  docs/review/f-a4/error-shape-report.md
  docs/review/f-a4/artifact-report.md
  docs/review/f-a4/api-change-guide.md
  docs/review/f-a4/known-debt.md
  docs/review/f-a4/reviewer-checklist.md
  docs/review/f-a4/artifacts/acquire_release.bin
  docs/review/f-a4/artifacts/acquire_release.lst
  docs/review/f-a4/artifacts/banking-flow.mmd
  docs/review/f-a4/artifacts/banking-flow.svg
)

for file in "${required[@]}"; do
  test -s "$file"
done

cargo fmt --check --all
cargo test -p gbf-runtime -- banking -- --nocapture
cargo test -p gbf-asm -- builder -- --nocapture
cargo test -p gbf-asm -- lowering -- --nocapture
demo_bytes="$(cargo run -p gbf-runtime --example banking_demo | tail -n 1)"
packet_bytes="$(sed 's/[[:space:]]*$//' docs/review/f-a4/artifacts/acquire_release.bin)"
test "$demo_bytes" = "$packet_bytes"
grep -Fq 'ld   ($2000), a' docs/review/f-a4/artifacts/acquire_release.lst
grep -q "flowchart" docs/review/f-a4/artifacts/banking-flow.mmd
grep -q "<svg" docs/review/f-a4/artifacts/banking-flow.svg

if rg -n "unsafe" gbf-runtime/src/banking.rs; then
  echo "unexpected unsafe in gbf-runtime/src/banking.rs" >&2
  exit 1
fi

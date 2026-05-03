# F-A4 Test Coverage

Focused gates run during implementation:

```text
cargo test -p gbf-runtime -- banking -- --nocapture
52 passed

cargo test -p gbf-asm -- builder -- --nocapture
12 passed

cargo test -p gbf-asm -- lowering -- --nocapture
4 passed
```

Coverage includes type invariants, HRAM offsets, byte-golden emit helpers,
cycle counts, local interrupt/residency checks, lowering disposition behavior,
durable lifetime preservation/rejection, stale cross-builder guard rejection,
full typed lease-to-lowered-bytes round trip, compare-and-trap assert lowering,
and provenance audit failure.

Full workspace gates are recorded in the PR body after final pre-commit runs.

# Chunk 2 E2E Review Packet

This packet covers the current executable chunk-closure harness for F-B3/F-B5.

## Artifacts

- `scripts/e2e/chunk2_pipeline.sh`
- `scripts/e2e/chunk2_reject_classes.sh`
- `scripts/e2e/lib/chunk2_logging.sh`
- `docs/review/chunk2/logging-schema.md`
- `docs/review/chunk2/golden/manifest.json`

## Current Coverage

- Passing QuantGraph fixtures are verified through `scripts/review/f-b3/verify.sh`
  and `scripts/e2e/quant_graph_fixtures.sh`.
- Passing InferIR fixtures, Stage 3 cache replay, and audit-parent rewrap are
  verified through `scripts/review/f-b5/verify.sh` and `scripts/e2e/stage3.sh`.
- Reject taxonomy coverage checks 36 QuantGraph and 36 InferIR fixture
  expectations and runs the available focused cargo gates.
- Cache replay, audit rewrap, BitExact equivalence, and reject observations are
  logged as delegated executable gate evidence. The chunk scripts do not emit
  per-fixture cache/audit/reject observations derived only from golden metadata.

## Current Runner Boundary

The current executable contract is split across the shipped feature-level
runners. The chunk harness therefore closes over concrete exported F-B3/F-B5
goldens and focused driver gates instead of inventing a second fixture pipeline
or claiming a chunk-owned report bundle. The only chunk-owned file under
`docs/review/chunk2/golden/` is `manifest.json`; it machine-records the delegated
golden and gate boundary. If a public Stage 0 -> Stage 3 fixture runner is added
later, this packet should grow to compare its emitted reports against a real
chunk-owned golden bundle.

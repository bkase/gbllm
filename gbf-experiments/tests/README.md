# gbf-experiments Test Matrix

Shared test scaffolding lives in `tests/common/mod.rs` and is intended to be
included from integration tests with `mod common;`.

## Core Gates

- `cargo test -p gbf-experiments`
- `cargo test -p gbf-experiments --test trybuild`
- `cargo test -p gbf-experiments --doc`

## S1 Test Categories

- `cargo test -p gbf-experiments --features falsify --test falsification`
- `cargo test -p gbf-experiments --test oracle`
- `cargo test -p gbf-experiments --test canonical_json`
- `cargo test -p gbf-experiments --test integration`
- `cargo test -p gbf-experiments --test e2e`
- `cargo bench -p gbf-experiments`

Some category tests are owned by later F-S1 beads. Until those files exist, the
commands document the expected matrix rather than a mandatory present-day gate.

## Shared Helpers

- `fixtures`: deterministic tiny corpus, hand-counted n-gram counts, and
  placeholder probability providers.
- `injectable_rng`: `ScriptedRng`, which exhausts by panicking instead of
  silently falling back to nondeterminism.
- `assertions`: canonical JSON byte equality, self-hash exclusion checks,
  deterministic-field rejection, and a fixture-local canonical tensor payload
  hashing helper. The fixture-local hash only proves shared test invariants; it
  is not the production `gbf_artifact::tensor::canonical_tensor_payload_hash`
  contract.
- `strategies`: proptest seeds, byte sequences, canonical JSON values, and
  sorted-name canonical tensor sets.
- `tracing_capture`: minimal event structs and order/field assertions for
  structured-log tests.
- `tempdir`: fresh run directories and a serialized process-env isolation guard.

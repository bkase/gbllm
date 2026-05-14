# Chunk 2 E2E Logging Schema

Schema id: `chunk2.e2e.logging.v1`

The chunk-closure scripts emit newline-delimited JSON through
`scripts/e2e/lib/chunk2_logging.sh`. Every record includes:

| Field | Meaning |
| --- | --- |
| `ts` | UTC emission time. |
| `event` | Closed taxonomy event name. |
| `fixture` | Fixture slug or `chunk2` for aggregate gates. |

## Pipeline Events

| Event | Required fields |
| --- | --- |
| `chunk2.pipeline.start` | `fixture`, `profile`, `run_index` |
| `chunk2.pipeline.stage.start` | `fixture`, `stage` |
| `chunk2.pipeline.stage.complete` | `fixture`, `stage`, `report_self_hash` |
| `chunk2.pipeline.gate.start` | `fixture`, `gate` |
| `chunk2.pipeline.gate.complete` | `fixture`, `gate`, `status` |
| `chunk2.pipeline.gate.skipped` | `fixture`, `gate`, `reason` |
| `chunk2.pipeline.debug` | `fixture`, `level`, `detail` |
| `chunk2.pipeline.golden.match` | `fixture`, `stage`, `golden_hash` |
| `chunk2.pipeline.golden.diff` | `fixture`, `stage`, `expected`, `observed`, `diff_path` |
| `chunk2.pipeline.complete` | `fixture`, `total_ms`, `all_stages_passed`, `status` |

Current implementation note: the chunk harness validates concrete exported
F-B3/F-B5 report goldens and executable driver/cache/semantic-equivalence
gates. It does not own five chunk-local per-fixture report files; the
chunk-local golden bundle contains only `docs/review/chunk2/golden/manifest.json`
and delegates report artifacts to `docs/review/f-b3/golden/` and
`docs/review/f-b5/golden/`. Cache replay, audit rewrap, and BitExact claims are
reported only as `chunk2.pipeline.gate.*` events for focused executable gates,
not as per-fixture cache or audit facts.

## Reject Events

| Event | Required fields |
| --- | --- |
| `chunk2.reject.gate.start` | `fixture`, `gate` |
| `chunk2.reject.gate.complete` | `fixture`, `gate`, `status` |
| `chunk2.reject.debug` | `fixture`, `level`, `detail` |
| `chunk2.reject.expected_class` | `fixture`, `family`, `expected_class`, `expected_source` |
| `chunk2.reject.diff` | `fixture`, `expected_class`, `observed_class`, `severity`, `message_excerpt` |
| `chunk2.reject.complete` | `fixture`, `total_ms`, `status`, `expected_class_count`, `executable_gates_passed` |

The reject script first runs the executable focused cargo gates for QuantGraph
and InferIR reject fixtures. It records the 36 + 36 typed expected diagnostic
classes from fixture metadata as expected-class coverage only; observed reject
behavior is evidenced by the `chunk2.reject.gate.complete` records.

## Semantic Equivalence Events

The full `chunk2.semeq.*` event family is reserved for the future public
cross-stage fixture runner. Today, BitExact coverage is delegated to the
feature-enabled F-B5/Stage3 gates:

- `scripts/review/f-b5/verify.sh` with `GBF_REVIEW_F_B5_RUN_CARGO=1`
- `scripts/e2e/stage3.sh`
- `cargo test -p gbf-codegen --features semantic_equivalence_check --lib fixture_infer_ir_fixture_semantic_equivalence_bit_exact`

Set `CHUNK2_E2E_VERBOSE=1` to emit one `*.debug` record at script startup.

No dashboard or production subscriber adoption is claimed by this schema; it is
a review/e2e artifact contract.

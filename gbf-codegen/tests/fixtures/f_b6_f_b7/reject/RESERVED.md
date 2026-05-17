Reserved or delegated diagnostic codes excluded from executable reject fixtures:

- OBSERVATION-OPTIONAL-CHECKPOINT-NOT-FEASIBLE: reserved by the RFC for future optional checkpoint policy
- OBSERVATION-WORKLOAD-CHECKPOINT-NOT-FEASIBLE: covered as a Stage 4 input feasibility guard by a later driver bead
- OBSERVATION-CHECKPOINT-NOT-IN-SCHEMA: covered as a Stage 4 schema-input guard by a later driver bead
- OBSERVATION-METRIC-ID-UNKNOWN: reserved in v1 because metrics are registry-owned and Stage 4 emits metric-source reserved diagnostics instead
- OBSERVATION-LOCKED-KNOB-DRIFT: input/projection validation guard; no Stage 4 construction fixture
- OBSERVATION-COMPARE-DOMAIN-MISMATCH: landed workload-vs-policy input validation before Stage 4 construction
- OBSERVATION-WORKLOAD-DETERMINISM-MISMATCH: landed workload-vs-manifest input validation before Stage 4 construction
- OBSERVATION-POLICY-WORKLOAD-DETERMINISM-MISMATCH: landed policy-vs-workload input validation before Stage 4 construction
- RANGE-LOCKED-KNOB-DRIFT: input/projection validation guard; no Stage 5 construction fixture
- RANGE-CERT-MALFORMED: owned by gbf-verify/tampered certificate validation rather than Stage 5 construction
- RANGE-CHUNK-LEN-ZERO: v1 Stage 5 candidate generation never constructs zero chunk_len; malformed external plan validation is a later schema/verifier owner
- RANGE-TILE-LEN-ZERO: v1 Stage 5 candidate generation never constructs zero tile_len; malformed external plan validation is a later schema/verifier owner
- RANGE-BITEXACT-MID-REDUCTION-SATURATION-FORBIDDEN: enforced as BitExact RenormLoop reservation in Stage 5; no separate mid-reduction saturation producer exists in v1
- RANGE-RENORM-STRATEGY-UNSUPPORTED-V1: range profile parsing rejects unsupported renorm strategy before Stage 5 construction
- RANGE-CAPS-INVALID: compile profile validation rejects invalid range caps before Stage 5 construction
- RANGE-INTEGER-OVERFLOW-DURING-PROOF: v1 public maxima are u32/u64-bounded and construction uses Failed proof evidence for arithmetic overflow rather than a Stage 5 diagnostic
- RANGE-TILE-LEN-EXCEEDS-U16: v1 non-BitExact RenormLoop supports term_count above u16 via tile_count; malformed proof-state validation is verifier-owned

These are reserved by the RFC, enforced before Stage 4/Stage 5 construction,
or delegated to later executable driver/verifier beads named by the closure
packet metadata. They must not be counted as active Stage 4/Stage 5 reject
fixtures by the bd-3ocg diagnostic algebra closure.

Stage 5 wire-shape note: active `RANGE-*` construction diagnostics remain
encoded as `ValidationCode::ReportSemanticInvariantViolated` with the RFC
code in `evidence.reference`. The fixture `expected_diag.json` files pin this
with `wire_code_kind = "ReportSemanticInvariantViolated"` and
`rfc_code_location = "evidence.reference"` rather than pretending the flat RFC
code is the public `code.kind`.

Schema-version decision: the `ValidationOrigin::{ObservationPlanConstruction,
RangePlanConstruction}` additions are treated as additive internal enrichment
for the still-in-flight F-B6/F-B7 report schemas. No schema-id bump is required
inside this RFC train; strict downstream consumers are not declared stable until
the parent F-B6/F-B7 feature beads close.

# F-A3 Reviewer Checklist

- [ ] Every `repr(C)` type in `layout-report.json` matches the source ABI definitions and targeted layout tests.
- [ ] No fixed layout stores Rust data enums directly where unknown discriminants could be UB.
- [ ] All magic/reserved-byte validators reject malformed input.
- [ ] `BuildIdentityBlock` carries the four lineage hashes plus tail/schema fields.
- [ ] `LivenessCounters` is non-optional in `InferenceStateHeader` and uses saturating arithmetic.
- [ ] `FaultCode::ALL` is complete and `classify_fault` is total.
- [ ] `SemanticCheckpointSchema` rejects duplicate ids, compact zero, and schema version zero.
- [ ] `TraceEvent` is exactly 32 bytes and trace budget zero/nonzero rules are tested.
- [ ] Host-only Vec/Cow/BTreeMap types are cfg-gated behind `host`/`alloc`.
- [ ] `gbf-abi` has no upward dependency on runtime/codegen/asm/artifact/hw.
- [ ] `#![forbid(unsafe_code)]` is present in `gbf-abi/src/lib.rs`.
- [ ] The full feature matrix and packet staleness checks have been run.

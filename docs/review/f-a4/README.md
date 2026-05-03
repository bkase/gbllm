# F-A4 Review Packet

Feature: `bd-1sv` (`gbf-runtime::banking` BankLease/BankGuard ABI)

This packet reviews the single F-A4 PR that implements:

- runtime-side `ValidatedBankLeaseSpec`, `BankGuard`, return-state, and ABI errors;
- the four-byte HRAM banking shadow at `$FF80..=$FF83`;
- MBC5 acquire/release emitters with explicit `InterruptPolicy`;
- local ISR-safety and fixed-residency declarations;
- production banking `PreLayoutOp` lowering plus `LoweringDisposition` composite dispatch.

Start review in this order:

1. `gbf-runtime/src/banking.rs`
2. `gbf-asm/src/section.rs`
3. `gbf-asm/src/builder.rs`
4. `gbf-asm/src/lowering.rs`
5. `gbf-runtime/examples/banking_demo.rs`

Required gates:

```bash
cargo test -p gbf-runtime -- banking -- --nocapture
cargo test -p gbf-asm -- builder -- --nocapture
cargo test -p gbf-asm -- lowering -- --nocapture
cargo run -p gbf-runtime --example banking_demo
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
scripts/review/f-a4/verify-packet.sh
```

F-A4 intentionally does not implement whole-program `ReachabilityValidation`,
F-A5 boot wiring, or live emulator validation. The local side is guarded by
typed privilege, residency, interrupt-safety, and provenance checks.

Packet artifacts:

- `claim-to-gate.md`
- `architecture.md`
- `correctness-dossier.md`
- `test-coverage.md`
- `diagrams.md`
- `reproducibility.md`
- `benchmark.md`
- `dependency-report.md`
- `error-shape-report.md`
- `artifact-report.md`
- `artifacts/acquire_release.bin`
- `artifacts/acquire_release.lst`
- `artifacts/banking-flow.mmd`
- `artifacts/banking-flow.svg`
- `known-debt.md`

# F-A4 Artifact Report

F-A4's durable code artifacts are the serialized asm wire records. The review
packet also checks in reproducible demonstration artifacts:

- `artifacts/acquire_release.bin`: the banking demo byte stream.
- `artifacts/acquire_release.lst`: a line-oriented listing for those bytes.
- `artifacts/banking-flow.mmd`: Mermaid source for the F-A4 flow.
- `artifacts/banking-flow.svg`: rendered SVG for the same flow.

Durable wire changes:

- `BankLeaseSpec` carries `LeaseId`, `LeaseGeneration`, `MbcBankClass`, bank
  number, and `LeaseLifetime`.
- `PreLayoutOp::BankRelease` carries `return_to: BankReleaseDisposition`.
- `LoweringDisposition` preserves `Lowered`, `NotOwned`, and terminal `Error`
  outcomes for the future composite lowerer.

The `gbf-runtime/examples/banking_demo.rs` example materializes the byte stream
for a ROM bank-3 acquire followed by release to bank 1, and
`scripts/review/f-a4/verify-packet.sh` compares that output against
`artifacts/acquire_release.bin`.

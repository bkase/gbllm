# F-A4 Error Shape Report

Public fallible APIs return typed errors:

- `ValidatedBankLeaseSpec` constructors return `BankAbiViolation`.
- Runtime emit helpers return `BankingEmitError`.
- Local interrupt annotations return `InterruptSafetyError`.
- Builder lifecycle failures are preserved as `BuilderError` before being
  wrapped for runtime emit APIs.

Important rejection paths are test-gated: bank zero, out-of-range banks,
stale/double guard release, cross-builder stale guards, non-privileged
sections, switchable residency, unsupported restoration lifetimes, forged
keep-current release, and forged/non-banking MBC provenance.


# F-A4 API Change Guide

New runtime public surface:

- `ValidatedBankLeaseSpec`
- `LeaseLifetime`, `LeaseGeneration` (re-exported from the asm wire shape)
- `BankGuard`
- `BankLease`
- `ReturnState`, `ReturnRomBank`, `ReturnSramState`, `KeepCurrentProof`
- `SectionResidency`
- `BankAbiViolation`, `BankingEmitError`
- `InterruptSafetyKind`, `InterruptSafety`, `InterruptSafetyTable`
- `lease_rom_switchable`, `lease_sram`, `release_bank`
- `lower_banking_shadow_zero_init`
- `BankingPreLayoutLowering`
- `BankingAssertBankPolicy`
- `mbc_write_provenance_audit` (takes the emitting
  `BankingPreLayoutLowering` as audit authority)

Changed asm surface:

- `BankLeaseSpec` now carries `LeaseGeneration` and `LeaseLifetime` in the
  durable pre-layout record.
- `PreLayoutOp::BankRelease` now carries `return_to: BankReleaseDisposition`.
- `Builder::bank_release` remains as a compatibility helper and defaults to
  `RomBank1`, not keep-current.
- `Builder::bank_release_to` / `try_bank_release_to` expose explicit return
  disposition.
- `Builder::try_finish` returns typed unreleased-lease errors.
- `LoweringDisposition`, `DispositionPreLayoutOpLowering`, and
  `lower_pre_layout_ops_with_disposition` support composite lowerers.

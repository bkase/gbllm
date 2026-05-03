# F-A4 Reviewer Checklist

- [ ] No raw byte emission is used for MBC writes.
- [ ] MBC register literals appear only through `gbf-hw::mbc5` constants in runtime banking.
- [ ] HRAM banking shadow is exactly four bytes: `$FF80..=$FF83`.
- [ ] `BankGuard` is non-borrowing and non-`Copy`.
- [ ] Guard release checks originating section plus active lease generation.
- [ ] `Builder::try_finish` rejects active leases.
- [ ] `Builder::finish` remains a compatibility panic wrapper over `try_finish`.
- [ ] `ReturnState::KeepCurrent` cannot be obtained from asm serde defaults.
- [ ] Lowering revalidates serialized `BankLeaseSpec` and release dispositions.
- [ ] Lowering preserves `LeaseLifetime` and rejects `ResumeWindow`/`Token`.
- [ ] `InterruptPolicy::Enabled` cannot acquire ROM/SRAM leases.
- [ ] Switchable-resident sections cannot inline banking primitives.
- [ ] `Yield`/`TraceProbe` are `NotOwned` by banking.
- [ ] A banking owned error stops composite lowering.
- [ ] Opt-in `AssertBank` compare-and-trap emits the expected HRAM/CP/JR/RST sequence.
- [ ] Focused and workspace gates in `README.md` pass.

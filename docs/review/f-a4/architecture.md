# F-A4 Architecture

The public API is lease-shaped:

```text
ValidatedBankLeaseSpec -> lease_rom_switchable / lease_sram -> BankGuard
BankGuard + ReturnState -> release_bank -> PreLayoutOp::BankRelease
```

`gbf-asm` owns the durable wire shape: `BankLeaseSpec`, `LeaseId`,
`LeaseGeneration`, `LeaseLifetime`, `MbcBankClass`, and
`BankReleaseDisposition`. `gbf-runtime::banking` owns runtime policy: bank-0
rejection, `BankGuard`, unforgeable `KeepCurrentProof`, HRAM shadow writes,
interrupt policy, and the production lowering.

`ReturnState::KeepCurrent` is not serialized directly. Runtime converts it into
`BankReleaseDisposition::KeepCurrent` only through `release_bank`, but the
production lowerer rejects keep-current until a scheduler-owned proof producer
lands. No public default or serde path can create the proof.

Banking writes are emitted inline only for privileged fixed-resident sections
(`Bank0Nucleus` or explicit HRAM residency). `CommonBank` and `ExpertBank`
therefore fail local F-A4 checks; later code that needs normal payload banking
must call fixed Bank0 helpers or wait for the F-A5/F-B13 owner path.

`AssertBank` lowers to a label in the default profile. The opt-in
`BankingAssertBankPolicy::CompareAndTrap` profile reads the HRAM shadow, emits
`CP`/`JR NZ`, and traps through `RST $38`.

MBC-write audit is tied to the `BankingPreLayoutLowering` instance that
materialized the bytes. Forged public provenance strings are rejected because
they lack that instance's private lowering token.

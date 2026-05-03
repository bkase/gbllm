# F-A4 Correctness Dossier

The MBC5 write path is:

```text
DI (SCS only)
LD A, value
LD ($MBC_REGISTER), A
LDH ($BANKING_SHADOW), A
EI (SCS only)
```

ROM acquire always writes BANK1 then BANK2, then the two HRAM shadow bytes.
SRAM acquire requires prior RAMG enable in `BankingLoweringState`. SRAM disable
writes RAMG `$00` and clears the SRAM shadow state.

Local correctness gates:

- `SectionPrivilege::privileged()` is required for concrete MBC stores.
- `check_lease_emission_legal` rejects interrupt handlers, normal sections,
  and non-fixed residency.
- `mbc_write_provenance_audit` rejects MBC writes without
  provenance from the `BankingPreLayoutLowering` instance that emitted them.
- `BankReleaseDisposition` is revalidated during lowering, so serde/public wire
  inputs do not bypass runtime constructors.

Deferred proof:

Whole-program ISR reachability remains Epic B `ReachabilityValidation`. F-A4
exports `InterruptSafetyTable` as the declaration substrate for that pass.

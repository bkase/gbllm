# RFC F-A4: `gbf-runtime::banking` — the `BankLease`/`BankGuard` ABI

| Field          | Value                                                                                                          |
|----------------|----------------------------------------------------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A4)                                                                               |
| Status         | Draft                                                                                                          |
| Feature bead   | `bd-1sv`                                                                                                       |
| Open tasks     | T-A4.1 (`bd-371`, types), T-A4.2 (`bd-2sv`, HRAM shadow), T-A4.3 (`bd-19j`, acquire/release), T-A4.4 (`bd-f5y`, IRQ scaffolding) |
| Closed tasks   | (none — F-A4 has no closed tasks; F-A1 T-A1.1–T-A1.4 closed tasks are dependencies)                            |
| Plan reference | `history/planv0.md` line 121 (ISR residency rule), line 1958 ("locks up after twenty minutes" failure mode), line 1886 (centralized MBC + `BankLease`/`BankGuard` ABI), line 2091 (MBC5 + `$0A` RAM enable), line 2936 (engineering rule 16, slice `InterruptPolicy`), line 1742 (`ResourceLeaseKind`), line 1791 (`InterruptPolicy`); rule 15 ("compiler-generated code may not perform raw MBC writes outside the `BankLease`/`BankGuard` ABI") is the §"Engineering rules" rule, currently emitted at the same offset |
| Glossary       | `history/glossary.md` (banking, lease, shadow, ISR-reachable)                                                  |
| Constitution   | `CONSTITUTION.md` §I.1 (correctness by construction), §III (shifting left), §IV (reproducibility), §V.3 (loud failure) |

## 0. TL;DR

`gbf-runtime::banking` is the *single, type-checked path* by which an LR35902 `Section`
ever rewrites an MBC5 register. F-A1 already gave us the typed eDSL: `Builder`,
`MachineEffect::StoreToMbcRegister { reg: MbcRegisterClass }` (privilege class
`Privileged`), `SystemCallKind::{BankLease, BankRelease}`, and the
`PreLayoutOp::{BankLease, BankRelease, AssertBank}` / `LegalizationOp::FarCall`
seams from §4 of the F-A1 RFC. The current F-A1 builder exposes `bank_lease(spec)`
and `bank_release(lease_id)` as plain `&mut self` methods returning `()` /
`Result<(), BuilderError>` — there is no `BankGuard` type yet, and the active-lease
table (`Builder::active_leases: BTreeSet<LeaseId>`) only catches duplicate
acquires and unknown releases; `Builder::finish() -> Section` does not error on
unreleased leases. F-A4 fills in the runtime side: the lease type backbone
(including `BankGuard`), the HRAM shadow layout, the actual acquire/release
builders that emit typed `Instr`s through `gbf-asm`, and the per-section
interrupt-safety annotations that `ReachabilityValidation` (Epic B Stage 12) will
later prove against. F-A4 also extends `Builder::finish` (or wraps it) so an
unreleased lease becomes a typed `BuilderError::UnreleasedBankGuard`.

The shape is small and deliberately boring. There are exactly three places in
the entire system where MBC5 register addresses (`$0000–$1FFF`, `$2000–$2FFF`,
`$3000–$3FFF`, `$4000–$5FFF`) may appear in executable-code construction:

```
gbf-hw::mbc5            (constants only — addresses, masks, $0A token)
gbf-runtime::banking    (the four emit_* helpers in this RFC)
gbf-asm::effect         (classifier — already in tree, addresses are read-only)
```

`gbf-runtime::boot` may *call* banking helpers (e.g. for cold-boot bank-1
acquire and HRAM shadow zero-init), but it must not duplicate MBC5 register
literals.

Everything else — every kernel, every expert payload, every codegen-emitted
section — speaks the `BankLease` / `BankGuard` ABI. The four MBC-writing
helpers are the only audited lowering path for MBC5 writes. They are not
part of the general authoring API; callers acquire and release banks
through `BankLease` / `BankGuard`, and only the banking lowerer may call
the raw MBC-write helpers.

F-A4 must enforce this structurally. A `Privileged` section outside
`gbf-runtime::banking` must not be able to call a public helper that emits
`LD ($2000), A` directly, because that bypasses lease tracking. The raw
emit functions are therefore `pub(crate)` and exposed only through the
`BankingPreLayoutLowering` impl; the user-facing API is lease-shaped
(`lease_rom_switchable`, `lease_sram`, `release_bank`).

The seven most load-bearing decisions in this RFC are:

1. **`BankGuard` is host-side linear bookkeeping, not target-side RAII.**
   ROM has no destructors. `BankGuard` lives in the compiler-side
   authoring layer and guarantees that every `BankLease` site is either
   explicitly released or reported by `Builder::finish()`. The authoritative
   pending-lease table is owned by `Builder`; `Drop` may be used only for
   debug logging or a debug assertion. The typed error
   (`BuilderError::UnreleasedBankGuard`) comes from the builder's pending
   table, not from `Drop` (which cannot return `Result`). `BankGuard` does
   *not* borrow `&'a BankLease` from the builder; it carries a
   `LeaseId + LeaseGeneration` token, so the builder remains freely
   `&mut`-usable while a guard is alive.
2. **HRAM shadow is authoritative for software.** MBC5 registers are
   write-only; software cannot read them back. The HRAM shadow at
   `$FF80..=$FF83` (4 banking-owned bytes) is the single source of truth
   for "what bank is currently live." Any code that wants to know reads
   the shadow; any code that wants to *change* the bank goes through the
   acquire lowering, which writes both the register and the shadow in one
   short critical section. **Shadow writes to the banking-owned bytes are
   privileged and `pub(crate)`-only**: arbitrary `Privileged` code cannot
   lie to the shadow.
3. **Banking primitives must execute from fixed residency (Bank 0 fixed
   ROM `$0000..=$3FFF` or HRAM).** A bank-switch sequence cannot run from
   `$4000..=$7FFF`: the `LD ($2000), A` write would change the memory
   backing the remaining instructions in flight. `check_lease_emission_legal`
   takes a `SectionResidency` argument and rejects switchable residency.
4. **The acquire/release primitives compile to `PrivilegeClass::Privileged`
   AsmIR — no exceptions, plus a second guard.** `Builder::validate_effect`
   rejects any `StoreToMbcRegister` from a `Normal` section. F-A4 adds a
   second guard: the closure-time effect/provenance audit verifies that
   *every* `StoreToMbcRegister` in the final AsmIR has provenance from
   `gbf-runtime::banking` (a grep is supplemental only). Privilege is
   necessary but not sufficient; banking writes also require fixed
   residency and non-ISR-reachability.
5. **`InterruptSafetyKind` is a section-level annotation, not an instruction
   property.** Local checks (`check_lease_emission_legal`) catch the easy
   cases — "an `InterruptHandler` section called the banking lowerer",
   "a `Normal` section called the banking lowerer", "a banking primitive
   was emitted into a switchable-residency section" — at builder time. The
   hard cases (ISR transitively reaches a privileged banking helper through
   a long call chain) are declared here and *proved* later by Epic B's
   `ReachabilityValidation`. F-A4 provides the declaration substrate, not
   the global proof.
6. **`InterruptPolicy::ShortCriticalSection` is non-nestable and has an
   IME-state precondition.** The Game Boy CPU does not provide a
   read-IME instruction. A `DI ... EI` bracket cannot restore the prior
   IME state — it unconditionally re-enables interrupts via the trailing
   `EI`. SCS therefore requires the caller to know IME was enabled on
   entry; callers already inside a disabled region must pass
   `InterruptPolicy::Disabled` instead.
7. **F-A4 implements the banking portion of `PreLayoutOpLowering`.**
   `gbf-asm::lowering::StubPreLayoutOpLowering` is currently
   unconditionally available (the asm crate declares a `stub-runtime`
   Cargo feature, but the stub itself is not gated behind it today)
   and is used only by `tiny_rom`-style tests in `gbf-asm`. The
   production lowering — the one Epic B's compiler backend eventually
   plugs into — lives here. The composite lowerer in F-A5 dispatches by
   op kind via a `LoweringDisposition::{Lowered, NotOwned, Error}` shape
   so a banking op that fails validation is *not* swallowed by a later
   lowerer. F-A4 introduces `LoweringDisposition` (the asm crate's
   current `PreLayoutOpLowering::lower` returns `Result<LoweredFragment,
   LoweringError>`, which the F-A4 PR extends or wraps).

The new surface adds roughly 600–700 LOC of production code plus about
1.5 KLOC of table-driven tests, golden AsmIR fixtures, and IRQ-safety
property tests. The *public* surface is intentionally smaller than the
implementation surface: raw emit helpers, mutable shadow helpers, and
lowering-only state types are `pub(crate)` or behind an internal test
feature. Public callers see only validated lease construction, guard /
release APIs, shadow read APIs, and the `BankingPreLayoutLowering`
integration point. There is no new emulator or hardware dependency.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A type backbone for the lease ABI: `MbcBankClass`, `LeaseLifetime`,
  `ValidatedBankLeaseSpec` (runtime-side validated request type),
  `BankLease`, `BankGuard`, `LeaseId`, `LeaseGeneration`, `ReturnState`
  (with `ReturnRomBank`, `ReturnSramState`, and an unforgeable
  `KeepCurrentProof`), `BankAbiViolation`, `BankingEmitError`,
  `SectionResidency`. All in `gbf-runtime::banking`.
- A reserved HRAM region (`$FF80..=$FF83`, 4 banking-owned bytes) for
  shadow registers, with typed offset constants and `pub(crate)` writer
  helpers. `pub` reader helpers for shadow inspection from non-banking
  code.
- Four `pub(crate)` acquire/release lowering primitives that emit AsmIR
  through `gbf-asm::Builder`: `lower_enable_sram`, `lower_disable_sram`,
  `lower_acquire_rom_bank(bank)`, `lower_acquire_sram_bank(bank)`, plus
  `lower_release(lease, return_state)`. The user-facing `pub` API is
  lease-shaped: `lease_rom_switchable`, `lease_sram`, `release_bank`.
- Section-level interrupt-safety annotations (`InterruptSafetyKind`) and
  the local check (`check_lease_emission_legal`, taking a
  `SectionResidency`) that catches the cases provable per-section.
- A production implementation of `gbf-asm::lowering::PreLayoutOpLowering`
  for `BankLease` / `BankRelease` / `AssertBank`. (`Yield` and `TraceProbe`
  remain owned by `gbf-runtime::scheduler` / `gbf-runtime::trace` and are
  out of scope here — F-A4 only touches banking-related ops.)
- A determinism contract: every emit helper is a byte-stable AsmIR
  emitter. Same inputs, same `Builder`, same bytes.
- An in-tree test harness using a synthetic `Builder` plus the encoder
  (already shipped in F-A1) that asserts exact byte sequences for every
  acquire/release path.
- An effect/provenance audit that walks every emitted `Instr` in the
  workspace, asserts every `StoreToMbcRegister` originated in
  `gbf-runtime::banking`, and asserts the surrounding section is
  `Privileged`, fixed-resident, and not ISR-reachable.

### 1.2 Non-goals (deferred)

- **Whole-program `ReachabilityValidation`.** Epic B Stage 12 sub-pass
  (`bd-18d` and the `gbf-codegen` reachability work). F-A4 produces the
  *declarations* (`InterruptSafetyKind`, lease lifetime, privilege class)
  that the pass consumes; it does not run the global walk.
- **The `Yield` / `TraceProbe` lowerings.** Yield belongs to
  `gbf-runtime::scheduler` (F-A5 / its own follow-up); trace belongs to
  `gbf-runtime::trace`. Both consume `PreLayoutOpLowering` like banking
  does, but their semantics are unrelated to MBC writes.
- **The full Bank-0 nucleus boot sequence.** F-A5 (`bd-2r1`) owns the boot
  vector, IRQ vector dispatch, scheduler skeleton, and HRAM zero-init. F-A4
  provides the HRAM shadow constants and the helper that *zeros the shadow
  region*; F-A5 calls it from `boot_init`.
- **Long critical sections / IRQ-deferral mechanisms.** Default policy is a
  short `DI`/`EI` window around the four-instruction MBC write. Anything
  more elaborate (deferred IRQ queues, nested critical sections, IRQ
  shadowing) is out of scope and left for a measured follow-up if the
  `SchedulerPolicy::max_interrupt_latency` budget proves insufficient.
- **MBC5 reserved register window (`$6000..=$7FFF`).** Already classified by
  `gbf-asm::effect::MbcRegisterClass::Reserved` (the actual variant name; on
  MBC5 this window is a hardware no-op rather than MBC1's mode-select
  register, and the comment in `gbf-asm/src/effect.rs` calls that out
  explicitly). Forbidden in *every* section (including `Privileged`) per
  F-A1's `SectionPrivilege::check_effect` returning
  `PrivilegeViolation::ForbiddenMbcReserved`. F-A4 does not re-introduce it.
- **Rumble bit on MBC5 SRAM bank.** On MBC5 rumble cartridges, bit 3 of the
  RAM bank register (`$4000..=$5FFF`) controls rumble instead of selecting
  an SRAM address line. F-A4 is DMG/MBC5-non-rumble only. The builder must
  reject MBC5+RUMBLE cartridge profiles until a future feature adds an
  explicit rumble-aware bank spec; treating SRAM bank values 8–15 as plain
  bank selects on a rumble cart would silently turn the motor on.
- **CGB / GBC features.** DMG/MBC5 only.
- **Production `gbf-emu`/`gbf-debug` validation.** Booting a synthesized
  Bank-0 nucleus in gameroy and asserting the shadow stays consistent under
  IRQ pressure ships with the `gbf-emu`/`gbf-debug` follow-up. F-A4 ships
  the AsmIR primitives and asserts byte-stable encoding of those
  primitives; live boot tests move to that feature.
- **Stage-cache / content-addressed lowering output.** F-B15 (`bd-1g7k`)
  territory; F-A4's emit helpers are pure functions of their inputs, so
  this is purely additive.

## 2. Background and existing state

### 2.1 What is already in tree

The dependencies F-A4 builds on are all closed:

- **`gbf-asm::isa`** (T-A1.1, `bd-11e` closed): `Instr`, `Reg8`, `Reg16Data`,
  `DirectAddr`, `HighDirectOffset`, `RstVector`, `Cond`, `AluSrc8`, the
  typed operand algebra. F-A4 uses `Instr::Ld8RegFromImm { dst: Reg8::A,
  imm }` (the actual `LD A, imm8` variant — note: the RFC's earlier name
  `Instr::LdAImm` does not exist in tree), `Instr::LdDirectFromA`,
  `Instr::LdHighDirectFromA`, `Instr::LdAFromHighDirect`, `Instr::Di`,
  `Instr::Ei`, `Instr::Halt`, `Instr::Nop`, plus `Instr::XorA { src: ... }`
  (already in tree, so the §5.4 zero-init variant that uses `XOR A` is
  available without new ISA work). If `AssertBank` lowers to a runtime
  compare-and-branch sequence, F-A4 uses `Instr::CpA { src: AluSrc8::Imm(n) }`
  for the immediate compare and `Instr::JrRel { cond: Some(Cond::Nz), off }`
  for the conditional relative jump (the RFC's earlier names `Instr::CpAImm`
  / `Instr::JrCond` do not exist; `CpA`/`JrRel` are the actual variants),
  plus branch/provenance support for a fixed-bank panic thunk. F-A4
  defaults to the *label-only* assertion shape and lists the
  compare-and-branch shape as an opt-in extension; see §8.
- **`gbf-asm::section`** (T-A1.2, `bd-1e2` closed): `Section`, `SectionRole`,
  `SectionPrivilege::{normal(), privileged(), interrupt_handler()}`
  (the third constructor is `interrupt_handler`, not `isr`),
  `BankLeaseSpec`, `LeaseId`, `MbcBankClass` (variants `Rom | Sram` —
  the RFC's bead description `Rom4000 | SramA000` is editorial; the
  in-tree variant names are the simpler `Rom`/`Sram`),
  `PrivilegeClass::{Normal, Privileged, InterruptHandler}`,
  the SoA item arrays (`labels`, `instrs`, `pre_layout_ops`,
  `legalization_ops`, `branches`). `PreLayoutOp::BankRelease`
  currently carries only `{ lease_id: LeaseId }` — see §8 for how F-A4
  extends it.
- **`gbf-asm::builder`** (T-A1.3, `bd-3p1` closed): `Builder` with typed
  emitters (`emit(Instr)`, `try_emit`, `db`/`dw`, `label`, `align`,
  `branch`, `far_call`, `yield_op`, `trace_probe`), plus the lease shaped
  emitters as currently in tree:

  ```rust
  pub fn bank_lease(&mut self, lease: BankLeaseSpec);                        // panics on bad
  pub fn try_bank_lease(&mut self, lease: BankLeaseSpec) -> Result<(), BuilderError>;
  pub fn bank_release(&mut self, lease_id: LeaseId);
  pub fn try_bank_release(&mut self, lease_id: LeaseId) -> Result<(), BuilderError>;
  pub fn assert_bank(&mut self, expected: MbcBankClass, expected_n: u16);
  pub fn finish(self) -> Section;                                            // no Result
  ```

  Note that `bank_lease` does **not** return a `BankGuard` and `bank_release`
  takes a `LeaseId`, not a guard; the `BankGuard` linear token is a new
  type that F-A4 introduces, layered on top of these primitives.
  Internally, `validate_effect` is a private method on `Builder`
  (`fn validate_effect(&self, effect: MachineEffect) -> Result<(), BuilderError>`)
  that delegates to `SectionPrivilege::check_effect`, which already rejects
  `MachineEffect::StoreToMbcRegister` from non-`Privileged` sections and
  rejects any `StoreToMbcRegister { reg: MbcRegisterClass::Reserved }`
  outright via `PrivilegeViolation::ForbiddenMbcReserved`. There is no
  `Builder::validate_effect` exposed publicly; F-A4 must rely on the
  emitter helpers (`try_*`) for typed gating.
- **`gbf-asm::effect`** (T-A1.4, `bd-1bw` closed): `MachineEffect`,
  `MachineEffectKind`, `MbcRegisterClass`, `PrivilegeClass`,
  `classify_effect`, `classify_pre_layout_op`, `classify_legalization_op`,
  `privilege_of`. Pre-classified mappings:
  - `$0000..=$1FFF` → `MbcRegisterClass::RamEnable`
  - `$2000..=$2FFF` → `MbcRegisterClass::RomBankLow`
  - `$3000..=$3FFF` → `MbcRegisterClass::RomBankHigh`
  - `$4000..=$5FFF` → `MbcRegisterClass::SramBank`
  - `$6000..=$7FFF` → `MbcRegisterClass::Reserved` (forbidden everywhere
    via `PrivilegeViolation::ForbiddenMbcReserved`; the variant name is
    `Reserved`, not `ModeSelect`, because the window is a hardware no-op
    on MBC5 — only MBC1 used this band as a mode-select register)
- **`gbf-asm::lowering`** (T-A1.6 part of, F-A1 §4): `PreLayoutOp`,
  `LegalizationOp`, `PreLayoutOpLowering`, `LoweredFragment`,
  `LoweringContext`, `LoweringError`, plus `StubPreLayoutOpLowering` (a
  concrete stub used by in-crate tests; the `stub-runtime` Cargo feature
  is declared but the stub itself is currently unconditionally available).
  The trait signature is:

  ```rust
  pub trait PreLayoutOpLowering {
      fn lower(&self, op: &PreLayoutOp, ctx: &LoweringContext<'_>)
          -> Result<LoweredFragment, LoweringError>;
  }
  ```

  Note the actual return is a plain `Result<LoweredFragment, LoweringError>`,
  *not* the `LoweringDisposition::{Lowered, NotOwned, Error}` shape this RFC
  uses in §8. `LegalizedFragment` is **not** in tree today — the asm crate
  uses `LoweredFragment` and `LoweredSection` only. F-A4 introduces both
  the disposition enum and (if required) a fragment variant for
  legalization-time output as part of its lowering work. F-A4 implements
  `PreLayoutOpLowering` for the banking ops, and (for the composite
  walker that returns `NotOwned` for non-banking ops) also adapts the
  trait shape; see §8.

### 2.2 What is stubbed

`gbf-runtime/src/banking.rs` is a single-line module stub:

```rust
//! Module stub.
```

The same is true for every other `gbf-runtime` module (`boot.rs`,
`scheduler.rs`, `interrupts.rs`, `joypad.rs`, `keyboard.rs`, `panic.rs`,
`persistence.rs`, `text.rs`, `trace.rs`, `harness.rs`, `video_commit.rs`)
— F-A5 owns those.

`gbf-runtime/src/lib.rs` already declares `pub mod banking;` (and the
sibling stub modules). The Cargo.toml already depends on `gbf-asm`,
`gbf-abi`, `gbf-foundation`, and `gbf-hw`.

### 2.3 Downstream pressure on this design

The completed `gbf-runtime::banking` module is consumed by:

- **F-A5** (`bd-2r1`, Bank0 runtime nucleus) — `boot_init` zeros HRAM shadow,
  IRQ vector dispatch must not depend on switchable bank state, scheduler
  saves/restores nothing because lease lifetimes (`Slice` / `ResumeWindow`)
  guarantee yield-safety.
- **F-B13** (`bd-18d`, compiler backend) — `gbf-codegen` calls the
  lease-shaped public API (`lease_rom_switchable` / `release_bank`) via
  `Builder` whenever a far-call thunk needs a `BANK1`/`BANK2` switch. The
  thunk skeleton from F-A1 §7.4 lives in fixed Bank 0 and the
  `BankingPreLayoutLowering` materializes the bank load before the indirect
  jump.
- **F-H1 / F-H2** (`bd-2f32`, `bd-3se9`, kernels) — every kernel that reads
  expert payload data acquires a `BankLease` over the `MbcBankClass::Rom`
  switchable window (i.e., the `$4000..=$7FFF` slot fed by BANK1/BANK2)
  for the payload's bank, holds it for the kernel's slice, and releases
  at the slice boundary. Kernels never touch `$2000`/`$3000` directly.
- **Epic B `ReachabilityValidation`** — walks the `MachineEffect` annotations
  emitted by F-A4's primitives and proves that no ISR-reachable code
  transitively calls them.

### 2.4 Engineering-rule grounding (planv0.md §"Engineering rules")

This RFC threads several rules tightly:

- **Rule 1**: All generated executable code originates from `AsmIR` /
  `Instr` / audited runtime builders. → F-A4 is one of the *audited runtime
  builders* and is constructed entirely from `Instr`. No raw byte pushes.
- **Rule 2**: Only the encoder translates legal instructions to bytes. →
  F-A4 emits `Instr` through `Builder`; the actual `Instr → bytes` step is
  still owned by `gbf-asm::encoder`.
- **Rule 4**: Every hard fit is proven in analysis/layout passes. →
  `BankAbiViolation` is an exhaustive enum; each variant points at the
  invariant it protects.
- **Rule 10 (overridden, stronger)**: No escape hatches. → `Db` / `Dw` are
  not used by F-A4. Every byte the banking module emits flows through a
  typed `Instr` variant.
- **Rule 12**: `unsafe` is forbidden by default. → none required.
- **Rule 15** (load-bearing for this RFC): "All ISR code/data is
  Bank0-resident and bank-agnostic, **proven** by `ReachabilityValidation`,
  not declared; compiler-generated code may not perform raw MBC writes
  outside the `BankLease`/`BankGuard` ABI." → F-A4 *is* the ABI. Rule 15 is
  satisfied for the local case (per-section privilege check) here, and the
  full whole-program proof is delivered by Epic B against the annotations
  defined here.
- **Rule 16**: Every slice has an explicit `InterruptPolicy`. → F-A4's
  acquire lowering honors `InterruptPolicy::ShortCriticalSection`
  (DI/EI bracket; precondition: caller's IME was enabled on entry, and SCS
  is not nestable); `Disabled` skips the bracket because IME is already
  cleared; `Enabled` is rejected for any acquire that crosses a yield
  boundary.

### 2.5 Constitutional grounding (`CONSTITUTION.md`)

- **§I.1 (correctness by construction)** — `BankAbiViolation`,
  `BankingEmitError`, `InterruptSafetyError` are exhaustive enums. Failures
  are typed.
- **§III (shift left)** — Lease lifetime + section privilege + section
  residency catches local misuse at *builder time*; cross-section misuse is
  caught by `ReachabilityValidation` at *codegen time*; nothing waits for
  emulator or hardware.
- **§IV.3 (reproducible builds)** — every emit helper is a pure function;
  identical inputs produce identical AsmIR; the composed pipeline produces
  identical bytes through `gbf-asm::encoder`.
- **§V.3 (silent on success, loud on failure)** — an unreleased
  `BankGuard` is a *typed error* surfaced through `Builder::finish`'s
  pending-lease table check (not from `Drop`, which cannot return
  `Result`).

### 2.6 MBC5 register quick reference (Pan Docs)

The four MBC5 registers F-A4 ever touches:

| Address window     | Register name | Purpose                                         | Width   | Canonical value        |
|--------------------|---------------|-------------------------------------------------|---------|------------------------|
| `$0000..=$1FFF`    | RAMG          | RAM enable: any low-nibble == `$0A` enables     | 8-bit   | `$0A` enable, `$00` disable |
| `$2000..=$2FFF`    | BANK1 (low)   | low 8 bits of ROM bank index                    | 8-bit   | `0x00..=0xFF`         |
| `$3000..=$3FFF`    | BANK2 (high)  | bit 8 of ROM bank index                          | 1-bit   | `0x00..=0x01`         |
| `$4000..=$5FFF`    | RAMB          | SRAM bank index (bit 3 = rumble on rumble carts; F-A4 rejects rumble carts) | 4-bit   | `0x00..=0x0F`         |

Pan Docs notes: any low-nibble `A` value technically enables RAM, but only
`$0A` is portable across cartridge revisions. F-A4 uses exactly `$0A`/`$00`.

F-A4 writes BANK1 (`$2000`) and then BANK2 (`$3000`) by convention and tests
that order in the golden fixtures. The correctness requirement is *not* the
order itself; it is that the whole sequence executes from fixed bank 0 or
HRAM and that no code or data fetch depends on the switchable window until
both writes and the shadow update are complete.

The full ROM-bank index is `bank9 = (bank2 << 8) | bank1`. MBC5 supports
ROM banks `0..=511`, and unlike several older MBCs (notably MBC1), writing
bank 0 selects bank 0 in the `$4000..=$7FFF` window — that is a hardware
capability, not a forbidden value.

F-A4 nevertheless reserves the `MbcBankClass::Rom`-with-bank-0 case as
an **ABI policy**: code that needs fixed bank 0 should use the fixed
`$0000..=$3FFF` residency path rather than acquiring bank 0 through the
switchable window. Therefore `ValidatedBankLeaseSpec::for_rom_switchable`
rejects bank 0 with `BankAbiViolation::RomBankZeroReservedByAbi` — not
because MBC5 hardware cannot map it. This keeps the lease ABI's `Rom`
class meaning unambiguously "I need a *non-fixed* ROM bank visible at
`$4000..=$7FFF`." (Note: the in-tree `gbf-asm::section::MbcBankClass`
has only `Rom` and `Sram` variants — there is no `Rom4000` / `SramA000`
naming. The earlier RFC draft's `Rom4000` references are editorial
shorthand for "the switchable ROM window at `$4000..=$7FFF`", not a
claim about variant names.)

## 3. Architecture

### 3.1 Data flow

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Authoring (existing F-A1)                                               │
│  Builder (Privileged section) → lease_rom_switchable(...) → BankGuard    │
│  Builder (Privileged section) → release_bank(guard, return_state)        │
│  Builder (any section)        → assert_bank(...)                         │
│  ↓                                                                       │
│  pre_layout_ops: Vec<OrderedItem<PreLayoutOp>>                           │
│  Builder.pending_bank_leases: BTreeMap<LeaseId, PendingBankLease>        │
│  Builder::finish() returns BuilderError::UnreleasedBankGuard if non-empty│
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<Section>
┌──────────────────────────────────────────────────────────────────────────┐
│  PreLayoutOp lowering (F-A1 §4 seam, F-A4 supplies the banking portion)  │
│  Section → LoweredSection                                                │
│  - Driven by composite `dyn PreLayoutOpLowering`                         │
│  - F-A4 implements: BankLease, BankRelease, AssertBank                   │
│  - LoweringDisposition::{Lowered, NotOwned, Error}                       │
│  - Each lowering site emits a LoweredFragment of typed Instr items       │
│  - check_lease_emission_legal(section, safety, residency) gates emission │
└──────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼  Vec<LoweredSection> (no pre_layout_ops)
            (then F-A1's layout → relax → encode → ROM pipeline)
```

The diagram covers only the F-A4 surface. The downstream pipeline
(`layout::layout_into_banks`, `relax::relax_and_legalize`,
`encoder::encode_section`, `rom::assemble_rom`) is unchanged from F-A1.

### 3.2 Module layout

F-A4 lands entirely inside `gbf-runtime/src/banking.rs`. A natural internal
breakdown (one file, organized by section comment) is:

| Section in `banking.rs`        | Visibility   | Owns                                                                                                |
|--------------------------------|--------------|------------------------------------------------------------------------------------------------------|
| `mod types`                    | `pub`         | `MbcBankClass`, `LeaseLifetime`, `ValidatedBankLeaseSpec`, `BankLease`, `BankGuard`, `LeaseId`, `LeaseGeneration`, `ReturnState`, `ReturnRomBank`, `ReturnSramState`, `KeepCurrentProof`, `SectionResidency`, `BankAbiViolation`, `BankingEmitError` |
| `mod shadow`                   | mixed        | `pub` HRAM offset constants, `pub` shadow-read helpers, `pub(crate)` shadow-write helpers           |
| `mod emit`                     | `pub(crate)` | `lower_enable_sram`, `lower_disable_sram`, `lower_acquire_rom_bank`, `lower_acquire_sram_bank`, `lower_release` (lowering-only); plus `BankingLoweringState` |
| `mod api`                      | `pub`        | User-facing `lease_rom_switchable`, `lease_sram`, `release_bank` (lease-shaped API only)            |
| `mod isr`                      | `pub`        | `InterruptSafetyKind`, `InterruptSafety`, `InterruptSafetyTable`, `InterruptSafetyError`, `mark_isr_*`, `check_lease_emission_legal` |
| `mod lowering` (impl)          | `pub`        | `BankingPreLayoutLowering`: `impl PreLayoutOpLowering`                                              |

The split is editorial. Public callers say
`use gbf_runtime::banking::{ValidatedBankLeaseSpec, lease_rom_switchable,
release_bank}`. We do not export sub-modules; the outward shape is one
flat module, but the *public surface* is intentionally narrower than the
implementation surface.

### 3.3 Determinism contract

Every emit helper in `gbf-runtime::banking` MUST be deterministic.
Concretely:

- **No `HashMap` iteration**. The pending-lease table on `Builder` and the
  `BankingLoweringState` use `BTreeMap<LeaseId, …>`.
- **No `SystemTime`, no `rand`, no `std::env`, no thread-local state.**
- **`LeaseId` derivation is a pure function** of the source `Section`'s
  builder counter (already exposed by `gbf-asm::Builder` for symbolic-branch
  `BranchId`s and reused here). `LeaseGeneration` increments monotonically
  per `Builder` instance to make stale `BankGuard` tokens detectable.
- **Stable AsmIR output**: same `ValidatedBankLeaseSpec` + same source
  section ID + same `BankingLoweringState` = byte-identical
  `LoweredFragment` whose `Instr`s encode to the same bytes through
  `gbf-asm::encoder`.

The determinism gate is `gbf-runtime::tests::banking::byte_stable_emit` —
a property test that runs every emit helper twice on the same input and
asserts identical AsmIR plus identical encoded bytes.

## 4. T-A4.1 — `BankLease` / `BankGuard` types + `MbcBankClass` + `LeaseLifetime`

### 4.1 Why these types live in `gbf-runtime::banking` rather than `gbf-asm`

`gbf-asm::section` already defines the serializable pre-layout wire shape:
`BankLeaseSpec`, `LeaseId`, and `MbcBankClass` as part of
`PreLayoutOp::BankLease`. F-A4 must *not* add inherent methods to those
foreign types from `gbf-runtime` — Rust permits inherent `impl` blocks
only in the crate that defines the type (Rust's orphan rule for inherent
impls). F-A4 therefore introduces a runtime-side validated request type,
`ValidatedBankLeaseSpec`, that wraps the asm-side spec.

The boundary is:

- **`gbf-asm::section`** owns the *durable description* — the
  `BankLeaseSpec` value carried inside `PreLayoutOp::BankLease`. It has
  no validating constructor; it serializes / deserializes verbatim.
- **`gbf-runtime::banking`** owns the *authoring layer* —
  `ValidatedBankLeaseSpec` (the only legal way to reach the emit path),
  `BankGuard` (host-side linear bookkeeping; not `&'a BankLease`),
  `LeaseLifetime` semantics, shadow updates, privilege/residency checks.

This split mirrors F-A1's split between `Instr` (in `isa.rs`) and the
`Builder` emitter (in `builder.rs`): the type lives next to the data, the
emitter lives next to the runtime.

(Alternatively: move the validating constructors into `gbf-asm::section`
and keep `gbf-runtime` as a pure consumer. F-A4 picks the wrapper
approach so the runtime-side `LeaseLifetime` and policy-shaped rejections
do not bleed into the asm crate.)

### 4.2 Type definitions

```rust
// in gbf-runtime/src/banking.rs

// Re-export the asm-side wire types; never `impl` them here.
pub use gbf_asm::section::{BankLeaseSpec, LeaseId, MbcBankClass};

/// Identifies the residency class of the section emitting a banking
/// primitive. Banking writes are only legal from FixedRom0 or Hram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SectionResidency {
    /// Section is placed in the fixed `$0000..=$3FFF` ROM bank.
    FixedRom0,
    /// Section is in HRAM ($FF80..=$FFFE).
    Hram,
    /// Section is placed in switchable ROM `$4000..=$7FFF`.
    SwitchableRom,
    /// Section is in fixed WRAM (banking writes still illegal).
    Wram,
    /// Other (VRAM, SRAM, OAM, etc.).
    Other,
}

/// Increments per `Builder` so a stale `BankGuard` token is detectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LeaseGeneration(pub u32);

/// How long a `BankLease` is held. Influences yield-safety and what the
/// lowered acquire/release sequence looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LeaseLifetime {
    /// The lease MUST NOT cross a yield boundary; the builder/lowerer
    /// inserts or requires a release before any `Yield`. Yield-safe in
    /// the trivial sense (because yields cannot occur while held).
    Slice,
    /// The continuation record stores the desired bank; the scheduler
    /// reacquires it before resuming. Yield-safe with restoration.
    ResumeWindow,
    /// The token subsystem owns the restoration trigger. Yield-safe.
    Token,
    /// Manual release only. Not yield-safe — the surrounding section
    /// must be statically non-yielding while the lease is active.
    Manual,
}

/// Runtime-side wrapper around `BankLeaseSpec` that has been validated
/// against F-A4's policy. The only way into `lease_rom_switchable` /
/// `lease_sram` is via this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedBankLeaseSpec {
    inner: BankLeaseSpec,
    lifetime: LeaseLifetime,
}

impl ValidatedBankLeaseSpec {
    /// Acquire a switchable ROM bank. Bank 0 is rejected by ABI policy
    /// (use fixed `$0000..=$3FFF` residency for bank-0 code instead).
    pub fn for_rom_switchable(
        bank_n: u16,
        lifetime: LeaseLifetime,
    ) -> Result<Self, BankAbiViolation>;

    /// Acquire an SRAM bank.
    pub fn for_sram(
        bank_n: u8,
        lifetime: LeaseLifetime,
    ) -> Result<Self, BankAbiViolation>;

    pub fn lifetime(&self) -> LeaseLifetime { self.lifetime }
    pub fn into_pre_layout_spec(self) -> BankLeaseSpec { self.inner }
}

/// Where to leave the bank state when releasing a lease. Class-correct:
/// SRAM releases cannot ask for a ROM target and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ReturnState {
    Rom(ReturnRomBank),
    Sram(ReturnSramState),
    /// `KeepCurrent` is *not* a freely-constructible variant. It is only
    /// legal when the scheduler/lowering context has a proof that the
    /// next control-transfer boundary immediately reacquires or restores
    /// the required bank. The proof token is unforgeable from outside
    /// `gbf-runtime::banking`.
    KeepCurrent(KeepCurrentProof),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ReturnRomBank {
    /// Canonical safe return: ROM bank 1 in the switchable window.
    Bank1,
    /// Caller-specified bank.
    Manual(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ReturnSramState {
    /// Disable SRAM (RAMG ← $00) on release.
    Disable,
    /// Switch to a specific SRAM bank on release.
    Bank(u8),
}

/// Unforgeable proof token; only constructible inside
/// `gbf-runtime::banking` (or, eventually, `gbf-runtime::scheduler`)
/// when the lowering context has a static guarantee that the next slice
/// will immediately reacquire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KeepCurrentProof(pub(crate) ());

/// Compiler-side linear-bookkeeping handle for a held lease.
///
/// `BankGuard` does NOT borrow `&'a BankLease` from the builder. It
/// carries a `LeaseId + LeaseGeneration` token; the authoritative
/// pending-lease table is owned by `Builder`. This keeps `&mut Builder`
/// freely usable while a guard is alive — the exact period during which
/// callers need to emit instructions.
#[must_use = "a BankGuard must be explicitly released with release_bank or it will fail Builder::finish"]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BankGuard {
    lease_id: LeaseId,
    source_section_id: SectionId,
    generation: LeaseGeneration,
}

impl BankGuard {
    pub fn lease_id(&self) -> LeaseId { self.lease_id }
    pub fn generation(&self) -> LeaseGeneration { self.generation }
}

// Drop is NOT the primary correctness mechanism. Drop cannot return a
// typed error. The authoritative leak detector is the pending-lease
// table on `Builder`:
//
//   pending_bank_leases: BTreeMap<LeaseId, PendingBankLease>
//
// `release_bank(b, guard, return_state)` consumes the guard and removes
// the matching entry. `Builder::finish()` returns
// `BuilderError::UnreleasedBankGuard` if the table is non-empty.
//
// In debug builds we may add a Drop logger or a debug_assert; we never
// rely on Drop for correctness in release.
impl Drop for BankGuard {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            // Optional: log via a Builder side-channel. Not a panic.
        }
    }
}

/// Resolved lease entry, stored inside `Builder.pending_bank_leases`.
/// Not exposed publicly; consumers see only `BankGuard`.
pub(crate) struct PendingBankLease {
    pub(crate) id: LeaseId,
    pub(crate) spec: BankLeaseSpec,
    pub(crate) lifetime: LeaseLifetime,
    pub(crate) generation: LeaseGeneration,
    pub(crate) source_section_id: SectionId,
}

/// Public read view of an active lease, used by `BankingPreLayoutLowering`
/// and by tests. Constructed by the lowerer, never by user code.
pub struct BankLease {
    pub id: LeaseId,
    pub spec: BankLeaseSpec,
    pub lifetime: LeaseLifetime,
}

/// Closed enumeration of ABI violations local to a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum BankAbiViolation {
    #[error("ROM bank {bank} out of MBC5 range 0..=511")]
    RomBankOutOfRange { bank: u16 },
    #[error("SRAM bank {bank} out of MBC5 range 0..=15")]
    SramBankOutOfRange { bank: u8 },
    #[error("ROM bank 0 reserved by ABI policy; use fixed bank-0 residency instead")]
    RomBankZeroReservedByAbi,
    #[error("ManualLifetime lease cannot be acquired in a yielding section")]
    ManualLeaseInYieldingSection { section: SectionId },
    #[error("InterruptHandler section may not call banking lowering; ISR code must be bank-agnostic")]
    IsrCannotAcquire { section: SectionId },
    #[error("section is Normal, not Privileged; banking emits require Privileged")]
    SectionNotPrivileged { section: SectionId, found: PrivilegeClass },
    #[error("banking primitive emitted from non-fixed residency {residency:?}; MBC writes must run from Bank0 fixed ROM or HRAM")]
    BankingPrimitiveNotFixedResident { section: SectionId, residency: SectionResidency },
    #[error("matching BankRelease missing for lease {lease:?}")]
    UnreleasedLease { lease: LeaseId },
    #[error("lower_acquire_sram_bank called without prior lower_enable_sram")]
    SramBankAcquiredWhileDisabled,
    #[error("ReturnRomBank::Manual({bank}) is out of MBC5 range 1..=511")]
    ReturnBankOutOfRange { bank: u16 },
    #[error("ReturnState class does not match held lease class: lease={lease_class:?}, return={return_class:?}")]
    ReturnStateWrongClass { lease_class: MbcBankClass, return_class: &'static str },
    #[error("nested {class:?} lease not supported (existing={existing:?}, attempted={attempted:?})")]
    NestedLeaseNotSupported { class: MbcBankClass, existing: LeaseId, attempted: LeaseId },
    #[error("MBC5+RUMBLE cartridge profile rejected; SRAM bank values may toggle rumble motor")]
    RumbleCartProfileRejected,
}
```

### 4.3 `ValidatedBankLeaseSpec` constructor invariants

The validating constructors live on the wrapper, not on the foreign
`BankLeaseSpec`:

```rust
impl ValidatedBankLeaseSpec {
    pub fn for_rom_switchable(bank_n: u16, lifetime: LeaseLifetime)
        -> Result<Self, BankAbiViolation>;
    pub fn for_sram(bank_n: u8, lifetime: LeaseLifetime)
        -> Result<Self, BankAbiViolation>;
}
```

`for_rom_switchable` rejects `bank_n == 0` with
`BankAbiViolation::RomBankZeroReservedByAbi` (ABI policy, not a hardware
limit) and rejects `bank_n > 511` with `RomBankOutOfRange`. `for_sram`
rejects `bank_n > 15` with `SramBankOutOfRange`, and (when the cartridge
profile resolves to MBC5+RUMBLE) rejects with `RumbleCartProfileRejected`.

The `lifetime` field flows through to `LeaseLifetime` and is consulted by
the acquire lowering to choose the critical-section policy and by the
release lowering to decide whether to emit a literal switch or rely on
the scheduler.

### 4.4 Tests (T-A4.1 acceptance)

```rust
#[test] fn lease_spec_invariants_rom();         // 0 rejected (RomBankZeroReservedByAbi), 1 ok, 511 ok, 512 rejected (RomBankOutOfRange)
#[test] fn lease_spec_invariants_sram();        // 0 ok, 15 ok, 16 rejected
#[test] fn lifetime_yield_safety();             // Slice/ResumeWindow/Token = yield-safe by construction;
                                                // Manual = rejected from yielding sections
#[test] fn rumble_cart_profile_rejected_for_sram_banks();
#[test] fn bank_guard_drop_without_release();   // Builder::finish returns BuilderError::UnreleasedBankGuard;
                                                // Drop itself does not panic in release
#[test] fn bank_guard_double_release();         // BuilderError::DoubleRelease via stale generation
#[test] fn bank_guard_does_not_borrow_builder(); // a guard is alive while &mut Builder is freely usable
#[test] fn return_bank_zero_rejected();         // ReturnRomBank::Manual(0) is BankAbiViolation::ReturnBankOutOfRange
#[test] fn return_state_class_correct();        // SRAM lease + ReturnState::Rom rejected (and vice versa)
#[test] fn keep_current_proof_unforgeable();    // KeepCurrentProof cannot be constructed outside banking
#[test] fn manual_lease_rejects_yield_while_held();
#[test] fn nested_rom_lease_rejected_or_stack_restored();
#[test] fn nested_sram_lease_rejected_or_stack_restored();
```

## 5. T-A4.2 — HRAM shadow registers + RAM-enable state

### 5.1 Why HRAM is the only legal location

DMG hardware constraints: the `$FF80..=$FFFE` HRAM region (127 bytes) is
addressable independently of ROM banking, never paged out, and accessible
through the 1-byte `LDH` instruction (2 cycles versus 4 for the absolute
form). This makes it the only RAM region that an ISR is guaranteed to be
able to touch without a bank-switch dance, which means:

- it is the *only* legal home for software shadows that ISRs may consult,
- every cycle saved by `LDH` matters in the bank-switch critical section,
- the HRAM budget is small (127 bytes total), so the shadow region must
  itself be small and fully audited.

F-A4 reserves only the **banking-owned** bytes:

| Address | Owner   | Meaning              |
|---------|---------|----------------------|
| `$FF80` | banking | current ROM bank lo  |
| `$FF81` | banking | current ROM bank hi  |
| `$FF82` | banking | current SRAM bank    |
| `$FF83` | banking | SRAM enabled token   |

Scheduler `yield_requested`, fault codes, ISR scratch, and any future
fast flags belong in a separate `gbf-runtime::hram` reservation table
owned by F-A5. F-A4 may *depend* on that table but does not claim or
zero non-banking bytes. This avoids hidden cross-feature coupling and
makes `lower_banking_shadow_zero_init` stomp only banking state.

### 5.2 Shadow layout

```rust
// in gbf-runtime/src/banking.rs (mod shadow)

pub const HRAM_SHADOW_BASE:                  u16 = 0xFF80;

// Absolute u16 addresses, used in error messages and grep-friendly audits.
pub const HRAM_ADDR_CURRENT_ROM_BANK_LO:     u16 = HRAM_SHADOW_BASE + 0;
pub const HRAM_ADDR_CURRENT_ROM_BANK_HI:     u16 = HRAM_SHADOW_BASE + 1;
pub const HRAM_ADDR_CURRENT_SRAM_BANK:       u16 = HRAM_SHADOW_BASE + 2;
pub const HRAM_ADDR_SRAM_ENABLED:            u16 = HRAM_SHADOW_BASE + 3;

// 8-bit `LDH` operands ($FF00 + offset). The encoder takes a
// `HighDirectOffset(u8)`, so every shadow access is two bytes wide.
pub const HRAM_LDH_CURRENT_ROM_BANK_LO:      u8  = 0x80;
pub const HRAM_LDH_CURRENT_ROM_BANK_HI:      u8  = 0x81;
pub const HRAM_LDH_CURRENT_SRAM_BANK:        u8  = 0x82;
pub const HRAM_LDH_SRAM_ENABLED:             u8  = 0x83;

// Banking-owned region only.
pub const HRAM_BANKING_SHADOW_END_EXCLUSIVE: u16 = HRAM_SHADOW_BASE + 4;  // $FF84

/// Pure read view of the banking shadow. Materialized only by tests and
/// by host-side reflection helpers; `gbf-runtime::banking` does not
/// store one.
pub struct ShadowRegisters {
    pub current_rom_bank:  u16,    // 9-bit MBC5
    pub current_sram_bank: u8,     // 4-bit MBC5
    pub sram_enabled:      bool,
}
```

### 5.3 Shadow access helpers

Shadow **reads** may be public because ISRs and diagnostics need to inspect
the current bank state. Shadow **writes** to the banking-owned bytes are
privileged and `pub(crate)`-internal to `gbf-runtime::banking`: corrupting
the shadow is equivalent to lying about the MBC state, which would
silently break `AssertBank`, ISR logic, and scheduler handoff.

Every helper emits typed `Instr`s through `Builder`; none touches a raw
byte. Each shadow access is an `LDH` (`Ld8HighDirectFromA`,
`Ld8AImmFromHighDirect`) or `Ld8AFromHighDirect` form, which never
depends on the switchable bank. This is what lets ISRs read the shadow
safely.

```rust
/// (pub(crate)) Emit `LDH ($FF00 + offset), A` — store register A into a
/// banking-owned shadow byte. Only banking-owned offsets are accepted.
pub(crate) fn emit_store_bank_shadow_byte_from_a(
    b: &mut Builder,
    offset: HighDirectOffset,
) -> Result<(), BankingEmitError>;

/// (pub(crate)) Emit `LD A, n; LDH ($FF00 + offset), A` — store an
/// immediate byte. Banking-owned offsets only.
pub(crate) fn emit_store_bank_shadow_byte_imm(
    b: &mut Builder,
    offset: HighDirectOffset,
    value: u8,
) -> Result<(), BankingEmitError>;

/// (pub) Emit `LDH A, ($FF00 + offset)` — load a banking-owned shadow
/// byte into register A. Public because non-banking code legitimately
/// needs to inspect (read) the current bank state.
pub fn emit_load_bank_shadow_byte_into_a(
    b: &mut Builder,
    offset: HighDirectOffset,
) -> Result<(), BankingEmitError>;
```

The store helpers:

- accept only the four banking-owned `HighDirectOffset` values
  (`HRAM_LDH_CURRENT_ROM_BANK_LO/HI`, `HRAM_LDH_CURRENT_SRAM_BANK`,
  `HRAM_LDH_SRAM_ENABLED`); any other offset is
  `BankingEmitError::ShadowOffsetOutOfRange`,
- pass the resulting `Instr` through the existing builder validation,
  which classifies the effect as `MachineEffect::StoreToHram` or
  `LoadFromHram` — not `Privileged`, because the privileged effect
  (`StoreToMbcRegister`) is the *paired* MBC write emitted alongside,
  not the shadow update itself,
- are unreachable from outside `gbf-runtime::banking`, so arbitrary
  `Privileged` code cannot lie to the authoritative shadow.

### 5.4 Banking-shadow init: `lower_banking_shadow_zero_init`

F-A5 (`bd-2r1`) calls this once from `boot_init`'s banking-init step
before any code runs that depends on the banking shadow. It writes 4
zero bytes across `$FF80..=$FF83`, leaving the banking shadow in a
consistent "no banks held, SRAM disabled" state. F-A5 separately
zero-initializes its own HRAM regions through its own helper; F-A4 does
not stomp those.

```rust
/// (pub) Emit the 4-byte banking-owned HRAM shadow zero-init. F-A5's
/// `boot_init` calls this exactly once. Other HRAM regions are owned
/// by F-A5 and zeroed separately.
pub fn lower_banking_shadow_zero_init(
    b: &mut Builder,
) -> Result<(), BankingEmitError>;
```

Sequence (4-byte banking shadow):

```text
LD  A, $00                    ; 2 bytes, 2 M-cycles
LDH ($FF80), A                ; 2 bytes, 3 M-cycles
LDH ($FF81), A                ; 2 bytes, 3 M-cycles
LDH ($FF82), A                ; 2 bytes, 3 M-cycles
LDH ($FF83), A                ; 2 bytes, 3 M-cycles
```

Total: **10 bytes / 14 M-cycles**.

(Per Pan Docs / SM83 optable: `LD A, n8` is 2 bytes / 8 T-cycles =
2 M-cycles; `LDH [a8], A` is 2 bytes / 12 T-cycles = 3 M-cycles. A
hypothetical 16-byte zero-init using one `LD A, 0` + sixteen `LDH`
stores would be 34 bytes / 50 M-cycles, *not* 33 / 35 as an earlier
draft of this RFC erroneously claimed; using `XOR A` for the load would
be 33 bytes / 49 M-cycles. `Instr::XorA { src: AluSrc8 }` is now in
tree, so the `XOR A` variant is reachable without ISA work — F-A4
nevertheless picks the `LD A, $00` form for the banking-shadow zero-init
because the 1-byte saving does not justify diverging from the
load-immediate shape used elsewhere in the helpers.)

### 5.5 Tests (T-A4.2 acceptance)

```rust
#[test] fn hram_shadow_offsets_within_banking_range();    // every offset is in $FF80..=$FF83
#[test] fn hram_banking_shadow_region_size();             // banking-owned region is exactly 4 bytes
#[test] fn hram_shadow_no_overlap_with_other_runtime();   // F-A5 reservation respected
#[test] fn store_bank_shadow_emits_ldh();                 // emit_store_bank_shadow_byte_imm produces
                                                          // exactly LD_A_IMM + LDH_DIRECT_FROM_A
#[test] fn store_bank_shadow_rejects_non_banking_offsets();
#[test] fn store_bank_shadow_helpers_are_pub_crate();     // grep + visibility check
#[test] fn banking_shadow_zero_init_byte_stable();        // identical bytes across two builds
#[test] fn banking_shadow_zero_init_byte_and_cycle_count();// 10 bytes / 14 M-cycles exactly
#[test] fn banking_shadow_zero_init_machine_effects();    // every emit is StoreToHram, not StoreToMbcRegister
#[test] fn no_bank_shadow_writes_outside_banking_rs();    // workspace-wide visibility audit
```

## 6. T-A4.3 — Acquire / release primitives in AsmIR

This is where the rubber meets the road. Each emit helper produces a fixed,
audited sequence of typed `Instr`s with explicit per-step provenance.

### 6.1 Surface — public API and lowering primitives

The user-facing public API is **lease-shaped**. Callers acquire and
release banks; only the banking lowerer emits MBC writes:

```rust
/// Public: validate, push a PreLayoutOp::BankLease, return a BankGuard.
pub fn lease_rom_switchable(
    b: &mut Builder,
    spec: ValidatedBankLeaseSpec,
) -> Result<BankGuard, BankingEmitError>;

/// Public: validate, push a PreLayoutOp::BankLease for SRAM, return a BankGuard.
pub fn lease_sram(
    b: &mut Builder,
    spec: ValidatedBankLeaseSpec,
) -> Result<BankGuard, BankingEmitError>;

/// Public: consume a guard, push a PreLayoutOp::BankRelease with the
/// chosen ReturnState. The lowerer materializes the bytes during
/// PreLayoutOpLowering, not here.
pub fn release_bank(
    b: &mut Builder,
    guard: BankGuard,
    return_state: ReturnState,
) -> Result<(), BankingEmitError>;
```

The lowering primitives are `pub(crate)`. They are reachable only from
`BankingPreLayoutLowering` (and from in-crate tests):

```rust
/// (pub(crate)) Enable SRAM by writing $0A to RAMG ($0000-$1FFF).
pub(crate) fn lower_enable_sram(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError>;

/// (pub(crate)) Disable SRAM by writing $00 to RAMG.
pub(crate) fn lower_disable_sram(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError>;

/// (pub(crate)) Switch ROM bank: BANK1 then BANK2 then shadow.
pub(crate) fn lower_acquire_rom_bank(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    bank: u16,                                     // 1..=511 by ABI policy
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError>;

/// (pub(crate)) Switch SRAM bank: RAMB then shadow.
/// Validates SRAM-enable ordering using `state`.
pub(crate) fn lower_acquire_sram_bank(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    bank: u8,                                      // 0..=15
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError>;

/// (pub(crate)) Release a held lease, dispatching on lease class and
/// the requested ReturnState.
pub(crate) fn lower_release(
    b: &mut Builder,
    state: &mut BankingLoweringState,
    lease: &BankLease,
    return_state: ReturnState,
    policy: InterruptPolicy,
) -> Result<(), BankingEmitError>;
```

Where `BankingLoweringState` tracks deterministic state during lowering
(needed for SRAM-enable ordering, nested-lease detection, and
KeepCurrent validation):

```rust
pub(crate) struct BankingLoweringState {
    pub(crate) current_rom_bank:  Option<u16>,
    pub(crate) current_sram_bank: Option<u8>,
    pub(crate) sram_enabled:      bool,
    pub(crate) active_leases:     BTreeMap<LeaseId, ActiveLease>,
}
```

Hardware does not require SRAM to be enabled before selecting RAMB;
F-A4's enable-before-bank rule is an ABI policy, and `BankingLoweringState`
is what lets the lowerer enforce it deterministically.

```rust
pub use gbf_abi::interrupt::InterruptPolicy;        // Enabled | ShortCriticalSection | Disabled
```

### 6.2 The four-instruction MBC write sequence

The canonical body of `lower_acquire_rom_bank(bank=N)` under
`InterruptPolicy::ShortCriticalSection` is:

```text
DI                                ; 1 byte, 1 M-cycle  — clear IME
LD   A, <N & 0xFF>                ; 2 bytes, 2 M-cycles — low 8 bits
LD   ($2000), A                   ; 3 bytes, 4 M-cycles — write BANK1
LD   A, <(N >> 8) & 0x01>          ; 2 bytes, 2 M-cycles — high bit
LD   ($3000), A                   ; 3 bytes, 4 M-cycles — write BANK2
LD   A, <N & 0xFF>                ; 2 bytes, 2 M-cycles — for shadow
LDH  ($FF80), A                   ; 2 bytes, 3 M-cycles — shadow lo
LD   A, <(N >> 8) & 0x01>          ; 2 bytes, 2 M-cycles
LDH  ($FF81), A                   ; 2 bytes, 3 M-cycles — shadow hi
EI                                ; 1 byte, 1 M-cycle  — request IME re-enable
                                  ;                    after the next instruction
```

Total: **20 bytes / 24 M-cycles** per acquire. `EI` itself costs 1 M-cycle;
its delayed effect (the IME re-enable taking effect after the *next*
instruction) is an interrupt-semantics rule, not an additional cycle to
add to the timing table.

The whole sequence must execute from fixed bank 0 (`$0000..=$3FFF`) or
HRAM. It must *not* run from the switchable window: the `LD ($2000), A`
write would change the memory backing the remaining instructions in
flight. `check_lease_emission_legal(section, safety, residency)` enforces
this with `BankingPrimitiveNotFixedResident`.

Optimizations possible but **not** taken in F-A4:

- Skip the BANK2 write if the static `N` has bit 8 == 0 and the shadow's
  current high byte is statically known to be 0. F-A4 *always* writes
  both bytes for safety; downstream optimization can prove the skip.
- Reuse register A across the two pairs. F-A4 always emits a fresh `LD A,
  imm` to keep each step independently verifiable; downstream peephole can
  collapse identical reloads.
- Use the absolute-address `LD ($FFnn), A` form instead of `LDH`. F-A4
  uses `LDH` exclusively because (a) it's one byte shorter and (b) the
  encoder's `Ld8HighDirectFromA` form is type-rejected from
  non-`$FF00..=$FFFF` targets, which is one extra invariant for free.

The fixed-size, fixed-shape sequence is the design point. Every kernel,
every codegen path, every far-call thunk gets the *same* AsmIR for the
*same* `(bank, policy)` tuple. This is what `byte_stable_emit` certifies.

### 6.3 Per-policy variations

```text
InterruptPolicy::ShortCriticalSection:
   DI ... EI bracket as shown above. 20 bytes / 24 M-cycles.

   Precondition: the caller expects IME to be enabled on entry. SCS is
   NOT nestable. The Game Boy CPU does not provide a normal "read IME"
   instruction, so a DI/EI bracket cannot restore the prior IME state —
   the trailing EI unconditionally re-enables interrupts. If the caller
   may already be inside an interrupt-disabled region (boot init, ISR
   prologue, an outer SCS), it MUST pass InterruptPolicy::Disabled
   instead; otherwise the trailing EI accidentally re-enables interrupts
   that the caller intended to keep disabled.

InterruptPolicy::Disabled:
   No DI/EI — IME is already cleared because the slice runs with
   interrupts off entirely. 18 bytes / 22 M-cycles.

InterruptPolicy::Enabled:
   Rejected by lower_acquire_*. Acquiring a switchable-bank lease while
   IME is enabled and the lifetime is yield-safe is provably unsafe
   (an IRQ between the BANK1 write and the shadow update would observe
   inconsistent state). The local check fires:
       BankingEmitError::EnabledPolicyForRomAcquire
       BankingEmitError::EnabledPolicyForSramAcquire
```

### 6.4 `lower_release` — class-correct, policy-explicit

The release dispatches on the held lease's `MbcBankClass` and the
requested `ReturnState`. The lowerer rejects class mismatch
(`ReturnStateWrongClass`).

For lifetime `Slice` / `ResumeWindow` / `Token` and
`ReturnState::KeepCurrent(_proof_)`, the release emits zero `Instr`s but
still emits a `Label` carrying the release provenance so listings show
the release event explicitly. `KeepCurrent` is legal only with a valid
`KeepCurrentProof` token, which only the banking module (and, eventually,
the F-A5 scheduler lowerer) can construct.

For ROM leases:

- `ReturnState::Rom(ReturnRomBank::Bank1)` → `lower_acquire_rom_bank(1, policy)`
  (the canonical safe return; reuses the audited 20/18-byte path).
- `ReturnState::Rom(ReturnRomBank::Manual(M))` →
  `lower_acquire_rom_bank(M, policy)` (same shape, parameter M).
  Bank-0 rejection still applies (`ReturnBankOutOfRange`).

For SRAM leases:

- `ReturnState::Sram(ReturnSramState::Disable)` →
  `lower_disable_sram(policy)` (write `$00` to RAMG, update shadow).
- `ReturnState::Sram(ReturnSramState::Bank(M))` →
  `lower_acquire_sram_bank(M, policy)`.

Cross-class returns (e.g., a ROM lease asking for a `ReturnSramState`)
return `BankAbiViolation::ReturnStateWrongClass`.

### 6.5 Effect classification — privilege + provenance + residency

Every `LD ($0000), A` / `LD ($2000), A` / `LD ($3000), A` /
`LD ($4000), A` instruction emitted by F-A4 classifies through
`gbf-asm::effect::classify_effect` to:

```rust
MachineEffect::StoreToMbcRegister { reg: <RamEnable | RomBankLow | RomBankHigh | SramBank> }
```

Whose `privilege_of` returns `PrivilegeClass::Privileged`. The
`Builder::validate_effect` path (already implemented in T-A1.3) compares
this against the section's `default_privilege` and rejects if the section
is `Normal`. That is necessary but not sufficient. F-A4 adds a second
guard: `Privileged` code outside `gbf-runtime::banking` must not emit
raw MBC stores. F-A4 enforces this with **two** mechanisms:

1. The MBC-writing emitters are `pub(crate)` and reachable only from
   `BankingPreLayoutLowering`. There is no public path to call
   `lower_acquire_rom_bank` directly.
2. A workspace-level **effect/provenance audit** walks every emitted
   `Instr`, checks for `StoreToMbcRegister`, and asserts the surrounding
   section is `Privileged`, fixed-resident (Bank-0 or HRAM), not
   ISR-reachable, and has provenance `gbf-runtime::banking`. A grep is
   supplemental only — code can construct `DirectAddr` values indirectly
   or through constants, so a grep cannot prove semantic absence of MBC
   writes.

The first-line failure modes are therefore:

```text
A Normal section calls lease_rom_switchable then triggers the lowering
       ↓
The first LD ($2000), A reaches Builder::validate_effect.
       ↓
Builder rejects with PrivilegeViolation { required: Privileged, section: Normal }.
       ↓
The section's compile fails before any byte is materialized.

A Privileged switchable-residency section calls the same path
       ↓
check_lease_emission_legal(section, safety, residency) returns
       BankAbiViolation::BankingPrimitiveNotFixedResident.

A Privileged fixed-resident section outside gbf-runtime::banking somehow
acquires a callable to lower_acquire_rom_bank
       ↓
Effect/provenance audit fails at PR/CI time.
```

This is the F-A1 §I.1 "shifted-left" guarantee in concrete form:
violations of the banking ABI are caught at *builder time* and at
*audit time*, not at codegen, layout, encoding, or run time.

### 6.6 Tests (T-A4.3 acceptance)

```rust
#[test] fn lower_enable_sram_byte_sequence();              // exact SCS sequence (DI, LD A,0A, LD (0000),A, LDH (FF83),A, EI)
#[test] fn lower_disable_sram_byte_sequence();             // exact SCS sequence (DI, LD A,00, LD (0000),A, LDH (FF83),A, EI)
#[test] fn lower_acquire_rom_bank_byte_sequence_bank3();   // exact 20-byte sequence for N=3
#[test] fn lower_acquire_rom_bank_byte_sequence_bank256(); // exact 20-byte sequence for N=256 (BANK2 != 0)
#[test] fn lower_acquire_rom_bank_disabled_policy();       // 18-byte sequence (no DI/EI)
#[test] fn lower_acquire_rom_bank_rejects_enabled_policy();// BankingEmitError::EnabledPolicyForRomAcquire
#[test] fn lower_acquire_rom_bank_rejects_bank_zero();     // BankAbiViolation::RomBankZeroReservedByAbi
#[test] fn lower_acquire_sram_bank_byte_sequence();        // exact sequence (see §6.4 for chosen byte count)
#[test] fn lower_acquire_sram_bank_rejects_disabled();     // BankingLoweringState says SRAM disabled → reject
#[test] fn lower_release_keep_current_is_label_only();     // zero Instrs, one Label provenance
#[test] fn lower_release_to_bank1_is_acquire_one();        // identical bytes to lower_acquire_rom_bank(1, …)
#[test] fn lower_release_class_correct();                  // ROM lease + ReturnSramState → ReturnStateWrongClass
#[test] fn lower_release_sram_disable();                   // ReturnSramState::Disable emits lower_disable_sram bytes
#[test] fn banking_emit_rejects_switchable_residency();    // residency=SwitchableRom rejected even with Privileged
#[test] fn scs_policy_does_not_claim_to_restore_prior_ime();// docs+test pin SCS precondition
#[test] fn scs_policy_rejects_nested_or_unknown_ime();     // nesting check
#[test] fn all_emits_are_privileged();                     // every emitted Instr that touches an MBC register classifies to PrivilegeClass::Privileged
#[test] fn shadow_update_after_register_write();           // the BANK1 write precedes the shadow write — never the other way around
#[test] fn golden_listings_never_start_in_4000_7fff();     // every golden artifact's start address is < $4000 or in HRAM
#[test] fn no_raw_mbc_writes_anywhere_else();              // effect/provenance audit over emitted AsmIR; grep is supplemental only
#[test] fn no_bank_shadow_writes_outside_banking_rs();     // visibility audit
#[test] fn mbc_write_provenance_audit();                   // every StoreToMbcRegister has provenance gbf-runtime::banking
#[test] fn manual_lease_rejects_yield_while_held();        // builder/lowering state machine refuses Yield
#[test] fn nested_rom_lease_rejected_or_stack_restored();
#[test] fn nested_sram_lease_rejected_or_stack_restored();
#[test] fn mbc5_rumble_profile_rejected_for_sram_banks();
#[test] fn byte_stable_emit_property();                    // every helper, called twice, produces byte-equal AsmIR
#[test] fn cycle_cost_matches_pan_docs();                  // sum of cycle_cost(Instr) per helper matches the table in §6.2
```

## 7. T-A4.4 — Interrupt-safe scaffolding for the BankLease ABI

### 7.1 The two-phase approach

Per `planv0.md` line 1583, the failure mode F-A4 must avoid is *"my runtime
locks up after twenty minutes on cartridge because bank shadow state and
hardware bank state diverged across an interrupt boundary."* The proof
obligation is:

```text
Every code path that may execute with IME enabled must not transitively
reach an MBC write unless the write is protected by an accepted
short-critical-section policy and the writing section is not
ISR-reachable. ISR handlers and ISR-reachable code must be bank-agnostic.
```

The "transitively reach" half is global. Epic B's `ReachabilityValidation`
owns the proof. F-A4 owns the *declarations* the proof reads, and a
*local-only* check that catches the obvious cases at builder time.

### 7.2 The annotations

```rust
/// Section-level interrupt-safety declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InterruptSafety {
    pub kind: InterruptSafetyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InterruptSafetyKind {
    /// IME is cleared the entire time this section runs. Free to depend
    /// on any switchable bank, and may legally emit banking primitives
    /// (subject to fixed-residency and privilege gates).
    InterruptDisabled,
    /// IME is enabled. Section MUST live in Bank0/HRAM/fixed-WRAM and
    /// MUST NOT depend on the switchable bank.
    InterruptEnabledBank0Only,
    /// Section IS an ISR.
    InterruptHandler,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum InterruptSafetyError {
    #[error("conflicting safety declaration for section {section:?}: was {old:?}, now {new:?}")]
    ConflictingDeclaration {
        section: SectionId,
        old: InterruptSafetyKind,
        new: InterruptSafetyKind,
    },
}

/// Declare a section's interrupt safety. The annotation lives in a
/// side-table; `gbf-asm::Section` is unmodified.
pub fn mark_isr_unreachable(
    table: &mut InterruptSafetyTable,
    section: &Section,
) -> Result<(), InterruptSafetyError>;

pub fn mark_isr_reachable(
    table: &mut InterruptSafetyTable,
    section: &Section,
) -> Result<(), InterruptSafetyError>;

pub fn mark_isr(
    table: &mut InterruptSafetyTable,
    section: &Section,
) -> Result<(), InterruptSafetyError>;
```

Why annotations rather than mutating `SectionPrivilege`: privilege is
about *what effects are legal*; interrupt safety is about *when the
section runs*. They are orthogonal. A `Privileged` section may be
`InterruptDisabled` and fixed-resident, such as boot init. A section
that is ISR-reachable must not perform banking writes, even if it is
fixed-resident.

The full matrix:

| Section role              | Privilege    | ISR-reachable? | Fixed-resident? | May emit banking writes? |
|---------------------------|--------------|----------------|-----------------|--------------------------|
| Normal payload/kernel     | Normal       | no             | n/a             | no                       |
| Banking runtime helper    | Privileged   | no             | yes (Bank0/HRAM)| **yes**                  |
| Banking helper, switchable| Privileged   | no             | no (`$4000`+)   | no — `BankingPrimitiveNotFixedResident` |
| ISR handler               | Normal       | yes            | yes (Bank0/HRAM)| no                       |
| ISR-reachable dispatcher  | Normal/Priv* | yes            | yes             | no                       |

*`Privileged + ISR-reachable` is allowed only for non-banking privileged
effects explicitly whitelisted elsewhere. It is **not** sufficient for
MBC writes.

### 7.3 The local check

```rust
pub fn check_lease_emission_legal(
    section: &Section,
    safety: InterruptSafety,
    residency: SectionResidency,
) -> Result<(), BankAbiViolation>;
```

The check is small but covers three cases:

- if `safety.kind == InterruptHandler`, return
  `BankAbiViolation::IsrCannotAcquire`;
- if the section's `default_privilege` is not `Privileged`, return
  `BankAbiViolation::SectionNotPrivileged`;
- if `residency` is neither `FixedRom0` nor `Hram`, return
  `BankAbiViolation::BankingPrimitiveNotFixedResident`. A bank-switch
  sequence cannot run from `$4000..=$7FFF`, because the first ROM-bank
  write may change the memory backing the remaining instructions.

This does *not* catch the case where a non-ISR, fixed-resident,
`Privileged` section is *reachable* from an ISR through a chain of
calls. That's the `ReachabilityValidation` obligation. F-A4 records the
section's `InterruptSafety` and exposes it through a serializable
side-table so the later pass has something to walk.

### 7.4 The side-table

```rust
/// Owned by the runtime build context (passed to `gbf-codegen` from
/// `gbf-runtime`). Populated as sections are constructed.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct InterruptSafetyTable {
    pub by_section: BTreeMap<SectionId, InterruptSafety>,
}

impl InterruptSafetyTable {
    pub fn declare(
        &mut self,
        section: &Section,
        kind: InterruptSafetyKind,
    ) -> Result<(), InterruptSafetyError>;
    pub fn lookup(&self, section: SectionId) -> Option<InterruptSafety>;
    pub fn export(&self) -> serde_json::Value;       // for ReachabilityValidation
}
```

`declare` returns `InterruptSafetyError::ConflictingDeclaration` if a
section is redeclared with a different kind. Redeclaration with the same
kind is a no-op.

The side-table is the *durable evidence* that survives serialization.
Epic B reads the JSON (or rmp-serde, depending on what the codegen
pipeline chooses) and walks the call graph; any reachable section whose
declared safety is incompatible with its caller's safety is a typed
`ReachabilityError`.

### 7.5 Tests (T-A4.4 acceptance)

```rust
#[test] fn isr_section_cannot_lease();                     // check_lease_emission_legal returns IsrCannotAcquire
#[test] fn privileged_non_isr_fixed_section_can_lease();   // check_lease_emission_legal returns Ok
#[test] fn normal_section_cannot_lease();                  // BankAbiViolation::SectionNotPrivileged
#[test] fn switchable_residency_cannot_lease();            // BankAbiViolation::BankingPrimitiveNotFixedResident
#[test] fn wram_residency_cannot_lease();                  // (banking writes still illegal from WRAM)
#[test] fn annotation_serializable_round_trip();           // serde JSON round-trip via InterruptSafetyTable::export
#[test] fn annotation_table_lookup_stable();               // BTreeMap-backed; iteration is sorted by SectionId
#[test] fn three_kinds_are_orthogonal_to_privilege();      // all 3*3 combos are constructible (the four banned combos error at builder time)
#[test] fn declare_conflicting_kind_errors();              // ConflictingDeclaration; same kind is no-op
```

## 8. The `PreLayoutOpLowering` implementation

F-A4 supplies the production implementation of the **banking portion** of
the trait F-A1 §4.3 left as a seam. The current `gbf-asm::lowering`
trait returns `Result<LoweredFragment, LoweringError>`; F-A4 introduces
a richer disposition shape (the composite lowerer needs to distinguish
"not owned" from "owned but failed"), plus the banking implementation
that consumes it. The two pieces:

```rust
// New, introduced by F-A4 in gbf-asm::lowering or a sibling module.
//
// The composite lowerer in F-A5 must distinguish "not owned by this
// lowerer" from "owned but failed". Returning the first `Ok` (or
// swallowing a typed error as 'not owned') would let a banking op that
// fails validation fall through to a later lowerer that doesn't know
// about MBC writes.
pub enum LoweringDisposition {
    Lowered(LoweredFragment),
    NotOwned,
    Error(LoweringError),
}

pub struct BankingPreLayoutLowering {
    pub default_policy: InterruptPolicy,
    pub default_lifetime: LeaseLifetime,
    pub safety_table: InterruptSafetyTable,
}

impl BankingPreLayoutLowering {
    pub fn lower(
        &self,
        op: &PreLayoutOp,
        ctx: &LoweringContext<'_>,
        state: &mut BankingLoweringState,
    ) -> LoweringDisposition {
        match op {
            PreLayoutOp::BankLease(spec) =>
                self.lower_bank_lease(spec, ctx, state).into(),
            // PreLayoutOp::BankRelease currently carries only `{ lease_id }`.
            // F-A4 either (a) extends the variant to
            // `BankRelease { lease_id, return_to: ReturnState }` in
            // `gbf-asm::section` so the lowering is pure-functional, or
            // (b) keeps the wire shape and threads the requested
            // `ReturnState` through `BankingLoweringState`'s active-lease
            // table indexed by `LeaseId`. Option (a) is preferred for
            // determinism; option (b) avoids touching the asm crate's
            // serializable PreLayoutOp shape. The implementation picks
            // one explicitly.
            PreLayoutOp::BankRelease { lease_id /* + return_to once extended */ } =>
                self.lower_bank_release(*lease_id, /* return_to */ ctx, state).into(),
            PreLayoutOp::AssertBank { expected, expected_n } =>
                self.lower_assert_bank(*expected, *expected_n, ctx, state).into(),
            // Yield + TraceProbe belong to other modules.
            PreLayoutOp::Yield { .. } | PreLayoutOp::TraceProbe { .. } =>
                LoweringDisposition::NotOwned,
        }
    }
}
```

The composite lowerer (in F-A5 `gbf-runtime::compose`) walks a chain of
lowerers and dispatches:

- `LoweringDisposition::Lowered(fragment)` — done.
- `LoweringDisposition::NotOwned` — try the next lowerer.
- `LoweringDisposition::Error(e)` — abort lowering with `e`; do **not**
  try the next lowerer.

F-A4 only ships its own implementation; the composite walker is F-A5
territory.

Note on `PreLayoutOp::BankRelease`: the F-A1-shipped variant is just
`{ lease_id: LeaseId }`. The release lowering needs the `ReturnState`
(and the held lease's class) to dispatch correctly. The cleanest design
is to extend the variant to `{ lease_id, return_to: ReturnState }` —
storing it on the `PreLayoutOp` keeps the lowering pure-functional
rather than requiring the lowerer to maintain release metadata in the
active-lease table. Because that change touches the asm crate's
serializable wire shape, F-A4 must include the asm-side migration in
this PR; alternatively, F-A4 can keep the wire shape and instead
record the requested `ReturnState` in `BankingLoweringState` indexed by
`LeaseId` at lease/lower time. The implementation picks one
explicitly and updates the claim-to-gate matrix accordingly.

### 8.1 `lower_assert_bank` — two shapes

F-A4 offers `AssertBank` lowering in two shapes; the build profile
selects which:

1. **Label-only (default in F-A4):** `lower_assert_bank` emits zero
   `Instr`s, just a `Label` carrying the assertion provenance. This is
   the F-A4 default because option 2 expands the dependency surface
   beyond what §2.1 lists.
2. **Compare-and-trap (opt-in extension under `Bringup`/`Trace`):**

   ```text
   LDH  A, ($FF80)              ; load shadow lo
   CP   A, <expected_n & 0xFF>  ; compare
   JR   NZ, .panic_thunk        ; on mismatch, jump to panic
   LDH  A, ($FF81)              ; load shadow hi
   CP   A, <(expected_n >> 8) & 0x01>
   JR   NZ, .panic_thunk
   ```

   In the actual F-A1 ISA, this shape uses `Instr::CpA { src:
   AluSrc8::Imm(n) }` and `Instr::JrRel { cond: Some(Cond::Nz), off }`;
   both variants exist (the earlier RFC names `Instr::CpAImm` and
   `Instr::JrCond` do not). The shape additionally requires a fixed-bank
   panic thunk and relocation support, and is therefore opt-in. If the
   feature is enabled and the panic-thunk symbol is missing,
   `BankingPreLayoutLowering::new` returns an `EmitterMissing` error so
   a misconfigured build fails immediately rather than silently
   producing label-only output.

Either way: under build profiles `Default` and `Speed`, `AssertBank`
collapses to a `Label`, exactly as in shape 1.

## 9. Cross-cutting concerns

### 9.1 `gbf-hw` dependency

T-A2.3 (`bd-121`) closed in May 2026; `gbf-hw/src/mbc5.rs` is no longer
a stub. F-A4 imports the following from the in-tree `gbf-hw::mbc5`
(note the actual constant names — the earlier RFC draft used
`MBC5_RAM_ENABLE_TOKEN`/`MBC5_RAM_DISABLE_TOKEN`, which do not exist):

```rust
pub const MBC5_RAMG_BASE:         u16 = 0x0000;
pub const MBC5_RAMG_END:          u16 = 0x1FFF;
pub const MBC5_BANK1_BASE:        u16 = 0x2000;
pub const MBC5_BANK1_END:         u16 = 0x2FFF;
pub const MBC5_BANK2_BASE:        u16 = 0x3000;
pub const MBC5_BANK2_END:         u16 = 0x3FFF;
pub const MBC5_RAMB_BASE:         u16 = 0x4000;
pub const MBC5_RAMB_END:          u16 = 0x5FFF;
pub const MBC5_RESERVED_BASE:     u16 = 0x6000;
pub const MBC5_RESERVED_END:      u16 = 0x7FFF;
pub const MBC5_RAM_ENABLE_VALUE:  u8 = 0x0A;
pub const MBC5_RAM_DISABLE_VALUE: u8 = 0x00;
```

`gbf-hw::mbc5` also exposes a `classify_mbc_write_address(addr) ->
Option<MbcRegisterClass>` helper and a `rom_bank_number(bank1, bank2)
-> u16` helper; F-A4 may use these in tests but its emit path uses the
explicit address constants directly. Note: `gbf-hw::mbc5::MbcRegisterClass`
(variants `Ramg | Bank1 | Bank2 | Ramb | Reserved`) is a distinct type
from `gbf-asm::effect::MbcRegisterClass` (variants
`RamEnable | RomBankLow | RomBankHigh | SramBank | Reserved`); the asm-
side type is the one F-A4's privilege/effect classification reasons
about, and `for_addr` translates from the hw classification into it.

F-A4 additionally consults `gbf-hw::target::CartridgeProfile` (which is
also closed) to detect MBC5+RUMBLE cartridges and reject them at
`ValidatedBankLeaseSpec::for_sram` construction time (§4.2
`RumbleCartProfileRejected`). The currently shipped `MbcType` enum has
only `Mbc5 | Mbc5Ram | Mbc5RamBattery` (rumble/sensor variants are
explicitly out of `gbf-hw`'s shipped subset, per the comment in
`cartridge_header.rs`), so today's `RumbleCartProfileRejected` path is
trivially unreachable; F-A4 still wires the rejection so a future
cart-feature bead that adds `Mbc5Rumble` cannot silently let bit 3 of
RAMB toggle the rumble motor on. A rumble-aware SRAM bank API is left
for that future feature.

### 9.2 `gbf-abi` dependency

F-A4 uses `InterruptPolicy` from `gbf-abi::interrupt`. T-A3.5 (`bd-30s`)
closed in May 2026; the type is final, with variants
`Enabled | ShortCriticalSection | Disabled` and an `ALL` array constant.
`gbf-abi::interrupt` additionally ships `LeaseId(u32)`, `SliceId(u32)`,
`OverlayId(u16)`, `RomWindowBinding`, `SramPageBinding`,
`ResourceLeaseKind` (with a `yield_safe()` predicate), and
`ResourceLease`. The `gbf-abi::interrupt::LeaseId` is **not** the same
nominal type as `gbf-asm::section::LeaseId`; both are
`#[repr(transparent)] u32` newtypes, but they live in different crates
and serialize/compare independently. F-A4 uses
`gbf-asm::section::LeaseId` for its lease bookkeeping (because the
`PreLayoutOp::BankLease(BankLeaseSpec)` wire shape carries it) and
treats the abi-side `ResourceLease` family as an *outward-facing
report shape* for ScheduleOracle / ReachabilityValidation rather than
something the lowering primitives consume directly. The two-LeaseId
situation is a known glossary item, not a F-A4 cleanup.

### 9.3 `no_std + alloc` capability

`gbf-runtime` ships as `no_std + alloc`-capable per planv0 rule 11. The
banking module uses `alloc::collections::BTreeMap` (for the safety table)
and `alloc::vec::Vec` (for the lowered fragment items). No `std` types
appear in the public surface. The `serde_json::Value` export from
`InterruptSafetyTable::export` is gated behind a `cfg(feature = "std")`
flag; the no_std export uses a tagged-tuple binary form via `bincode`.

### 9.4 `serde` boundary

Every public type that names a stable serializable shape (`MbcBankClass`,
`LeaseLifetime`, `ValidatedBankLeaseSpec` (transparent over the inner
`BankLeaseSpec`), `BankAbiViolation`, `BankingEmitError`,
`InterruptSafetyKind`, `InterruptSafety`, `InterruptSafetyTable`,
`InterruptSafetyError`, `SectionResidency`, `ReturnState` family) derives
`Serialize` and `Deserialize`. `BankGuard` is intentionally
non-serializable: a guard is a process-local linear token whose
`LeaseGeneration` is meaningful only relative to the originating
`Builder`.

### 9.5 Error handling style

Every public function that can fail returns `Result<T, E>` where `E` is
one of:

- `BankAbiViolation` — invariant violation; user error.
- `BankingEmitError` — emit-time failure inside a builder context; often
  wraps a `BuilderError` from `gbf-asm` or a `BankAbiViolation`.
- `InterruptSafetyError` — annotation-table conflict.
- `LoweringError` — for the lowering integration; surfaces via
  `LoweringDisposition::Error`.

No panics on bad input. `BankGuard::Drop` may include a
`#[cfg(debug_assertions)]` log line, but **never panics** in release;
the typed `BuilderError::UnreleasedBankGuard` comes from
`Builder::finish`'s pending-lease table check.

### 9.6 Performance targets

The banking module's hot path is the four-instruction MBC write. Targets:

- `lower_acquire_rom_bank`: ≤ 2 µs on host at `cargo test --release`.
- `byte_stable_emit_property`: ≤ 50 ms across 10,000 random
  `(bank, policy)` inputs.
- compiled output size: ≤ 20 bytes per acquire under
  `ShortCriticalSection`, ≤ 18 under `Disabled`.
- runtime cycle cost: ≤ 24 M-cycles per acquire under
  `ShortCriticalSection`, ≤ 22 under `Disabled` (per `cycle_model`).

## 10. Implementation order

The four child tasks already have a dependency DAG:

```text
T-A4.1 (types)  ─┬──> T-A4.3 (acquire/release) ──> T-A4.4 (IRQ scaffolding)
                 │                                 ▲
T-A4.2 (HRAM) ───┘─────────────────────────────────┘
```

F-A4 ships as a **single PR** containing all four child tasks
(T-A4.1 + T-A4.2 + T-A4.3 + T-A4.4). Implementation order within the
PR follows the DAG above:

1. T-A4.1 (types) and T-A4.2 (HRAM shadow constants + access helpers)
   land first as the type/shadow groundwork.
2. T-A4.3 (the four emit primitives, golden tests, no-raw-MBC grep)
   builds on the groundwork. This is the load-bearing portion of the
   PR.
3. T-A4.4 (IRQ-safety annotations + side-table + the production
   `PreLayoutOpLowering` impl) closes the PR.

Reviewers should walk the diff in this same order so they sign off on
the type/shadow groundwork before context-switching into the opcode
tables, and audit the opcode tables before reviewing the IRQ
scaffolding.

## 11. Testing strategy summary

| Area                           | Coverage                                                                                                        |
|--------------------------------|-----------------------------------------------------------------------------------------------------------------|
| Type invariants (T-A4.1)       | `ValidatedBankLeaseSpec` constructors; lifetime↔yield-safety; `BankGuard` linear-token semantics; `ReturnState` class-correctness; `KeepCurrentProof` unforgeability |
| HRAM shadow (T-A4.2)           | offset bounds (4-byte banking-owned region); LDH-only emission; banking-only writers `pub(crate)`; banking-shadow zero-init byte/cycle count exactly 10 / 14 |
| Acquire/release (T-A4.3)       | exact byte sequences for each (helper × policy) combo; `byte_stable_emit_property` (10k cases); privilege classification; cycle-cost-matches-Pan-Docs; provenance audit; SRAM-state tracking; switchable-residency rejection; SCS preconditions |
| IRQ scaffolding (T-A4.4)       | `check_lease_emission_legal(section, safety, residency)` truth table including residency dimension; annotation serde round-trip; safety-table iteration order; `ConflictingDeclaration` |
| Lowering (`PreLayoutOpLowering`) | `BankLease` lowers to acquire sequence; `BankRelease { lease_id, return_to }` lowers per `ReturnState`; `AssertBank` label-only by default, compare-and-trap as opt-in extension |
| Composite lowerer integration  | `LoweringDisposition::{Lowered, NotOwned, Error}`; non-banking ops return `NotOwned` (not swallowed errors)     |
| Determinism                    | full pipeline byte-stability across two builds                                                                  |
| Negative API                   | `Normal` section → `PrivilegeViolation`; `InterruptHandler` section → `IsrCannotAcquire`; switchable residency → `BankingPrimitiveNotFixedResident`; nested leases policy pinned |

A `gbf-runtime/tests/banking_integration.rs` exercises the full flow:
build a synthetic `Privileged`, fixed-bank-0 section with a
`BankLease(spec)` and a matching `BankRelease`, run
`BankingPreLayoutLowering`, then encode the resulting `LegalizedSection`
through `gbf-asm::encoder`, and assert the final byte sequence matches a
checked-in golden fixture. Fixed residency is part of the correctness
proof: a parallel test asserts that running the same flow inside a
`SwitchableRom`-residency section returns
`BankAbiViolation::BankingPrimitiveNotFixedResident`.

## 12. Resolved questions

1. **Where does the lease ABI live — gbf-asm or gbf-runtime?**
   *Resolution:* `gbf-asm::section` owns the serializable `BankLeaseSpec`
   wire shape. `gbf-runtime::banking` owns the *authoring* layer:
   `ValidatedBankLeaseSpec` (the only legal way to reach the emit path),
   `BankGuard` (a `Copy` linear token, not `&'a BankLease`),
   `LeaseLifetime`, and the lowering primitives. F-A4 cannot add inherent
   methods to foreign types per Rust's orphan rule, which is why
   constructors live on the wrapper rather than on `BankLeaseSpec`.

2. **Does the shadow update precede or follow the MBC register write?**
   *Resolution:* MBC write first, shadow second. The atomicity of the
   pair is enforced by `InterruptPolicy::ShortCriticalSection` (DI/EI
   bracket; SCS is non-nestable and assumes IME was enabled on entry,
   per §6.3) or by `Disabled` (IME already cleared); the order is then a
   property of the audit and convention, not a hardware necessity. We
   pick "MBC first" because it matches the natural reasoning ("I wrote
   the hardware, now I record what I did") and it's what hardware-test
   fixtures expect.

3. **Does `lower_release` emit literal bytes, or nothing?**
   *Resolution:* For yield-safe lifetimes (`Slice` / `ResumeWindow` /
   `Token`) and `ReturnState::KeepCurrent(_proof_)`, nothing — but a
   `Label` carrying release provenance is still emitted. `KeepCurrent`
   requires an unforgeable `KeepCurrentProof` token, constructible only
   inside `gbf-runtime::banking` (and, eventually, by the F-A5
   scheduler lowerer when it has a static guarantee that the next
   control-transfer boundary immediately reacquires). For ROM leases
   with `ReturnRomBank::Bank1`/`Manual(N)`, the release emits a normal
   `lower_acquire_rom_bank` to the target bank. For SRAM leases with
   `ReturnSramState::Disable`/`Bank(N)`, the release emits the matching
   disable or acquire sequence. **Open implementation point:** the
   F-A1-shipped `PreLayoutOp::BankRelease { lease_id }` does not carry
   a `ReturnState`, so the F-A4 PR must either extend the variant
   to `{ lease_id, return_to: ReturnState }` (preferred, makes the
   lowering pure-functional) or keep the wire shape and thread
   `ReturnState` through `BankingLoweringState`'s active-lease table.
   See §8.

4. **Should F-A4 use `LDH` or absolute `LD ($FFnn), A` for shadow writes?**
   *Resolution:* `LDH` exclusively. One byte shorter, one M-cycle
   cheaper, and the type system rejects non-high-memory targets through
   `gbf-asm::isa::HighDirectOffset`'s constructor.

5. **What's the policy for sections that hold a `BankLease` across a
   yield?** *Resolution:* `LeaseLifetime::Slice` cannot cross a yield —
   the builder/lowering state machine refuses `Yield` while a `Slice`
   lease is active, and the test
   `manual_lease_rejects_yield_while_held` pins this. `ResumeWindow`
   and `Token` are yield-safe with restoration: F-A4 declares the
   contract; F-A5 supplies the scheduler restoration machinery. Until
   F-A5's scheduler lands, F-A4's production lowering rejects
   `ResumeWindow` and `Token` at lowering time so they cannot silently
   appear in shipped builds without their counterpart restoration code.

6. **Does the `BankGuard` borrow from `Builder`?**
   *Resolution:* No. `BankGuard` carries `LeaseId + LeaseGeneration +
   SectionId` by value (it is `Copy`). The authoritative pending-lease
   table lives on `Builder`; `release_bank` consumes the guard and
   removes the entry; `Builder::finish` returns
   `BuilderError::UnreleasedBankGuard` if the table is non-empty. This
   keeps `&mut Builder` freely usable while a guard is alive.

## 13. Risks

1. **The "twenty-minute lockup" is the only failure mode that *only*
   shows up on hardware/long-soak emulator runs.** F-A4's local checks
   (privilege + residency + ISR-safety) plus Epic B's whole-program
   check are the proof; if either is wrong, the bug surfaces only after
   release. *Mitigation:* the workspace effect/provenance audit
   (`mbc_write_provenance_audit`) walks every emitted `Instr` and
   asserts every `StoreToMbcRegister` originated in
   `gbf-runtime::banking` from a `Privileged`, fixed-resident,
   non-ISR-reachable section. A grep is supplemental only.

2. **`InterruptPolicy::Enabled` rejection might be too strict.** Some
   future kernel might want a long-running `Slice` lifetime with
   `Enabled` IME for IRQ-driven progress reporting. *Mitigation:* the
   rejection is mechanical and easily relaxed once we have a concrete
   workload; right now no such workload exists, so the strictest policy
   is the right default.

3. **`SCS` non-nestability.** Because the LR35902 has no read-IME
   instruction, a `DI ... EI` bracket cannot restore prior IME state.
   If a caller already inside an interrupt-disabled region passes
   `ShortCriticalSection`, the trailing `EI` re-enables interrupts the
   caller intended to keep disabled. *Mitigation:* §6.3 makes the
   precondition explicit; `scs_policy_does_not_claim_to_restore_prior_ime`
   pins the docs/test contract; nested-region callers must pass
   `Disabled`. A future feature could add an IME-tracking variant if
   measured workloads need it.

4. **Shadow drift across reset / power-cut.** The HRAM shadow is
   volatile; on a power cycle it returns to whatever the boot code
   initializes it to. If `boot_init` zeroes the shadow but the previous
   boot left the *hardware* with a non-bank-1 ROM bank selected, the
   shadow lies until the first acquire. *Mitigation:* the boot code
   (F-A5 owns this) issues `lease_rom_switchable(1, …)` (lowered to
   `lower_acquire_rom_bank(1, InterruptPolicy::Disabled)`) immediately
   after `lower_banking_shadow_zero_init`, before any other code runs.
   F-A5 must include this — the F-A5 RFC will name it as a hard
   requirement; the F-A4 review packet checks it via a fixture test
   that imports the F-A5 boot section and asserts the first MBC write
   after shadow init is `LD ($2000), 0x01`.

5. **`gbf-hw` constants drift (mostly retired).** T-A2.3 (`bd-121`)
   closed before F-A4 starts, so `gbf-hw::mbc5` is now the single home
   for the MBC5 address constants and F-A4 imports them directly — no
   `gbf-runtime::banking::mbc5_constants` shim is needed. The remaining
   drift surface is the *naming conventions* between
   `gbf-hw::mbc5::MbcRegisterClass` (`Ramg | Bank1 | Bank2 | Ramb |
   Reserved`) and `gbf-asm::effect::MbcRegisterClass` (`RamEnable |
   RomBankLow | RomBankHigh | SramBank | Reserved`); the asm-side type
   already exposes a `for_addr` helper that bridges them.
   *Mitigation:* keep the `banking::mbc5_constants_match_gbf_hw` test
   (now unconditional) so any future renaming or value change in
   `gbf-hw::mbc5` is caught at unit-test time, and pin the asm-side
   `for_addr` mapping with a parallel test in
   `banking::asm_mbc_register_class_matches_hw`.

6. **`LDH` form vs absolute form drift.** If a future opcode-table
   refactor in `gbf-asm::isa` changes `Ld8HighDirectFromA`'s encoding,
   F-A4's golden tests catch the drift on the next CI run, but the
   landed code is wrong until then. *Mitigation:* the cycle-cost-matches
   test (which sums `cycle_model::cycle_cost(Instr)` and asserts against
   the per-helper M-cycle tables in §6.2) and the encoded-byte golden
   fixtures double-cover this.

7. **`ReachabilityValidation` is much later.** Until Epic B Stage 12
   lands, the only invariant F-A4 enforces is local. ISR sections that
   transitively reach a banking primitive through a long call chain will
   compile and run; the bug only manifests when the IRQ fires while the
   chain is on the stack. *Mitigation:* F-A5's IRQ vector dispatch table
   itself is a small fixed shape that F-A4 can audit-by-eye; the
   short-term risk surface is small. Epic B closes the rest.

8. **Rumble-cart misclassification.** If a future cartridge profile
   silently resolves to MBC5+RUMBLE without `for_sram` rejecting it,
   SRAM bank values 8–15 would toggle the rumble motor instead of
   selecting an upper SRAM bank. *Mitigation:*
   `RumbleCartProfileRejected` is constructed from the cartridge
   profile, not from a per-call argument; the test
   `mbc5_rumble_profile_rejected_for_sram_banks` pins it.

## 14. Claim-to-gate matrix (closure-style)

The closure skill (`.agents/skills/asm-bead-closure/SKILL.md` for asm-aligned
beads) requires this for non-trivial runtime beads. Pre-emptive matrix for
F-A4 closure:

| Claim                                                                              | Gating test / artifact                                                                                                |
|------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------|
| `ValidatedBankLeaseSpec::for_rom_switchable` rejects bank 0 by ABI policy          | `banking::rom_bank_zero_reserved_by_abi`                                                                              |
| `ValidatedBankLeaseSpec::for_rom_switchable` rejects banks > 511                    | `banking::lease_spec_invariants_rom`                                                                                  |
| `ValidatedBankLeaseSpec::for_sram` rejects banks > 15                              | `banking::lease_spec_invariants_sram`                                                                                 |
| `LeaseLifetime` yield-safety table is exact                                        | `banking::lifetime_yield_safety`                                                                                      |
| `BankGuard` carries no `&Builder` borrow                                            | `banking::bank_guard_does_not_borrow_builder`                                                                         |
| Unreleased `BankGuard` fails `Builder::finish`                                      | `banking::bank_guard_drop_without_release` (BuilderError::UnreleasedBankGuard from pending-table check, not from Drop)|
| `BankGuard::Drop` does not panic in release                                          | `banking::bank_guard_drop_does_not_panic_in_release`                                                                  |
| `KeepCurrentProof` cannot be constructed outside `gbf-runtime::banking`             | `banking::keep_current_proof_unforgeable`                                                                             |
| `ReturnState` class-correctness                                                      | `banking::return_state_class_correct`                                                                                 |
| Rumble cartridges are rejected until supported                                       | `banking::mbc5_rumble_profile_rejected_for_sram_banks`                                                                |
| HRAM banking shadow region is exactly 4 bytes at `$FF80..=$FF83`                    | `banking::hram_banking_shadow_region_size` + `banking::hram_shadow_offsets_within_banking_range`                      |
| Shadow reads/writes are bank-agnostic LDH                                            | `banking::store_bank_shadow_emits_ldh` + `banking::banking_shadow_zero_init_machine_effects` (every effect is StoreToHram) |
| Banking shadow writers are `pub(crate)` only                                         | `banking::store_bank_shadow_helpers_are_pub_crate`                                                                    |
| `lower_banking_shadow_zero_init` is byte-stable                                      | `banking::banking_shadow_zero_init_byte_stable`                                                                       |
| `lower_banking_shadow_zero_init` is exactly 10 bytes / 14 M-cycles                  | `banking::banking_shadow_zero_init_byte_and_cycle_count` + `banking::shadow_zero_init_byte_and_cycle_count`           |
| `lower_enable_sram` writes `$0A` to RAMG (`$0000`) and shadow `sram_enabled`        | `banking::lower_enable_sram_byte_sequence`                                                                            |
| `lower_disable_sram` writes `$00` to RAMG and shadow                                 | `banking::lower_disable_sram_byte_sequence`                                                                           |
| `lower_acquire_rom_bank` produces a 20-byte sequence under `ShortCriticalSection`    | `banking::lower_acquire_rom_bank_byte_sequence_bank3` + `banking::lower_acquire_rom_bank_byte_sequence_bank256`       |
| `lower_acquire_rom_bank` produces an 18-byte sequence under `Disabled`               | `banking::lower_acquire_rom_bank_disabled_policy`                                                                     |
| `lower_acquire_rom_bank` rejects `InterruptPolicy::Enabled`                          | `banking::lower_acquire_rom_bank_rejects_enabled_policy`                                                              |
| `lower_acquire_rom_bank` rejects bank 0 by ABI policy                                | `banking::lower_acquire_rom_bank_rejects_bank_zero`                                                                   |
| `lower_acquire_sram_bank` writes RAMB and shadow                                      | `banking::lower_acquire_sram_bank_byte_sequence`                                                                      |
| SRAM acquire rejected when `BankingLoweringState` says SRAM disabled                  | `banking::lower_acquire_sram_bank_rejects_disabled`                                                                   |
| `lower_release(KeepCurrent, yield_safe)` emits zero `Instr`s but keeps a Label       | `banking::lower_release_keep_current_is_label_only`                                                                   |
| `lower_release(Bank1, …)` matches `lower_acquire_rom_bank(1, …)` byte-for-byte       | `banking::lower_release_to_bank1_is_acquire_one`                                                                      |
| `lower_release` rejects class-mismatched `ReturnState`                                | `banking::lower_release_class_correct`                                                                                |
| Banking MBC-write sequences never execute from `$4000..=$7FFF`                       | `banking::banking_emit_rejects_switchable_residency` + `banking::golden_listings_never_start_in_4000_7fff`            |
| DI/EI text and cycle model use IME, not IF                                            | `banking::interrupt_policy_docs_and_cycle_model`                                                                      |
| SCS does not restore prior IME and is non-nestable                                    | `banking::scs_policy_does_not_claim_to_restore_prior_ime` + `banking::scs_policy_rejects_nested_or_unknown_ime`       |
| Every emitted MBC-touching `Instr` classifies to `PrivilegeClass::Privileged`        | `banking::all_emits_are_privileged`                                                                                   |
| Shadow update follows the corresponding register write                                | `banking::shadow_update_after_register_write`                                                                         |
| Banking shadow bytes cannot be written outside `banking.rs`                          | `banking::no_bank_shadow_writes_outside_banking_rs`                                                                   |
| Raw MBC writes outside banking are caught semantically                                | `banking::mbc_write_provenance_audit` (effect/provenance audit; grep is supplemental)                                 |
| `Normal` section calling an emit helper fails at builder time                         | `banking::normal_section_emit_rejected_with_privilege_violation`                                                      |
| `InterruptHandler` section calling an emit helper fails at builder time               | `banking::isr_section_cannot_lease`                                                                                   |
| `Privileged` non-ISR fixed-resident section is allowed                                | `banking::privileged_non_isr_fixed_section_can_lease`                                                                 |
| Switchable-residency Privileged section is rejected                                    | `banking::switchable_residency_cannot_lease`                                                                          |
| `InterruptSafetyTable` round-trips through serde                                      | `banking::annotation_serializable_round_trip`                                                                         |
| `InterruptSafetyTable` iteration is BTreeMap-stable                                    | `banking::annotation_table_lookup_stable`                                                                             |
| `InterruptSafetyTable::declare` rejects conflicting kinds                              | `banking::declare_conflicting_kind_errors`                                                                            |
| `BankingPreLayoutLowering` lowers `BankLease` to the acquire sequence                 | `banking::lowering_bank_lease_emits_acquire_sequence`                                                                 |
| `BankingPreLayoutLowering` lowers `BankRelease { lease_id, return_to }` per `ReturnState` | `banking::lowering_bank_release_per_return_state`                                                                  |
| `BankingPreLayoutLowering` lowers `AssertBank` to label-only by default                | `banking::lowering_assert_bank_label_only`                                                                            |
| `BankingPreLayoutLowering` lowers `AssertBank` to compare-and-trap when feature enabled| `banking::lowering_assert_bank_emits_compare` (gated)                                                                 |
| `BankingPreLayoutLowering::lower(Yield/TraceProbe)` returns `LoweringDisposition::NotOwned` | `banking::lowering_rejects_non_banking_ops`                                                                       |
| `LoweringDisposition::Error` is not silently treated as `NotOwned`                     | `banking::composite_lowerer_does_not_swallow_error`                                                                   |
| Per-helper cycle cost matches Pan Docs sum                                             | `banking::cycle_cost_matches_pan_docs`                                                                                |
| `byte_stable_emit_property` over 10k random inputs                                     | `banking::byte_stable_emit_property` (proptest)                                                                       |
| Nested leases are either rejected or restored stack-wise                               | `banking::nested_lease_policy_pinned` + `banking::nested_rom_lease_rejected_or_stack_restored` + `banking::nested_sram_lease_rejected_or_stack_restored` |
| Manual lease rejects yields while held                                                  | `banking::manual_lease_rejects_yield_while_held`                                                                      |
| SRAM release semantics are class-correct                                                | `banking::sram_release_disable_or_restore`                                                                            |
| Full integration: typed `BankLease` → lowered → encoded → golden bytes (fixed-bank-0)  | `gbf-runtime/tests/banking_integration.rs::full_acquire_release_round_trip`                                           |
| `mbc5_constants_match_gbf_hw` (drift detector against `gbf-hw::mbc5`)                  | `banking::mbc5_constants_match_gbf_hw` (now unconditional — T-A2.3 closed; the earlier `#[cfg(feature = "gbf-hw-constants")]` gate is no longer needed) |
| No new `unsafe`                                                                         | `grep -r "unsafe" gbf-runtime/src/banking.rs` returns zero hits                                                       |

## 15. Review packet requirements

The F-A4 review packet is a **first-class artifact** the engineer must
check into the PR (not an informal note). Its job is to let a
reviewer answer four questions quickly:

1. **Is the implementation correct?**
2. **Is the implementation clear and maintainable?**
3. **Are the riskiest invariants actually proved by tests, types, or
   generated artifacts?**
4. **Can I reproduce every claimed output locally?**

This section enumerates the items the engineer must include in the
packet (under `docs/review/f-a4/`, with reproducibility scripts under
`scripts/review/f-a4/`). For each item, the listed properties are
gates — the packet is not ready for review until every property holds.
The packet should not duplicate this RFC; it should be the executable,
reproducible evidence that what this RFC describes is also what
landed.

The most important rule: **the packet is reproducible**. The engineer
must include a `verify-packet.sh` script under `scripts/review/f-a4/`
that, when run on a fresh checkout, runs the relevant tests,
regenerates every `.bin` / `.lst` file from the typed emit helpers,
recomputes hashes, optionally regenerates diagrams, and fails loudly
if any claimed artifact is stale.

### 15.1 Reviewer entry point (README)

The engineer must include a packet README that serves as the
reviewer's landing page. The README must:

- identify the RFC, branch, commit, and engineer for the change.
- name the single PR (F-A4 ships as one PR per §10) and list the
  child tasks it closes (T-A4.1, T-A4.2, T-A4.3, T-A4.4).
- list every item still deferred (whole-program
  `ReachabilityValidation`, long critical sections / IRQ-deferral,
  `Yield`/`TraceProbe` lowerings, rumble-bit support, F-A5 boot
  nucleus integration), with the owning bead/feature for each.
- include a one-page executive summary that names the highest-risk
  implementation points: the four-instruction MBC write sequence
  under `InterruptPolicy::ShortCriticalSection` (DI/EI bracketing
  over IME, shadow-update ordering, fixed-residency precondition,
  SCS non-nestability); the privilege+fixed-residency+non-ISR
  enforcement at builder time and audit time; HRAM banking-shadow
  layout exclusivity; `InterruptSafetyKind` substrate for Epic B's
  `ReachabilityValidation`; the no-raw-MBC effect/provenance audit;
  determinism of every emit helper; ABI-policy bank-0 rejection.
- list the exact reviewer commands a reader should be able to paste
  to reproduce the packet (the relevant `cargo test` invocations,
  the `banking_demo` example invocation, and `verify-packet.sh`).

### 15.2 Scope document

The engineer must include a scope document that separates what is
implemented, what is intentionally deferred, and what is explicitly
not claimed. The scope document must:

- enumerate every in-scope item the implementation touches: the new
  `gbf-runtime/src/banking.rs` content (the type backbone listed in
  §3 and §4); the HRAM banking-shadow constants, `ShadowRegisters`,
  shadow read/write helpers, and `lower_banking_shadow_zero_init`;
  the `pub(crate)` lowering primitives and the `pub` user-facing
  API; the `InterruptSafetyKind` substrate and
  `check_lease_emission_legal`; `BankingPreLayoutLowering`; the
  per-helper byte-sequence golden artifacts; the
  `mbc_write_provenance_audit`; and the `banking_demo` example plus
  the fixed-bank-0 integration test.
- enumerate every out-of-scope item with one entry per deferred
  topic, formatted as: deferred item, why not in F-A4, the owning
  feature/bead, and the F-A4-side guard that prevents accidental
  dependence on the deferred item.

### 15.3 Review order guide

The engineer must include a review-order guide that tells the reviewer
where to start, what to read deeply, and what to skim. The guide must:

- structure the review as a Pass 0 (sanity + reproduction; reviewer
  runs `verify-packet.sh` and confirms tests pass and goldens match)
  followed by per-task passes corresponding to §10's intra-PR
  implementation order: types, HRAM shadow, acquire/release
  primitives, IRQ scaffolding, lowering integration.
- for each pass, name the files the reviewer must read deeply, the
  surrounding context they may skim, and the questions they should be
  able to answer afterwards. The questions must include at least:
  - **Types pass**: is `ValidatedBankLeaseSpec`'s constructor
    invariant tight enough? Is bank-0 rejection clearly an ABI
    policy, not a hardware claim? Does `BankGuard` avoid borrowing
    `&Builder`? Is the `LeaseLifetime` ↔ yield-safety table
    exhaustive? Is `KeepCurrentProof` unforgeable from outside the
    crate? Is `ReturnState` class-correct?
  - **Shadow pass**: is the banking-owned region exactly 4 bytes
    (`$FF80..=$FF83`)? Is every shadow access an `LDH`? Are shadow
    writers `pub(crate)` and only the read API public? Does
    `lower_banking_shadow_zero_init` produce exactly 10 bytes / 14
    M-cycles? Does F-A4 leave HRAM bytes ≥ `$FF84` to F-A5?
  - **Acquire/release pass**: does every MBC store classify
    `Privileged`? Does `check_lease_emission_legal` reject
    `SwitchableRom` residency? Does the DI/EI bracket talk about IME
    consistently? Is SCS documented as non-nestable with an
    IME-enabled-on-entry precondition? Does the BANK1-then-BANK2
    order appear as a tested convention rather than a hardware
    requirement? Do the `.bin` byte sequences match the documented
    sizes (20 bytes SCS / 18 bytes Disabled for ROM acquire)? Are
    listing addresses always in fixed Bank 0 or HRAM? Does
    `lower_release(KeepCurrent, _proof_)` emit zero `Instr`s? Does
    `mbc_write_provenance_audit` cover every callsite? Are SRAM
    acquire byte/cycle counts consistent between docs and goldens?
  - **IRQ scaffolding pass**: does `check_lease_emission_legal` take
    `(section, safety, residency)`? Does the truth table reject
    ISR-reachable + `Privileged` for MBC writes? Does
    `InterruptSafetyTable` serialize stably? Does `declare` reject
    conflicting kinds? Is the `ReachabilityValidation` handoff
    documented well enough for Epic B to consume?
  - **Lowering pass**: does `BankingPreLayoutLowering` return
    `LoweringDisposition::NotOwned` for `Yield`/`TraceProbe` rather
    than `Err`? Does `AssertBank` default to label-only with
    compare-and-trap as a feature-gated opt-in? Does
    `PreLayoutOp::BankRelease` carry `{ lease_id, return_to }` as the
    RFC claims (or use an active-lease table)? Does the integration
    test produce the same bytes as the unit goldens, with fixed-bank-0
    residency?

### 15.4 Safe-to-ignore / must-review guide

The engineer must include a safe-to-ignore / must-review guide that
classifies the diff into four buckets. The guide must:

- mark as **must review deeply** the `mod emit` (especially the
  four-instruction sequence), `mod isr`, and `mod lowering` content
  in `gbf-runtime/src/banking.rs`, every byte-golden test, the
  generated `.bin`/`.lst` artifacts, and the correctness dossier.
- mark as **must review for API-boundary changes** the type backbone
  (`ValidatedBankLeaseSpec`, `BankGuard`, `ReturnState`,
  `KeepCurrentProof`), the public/`pub(crate)` split in `mod shadow`,
  the user-facing API helpers, the public re-exports in
  `gbf-runtime/src/lib.rs`, and the new dependencies in
  `gbf-runtime/Cargo.toml`.
- mark as **can skim** the rendered SVG diagrams (when the Mermaid
  sources have been reviewed and the verify script regenerates
  cleanly), `.bin` artifacts after the first version (verifiable by
  hash plus focused diff), and `Cargo.lock` subtrees beyond the
  changed dependencies (covered by the dependency report).
- mark as **not safe to ignore** the opcode bytes inside the four
  emit primitives, the DI/EI bracketing under each `InterruptPolicy`
  (with IME wording), SCS preconditions, shadow-update ordering
  relative to MBC writes, the `$FF80..=$FF83` shadow-byte offsets,
  privilege classification of every emitted `Instr`, the
  `SectionResidency` check on every banking emit, any code path that
  emits an MBC store outside the audited four helpers (audit, not
  grep), any path that constructs `KeepCurrentProof` outside
  `gbf-runtime::banking`, and any path that writes the banking
  shadow bytes outside `banking.rs`.

### 15.5 Diff map

The engineer must include a diff map: a table with one row per
changed file. Each row must include the change type (new / replaces
stub / deps / unchanged), risk level (Low / Medium / High), a
one-line "why reviewer should care", and the main test(s) that
exercise the change. The map must also classify each file as one of:
critical correctness, test only, generated fixture, docs only, or API
boundary.

### 15.6 Architecture guide

The engineer must include an architecture guide that explains the
module in reviewer-oriented terms. The guide must include diagrams
covering, at minimum:

- the authoring flow from `Builder`'s lease/release helpers through
  `BankGuard` (a host-side `Copy` token) and the pending-lease table
  to `pre_layout_ops`, `BankingPreLayoutLowering`, the resulting
  `LoweredFragment`, and the F-A1 `LoweredSection`, including the
  `BuilderError::UnreleasedBankGuard` path on `Builder::finish`.
- the bank-lease lifecycle: held → released; held → auto-restored at
  a slice/resume boundary (for `Slice`/`ResumeWindow`/`Token`
  lifetimes); held → `UnreleasedBankGuard` at finish; held + yield
  → builder/lowering rejection (for `Manual` lifetime).
- the privilege ↔ ISR-safety ↔ residency matrix, naming the
  rejection variants `check_lease_emission_legal` returns
  (`IsrCannotAcquire`, `SectionNotPrivileged`,
  `BankingPrimitiveNotFixedResident`).
- the acquire byte sequence under `ShortCriticalSection` from a
  fixed-bank-0 / HRAM section, naming each `Instr` and the
  shadow-update ordering.
- the HRAM banking-shadow layout, naming each of the four
  banking-owned bytes (`+0` rom_bank lo, `+1` rom_bank hi, `+2`
  sram_bank, `+3` sram_enabled) and explicitly handing off
  `$FF84..=$FFFE` to F-A5.
- the lowering pipeline, naming the `BankLease`, `BankRelease`,
  `AssertBank`, `Yield/TraceProbe` (returning
  `LoweringDisposition::NotOwned`), and validation-failure
  (`LoweringDisposition::Error`) branches of
  `BankingPreLayoutLowering`.

Every diagram must be checked in as both a `.mmd` source and a
rendered `.svg`, and the verify script must regenerate them.

### 15.7 Correctness dossier

The engineer must include a correctness dossier — a rigorous,
proof-oriented document that explains *why* the implementation is
safe. The dossier must contain a separate writeup for each of the
following proofs, and each writeup must cite the test, type
invariant, or generated artifact that proves it (English alone is
not enough):

- **Privilege + residency proof**: explain how every emitted
  `MachineEffect::StoreToMbcRegister` is gated through
  `Builder::validate_effect`, and how `check_lease_emission_legal`
  additionally rejects `InterruptHandler` sections, non-`Privileged`
  sections, and non-`FixedRom0`/`Hram` residencies; explain why
  switchable-residency banking writes are unsafe even from a
  `Privileged` ISR-unreachable section.
- **No-raw-MBC proof (semantic, not grep)**: enumerate every legal
  emit path (the four `lower_*` helpers plus `lower_release`'s
  dispatch); state every impossible path (no `SectionItem::Raw`, no
  `DataBlock::Bytes`); document that the authoritative gate is the
  `mbc_write_provenance_audit` (asserts module provenance,
  privilege, residency, ISR-unreachability, and banking-lowerer
  proof for every emitted MBC store), with the supplemental grep
  named explicitly as supplemental.
- **HRAM banking-shadow exclusivity proof**: state that the
  banking module owns exactly the four bytes `$FF80..=$FF83`, that
  shadow writers are `pub(crate)`, that the only public shadow
  operation is a read, and that bytes `≥ $FF84` are owned by F-A5.
- **Critical-section atomicity proof**: explain the DI/EI bracket
  under `ShortCriticalSection` (IME wording, no fault-or-yield
  inside the bracket, non-nestability with the IME-enabled-on-entry
  precondition) and the rejection of `InterruptPolicy::Enabled`.
- **Cycle-cost proof**: explain that every per-helper M-cycle count
  in §5.4 / §6.2 is verified by summing
  `cycle_model::cycle_cost(Instr)` over the emitted stream against a
  hardcoded expected total.
- **IRQ-safety scaffolding proof**: enumerate the three cases the
  local `check_lease_emission_legal` covers and name the fourth
  case (cross-section ISR-reachability) as declared via
  `mark_isr_*` / `InterruptSafetyTable` and proven by Epic B's
  `ReachabilityValidation` pass (the F-A4 side is substrate, not
  the global proof).

For each proof, the dossier must list the specific test names that
gate it (e.g., `banking::all_emits_are_privileged`,
`banking::mbc_write_provenance_audit`,
`banking::hram_banking_shadow_region_size`,
`banking::lower_acquire_rom_bank_byte_sequence_bank3`,
`banking::scs_policy_rejects_nested_or_unknown_ime`,
`banking::cycle_cost_matches_pan_docs`,
`banking::isr_section_cannot_lease`, etc.) so a reviewer can
re-run each gate.

### 15.8 Claim-to-gate matrix

The engineer must include a copy of the claim-to-gate matrix from
§14 of this RFC so the packet stands alone, with one row per
acceptance claim and one or more gates per row (test name, type
invariant, or generated artifact). Every row must have at least one
gate populated.

### 15.9 Test coverage report

The engineer must include a test coverage report. **Risk coverage**
matters more than raw line percentage. The report must include:

- the exact test commands a reviewer can paste to reproduce the
  output (per-crate `cargo test`, full-workspace `cargo test`, and
  the `banking_demo` example invocation), and a captured
  `test-output.txt` artifact under `artifacts/`.
- a coverage-by-risk-area table that names the required coverage
  for at least: acquire byte sequences (per (helper × policy ×
  representative bank)), privilege + residency gating (with negative
  tests for Normal / ISR / switchable / WRAM / mixed-effect
  sections), the HRAM banking shadow, the lifetime × policy table,
  the `InterruptSafetyKind` × `PrivilegeClass` × residency matrix,
  nested leases, lowering wiring (every `PreLayoutOp` variant plus
  `LoweringDisposition::NotOwned` for non-banking ops and a
  non-swallowed `Error`), determinism (byte-stable property + the
  integration round-trip), cross-module drift
  (`mbc5_constants_match_gbf_hw`), the no-raw-MBC semantic audit,
  banking-shadow exclusivity, rumble-cart safety, and the SCS
  preconditions.
- a per-property-test entry for every proptest, with: test name,
  generator shape, number of cases, deterministic seed, the
  reproduction command (e.g. `PROPTEST_CASES=1 PROPTEST_SEED=<seed>
  cargo test -p gbf-runtime -- <name>`), and the invariant the test
  proves.
- a snapshot/golden update policy stating that a golden artifact may
  only be updated by a PR section listing old hash, new hash, the
  `Instr` that changed (with rationale: fix or refactor), a focused
  diff for the `.lst`, and a check against the cycle-cost table in
  §6.2.
- a mutational/adversarial test list covering at minimum: bank 0
  acquire via `for_rom_switchable`; bank 512 acquire; SRAM bank 16;
  `ManualLifetime` in a yielding section; `InterruptHandler` /
  `Normal` / switchable-residency / WRAM-residency sections calling
  banking lowering; `BankGuard` dropped without release; `BankGuard`
  released twice; nested ROM and SRAM leases; `lower_acquire_sram_bank`
  without prior `lower_enable_sram`; `ReturnRomBank::Manual(0)`;
  `ReturnState` class mismatch; `KeepCurrentProof` forged outside
  the crate; MBC5+RUMBLE cartridge profile; nested SCS /
  SCS-in-IME-disabled-context; `mbc5` constant drift; and a listing
  whose first byte address falls in `$4000..=$7FFF`.

### 15.10 Reproducibility report

The engineer must include a reproducibility report that lets a
reviewer reproduce every claimed output locally. The report must:

- pin the exact environment: `rustc` version, `cargo` version,
  workspace commit, OS, target triple, features used, and any
  environment variables required.
- include the exact command sequence a reviewer should run
  (toolchain version checks, the relevant `cargo test`, the
  `banking_demo` example, and `sha256sum` over each generated
  artifact).
- include a hash manifest enumerating each generated artifact (the
  per-helper `acquire_*.bin` files for both representative ROM
  banks and the disabled-policy variant; `enable_sram.bin`,
  `disable_sram.bin`, `acquire_sram_bank_2.bin`,
  `release_to_bank1.bin`, `banking_shadow_zero_init.bin`) with its
  SHA-256 and exact byte count, where the byte counts agree with
  §5.4 / §6.2 / §6.4.
- include a per-helper byte report listing the exact `Instr`
  sequence each helper emits and the resulting byte/M-cycle totals,
  with the SRAM-acquire shape pinned to the choice made in §6.4 and
  propagated to every test and golden.

The verify script must regenerate this report; the engineer must
confirm before review that the chosen SRAM-acquire byte count
matches across §6, §15.10, and the golden artifact.

### 15.11 Generated artifacts guide

The engineer must include a generated-artifacts guide with one entry
per artifact. Each entry must state the artifact's purpose, the
specific claims it proves, the script that regenerates it, and the
review method (e.g., "run verify-packet.sh, inspect the paired
`.lst`, verify against §6.2 cycle table").

The guide must include entries for, at minimum:

- each `acquire_*.bin` (per-helper byte-level proof for a fixed
  input, claiming exact byte count, DI opcode first under SCS,
  Privileged classification of every MBC store, byte-stability).
- `banking_shadow_zero_init.bin` (boot-time banking-shadow zeroing
  of the 4 banking-owned bytes only — exactly 10 bytes / 14
  M-cycles, 4 `LDH` stores covering `$FF80..=$FF83`, no MBC store,
  F-A5 owning the rest of HRAM separately).
- `release_to_bank1.bin` (identity check: byte-equal to
  `acquire_rom_bank_1_short.bin`).
- the captured `test-output.txt`, `bench-output.txt`,
  `cargo-tree.txt`, `cargo-deny.txt`, and `coverage-summary.txt`
  produced by the verify script.

### 15.12 Benchmarks and performance impact

The engineer must include a benchmarks document showing that F-A4's
emit helpers are comfortably fast and that the runtime cost of the
emitted code matches the documented envelopes. The document must:

- include microbenchmarks covering at minimum: `emit_acquire_rom_bank`
  across all `1..=511` banks; `emit_acquire_sram_bank` across all
  `0..=15` banks; `emit_enable_sram`/`emit_disable_sram` (constant
  time); `emit_shadow_zero_init`; `BankingPreLayoutLowering::lower`
  at 100 / 1000 / 10000 `PreLayoutOps`; `byte_stable_emit_property`
  across 10k random inputs; the full `banking_integration`
  round-trip.
- include a results table with input, wall time, allocations, and
  notes per row.
- explicitly state: no new parallelism; no nondeterministic
  iteration; no `SystemTime`/`rand`/env dependence; no `unsafe`
  introduced; no allocations beyond the `LoweredFragment` `Vec`.
- pin the runtime-side per-helper byte/cycle envelopes
  (per-ROM-acquire under SCS / `Disabled`; per-enable-sram and
  per-disable-sram under SCS; per-banking-shadow-zero-init at boot)
  so that F-A5's slice-budget arithmetic and Epic B's
  `ScheduleCostAnalysis` can consume them.

### 15.13 Dependency report

The engineer must include a dependency report with one row per
new/changed dependency, stating: why it is needed, the feature
flags used, alternatives considered, license, and risk level. The
report must cover at minimum the `gbf-*` workspace deps F-A4
consumes (`gbf-asm`, `gbf-abi`, `gbf-foundation`, `gbf-hw`) plus
`serde`, `thiserror`, `proptest` (dev), and the optional `bincode`
dependency for the `no_std` `InterruptSafetyTable` export.

The report must also include the regenerated `cargo tree`,
`cargo tree -e features`, and `cargo deny check` outputs (stored as
artifacts) and explicit assertions that no dependency is used for
opcode semantics, nondeterministic ordering, runtime code
generation, or `unsafe` code.

### 15.14 Known debt ledger

The engineer must include a brutally explicit known-debt ledger
listing every shortcut, follow-up, or "good enough for F-A4 but
rework in F-A5/Epic B" decision the engineer made during
implementation. Each entry must include: the debt item, why it is
acceptable for F-A4, the risk if forgotten, the owner / follow-up
feature, and the guardrail in the current implementation that
prevents accidental dependence.

The ledger must include at minimum the following entries (each in
the format above):

- whole-program `ReachabilityValidation` absent (cross-section ISR
  reachability beyond the local checks; owned by Epic B Stage 12 /
  `bd-18d`; guardrails: `InterruptSafetyTable` export plus
  `mbc_write_provenance_audit`).
- `gbf-hw::mbc5` constant naming asymmetry (the
  `Ramg|Bank1|Bank2|Ramb|Reserved` vs
  `RamEnable|RomBankLow|RomBankHigh|SramBank|Reserved` overlap
  bridged through `effect::MbcRegisterClass::for_addr`; guardrails:
  `mbc5_constants_match_gbf_hw` and
  `asm_mbc_register_class_matches_hw`).
- F-A5 boot integration declared but not built (the `lower_banking_
  shadow_zero_init` contract for F-A5's `boot_init`; guardrail:
  F-A4's integration test asserts the synthetic F-A5-shaped boot
  call sequence).
- `Yield`/`TraceProbe` lowerings absent (F-A4 returns
  `LoweringDisposition::NotOwned`; guardrail:
  `banking::lowering_rejects_non_banking_ops`).
- `InterruptPolicy::Enabled` rejection is mechanical (could become
  `Warn` later if a workload justifies it; guardrail: rejection is
  a typed `BankingEmitError` easy to relax).

### 15.15 Out-of-scope ledger

The engineer must include an out-of-scope ledger separate from the
known-debt ledger. Each entry must include: the deferred item, the
owner (which feature/bead picks it up), and the F-A4 seam that lets
that work plug in cleanly. The ledger must cover at minimum: a live
emulator boot test of an F-A5 nucleus; long critical sections /
IRQ-deferral mechanisms; `Yield`/`TraceProbe` lowerings; rumble
support on MBC5 RAMB bit 3; CGB/GBC features; and stage-cache
integration.

### 15.16 API-change guide

The engineer must include an API-change guide enumerating every new
or changed `pub` and `pub(crate)` item in `gbf_runtime::banking`,
with intent (new / extended / replaced / deleted) and rationale for
visibility. The guide must:

- include the public surface (the type backbone, HRAM banking-shadow
  constants and `ShadowRegisters`, the read-only shadow helper, the
  `lower_banking_shadow_zero_init` boot-time helper, the user-facing
  lease/release helpers, the `mark_isr_*` and
  `check_lease_emission_legal` helpers, and
  `BankingPreLayoutLowering`).
- include the `pub(crate)` lowering internals (the four raw
  shadow/MBC emit helpers and `BankingLoweringState`), explicitly
  marked as not part of the public ABI to enforce the audit
  surface.
- for each item, state: purpose, stability expectation,
  serialization behavior, main invariants, and main tests.

### 15.17 Error-shape report

The engineer must include an error-shape report covering every
public error enum (`BankAbiViolation`, `BankingEmitError`,
`InterruptSafetyError`). For each enum, the report must enumerate
every variant the implementation produces (with all fields), and
for each variant: the invariant the variant protects, the tests
exercising it, and the reproduction state the variant carries.

### 15.18 Reviewer checklist

The engineer must include a concise, actionable reviewer checklist
(literal `[ ]` items) covering, at minimum:

- **Reproducibility**: `verify-packet.sh` passes; every artifact
  hash matches; `cargo test -p gbf-runtime -- banking` passes.
- **Type backbone**: `ValidatedBankLeaseSpec` rejects rom bank 0
  (ABI policy), `>511`, and SRAM `>15`; rom-bank-0 rejection is
  documented as an ABI policy, not a hardware claim;
  `LeaseLifetime` ↔ yield-safety table exhaustive; `BankGuard`
  doesn't borrow `&Builder`; `BankGuard` drop doesn't panic in
  release; `Builder::finish` surfaces `UnreleasedBankGuard`;
  `KeepCurrentProof` unforgeable from outside the crate;
  `ReturnState` class-correct; rumble cartridge profile rejected
  for SRAM; `BankAbiViolation` exhaustive.
- **HRAM shadow**: banking-owned region exactly `$FF80..=$FF83`;
  every shadow access is `LDH` with a `HighDirectOffset` operand;
  shadow writers `pub(crate)`; the only public shadow op is a
  read; `lower_banking_shadow_zero_init` byte-stable at exactly
  10 bytes / 14 M-cycles; `no_bank_shadow_writes_outside_banking_rs`
  returns zero hits.
- **Acquire/release**: `lower_acquire_rom_bank` emits exactly 20
  bytes under SCS / 18 under `Disabled`; BANK1 precedes BANK2 (and
  is tested as convention); shadow updates follow MBC writes; DI
  first under SCS / EI last; DI/EI wording consistently names IME
  (not IF); SCS preconditions documented (IME enabled on entry,
  non-nestable); `lower_release(KeepCurrent, _proof_)` emits zero
  `Instr`s but does emit a `Label`; every MBC store classifies
  `Privileged`; every banking emit lands in a Bank0/HRAM-resident
  section; golden listings never start in `$4000..=$7FFF`.
- **Banking ABI surface**: public API is lease-shaped
  (`lease_rom_switchable` / `lease_sram` / `release_bank`); raw
  emit functions are `pub(crate)` or otherwise unreachable from
  outside `gbf-runtime::banking`; `mbc_write_provenance_audit`
  covers every emitted `Instr` (semantic, not grep).
- **ISR scaffolding**: `check_lease_emission_legal` takes
  `(section, safety, residency)`; rejects ISR / Normal /
  switchable-residency / WRAM-residency sections;
  `InterruptSafetyTable` serializes stably; `declare` rejects
  conflicts; declarations documented for Epic B consumption.
- **Lowering**: `BankingPreLayoutLowering` lowers `BankLease` to
  the acquire sequence and `BankRelease { lease_id, return_to }`
  per `ReturnState` (class-correct); `AssertBank` defaults to
  label-only with compare-and-trap as an opt-in that pulls in CP/JR
  `Instr` deps; `LoweringDisposition::NotOwned` for
  `Yield`/`TraceProbe`; `Error` not swallowed.
- **Nesting and yields**: `manual_lease_rejects_yield_while_held`;
  nested ROM and SRAM lease policy pinned.
- **Security / cleanliness**: no `unsafe` introduced; no raw MBC
  writes outside `banking.rs` (semantic audit, not just grep); no
  nondeterministic iteration; no `SystemTime`/`rand`/env
  dependence; new dependencies justified; the
  `mbc5_constants_match_gbf_hw` drift detector is wired
  unconditionally (T-A2.3 closed, no local fallback shipped);
  MBC5+RUMBLE cartridge profile rejected.
- **Cross-feature handoff**: `InterruptSafetyTable` export shape
  documented for Epic B; banking-shadow zero-init contract
  documented for F-A5 boot; F-A5 boot expected to include
  `lease_rom_switchable(1, Disabled)` immediately after
  `banking_shadow_zero_init`; `no_std + alloc` compatibility
  preserved.

### 15.19 Short video recordings (optional)

Videos are supplemental, not source-of-truth, and may be omitted.
If included, the engineer must check in a transcript and the exact
commands invoked alongside each video. Suggested topics: an ABI
walkthrough (covering `MbcBankClass` / `LeaseLifetime` /
`BankLeaseSpec`, `BankGuard` host-side semantics,
`check_lease_emission_legal`, and `Builder`'s rejection of MBC
stores from `Normal` sections); an acquire byte-sequence
walkthrough across multiple `(bank, policy)` combinations with the
provenance audit running green; an IRQ-safety walkthrough
(declaring ISR / `Normal` sections and watching the rejections,
exporting an `InterruptSafetyTable`, and naming the Epic B
handoff); and a tiny banking reproducibility demo (running
`banking_demo`, regenerating the `.bin` artifacts, checking
hashes, and explicitly stating that F-A4 makes no live emulator
boot claim).

### 15.20 Cleanliness and maintainability

The engineer must include a cleanliness-and-maintainability
writeup that explicitly asserts: no `unsafe` introduced; no global
mutable state; no nondeterministic `HashMap` iteration in
observable output; no direct opcode byte pushes outside the
encoder; no public API panics for invalid user input
(`BankGuard::Drop` may panic only under `cfg(debug_assertions)`);
no environment-dependent output; no hidden feature-gated behavior
that changes default semantics.

The writeup must include the mechanical checks the engineer ran
(at minimum: `grep -R "unsafe" gbf-runtime/src`;
a search for `SystemTime`/`Instant::now`/`rand`/`thread_rng`/
`HashMap` in `gbf-runtime/src`; a grep for raw MBC-shaped literals
outside `gbf-runtime/src/banking.rs`) and the result of each (the
last grep must return zero hits, with any false positive added to
a reviewed exception list). The writeup must also enumerate the
allowed MBC-touching literals (the four emit primitives plus the
test fixtures) and explicitly disallow MBC stores from any other
module, including `listing.rs` and `rom.rs`.

### 15.21 Source-to-artifact traceability

The engineer must include a source-to-artifact traceability
writeup: a worked trace from a single source `bank_lease(spec)`
call through the authoring layer (`Builder` pushing
`PreLayoutOp::BankLease`, returning a `BankGuard`), the lowering
(`BankingPreLayoutLowering::lower` selecting policy and calling
the appropriate `lower_acquire_*`, returning a `LoweredFragment`),
the unchanged F-A1 layout/relax/encode pipeline, and the final
bytes (matching a checked-in `acquire_*.bin`) plus the matching
`.lst` listing. This worked trace is the most direct evidence
that the implementation matches the RFC; the engineer must update
it whenever the byte sequence changes.

### 15.22 Required items for the single-PR packet

Because F-A4 ships as a single PR (per §10), there is one review packet
covering all four child tasks. The engineer must include the following
items in that packet by the time the PR is opened for review:

- a per-child-task walkthrough section that maps each item below to the
  T-A4.x bead it closes:
  - **T-A4.1 + T-A4.2 walkthrough**: type backbone overview;
    `BankLeaseSpec` constructor invariants; `LeaseLifetime` ↔
    yield-safety table; `BankGuard` RAII semantics; HRAM shadow layout
    diagram; HRAM offset constants table; `emit_shadow_zero_init` byte
    sequence.
  - **T-A4.3 walkthrough**: four-instruction MBC write sequence;
    `DI`/`EI` bracketing under each `InterruptPolicy`; byte-by-byte
    tables for every (helper × policy × representative bank);
    privilege-gating proof; cycle-cost-matches-Pan-Docs proof;
    `no_raw_mbc_writes_outside_banking_rs` proof; results from the
    `byte_stable_emit_property` proptest.
  - **T-A4.4 walkthrough**: `InterruptSafetyKind` taxonomy;
    `check_lease_emission_legal` truth table; `InterruptSafetyTable`
    serialization shape; `BankingPreLayoutLowering` implementation
    notes; composite-lowerer expectation (F-A5 will own that);
    Epic B handoff documentation.
- a reviewer-focus index pointing at the files that carry the
  load-bearing logic for each task slice:
  - T-A4.1 + T-A4.2 → `gbf-runtime/src/banking.rs` (`mod types` +
    `mod shadow`), `gbf-runtime/tests/banking_types.rs`,
    `gbf-runtime/tests/banking_shadow.rs`.
  - T-A4.3 → `gbf-runtime/src/banking.rs` (`mod emit`),
    `gbf-runtime/tests/banking_emit.rs`,
    `docs/review/f-a4/artifacts/*.bin`,
    `docs/review/f-a4/artifacts/*.lst`.
  - T-A4.4 → `gbf-runtime/src/banking.rs` (`mod isr` + `mod lowering`),
    `gbf-runtime/tests/banking_isr.rs`,
    `gbf-runtime/tests/banking_lowering.rs`,
    `gbf-runtime/tests/banking_integration.rs`.
- a recommended diff-walk order that mirrors §10's intra-PR
  implementation order (types/shadow → emit → isr/lowering), so a
  reviewer can sign off on each slice before context-switching into the
  next.

The per-task walkthroughs above are additions to the items in
§15.1–§15.21, not replacements; the engineer must keep both up to
date as the PR evolves.

### 15.23 Minimum acceptance bar for the review packet

The review packet is complete only when:

```text
[ ] A fresh checkout can run verify-packet.sh successfully.
[ ] The packet says exactly which files to read first.
[ ] The packet says exactly which files/artifacts can be skimmed.
[ ] Every claim in §14 maps to at least one test, type invariant, or
    generated artifact.
[ ] All eight .bin artifacts are reproducible and hashed.
[ ] Diagrams have both .mmd source and rendered .svg.
[ ] Benchmark output present or a clear performance justification.
[ ] Dependency report present.
[ ] Known-debt ledger present.
[ ] Out-of-scope ledger present.
[ ] Typed error coverage present.
[ ] Reviewer checklist present.
[ ] If supplemental videos are included, each has a transcript and the
    exact commands invoked.
[ ] no_raw_mbc_writes_outside_banking_rs returns zero hits across the
    full workspace.
```

Core principle:

> The engineer should not make the reviewer rediscover the architecture,
> risk model, test strategy, or artifact story from the diff. The packet
> pre-digests all of that, while still giving the reviewer enough precise
> links, commands, and evidence to independently verify every claim.

## 16. References

### Internal

- `history/planv0.md` — line 121 (ISR residency rule), line 1583
  ("twenty-minute lockup" failure mode), line 1631 (`gbf-runtime` modules),
  line 1716 (centralized MBC handling), line 2091 (MBC5 + `$0A` RAM enable),
  line 2935 (engineering rule 15), line 1742 (`ResourceLeaseKind`),
  line 1791 (`InterruptPolicy`).
- `CONSTITUTION.md` — §I.1 (correctness by construction), §III (shifting
  left), §IV.3 (reproducible builds), §V.3 (loud failure).
- `.agents/skills/asm-bead-closure/SKILL.md` — closure-skill checklist
  (banking emits AsmIR through gbf-asm, so this skill applies).
- `bd-1sv` (F-A4 feature bead) and child tasks `bd-371`, `bd-2sv`,
  `bd-19j`, `bd-f5y`.
- `bd-121` (T-A2.3, MBC5 register semantics), `bd-1yu` (T-A2.2, memory map
  constants), `bd-30s` (T-A3.5, `InterruptPolicy` + `ResourceLease`).
- F-A1 RFC (`history/rfcs/F-A1-gbf-asm.md`) — §4 (lowering seams), §7.4
  (far-call thunk model), §16 (claim-to-gate matrix style).
- F-A1 review packet requirements (`history/rfcs/F-A1-review-packet-requirements.md`).
- Existing source: `gbf-asm/src/{isa,section,builder,effect,provenance,
  symbols,lowering}.rs`; `gbf-runtime/src/lib.rs`; `gbf-runtime/src/banking.rs`
  (stub); `gbf-abi/src/interrupt.rs` (T-A3.5 closed, type final);
  `gbf-hw/src/mbc5.rs` (T-A2.3 closed, constants final);
  `gbf-hw/src/{target,cartridge_header}.rs` (CartridgeProfile / MbcType,
  closed; rumble variant deliberately not yet present).

### External

- Pan Docs MBC5: <https://gbdev.io/pandocs/MBC5.html>
- Pan Docs memory map: <https://gbdev.io/pandocs/Memory_Map.html>
- Pan Docs HRAM: <https://gbdev.io/pandocs/Memory_Map.html#high-ram-hram>
- Pan Docs interrupt sources: <https://gbdev.io/pandocs/Interrupt_Sources.html>
- Pan Docs LR35902 instruction timing: <https://gbdev.io/pandocs/CPU_Instruction_Set.html>
- gameroy emulator (for the eventual gbf-emu/gbf-debug-driven boot test):
  <https://github.com/Rodrigodd/gameroy>
- GameBoy CPU manual (gekkio): <https://gekkio.fi/files/gb-docs/gbctr.pdf>

## 17. Appendix: file-by-file change set

| File                                          | Change             | Lines (est.) |
|-----------------------------------------------|--------------------|--------------|
| `gbf-runtime/src/banking.rs`                  | New (replace stub) | ~700         |
| `gbf-runtime/src/lib.rs`                      | Add `pub use`      | +6           |
| `gbf-runtime/Cargo.toml`                      | Add `thiserror`, optional `bincode`, `proptest` (dev) | +4 |
| `gbf-runtime/examples/banking_demo.rs`        | New                | ~120         |
| `gbf-runtime/tests/banking_types.rs`          | New                | ~150         |
| `gbf-runtime/tests/banking_shadow.rs`         | New                | ~180         |
| `gbf-runtime/tests/banking_emit.rs`           | New                | ~500         |
| `gbf-runtime/tests/banking_isr.rs`            | New                | ~150         |
| `gbf-runtime/tests/banking_lowering.rs`       | New                | ~200         |
| `gbf-runtime/tests/banking_integration.rs`    | New                | ~150         |
| `gbf-hw/src/mbc5.rs`                          | Unchanged (T-A2.3 closed; constants imported) | 0 |
| `docs/review/f-a4/**`                         | New (review packet) | ~1500       |
| `scripts/review/f-a4/{build,verify,clean}-packet.sh` | New          | ~150         |

**Total: ~3850 LOC, ~75% of which is tests, fixtures, golden artifacts,
and the review packet itself.**

## 18. End

This RFC stays inside the F-A4 boundary. Anything that requires Epic B's
`ReachabilityValidation`, F-A5's full nucleus, or the `Yield`/`TraceProbe`
lowerings is explicitly deferred. The proposal lets F-A4 close without
those features existing, while leaving every seam (`PreLayoutOpLowering`,
`InterruptSafetyTable`, `MachineEffect::StoreToMbcRegister`, the HRAM
shadow contract) shaped for them to plug in cleanly.

Reviewer asks I would value most:

1. **Is the four-instruction MBC write sequence the right shape?** §6.2
   pins it; the `byte_stable_emit_property` and golden `.bin` artifacts
   gate it. Flag any opcode choice that should be different (e.g., should
   we always re-load `A` between BANK1 and BANK2 even when both are equal,
   or should the helper short-circuit?).
2. **Is `LeaseLifetime::Manual` worth keeping at all?** It's the only
   non-yield-safe lifetime. The cost is one variant + one rejection rule
   in `check_lease_emission_legal`; the benefit is "explicit caller-owned
   lease lifetime when the caller knows better than the scheduler." If
   no real workload exists, removing it would shrink the API surface.
3. **The `BankGuard` RAII semantics — host-side panic on debug, typed
   error on release — is the right tradeoff?** The panic is loud during
   development; the typed error is clean in production. Flag if you'd
   prefer one or the other consistently.
4. **`InterruptPolicy::Enabled` rejection** for `emit_acquire_*` is the
   strict-by-default policy. Any reason to relax it under specific
   yield-safe lifetimes?
5. **Anything in the claim-to-gate matrix (§14) missing for closure?**
   Specifically anything around the F-A5 boot integration that F-A4
   should pre-test rather than declare-by-contract.

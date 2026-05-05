# T-A8.8b Runtime ASM Conformance

Bead: `bd-2j4m`

This packet pins the first scripted emulator conformance smoke suite for
emitted Epic-A ROMs. The suite lives in `gbf-test/tests/runtime_asm_conformance.rs`
and drives ROMs only through `gbf-debug` session init/exec calls.

## Contents

- `conformance-manifest.json`: ROM/symbol fixture provenance, SHA-256 hashes,
  structured observation fingerprints, script paths, trace capacity, and
  expected observations.
- `scripts/tiny_rom_entry.js`: F-A1 tiny-ROM boot, HRAM sentinel, and trace smoke.
- `scripts/banklease_trace.js`: F-A4 BankLease ROM-bank trace for bank 3,
  bank 256 high-bit MBC5 writes, and release back to bank 1.
- `scripts/banklease_sram_window.js`: F-A4 SRAM enable/select/write/disable
  window with HRAM shadow checks and a guest SRAM sentinel write.
- `scripts/runtime_boot_scheduler.js`: F-A5 boot-to-scheduler, runtime HRAM,
  LCD/interrupt setup, and boot trace evidence.
- `scripts/runtime_irq_timer.js`: F-A5 IRQ vector dispatch into the timer
  handler and `yield_requested` write evidence.
- `scripts/runtime_yield_safe_point.js`: F-A5 emitted safe-point helper that
  observes `yield_requested`, clears it, and takes the yield path.
- `scripts/runtime_panic_smoke.js`: F-A5 panic path fault-code storage,
  visible panic text, LCDC re-enable, and halt evidence.

Generated ROM and `.sym` fixtures are materialized by the test under
`target/review/t-a8.8b/generated/`. Per-run sessions and failure capsules are
kept under `target/review/t-a8.8b/runs/<fixture>/run{0,1}/`.

## Gate

```bash
cargo test -p gbf-test --test runtime_asm_conformance
```

The harness runs every fixture twice from fresh `gbf-debug init` sessions and
compares the structured observations plus trace summary digest across runs.

## Claim To Gate

| Claim | Gate |
| --- | --- |
| F-A1 `tiny_rom` boots and writes its HRAM sentinel through `gbf-debug` | `f_a1_tiny_rom_runs_through_scripted_debugger` |
| F-A4 ROM BankLease bytes switch to bank 3, exercise MBC5 high-bit bank 256, and restore bank 1 | `f_a4_banklease_rom_switches_under_scripted_debugger` |
| F-A4 SRAM BankLease bytes enable SRAM, select bank 2, perform a guest sentinel write, and disable SRAM | `f_a4_banklease_sram_window_runs_under_scripted_debugger` |
| F-A4 generated MBC writes come only from trusted runtime banking lowering | `mbc_write_provenance_audit` in the two F-A4 fixture builders |
| F-A5 runtime demo boots to the scheduler with expected HRAM, LCD, and interrupt setup | `f_a5_runtime_boots_to_scheduler_under_scripted_debugger` |
| F-A5 IRQ vectors can dispatch to the timer handler and set `yield_requested` | `f_a5_runtime_timer_irq_sets_yield_under_scripted_debugger` |
| F-A5 emitted safe points observe and clear `yield_requested` before entering compiler-owned yield code | `f_a5_runtime_safe_point_clears_yield_under_scripted_debugger` |
| F-A5 panic path stores a typed fault code, renders `FAULT 0041`, re-enables LCDC, and halts | `f_a5_runtime_panic_path_renders_fault_under_scripted_debugger` |

## Boundaries

This is still a smoke suite. It does not claim compiler-generated inference
correctness, denotational equivalence, hardware-in-the-loop coverage, or a full
compiled inference continuation save/resume after a yielded safe point. That
compiler-owned path belongs with future scheduler-loop and harness-control
owner beads once real generated slice dispatch exists.

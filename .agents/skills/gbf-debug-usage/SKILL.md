---
name: gbf-debug-usage
description: Drive the gbf-debug agent CLI to inspect, step, and script Game Boy ROM execution. Use when working with .gbsess files, gbf-debug, the gb JS object, breakpoints, watchpoints, symbols, or Game Boy ROM debugging in this workspace.
license: Proprietary. LICENSE.txt has complete terms
compatibility: Requires the gbf-debug binary built from this workspace
metadata:
  feature: F-A8
  bead: bd-1aaz
---

# gbf-debug Usage

Use this skill when you need to debug a Game Boy ROM through the workspace's agent-facing CLI. The normal loop is: create a session with `init`, write a small JavaScript script, run it with `exec`, read the JSON envelope, then repeat with the new `.gbsess`.

## Agent Loop

```bash
gbf-debug init --rom target/rom.gb --sym target/rom.sym --out s0.gbsess
gbf-debug exec --in s0.gbsess --script run.js --out s1.gbsess
gbf-debug inspect s1.gbsess
```

Each command emits exactly one JSON object. Success writes to stdout; failure writes an `ErrorEnvelope` to stderr. Do not scrape prose output.

For a smoke test in this workspace, use `gbf-emu/tests/fixtures/tiny_rom.gb` with `docs/review/f-a1/artifacts/tiny_rom.sym`.

## Five Rules

- Set `globalThis.result = ...`; a lexical `let result = ...` is not captured.
- Use `gb.read` / `gb.write` for side-effect-free debugger access and `gb.bus_read` / `gb.bus_write` when you intentionally want CPU-bus side effects.
- Closure predicates are invocation-local and emit a warning; string predicates persist in the session.
- Use `gb.snapshot()` / `gb.restore(handle)` for in-script branching. Copy the `.gbsess` file for cross-invocation forks.
- Every `run_until*` has a deterministic M-cycle cap. Pass an explicit cap when a script may run long.

## Common Script

```js
const entry = gb.symbol("gbf_runtime_dtiny_dentry");
gb.run_until(entry, 100000);
log("at-entry", { regs: gb.regs });
globalThis.result = { pc: gb.regs.pc, a: gb.regs.a };
```

## Symbols

`gb.symbol(name)` returns an address or `null`, but raises on banked ambiguity. Use `gb.symbol_in_bank(name, bank)` or `gb.symbol_at_in_bank(addr, bank)` when a name or address appears in more than one bank.

## Determinism

`Date.now()` is virtual emulator time, `Math.random()` is fixed-seeded, and `console` is unavailable. Use `log(message, data)` for structured logs; log timestamps use the same virtual clock as `Date.now()`.

## References

- `references/ENVELOPE.md` has the JSON envelope shapes.
- `references/GB_OBJECT.md` lists the `gb.*` methods.
- `references/PREDICATES.md` explains closure vs string predicates.
- `references/DETERMINISM.md` covers virtual time, budgets, and watchdog behavior.
- `assets/recipes/` contains reusable JavaScript snippets.

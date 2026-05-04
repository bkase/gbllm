# `gb` API Surface

| Method | Contract | Primary gate |
|---|---|---|
| `gb.regs` | Frozen register snapshot; no live mutable reference. | e2e smoke |
| `gb.read` / `gb.write` | Side-effect-free debugger memory access via F-A7 peek/poke. | e2e smoke |
| `gb.bus_read` / `gb.bus_write` | Side-effecting host bus operations and trace ingestion. | recipe tests |
| `gb.step` | Bounded instruction stepping; timeout checked inside long host loops. | e2e smoke |
| `gb.run_until` | Delegates to F-A7 `run_until_pc`; does not mutate persisted breakpoints. | e2e smoke |
| `gb.run_until_breakpoint` | Runs until an active trap whose predicate evaluates truthy, budget elapsed, or idle. | predicate e2e regression |
| `gb.add_breakpoint` / `gb.add_watchpoint` | Persists no-predicate/string predicates; closure predicates are invocation-local with a warning. | predicate e2e regression |
| `gb.snapshot` / `gb.restore` | In-script transient emulator branch points. | recipe tests |
| `gb.symbol*` | Embedded symbol table lookup with ambiguity refusal. | session wire tests |
| `gb.framebuffer`, `gb.input`, `gb.trace_ring`, `gb.clear_trace` | Display/input/trace accessors over F-A7 primitives. | recipe tests |

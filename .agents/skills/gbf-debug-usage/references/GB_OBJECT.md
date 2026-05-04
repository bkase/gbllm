# gb Object

- `gb.regs`
- `gb.read(addr, len)` / `gb.write(addr, bytes)`
- `gb.bus_read(addr, len)` / `gb.bus_write(addr, bytes)`
- `gb.step(n = 1)`
- `gb.run_until(pc, max_m_cycles?)`
- `gb.run_until_breakpoint(max_m_cycles?)`
- `gb.add_breakpoint(addr, predicate?)`
- `gb.remove_breakpoint(addr)`
- `gb.list_breakpoints()`
- `gb.add_watchpoint(addr, "read"|"write"|"rw", predicate?)`
- `gb.remove_watchpoint(addr, kind)`
- `gb.list_watchpoints()`
- `gb.snapshot()` / `gb.restore(handle)`
- `gb.symbol(name)` / `gb.symbol_in_bank(name, bank)`
- `gb.symbol_at(addr)` / `gb.symbol_at_in_bank(addr, bank)`
- `gb.framebuffer()`
- `gb.input(["a", "start"])`; pass `[]` to release all buttons
- `gb.trace_ring()` / `gb.clear_trace()`

## Common Return Shapes

- `gb.step(n)` returns the post-step state, including `pc_after`.
- `gb.run_until(pc, max_m_cycles?)` returns `{ reason, pc_at_stop }`; `reason` is commonly `pc_reached`.
- `gb.run_until_breakpoint(max_m_cycles?)` returns `{ reason, pc_at_stop }`; `reason` can be `breakpoint`, `watchpoint`, `halt`, or a budget/error reason.
- `gb.trace_ring()` returns the persisted trace event list plus ring metadata.
- `gb.list_breakpoints()` returns entries with `addr`, `has_predicate`, and `persisted_kind`.
- `gb.list_watchpoints()` returns entries with `addr`, `kind`, `has_predicate`, and `persisted_kind`.

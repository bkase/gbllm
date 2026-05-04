gb.add_watchpoint(0xff80, "write", "true");
const outcome = gb.run_until_breakpoint(100000);
globalThis.result = { outcome, trace: gb.trace_ring() };


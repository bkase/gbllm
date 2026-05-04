function assert(condition, message, detail = {}) {
  if (!condition) {
    throw new Error(`${message}: ${JSON.stringify(detail)}`);
  }
}

function byte(addr) {
  return gb.read(addr, 1)[0];
}

function canonicalTrace(trace) {
  return trace.map((event) => ({
    kind: event.kind,
    addr: event.addr,
    data: Array.from(event.data),
    pc_at: event.pc_at,
  }));
}

const IF = 0xff0f;
const HRAM_YIELD_REQUESTED = 0xff84;
const main = gb.symbol("runtime.scheduler.main_loop");
const timerHandler = gb.symbol("runtime.interrupts.timer_handler");
assert(main !== null, "missing scheduler main-loop symbol");
assert(timerHandler !== null, "missing timer-handler symbol");

const boot = gb.run_until(main, 250000);
assert(boot.reason === "pc_reached", "runtime did not boot to scheduler", boot);
assert(byte(HRAM_YIELD_REQUESTED) === 0, "yield flag was not clear after boot");

gb.clear_trace();
gb.add_breakpoint(timerHandler);
gb.bus_write(IF, [0x04]);
const isr = gb.run_until_breakpoint(20000);
gb.remove_breakpoint(timerHandler);
assert(isr.reason === "breakpoint", "timer interrupt did not reach handler", isr);
assert(gb.regs.pc === timerHandler, "breakpoint did not stop at timer handler", {
  pc: gb.regs.pc,
  timerHandler,
});

gb.add_watchpoint(HRAM_YIELD_REQUESTED, "write");
const yieldWrite = gb.run_until_breakpoint(20000);
gb.remove_watchpoint(HRAM_YIELD_REQUESTED, "write");
const trace = gb.trace_ring();
assert(yieldWrite.reason === "watchpoint", "timer ISR did not write yield_requested", yieldWrite);
assert(byte(HRAM_YIELD_REQUESTED) === 1, "timer ISR did not set yield_requested");
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === IF && event.data[0] === 0x04),
  "host IF request was not captured in trace ring",
  { trace }
);
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === HRAM_YIELD_REQUESTED && event.data[0] === 1),
  "yield_requested write was not captured in trace ring",
  { trace }
);

globalThis.result = {
  fixture: "f-a5-runtime-irq-timer",
  boot_reason: boot.reason,
  isr_reason: isr.reason,
  yield_write_reason: yieldWrite.reason,
  pc: gb.regs.pc,
  main,
  timer_handler: timerHandler,
  yield_requested: byte(HRAM_YIELD_REQUESTED),
  trace_events: canonicalTrace(trace),
  trace_kinds: trace.map((event) => event.kind),
};

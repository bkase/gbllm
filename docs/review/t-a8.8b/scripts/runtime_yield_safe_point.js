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

const HRAM_YIELD_REQUESTED = 0xff84;
const WRAM_CLEARED_SENTINEL = 0xc372;
const WRAM_SKIPPED_SENTINEL = 0xc373;

const entry = gb.symbol("runtime.conformance.yield_safe_point_entry");
const yielded = gb.symbol("runtime.conformance.yield_safe_point_observed");
const done = gb.symbol("runtime.conformance.yield_safe_point_done");
assert(entry !== null, "missing yield safe-point entry symbol");
assert(yielded !== null, "missing yield safe-point observed symbol");
assert(done !== null, "missing yield safe-point done symbol");

gb.clear_trace();
const outcome = gb.run_until(done, 10000);
const trace = gb.trace_ring();
const yieldRequested = byte(HRAM_YIELD_REQUESTED);
const clearedSentinel = byte(WRAM_CLEARED_SENTINEL);
const skippedSentinel = byte(WRAM_SKIPPED_SENTINEL);

assert(outcome.reason === "pc_reached", "yield safe-point fixture did not reach done label", outcome);
assert(gb.regs.pc === done, "yield safe-point fixture stopped at unexpected PC", { pc: gb.regs.pc, done });
assert(yieldRequested === 0, "yield safe point did not clear yield_requested", { yieldRequested });
assert(clearedSentinel === 0x59, "yield safe point did not take the yielded path", {
  clearedSentinel,
});
assert(skippedSentinel === 0x00, "yield safe point took the clear-flag fast path unexpectedly", {
  skippedSentinel,
});
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === HRAM_YIELD_REQUESTED && event.data[0] === 1),
  "fixture did not set yield_requested before polling",
  { trace }
);
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === HRAM_YIELD_REQUESTED && event.data[0] === 0),
  "emit_yield_check did not clear yield_requested",
  { trace }
);

globalThis.result = {
  fixture: "f-a5-runtime-yield-safe-point",
  stop_reason: outcome.reason,
  pc: gb.regs.pc,
  entry,
  yielded,
  done,
  yield_requested: yieldRequested,
  cleared_sentinel: clearedSentinel,
  skipped_sentinel: skippedSentinel,
  trace_events: canonicalTrace(trace),
  trace_kinds: trace.map((event) => event.kind),
};

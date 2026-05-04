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

const entry = gb.symbol("gbf_runtime_dtiny_dentry");
const loop = gb.symbol("gbf_runtime_dtiny_dloop");
assert(entry !== null, "missing tiny entry symbol");
assert(loop !== null, "missing tiny loop symbol");

gb.clear_trace();
const outcome = gb.run_until(loop, 1024);
const trace = gb.trace_ring();
const hramValue = byte(0xff80);

assert(outcome.reason === "pc_reached", "tiny ROM did not reach loop", outcome);
assert(gb.regs.pc === loop, "tiny ROM stopped at unexpected PC", { pc: gb.regs.pc, loop });
assert(hramValue === 0x42, "tiny ROM did not write the HRAM sentinel", { hramValue });
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === 0xff80 && event.data[0] === 0x42),
  "tiny ROM HRAM write was not captured in the trace ring",
  { trace }
);

globalThis.result = {
  fixture: "f-a1-tiny-rom",
  stop_reason: outcome.reason,
  pc: gb.regs.pc,
  entry,
  loop,
  hram_ff80: hramValue,
  trace_events: canonicalTrace(trace),
  trace_kinds: trace.map((event) => event.kind),
};

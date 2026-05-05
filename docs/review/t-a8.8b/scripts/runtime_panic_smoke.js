function assert(condition, message, detail = {}) {
  if (!condition) {
    throw new Error(`${message}: ${JSON.stringify(detail)}`);
  }
}

function byte(addr) {
  return gb.read(addr, 1)[0];
}

function text(addr, len) {
  return String.fromCharCode(...gb.read(addr, len));
}

function canonicalTrace(trace) {
  return trace.map((event) => ({
    kind: event.kind,
    addr: event.addr,
    data: Array.from(event.data),
    pc_at: event.pc_at,
  }));
}

const WRAM_LAST_FAULT = 0xc360;
const PANIC_SCREEN_BG = 0x9800;
const LCDC = 0xff40;
const entry = gb.symbol("runtime.conformance.panic_entry");
assert(entry !== null, "missing panic entry symbol");

gb.clear_trace();
gb.add_watchpoint(WRAM_LAST_FAULT, "write");
const faultWrite = gb.run_until_breakpoint(500000);
gb.remove_watchpoint(WRAM_LAST_FAULT, "write");
assert(faultWrite.reason === "watchpoint", "panic did not write the fault code", faultWrite);

const halt = gb.run_until_breakpoint(2000000);
const trace = gb.trace_ring();
const faultCode = byte(WRAM_LAST_FAULT) | (byte(WRAM_LAST_FAULT + 1) << 8);
const rendered = text(PANIC_SCREEN_BG, 10);
const lcdc = gb.bus_read(LCDC, 1)[0];

assert(halt.reason === "idle_halt", "panic did not halt after rendering", halt);
assert(faultCode === 0x0041, "unexpected panic fault code", { faultCode });
assert(rendered === "FAULT 0041", "panic screen did not render fault code", { rendered });
assert(lcdc === 0x91, "panic did not re-enable visible LCDC mode", { lcdc });
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === WRAM_LAST_FAULT && event.data[0] === 0x41),
  "fault low-byte write missing from trace ring",
  { trace }
);
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === PANIC_SCREEN_BG && event.data[0] === 0x46),
  "panic VRAM render write missing from trace ring",
  { trace }
);

globalThis.result = {
  fixture: "f-a5-runtime-panic-smoke",
  fault_write_reason: faultWrite.reason,
  halt_reason: halt.reason,
  pc: gb.regs.pc,
  entry,
  fault_code: faultCode,
  rendered,
  lcdc,
  trace_events: canonicalTrace(trace),
  trace_kinds: trace.map((event) => event.kind),
};

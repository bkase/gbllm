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

function writeSramBank(bank, value) {
  gb.bus_write(0x0000, [0x0a]);
  gb.bus_write(0x4000, [bank]);
  gb.bus_write(0xa000, [value]);
  gb.bus_write(0x0000, [0x00]);
}

function readSramBank(bank) {
  gb.bus_write(0x0000, [0x0a]);
  gb.bus_write(0x4000, [bank]);
  const value = gb.bus_read(0xa000, 1)[0];
  gb.bus_write(0x0000, [0x00]);
  return value;
}

const entry = gb.symbol("runtime.conformance.sram_window_entry");
const done = gb.symbol("runtime.conformance.sram_window_done");
assert(entry !== null, "missing SRAM-window entry symbol");
assert(done !== null, "missing SRAM-window done symbol");

for (const bank of [0, 1, 2]) {
  writeSramBank(bank, 0);
}
gb.bus_write(0x4000, [0]);
gb.bus_write(0x0000, [0x00]);
gb.clear_trace();
const outcome = gb.run_until(done, 10000);
const trace = gb.trace_ring();
const sramSwitches = trace
  .filter((event) => event.kind === "sram_bank_switch")
  .map((event) => event.data[0]);
const mbcWrites = trace
  .filter((event) =>
    event.kind === "mem_write" &&
    ((event.addr >= 0x0000 && event.addr < 0x2000) ||
      (event.addr >= 0x4000 && event.addr < 0x6000))
  )
  .map((event) => [event.addr, event.data[0]]);
const sramWrites = trace
  .filter((event) => event.kind === "mem_write" && event.addr === 0xa000)
  .map((event) => event.data[0]);
const currentSramBank = byte(0xff82);
const sramEnabled = byte(0xff83);
const directSramBanks = [readSramBank(0), readSramBank(1), readSramBank(2)];

assert(outcome.reason === "pc_reached", "SRAM-window fixture did not reach done label", outcome);
assert(gb.regs.pc === done, "SRAM-window fixture stopped at unexpected PC", { pc: gb.regs.pc, done });
assert(currentSramBank === 2, "SRAM bank shadow did not retain selected bank 2", {
  currentSramBank,
});
assert(sramEnabled === 0, "SRAM release did not clear enabled shadow", { sramEnabled });
assert(JSON.stringify(sramSwitches) === JSON.stringify([2]), "unexpected SRAM bank switch trace", {
  sramSwitches,
  trace,
});
assert(
  JSON.stringify(mbcWrites) === JSON.stringify([
    [0x0000, 0x0a],
    [0x4000, 2],
    [0x0000, 0x00],
  ]),
  "unexpected SRAM MBC write sequence",
  { mbcWrites, trace }
);
assert(JSON.stringify(sramWrites) === JSON.stringify([0x5a]), "missing guest SRAM sentinel write", {
  sramWrites,
  trace,
});
assert(
  JSON.stringify(directSramBanks) === JSON.stringify([0x00, 0x00, 0x5a]),
  "guest SRAM sentinel landed in the wrong backing bank",
  { directSramBanks }
);

globalThis.result = {
  fixture: "f-a4-banklease-sram-window",
  stop_reason: outcome.reason,
  pc: gb.regs.pc,
  entry,
  done,
  current_sram_bank: currentSramBank,
  sram_enabled: sramEnabled,
  sram_bank_switches: sramSwitches,
  mbc_writes: mbcWrites,
  sram_writes: sramWrites,
  direct_sram_banks: directSramBanks,
  trace_events: canonicalTrace(trace),
  trace_kinds: trace.map((event) => event.kind),
};

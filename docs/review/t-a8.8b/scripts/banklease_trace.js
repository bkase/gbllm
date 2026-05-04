function assert(condition, message, detail = {}) {
  if (!condition) {
    throw new Error(`${message}: ${JSON.stringify(detail)}`);
  }
}

function byte(addr) {
  return gb.read(addr, 1)[0];
}

function u16le(bytes) {
  return bytes[0] | (bytes[1] << 8);
}

function canonicalTrace(trace) {
  return trace.map((event) => ({
    kind: event.kind,
    addr: event.addr,
    data: Array.from(event.data),
    pc_at: event.pc_at,
  }));
}

const entry = gb.symbol("runtime.conformance.rom_switch_entry");
const done = gb.symbol("runtime.conformance.rom_switch_done");
assert(entry !== null, "missing banklease entry symbol");
assert(done !== null, "missing banklease done symbol");

gb.clear_trace();
const outcome = gb.run_until(done, 10000);
const trace = gb.trace_ring();
const bankSwitches = trace
  .filter((event) => event.kind === "rom_bank_switch")
  .map((event) => u16le(event.data));
const mbcWrites = trace
  .filter((event) => event.kind === "mem_write" && event.addr >= 0x0000 && event.addr < 0x6000)
  .map((event) => [event.addr, event.data[0]]);
const romBankShadow = byte(0xff80) | (byte(0xff81) << 8);
const bank3Sentinel = byte(0xc370);
const bank256Sentinel = byte(0xc371);

assert(outcome.reason === "pc_reached", "BankLease fixture did not reach done label", outcome);
assert(gb.regs.pc === done, "BankLease fixture stopped at unexpected PC", { pc: gb.regs.pc, done });
assert(romBankShadow === 1, "BankLease release did not restore ROM bank 1 shadow", {
  romBankShadow,
});
assert(bank3Sentinel === 0xa3, "BankLease fixture did not read the bank-3 ROMX sentinel", {
  bank3Sentinel,
});
assert(bank256Sentinel === 0xc0, "BankLease fixture did not read the bank-256 ROMX sentinel", {
  bank256Sentinel,
});
assert(bankSwitches.includes(3), "ROM bank 3 switch was not traced", { bankSwitches, trace });
assert(bankSwitches.includes(256), "ROM bank 256 high-bit switch was not traced", {
  bankSwitches,
  trace,
});
assert(bankSwitches[bankSwitches.length - 1] === 1, "final ROM bank switch did not restore bank 1", {
  bankSwitches,
  trace,
});
assert(
  JSON.stringify(mbcWrites) === JSON.stringify([
    [0x2000, 3],
    [0x3000, 0],
    [0x2000, 1],
    [0x3000, 0],
    [0x2000, 0],
    [0x3000, 1],
    [0x2000, 1],
    [0x3000, 0],
  ]),
  "unexpected MBC5 ROM write sequence",
  { mbcWrites, trace }
);

globalThis.result = {
  fixture: "f-a4-banklease-rom-switch",
  stop_reason: outcome.reason,
  pc: gb.regs.pc,
  entry,
  done,
  rom_bank_shadow: romBankShadow,
  bank3_sentinel: bank3Sentinel,
  bank256_sentinel: bank256Sentinel,
  rom_bank_switches: bankSwitches,
  mbc_writes: mbcWrites,
  trace_events: canonicalTrace(trace),
  trace_kinds: trace.map((event) => event.kind),
};

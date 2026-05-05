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

const HRAM_ROM_BANK_LO = 0xff80;
const HRAM_ROM_BANK_HI = 0xff81;
const HRAM_SRAM_BANK = 0xff82;
const HRAM_SRAM_ENABLED = 0xff83;
const HRAM_YIELD_REQUESTED = 0xff84;
const HRAM_FRAME_COUNT = 0xff85;

const LCDC = 0xff40;
const STAT = 0xff41;
const IE = 0xffff;
const BOOTSTRAP_BG_MAP_ORIGIN = 0x9800;

const main = gb.symbol("runtime.scheduler.main_loop");
const bootEntry = gb.symbol("runtime.boot.runtime_boot_entry");
assert(main !== null, "missing scheduler main-loop symbol");
assert(bootEntry !== null, "missing boot-entry symbol");

gb.clear_trace();
const boot = gb.run_until(main, 250000);
const trace = gb.trace_ring();
assert(boot.reason === "pc_reached", "runtime did not boot to scheduler", boot);
assert(byte(HRAM_ROM_BANK_LO) === 0, "boot did not clear ROM bank shadow lo");
assert(byte(HRAM_ROM_BANK_HI) === 0, "boot did not clear ROM bank shadow hi");
assert(byte(HRAM_SRAM_BANK) === 0, "boot did not clear SRAM bank shadow");
assert(byte(HRAM_SRAM_ENABLED) === 0, "boot did not clear SRAM enabled shadow");
assert(byte(HRAM_YIELD_REQUESTED) === 0, "boot did not clear yield flag");
assert(byte(HRAM_FRAME_COUNT) === 0, "boot did not clear frame count");

const lcdc = gb.bus_read(LCDC, 1)[0];
const stat = gb.bus_read(STAT, 1)[0];
const ie = byte(IE);
assert(lcdc === 0x91, "boot did not install bring-up LCDC", { lcdc });
assert((stat & 0x08) !== 0, "boot did not enable STAT HBlank interrupt", { stat });
assert((ie & 0x17) === 0x17, "boot did not enable expected interrupt mask", { ie });
assert(
  trace.some((event) => event.kind === "mem_write" && event.addr === BOOTSTRAP_BG_MAP_ORIGIN && event.data[0] === 0),
  "runtime BG-map bootstrap write was not captured in trace ring",
  { trace }
);
assert(
  trace.some((event) => event.kind === "io_write" && event.addr === LCDC && event.data[0] === 0x91),
  "boot LCDC enable did not produce IO write trace",
  { trace }
);
assert(
  trace.some((event) => event.kind === "io_write" && event.addr === STAT && (event.data[0] & 0x08) !== 0),
  "boot STAT setup did not produce IO write trace",
  { trace }
);

globalThis.result = {
  fixture: "f-a5-runtime-boot-scheduler",
  boot_reason: boot.reason,
  pc: gb.regs.pc,
  main,
  boot_entry: bootEntry,
  hram_rom_bank_shadow: byte(HRAM_ROM_BANK_LO) | (byte(HRAM_ROM_BANK_HI) << 8),
  lcdc,
  stat,
  ie,
  trace_events: canonicalTrace(trace),
  trace_kinds: trace.map((event) => event.kind),
};

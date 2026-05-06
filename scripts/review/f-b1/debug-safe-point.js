const safePoint = gb.symbol("f_b1.tile_safe_point");
if (safePoint === null) {
  throw new Error("missing f_b1.tile_safe_point symbol");
}
const computeYield = gb.symbol("f_b1.compute_yield_safe_point");
if (computeYield === null) {
  throw new Error("missing f_b1.compute_yield_safe_point symbol");
}
const copyYield = gb.symbol("f_b1.copy_yield_safe_point");
if (copyYield === null) {
  throw new Error("missing f_b1.copy_yield_safe_point symbol");
}
const vblankHandler = gb.symbol("f_b1.vblank_handler");
if (vblankHandler === null) {
  throw new Error("missing f_b1.vblank_handler symbol");
}

function hram(offset) {
  return Array.from(gb.read(0xff00 + offset, 1))[0];
}

function serviceState() {
  return {
    frameCount: hram(0x85),
    lastServicedFrame: hram(0x88),
    widgetUpdateCount: hram(0x89),
    schedulerServiceCount: hram(0x8a),
  };
}

const firstVblank = gb.run_until(vblankHandler, 2000000);
const afterFirstVblank = serviceState();
const firstCopyYield = gb.run_until(copyYield, 10000000);
const afterFirstCopyYield = serviceState();
const firstComputeYield = gb.run_until(computeYield, 10000000);
const afterFirstComputeYield = serviceState();
const first = gb.run_until(safePoint, 60000000);
const afterFirstTile = serviceState();
const firstRegs = gb.regs;
const firstTilePrefix = Array.from(gb.read(0xc000, 32));
const firstTile = Array.from(gb.read(0xc000, 1024));
let firstNonzero = 0;
let firstChecksum = 0;
for (const byte of firstTile) {
  if (byte !== 0) {
    firstNonzero += 1;
  }
  firstChecksum = (firstChecksum + byte) >>> 0;
}

const step = gb.step(1);
const second = gb.run_until(safePoint, 60000000);
const afterSecondTile = serviceState();
const secondRegs = gb.regs;
const secondTilePrefix = Array.from(gb.read(0xc000, 32));
const secondTile = Array.from(gb.read(0xc000, 1024));
let secondNonzero = 0;
let secondChecksum = 0;
for (const byte of secondTile) {
  if (byte !== 0) {
    secondNonzero += 1;
  }
  secondChecksum = (secondChecksum + byte) >>> 0;
}

if (afterFirstComputeYield.lastServicedFrame < 1) {
  throw new Error("yield routine did not service a VBlank frame");
}
if (afterFirstComputeYield.widgetUpdateCount !== afterFirstComputeYield.schedulerServiceCount) {
  throw new Error("widget and scheduler service counters diverged");
}
if (afterFirstTile.widgetUpdateCount <= afterFirstComputeYield.widgetUpdateCount) {
  throw new Error("tile execution did not continue frame service progress");
}
if (afterSecondTile.widgetUpdateCount <= afterFirstTile.widgetUpdateCount) {
  throw new Error("resume after tile safe point did not continue frame service progress");
}

globalThis.result = {
  safePoint,
  computeYield,
  copyYield,
  vblankHandler,
  firstVblank,
  afterFirstVblank,
  firstCopyYield,
  afterFirstCopyYield,
  firstComputeYield,
  afterFirstComputeYield,
  first,
  afterFirstTile,
  firstPc: firstRegs.pc,
  firstTilePrefix,
  firstNonzero,
  firstChecksum,
  step,
  second,
  afterSecondTile,
  secondPc: secondRegs.pc,
  secondTilePrefix,
  secondNonzero,
  secondChecksum,
  tilesDiffer: JSON.stringify(firstTile) !== JSON.stringify(secondTile),
  prefixesDiffer: JSON.stringify(firstTilePrefix) !== JSON.stringify(secondTilePrefix),
};

// End-to-end gbf-debug acceptance for the interactive subword cartridge.
//
// This script deliberately performs no debugger-side memory mutation. The
// prompt enters through JOYP alone: every D-pad/A press occupies one idle frame
// and is followed by one released idle frame. START is released as soon as the
// ROM reaches its post-tokenizer boundary.

const PROMPT = "Once upon a time";
const PROMPT_BYTES = Array.from(PROMPT).map((char) => char.charCodeAt(0));
const EXPECTED_PROMPT_IDS = [435, 443, 258, 402];
const EXPECTED_GENERATED_IDS = [
  44, 405, 282, 258, 395, 486, 406, 712, 46, 341, 508, 265,
  325, 330, 339, 478, 44, 404, 617, 297, 310, 46, 10, 388,
];
const EXPECTED_TRANSCRIPT_ROWS = [
  ", there was a little",
  " girl named Lucy. Sh",
  "e loved to play with",
  " her ball, kicking i",
  "t.                  ",
  "One█                ",
  "                    ",
  "                    ",
  "                    ",
  "                    ",
];
const EXPECTED_INITIAL_RNG = 0x5eed;

const IDLE_FRAME_BUDGET = 100000;
const TOKENIZE_BUDGET = 4194304;
// One DMG machine-cycle tick is 1 / 1,048,576 seconds. Reaching every model
// boundary inside this cap proves the <=30-second/token device requirement.
const MODEL_BOUNDARY_BUDGET = 30 * 1048576;
const MAX_GENERATED_TOKENS = 64;

const TRANSCRIPT_BASE = 0x9800;
const TRANSCRIPT_ROWS = 10;
const TRANSCRIPT_COLS = 20;
const BG_MAP_STRIDE = 32;
const PROMPT_ROW = 11;
const SUBWORD_CURSOR_TILE = 0xa0;

// The on-screen keyboard is the deployed 4x19 charset-v1 grid. Its A-button
// value is the literal byte at the corresponding cell.
const KEY_BYTES = [];
for (let byte = 0x41; byte <= 0x5a; byte += 1) KEY_BYTES.push(byte);
for (let byte = 0x61; byte <= 0x7a; byte += 1) KEY_BYTES.push(byte);
for (let byte = 0x30; byte <= 0x39; byte += 1) KEY_BYTES.push(byte);
for (const byte of [
  0x20, 0x2e, 0x2c, 0x21, 0x3f, 0x2d, 0x27,
  0x3a, 0x3b, 0x28, 0x29, 0x22, 0x2f, 0x0a,
]) {
  KEY_BYTES.push(byte);
}

function fail(message) {
  throw new Error(`bd-36akp acceptance failed: ${message}`);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function requireSymbol(name) {
  const address = gb.symbol(name);
  if (address === null) fail(`missing required symbol ${name}`);
  return address;
}

const symbols = {
  idle: requireSymbol("subword_shell_idle"),
  tokenizeDone: requireSymbol("subword_tokenize_done"),
  warmBoundary: requireSymbol("subword_warm_boundary"),
  tokenBoundary: requireSymbol("subword_token_boundary"),
  generationDone: requireSymbol("subword_generation_done"),
  promptBytes: requireSymbol("subword_prompt_bytes"),
  promptByteLen: requireSymbol("subword_prompt_byte_len"),
  promptTokenIds: requireSymbol("subword_prompt_token_ids"),
  promptTokenLen: requireSymbol("subword_prompt_token_len"),
  rng: requireSymbol("subword_rng"),
  sampledLo: requireSymbol("subword_sampled_lo"),
  sampledHi: requireSymbol("subword_sampled_hi"),
};

function readU8(address) {
  return Array.from(gb.read(address, 1))[0];
}

function readU16Le(address) {
  const bytes = Array.from(gb.read(address, 2));
  return bytes[0] | (bytes[1] << 8);
}

function readSplitU16(lowAddress, highAddress) {
  return readU8(lowAddress) | (readU8(highAddress) << 8);
}

function arraysEqual(left, right) {
  return left.length === right.length && left.every((value, i) => value === right[i]);
}

function runTo(target, maxMCycles, phase) {
  const outcome = gb.run_until(target, maxMCycles);
  assert(
    outcome.reason === "pc_reached" && outcome.pc_at_stop === target,
    `${phase} did not reach ${target.toString(16)}: ${JSON.stringify(outcome)}`,
  );
  return outcome;
}

// Trap labels are reached before the instruction at that address executes.
// Step once before asking to reach the same label again.
function stepAndRunTo(target, maxMCycles, phase) {
  const step = gb.step(1);
  const run = runTo(target, maxMCycles, phase);
  return { step, run };
}

const boot = runTo(symbols.idle, IDLE_FRAME_BUDGET, "boot/idle");

const idleFrameMCycles = [];
let joypPressCount = 0;

function idleFrame(buttons, phase) {
  gb.input(buttons);
  const boundary = stepAndRunTo(symbols.idle, IDLE_FRAME_BUDGET, phase);
  idleFrameMCycles.push(Number(boundary.run.m_cycles_floor_consumed));
}

function pressAndRelease(button, phase) {
  idleFrame([button], `${phase}/press`);
  idleFrame([], `${phase}/release`);
  joypPressCount += 1;
}

assert(KEY_BYTES.length === 76, `keyboard has ${KEY_BYTES.length} cells, expected 76`);
let cursor = 0;
for (let promptIndex = 0; promptIndex < PROMPT_BYTES.length; promptIndex += 1) {
  const byte = PROMPT_BYTES[promptIndex];
  const target = KEY_BYTES.indexOf(byte);
  assert(target >= 0, `prompt byte ${byte} is absent from the keyboard`);

  const cursorRow = Math.floor(cursor / 19);
  const cursorCol = cursor % 19;
  const targetRow = Math.floor(target / 19);
  const targetCol = target % 19;

  for (let row = cursorRow; row < targetRow; row += 1) {
    pressAndRelease("down", `byte-${promptIndex}/down`);
  }
  for (let row = targetRow; row < cursorRow; row += 1) {
    pressAndRelease("up", `byte-${promptIndex}/up`);
  }
  for (let col = cursorCol; col < targetCol; col += 1) {
    pressAndRelease("right", `byte-${promptIndex}/right`);
  }
  for (let col = targetCol; col < cursorCol; col += 1) {
    pressAndRelease("left", `byte-${promptIndex}/left`);
  }
  pressAndRelease("a", `byte-${promptIndex}/type`);
  cursor = target;
}

const typedPromptLen = readU8(symbols.promptByteLen);
const typedPromptBytes = Array.from(gb.read(symbols.promptBytes, typedPromptLen));
assert(
  arraysEqual(typedPromptBytes, PROMPT_BYTES),
  `JOYP prompt mismatch: observed ${JSON.stringify(typedPromptBytes)}`,
);
const promptEcho = Array.from(
  gb.read(TRANSCRIPT_BASE + PROMPT_ROW * BG_MAP_STRIDE, TRANSCRIPT_COLS),
);
assert(
  arraysEqual(promptEcho.slice(0, PROMPT_BYTES.length), PROMPT_BYTES) &&
    promptEcho.slice(PROMPT_BYTES.length).every((tile) => tile === 0x20),
  `visible prompt echo mismatch: observed ${JSON.stringify(promptEcho)}`,
);

const rngBeforeSubmit = readU16Le(symbols.rng);
assert(
  rngBeforeSubmit === EXPECTED_INITIAL_RNG,
  `ROM-owned RNG seed is ${rngBeforeSubmit.toString(16)}, expected ${EXPECTED_INITIAL_RNG.toString(16)}`,
);
gb.input(["start"]);
const tokenize = stepAndRunTo(symbols.tokenizeDone, TOKENIZE_BUDGET, "on-device BPE");
gb.input([]);

const promptTokenLen = readU8(symbols.promptTokenLen);
const promptTokenIds = [];
for (let i = 0; i < promptTokenLen; i += 1) {
  promptTokenIds.push(readU16Le(symbols.promptTokenIds + 2 * i));
}
assert(
  arraysEqual(promptTokenIds, EXPECTED_PROMPT_IDS),
  `on-device BPE mismatch: observed ${JSON.stringify(promptTokenIds)}, expected ${JSON.stringify(EXPECTED_PROMPT_IDS)}`,
);

// Each prompt token must pass through the recurrent model independently and
// reach its boundary within the real 30-second device-cycle budget.
const warmBoundaryMCycles = [];
for (let i = 0; i < promptTokenIds.length; i += 1) {
  const boundary = stepAndRunTo(
    symbols.warmBoundary,
    MODEL_BOUNDARY_BUDGET,
    `prompt warm boundary ${i}`,
  );
  warmBoundaryMCycles.push(Number(boundary.run.m_cycles_floor_consumed));
}

// The first token samples directly from the final prompt forward pass.
const firstBoundary = stepAndRunTo(
  symbols.tokenBoundary,
  MODEL_BOUNDARY_BUDGET,
  "first generated token boundary",
);
const generatedIds = [readSplitU16(symbols.sampledLo, symbols.sampledHi)];
const generationBoundaryMCycles = [Number(firstBoundary.run.m_cycles_floor_consumed)];

// From here either another token boundary or generation_done can occur first.
// Persistent breakpoints let every subsequent recurrent step retain the same
// 30-second cap instead of hiding the whole tail behind one aggregate budget.
gb.step(1);
gb.add_breakpoint(symbols.tokenBoundary);
gb.add_breakpoint(symbols.generationDone);
let generationDone = null;
while (generationDone === null) {
  const boundary = gb.run_until_breakpoint(MODEL_BOUNDARY_BUDGET);
  assert(
    boundary.reason === "breakpoint",
    `generation boundary exceeded the 30-second device budget: ${JSON.stringify(boundary)}`,
  );
  if (boundary.pc_at_stop === symbols.generationDone) {
    generationDone = boundary;
    break;
  }
  assert(
    boundary.pc_at_stop === symbols.tokenBoundary,
    `unexpected generation breakpoint at ${boundary.pc_at_stop.toString(16)}`,
  );
  generatedIds.push(readSplitU16(symbols.sampledLo, symbols.sampledHi));
  generationBoundaryMCycles.push(Number(boundary.m_cycles_floor_consumed));
  assert(
    generatedIds.length <= MAX_GENERATED_TOKENS,
    `generation did not terminate within ${MAX_GENERATED_TOKENS} tokens`,
  );

  // Remove the trap at the current PC so exactly one instruction can retire,
  // then reinstall it for the next generated-token boundary.
  gb.remove_breakpoint(symbols.tokenBoundary);
  gb.step(1);
  gb.add_breakpoint(symbols.tokenBoundary);
}
gb.remove_breakpoint(symbols.tokenBoundary);
gb.remove_breakpoint(symbols.generationDone);

function captureTranscript() {
  const tiles = [];
  const rows = [];
  for (let row = 0; row < TRANSCRIPT_ROWS; row += 1) {
    const rowTiles = Array.from(
      gb.read(TRANSCRIPT_BASE + row * BG_MAP_STRIDE, TRANSCRIPT_COLS),
    );
    tiles.push(...rowTiles);
    let text = "";
    for (const tile of rowTiles) {
      if (tile === SUBWORD_CURSOR_TILE) {
        text += "█";
      } else if (tile >= 0x20 && tile <= 0x7e) {
        text += String.fromCharCode(tile);
      } else {
        text += "?";
      }
    }
    rows.push(text);
  }
  return { tiles, rows };
}

const transcript = captureTranscript();
assert(
  arraysEqual(generatedIds, EXPECTED_GENERATED_IDS),
  `generated u16 sequence drifted: observed ${JSON.stringify(generatedIds)}`,
);
const expectedTranscriptTiles = EXPECTED_TRANSCRIPT_ROWS.flatMap((row) =>
  Array.from(row).map((char) => char === "█" ? SUBWORD_CURSOR_TILE : char.charCodeAt(0)),
);
assert(
  arraysEqual(transcript.tiles, expectedTranscriptTiles),
  `rendered transcript drifted: observed ${JSON.stringify(transcript.rows)}`,
);
const rngAfterGeneration = readU16Le(symbols.rng);

// Return to idle and give the PPU one released-input frame to paint the final
// BG map before taking the framebuffer. This is still JOYP-only interaction.
gb.input([]);
const returnedToIdle = stepAndRunTo(symbols.idle, IDLE_FRAME_BUDGET, "return to idle");
idleFrame([], "final framebuffer settle");
assert(readU8(symbols.promptByteLen) === 0, "typed-byte length did not reset");
assert(readU8(symbols.promptTokenLen) === 0, "BPE-token length did not reset");
const resetPromptEcho = Array.from(
  gb.read(TRANSCRIPT_BASE + PROMPT_ROW * BG_MAP_STRIDE, TRANSCRIPT_COLS),
);
assert(resetPromptEcho.every((tile) => tile === 0x20), "prompt row did not clear");
const framebuffer = Array.from(gb.framebuffer());
assert(framebuffer.length === 160 * 144, `framebuffer has ${framebuffer.length} pixels`);
assert(framebuffer.every((pixel) => pixel >= 0 && pixel <= 3), "invalid DMG palette index");

globalThis.result = {
  schema: "bd_36akp_interactive_acceptance.v1",
  passed: true,
  prompt: PROMPT,
  expectedPromptIds: EXPECTED_PROMPT_IDS,
  typedPromptBytes,
  promptTokenIds,
  joypPressCount,
  joypIdleFrameCount: idleFrameMCycles.length,
  idleFrameMCycles,
  rngBeforeSubmit,
  rngAfterGeneration,
  firstGeneratedId: generatedIds[0],
  generatedIds,
  warmBoundaryMCycles,
  generationBoundaryMCycles,
  boot,
  tokenize: tokenize.run,
  generationDone,
  returnedToIdle: returnedToIdle.run,
  transcriptTiles: transcript.tiles,
  transcriptRows: transcript.rows,
  framebufferWidth: 160,
  framebufferHeight: 144,
  framebuffer,
};

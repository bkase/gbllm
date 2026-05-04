const h = gb.snapshot();
gb.step(1);
const after = gb.regs.pc;
gb.restore(h);
globalThis.result = { after, restored: gb.regs.pc };


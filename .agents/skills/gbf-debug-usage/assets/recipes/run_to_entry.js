const entry = gb.symbol("gbf_runtime_dtiny_dentry");
gb.run_until(entry, 100000);
globalThis.result = { pc: gb.regs.pc, entry };


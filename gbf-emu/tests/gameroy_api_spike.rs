mod common;

use gameroy::gameboy::GameBoy;
use gameroy::gameboy::cartridge::Cartridge;
use gameroy::interpreter::Interpreter;

#[test]
fn gameroy_core_public_surface_compiles() {
    let cartridge = Cartridge::new(common::rom(&[0x00, 0x76], 0x00, 0x00)).expect("valid ROM");
    let mut gb = GameBoy::new(None, cartridge);
    gb.reset_after_boot();
    gb.io_trace.borrow_mut().clear();

    Interpreter(&mut gb).interpret_op();
    let _clock = gb.clock_count;
    let _trace = gb.io_trace.borrow().clone();
    let _framebuffer = gb.ppu.borrow().screen.packed();
    gb.joypad = 0xFE;

    let mut snapshot = Vec::new();
    gb.save_state(Some(gbf_emu::FIXED_SAVE_STATE_UNIX_MS), &mut snapshot)
        .expect("save state works");
    gb.load_state(&mut std::io::Cursor::new(snapshot))
        .expect("load state works");
}

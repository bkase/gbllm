use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use gbf_emu::{
    BootMode, ClockCycles, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator,
    Framebuffer, ImeSnapshot,
};
use gbf_hw::joypad::Button;
use gbf_runtime::{boot, demo_bank0_rom_image, keyboard, text, video_commit};

const SCALE: usize = 4;
const SCREEN_WIDTH: usize = 160;
const SCREEN_HEIGHT: usize = 144;
const DEMO_PROMPT_X: u8 = 7;
const DEMO_PROMPT_Y: u8 = 8;
const DEMO_PROMPT: &[u8] = b"FA5 OK";
const KEYBOARD_PROMPT_X: u8 = 9;
const KEYBOARD_PROMPT_Y: u8 = 4;
const KEYBOARD_PROMPT_CURSOR: u8 = KEYBOARD_PROMPT_Y * 20 + KEYBOARD_PROMPT_X;
const KEYBOARD_PROMPT: &[u8] = b"ab";
const KEYBOARD_LAYOUT_X: u8 = 5;
const KEYBOARD_LAYOUT_Y: u8 = 10;
const SUBROUTINE_STEP_BUDGET: ClockCycles = ClockCycles(DMG_FRAME_CLOCK_CYCLES.0.saturating_mul(3));
const TEST_RETURN_PC: u16 = 0x3FF0;
const TEST_STACK_SP: u16 = 0xDFFE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScreenKind {
    DemoPrompt,
    Keyboard,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<OsString> = std::env::args_os().collect();
    let out_path = args
        .get(1)
        .cloned()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/review/f-a5/demo-screen.png"));
    let screen_kind = match args.get(2).and_then(|mode| mode.to_str()) {
        None | Some("demo") => ScreenKind::DemoPrompt,
        Some("keyboard") => ScreenKind::Keyboard,
        Some(other) => return Err(format!("unknown screen mode {other:?}").into()),
    };
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut emu = Emulator::builder()
        .boot_mode(BootMode::PostBootDmg)
        .policy(DeterminismPolicy::default())
        .load_rom(&demo_bank0_rom_image()?)?;
    emu.run_until_pc(
        boot::SCHEDULER_MAIN_LOOP_ADDR,
        CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(5)),
    )?;

    match screen_kind {
        ScreenKind::DemoPrompt => write_demo_prompt_to_bg_map(&mut emu)?,
        ScreenKind::Keyboard => prepare_keyboard_screen(&mut emu)?,
    }
    park_guest_cpu(&mut emu)?;
    emu.run_for(CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(4)))?;

    assert_expected_bg_map(&emu, screen_kind)?;

    let framebuffer = emu.framebuffer();
    assert_framebuffer_matches_screen(&framebuffer, screen_kind)?;

    let png = framebuffer_png(&framebuffer, SCALE);
    fs::write(&out_path, png)?;
    println!("{}", out_path.display());
    Ok(())
}

fn write_demo_prompt_to_bg_map(emu: &mut Emulator) -> Result<(), Box<dyn Error>> {
    let lcdc = emu.bus_read(gbf_hw::lcd::LCDC_REG)?;
    emu.bus_write(gbf_hw::lcd::LCDC_REG, 0)?;
    write_glyphs_to_bg_map(emu, DEMO_PROMPT_X, DEMO_PROMPT_Y, DEMO_PROMPT)?;
    emu.bus_write(gbf_hw::lcd::LCDC_REG, lcdc)?;
    Ok(())
}

fn prepare_keyboard_screen(emu: &mut Emulator) -> Result<(), Box<dyn Error>> {
    emu.poke(joypad_addr_prev(), 0)?;
    emu.poke(joypad_addr_cached(), 0)?;
    emu.poke(keyboard::PROMPT_CURSOR_ADDR.get(), KEYBOARD_PROMPT_CURSOR)?;
    emu.poke(keyboard::PROMPT_SUBMITTED_FLAG_ADDR.get(), 0)?;
    emu.poke(keyboard::KEYBOARD_CURSOR_ADDR.get(), 0)?;
    emu.poke(video_commit::COMMIT_QUEUE_HEAD_ADDR.get(), 0)?;
    emu.poke(video_commit::COMMIT_QUEUE_TAIL_ADDR.get(), 0)?;

    accept_keyboard_cell(emu, 0, b'a', 0)?;
    accept_keyboard_cell(emu, 1, b'b', 1)?;

    let lcdc = emu.bus_read(gbf_hw::lcd::LCDC_REG)?;
    emu.bus_write(gbf_hw::lcd::LCDC_REG, 0)?;
    apply_queued_keyboard_glyphs_to_bg_map(emu)?;
    write_keyboard_layout_to_bg_map(emu)?;
    emu.bus_write(gbf_hw::lcd::LCDC_REG, lcdc)?;
    Ok(())
}

fn accept_keyboard_cell(
    emu: &mut Emulator,
    keyboard_cursor: u8,
    glyph: u8,
    slot: u8,
) -> Result<(), Box<dyn Error>> {
    emu.poke(joypad_addr_prev(), 0)?;
    emu.poke(joypad_addr_cached(), Button::A.state_mask())?;
    emu.poke(keyboard::KEYBOARD_CURSOR_ADDR.get(), keyboard_cursor)?;

    call_keyboard_step(emu)?;

    let prompt_idx = KEYBOARD_PROMPT_CURSOR + slot;
    assert_byte(
        emu.peek(keyboard::PROMPT_BUFFER_BASE_ADDR.get() + u16::from(prompt_idx))?,
        glyph,
        "prompt buffer glyph",
    )?;
    assert_byte(
        emu.peek(keyboard::PROMPT_CURSOR_ADDR.get())?,
        prompt_idx + 1,
        "prompt cursor",
    )?;
    assert_byte(
        emu.peek(video_commit::COMMIT_QUEUE_TAIL_ADDR.get())?,
        slot + 1,
        "commit queue tail",
    )?;
    assert_queued_glyph(
        emu,
        slot,
        KEYBOARD_PROMPT_X + slot,
        KEYBOARD_PROMPT_Y,
        glyph,
    )
}

fn call_keyboard_step(emu: &mut Emulator) -> Result<(), Box<dyn Error>> {
    emu.poke(TEST_STACK_SP, (TEST_RETURN_PC & 0x00FF) as u8)?;
    emu.poke(TEST_STACK_SP + 1, (TEST_RETURN_PC >> 8) as u8)?;
    let mut regs = emu.regs();
    regs.pc = gbf_runtime::SECTION_KEYBOARD_ADDR;
    regs.sp = TEST_STACK_SP;
    regs.ime = ImeSnapshot::Disabled;
    emu.set_regs(regs)?;
    run_until(
        emu,
        SUBROUTINE_STEP_BUDGET,
        |emu| emu.regs().pc == TEST_RETURN_PC,
        "keyboard step return",
    )
}

fn run_until(
    emu: &mut Emulator,
    budget: ClockCycles,
    mut predicate: impl FnMut(&mut Emulator) -> bool,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    let deadline = emu.clock_count().0.saturating_add(budget.0);
    while emu.clock_count().0 < deadline {
        if predicate(emu) {
            return Ok(());
        }
        emu.step()?;
    }
    Err(format!("{label} did not reach expected state within {budget:?}").into())
}

fn assert_queued_glyph(
    emu: &Emulator,
    slot: u8,
    x: u8,
    y: u8,
    glyph: u8,
) -> Result<(), Box<dyn Error>> {
    let base = queue_slot_addr(slot);
    assert_byte(
        emu.peek(base)?,
        video_commit::UiCommitOpKind::PutGlyphCell as u8,
        "queue op kind",
    )?;
    assert_byte(emu.peek(base + 2)?, x, "queue op x")?;
    assert_byte(emu.peek(base + 3)?, y, "queue op y")?;
    assert_byte(emu.peek(base + 4)?, glyph, "queue op glyph")
}

fn apply_queued_keyboard_glyphs_to_bg_map(emu: &mut Emulator) -> Result<(), Box<dyn Error>> {
    let tail = emu.peek(video_commit::COMMIT_QUEUE_TAIL_ADDR.get())?;
    for slot in 0..tail {
        let base = queue_slot_addr(slot);
        assert_byte(
            emu.peek(base)?,
            video_commit::UiCommitOpKind::PutGlyphCell as u8,
            "queue op kind",
        )?;
        let x = emu.peek(base + 2)?;
        let y = emu.peek(base + 3)?;
        let glyph = emu.peek(base + 4)?;
        emu.bus_write(prompt_cell_addr(x, y), glyph)?;
    }
    Ok(())
}

fn queue_slot_addr(slot: u8) -> u16 {
    video_commit::COMMIT_QUEUE_BASE_ADDR.get()
        + u16::from(slot) * u16::from(video_commit::UI_COMMIT_WIRE_OP_BYTES)
}

fn assert_byte(actual: u8, expected: u8, label: &str) -> Result<(), Box<dyn Error>> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label}: expected {expected:#04x}, got {actual:#04x}").into())
    }
}

fn park_guest_cpu(emu: &mut Emulator) -> Result<(), Box<dyn Error>> {
    const SPIN_LOOP_ADDR: u16 = gbf_hw::memory::WRAM_BASE;
    // Park at `JR -2` with IME off so the scheduler cannot redraw these visual-review cells.
    emu.poke(SPIN_LOOP_ADDR, 0x18)?;
    emu.poke(SPIN_LOOP_ADDR + 1, 0xFE)?;
    let mut regs = emu.regs();
    regs.pc = SPIN_LOOP_ADDR;
    regs.ime = ImeSnapshot::Disabled;
    emu.set_regs(regs)?;
    Ok(())
}

fn write_glyphs_to_bg_map(
    emu: &mut Emulator,
    x: u8,
    y: u8,
    glyphs: &[u8],
) -> Result<(), Box<dyn Error>> {
    for (idx, glyph) in glyphs.iter().copied().enumerate() {
        emu.bus_write(prompt_cell_addr(x + idx as u8, y), glyph)?;
    }
    Ok(())
}

fn write_keyboard_layout_to_bg_map(emu: &mut Emulator) -> Result<(), Box<dyn Error>> {
    let layout = keyboard::default_layout();
    for row in 0..layout.rows {
        for col in 0..layout.columns {
            let idx = usize::from(row) * usize::from(layout.columns) + usize::from(col);
            let glyph = keyboard_cell_glyph(layout.cells[idx]);
            emu.bus_write(
                prompt_cell_addr(KEYBOARD_LAYOUT_X + col, KEYBOARD_LAYOUT_Y + row),
                glyph,
            )?;
        }
    }
    Ok(())
}

fn keyboard_cell_glyph(cell: keyboard::KeyboardCell) -> u8 {
    match cell {
        keyboard::KeyboardCell::Char(ch) => ch,
        keyboard::KeyboardCell::Special(keyboard::SpecialKey::Backspace) => b'<',
        keyboard::KeyboardCell::Special(keyboard::SpecialKey::Submit) => b'>',
        keyboard::KeyboardCell::Special(keyboard::SpecialKey::Shift) => b'^',
        keyboard::KeyboardCell::Special(keyboard::SpecialKey::Cancel) => b'!',
        keyboard::KeyboardCell::Empty => b' ',
    }
}

fn joypad_addr_prev() -> u16 {
    gbf_runtime::joypad::JOYPAD_PREV_STATE_ADDR.get()
}

fn joypad_addr_cached() -> u16 {
    gbf_runtime::joypad::JOYPAD_CACHED_STATE_ADDR.get()
}

fn prompt_cell_addr(x: u8, y: u8) -> u16 {
    video_commit::BOOTSTRAP_BG_MAP_ORIGIN + u16::from(y) * 32 + u16::from(x)
}

fn assert_expected_bg_map(emu: &Emulator, screen_kind: ScreenKind) -> Result<(), Box<dyn Error>> {
    for y in 0..18 {
        for x in 0..20 {
            let expected = expected_cell_glyph(x, y, screen_kind);
            let actual = emu.peek(prompt_cell_addr(x, y))?;
            if actual != expected {
                return Err(format!(
                    "{screen_kind:?} BG map mismatch at ({x}, {y}): expected {:?}, got {:?}",
                    char::from(expected),
                    char::from(actual)
                )
                .into());
            }
        }
    }
    Ok(())
}

fn assert_framebuffer_matches_screen(
    framebuffer: &Framebuffer,
    screen_kind: ScreenKind,
) -> Result<(), Box<dyn Error>> {
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            let actual = framebuffer.pixel(x, y) & 0x03;
            let expected = expected_screen_pixel(x, y, screen_kind)?;
            if actual != expected {
                return Err(format!(
                    "{screen_kind:?} framebuffer mismatch at ({x}, {y}): expected color {expected}, got {actual}"
                )
                .into());
            }
        }
    }
    Ok(())
}

fn expected_screen_pixel(
    x: usize,
    y: usize,
    screen_kind: ScreenKind,
) -> Result<u8, Box<dyn Error>> {
    let tile_x = x / 8;
    let tile_y = y / 8;
    let glyph = expected_cell_glyph(tile_x as u8, tile_y as u8, screen_kind);
    let tile_offset = usize::from(glyph) * text::FONT_BYTES_PER_TILE;
    if tile_offset + text::FONT_BYTES_PER_TILE > text::font_bytes().len() {
        return Err(format!("glyph {glyph} is outside the installed M0 font").into());
    }
    let row = y % 8;
    let lo = text::font_bytes()[tile_offset + row * 2];
    let hi = text::font_bytes()[tile_offset + row * 2 + 1];
    let bit = 7 - (x % 8);
    Ok(((hi >> bit) & 1) << 1 | ((lo >> bit) & 1))
}

fn expected_cell_glyph(x: u8, y: u8, screen_kind: ScreenKind) -> u8 {
    match screen_kind {
        ScreenKind::DemoPrompt => {
            if y == DEMO_PROMPT_Y
                && x >= DEMO_PROMPT_X
                && usize::from(x - DEMO_PROMPT_X) < DEMO_PROMPT.len()
            {
                DEMO_PROMPT[usize::from(x - DEMO_PROMPT_X)]
            } else {
                0
            }
        }
        ScreenKind::Keyboard => keyboard_screen_cell_glyph(x, y),
    }
}

fn keyboard_screen_cell_glyph(x: u8, y: u8) -> u8 {
    if y == KEYBOARD_PROMPT_Y
        && x >= KEYBOARD_PROMPT_X
        && usize::from(x - KEYBOARD_PROMPT_X) < KEYBOARD_PROMPT.len()
    {
        return KEYBOARD_PROMPT[usize::from(x - KEYBOARD_PROMPT_X)];
    }

    let layout = keyboard::default_layout();
    if y >= KEYBOARD_LAYOUT_Y
        && y < KEYBOARD_LAYOUT_Y + layout.rows
        && x >= KEYBOARD_LAYOUT_X
        && x < KEYBOARD_LAYOUT_X + layout.columns
    {
        let row = usize::from(y - KEYBOARD_LAYOUT_Y);
        let col = usize::from(x - KEYBOARD_LAYOUT_X);
        let idx = row * usize::from(layout.columns) + col;
        return keyboard_cell_glyph(layout.cells[idx]);
    }

    0
}

fn framebuffer_png(framebuffer: &Framebuffer, scale: usize) -> Vec<u8> {
    let width = SCREEN_WIDTH * scale;
    let height = SCREEN_HEIGHT * scale;
    let palette = Framebuffer::dmg_palette();
    let mut rgb = Vec::with_capacity((width * 3 + 1) * height);
    for y in 0..SCREEN_HEIGHT {
        for _ in 0..scale {
            rgb.extend_from_slice(&[0]);
            for x in 0..SCREEN_WIDTH {
                let color = palette[usize::from(framebuffer.pixel(x, y) & 0x03)];
                for _ in 0..scale {
                    rgb.extend_from_slice(&[color.r, color.g, color.b]);
                }
            }
        }
    }

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1A\n");

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    write_png_chunk(&mut png, b"IHDR", &ihdr);
    write_png_chunk(&mut png, b"IDAT", &zlib_stored_blocks(&rgb));
    write_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_data = Vec::with_capacity(kind.len() + data.len());
    crc_data.extend_from_slice(kind);
    crc_data.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_data).to_be_bytes());
}

fn zlib_stored_blocks(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (idx, chunk) in data.chunks(u16::MAX as usize).enumerate() {
        let final_block = idx + 1 == data.len().div_ceil(u16::MAX as usize);
        out.push(u8::from(final_block));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1_u32;
    let mut b = 0_u32;
    for &byte in data {
        a = (a + u32::from(byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use gbf_emu::{
    BootMode, CycleBudget, DMG_FRAME_CLOCK_CYCLES, DeterminismPolicy, Emulator, Framebuffer,
    ImeSnapshot,
};
use gbf_runtime::{boot, demo_bank0_rom_image, text, video_commit};

const SCALE: usize = 4;
const SCREEN_WIDTH: usize = 160;
const SCREEN_HEIGHT: usize = 144;
const PROMPT_X: u8 = 7;
const PROMPT_Y: u8 = 8;
const PROMPT: &[u8] = b"FA5 OK";

fn main() -> Result<(), Box<dyn Error>> {
    let out_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/review/f-a5/demo-screen.png"));
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

    write_prompt_to_bg_map(&mut emu)?;
    park_guest_cpu(&mut emu)?;
    emu.run_for(CycleBudget::Clock(DMG_FRAME_CLOCK_CYCLES.saturating_mul(4)))?;

    for (idx, &expected) in PROMPT.iter().enumerate() {
        let actual = emu.peek(prompt_cell_addr(idx))?;
        if actual != expected {
            return Err(format!(
                "demo screen BG map mismatch at cell {idx}: expected {:?}, got {:?}",
                char::from(expected),
                char::from(actual)
            )
            .into());
        }
    }

    let framebuffer = emu.framebuffer();
    assert_framebuffer_matches_prompt(&framebuffer)?;

    let png = framebuffer_png(&framebuffer, SCALE);
    fs::write(&out_path, png)?;
    println!("{}", out_path.display());
    Ok(())
}

fn write_prompt_to_bg_map(emu: &mut Emulator) -> Result<(), Box<dyn Error>> {
    let lcdc = emu.bus_read(gbf_hw::lcd::LCDC_REG)?;
    emu.bus_write(gbf_hw::lcd::LCDC_REG, 0)?;
    for (idx, glyph) in PROMPT.iter().copied().enumerate() {
        emu.bus_write(prompt_cell_addr(idx), glyph)?;
    }
    emu.bus_write(gbf_hw::lcd::LCDC_REG, lcdc)?;
    Ok(())
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

fn prompt_cell_addr(idx: usize) -> u16 {
    video_commit::BOOTSTRAP_BG_MAP_ORIGIN
        + u16::from(PROMPT_Y) * 32
        + u16::from(PROMPT_X)
        + idx as u16
}

fn assert_framebuffer_matches_prompt(framebuffer: &Framebuffer) -> Result<(), Box<dyn Error>> {
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            let actual = framebuffer.pixel(x, y) & 0x03;
            let expected = expected_screen_pixel(x, y)?;
            if actual != expected {
                return Err(format!(
                    "demo screen framebuffer mismatch at ({x}, {y}): expected color {expected}, got {actual}"
                )
                .into());
            }
        }
    }
    Ok(())
}

fn expected_screen_pixel(x: usize, y: usize) -> Result<u8, Box<dyn Error>> {
    let tile_x = x / 8;
    let tile_y = y / 8;
    let glyph = if tile_y == usize::from(PROMPT_Y)
        && tile_x >= usize::from(PROMPT_X)
        && tile_x < usize::from(PROMPT_X) + PROMPT.len()
    {
        PROMPT[tile_x - usize::from(PROMPT_X)]
    } else {
        0
    };
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

fn framebuffer_png(framebuffer: &Framebuffer, scale: usize) -> Vec<u8> {
    let width = SCREEN_WIDTH * scale;
    let height = SCREEN_HEIGHT * scale;
    let palette = Framebuffer::dmg_palette();
    let mut rgb = Vec::with_capacity((width * 3 + 1) * height);
    for y in 0..SCREEN_HEIGHT {
        for _ in 0..scale {
            rgb.push(0);
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

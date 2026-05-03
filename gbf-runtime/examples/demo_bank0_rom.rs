use std::error::Error;
use std::fs;
use std::path::PathBuf;

use gbf_runtime::{
    compute_runtime_nucleus_hash, demo_bank0_rom_image, normalized_bank0_image_for_test,
    runtime_nucleus_section_sizes,
};

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/review/f-a5"));
    fs::create_dir_all(&out_dir)?;

    let bank0 = normalized_bank0_image_for_test();
    let rom = demo_bank0_rom_image()?;

    let hash = compute_runtime_nucleus_hash(&bank0);
    fs::write(out_dir.join("demo_bank0_rom.gb"), rom)?;
    fs::write(
        out_dir.join("runtime_nucleus_hash.txt"),
        format!("{hash}\n"),
    )?;
    fs::write(
        out_dir.join("bank0_section_sizes.json"),
        serde_json::to_vec_pretty(&runtime_nucleus_section_sizes())?,
    )?;
    Ok(())
}

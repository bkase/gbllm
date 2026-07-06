//! Emit the deployable interactive-shell ROM as a .gb file.
//!
//! Usage: `cargo run --release -p gbf-bench --bin emit-shell-rom -- [export_dir] [out.gb] [n_tokens]`
//! Defaults: the committed d192 export, `artifacts/builds/gbllm-shell-d192.gb`, 200 tokens.
//! The emitted image is the same builder output the demo-acceptance and
//! latency gates ran byte-exactly in gameroy; flash it or load it in any
//! accurate emulator (MBC5, no SRAM required).

use gbf_bench::d192_real::D192_REAL_EXPORT_DIR;
use gbf_bench::shell::{SHELL_TEMPERATURE, SHELL_TOP_K, shell_font_tiles};
use gbf_bench::stateful::load_state_checkpoint;
use gbf_kernel::asm_impl_shell::build_state_shell_rom;
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::IntStateLoweredModel;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let export_dir = args
        .get(1)
        .map_or_else(|| D192_REAL_EXPORT_DIR.to_owned(), Clone::clone);
    let out = args.get(2).map_or_else(
        || PathBuf::from("artifacts/builds/gbllm-shell-d192.gb"),
        PathBuf::from,
    );
    let n_tokens: u8 = args.get(3).map_or(Ok(200), |s| s.parse())?;

    let bundle = load_state_checkpoint(&PathBuf::from(&export_dir))?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)?;
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(SHELL_TOP_K, step, SHELL_TEMPERATURE)?;
    let rom = build_state_shell_rom(&lowered, &cfg, n_tokens, &shell_font_tiles())?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &rom.rom)?;
    println!(
        "wrote {} ({} bytes, {} banks) from {}",
        out.display(),
        rom.rom.len(),
        rom.rom.len() / 16384,
        export_dir
    );
    Ok(())
}

//! Compatibility wrapper for the compiler-owned interactive-subword profile.
//!
//! Usage:
//! `cargo run --release -p gbf-bench --bin emit-interactive-subword-rom -- \
//!     <bridged_dir> [out.gb] [n_gen] [tokenizer.json]`
//!
//! The production dense d192 checkpoint selects the exact SRAM-full head and
//! therefore requires an MBC5 cartridge/emulator with 8 KiB RAM enabled.

use std::path::{Path, PathBuf};

use gbf_codegen::compile_state_subword::{
    InteractiveSubwordCompileOptions, compile_interactive_subword, interactive_subword_symbols,
};
use gbf_kernel::asm_impl_shell::SUBWORD_SHELL_PROMPT_CAP;

fn symbols_path(rom_path: &Path) -> PathBuf {
    rom_path.with_extension("sym")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(bridged_dir) = args.get(1).map(PathBuf::from) else {
        return Err(
            "usage: emit-interactive-subword-rom <bridged_dir> [out.gb] [n_gen] [tokenizer.json]"
                .into(),
        );
    };
    let out = args.get(2).map_or_else(
        || PathBuf::from("artifacts/builds/gbllm-dense-d192-interactive.gb"),
        PathBuf::from,
    );
    let n_gen: u8 = args.get(3).map_or(Ok(24), |value| value.parse())?;

    let bpe_path = args.get(4).map_or_else(
        || bridged_dir.join("tokenizer/gbllm_bpe.v2.json"),
        PathBuf::from,
    );
    let options = InteractiveSubwordCompileOptions {
        n_tokens: n_gen,
        ..InteractiveSubwordCompileOptions::default()
    };
    let compiled = compile_interactive_subword(&bridged_dir.join("ckpt"), &bpe_path, &options)?;
    let rom = &compiled.rom;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &rom.rom)?;
    let sym = symbols_path(&out);
    std::fs::write(&sym, interactive_subword_symbols(rom))?;

    println!("wrote {} ({} bytes)", out.display(), rom.rom.len());
    println!("wrote {}", sym.display());
    let topology = compiled.program.topology;
    println!(
        "  topology=d{} ff{} blocks={} experts={} vocab={}",
        topology.d_model, topology.d_ff, topology.n_blocks, topology.n_experts, topology.vocab
    );
    println!(
        "  banks={} rom_size={:?} driver={}B ui={}B tokenizer={}B head_storage={:?}",
        rom.bank_count,
        rom.rom_size,
        rom.driver_bytes,
        rom.ui_bank_bytes,
        rom.tokenizer_bank_bytes,
        rom.paged_head_storage,
    );
    println!(
        "  prompt=joypad keyboard ({} bytes) sampler=k{}/T{} seed={:#06x} n_gen={n_gen}",
        SUBWORD_SHELL_PROMPT_CAP,
        compiled.report.sampler.top_k,
        compiled.report.sampler.requested_temperature,
        rom.rng_seed
    );
    println!("  controls=D-pad move, A type, B delete, START generate");
    Ok(())
}

//! Emit the playable wide-vocabulary cartridge: joypad keyboard, exact
//! on-device BPE prompt encoding, recurrent prefill, and subword generation.
//!
//! Usage:
//! `cargo run --release -p gbf-bench --bin emit-interactive-subword-rom -- \
//!     <bridged_dir> [out.gb] [n_gen] [tokenizer.json]`
//!
//! The production dense d192 checkpoint selects the exact SRAM-full head and
//! therefore requires an MBC5 cartridge/emulator with 8 KiB RAM enabled.

use std::path::{Path, PathBuf};

use gbf_bench::stateful::load_state_checkpoint;
use gbf_bench::subword_demo::subword_font_tiles;
use gbf_kernel::asm_impl_shell::{SUBWORD_SHELL_PROMPT_CAP, build_state_subword_shell_rom};
use gbf_kernel::asm_impl_state::{S_RNG_ADDR, S_SAMPLED_ADDR, S_SAMPLED_HI_ADDR};
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::{IntStateLoweredModel, LogitPaging};

/// Coherence-first sampling policy shared with the proven fixed-prompt ROM.
const TOP_K: u8 = 4;
const TEMPERATURE: f64 = 0.6;

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

    let bundle = load_state_checkpoint(&bridged_dir.join("ckpt"))?;
    let topology = bundle.topology;
    assert_eq!(
        topology.logit_paging,
        LogitPaging::Paged,
        "interactive subword shell requires a wide paged vocabulary"
    );
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)?;

    let bpe_path = args.get(4).map_or_else(
        || bridged_dir.join("tokenizer/gbllm_bpe.v2.json"),
        PathBuf::from,
    );
    let bpe = gbf_data::bpe::BpeModel::from_json(&std::fs::read_to_string(&bpe_path)?)?;
    assert_eq!(bpe.vocab_size(), topology.vocab, "BPE vocab matches model");
    let id_bytes: Vec<Vec<u8>> = (0..topology.vocab)
        .map(|id| bpe.id_bytes(id as u16).expect("id in vocab").to_vec())
        .collect();

    let sampler =
        SamplerConfig::from_temperature(TOP_K, lowered.logit_dequant_step(), TEMPERATURE)?;
    let rom = build_state_subword_shell_rom(
        &lowered,
        &sampler,
        n_gen,
        &subword_font_tiles(),
        &id_bytes,
        bpe.merges(),
    )?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &rom.rom)?;
    let sym = symbols_path(&out);
    std::fs::write(
        &sym,
        format!(
            "00:{:04x} subword_shell_idle\n\
             00:{:04x} subword_tokenize_done\n\
             00:{:04x} subword_warm_boundary\n\
             00:{:04x} subword_forward_pass\n\
             00:{:04x} subword_token_boundary\n\
             00:{:04x} subword_generation_done\n\
             00:{:04x} subword_prompt_bytes\n\
             00:{:04x} subword_prompt_byte_len\n\
             00:{:04x} subword_prompt_token_ids\n\
             00:{:04x} subword_prompt_token_len\n\
             00:{S_RNG_ADDR:04x} subword_rng\n\
             00:{S_SAMPLED_ADDR:04x} subword_sampled_lo\n\
             00:{S_SAMPLED_HI_ADDR:04x} subword_sampled_hi\n",
            rom.idle_pc,
            rom.tokenize_done_pc,
            rom.warm_boundary_pc,
            rom.forward_pass_pc,
            rom.token_boundary_pc,
            rom.gen_done_pc,
            rom.prompt_bytes_addr,
            rom.prompt_byte_len_addr,
            rom.prompt_ids_addr,
            rom.prompt_token_len_addr,
        ),
    )?;

    println!("wrote {} ({} bytes)", out.display(), rom.rom.len());
    println!("wrote {}", sym.display());
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
        "  prompt=joypad keyboard ({} bytes) sampler=k{TOP_K}/T{TEMPERATURE} seed={:#06x} n_gen={n_gen}",
        SUBWORD_SHELL_PROMPT_CAP, rom.rng_seed
    );
    println!("  controls=D-pad move, A type, B delete, START generate");
    Ok(())
}

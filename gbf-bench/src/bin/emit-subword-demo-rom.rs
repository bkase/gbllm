//! Emit a self-booting wide-vocabulary subword demo cartridge.
//!
//! Usage:
//! `cargo run --release -p gbf-bench --bin emit-subword-demo-rom -- \
//!     <bridged_dir> [out.gb] [prompt] [n_gen] [tokenizer.json]`
//!
//! Dense d192 selects the exact SRAM-full head, which requires an MBC5
//! cartridge with 8 KiB RAM and stays below the 30-second/token DMG budget.
//! MoE bundles remain supported through the streamed V2 path.

use std::path::PathBuf;

use gbf_bench::stateful::load_state_checkpoint;
use gbf_bench::subword_demo::subword_font_tiles;
use gbf_kernel::asm_impl_shell::build_state_subword_demo_rom_baked;
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::{IntStateLoweredModel, LogitPaging};

/// Coherence-first deployed policy. The previous k=8/T=0.8 path is still
/// available to callers, but this narrower default suppresses weak-tail tokens
/// without changing model inference or the exact head computation.
const TOP_K: u8 = 4;
const TEMPERATURE: f64 = 0.6;
const RNG_SEED: u16 = 0x5EED;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(bridged_dir) = args.get(1).map(PathBuf::from) else {
        return Err(
            "usage: emit-subword-demo-rom <bridged_dir> [out.gb] [prompt] [n_gen] [tokenizer.json]"
                .into(),
        );
    };
    let out = args.get(2).map_or_else(
        || PathBuf::from("artifacts/builds/gbllm-dense-d192-subword-demo.gb"),
        PathBuf::from,
    );
    let prompt = args
        .get(3)
        .map_or_else(|| "Once upon a time".to_owned(), Clone::clone);
    let n_gen: u8 = args.get(4).map_or(Ok(24), |value| value.parse())?;

    let ckpt = bridged_dir.join("ckpt");
    let bundle = load_state_checkpoint(&ckpt)?;
    let topo = bundle.topology;
    assert_eq!(
        topo.logit_paging,
        LogitPaging::Paged,
        "subword demo requires a paged wide-vocabulary checkpoint"
    );
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)?;

    let bpe_path = args.get(5).map_or_else(
        || bridged_dir.join("tokenizer/gbllm_bpe.v2.json"),
        PathBuf::from,
    );
    let bpe = gbf_data::bpe::BpeModel::from_json(&std::fs::read_to_string(&bpe_path)?)?;
    assert_eq!(bpe.vocab_size(), topo.vocab, "BPE vocab matches model");
    let id_bytes: Vec<Vec<u8>> = (0..topo.vocab)
        .map(|id| bpe.id_bytes(id as u16).expect("id in vocab").to_vec())
        .collect();
    let prompt_ids = bpe.encode(&prompt);
    assert!(
        !prompt_ids.is_empty(),
        "prompt must encode to at least one id"
    );

    let cfg = SamplerConfig::from_temperature(TOP_K, lowered.logit_dequant_step(), TEMPERATURE)?;
    let rom = build_state_subword_demo_rom_baked(
        &lowered,
        &cfg,
        n_gen,
        &subword_font_tiles(),
        &id_bytes,
        &prompt_ids,
        RNG_SEED,
    )?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &rom.rom)?;

    println!("wrote {} ({} bytes)", out.display(), rom.rom.len());
    println!(
        "  topology=d{} ff{} blocks={} experts={} vocab={}",
        topo.d_model, topo.d_ff, topo.n_blocks, topo.n_experts, topo.vocab
    );
    println!(
        "  banks={} rom_size={:?} driver={}B head_storage={:?}",
        rom.bank_count, rom.rom_size, rom.driver_bytes, rom.paged_head_storage
    );
    println!("  sampler=k{TOP_K}/T{TEMPERATURE} rng_seed={RNG_SEED:#06x} n_gen={n_gen}");
    println!("  prompt={prompt:?} ids={prompt_ids:?}");
    Ok(())
}

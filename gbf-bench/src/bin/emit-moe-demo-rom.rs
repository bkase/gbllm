//! Emit a FLASHABLE, self-booting subword MoE d192x8 demo cartridge (.gb).
//!
//! Usage: `cargo run --release -p gbf-bench --bin emit-moe-demo-rom -- \
//!     [bridged_dir] [out.gb] [prompt] [n_gen]`
//! Defaults: bridged_dir=/private/tmp/claude-501/parity_moe,
//!           out=artifacts/builds/gbllm-moe-d192x8-demo.gb,
//!           prompt="Once upon a time", n_gen=24.
//!
//! Unlike the poked demo ROM (driven by the bench harness), the prompt is
//! BAKED into ROM data: on boot the cartridge copies the pre-encoded prompt
//! ids into WRAM, seeds the RNG, and starts warmup + generation with NO
//! external poke. Flash it or load it in any accurate MBC5 emulator (no SRAM).

use std::path::PathBuf;

use gbf_bench::stateful::load_state_checkpoint;
use gbf_bench::subword_demo::subword_font_tiles;
use gbf_kernel::asm_impl_shell::build_state_moe_demo_rom_baked;
use gbf_kernel::decode::SamplerConfig;
use gbf_kernel::state_model_ref::{IntStateLoweredModel, LogitPaging};

/// Sampler config the deployed subword student generates under (matches the
/// real `moe_subword_demo` gate: top-8, temperature 0.8).
const TOP_K: u8 = 8;
const TEMPERATURE: f64 = 0.8;
/// Baked RNG seed (XorShift16); the on-device run and any host mirror must use
/// this same seed for byte-exact parity.
const RNG_SEED: u16 = 0x5EED;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let bridged_dir = args.get(1).map_or_else(
        || PathBuf::from("/private/tmp/claude-501/parity_moe"),
        PathBuf::from,
    );
    let out = args.get(2).map_or_else(
        || PathBuf::from("artifacts/builds/gbllm-moe-d192x8-demo.gb"),
        PathBuf::from,
    );
    let prompt = args
        .get(3)
        .map_or_else(|| "Once upon a time".to_owned(), Clone::clone);
    let n_gen: u8 = args.get(4).map_or(Ok(24), |s| s.parse())?;

    // Load + lower the real bridged MoE student.
    let ckpt = bridged_dir.join("ckpt");
    let bundle = load_state_checkpoint(&ckpt)?;
    let topo = bundle.topology;
    assert!(topo.is_moe(), "expected MoE (n_experts={})", topo.n_experts);
    assert_eq!(topo.logit_paging, LogitPaging::Paged, "vocab-1024 is Paged");
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)?;

    // Load the deployed BPE artifact for id_bytes + prompt encode.
    let bpe_path = bridged_dir.join("tokenizer/gbllm_bpe.v2.json");
    let bpe_text = std::fs::read_to_string(&bpe_path)?;
    let bpe = gbf_data::bpe::BpeModel::from_json(&bpe_text)?;
    assert_eq!(bpe.vocab_size(), topo.vocab, "BPE vocab matches the model");
    let id_bytes: Vec<Vec<u8>> = (0..topo.vocab)
        .map(|id| bpe.id_bytes(id as u16).expect("id in vocab").to_vec())
        .collect();

    let prompt_ids = bpe.encode(&prompt);
    assert!(
        !prompt_ids.is_empty(),
        "prompt encodes to a nonempty id list"
    );

    let font = subword_font_tiles();
    let step = lowered.logit_dequant_step();
    let cfg = SamplerConfig::from_temperature(TOP_K, step, TEMPERATURE)?;

    let rom = build_state_moe_demo_rom_baked(
        &lowered,
        &cfg,
        n_gen,
        &font,
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
        "  banks={} rom_size={:?} driver={}B ui_bank={}B table={}B id_bytes_stride={}",
        rom.bank_count,
        rom.rom_size,
        rom.driver_bytes,
        rom.ui_bank_bytes,
        rom.table_bytes,
        rom.id_bytes_geom.stride,
    );
    println!(
        "  WRAM: state@{:#06x} prompt_ids@{:#06x} plen@{:#06x} go@{:#06x}",
        rom.layout.state, rom.prompt_ids_addr, rom.prompt_len_addr, rom.go_addr,
    );
    println!("  prompt   = {prompt:?}");
    println!("  n_gen    = {n_gen}  rng_seed = {RNG_SEED:#06x}");
    println!(
        "  encoded prompt ids ({}) = {:?}",
        prompt_ids.len(),
        prompt_ids
    );
    Ok(())
}

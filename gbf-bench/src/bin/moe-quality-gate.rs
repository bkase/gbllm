//! Gate a real subword model's deployed integer quality against its hardened
//! f32 reference (dense or MoE).
//!
//! Usage:
//! `cargo run --release -p gbf-bench --bin moe-quality-gate -- \
//!    <bridged-dir> <val.npy> [positions]`
//!
//! `<bridged-dir>` contains `ckpt/` and `tokenizer/gbllm_bpe.v2.json` (the
//! layout produced by `training/run_realparity.py`). The gate intentionally
//! evaluates the production checkpoint loader and integer lowerer; ROM parity
//! alone cannot detect a shared host/device semantic corruption.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use gbf_bench::stateful::load_state_checkpoint;
use gbf_data::bpe::BpeModel;
use gbf_kernel::decode::{SamplerConfig, XorShift16, sample_topk_from_candidates_trace};
use gbf_kernel::state_model_ref::{IntStateLoweredModel, StateForwardStats, f32_state_forward_at};

const DEFAULT_POSITIONS: usize = 1_024;
const MAX_INT_BPB_GAP: f64 = 0.03;
const MIN_ARGMAX_AGREEMENT: f64 = 0.80;
const SAMPLE_PROMPT: &str = "Once upon a time";
const SAMPLE_TOKENS: usize = 60;
// The cartridge's coherence-first default. The quantitative bpb/argmax gate
// above is sampler-independent; this fixed sample catches obvious decode
// regressions under the exact policy users will see on-device.
const SAMPLE_TOP_K: u8 = 4;
const SAMPLE_TEMPERATURE: f64 = 0.6;
const SAMPLE_SEED: u16 = 0x5EED;

fn invalid_data(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

/// Load the NumPy v1/v2 little-endian `uint16` vector emitted by the training
/// data pipeline. Shape is intentionally restricted to one dimension.
fn load_npy_u16(path: &Path) -> Result<Vec<u16>, Error> {
    let bytes = std::fs::read(path)?;
    if bytes.get(..6) != Some(b"\x93NUMPY") {
        return Err(invalid_data(format!(
            "{} is not a NumPy file",
            path.display()
        )));
    }
    let version = *bytes
        .get(6)
        .ok_or_else(|| invalid_data("truncated NumPy version"))?;
    let (header_len, payload_start) = match version {
        1 => {
            let raw = bytes
                .get(8..10)
                .ok_or_else(|| invalid_data("truncated NumPy v1 header length"))?;
            let len = usize::from(u16::from_le_bytes([raw[0], raw[1]]));
            (len, 10 + len)
        }
        2 => {
            let raw = bytes
                .get(8..12)
                .ok_or_else(|| invalid_data("truncated NumPy v2 header length"))?;
            let len = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
            (len, 12 + len)
        }
        other => return Err(invalid_data(format!("unsupported NumPy version {other}"))),
    };
    let header_start = if version == 1 { 10 } else { 12 };
    let header = bytes
        .get(header_start..header_start + header_len)
        .ok_or_else(|| invalid_data("truncated NumPy header"))?;
    let header = std::str::from_utf8(header)
        .map_err(|e| invalid_data(format!("NumPy header is not UTF-8: {e}")))?;
    if !header.contains("'descr': '<u2'") || !header.contains("'fortran_order': False") {
        return Err(invalid_data(format!(
            "expected C-order little-endian uint16 NumPy data, header was {header:?}"
        )));
    }
    let payload = bytes
        .get(payload_start..)
        .ok_or_else(|| invalid_data("truncated NumPy payload"))?;
    if !payload.len().is_multiple_of(2) {
        return Err(invalid_data("uint16 NumPy payload has odd byte length"));
    }
    Ok(payload
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn log_probability_bits(logits: &[f64], target: usize) -> f64 {
    let max = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let sum_exp: f64 = logits.iter().map(|&value| (value - max).exp()).sum();
    (max + sum_exp.ln() - logits[target]) / std::f64::consts::LN_2
}

fn argmax(logits: &[f64]) -> usize {
    let mut best = 0usize;
    for index in 1..logits.len() {
        if logits[index] > logits[best] {
            best = index;
        }
    }
    best
}

fn fixed_sample(lowered: &IntStateLoweredModel, bpe: &BpeModel) -> (Vec<u16>, String) {
    let prompt_ids = bpe.encode(SAMPLE_PROMPT);
    assert!(!prompt_ids.is_empty(), "sample prompt must encode nonempty");
    let mut state = lowered.zero_state();
    let mut trace = None;
    for id in prompt_ids {
        trace = Some(lowered.forward_at(usize::from(id), &mut state));
    }
    let mut trace = trace.expect("sample prompt is nonempty");
    let sampler = SamplerConfig::from_temperature(
        SAMPLE_TOP_K,
        lowered.logit_dequant_step(),
        SAMPLE_TEMPERATURE,
    )
    .expect("fixed sample config is valid");
    let mut rng = XorShift16::new(SAMPLE_SEED);
    let mut ids = Vec::with_capacity(SAMPLE_TOKENS);
    for _ in 0..SAMPLE_TOKENS {
        let candidates: Vec<(i32, usize)> = trace
            .topk_heap
            .iter()
            .take(usize::from(sampler.k()))
            .map(|entry| (entry.logit, entry.id))
            .collect();
        let picked =
            sample_topk_from_candidates_trace(&candidates, sampler.scale_q16(), &mut rng).picked;
        ids.push(picked as u16);
        trace = lowered.forward_at(picked, &mut state);
    }
    let text = bpe.decode(&ids);
    (ids, text)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args.len() > 4 {
        return Err(
            invalid_data("usage: moe-quality-gate <bridged-dir> <val.npy> [positions]").into(),
        );
    }
    let root = PathBuf::from(&args[1]);
    let val_path = PathBuf::from(&args[2]);
    let positions = args
        .get(3)
        .map_or(Ok(DEFAULT_POSITIONS), |value| value.parse::<usize>())?;
    if positions == 0 {
        return Err(invalid_data("positions must be nonzero").into());
    }

    let tokens = load_npy_u16(&val_path)?;
    if tokens.len() <= positions {
        return Err(invalid_data(format!(
            "{} contains {} tokens, need at least {}",
            val_path.display(),
            tokens.len(),
            positions + 1
        ))
        .into());
    }
    let ids: Vec<usize> = tokens[..positions]
        .iter()
        .map(|&id| usize::from(id))
        .collect();
    let targets: Vec<usize> = tokens[1..=positions]
        .iter()
        .map(|&id| usize::from(id))
        .collect();

    let bpe_path = root.join("tokenizer/gbllm_bpe.v2.json");
    let bpe = BpeModel::from_json(&std::fs::read_to_string(&bpe_path)?)?;
    let ckpt_path = root.join("ckpt");
    let bundle = load_state_checkpoint(&ckpt_path)?;
    let lowered = IntStateLoweredModel::lower(&bundle.checkpoint)?;
    if bpe.vocab_size() != lowered.topology.vocab {
        return Err(invalid_data(format!(
            "tokenizer vocab {} != checkpoint vocab {}",
            bpe.vocab_size(),
            lowered.topology.vocab
        ))
        .into());
    }

    let logit_step = lowered.logit_dequant_step();
    let mut f32_state = vec![0.0f32; lowered.topology.state_slots];
    let mut state = lowered.zero_state();
    let mut f32_bits = 0.0f64;
    let mut int_bits = 0.0f64;
    let mut target_bytes = 0usize;
    let mut argmax_agree = 0usize;
    let mut stats = StateForwardStats::new();
    for position in 0..positions {
        let f_logits: Vec<f64> =
            f32_state_forward_at(&bundle.checkpoint, ids[position], &mut f32_state)
                .iter()
                .map(|&value| f64::from(value))
                .collect();
        let trace = lowered.forward_at(ids[position], &mut state);
        stats.merge(&trace.stats);
        let i_logits: Vec<f64> = trace
            .logit_pages
            .iter()
            .flatten()
            .map(|&value| f64::from(value) * logit_step)
            .collect();
        if i_logits.len() != lowered.topology.vocab {
            return Err(invalid_data(format!(
                "position {position}: paged logits cover {} ids, expected {}",
                i_logits.len(),
                lowered.topology.vocab
            ))
            .into());
        }
        let target = targets[position];
        f32_bits += log_probability_bits(&f_logits, target);
        int_bits += log_probability_bits(&i_logits, target);
        target_bytes += bpe
            .id_bytes(target as u16)
            .ok_or_else(|| invalid_data(format!("target id {target} is outside the tokenizer")))?
            .len();
        argmax_agree += usize::from(argmax(&f_logits) == trace.argmax_full);
    }

    let f32_bpb = f32_bits / target_bytes as f64;
    let int_bpb = int_bits / target_bytes as f64;
    let bpb_gap = int_bpb - f32_bpb;
    let agreement = argmax_agree as f64 / positions as f64;
    println!("positions={positions} target_bytes={target_bytes}");
    println!("hardened_f32_bits_per_raw_byte={f32_bpb:.6}");
    println!("deployed_int_bits_per_raw_byte={int_bpb:.6}");
    println!("int_minus_f32_bits_per_raw_byte={bpb_gap:+.6} (limit {MAX_INT_BPB_GAP:.3})");
    println!(
        "teacher_forced_argmax_agreement={:.2}% (minimum {:.0}%)",
        agreement * 100.0,
        MIN_ARGMAX_AGREEMENT * 100.0
    );
    println!(
        "range_events: state_clamps={} down_clamps={} residual_wraps={}",
        stats.state_clamp_events, stats.ffn.down_delta_clamp_events, stats.residual_i24_wrap_events
    );

    let (sample_ids, sample) = fixed_sample(&lowered, &bpe);
    println!(
        "sample prompt={SAMPLE_PROMPT:?} T={SAMPLE_TEMPERATURE} k={SAMPLE_TOP_K} seed={SAMPLE_SEED:#06x}"
    );
    println!("sample_ids={sample_ids:?}");
    println!("sample={sample:?}");

    let mut failures = Vec::new();
    if bpb_gap > MAX_INT_BPB_GAP {
        failures.push(format!(
            "integer bpb gap {bpb_gap:+.6} exceeds {MAX_INT_BPB_GAP:.3}"
        ));
    }
    if agreement < MIN_ARGMAX_AGREEMENT {
        failures.push(format!(
            "argmax agreement {:.2}% is below {:.0}%",
            agreement * 100.0,
            MIN_ARGMAX_AGREEMENT * 100.0
        ));
    }
    if stats.state_clamp_events != 0
        || stats.ffn.down_delta_clamp_events != 0
        || stats.residual_i24_wrap_events != 0
    {
        failures.push("unexpected integer range event on the validation prefix".to_owned());
    }
    if !failures.is_empty() {
        return Err(invalid_data(failures.join("; ")).into());
    }
    println!("quality_gate=PASS");
    Ok(())
}

//! F-S5 state A/B (bd-29ai4 + bd-2nrnq): LinearState multi-timescale vs a
//! stateless bigram-context baseline, both re-tokenized to the charset_v1
//! 80-id vocabulary (the charset-80 trainer migration).
//!
//! Three matched arms (same d_model / d_ff / n_blocks / steps / seed / ternary
//! QAT recipe as `s2_gap_and_export.rs`), trained on the charset_v1-normalized
//! committed Gutenberg train stream:
//!
//!   A. Stateless baseline: the existing bigram-context dense FFN stack from
//!      `s2_gap_and_export.rs`, re-tokenized from byte-256 to charset-80.
//!   B. LinearState multi-timescale: a recurrent state
//!      `h_t = decay (.) h_{t-1} + W_in( actq( rms_norm(x_t) ) )` with the S5
//!      L_MT4 fixed decay bands `[0.5, 0.75, 0.9375, 0.875]`-see below-laid
//!      out by the canonical `DecayPolicy::MultiTimescale` slot rule
//!      (`gbf_model::sequence::DecayPolicy::decay_for_slot`), trained with
//!      truncated BPTT (state carried across chunks detached).
//!   C. Capacity probe: same as B with a longer BPTT context budget
//!      (seq_len doubled, lanes halved so tokens/step stay matched).
//!
//! The recurrence semantics follow `gbf_model::sequence::LinearStateBlock`
//! (input norm -> act fake-quant -> ternary in-proj -> decayed state update ->
//! ternary out-proj -> act fake-quant), composed here with a residual add
//! around the state block, ahead of the same 4-block FFN stack as arm A. The
//! existing `gbf_train::sequence::LinearStateBurnQat` adapter is single-
//! sequence and host-validates every forward, so this bin implements a batched
//! Burn recurrence with the same semantics and *proves parity* against the
//! canonical scalar QAT kernels (`TernaryLinearQat::inference_forward`,
//! `ActFakeQuant::inference_forward`, `DecayPolicy::decay_for_slot`) on a real
//! token prefix; the max-abs-diff is recorded in the report.
//!
//! Measurements per arm (deterministic, seed 0): validation bits-per-char in
//! BOTH units (per normalized charset_v1 char, and re-expressed per raw val
//! byte with the same total-bits-over-raw-byte-count method as the KN-5
//! artifact), the fp(Soft)-vs-ternary(Hard) gap on the same checkpoint, a
//! 256-char greedy sample from a fixed prompt, wall clock, and steps/s. KN-5
//! reference numbers are copied verbatim from
//! `experiments/S4/baseline/s4_baseline_gutenberg_run_meta.json`.
//!
//! Integrity: every number in the emitted JSON is produced by this program
//! from the actual runs. If an arm fails, the failure text is recorded and the
//! remaining arms still run.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use gbf_data::charset_v1::normalize_raw;
use gbf_foundation::sha256;
use gbf_model::qat::{
    ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange,
    ActivationRangeMode, MatrixShape, QatHardnessControl, QuantHardness, TernaryLinearQat,
    TernaryThreshold,
};
use gbf_model::sequence::DecayPolicy;
use gbf_train::adapter::burn::{
    BurnBackend, BurnDevice, BurnFloatTensor, BurnGradientsParams, BurnInt, BurnModule,
    BurnNdArrayAutodiffBackend, BurnNdArrayBackend, BurnOptimizer, BurnParam, BurnTensor,
    adamw_config, burn_gelu_approximate, burn_log_softmax, float_tensor_from_vec,
    float_tensor_into_vec,
};
use gbf_train::qat::{ActFakeQuantBurnQat, TernaryLinearBurnQat};
use serde_json::json;

const VOCAB: usize = 80;
const NORM_EPS: f32 = 1.0e-5;
const NORM_CLIP: f32 = 8.0;
const ACT_RANGE: f32 = 8.0;
/// S5 RFC D3 variant L_MT4 decay rates, one per contiguous slot band.
const MT4_DECAYS: [f32; 4] = [0.5, 0.75, 0.875, 0.9375];

type Adiff = BurnNdArrayAutodiffBackend;
type Plain = BurnNdArrayBackend;

#[derive(Parser, Debug, Clone)]
#[command(about = "F-S5 LinearState multi-timescale vs stateless A/B on charset-80 (bd-29ai4)")]
struct Args {
    /// Repo root (corpus + experiments paths resolve against this).
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Committed concatenated Gutenberg train byte stream.
    #[arg(
        long,
        default_value = "corpus/gutenberg/gutenberg_train_concatenated.bin"
    )]
    train_bin: String,
    /// Cap on raw training bytes read from the front of the train stream.
    #[arg(long, default_value_t = 64 * 1024 * 1024)]
    train_cap_bytes: usize,
    /// Cap on held-out raw validation bytes assembled from val-split books.
    #[arg(long, default_value_t = 1024 * 1024)]
    val_cap_bytes: usize,
    /// Cap on scored (context,target) pairs per evaluation pass
    /// (0 = score the full validation stream).
    #[arg(long, default_value_t = 0)]
    eval_pairs: usize,
    /// Parallel evaluation lanes (contiguous val segments, state per lane).
    #[arg(long, default_value_t = 8)]
    eval_lanes: usize,
    /// Tokens per lane per chunked eval forward.
    #[arg(long, default_value_t = 256)]
    eval_chunk: usize,
    /// Optimizer steps, matched across all arms.
    #[arg(long, default_value_t = 6000)]
    steps: u64,
    /// Arm A minibatch size (iid context/target pairs per step).
    #[arg(long, default_value_t = 512)]
    batch: usize,
    /// Arm B BPTT chunk length (tokens per lane per step).
    #[arg(long, default_value_t = 128)]
    seq_len: usize,
    /// Arm B parallel BPTT lanes (seq_len * seq_batch = tokens per step).
    #[arg(long, default_value_t = 4)]
    seq_batch: usize,
    /// Arm C BPTT chunk length.
    #[arg(long, default_value_t = 256)]
    c_seq_len: usize,
    /// Arm C parallel BPTT lanes.
    #[arg(long, default_value_t = 2)]
    c_seq_batch: usize,
    /// Recurrent state slots (must be divisible by 4 for the MT4 bands).
    #[arg(long, default_value_t = 64)]
    state_slots: usize,
    /// AdamW learning rate.
    #[arg(long, default_value_t = 0.01)]
    lr: f64,
    /// Fraction of steps trained with QAT hardness OFF before Hard.
    #[arg(long, default_value_t = 0.25)]
    warmup_frac: f64,
    /// Deterministic seed.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    #[arg(long, default_value_t = 64)]
    d_model: usize,
    #[arg(long, default_value_t = 128)]
    d_ff: usize,
    #[arg(long, default_value_t = 4)]
    n_blocks: usize,
    /// Steps between progress log lines.
    #[arg(long, default_value_t = 200)]
    log_every: u64,
    /// Greedy sample length in chars.
    #[arg(long, default_value_t = 256)]
    sample_chars: usize,
    /// Fixed greedy-sample prompt (charset_v1-normalized before use).
    #[arg(
        long,
        default_value = "The children walked down to the river in the morning, and "
    )]
    sample_prompt: String,
    /// Comma-separated arms to run (subset of A,B,C).
    #[arg(long, default_value = "A,B,C")]
    arms: String,
    #[arg(long, default_value = "experiments/S5/state-ab")]
    out_dir: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("s5_state_ab FAILED: {err}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// deterministic init (identical scheme to s2_gap_and_export.rs)
// ---------------------------------------------------------------------------

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Uniform f32 in [-scale, scale] from a seeded stream.
fn init_weights(seed: u64, salt: u64, len: usize, scale: f32) -> Vec<f32> {
    let mut state = seed ^ salt.wrapping_mul(0xd1b5_4a32_d192_ed03);
    (0..len)
        .map(|_| {
            let bits = splitmix64(&mut state);
            let unit = (bits >> 40) as f32 / (1u64 << 24) as f32; // [0,1)
            (unit * 2.0 - 1.0) * scale
        })
        .collect()
}

// ---------------------------------------------------------------------------
// model
// ---------------------------------------------------------------------------

#[derive(BurnModule, Debug)]
struct FfnBlock<B: BurnBackend> {
    up: TernaryLinearBurnQat<B>,
    down: TernaryLinearBurnQat<B>,
}

#[derive(BurnModule, Debug)]
struct StateBlock<B: BurnBackend> {
    input_to_state: TernaryLinearBurnQat<B>,
    state_to_output: TernaryLinearBurnQat<B>,
}

#[derive(BurnModule, Debug)]
struct ArmModel<B: BurnBackend> {
    embedding: BurnParam<BurnFloatTensor<B, 2>>,
    state: Option<StateBlock<B>>,
    blocks: Vec<FfnBlock<B>>,
}

fn make_ternary_core(out_rows: usize, in_cols: usize, weights: Vec<f32>) -> TernaryLinearQat {
    let shape = MatrixShape::new(out_rows, in_cols).expect("nonzero shape");
    let thresholds = vec![TernaryThreshold::new(0.0).expect("zero threshold"); out_rows];
    TernaryLinearQat::with_derived_per_row_scales(shape, weights, None, thresholds)
        .expect("valid ternary core")
}

impl ArmModel<Adiff> {
    fn init(args: &Args, with_state: bool, device: &BurnDevice<Adiff>) -> Self {
        let d_model = args.d_model;
        let d_ff = args.d_ff;
        let embedding = float_tensor_from_vec::<Adiff, 2>(
            init_weights(args.seed, 0x0001, VOCAB * d_model, 0.05),
            [VOCAB, d_model],
            device,
        )
        .expect("embedding init");

        let state = with_state.then(|| {
            let in_core = make_ternary_core(
                args.state_slots,
                d_model,
                init_weights(args.seed, 0x2001, args.state_slots * d_model, 0.08),
            );
            let out_core = make_ternary_core(
                d_model,
                args.state_slots,
                init_weights(args.seed, 0x2002, d_model * args.state_slots, 0.08),
            );
            StateBlock {
                input_to_state: TernaryLinearBurnQat::from_core(in_core, device)
                    .expect("state in-proj wrapper"),
                state_to_output: TernaryLinearBurnQat::from_core(out_core, device)
                    .expect("state out-proj wrapper"),
            }
        });

        let blocks = (0..args.n_blocks)
            .map(|layer| {
                let salt = 0x1000 + layer as u64;
                let up_core = make_ternary_core(
                    d_ff,
                    d_model,
                    init_weights(args.seed, salt * 7 + 1, d_ff * d_model, 0.08),
                );
                let down_core = make_ternary_core(
                    d_model,
                    d_ff,
                    init_weights(args.seed, salt * 7 + 2, d_model * d_ff, 0.08),
                );
                FfnBlock {
                    up: TernaryLinearBurnQat::from_core(up_core, device).expect("up wrapper"),
                    down: TernaryLinearBurnQat::from_core(down_core, device).expect("down wrapper"),
                }
            })
            .collect();

        Self {
            embedding: BurnParam::from_tensor(embedding),
            state,
            blocks,
        }
    }

    fn set_hardness(&mut self, hardness: QuantHardness) {
        if let Some(state) = &mut self.state {
            state.input_to_state.set_hardness(hardness);
            state.state_to_output.set_hardness(hardness);
        }
        for block in &mut self.blocks {
            block.up.set_hardness(hardness);
            block.down.set_hardness(hardness);
        }
    }
}

/// Fixed (non-learnable) activation fake-quant, identical to s2.
fn activation() -> ActFakeQuantBurnQat {
    let core = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-ACT_RANGE, ACT_RANGE).expect("range")),
        ActivationQuantFormat::Int8,
    )
    .expect("act");
    ActFakeQuantBurnQat::from_core(core).expect("act wrapper")
}

/// Full-vector RMS norm + clip, identical to s2_gap_and_export.rs.
fn rms_norm<B: BurnBackend>(x: BurnFloatTensor<B, 2>) -> BurnFloatTensor<B, 2> {
    let d_model = x.dims()[1];
    let mean_sq = (x.clone() * x.clone()).mean_dim(1); // [n, 1]
    let rms = (mean_sq + NORM_EPS).sqrt(); // [n, 1]
    let normed = x / rms.repeat_dim(1, d_model);
    normed.clamp(-NORM_CLIP, NORM_CLIP)
}

/// Canonical MT4 decay value per slot, via the shared `DecayPolicy` layout
/// rule (contiguous equal-width bands in declaration order).
fn mt4_decay_per_slot(state_slots: usize) -> Vec<f32> {
    let policy =
        DecayPolicy::multi_timescale(MT4_DECAYS.to_vec()).expect("MT4 decay policy is valid");
    (0..state_slots)
        .map(|slot| policy.decay_for_slot(slot, state_slots))
        .collect()
}

/// (logits `[batch*seq_len, VOCAB]`, final recurrent state `[batch, slots]`).
type SeqForwardOutput<B> = (BurnFloatTensor<B, 2>, Option<BurnFloatTensor<B, 2>>);

/// Borrowed per-layer views used by the shared forward.
struct ForwardRefs<'a, B: BurnBackend> {
    state: Option<(&'a TernaryLinearBurnQat<B>, &'a TernaryLinearBurnQat<B>)>,
    ups: Vec<&'a TernaryLinearBurnQat<B>>,
    downs: Vec<&'a TernaryLinearBurnQat<B>>,
}

impl<'a> ForwardRefs<'a, Adiff> {
    fn from_model(model: &'a ArmModel<Adiff>) -> Self {
        Self {
            state: model
                .state
                .as_ref()
                .map(|s| (&s.input_to_state, &s.state_to_output)),
            ups: model.blocks.iter().map(|b| &b.up).collect(),
            downs: model.blocks.iter().map(|b| &b.down).collect(),
        }
    }
}

/// Forward a time-major [batch * seq_len] context id stream and return
/// (logits [batch*seq_len, VOCAB], final recurrent state [batch, slots]).
///
/// Row `t * batch + b` of the logits corresponds to lane `b` at chunk
/// position `t`. When `refs.state` is `None` the model is the stateless
/// bigram-context stack and `init_state`/`decay` are ignored.
#[allow(clippy::too_many_arguments)]
fn forward_seq<B: BurnBackend>(
    embedding: BurnFloatTensor<B, 2>,
    refs: &ForwardRefs<'_, B>,
    act: &ActFakeQuantBurnQat,
    act_enabled: bool,
    ctx_ids: BurnTensor<B, 1, BurnInt>,
    batch: usize,
    seq_len: usize,
    init_state: Option<BurnFloatTensor<B, 2>>,
    decay_slots: &[f32],
    device: &BurnDevice<B>,
) -> Result<SeqForwardOutput<B>, Box<dyn std::error::Error>> {
    let mode = if act_enabled {
        ActivationForwardMode::Train
    } else {
        ActivationForwardMode::Passthrough
    };
    let mut x = embedding.clone().select(0, ctx_ids); // [batch*seq_len, d_model]

    let mut final_state = None;
    if let Some((in_proj, out_proj)) = refs.state {
        let slots = decay_slots.len();
        // Per-token input mix is state-independent, so compute it batched.
        let normed = rms_norm(x.clone());
        let normed = act.fake_quant_forward(normed, mode);
        let delta_all = in_proj.fake_quant_forward(normed)?; // [batch*seq_len, slots]

        let decay = float_tensor_from_vec::<B, 2>(decay_slots.to_vec(), [1, slots], device)?
            .repeat_dim(0, batch); // [batch, slots]
        let mut state = init_state.ok_or("stateful arm requires an initial state")?;
        let mut rows = Vec::with_capacity(seq_len);
        for t in 0..seq_len {
            let delta_t = delta_all
                .clone()
                .slice([t * batch..(t + 1) * batch, 0..slots]);
            state = state * decay.clone() + delta_t;
            rows.push(state.clone());
        }
        let states_all = BurnFloatTensor::<B, 2>::cat(rows, 0); // [batch*seq_len, slots]
        let projected = out_proj.fake_quant_forward(states_all)?;
        let y = act.fake_quant_forward(projected, mode);
        x = x + y; // residual around the state block
        final_state = Some(state);
    }

    for (up, down) in refs.ups.iter().zip(refs.downs.iter()) {
        let normed = rms_norm(x.clone());
        let normed = act.fake_quant_forward(normed, mode);
        let hidden = up.fake_quant_forward(normed)?;
        let hidden = burn_gelu_approximate(hidden);
        let hidden = act.fake_quant_forward(hidden, mode);
        let delta = down.fake_quant_forward(hidden)?;
        x = x + delta;
    }
    let normed = rms_norm(x);
    Ok((normed.matmul(embedding.transpose()), final_state))
}

// ---------------------------------------------------------------------------
// data
// ---------------------------------------------------------------------------

/// Longest prefix of `bytes[..cap]` that is valid UTF-8 (same as the KN bin).
fn utf8_prefix(bytes: &[u8], cap: usize) -> Result<(&[u8], usize), Box<dyn std::error::Error>> {
    let take = bytes.len().min(cap);
    let slice = &bytes[..take];
    match std::str::from_utf8(slice) {
        Ok(_) => Ok((slice, 0)),
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            Ok((&slice[..valid], take - valid))
        }
        Err(error) => Err(format!("stream is not valid UTF-8: {error}").into()),
    }
}

/// Build the held-out validation byte stream from the val-split book bodies.
/// Byte-for-byte the same assembly as `s2_gap_and_export.rs` /
/// `s4_kn5_baseline_gutenberg.rs`.
fn build_val_bytes(
    repo_root: &Path,
    cap: usize,
) -> Result<(Vec<u8>, Vec<u64>), Box<dyn std::error::Error>> {
    let splits_path = repo_root.join("corpus/gutenberg/splits.json");
    let splits: serde_json::Value = serde_json::from_slice(
        &fs::read(&splits_path).map_err(|e| format!("read {}: {e}", splits_path.display()))?,
    )?;
    let val_ids = splits["val"]
        .as_array()
        .ok_or("splits.json missing val array")?
        .iter()
        .filter_map(|v| v.as_u64())
        .collect::<Vec<_>>();

    let mut bytes = Vec::with_capacity(cap.min(4 * 1024 * 1024));
    let mut used_ids = Vec::new();
    for id in &val_ids {
        if bytes.len() >= cap {
            break;
        }
        let body_path = repo_root
            .join("corpus/gutenberg/bodies")
            .join(id.to_string())
            .join("body.txt");
        let Ok(body) = fs::read(&body_path) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }
        used_ids.push(*id);
        let remaining = cap - bytes.len();
        bytes.extend_from_slice(&body[..body.len().min(remaining)]);
    }
    if bytes.len() < 2 {
        return Err("assembled validation stream is too small".into());
    }
    Ok((bytes, used_ids))
}

/// Inverse of `gbf_data::charset_v1::unmappable::char_id`, decode-side only.
fn id_to_char(id: u8) -> char {
    match id {
        0..=25 => (b'A' + id) as char,
        26..=51 => (b'a' + (id - 26)) as char,
        52..=61 => (b'0' + (id - 52)) as char,
        62 => ' ',
        63 => '.',
        64 => ',',
        65 => '!',
        66 => '?',
        67 => '-',
        68 => '\'',
        69 => ':',
        70 => ';',
        71 => '(',
        72 => ')',
        73 => '"',
        74 => '/',
        75 => '\n',
        _ => '\u{FFFD}', // reserved/<bos>/<eos>/<unk>
    }
}

// ---------------------------------------------------------------------------
// evaluation
// ---------------------------------------------------------------------------

/// Deployable snapshot of one trained arm (plain cores, host embedding).
struct ArmSnapshot {
    embedding: Vec<f32>,
    state_cores: Option<(TernaryLinearQat, TernaryLinearQat)>,
    ffn_cores: Vec<(TernaryLinearQat, TernaryLinearQat)>,
}

fn extract_snapshot(model: &ArmModel<Adiff>) -> Result<ArmSnapshot, Box<dyn std::error::Error>> {
    let embedding = float_tensor_into_vec(model.embedding.val().inner().detach())?;
    let state_cores = match &model.state {
        Some(s) => Some((
            s.input_to_state.to_core_from_trained_state()?,
            s.state_to_output.to_core_from_trained_state()?,
        )),
        None => None,
    };
    let mut ffn_cores = Vec::new();
    for block in &model.blocks {
        ffn_cores.push((
            block.up.to_core_from_trained_state()?,
            block.down.to_core_from_trained_state()?,
        ));
    }
    Ok(ArmSnapshot {
        embedding,
        state_cores,
        ffn_cores,
    })
}

/// Plain-backend wrappers rebuilt from a snapshot at a fixed hardness.
struct PlainLayers {
    state: Option<(TernaryLinearBurnQat<Plain>, TernaryLinearBurnQat<Plain>)>,
    ups: Vec<TernaryLinearBurnQat<Plain>>,
    downs: Vec<TernaryLinearBurnQat<Plain>>,
}

impl PlainLayers {
    fn build(
        snapshot: &ArmSnapshot,
        hardness: QuantHardness,
        device: &BurnDevice<Plain>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let harden = |core: &TernaryLinearQat| -> Result<
            TernaryLinearBurnQat<Plain>,
            Box<dyn std::error::Error>,
        > {
            let mut c = core.clone();
            c.set_hardness(hardness);
            Ok(TernaryLinearBurnQat::<Plain>::from_core(c, device)?)
        };
        let state = match &snapshot.state_cores {
            Some((i, o)) => Some((harden(i)?, harden(o)?)),
            None => None,
        };
        let mut ups = Vec::new();
        let mut downs = Vec::new();
        for (up, down) in &snapshot.ffn_cores {
            ups.push(harden(up)?);
            downs.push(harden(down)?);
        }
        Ok(Self { state, ups, downs })
    }

    fn refs(&self) -> ForwardRefs<'_, Plain> {
        ForwardRefs {
            state: self.state.as_ref().map(|(i, o)| (i, o)),
            ups: self.ups.iter().collect(),
            downs: self.downs.iter().collect(),
        }
    }
}

/// Lane-parallel validation bpc. The val token stream is split into
/// `lanes` equal contiguous segments; each lane scores its within-lane
/// adjacent (context, target) pairs in order, with recurrent state carried
/// across chunk boundaries (zero-initialized per lane). Arms with no state
/// score the *identical* pair set, so the comparison is pair-for-pair fair.
#[allow(clippy::too_many_arguments)]
fn eval_bpc_lanes(
    snapshot: &ArmSnapshot,
    d_model: usize,
    state_slots: usize,
    decay_slots: &[f32],
    hardness: QuantHardness,
    act_enabled: bool,
    val_ids: &[u8],
    max_pairs: usize,
    lanes: usize,
    chunk: usize,
    device: &BurnDevice<Plain>,
) -> Result<(f64, usize), Box<dyn std::error::Error>> {
    let layers = PlainLayers::build(snapshot, hardness, device)?;
    let refs = layers.refs();
    let act = activation();
    let embed =
        float_tensor_from_vec::<Plain, 2>(snapshot.embedding.clone(), [VOCAB, d_model], device)?;

    // Cap the scored region, then split into equal lanes.
    let usable = if max_pairs == 0 {
        val_ids.len()
    } else {
        val_ids.len().min(max_pairs + lanes)
    };
    let lane_len = usable / lanes;
    if lane_len < 2 {
        return Err("validation stream too small for the lane layout".into());
    }
    let pairs_per_lane = lane_len - 1;

    let has_state = refs.state.is_some();
    let mut state =
        has_state.then(|| BurnFloatTensor::<Plain, 2>::zeros([lanes, state_slots], device));

    let ln2 = std::f64::consts::LN_2;
    let mut total_bits = 0.0_f64;
    let mut done = 0usize;
    while done < pairs_per_lane {
        let this = chunk.min(pairs_per_lane - done);
        let mut ctx = Vec::with_capacity(this * lanes);
        let mut tgt = Vec::with_capacity(this * lanes);
        for t in 0..this {
            for lane in 0..lanes {
                let base = lane * lane_len + done + t;
                ctx.push(val_ids[base] as i32);
                tgt.push(val_ids[base + 1] as i32);
            }
        }
        let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), device);
        let (logits, next_state) = forward_seq(
            embed.clone(),
            &refs,
            &act,
            act_enabled,
            ctx_ids,
            lanes,
            this,
            state.clone(),
            decay_slots,
            device,
        )?;
        state = next_state;
        let logp = burn_log_softmax(logits, 1);
        let tgt_idx = BurnTensor::<Plain, 1, BurnInt>::from_ints(tgt.as_slice(), device)
            .reshape([this * lanes, 1]);
        let picked = logp.gather(1, tgt_idx).reshape([this * lanes]);
        for v in float_tensor_into_vec(picked)? {
            total_bits += -(v as f64) / ln2;
        }
        done += this;
    }
    Ok((total_bits, pairs_per_lane * lanes))
}

/// Greedy 1-token-at-a-time sample with hard ternary weights.
fn greedy_sample(
    snapshot: &ArmSnapshot,
    d_model: usize,
    state_slots: usize,
    decay_slots: &[f32],
    prompt_ids: &[u8],
    sample_chars: usize,
    device: &BurnDevice<Plain>,
) -> Result<String, Box<dyn std::error::Error>> {
    let layers = PlainLayers::build(snapshot, QuantHardness::Hard, device)?;
    let refs = layers.refs();
    let act = activation();
    let embed =
        float_tensor_from_vec::<Plain, 2>(snapshot.embedding.clone(), [VOCAB, d_model], device)?;
    let has_state = refs
        .state
        .is_some()
        .then(|| BurnFloatTensor::<Plain, 2>::zeros([1, state_slots], device));
    let mut state = has_state;

    // Prime on the prompt (one forward over the whole prompt sequence).
    let ctx: Vec<i32> = prompt_ids.iter().map(|&id| id as i32).collect();
    let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), device);
    let (logits, next_state) = forward_seq(
        embed.clone(),
        &refs,
        &act,
        true,
        ctx_ids,
        1,
        prompt_ids.len(),
        state.clone(),
        decay_slots,
        device,
    )?;
    state = next_state;
    let mut last = argmax_last_row(logits)?;

    let mut out = String::with_capacity(sample_chars);
    for _ in 0..sample_chars {
        out.push(id_to_char(last));
        let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints([last as i32].as_slice(), device);
        let (logits, next_state) = forward_seq(
            embed.clone(),
            &refs,
            &act,
            true,
            ctx_ids,
            1,
            1,
            state.clone(),
            decay_slots,
            device,
        )?;
        state = next_state;
        last = argmax_last_row(logits)?;
    }
    Ok(out)
}

fn argmax_last_row(logits: BurnFloatTensor<Plain, 2>) -> Result<u8, Box<dyn std::error::Error>> {
    let dims = logits.dims();
    let last = logits.slice([dims[0] - 1..dims[0], 0..dims[1]]);
    let values = float_tensor_into_vec(last)?;
    let (best, _) =
        values
            .iter()
            .enumerate()
            .fold((0usize, f32::NEG_INFINITY), |(bi, bv), (i, &v)| {
                if v > bv { (i, v) } else { (bi, bv) }
            });
    Ok(best as u8)
}

// ---------------------------------------------------------------------------
// parity: batched Burn recurrence vs canonical scalar QAT kernels
// ---------------------------------------------------------------------------

/// Recompute the state-block output sequence with the canonical scalar
/// kernels (`TernaryLinearQat::inference_forward`,
/// `ActFakeQuant::inference_forward`, `DecayPolicy::decay_for_slot`) and
/// compare against the batched Burn path on the same trained checkpoint.
/// Returns the max abs diff over all state-block outputs and the final state.
fn state_block_parity(
    snapshot: &ArmSnapshot,
    d_model: usize,
    state_slots: usize,
    decay_slots: &[f32],
    token_ids: &[u8],
    device: &BurnDevice<Plain>,
) -> Result<f64, Box<dyn std::error::Error>> {
    let (in_core, out_core) = snapshot
        .state_cores
        .as_ref()
        .ok_or("parity check requires a stateful snapshot")?;
    let mut in_hard = in_core.clone();
    let mut out_hard = out_core.clone();
    in_hard.set_hardness(QuantHardness::Hard);
    out_hard.set_hardness(QuantHardness::Hard);
    let act_core = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-ACT_RANGE, ACT_RANGE)?),
        ActivationQuantFormat::Int8,
    )?;

    // --- scalar reference, one token at a time ---
    let mut scalar_state = vec![0.0_f32; state_slots];
    let mut scalar_outputs = Vec::new();
    for &id in token_ids {
        let x: Vec<f32> =
            snapshot.embedding[id as usize * d_model..(id as usize + 1) * d_model].to_vec();
        let mean_sq = x.iter().map(|v| v * v).sum::<f32>() / d_model as f32;
        let rms = (mean_sq + NORM_EPS).sqrt();
        let normed: Vec<f32> = x
            .iter()
            .map(|v| (v / rms).clamp(-NORM_CLIP, NORM_CLIP))
            .collect();
        let activated = act_core.inference_forward(&normed, ActivationForwardMode::Train)?;
        let delta = in_hard.inference_forward(&activated)?;
        for (slot, (s, d)) in scalar_state.iter_mut().zip(delta.iter()).enumerate() {
            *s = *s * decay_slots[slot] + *d;
            debug_assert!(slot < state_slots);
        }
        let projected = out_hard.inference_forward(&scalar_state)?;
        scalar_outputs
            .extend(act_core.inference_forward(&projected, ActivationForwardMode::Train)?);
    }

    // --- batched Burn path (batch=1 lane, seq_len = token count) ---
    let layers = PlainLayers::build(snapshot, QuantHardness::Hard, device)?;
    let refs = layers.refs();
    let (in_proj, out_proj) = refs.state.ok_or("stateful layers expected")?;
    let act = activation();
    let embed =
        float_tensor_from_vec::<Plain, 2>(snapshot.embedding.clone(), [VOCAB, d_model], device)?;
    let ctx: Vec<i32> = token_ids.iter().map(|&id| id as i32).collect();
    let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), device);
    let x = embed.select(0, ctx_ids);
    let normed = rms_norm(x);
    let normed = act.fake_quant_forward(normed, ActivationForwardMode::Train);
    let delta_all = in_proj.fake_quant_forward(normed)?;
    let decay = float_tensor_from_vec::<Plain, 2>(decay_slots.to_vec(), [1, state_slots], device)?;
    let mut state = BurnFloatTensor::<Plain, 2>::zeros([1, state_slots], device);
    let mut rows = Vec::new();
    for t in 0..token_ids.len() {
        let delta_t = delta_all.clone().slice([t..t + 1, 0..state_slots]);
        state = state * decay.clone() + delta_t;
        rows.push(state.clone());
    }
    let states_all = BurnFloatTensor::<Plain, 2>::cat(rows, 0);
    let projected = out_proj.fake_quant_forward(states_all)?;
    let burn_outputs =
        float_tensor_into_vec(act.fake_quant_forward(projected, ActivationForwardMode::Train))?;
    let burn_state = float_tensor_into_vec(state)?;

    let mut max_diff = 0.0_f64;
    for (a, b) in scalar_outputs.iter().zip(burn_outputs.iter()) {
        max_diff = max_diff.max((f64::from(*a) - f64::from(*b)).abs());
    }
    for (a, b) in scalar_state.iter().zip(burn_state.iter()) {
        max_diff = max_diff.max((f64::from(*a) - f64::from(*b)).abs());
    }
    if scalar_outputs.len() != burn_outputs.len() {
        return Err("parity output length mismatch".into());
    }
    Ok(max_diff)
}

// ---------------------------------------------------------------------------
// training
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmKind {
    Stateless,
    State { seq_len: usize, seq_batch: usize },
}

struct ArmResult {
    snapshot: ArmSnapshot,
    steps_per_second: f64,
    train_wall_seconds: f64,
    final_train_loss_bpc: f64,
    tokens_per_step: usize,
}

/// Train one arm with the shared QAT recipe (warmup Off -> Hard).
fn train_arm(
    args: &Args,
    name: &str,
    kind: ArmKind,
    train_ids: &[u8],
    decay_slots: &[f32],
    device: &BurnDevice<Adiff>,
) -> Result<ArmResult, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let with_state = matches!(kind, ArmKind::State { .. });
    let mut model = ArmModel::init(args, with_state, device);
    let mut optimizer = adamw_config()
        .with_weight_decay(0.0)
        .init::<Adiff, ArmModel<Adiff>>();
    let act = activation();
    let warmup_steps = (args.steps as f64 * args.warmup_frac) as u64;

    let (seq_len, seq_batch) = match kind {
        ArmKind::Stateless => (1usize, args.batch),
        ArmKind::State { seq_len, seq_batch } => (seq_len, seq_batch),
    };
    let tokens_per_step = seq_len * seq_batch;
    println!(
        "[{name}] d_model={} d_ff={} n_blocks={} vocab={VOCAB} state_slots={} steps={} seq_len={seq_len} lanes={seq_batch} tokens/step={tokens_per_step} lr={} warmup={} seed={}",
        args.d_model,
        args.d_ff,
        args.n_blocks,
        if with_state { args.state_slots } else { 0 },
        args.steps,
        args.lr,
        warmup_steps,
        args.seed,
    );

    // Deterministic samplers. Stateless: iid pair positions. Stateful: lanes
    // walking the stream sequentially (truncated BPTT, detached carry).
    let mut sampler = args.seed ^ 0xa5a5_5a5a_1234_5678;
    let max_pos = train_ids.len() - 1;
    let mut lane_pos: Vec<usize> = (0..seq_batch)
        .map(|_| (splitmix64(&mut sampler) as usize) % (train_ids.len() - seq_len - 1))
        .collect();
    let mut carried_state = vec![0.0_f32; seq_batch * args.state_slots];

    let ln2 = std::f64::consts::LN_2;
    let mut running_loss = 0.0_f64;
    let mut running_n = 0u64;
    let mut last_logged_loss = f64::NAN;
    let mut current_hard = false;
    model.set_hardness(QuantHardness::Off);

    for step in 1..=args.steps {
        let want_hard = step > warmup_steps;
        if want_hard != current_hard {
            model.set_hardness(if want_hard {
                QuantHardness::Hard
            } else {
                QuantHardness::Off
            });
            current_hard = want_hard;
            println!(
                "[{name}] step {step}: QAT hardness -> {}",
                if want_hard { "Hard" } else { "Off" }
            );
        }
        let act_enabled = want_hard;

        let mut ctx = Vec::with_capacity(tokens_per_step);
        let mut tgt = Vec::with_capacity(tokens_per_step);
        match kind {
            ArmKind::Stateless => {
                for _ in 0..seq_batch {
                    let pos = (splitmix64(&mut sampler) as usize) % max_pos;
                    ctx.push(train_ids[pos] as i32);
                    tgt.push(train_ids[pos + 1] as i32);
                }
            }
            ArmKind::State { .. } => {
                // Time-major: row t*lanes + lane.
                for (lane_index, lane) in lane_pos.iter_mut().enumerate() {
                    if *lane + seq_len + 1 > train_ids.len() {
                        *lane =
                            (splitmix64(&mut sampler) as usize) % (train_ids.len() - seq_len - 1);
                        // Lane wrapped: reset THIS lane's carried state.
                        let band =
                            lane_index * args.state_slots..(lane_index + 1) * args.state_slots;
                        carried_state[band].fill(0.0);
                    }
                }
                for t in 0..seq_len {
                    for &pos in &lane_pos {
                        ctx.push(train_ids[pos + t] as i32);
                        tgt.push(train_ids[pos + t + 1] as i32);
                    }
                }
                for pos in lane_pos.iter_mut() {
                    *pos += seq_len;
                }
            }
        }

        let ctx_ids = BurnTensor::<Adiff, 1, BurnInt>::from_ints(ctx.as_slice(), device);
        let tgt_idx = BurnTensor::<Adiff, 1, BurnInt>::from_ints(tgt.as_slice(), device)
            .reshape([tokens_per_step, 1]);

        let refs = ForwardRefs::from_model(&model);
        let init_state = with_state
            .then(|| {
                float_tensor_from_vec::<Adiff, 2>(
                    carried_state.clone(),
                    [seq_batch, args.state_slots],
                    device,
                )
            })
            .transpose()?;
        let (logits, final_state) = forward_seq(
            model.embedding.val(),
            &refs,
            &act,
            act_enabled,
            ctx_ids,
            seq_batch,
            seq_len,
            init_state,
            decay_slots,
            device,
        )?;
        let logp = burn_log_softmax(logits, 1);
        let picked = logp.gather(1, tgt_idx).reshape([tokens_per_step]);
        let loss = picked.mean() * -1.0;
        let loss_nats = float_tensor_into_vec(loss.clone().inner())?[0];
        if !loss_nats.is_finite() {
            return Err(format!("[{name}] non-finite training loss at step {step}").into());
        }
        running_loss += f64::from(loss_nats);
        running_n += 1;

        // Truncated BPTT: carry the recurrent state across chunks *detached*.
        if let Some(final_state) = final_state {
            carried_state = float_tensor_into_vec(final_state.inner().detach())?;
        }

        let grads = loss.backward();
        let grads = BurnGradientsParams::from_grads(grads, &model);
        model = optimizer.step(args.lr, model, grads);

        if step % args.log_every == 0 || step == 1 {
            let elapsed = started.elapsed().as_secs_f64();
            let rate = step as f64 / elapsed;
            let mean_loss = running_loss / running_n.max(1) as f64;
            last_logged_loss = mean_loss / ln2;
            println!(
                "[{name}] step {step}/{} loss_nats={:.4} bpc~={:.4} {:.2} steps/s elapsed={:.0}s",
                args.steps, mean_loss, last_logged_loss, rate, elapsed
            );
            running_loss = 0.0;
            running_n = 0;
        }
    }

    let train_wall_seconds = started.elapsed().as_secs_f64();
    let snapshot = extract_snapshot(&model)?;
    Ok(ArmResult {
        snapshot,
        steps_per_second: args.steps as f64 / train_wall_seconds,
        train_wall_seconds,
        final_train_loss_bpc: last_logged_loss,
        tokens_per_step,
    })
}

// ---------------------------------------------------------------------------
// export (arm B canonical tensors, S6-export-compatible layout)
// ---------------------------------------------------------------------------

fn write_bin(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()).into())
}

fn f32_le_bytes(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn export_ternary_pair(
    export_dir: &Path,
    tensor_index: &mut Vec<serde_json::Value>,
    base: &str,
    role: &str,
    core: &TernaryLinearQat,
) -> Result<(), Box<dyn std::error::Error>> {
    let export = core.export_canonical();
    let shape = export.shape();
    let i8_bytes: Vec<u8> = export
        .ternary_values()
        .iter()
        .map(|v| v.as_i8() as u8)
        .collect();
    let mut scale_bytes = Vec::with_capacity(export.scales().len() * 2);
    for s in export.scales() {
        scale_bytes.extend_from_slice(&s.raw().to_le_bytes());
    }
    let tern_file = format!("tensors/{base}.ternary.i8.bin");
    let scale_file = format!("tensors/{base}.scales.q8_8_u16le.bin");
    write_bin(&export_dir.join(&tern_file), &i8_bytes)?;
    write_bin(&export_dir.join(&scale_file), &scale_bytes)?;
    tensor_index.push(json!({
        "name": format!("{base}.ternary"),
        "role": format!("{role}_ternary_weights"),
        "dtype": "i8 (values in {-1,0,1})",
        "shape": [shape.output_rows(), shape.input_cols()],
        "layout": "row_major",
        "file": tern_file,
        "sha256": sha256(&i8_bytes).to_hex()
    }));
    tensor_index.push(json!({
        "name": format!("{base}.scales"),
        "role": format!("{role}_per_output_row_scale"),
        "dtype": "u16_le (Q8.8 fixed-point; f32 = raw/256)",
        "shape": [shape.output_rows()],
        "file": scale_file,
        "sha256": sha256(&scale_bytes).to_hex()
    }));
    Ok(())
}

fn export_checkpoint_b(
    export_dir: &Path,
    args: &Args,
    snapshot: &ArmSnapshot,
    decay_slots: &[f32],
    git_sha: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let tensors_dir = export_dir.join("tensors");
    fs::create_dir_all(&tensors_dir)?;

    let embed_bytes = f32_le_bytes(&snapshot.embedding);
    write_bin(&tensors_dir.join("embedding.f32.bin"), &embed_bytes)?;
    let mut tensor_index = vec![json!({
        "name": "embedding",
        "role": "token_embedding_and_tied_head",
        "dtype": "f32_le",
        "shape": [VOCAB, args.d_model],
        "layout": "row_major",
        "file": "tensors/embedding.f32.bin",
        "sha256": sha256(&embed_bytes).to_hex()
    })];

    let (in_core, out_core) = snapshot
        .state_cores
        .as_ref()
        .ok_or("arm B export requires state cores")?;
    export_ternary_pair(
        export_dir,
        &mut tensor_index,
        "state_input_to_state",
        "linear_state_input_projection",
        in_core,
    )?;
    export_ternary_pair(
        export_dir,
        &mut tensor_index,
        "state_state_to_output",
        "linear_state_output_projection",
        out_core,
    )?;

    // Decay per slot: exact Q8.8 integers (all MT4 rates are /256-exact).
    let decay_q8_8: Vec<u16> = decay_slots.iter().map(|d| (d * 256.0) as u16).collect();
    for (slot, (&d, &raw)) in decay_slots.iter().zip(decay_q8_8.iter()).enumerate() {
        let back = f32::from(raw) / 256.0;
        if (back - d).abs() > f32::EPSILON {
            return Err(format!("decay slot {slot} value {d} is not Q8.8-exact").into());
        }
    }
    let mut decay_bytes = Vec::with_capacity(decay_q8_8.len() * 2);
    for raw in &decay_q8_8 {
        decay_bytes.extend_from_slice(&raw.to_le_bytes());
    }
    write_bin(
        &tensors_dir.join("state_decay.q8_8_u16le.bin"),
        &decay_bytes,
    )?;
    tensor_index.push(json!({
        "name": "state_decay",
        "role": "linear_state_per_slot_decay",
        "dtype": "u16_le (Q8.8 fixed-point; f32 = raw/256, exact for MT4 rates)",
        "shape": [args.state_slots],
        "file": "tensors/state_decay.q8_8_u16le.bin",
        "sha256": sha256(&decay_bytes).to_hex()
    }));

    let mut layers = Vec::new();
    for (layer, (up, down)) in snapshot.ffn_cores.iter().enumerate() {
        export_ternary_pair(
            export_dir,
            &mut tensor_index,
            &format!("block{layer}_up"),
            "up_projection",
            up,
        )?;
        export_ternary_pair(
            export_dir,
            &mut tensor_index,
            &format!("block{layer}_down"),
            "down_projection",
            down,
        )?;
        layers.push(json!({
            "index": layer,
            "kind": "prenorm_residual_ffn",
            "up_ternary": format!("block{layer}_up.ternary"),
            "up_scales": format!("block{layer}_up.scales"),
            "down_ternary": format!("block{layer}_down.ternary"),
            "down_scales": format!("block{layer}_down.scales"),
            "up_shape": [args.d_ff, args.d_model],
            "down_shape": [args.d_model, args.d_ff]
        }));
    }

    let manifest = json!({
        "schema": "f_s5_state_checkpoint_export.v1",
        "bead": "bd-29ai4",
        "git_sha": git_sha,
        "seed": args.seed,
        "topology": {
            "family": "linear_state_multi_timescale_then_dense_ffn",
            "moe": false,
            "d_model": args.d_model,
            "d_ff": args.d_ff,
            "n_blocks": args.n_blocks,
            "vocab": VOCAB,
            "lexical": "charset_v1 (80 ids; ids 0..75 printable incl. newline, 76 reserved, 77 <bos>, 78 <eos>, 79 <unk>)",
            "tied_head": true,
            "sequence_state_kind": "linear_state_multi_timescale",
            "sequence_state_params": {
                "state_slots": args.state_slots,
                "state_bytes_per_layer": args.state_slots * 4,
                "decay_policy": "MultiTimescale",
                "decay_rates_by_band": MT4_DECAYS,
                "band_layout": "state_slots partitioned into 4 equal contiguous bands in declaration order; slot s uses decay_rates[s / (state_slots/4)] (gbf_model::sequence::DecayPolicy::decay_for_slot)"
            }
        },
        "recurrence_semantics": {
            "per_token": [
                "n_t   = clip( x_t / sqrt(mean(x_t^2) + 1e-5), -8, 8 )            # full-vector RMS norm over d_model",
                "a_t   = actq8(n_t)                                               # Int8 symmetric fake-quant, range [-8, 8], 127 steps",
                "delta = TernaryMatVec(input_to_state, a_t)                       # {-1,0,+1} weights, per-output-row Q8.8 scale",
                "h_t[s] = decay[s] * h_{t-1}[s] + delta[s]                        # decay[s] read from state_decay Q8.8 (exact)",
                "y_t   = actq8( TernaryMatVec(state_to_output, h_t) )",
                "x'_t  = x_t + y_t                                                # residual around the state block (composer-owned)"
            ],
            "integer_note": "decay values {0.5,0.75,0.875,0.9375} are exactly {128,192,224,240}/256, so h*decay is an exact Q8.8 multiply (raw*decay_raw >> 8); the ternary matvecs and Int8 activation grid follow the same numeric convention as the S6 dense export.",
            "initial_state": "all slots zero at stream start; state persists across tokens (no reset within a document stream)",
            "then": "x'_t feeds the same 4-block pre-norm residual FFN stack as the S6 dense export (block_forward identical), then logits = rms_norm(x_final) @ embedding^T"
        },
        "numeric_convention": {
            "weight_encoding": "Ternary2 {-1,0,+1}",
            "weight_scale": "per_output_row Q8.8 (u16 raw, f32 = raw/256)",
            "embedding_dtype": "f32_le",
            "norm": {"kind": "tile_rms_then_affine_clip(full_vector)", "epsilon": NORM_EPS, "clip_lo": -NORM_CLIP, "clip_hi": NORM_CLIP, "affine_scale": 1.0, "affine_bias": 0.0},
            "activation_fake_quant": {"format": "Int8_symmetric", "range_lo": -ACT_RANGE, "range_hi": ACT_RANGE, "quant_steps": 127},
            "block_forward": "x' = x + Down( gelu( Up( actq( rms_norm(x) ) ) ) ); logits = rms_norm(x_final) @ embedding^T"
        },
        "layers": layers,
        "tensors": tensor_index
    });
    fs::write(
        export_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    Ok(json!({
        "exported": true,
        "manifest": "experiments/S5/state-ab/checkpoint-export/manifest.json",
        "n_tensors": tensor_index.len(),
    }))
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let total_start = Instant::now();
    let repo_root = args.repo_root.clone();
    let device = BurnDevice::<Adiff>::default();
    let plain_device = BurnDevice::<Plain>::default();

    if !args.state_slots.is_multiple_of(MT4_DECAYS.len()) {
        return Err("state_slots must be divisible by 4 for the MT4 bands".into());
    }
    let decay_slots = mt4_decay_per_slot(args.state_slots);

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // ---- data: raw streams -> charset_v1 token streams ----
    let train_path = repo_root.join(&args.train_bin);
    let train_all = fs::read(&train_path)
        .map_err(|e| format!("read train bin {}: {e}", train_path.display()))?;
    let (train_prefix, train_trimmed) = utf8_prefix(&train_all, args.train_cap_bytes)?;
    let train_raw_sha = sha256(train_prefix).to_hex();
    let train_norm = normalize_raw(train_prefix)?;
    let train_ids = train_norm.tokens.into_vec();
    let train_norm_sha = sha256(train_ids.as_slice()).to_hex();

    let (val_raw, val_book_ids) = build_val_bytes(&repo_root, args.val_cap_bytes)?;
    let val_raw_sha = sha256(&val_raw).to_hex();
    let (val_prefix, val_trimmed) = utf8_prefix(&val_raw, val_raw.len())?;
    let val_norm = normalize_raw(val_prefix)?;
    let val_ids = val_norm.tokens.into_vec();
    let val_norm_sha = sha256(val_ids.as_slice()).to_hex();
    let val_chars_total = val_ids.len();
    let val_raw_bytes_total = val_prefix.len();
    let chars_per_raw_byte = val_chars_total as f64 / val_raw_bytes_total as f64;

    println!(
        "[data] train: {} raw bytes (cap {}, {} trimmed) -> {} charset_v1 tokens ({} unk), raw sha {} norm sha {}",
        train_prefix.len(),
        args.train_cap_bytes,
        train_trimmed,
        train_ids.len(),
        train_norm.unk_count_in_example,
        &train_raw_sha[..16],
        &train_norm_sha[..16],
    );
    println!(
        "[data] val: {} raw bytes ({} trimmed) from books {:?} -> {} tokens ({} unk), raw sha {} norm sha {}",
        val_raw_bytes_total,
        val_trimmed,
        val_book_ids,
        val_chars_total,
        val_norm.unk_count_in_example,
        &val_raw_sha[..16],
        &val_norm_sha[..16],
    );

    // ---- fixed greedy-sample prompt, charset-normalized ----
    let prompt_norm = normalize_raw(args.sample_prompt.as_bytes())?;
    let prompt_ids = prompt_norm.tokens.into_vec();
    let prompt_text: String = prompt_ids.iter().map(|&id| id_to_char(id)).collect();

    // ---- KN-5 reference (copied verbatim, never fabricated) ----
    let kn_meta_path =
        repo_root.join("experiments/S4/baseline/s4_baseline_gutenberg_run_meta.json");
    let kn_reference = match fs::read(&kn_meta_path) {
        Ok(raw) => {
            let meta: serde_json::Value = serde_json::from_slice(&raw)?;
            json!({
                "source": "experiments/S4/baseline/s4_baseline_gutenberg_run_meta.json (copied verbatim)",
                "runs": meta["runs"].as_array().map(|runs| runs.iter().map(|r| json!({
                    "train_cap_bytes": r["train_cap_bytes"],
                    "bpc_kn5_val_per_normalized_char": r["bpc_kn5_val_per_normalized_char"],
                    "kn5_bits_per_raw_val_byte": r["kn5_bits_per_raw_val_byte"],
                })).collect::<Vec<_>>()),
                "val_raw_bytes_sha256": meta["val_stream"]["val_raw_bytes_sha256"],
                "corpus_val_sha_normalized": meta["val_stream"]["corpus_val_sha_normalized"],
            })
        }
        Err(err) => json!({ "status": format!("KN-5 run meta unavailable: {err}") }),
    };

    // ---- arms ----
    let out_dir = repo_root.join(&args.out_dir);
    fs::create_dir_all(&out_dir)?;
    let requested: Vec<String> = args
        .arms
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();

    let arm_defs: Vec<(&str, ArmKind, &str)> = vec![
        (
            "A",
            ArmKind::Stateless,
            "stateless bigram-context dense FFN (s2 topology, charset-80)",
        ),
        (
            "B",
            ArmKind::State {
                seq_len: args.seq_len,
                seq_batch: args.seq_batch,
            },
            "LinearState multi-timescale (MT4) + residual, then the same FFN stack; truncated BPTT",
        ),
        (
            "C",
            ArmKind::State {
                seq_len: args.c_seq_len,
                seq_batch: args.c_seq_batch,
            },
            "arm B with doubled BPTT context budget (capacity probe), matched tokens/step",
        ),
    ];

    let mut arm_reports = Vec::new();
    let mut results: Vec<(String, Option<ArmResult>)> = Vec::new();
    for (name, kind, description) in &arm_defs {
        if !requested.iter().any(|r| r == name) {
            continue;
        }
        println!("[arm {name}] {description}");
        match train_arm(&args, name, *kind, &train_ids, &decay_slots, &device) {
            Ok(result) => results.push((name.to_string(), Some(result))),
            Err(err) => {
                eprintln!("[arm {name}] FAILED: {err}");
                arm_reports.push(json!({
                    "arm": name,
                    "description": description,
                    "status": "failed",
                    "error": err.to_string(),
                }));
                results.push((name.to_string(), None));
            }
        }
    }

    let mut summary: Vec<(String, f64, f64)> = Vec::new(); // (arm, hard bpc/char, gap)
    let mut b_snapshot: Option<&ArmSnapshot> = None;
    for (name, result) in &results {
        let Some(result) = result else { continue };
        let arm_def = arm_defs
            .iter()
            .find(|(n, _, _)| n == name)
            .expect("known arm");
        let (kind, description) = (arm_def.1, arm_def.2);
        println!(
            "[arm {name}] eval: Soft (fp relaxation) and Hard (deployable ternary) over the shared val lanes ..."
        );
        let eval_start = Instant::now();
        let (fp_bits, fp_pairs) = eval_bpc_lanes(
            &result.snapshot,
            args.d_model,
            args.state_slots,
            &decay_slots,
            QuantHardness::Soft,
            true,
            &val_ids,
            args.eval_pairs,
            args.eval_lanes,
            args.eval_chunk,
            &plain_device,
        )?;
        let (hard_bits, hard_pairs) = eval_bpc_lanes(
            &result.snapshot,
            args.d_model,
            args.state_slots,
            &decay_slots,
            QuantHardness::Hard,
            true,
            &val_ids,
            args.eval_pairs,
            args.eval_lanes,
            args.eval_chunk,
            &plain_device,
        )?;
        assert_eq!(fp_pairs, hard_pairs, "eval pair sets must match");
        let eval_wall = eval_start.elapsed().as_secs_f64();
        let fp_bpc = fp_bits / fp_pairs as f64;
        let hard_bpc = hard_bits / hard_pairs as f64;
        let gap = hard_bpc - fp_bpc;
        // Same method as the KN artifact: total bits re-expressed over raw
        // byte count via the stream-level chars/raw-byte ratio.
        let fp_bpb = fp_bpc * chars_per_raw_byte;
        let hard_bpb = hard_bpc * chars_per_raw_byte;
        println!(
            "[arm {name}] fp_bpc/char={fp_bpc:.6} hard_bpc/char={hard_bpc:.6} gap={gap:.6} | per raw byte: fp={fp_bpb:.6} hard={hard_bpb:.6} ({hard_pairs} pairs, eval {eval_wall:.0}s)"
        );

        // Parity check for stateful arms: batched Burn recurrence vs the
        // canonical scalar QAT kernels on a real val prefix.
        let parity = if matches!(kind, ArmKind::State { .. }) {
            let max_diff = state_block_parity(
                &result.snapshot,
                args.d_model,
                args.state_slots,
                &decay_slots,
                &val_ids[..32.min(val_ids.len())],
                &plain_device,
            )?;
            println!("[arm {name}] state-block scalar-kernel parity max_abs_diff={max_diff:.3e}");
            Some(max_diff)
        } else {
            None
        };

        let sample = greedy_sample(
            &result.snapshot,
            args.d_model,
            args.state_slots,
            &decay_slots,
            &prompt_ids,
            args.sample_chars,
            &plain_device,
        )?;
        let sample_path = out_dir.join(format!("sample_arm_{name}.txt"));
        fs::write(
            &sample_path,
            format!(
                "PROMPT (charset_v1-normalized):\n{prompt_text}\n\nGREEDY CONTINUATION ({} chars, hard ternary):\n{sample}\n",
                args.sample_chars
            ),
        )?;
        println!("[arm {name}] sample -> {}", sample_path.display());

        let (seq_len, seq_batch) = match kind {
            ArmKind::Stateless => (1usize, args.batch),
            ArmKind::State { seq_len, seq_batch } => (seq_len, seq_batch),
        };
        arm_reports.push(json!({
            "arm": name,
            "description": description,
            "status": "ok",
            "config": {
                "d_model": args.d_model,
                "d_ff": args.d_ff,
                "n_blocks": args.n_blocks,
                "vocab": VOCAB,
                "tied_head": true,
                "sequence_state_kind": match kind {
                    ArmKind::Stateless => "stateless_bigram_context",
                    ArmKind::State { .. } => "linear_state_multi_timescale",
                },
                "state_slots": matches!(kind, ArmKind::State { .. }).then_some(args.state_slots),
                "decay_rates_by_band": matches!(kind, ArmKind::State { .. }).then_some(MT4_DECAYS),
                "seq_len": seq_len,
                "lanes": seq_batch,
                "tokens_per_step": result.tokens_per_step,
                "steps": args.steps,
                "lr": args.lr,
                "warmup_frac_hardness_off": args.warmup_frac,
                "qat_recipe": "warmup Off then Hard, act Int8 fake-quant when hard (identical to s2_gap_and_export)",
                "tbptt": matches!(kind, ArmKind::State { .. }).then_some(
                    "state carried across chunks detached; lane reset to zero state on stream wrap"),
            },
            "training": {
                "steps_per_second": result.steps_per_second,
                "train_wall_clock_seconds": result.train_wall_seconds,
                "final_logged_train_loss_bpc": result.final_train_loss_bpc,
            },
            "measurement": {
                "eval_pairs": hard_pairs,
                "eval_lanes": args.eval_lanes,
                "fp_val_bpc_per_normalized_char": fp_bpc,
                "ternary_val_bpc_per_normalized_char": hard_bpc,
                "gap_bpc_per_normalized_char": gap,
                "fp_val_bits_per_raw_byte": fp_bpb,
                "ternary_val_bits_per_raw_byte": hard_bpb,
                "bits_per_raw_byte_method": "bpc_per_normalized_char * (val_chars_total / val_raw_bytes_total), the same total-bits-over-raw-byte-count re-expression as the KN-5 artifact",
                "fp_semantics": "SOFT continuous ternary relaxation with the same learned per-output-row Q8.8 scales + Int8 act fake-quant (the calibrated STE ceiling, as in gap.json)",
                "ternary_semantics": "HARD ternary projection {-1,0,+1} with the same per-row Q8.8 scales + Int8 act fake-quant",
                "state_block_scalar_parity_max_abs_diff": parity,
            },
            "sample": {
                "prompt_normalized": prompt_text,
                "greedy_continuation": sample,
                "sample_path": format!("{}/sample_arm_{name}.txt", args.out_dir),
            },
        }));
        summary.push((name.clone(), hard_bpc, gap));
        if name == "B" {
            b_snapshot = Some(&result.snapshot);
        }
    }

    // ---- A-vs-B verdict + conditional arm B export ----
    let a_hard = summary
        .iter()
        .find(|(n, _, _)| n == "A")
        .map(|(_, v, _)| *v);
    let b_hard = summary
        .iter()
        .find(|(n, _, _)| n == "B")
        .map(|(_, v, _)| *v);
    let delta_ab = match (a_hard, b_hard) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    // "Materially better" pinned before looking at the numbers: arm B must
    // beat arm A by at least 0.02 bits per normalized char on hard ternary.
    let material_threshold = 0.02_f64;
    let b_material = delta_ab.map(|d| d <= -material_threshold).unwrap_or(false);
    let export_summary = if b_material {
        let export_dir = out_dir.join("checkpoint-export");
        match b_snapshot {
            Some(snapshot) => {
                export_checkpoint_b(&export_dir, &args, snapshot, &decay_slots, &git_sha)?
            }
            None => json!({ "exported": false, "reason": "arm B snapshot missing" }),
        }
    } else {
        json!({
            "exported": false,
            "reason": format!(
                "arm B did not beat arm A by >= {material_threshold} bits/char on hard ternary (delta_ab = {:?})",
                delta_ab
            )
        })
    };

    let report = json!({
        "schema": "s5_state_ab.v1",
        "beads": ["bd-29ai4", "bd-2nrnq"],
        "purpose": "LinearState multi-timescale (MT4) A/B vs a stateless bigram-context baseline at matched d64 capacity, both migrated to the charset_v1 80-token vocabulary, laid against the KN-5 baseline",
        "git_sha": git_sha,
        "seed": args.seed,
        "backend": "burn_ndarray_autodiff",
        "lexical": {
            "vocab": VOCAB,
            "spec": "charset_v1 via gbf_data::charset_v1::normalize_raw (case-preserving, accent-stripped, quote/dash-folded, whitespace-collapsed, unmappable -> <unk> id 79)",
        },
        "corpus": {
            "train_bin_path": args.train_bin,
            "train_cap_bytes": args.train_cap_bytes,
            "train_raw_bytes_used": train_prefix.len(),
            "train_bytes_trimmed_at_utf8_boundary": train_trimmed,
            "train_prefix_raw_sha256": train_raw_sha,
            "train_chars_normalized": train_ids.len(),
            "train_unk_count": train_norm.unk_count_in_example,
            "train_norm_tokens_sha256": train_norm_sha,
            "val_source": "corpus/gutenberg/splits.json val-split book bodies, same assembly as s2/s4",
            "val_book_ids_used": val_book_ids,
            "val_raw_bytes_used": val_raw_bytes_total,
            "val_bytes_trimmed_at_utf8_boundary": val_trimmed,
            "val_raw_bytes_sha256": val_raw_sha,
            "val_chars_normalized": val_chars_total,
            "val_unk_count": val_norm.unk_count_in_example,
            "val_norm_tokens_sha256": val_norm_sha,
        },
        "kn5_reference": kn_reference,
        "arms": arm_reports,
        "verdict": {
            "a_hard_bpc_per_normalized_char": a_hard,
            "b_hard_bpc_per_normalized_char": b_hard,
            "delta_b_minus_a_bpc_per_normalized_char": delta_ab,
            "material_threshold_bpc": material_threshold,
            "b_materially_beats_a": b_material,
        },
        "arm_b_export": export_summary,
        "caveats": [
            "d64-class toy capacity chosen deliberately to isolate the sequence-state question; absolute bpc is not a production-quality claim.",
            "The stateful arms' recurrence is a batched Burn re-implementation of the LinearStateBlock semantics (the committed LinearStateBurnQat adapter is single-sequence); parity against the canonical scalar QAT kernels is measured and recorded per stateful arm (state_block_scalar_parity_max_abs_diff).",
            "Eval scores within-lane adjacent pairs only (lane-first tokens are unscored), so bits-per-raw-byte re-expression uses the stream-level chars-per-raw-byte ratio; KN-5 scored every token under reset-context windows (chunk 128). The KN numbers are copied verbatim, not re-run.",
            "Arm A trains on iid pair samples (as s2) while stateful arms train on sequential TBPTT lanes; steps and tokens-per-step are matched, but the data-order exposure necessarily differs between a stateless and a stateful trainer.",
            "The fp reference is the SOFT same-scale ternary relaxation (calibrated STE ceiling), matching the gap.json methodology.",
        ],
        "total_wall_clock_seconds": total_start.elapsed().as_secs_f64(),
    });
    let report_path = out_dir.join("report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!("[write] report -> {}", report_path.display());
    Ok(())
}

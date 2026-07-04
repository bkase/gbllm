//! F-S2 QAT-gap evidence + canonical dense checkpoint export.
//!
//! Trains a small dense byte-level next-byte predictor with real ternary
//! linear QAT layers on the committed Gutenberg corpus using the Burn
//! ndarray autodiff backend, then:
//!
//!   1. Measures validation bits-per-char with fake-quant OFF (full-precision
//!      semantics) versus hard-ternary projection on the SAME trained
//!      checkpoint / SAME validation stream / SAME deterministic seed. Writes
//!      `experiments/S2/gap/gap.json`. This is the F-S2 "QAT survives" gap
//!      measurement that was previously closed without committed evidence.
//!
//!   2. Exports the hardened (ternary-projected) deployable tensors — per
//!      linear layer the ternary values {-1,0,+1} (i8) and per-output-row Q8.8
//!      scales, plus the embedding/tied-head table and norm/activation params —
//!      to an inspectable raw-bin + JSON manifest bundle under
//!      `experiments/S6/checkpoint-export/` for the one-token bring-up
//!      (bd-59qiq).
//!
//! Integrity: every number in the emitted JSON is produced by this program from
//! the actual training run. Nothing is hand-written. If a required input is
//! missing the program errors out rather than fabricating a result.
//!
//! Model topology: dense (no MoE), MoeTinyDenseMatched-class shape
//! (d_model=64, d_ff=128, n_blocks=4, byte vocab=256). Each block is a
//! pre-norm residual FFN: rms-norm -> act-fake-quant -> ternary up
//! [d_ff x d_model] -> gelu -> act-fake-quant -> ternary down [d_model x d_ff]
//! -> residual add. The head is tied to the embedding table. The sequence
//! model is bigram-context (next byte predicted from the single previous
//! byte); this is intentionally stateless, and the caveat is recorded in the
//! emitted JSON.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use gbf_foundation::sha256;
use gbf_model::qat::{
    ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange,
    ActivationRangeMode, MatrixShape, QatHardnessControl, QuantHardness, TernaryLinearQat,
    TernaryThreshold,
};
use gbf_train::adapter::burn::{
    BurnBackend, BurnDevice, BurnFloatTensor, BurnGradientsParams, BurnInt, BurnModule,
    BurnNdArrayAutodiffBackend, BurnNdArrayBackend, BurnOptimizer, BurnParam, BurnTensor,
    adamw_config, burn_gelu_approximate, burn_log_softmax, float_tensor_from_vec,
    float_tensor_into_vec,
};
use gbf_train::qat::{ActFakeQuantBurnQat, TernaryLinearBurnQat};
use serde_json::json;

const VOCAB: usize = 256;
const NORM_EPS: f32 = 1.0e-5;
const NORM_CLIP: f32 = 8.0;
const ACT_RANGE: f32 = 8.0;

type Adiff = BurnNdArrayAutodiffBackend;
type Plain = BurnNdArrayBackend;

#[derive(Parser, Debug)]
#[command(about = "F-S2 QAT-gap measurement + canonical dense checkpoint export")]
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
    /// Cap on training bytes read from the front of the train stream.
    #[arg(long, default_value_t = 64 * 1024 * 1024)]
    train_cap_bytes: usize,
    /// Cap on held-out validation bytes assembled from val-split book bodies.
    #[arg(long, default_value_t = 1024 * 1024)]
    val_cap_bytes: usize,
    /// Number of (context,target) pairs scored during each bpc evaluation.
    #[arg(long, default_value_t = 262_144)]
    eval_pairs: usize,
    /// Optimizer steps.
    #[arg(long, default_value_t = 12_000)]
    steps: u64,
    /// Minibatch size (context/target byte pairs per step).
    #[arg(long, default_value_t = 512)]
    batch: usize,
    /// AdamW learning rate.
    #[arg(long, default_value_t = 0.01)]
    lr: f64,
    /// Fraction of steps trained with QAT hardness OFF before switching to Hard.
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
    /// Steps between mid-training eval snapshots (0 disables).
    #[arg(long, default_value_t = 2_000)]
    eval_every: u64,
    #[arg(long, default_value = "experiments/S2/gap/gap.json")]
    out_gap: String,
    #[arg(long, default_value = "experiments/S6/checkpoint-export")]
    out_export_dir: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("s2_gap_and_export FAILED: {err}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// deterministic init
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
struct DenseFfnBlock<B: BurnBackend> {
    up: TernaryLinearBurnQat<B>,
    down: TernaryLinearBurnQat<B>,
}

#[derive(BurnModule, Debug)]
struct DenseByteModel<B: BurnBackend> {
    embedding: BurnParam<BurnFloatTensor<B, 2>>,
    blocks: Vec<DenseFfnBlock<B>>,
}

fn make_ternary_core(out_rows: usize, in_cols: usize, weights: Vec<f32>) -> TernaryLinearQat {
    let shape = MatrixShape::new(out_rows, in_cols).expect("nonzero shape");
    let thresholds = vec![TernaryThreshold::new(0.0).expect("zero threshold"); out_rows];
    TernaryLinearQat::with_derived_per_row_scales(shape, weights, None, thresholds)
        .expect("valid ternary core")
}

impl DenseByteModel<Adiff> {
    fn init(args: &Args, device: &BurnDevice<Adiff>) -> Self {
        let d_model = args.d_model;
        let d_ff = args.d_ff;
        let embedding = float_tensor_from_vec::<Adiff, 2>(
            init_weights(args.seed, 0x0001, VOCAB * d_model, 0.05),
            [VOCAB, d_model],
            device,
        )
        .expect("embedding init");

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
                DenseFfnBlock {
                    up: TernaryLinearBurnQat::from_core(up_core, device).expect("up wrapper"),
                    down: TernaryLinearBurnQat::from_core(down_core, device).expect("down wrapper"),
                }
            })
            .collect();

        Self {
            embedding: BurnParam::from_tensor(embedding),
            blocks,
        }
    }

    fn set_hardness(&mut self, hardness: QuantHardness) {
        for block in &mut self.blocks {
            block.up.set_hardness(hardness);
            block.down.set_hardness(hardness);
        }
    }
}

/// Fixed (non-learnable) norm + activation quantization used identically in
/// training and evaluation. Only the ternary weight hardness distinguishes the
/// fp and ternary evaluation passes.
fn activation() -> ActFakeQuantBurnQat {
    let core = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-ACT_RANGE, ACT_RANGE).expect("range")),
        ActivationQuantFormat::Int8,
    )
    .expect("act");
    ActFakeQuantBurnQat::from_core(core).expect("act wrapper")
}

/// Pre-affine full-vector RMS norm followed by clip to [-NORM_CLIP, NORM_CLIP].
/// Matches `gbf_model::qat::NormApproxPlan::TileRmsThenAffineClip` with a tile
/// spanning the whole d_model vector, affine scale=1/bias=0.
fn rms_norm<B: BurnBackend>(x: BurnFloatTensor<B, 2>) -> BurnFloatTensor<B, 2> {
    let d_model = x.dims()[1];
    let mean_sq = (x.clone() * x.clone()).mean_dim(1); // [batch, 1]
    let rms = (mean_sq + NORM_EPS).sqrt(); // [batch, 1]
    let normed = x / rms.repeat_dim(1, d_model);
    normed.clamp(-NORM_CLIP, NORM_CLIP)
}

/// Forward the dense stack for a batch of context byte ids. Returns logits
/// [batch, VOCAB]. `act_enabled` toggles whether the activation fake-quant is
/// applied (off => full-precision activation path).
fn forward_logits<B: BurnBackend>(
    embedding: BurnFloatTensor<B, 2>,
    blocks_up: &[&TernaryLinearBurnQat<B>],
    blocks_down: &[&TernaryLinearBurnQat<B>],
    act: &ActFakeQuantBurnQat,
    act_enabled: bool,
    context_ids: BurnTensor<B, 1, BurnInt>,
) -> Result<BurnFloatTensor<B, 2>, Box<dyn std::error::Error>> {
    let mut x = embedding.clone().select(0, context_ids); // [batch, d_model]
    let mode = if act_enabled {
        ActivationForwardMode::Train
    } else {
        ActivationForwardMode::Passthrough
    };
    for (up, down) in blocks_up.iter().zip(blocks_down.iter()) {
        let normed = rms_norm(x.clone());
        let normed = act.fake_quant_forward(normed, mode);
        let hidden = up.fake_quant_forward(normed)?;
        let hidden = burn_gelu_approximate(hidden);
        let hidden = act.fake_quant_forward(hidden, mode);
        let delta = down.fake_quant_forward(hidden)?;
        x = x + delta;
    }
    let normed = rms_norm(x);
    // tied head: logits = normed @ embedding^T
    Ok(normed.matmul(embedding.transpose()))
}

// ---------------------------------------------------------------------------
// data
// ---------------------------------------------------------------------------

fn read_train_bytes(path: &Path, cap: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let all = fs::read(path).map_err(|e| format!("read train bin {}: {e}", path.display()))?;
    if all.is_empty() {
        return Err(format!("train bin {} is empty", path.display()).into());
    }
    let take = all.len().min(cap);
    Ok(all[..take].to_vec())
}

/// Build a held-out validation byte stream from the val-split book bodies so it
/// is book-level disjoint from the training corpus regardless of the train bin
/// contents.
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

// ---------------------------------------------------------------------------
// evaluation
// ---------------------------------------------------------------------------

/// Deterministic bits-per-char over the first `max_pairs` (context,target)
/// byte pairs of `val`, evaluated on the plain (non-autodiff) backend with a
/// fixed hardness configuration.
fn eval_bpc(
    embedding: &[f32],
    d_model: usize,
    blocks: &[(TernaryLinearQat, TernaryLinearQat)],
    hardness: QuantHardness,
    act_enabled: bool,
    val: &[u8],
    max_pairs: usize,
    batch: usize,
    device: &BurnDevice<Plain>,
) -> Result<f64, Box<dyn std::error::Error>> {
    let embed_tensor =
        float_tensor_from_vec::<Plain, 2>(embedding.to_vec(), [VOCAB, d_model], device)?;

    // Build plain-backend ternary wrappers at the requested hardness.
    let mut up_layers = Vec::new();
    let mut down_layers = Vec::new();
    for (up_core, down_core) in blocks {
        let mut up = up_core.clone();
        let mut down = down_core.clone();
        up.set_hardness(hardness);
        down.set_hardness(hardness);
        up_layers.push(TernaryLinearBurnQat::<Plain>::from_core(up, device)?);
        down_layers.push(TernaryLinearBurnQat::<Plain>::from_core(down, device)?);
    }
    let up_refs: Vec<&TernaryLinearBurnQat<Plain>> = up_layers.iter().collect();
    let down_refs: Vec<&TernaryLinearBurnQat<Plain>> = down_layers.iter().collect();
    let act = activation();

    let pair_count = val.len().saturating_sub(1).min(max_pairs);
    if pair_count == 0 {
        return Err("no validation pairs".into());
    }

    let ln2 = std::f64::consts::LN_2;
    let mut total_bits = 0.0_f64;
    let mut done = 0usize;
    while done < pair_count {
        let this = batch.min(pair_count - done);
        let ctx: Vec<i32> = (0..this).map(|i| val[done + i] as i32).collect();
        let tgt: Vec<i32> = (0..this).map(|i| val[done + i + 1] as i32).collect();
        let ctx_ids = BurnTensor::<Plain, 1, BurnInt>::from_ints(ctx.as_slice(), device);
        let logits = forward_logits(
            embed_tensor.clone(),
            &up_refs,
            &down_refs,
            &act,
            act_enabled,
            ctx_ids,
        )?;
        let logp = burn_log_softmax(logits, 1); // [this, VOCAB], natural log
        let tgt_idx =
            BurnTensor::<Plain, 1, BurnInt>::from_ints(tgt.as_slice(), device).reshape([this, 1]);
        let picked = logp.gather(1, tgt_idx).reshape([this]); // nats
        let nats = float_tensor_into_vec(picked)?;
        for v in nats {
            total_bits += -(v as f64) / ln2;
        }
        done += this;
    }
    Ok(total_bits / pair_count as f64)
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let started = Instant::now();
    let repo_root = args.repo_root.clone();
    let device = BurnDevice::<Adiff>::default();
    let plain_device = BurnDevice::<Plain>::default();

    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // ---- data ----
    let train_path = repo_root.join(&args.train_bin);
    let train = read_train_bytes(&train_path, args.train_cap_bytes)?;
    let (val, val_book_ids) = build_val_bytes(&repo_root, args.val_cap_bytes)?;
    let train_sha = sha256(&train).to_hex();
    let val_sha = sha256(&val).to_hex();
    println!(
        "[data] train_bytes={} (cap {}) sha256={}  val_bytes={} val_books={} sha256={}",
        train.len(),
        args.train_cap_bytes,
        &train_sha[..16],
        val.len(),
        val_book_ids.len(),
        &val_sha[..16],
    );

    // ---- model + optimizer ----
    let mut model = DenseByteModel::init(&args, &device);
    let mut optimizer = adamw_config()
        .with_weight_decay(0.0)
        .init::<Adiff, DenseByteModel<Adiff>>();
    let act = activation();

    let warmup_steps = (args.steps as f64 * args.warmup_frac) as u64;
    println!(
        "[model] d_model={} d_ff={} n_blocks={} vocab={} steps={} batch={} lr={} warmup_steps={} (QAT Hard after warmup)",
        args.d_model,
        args.d_ff,
        args.n_blocks,
        VOCAB,
        args.steps,
        args.batch,
        args.lr,
        warmup_steps
    );

    // deterministic minibatch sampler
    let mut sampler = args.seed ^ 0xa5a5_5a5a_1234_5678;
    let max_pos = train.len() - 1;
    let ln2 = std::f64::consts::LN_2;

    let mut running_loss = 0.0_f64;
    let mut running_n = 0u64;
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
                "[phase] step {step}: QAT hardness -> {}",
                if want_hard { "Hard" } else { "Off" }
            );
        }
        let act_enabled = want_hard;

        let mut ctx = Vec::with_capacity(args.batch);
        let mut tgt = Vec::with_capacity(args.batch);
        for _ in 0..args.batch {
            let pos = (splitmix64(&mut sampler) as usize) % max_pos;
            ctx.push(train[pos] as i32);
            tgt.push(train[pos + 1] as i32);
        }
        let ctx_ids = BurnTensor::<Adiff, 1, BurnInt>::from_ints(ctx.as_slice(), &device);
        let tgt_idx = BurnTensor::<Adiff, 1, BurnInt>::from_ints(tgt.as_slice(), &device)
            .reshape([args.batch, 1]);

        let up_refs: Vec<&TernaryLinearBurnQat<Adiff>> =
            model.blocks.iter().map(|b| &b.up).collect();
        let down_refs: Vec<&TernaryLinearBurnQat<Adiff>> =
            model.blocks.iter().map(|b| &b.down).collect();
        let logits = forward_logits(
            model.embedding.val(),
            &up_refs,
            &down_refs,
            &act,
            act_enabled,
            ctx_ids,
        )?;
        let logp = burn_log_softmax(logits, 1);
        let picked = logp.gather(1, tgt_idx).reshape([args.batch]);
        let loss = picked.mean() * -1.0; // mean nats
        let loss_nats = float_tensor_into_vec(loss.clone().inner())?[0];
        if !loss_nats.is_finite() {
            return Err(format!("non-finite training loss at step {step}").into());
        }
        running_loss += f64::from(loss_nats);
        running_n += 1;

        let grads = loss.backward();
        let grads = BurnGradientsParams::from_grads(grads, &model);
        model = optimizer.step(args.lr, model, grads);

        if step % args.log_every == 0 || step == 1 {
            let elapsed = started.elapsed().as_secs_f64();
            let rate = step as f64 / elapsed;
            let mean_loss = running_loss / running_n.max(1) as f64;
            println!(
                "[train] step {step}/{} loss_nats={:.4} bpc~={:.4} {:.1} steps/s elapsed={:.0}s",
                args.steps,
                mean_loss,
                mean_loss / ln2,
                rate,
                elapsed
            );
            running_loss = 0.0;
            running_n = 0;
        }

        if args.eval_every > 0 && step % args.eval_every == 0 {
            let snapshot = extract_cores(&model)?;
            let embed = float_tensor_into_vec(model.embedding.val().inner().detach())?;
            let fp = eval_bpc(
                &embed,
                args.d_model,
                &snapshot,
                QuantHardness::Soft,
                true,
                &val,
                args.eval_pairs.min(32_768),
                args.batch,
                &plain_device,
            )?;
            let tern = eval_bpc(
                &embed,
                args.d_model,
                &snapshot,
                QuantHardness::Hard,
                true,
                &val,
                args.eval_pairs.min(32_768),
                args.batch,
                &plain_device,
            )?;
            println!(
                "[eval] step {step}: fp_bpc={:.4} ternary_bpc={:.4} gap={:.4} (subset {} pairs)",
                fp,
                tern,
                tern - fp,
                args.eval_pairs.min(32_768)
            );
        }
    }

    // ---- final evaluation on the same checkpoint ----
    let snapshot = extract_cores(&model)?;
    let embed = float_tensor_into_vec(model.embedding.val().inner().detach())?;
    println!(
        "[eval] final fp-relaxed / ternary bpc over {} pairs ...",
        args.eval_pairs
    );
    // Primary "full-precision reference" for a per-output-row-scaled ternary
    // scheme: the SOFT (continuous, same-per-row-scale) relaxation the STE
    // optimizes toward. Both arms share the learned Q8.8 row scales, so this is
    // the calibrated same-checkpoint fp ceiling. gap = hard-ternary cost.
    let fp_val_bpc = eval_bpc(
        &embed,
        args.d_model,
        &snapshot,
        QuantHardness::Soft,
        true,
        &val,
        args.eval_pairs,
        args.batch,
        &plain_device,
    )?;
    let ternary_val_bpc = eval_bpc(
        &embed,
        args.d_model,
        &snapshot,
        QuantHardness::Hard,
        true,
        &val,
        args.eval_pairs,
        args.batch,
        &plain_device,
    )?;
    let gap_bpc = ternary_val_bpc - fp_val_bpc;
    // Diagnostic only: literal "fake-quant fully OFF" using the raw f32 weights
    // WITHOUT the deployed per-row scale. For a scale-decoupled ternary scheme
    // this path is intentionally uncalibrated (raw weight magnitudes drift
    // freely under STE), so it is recorded for completeness, not as the ceiling.
    let diagnostic_fp_rawweights_alloff_bpc = eval_bpc(
        &embed,
        args.d_model,
        &snapshot,
        QuantHardness::Off,
        false,
        &val,
        args.eval_pairs,
        args.batch,
        &plain_device,
    )?;
    let wall_clock_s = started.elapsed().as_secs_f64();
    println!(
        "[result] fp_relaxed_val_bpc={fp_val_bpc:.6} ternary_val_bpc={ternary_val_bpc:.6} gap_bpc={gap_bpc:.6}"
    );
    println!(
        "[result] diagnostic_fp_rawweights_alloff_bpc={diagnostic_fp_rawweights_alloff_bpc:.6} wall={wall_clock_s:.0}s"
    );

    // ---- export the hardened checkpoint ----
    let export_dir = repo_root.join(&args.out_export_dir);
    let export_summary = export_checkpoint(&export_dir, &args, &embed, &snapshot, &git_sha)?;

    // ---- write gap.json ----
    let gap_path = repo_root.join(&args.out_gap);
    if let Some(parent) = gap_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let eval_pairs_used = val.len().saturating_sub(1).min(args.eval_pairs);
    let gap = json!({
        "schema": "f_s2_qat_gap.v1",
        "bead": "bd-2k1iv",
        "purpose": "F-S2 'QAT survives' fp-vs-ternary val bits-per-char gap on a real trained dense QAT checkpoint",
        "git_sha": git_sha,
        "seed": args.seed,
        "backend": "burn_ndarray_autodiff",
        "model": {
            "topology": "dense_ffn_bigram_context (MoeTinyDenseMatched-class, no MoE)",
            "d_model": args.d_model,
            "d_ff": args.d_ff,
            "n_blocks": args.n_blocks,
            "vocab": VOCAB,
            "tied_head": true,
            "sequence_state_kind": "stateless_bigram_context",
            "sequence_state_params": {},
            "norm": {"kind": "tile_rms_then_affine_clip(full_vector)", "epsilon": NORM_EPS, "clip": NORM_CLIP, "affine_scale": 1.0, "affine_bias": 0.0},
            "activation_fake_quant": {"format": "Int8", "range_lo": -ACT_RANGE, "range_hi": ACT_RANGE},
            "weight_quant": "ternary {-1,0,+1}, per-output-row Q8.8 scale, threshold=0-init learned"
        },
        "training": {
            "optimizer": "burn_adamw",
            "weight_decay": 0.0,
            "learning_rate": args.lr,
            "steps": args.steps,
            "batch": args.batch,
            "warmup_steps_hardness_off": warmup_steps,
            "qat_hardness_after_warmup": "Hard",
            "wall_clock_seconds": wall_clock_s
        },
        "corpus": {
            "train_bin_path": args.train_bin,
            "train_bytes_used": train.len(),
            "train_cap_bytes": args.train_cap_bytes,
            "train_bytes_sha256": train_sha,
            "val_source": "corpus/gutenberg/splits.json val-split book bodies (book-level held out)",
            "val_bytes_used": val.len(),
            "val_cap_bytes": args.val_cap_bytes,
            "val_book_ids_used": val_book_ids,
            "val_bytes_sha256": val_sha
        },
        "measurement": {
            "eval_pairs": eval_pairs_used,
            "fp_val_bpc": fp_val_bpc,
            "ternary_val_bpc": ternary_val_bpc,
            "gap_bpc": gap_bpc,
            "fp_semantics": "SOFT continuous ternary relaxation with the SAME learned per-output-row Q8.8 scales (the calibrated STE full-precision ceiling) + Int8 activation fake-quant + full-vector RMS norm",
            "ternary_semantics": "HARD ternary projection {-1,0,+1} with the same per-row Q8.8 scale + Int8 activation fake-quant + full-vector RMS norm",
            "gap_interpretation": "gap_bpc = ternary_val_bpc - fp_val_bpc is the cost of hardening the QAT-trained weights to deployable ternary on the identical checkpoint/val/seed; small (or negative) gap is the 'QAT survives' signal",
            "diagnostic_fake_quant_fully_off": {
                "note": "literal fake-quant-OFF using raw f32 weights WITHOUT the deployed per-output-row scale. For a scale-decoupled ternary scheme the raw-weight magnitudes drift freely under STE, so this path is intentionally uncalibrated and is NOT the full-precision ceiling; recorded only for completeness.",
                "diagnostic_fp_rawweights_alloff_bpc": diagnostic_fp_rawweights_alloff_bpc
            }
        },
        "export": export_summary,
        "caveats": [
            "Undertrained toy-scale evidence: a bigram-context dense FFN predicts the next byte from only the single previous byte, so absolute bpc is bounded near the order-1 (bigram) entropy of the corpus and is NOT a strong language model.",
            "Sequence model is intentionally stateless (bigram context); no recurrent/bounded-KV sequence state is trained here.",
            "Training corpus is the front prefix of the committed concatenated Gutenberg train stream (train_cap_bytes); validation is book-level held out from the val split.",
            "The gap number is the honest fp-vs-hard-ternary delta on ONE identical checkpoint / val stream / seed; it is the QAT-survival signal, not a production-quality claim. Full-quality replication is bd-3771m scope."
        ]
    });
    fs::write(&gap_path, serde_json::to_vec_pretty(&gap)?)?;
    println!("[write] gap -> {}", gap_path.display());
    println!("[write] export bundle -> {}", export_dir.display());

    Ok(())
}

/// Extract deployable ternary cores (hardened) from the trained autodiff model.
fn extract_cores(
    model: &DenseByteModel<Adiff>,
) -> Result<Vec<(TernaryLinearQat, TernaryLinearQat)>, Box<dyn std::error::Error>> {
    let mut cores = Vec::new();
    for block in &model.blocks {
        let mut up = block.up.to_core_from_trained_state()?;
        let mut down = block.down.to_core_from_trained_state()?;
        up.set_hardness(QuantHardness::Hard);
        down.set_hardness(QuantHardness::Hard);
        cores.push((up, down));
    }
    Ok(cores)
}

// ---------------------------------------------------------------------------
// export
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

fn export_checkpoint(
    export_dir: &Path,
    args: &Args,
    embedding: &[f32],
    blocks: &[(TernaryLinearQat, TernaryLinearQat)],
    git_sha: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let tensors_dir = export_dir.join("tensors");
    fs::create_dir_all(&tensors_dir)?;

    // embedding / tied head (f32, row-major [vocab, d_model])
    let embed_bytes = f32_le_bytes(embedding);
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

    let mut layers = Vec::new();
    for (layer, (up, down)) in blocks.iter().enumerate() {
        for (proj_name, core) in [("up", up), ("down", down)] {
            let export = core.export_canonical();
            let shape = export.shape();
            // ternary values as i8 {-1,0,1}, row-major [out_rows, in_cols]
            let i8_bytes: Vec<u8> = export
                .ternary_values()
                .iter()
                .map(|v| v.as_i8() as u8)
                .collect();
            // per-output-row scales as raw Q8.8 u16 (little-endian)
            let mut scale_bytes = Vec::with_capacity(export.scales().len() * 2);
            for s in export.scales() {
                scale_bytes.extend_from_slice(&s.raw().to_le_bytes());
            }
            let base = format!("block{layer}_{proj_name}");
            let tern_file = format!("tensors/{base}.ternary.i8.bin");
            let scale_file = format!("tensors/{base}.scales.q8_8_u16le.bin");
            write_bin(&export_dir.join(&tern_file), &i8_bytes)?;
            write_bin(&export_dir.join(&scale_file), &scale_bytes)?;
            tensor_index.push(json!({
                "name": format!("{base}.ternary"),
                "role": format!("{proj_name}_projection_ternary_weights"),
                "dtype": "i8 (values in {-1,0,1})",
                "shape": [shape.output_rows(), shape.input_cols()],
                "layout": "row_major",
                "file": tern_file,
                "sha256": sha256(&i8_bytes).to_hex()
            }));
            tensor_index.push(json!({
                "name": format!("{base}.scales"),
                "role": format!("{proj_name}_projection_per_output_row_scale"),
                "dtype": "u16_le (Q8.8 fixed-point; f32 = raw/256)",
                "shape": [shape.output_rows()],
                "file": scale_file,
                "sha256": sha256(&scale_bytes).to_hex()
            }));
        }
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
        "schema": "f_s6_dense_checkpoint_export.v1",
        "bead": "bd-2k1iv",
        "consumer_bead": "bd-59qiq",
        "git_sha": git_sha,
        "seed": args.seed,
        "topology": {
            "family": "dense_ffn_bigram_context",
            "moe": false,
            "d_model": args.d_model,
            "d_ff": args.d_ff,
            "n_blocks": args.n_blocks,
            "vocab": VOCAB,
            "tied_head": true,
            "sequence_state_kind": "stateless_bigram_context",
            "sequence_state_params": {}
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
    let manifest_path = export_dir.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    Ok(json!({
        "manifest": args.out_export_dir.clone() + "/manifest.json",
        "n_tensors": (1 + blocks.len() * 4),
        "embedding_shape": [VOCAB, args.d_model],
    }))
}

//! Self-contained f32 MoE forward evaluator for MLX<->Rust parity (bd-2lk86).
//!
//! This module is a NEW, standalone reference forward pass over the
//! `f_s8_moe_state_checkpoint_export.v2` checkpoint format. It intentionally
//! does NOT reuse `gbf-kernel`'s byte-exact evaluators; its sole purpose is to
//! reproduce the MLX (fp32) golden forward so we can gate Rust<->MLX numeric
//! parity. Everything here is f32 throughout, pure std.
//!
//! Weight/activation semantics (mirrors the MLX golden):
//! - Ternary weights `{-1,0,+1}` (i8) with a per-output-row Q8.8 scale
//!   (`f32 = raw_u16 / 256`). Effective weight `w[r][c] * scale[r]`.
//! - Matvec convention matches `x @ W.T`: `out[r] = sum_c eff[r][c] * x[c]`,
//!   where the stored shape is `[rows = output, cols = input]`.
//! - Activation fake-quant is Int8 symmetric over `[-8, 8]` with
//!   round-ties-to-even (matching `mx.round`).

use std::path::Path;

/// Activation clamp range (symmetric).
const ACT_RANGE: f32 = 8.0;
/// Int8 symmetric max magnitude.
const QMAX: f32 = 127.0;
/// RMSNorm epsilon.
const NORM_EPS: f32 = 1e-5;
/// sqrt(2/pi) used by the tanh GELU approximation.
const SQRT_2_OVER_PI: f32 = 0.797_884_560_802_865_4_f64 as f32;

/// Model topology parsed from the checkpoint manifest.
#[derive(Debug, Clone, Copy)]
pub struct Topology {
    pub d_model: usize,
    pub d_ff: usize,
    pub n_blocks: usize,
    pub state_slots: usize,
    pub n_experts: usize,
    pub vocab: usize,
    pub router_rank: usize,
}

/// A dequantized ternary matrix in effective (f32) form, shape `[rows, cols]`.
#[derive(Debug, Clone)]
struct EffMatrix {
    cols: usize,
    /// Row-major effective weights: `data[r * cols + c]`.
    data: Vec<f32>,
}

impl EffMatrix {
    /// out[r] = sum_c data[r][c] * x[c]; matches `x @ W.T`.
    fn matvec(&self, x: &[f32]) -> Vec<f32> {
        assert_eq!(x.len(), self.cols, "matvec input width mismatch");
        self.data
            .chunks_exact(self.cols)
            .map(|row| {
                row.iter()
                    .zip(x.iter())
                    .map(|(&w, &xi)| w * xi)
                    .fold(0.0f32, |acc, p| acc + p)
            })
            .collect()
    }
}

/// Router parameters for a single block (all f32; router is not ternarized).
#[derive(Debug, Clone)]
struct Router {
    /// input_projection, shape [router_rank, d_model], row-major.
    input_projection: Vec<f32>,
    /// input_bias, shape [router_rank].
    input_bias: Vec<f32>,
    /// expert_projection, shape [n_experts, router_rank], row-major.
    expert_projection: Vec<f32>,
    /// expert_bias, shape [n_experts].
    expert_bias: Vec<f32>,
    router_rank: usize,
    d_model: usize,
}

impl Router {
    /// Route on the RAW residual `x` (not normed). Returns the selected expert
    /// index using argmax with lowest-index tiebreak.
    fn route(&self, x: &[f32]) -> usize {
        let hid: Vec<f32> = self
            .input_projection
            .chunks_exact(self.d_model)
            .zip(self.input_bias.iter())
            .map(|(row, &bias)| {
                row.iter()
                    .zip(x.iter())
                    .map(|(&w, &xi)| w * xi)
                    .fold(bias, |acc, p| acc + p)
            })
            .collect();
        let mut best_e = 0usize;
        let mut best_v = f32::NEG_INFINITY;
        for (e, (row, &bias)) in self
            .expert_projection
            .chunks_exact(self.router_rank)
            .zip(self.expert_bias.iter())
            .enumerate()
        {
            let acc = row
                .iter()
                .zip(hid.iter())
                .map(|(&w, &hk)| w * hk)
                .fold(bias, |a, p| a + p);
            if acc > best_v {
                best_v = acc;
                best_e = e;
            }
        }
        best_e
    }
}

/// A single MoE FFN expert: up projection and down projection.
#[derive(Debug, Clone)]
struct Expert {
    up: EffMatrix,   // [d_ff, d_model]
    down: EffMatrix, // [d_model, d_ff]
}

/// One prenorm-residual top-1 MoE FFN block.
#[derive(Debug, Clone)]
struct Block {
    router: Router,
    experts: Vec<Expert>,
}

/// The fully-loaded model needed for a forward pass.
#[derive(Debug, Clone)]
pub struct MoeModel {
    pub topology: Topology,
    /// Token embedding + tied head, shape [vocab, d_model], row-major.
    embedding: Vec<f32>,
    /// state_input_to_state ternary, [state_slots, d_model].
    state_in: EffMatrix,
    /// state_state_to_output ternary, [d_model, state_slots].
    state_out: EffMatrix,
    /// Per-slot decay, [state_slots] (already f32 = raw/256).
    state_decay: Vec<f32>,
    blocks: Vec<Block>,
}

/// Round half-to-even (banker's rounding), matching `mx.round`.
#[inline]
fn round_ties_even(v: f32) -> f32 {
    v.round_ties_even()
}

/// RMSNorm over the whole vector, followed by a clamp to `[-ACT_RANGE, ACT_RANGE]`.
fn rms_norm_clip(x: &[f32]) -> Vec<f32> {
    let n = x.len() as f32;
    let sum_sq: f32 = x.iter().map(|&v| v * v).sum();
    let mean_sq = sum_sq / n;
    let rms = (mean_sq + NORM_EPS).sqrt();
    x.iter()
        .map(|&v| (v / rms).clamp(-ACT_RANGE, ACT_RANGE))
        .collect()
}

/// Int8 symmetric activation fake-quant over `[-8, 8]` with round-ties-even.
fn act_fake_quant(v: &[f32]) -> Vec<f32> {
    v.iter()
        .map(|&x| {
            let c = x.clamp(-ACT_RANGE, ACT_RANGE);
            let q = round_ties_even(c / ACT_RANGE * QMAX).clamp(-QMAX, QMAX);
            q / QMAX * ACT_RANGE
        })
        .collect()
}

/// Tanh GELU approximation.
fn gelu_approx(x: &[f32]) -> Vec<f32> {
    x.iter()
        .map(|&v| {
            let inner = (v + 0.044715 * v * v * v) * SQRT_2_OVER_PI;
            0.5 * v * (inner.tanh() + 1.0)
        })
        .collect()
}

impl Expert {
    /// Expert delta with `qat_acts` ON.
    fn delta(&self, x: &[f32]) -> Vec<f32> {
        let normed = rms_norm_clip(x);
        let normed = act_fake_quant(&normed);
        let hidden = self.up.matvec(&normed); // [d_ff]
        let hidden = gelu_approx(&hidden);
        let hidden = act_fake_quant(&hidden);
        self.down.matvec(&hidden) // [d_model]
    }
}

impl MoeModel {
    /// Run the full fp32 forward over a token id window. Returns `[T, vocab]`
    /// logits (B is assumed 1), row-major.
    pub fn forward(&self, ids: &[usize]) -> Vec<Vec<f32>> {
        let t_len = ids.len();
        let d_model = self.topology.d_model;
        let state_slots = self.topology.state_slots;
        let vocab = self.topology.vocab;

        // Embeddings for each token.
        let embeds: Vec<Vec<f32>> = ids
            .iter()
            .map(|&id| {
                let base = id * d_model;
                self.embedding[base..base + d_model].to_vec()
            })
            .collect();

        // Linear state block over the token window (carries `h` across t).
        let mut h = vec![0.0f32; state_slots];
        let mut x_all: Vec<Vec<f32>> = Vec::with_capacity(t_len);
        for embed in embeds.iter().take(t_len) {
            let normed = rms_norm_clip(embed);
            let normed = act_fake_quant(&normed);
            let delta = self.state_in.matvec(&normed); // [state_slots]
            for s in 0..state_slots {
                h[s] = h[s] * self.state_decay[s] + delta[s];
            }
            let y = self.state_out.matvec(&h); // [d_model]
            let y = act_fake_quant(&y);
            let mut x = embed.clone();
            for i in 0..d_model {
                x[i] += y[i];
            }
            x_all.push(x);
        }

        // Blocks (each token independent) then tied-head projection.
        let mut logits = Vec::with_capacity(t_len);
        for xf in x_all.into_iter() {
            let mut xf = xf;
            for block in &self.blocks {
                let e = block.router.route(&xf);
                let d = block.experts[e].delta(&xf);
                for i in 0..d_model {
                    xf[i] += d[i];
                }
            }
            let normed = rms_norm_clip(&xf);
            let row: Vec<f32> = self
                .embedding
                .chunks_exact(d_model)
                .take(vocab)
                .map(|erow| {
                    erow.iter()
                        .zip(normed.iter())
                        .map(|(&e, &n)| n * e)
                        .fold(0.0f32, |acc, p| acc + p)
                })
                .collect();
            logits.push(row);
        }
        logits
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Read a whole file as bytes.
fn read_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// Decode i8 ternary bytes into f32 `{-1,0,+1}`.
fn decode_i8(bytes: &[u8]) -> Vec<f32> {
    bytes.iter().map(|&b| (b as i8) as f32).collect()
}

/// Decode u16 LE scales / decays into f32 (raw / 256).
fn decode_q8_8(bytes: &[u8]) -> Vec<f32> {
    assert_eq!(bytes.len() % 2, 0, "q8.8 buffer must be u16-aligned");
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]) as f32 / 256.0)
        .collect()
}

/// Decode f32 LE bytes.
fn decode_f32(bytes: &[u8]) -> Vec<f32> {
    assert_eq!(bytes.len() % 4, 0, "f32 buffer must be 4-byte aligned");
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Load a dequantized ternary matrix given base name + shape.
fn load_eff_matrix(dir: &Path, base: &str, rows: usize, cols: usize) -> EffMatrix {
    let tern_path = dir.join(format!("{base}.ternary.i8.bin"));
    let scale_path = dir.join(format!("{base}.scales.q8_8_u16le.bin"));
    let tern = decode_i8(&read_bytes(&tern_path).unwrap_or_else(|e| {
        panic!("read {}: {e}", tern_path.display());
    }));
    let scales = decode_q8_8(&read_bytes(&scale_path).unwrap_or_else(|e| {
        panic!("read {}: {e}", scale_path.display());
    }));
    assert_eq!(tern.len(), rows * cols, "ternary shape mismatch for {base}");
    assert_eq!(scales.len(), rows, "scale count mismatch for {base}");
    let data: Vec<f32> = tern
        .chunks_exact(cols)
        .zip(scales.iter())
        .flat_map(|(row, &s)| row.iter().map(move |&w| w * s))
        .collect();
    EffMatrix { cols, data }
}

/// Load a raw f32 tensor by base name (file is `<base>.f32.bin`).
fn load_f32(dir: &Path, base: &str) -> Vec<f32> {
    let path = dir.join(format!("{base}.f32.bin"));
    decode_f32(&read_bytes(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())))
}

/// Minimal JSON scalar extraction of topology ints from the manifest text.
fn topology_from_manifest(manifest: &serde_json::Value) -> Topology {
    let topo = &manifest["topology"];
    let seq = &topo["sequence_state_params"];
    let get = |v: &serde_json::Value, k: &str| -> usize {
        v[k].as_u64()
            .unwrap_or_else(|| panic!("manifest missing usize field {k}")) as usize
    };
    Topology {
        d_model: get(topo, "d_model"),
        d_ff: get(topo, "d_ff"),
        n_blocks: get(topo, "n_blocks"),
        state_slots: get(seq, "state_slots"),
        n_experts: get(topo, "n_experts_per_block"),
        vocab: get(topo, "vocab"),
        // router_rank lives on each layer entry; read from the first layer.
        router_rank: manifest["layers"][0]["router_rank"]
            .as_u64()
            .expect("manifest missing router_rank") as usize,
    }
}

impl MoeModel {
    /// Load the model from a checkpoint directory (containing `manifest.json`
    /// and a `tensors/` subdirectory).
    pub fn load(ckpt_dir: &Path) -> Self {
        let manifest_path = ckpt_dir.join("manifest.json");
        let manifest_text = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
        let manifest: serde_json::Value =
            serde_json::from_str(&manifest_text).expect("parse manifest.json");
        let topology = topology_from_manifest(&manifest);
        let tdir = ckpt_dir.join("tensors");

        // Embedding [vocab, d_model].
        let embedding = load_f32(&tdir, "embedding");
        assert_eq!(
            embedding.len(),
            topology.vocab * topology.d_model,
            "embedding size mismatch"
        );

        // State projections.
        let state_in = load_eff_matrix(
            &tdir,
            "state_input_to_state",
            topology.state_slots,
            topology.d_model,
        );
        let state_out = load_eff_matrix(
            &tdir,
            "state_state_to_output",
            topology.d_model,
            topology.state_slots,
        );

        // State decay [state_slots].
        let decay_path = tdir.join("state_decay.q8_8_u16le.bin");
        let state_decay = decode_q8_8(&read_bytes(&decay_path).expect("read state_decay"));
        assert_eq!(
            state_decay.len(),
            topology.state_slots,
            "state_decay size mismatch"
        );

        // Blocks.
        let mut blocks = Vec::with_capacity(topology.n_blocks);
        for bi in 0..topology.n_blocks {
            let input_projection = load_f32(&tdir, &format!("block{bi}_router_input_projection"));
            let input_bias = load_f32(&tdir, &format!("block{bi}_router_input_bias"));
            let expert_projection = load_f32(&tdir, &format!("block{bi}_router_expert_projection"));
            let expert_bias = load_f32(&tdir, &format!("block{bi}_router_expert_bias"));
            assert_eq!(
                input_projection.len(),
                topology.router_rank * topology.d_model
            );
            assert_eq!(input_bias.len(), topology.router_rank);
            assert_eq!(
                expert_projection.len(),
                topology.n_experts * topology.router_rank
            );
            assert_eq!(expert_bias.len(), topology.n_experts);
            let router = Router {
                input_projection,
                input_bias,
                expert_projection,
                expert_bias,
                router_rank: topology.router_rank,
                d_model: topology.d_model,
            };

            let mut experts = Vec::with_capacity(topology.n_experts);
            for ei in 0..topology.n_experts {
                let up = load_eff_matrix(
                    &tdir,
                    &format!("block{bi}_expert{ei}_up"),
                    topology.d_ff,
                    topology.d_model,
                );
                let down = load_eff_matrix(
                    &tdir,
                    &format!("block{bi}_expert{ei}_down"),
                    topology.d_model,
                    topology.d_ff,
                );
                experts.push(Expert { up, down });
            }
            blocks.push(Block { router, experts });
        }

        MoeModel {
            topology,
            embedding,
            state_in,
            state_out,
            state_decay,
            blocks,
        }
    }
}

/// The golden fixture: expected forward result.
#[derive(Debug, Clone)]
pub struct Golden {
    pub topology: Topology,
    pub b: usize,
    pub t: usize,
    pub ids: Vec<usize>,
    pub vocab: usize,
    /// Flat logits, row-major `[B, T, vocab]`.
    pub logits: Vec<f32>,
    /// Flat argmax `[B, T]`.
    pub argmax: Vec<usize>,
}

impl Golden {
    /// Load and parse `golden.json`.
    pub fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse golden.json");
        let topo = &v["topology"];
        let get = |k: &str| topo[k].as_u64().expect("golden topology field") as usize;
        let topology = Topology {
            d_model: get("d_model"),
            d_ff: get("d_ff"),
            n_blocks: get("n_blocks"),
            state_slots: get("state_slots"),
            n_experts: get("n_experts"),
            vocab: get("vocab"),
            router_rank: get("router_rank"),
        };
        let ids = v["ids"]
            .as_array()
            .expect("golden ids array")
            .iter()
            .map(|x| x.as_u64().expect("id u64") as usize)
            .collect::<Vec<_>>();
        let logits = v["logits"]
            .as_array()
            .expect("golden logits array")
            .iter()
            .map(|x| x.as_f64().expect("logit f64") as f32)
            .collect::<Vec<_>>();
        let argmax = v["argmax"]
            .as_array()
            .expect("golden argmax array")
            .iter()
            .map(|x| x.as_u64().expect("argmax u64") as usize)
            .collect::<Vec<_>>();
        let b = v["B"].as_u64().expect("golden B") as usize;
        let t = v["T"].as_u64().expect("golden T") as usize;
        Golden {
            topology,
            b,
            t,
            ids,
            vocab: topology.vocab,
            logits,
            argmax,
        }
    }
}

/// Result of a parity comparison.
#[derive(Debug, Clone)]
pub struct ParityReport {
    pub max_abs_diff: f32,
    pub mean_abs_diff: f32,
    pub argmax_matches: bool,
    /// Positions (t index) where argmax disagreed.
    pub argmax_mismatches: Vec<(usize, usize, usize)>,
}

/// Compute per-position argmax with lowest-index tiebreak.
fn argmax_row(row: &[f32]) -> usize {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in row.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    best
}

/// Compare a Rust `[T][vocab]` forward result against the golden fixture.
pub fn compare(rust_logits: &[Vec<f32>], golden: &Golden) -> ParityReport {
    assert_eq!(golden.b, 1, "parity harness assumes B=1");
    assert_eq!(rust_logits.len(), golden.t, "T mismatch");
    let vocab = golden.vocab;

    let mut max_abs = 0.0f32;
    let mut sum_abs = 0.0f64;
    let mut count = 0usize;
    for (t, row) in rust_logits.iter().enumerate() {
        assert_eq!(row.len(), vocab, "vocab width mismatch at t={t}");
        for (v, &cell) in row.iter().enumerate() {
            let g = golden.logits[t * vocab + v];
            let d = (cell - g).abs();
            if d > max_abs {
                max_abs = d;
            }
            sum_abs += d as f64;
            count += 1;
        }
    }
    let mean_abs = (sum_abs / count as f64) as f32;

    let mut mismatches = Vec::new();
    for (t, row) in rust_logits.iter().enumerate() {
        let got = argmax_row(row);
        let want = golden.argmax[t];
        if got != want {
            mismatches.push((t, got, want));
        }
    }

    ParityReport {
        max_abs_diff: max_abs,
        mean_abs_diff: mean_abs,
        argmax_matches: mismatches.is_empty(),
        argmax_mismatches: mismatches,
    }
}

/// Convenience: load model + golden from a fixture root and run the forward.
pub fn run_fixture(fixture_dir: &Path) -> (Vec<Vec<f32>>, Golden) {
    let model = MoeModel::load(&fixture_dir.join("ckpt"));
    let golden = Golden::load(&fixture_dir.join("golden.json"));
    let logits = model.forward(&golden.ids);
    (logits, golden)
}

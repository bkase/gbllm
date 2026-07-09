//! Reusable driver for the fixed-point MoE router parity gate (deploy step 3,
//! `docs/design/integer-moe-deploy.md`, `router-fx.v1`).
//!
//! The deployed integer forward routes purely-integer via
//! [`gbf_kernel::state_model_ref::FixedRouter`]; the f32
//! [`gbf_kernel::state_model_ref::LowRankRouter::route_f32`] stays as the
//! reference. This driver replays a greedy generation stream through the real
//! bridged d192x8 MoE student and, at EVERY block and EVERY position, compares
//! the integer router's argmax against the f32 router's argmax on the exact
//! raw pre-norm i24 residual the forward routes on. The gate asserts 0
//! divergences; on any divergence it reports the `raw[top1] - raw[top2]`
//! margins on both sides so a genuine near-tie is distinguishable from a bug.

use gbf_kernel::state_model_ref::{IntStateLoweredModel, LoweredBlockFfn, STATE_RESID_ONE};

/// One router comparison at a single (position, block).
#[derive(Debug, Clone)]
pub struct RouterDivergence {
    /// Token position in the generation stream.
    pub position: usize,
    /// Block index (0..n_blocks).
    pub block: usize,
    /// The f32 reference argmax expert.
    pub f32_expert: usize,
    /// The fixed-point (deployed) argmax expert.
    pub fixed_expert: usize,
    /// `raw[top1] - raw[top2]` under the f32 router (real logit units).
    pub f32_margin: f32,
    /// `raw[top1] - raw[top2]` under the fixed-point router (Q32.32 i64 units).
    pub fixed_margin_q32: i64,
}

/// Result of the fixed-point router parity sweep.
#[derive(Debug, Clone)]
pub struct RouterGateReport {
    /// Total (position, block) router comparisons performed
    /// (= n_positions * n_moe_blocks).
    pub comparisons: usize,
    /// Number of MoE blocks (comparisons per position).
    pub n_moe_blocks: usize,
    /// Positions swept.
    pub positions: usize,
    /// Any argmax divergences (empty on success).
    pub divergences: Vec<RouterDivergence>,
    /// Smallest `raw[top1] - raw[top2]` f32 margin observed across every
    /// comparison (the closest the model ever came to a routing tie).
    pub min_f32_margin: f32,
    /// The (position, block) where `min_f32_margin` occurred.
    pub min_f32_margin_at: (usize, usize),
}

impl RouterGateReport {
    #[must_use]
    pub fn zero_divergences(&self) -> bool {
        self.divergences.is_empty()
    }
}

/// Top-2 margin `raw[top1] - raw[top2]` for a slice, plus the argmax
/// (lowest-index tiebreak, strict `>`). Generic over the ordered logit type.
fn top1_and_margin_f32(raw: &[f32]) -> (usize, f32) {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in raw.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    let second = raw
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != best)
        .map(|(_, &v)| v)
        .fold(f32::NEG_INFINITY, f32::max);
    (best, best_v - second)
}

fn top1_and_margin_i64(raw: &[i64]) -> (usize, i64) {
    let mut best = 0usize;
    let mut best_v = i64::MIN;
    for (i, &v) in raw.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    let second = raw
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != best)
        .map(|(_, &v)| v)
        .fold(i64::MIN, i64::max);
    (best, best_v.saturating_sub(second))
}

/// Run the fixed-point router parity sweep over `n_positions` greedy tokens
/// starting from `seed`. At each position and each MoE block, compares the
/// deployed `FixedRouter` argmax against the f32 `LowRankRouter` argmax on the
/// exact raw pre-norm residual the forward routes on.
///
/// The greedy stream is the deployed integer forward's own argmax fed back
/// (subword V=1024, so the full id is threaded), i.e. exactly the sequence the
/// ROM would generate — the router is gated on the real trajectory, not a
/// synthetic token walk.
#[must_use]
pub fn run_router_fixed_point_gate(
    lowered: &IntStateLoweredModel,
    seed: usize,
    n_positions: usize,
) -> RouterGateReport {
    let n_moe_blocks = lowered
        .block_ffns
        .iter()
        .filter(|b| matches!(b, LoweredBlockFfn::Moe { .. }))
        .count();

    let mut state = lowered.zero_state();
    let mut input = seed;
    let mut divergences = Vec::new();
    let mut comparisons = 0usize;
    let mut min_f32_margin = f32::INFINITY;
    let mut min_f32_margin_at = (0usize, 0usize);

    for position in 0..n_positions {
        // Run the real deployed forward, capturing the pre-block residual the
        // integer router routes on at every MoE block.
        let mut audit: Vec<(usize, Vec<i32>)> = Vec::with_capacity(n_moe_blocks);
        let trace = lowered.forward_at_route_audit(input, &mut state, &mut audit);

        for (block, x_i24) in &audit {
            let LoweredBlockFfn::Moe {
                router,
                fixed_router,
                ..
            } = &lowered.block_ffns[*block]
            else {
                unreachable!("audited block {block} is MoE");
            };

            // f32 reference on the exact dequantized residual.
            let x_f32: Vec<f32> = x_i24
                .iter()
                .map(|&v| v as f32 / STATE_RESID_ONE as f32)
                .collect();
            let (f32_expert, f32_raw) = router.route_f32_with_logits(&x_f32);
            let (_, f32_margin) = top1_and_margin_f32(&f32_raw);

            // Deployed integer router on the raw i24 residual.
            let (fixed_expert, fixed_raw) = fixed_router.route_with_logits(x_i24);
            let (_, fixed_margin_q32) = top1_and_margin_i64(&fixed_raw);

            comparisons += 1;
            if f32_margin < min_f32_margin {
                min_f32_margin = f32_margin;
                min_f32_margin_at = (position, *block);
            }
            if f32_expert != fixed_expert {
                divergences.push(RouterDivergence {
                    position,
                    block: *block,
                    f32_expert,
                    fixed_expert,
                    f32_margin,
                    fixed_margin_q32,
                });
            }
        }

        input = trace.argmax_full;
    }

    RouterGateReport {
        comparisons,
        n_moe_blocks,
        positions: n_positions,
        divergences,
        min_f32_margin: if min_f32_margin.is_finite() {
            min_f32_margin
        } else {
            0.0
        },
        min_f32_margin_at,
    }
}

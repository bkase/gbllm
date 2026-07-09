//! Cross-language parity test: Rust f32 MoE forward vs MLX golden (bd-2lk86).

use std::path::PathBuf;

use gbf_bench::moe_parity::{compare, run_fixture};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("moe_parity")
}

#[test]
fn rust_forward_matches_mlx_golden() {
    let dir = fixture_dir();
    let (rust_logits, golden) = run_fixture(&dir);

    assert_eq!(golden.b, 1, "fixture assumes B=1");
    assert_eq!(rust_logits.len(), golden.t, "T mismatch");

    let report = compare(&rust_logits, &golden);

    println!(
        "moe_parity: T={} vocab={} max_abs_diff={:.6e} mean_abs_diff={:.6e} argmax_matches={}",
        golden.t, golden.vocab, report.max_abs_diff, report.mean_abs_diff, report.argmax_matches
    );
    for (t, got, want) in &report.argmax_mismatches {
        println!("  argmax mismatch at t={t}: rust={got} golden={want}");
    }

    // Primary, deployment-relevant gate: argmax must match exactly.
    assert!(
        report.argmax_matches,
        "argmax mismatch vs golden: {:?}",
        report.argmax_mismatches
    );

    // Numeric gate. We try hard for 1e-3; allow up to 5e-3 for accumulated
    // libm tanh/sqrt last-ULP drift across two blocks.
    assert!(
        report.max_abs_diff <= 5e-3,
        "max_abs_diff {:.6e} exceeds 5e-3 tolerance",
        report.max_abs_diff
    );
}

/// Real-geometry parity on an EXTERNAL fixture dir (set `MOE_PARITY_DIR` to a
/// directory containing `ckpt/` + `golden.json`, produced by
/// `training/run_realparity.py`). Ignored by default so CI runs only the
/// committed tiny fixture; used to cross-check the full d192/V=1024/8-expert
/// student post-training with one command:
///   MOE_PARITY_DIR=/path/to/scratch cargo test -p gbf-bench --test moe_parity \
///     -- --ignored --nocapture rust_forward_matches_mlx_golden_external
///
/// TOLERANCE RATIONALE. At the tiny fixture scale this Rust forward matches MLX
/// to ~1e-6. At real scale (d192, d_ff384, 6 blocks) a RARE per-token
/// divergence appears: MLX's Metal/Accelerate `tanh`/reductions land on the
/// opposite side of an `act_fake_quant` rounding boundary from portable CPU
/// math for the occasional boundary-adjacent activation, injecting one discrete
/// 8/127 (~0.063) step into that token's stream. This is NOT a bug -- this Rust
/// forward agrees with a portable numpy f32 forward to ~1e-6; both differ from
/// MLX only on those boundary tokens (verified: numpy and Rust both diverge by
/// the identical 0.2366 on the identical single token). The effect is inherent
/// fp cross-implementation nondeterminism at a quantizer boundary and is ABSENT
/// in the exact-integer LUT-gelu ROM (the real deploy target; its definitive
/// gate is the integer evaluator vs MLX, per docs/design/integer-moe-deploy.md).
/// So the gate here is: argmax exact (the deployment-relevant invariant) + a
/// robust mean bound + a cap on how many tokens may carry a boundary flip.
#[test]
#[ignore = "requires MOE_PARITY_DIR pointing at a real-geometry fixture"]
fn rust_forward_matches_mlx_golden_external() {
    let dir = std::env::var("MOE_PARITY_DIR")
        .expect("set MOE_PARITY_DIR to a dir with ckpt/ + golden.json");
    let dir = PathBuf::from(dir);
    let (rust_logits, golden) = run_fixture(&dir);
    let report = compare(&rust_logits, &golden);

    // Count tokens carrying a boundary flip (max per-token logit diff > 0.02;
    // benign flips land ~0.06+, exact tokens land ~1e-6).
    let vocab = golden.vocab;
    let mut flip_tokens = 0usize;
    for (t, row) in rust_logits.iter().enumerate() {
        let tmax = row
            .iter()
            .enumerate()
            .map(|(v, &c)| (c - golden.logits[t * vocab + v]).abs())
            .fold(0.0f32, f32::max);
        if tmax > 0.02 {
            flip_tokens += 1;
        }
    }
    println!(
        "moe_parity[external]: T={} vocab={} max_abs_diff={:.6e} mean_abs_diff={:.6e} argmax_matches={} flip_tokens={}/{}",
        golden.t,
        vocab,
        report.max_abs_diff,
        report.mean_abs_diff,
        report.argmax_matches,
        flip_tokens,
        golden.t
    );
    for (t, got, want) in &report.argmax_mismatches {
        println!("  argmax mismatch at t={t}: rust={got} golden={want}");
    }

    // Deployment-relevant invariant: same next-token everywhere.
    assert!(
        report.argmax_matches,
        "argmax mismatch vs golden: {:?}",
        report.argmax_mismatches
    );
    // Mean is robust to rare per-token boundary flips.
    assert!(
        report.mean_abs_diff <= 1e-2,
        "mean_abs_diff {:.6e} exceeds 1e-2",
        report.mean_abs_diff
    );
    // Boundary flips must stay rare (else it is a real port bug, not fp noise).
    assert!(
        flip_tokens * 4 <= golden.t.max(1),
        "too many boundary-flip tokens: {flip_tokens}/{} (>25%) -- likely a real bug, not fp noise",
        golden.t
    );
}

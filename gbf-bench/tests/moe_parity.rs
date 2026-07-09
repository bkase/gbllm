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

//! Full kernel bake-off sweep: every variant x sparsity must run in the
//! emulator and agree byte-exactly with the host reference (bd-rzq5n).

use gbf_bench::kernel_bakeoff::{KernelVariant, run_kernel_bakeoff};

#[test]
fn full_sweep_conforms_and_orders_strategies() {
    let report = run_kernel_bakeoff().expect("all kernels verify against the reference");

    // 3 variants x 4 sparsities, all conformant by construction.
    assert_eq!(report.runs.len(), 12);

    // At the projection sparsity, the strategies must hold their designed
    // ordering: weights-as-code < threaded dispatch < interpreted.
    let per_mac = |variant: KernelVariant| {
        report
            .runs
            .iter()
            .find(|run| run.variant == variant && run.zero_permille == 400)
            .expect("sweep includes 400 permille")
            .m_cycles_per_mac_x1000
    };
    let v1 = per_mac(KernelVariant::V1Interpreted);
    let v2 = per_mac(KernelVariant::V2Dispatch);
    let v3 = per_mac(KernelVariant::V3WeightsAsCode);
    assert!(
        v3 < v2 && v2 < v1,
        "expected v3 < v2 < v1, got {v3} {v2} {v1}"
    );

    // Report must cover every projection profile for every variant.
    assert_eq!(report.projections.len(), 7);
    for projection in &report.projections {
        assert_eq!(projection.per_variant.len(), 3);
        for entry in &projection.per_variant {
            assert!(entry.m_cycles_per_token > 0);
        }
    }
}

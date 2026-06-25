use gbf_train::runtime::collapse_halt::{
    CollapseHaltConfig, CollapseHaltDecision, CollapseHaltMonitor,
};

const PHASE_B_START: u64 = 2_000;
const N_EXPERTS: usize = 4;
const LOW_LAYER_ENTROPY_BITS: &[f32] = &[0.20, 0.25];
const HEALTHY_LAYER_ENTROPY_BITS: &[f32] = &[1.70, 1.80];

fn monitor() -> CollapseHaltMonitor {
    CollapseHaltMonitor::new(CollapseHaltConfig::new(PHASE_B_START, N_EXPERTS).unwrap())
}

#[test]
fn single_low_dip_with_healthy_window_does_not_halt() {
    let mut monitor = monitor();
    let config = monitor.config();
    let first_checked_step = config.first_checked_step().unwrap();

    for offset in 0..100 {
        let layer_entropy_bits = if offset == 42 {
            LOW_LAYER_ENTROPY_BITS
        } else {
            HEALTHY_LAYER_ENTROPY_BITS
        };
        let observation = monitor
            .observe_step(first_checked_step + offset, layer_entropy_bits)
            .unwrap();
        assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    }

    let rolling_mean_bits = monitor
        .observe_step(first_checked_step + 100, HEALTHY_LAYER_ENTROPY_BITS)
        .unwrap()
        .rolling_mean_bits()
        .unwrap();
    assert!(
        rolling_mean_bits > config.entropy_floor_bits(),
        "single low dip must not pull the rolling window below the floor"
    );
}

#[test]
fn entropy_for_step_is_min_over_layers() {
    let mut monitor = monitor();
    let first_checked_step = monitor.config().first_checked_step().unwrap();

    let observation = monitor
        .observe_step(first_checked_step, &[1.75, 0.40, 1.25])
        .unwrap();

    assert_eq!(observation.entropy_for_step_bits(), 0.40);
    assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
}

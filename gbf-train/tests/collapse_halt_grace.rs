use gbf_train::runtime::collapse_halt::{
    CollapseHaltConfig, CollapseHaltDecision, CollapseHaltMonitor, ROUTER_COLLAPSE_GRACE_STEPS,
};

const PHASE_B_START: u64 = 1_000;
const N_EXPERTS: usize = 4;
const LOW_LAYER_ENTROPY_BITS: &[f32] = &[0.25, 0.30];
const HEALTHY_LAYER_ENTROPY_BITS: &[f32] = &[1.70, 1.80];

fn monitor() -> CollapseHaltMonitor {
    CollapseHaltMonitor::new(CollapseHaltConfig::new(PHASE_B_START, N_EXPERTS).unwrap())
}

#[test]
fn single_low_step_inside_grace_does_not_halt() {
    let mut monitor = monitor();
    let low_grace_step = PHASE_B_START + 17;

    let observation = monitor
        .observe_step(low_grace_step, LOW_LAYER_ENTROPY_BITS)
        .unwrap();

    assert_eq!(observation.step(), low_grace_step);
    assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    assert_eq!(observation.rolling_mean_bits(), None);
    assert_eq!(observation.entropy_for_step_bits(), 0.25);
}

#[test]
fn sustained_100_step_collapse_after_grace_halts_with_collapsed_at() {
    let mut monitor = monitor();
    let config = monitor.config();
    let first_checked_step = config.first_checked_step().unwrap();
    assert_eq!(
        first_checked_step,
        PHASE_B_START + ROUTER_COLLAPSE_GRACE_STEPS
    );
    assert_eq!(config.entropy_floor_bits(), 1.0);

    for offset in 0..99 {
        let observation = monitor
            .observe_step(first_checked_step + offset, LOW_LAYER_ENTROPY_BITS)
            .unwrap();
        assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    }

    let collapsed_step = first_checked_step + 99;
    let observation = monitor
        .observe_step(collapsed_step, LOW_LAYER_ENTROPY_BITS)
        .unwrap();

    assert_eq!(
        observation.decision(),
        CollapseHaltDecision::CollapsedAt(collapsed_step)
    );
    assert!(
        observation.rolling_mean_bits().unwrap() < config.entropy_floor_bits(),
        "rolling mean should be below the D16 entropy floor"
    );
}

#[test]
fn phase_a_entropy_is_reported_but_not_asserted_for_halt() {
    let mut monitor = monitor();
    let phase_a_step = PHASE_B_START - 1;

    let observation = monitor
        .observe_step(phase_a_step, LOW_LAYER_ENTROPY_BITS)
        .unwrap();

    assert_eq!(observation.step(), phase_a_step);
    assert_eq!(observation.entropy_for_step_bits(), 0.25);
    assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    assert_eq!(observation.rolling_mean_bits(), None);

    let first_checked_step = monitor.config().first_checked_step().unwrap();
    for offset in 0..100 {
        let observation = monitor
            .observe_step(first_checked_step + offset, HEALTHY_LAYER_ENTROPY_BITS)
            .unwrap();
        assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    }
}

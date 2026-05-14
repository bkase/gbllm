use gbf_train::phase::TrainPhaseKind;
use proptest::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseEntry {
    pub from: Option<TrainPhaseKind>,
    pub to: TrainPhaseKind,
    pub step: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseEvent {
    pub name: String,
    pub phase: TrainPhaseKind,
    pub step: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LossTermEvalPoint {
    pub term: String,
    pub raw_loss: f32,
    pub weight: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HardnessTriple {
    pub expert_qat: f32,
    pub activation_qat: f32,
    pub norm_qat: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhaseEffectiveLambda {
    pub phase: TrainPhaseKind,
    pub name: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S2Outcome {
    Pass,
    FailGap,
    PassWithDistillWarn,
    Refuted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSubcheck {
    pub name: String,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureResult {
    pub fixture_name: String,
    pub passed: bool,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrainConfigS2 {
    pub seed: u64,
    pub optimizer_steps: u64,
    pub lambda_distill: f32,
    pub lambda_zero: f32,
}

pub fn arb_phase_entry() -> impl Strategy<Value = PhaseEntry> {
    (
        prop::option::of(arb_phase_kind()),
        arb_phase_kind(),
        0_u64..=10_000,
    )
        .prop_map(|(from, to, step)| PhaseEntry { from, to, step })
}

pub fn arb_phase_event() -> impl Strategy<Value = PhaseEvent> {
    ("[a-z_]{1,32}", arb_phase_kind(), 0_u64..=10_000).prop_map(|(name, phase, step)| PhaseEvent {
        name,
        phase,
        step,
    })
}

pub fn arb_loss_term_eval_point() -> impl Strategy<Value = LossTermEvalPoint> {
    ("[a-z_]{1,32}", finite_nonnegative_f32(), 0.0_f32..=4.0).prop_map(
        |(term, raw_loss, weight)| LossTermEvalPoint {
            term,
            raw_loss,
            weight,
        },
    )
}

pub fn arb_hardness_triple() -> impl Strategy<Value = HardnessTriple> {
    (0.0_f32..=1.0, 0.0_f32..=1.0, 0.0_f32..=1.0).prop_map(
        |(expert_qat, activation_qat, norm_qat)| HardnessTriple {
            expert_qat,
            activation_qat,
            norm_qat,
        },
    )
}

pub fn arb_phase_effective_lambda() -> impl Strategy<Value = PhaseEffectiveLambda> {
    (arb_phase_kind(), "[a-z_]{1,32}", 0.0_f32..=8.0)
        .prop_map(|(phase, name, value)| PhaseEffectiveLambda { phase, name, value })
}

pub fn arb_s2_outcome() -> impl Strategy<Value = S2Outcome> {
    prop_oneof![
        Just(S2Outcome::Pass),
        Just(S2Outcome::FailGap),
        Just(S2Outcome::PassWithDistillWarn),
        Just(S2Outcome::Refuted),
    ]
}

pub fn arb_diagnostic_subcheck() -> impl Strategy<Value = DiagnosticSubcheck> {
    ("[a-z_]{1,32}", any::<bool>()).prop_map(|(name, passed)| DiagnosticSubcheck { name, passed })
}

pub fn arb_fixture_result() -> impl Strategy<Value = FixtureResult> {
    (
        "[a-z0-9_-]{1,32}",
        any::<bool>(),
        arb_seed_in_range(0..=u64::MAX),
    )
        .prop_map(|(fixture_name, passed, seed)| FixtureResult {
            fixture_name,
            passed,
            seed,
        })
}

pub fn arb_seed_in_range(range: std::ops::RangeInclusive<u64>) -> impl Strategy<Value = u64> {
    range
}

pub fn arb_canonical_tensor_set() -> BoxedStrategy<Vec<crate::common::assertions::CanonicalTensor>>
{
    crate::common::strategies::arb_canonical_tensor_set()
}

pub fn arb_train_config_s2() -> impl Strategy<Value = TrainConfigS2> {
    (
        0_u64..=u64::MAX,
        1_u64..=20_000,
        0.0_f32..=4.0,
        0.0_f32..=2.0,
    )
        .prop_map(
            |(seed, optimizer_steps, lambda_distill, lambda_zero)| TrainConfigS2 {
                seed,
                optimizer_steps,
                lambda_distill,
                lambda_zero,
            },
        )
}

fn arb_phase_kind() -> impl Strategy<Value = TrainPhaseKind> {
    prop_oneof![
        Just(TrainPhaseKind::DenseTeacherWarmup),
        Just(TrainPhaseKind::RouterWarmup),
        Just(TrainPhaseKind::ExpertTernaryQat),
        Just(TrainPhaseKind::FullNumericQat),
        Just(TrainPhaseKind::HardenAndSelect),
    ]
}

fn finite_nonnegative_f32() -> impl Strategy<Value = f32> {
    0.0_f32..=1024.0
}

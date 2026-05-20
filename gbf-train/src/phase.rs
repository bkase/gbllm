//! Explicit phased-training schedule contracts.
//!
//! This module owns the config schema and validation for the canonical five
//! F4 training phases. Router phase behavior stays on `RouterTrainMode`
//! because router transitions control dispatch selection rather than numeric
//! quantization hardness.

use std::error::Error;
use std::fmt;

pub use gbf_model::qat::QuantHardness;
use gbf_model::qat::RouterTrainMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainPhaseKind {
    DenseTeacherWarmup,
    RouterWarmup,
    ExpertTernaryQat,
    FullNumericQat,
    HardenAndSelect,
}

impl TrainPhaseKind {
    #[must_use]
    pub const fn canonical_order() -> [Self; TRAIN_PHASE_COUNT] {
        [
            Self::DenseTeacherWarmup,
            Self::RouterWarmup,
            Self::ExpertTernaryQat,
            Self::FullNumericQat,
            Self::HardenAndSelect,
        ]
    }
}

impl fmt::Display for TrainPhaseKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DenseTeacherWarmup => f.write_str("dense_teacher_warmup"),
            Self::RouterWarmup => f.write_str("router_warmup"),
            Self::ExpertTernaryQat => f.write_str("expert_ternary_qat"),
            Self::FullNumericQat => f.write_str("full_numeric_qat"),
            Self::HardenAndSelect => f.write_str("harden_and_select"),
        }
    }
}

pub const TRAIN_PHASE_COUNT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct TrainPhaseSpec {
    kind: TrainPhaseKind,
    start_step: u64,
    end_step: u64,
    expert_qat: QuantHardness,
    activation_qat: QuantHardness,
    norm_qat: QuantHardness,
    router_mode: RouterTrainMode,
}

impl TrainPhaseSpec {
    pub fn new(
        kind: TrainPhaseKind,
        start_step: u64,
        end_step: u64,
        expert_qat: QuantHardness,
        activation_qat: QuantHardness,
        norm_qat: QuantHardness,
        router_mode: RouterTrainMode,
    ) -> Result<Self, PhaseScheduleError> {
        if start_step >= end_step {
            return Err(PhaseScheduleError::InvalidPhaseRange {
                kind,
                start_step,
                end_step,
            });
        }

        Ok(Self {
            kind,
            start_step,
            end_step,
            expert_qat,
            activation_qat,
            norm_qat,
            router_mode,
        })
    }

    #[must_use]
    pub const fn kind(self) -> TrainPhaseKind {
        self.kind
    }

    #[must_use]
    pub const fn start_step(self) -> u64 {
        self.start_step
    }

    #[must_use]
    pub const fn end_step(self) -> u64 {
        self.end_step
    }

    #[must_use]
    pub const fn len_steps(self) -> u64 {
        self.end_step - self.start_step
    }

    #[must_use]
    pub const fn expert_qat(self) -> QuantHardness {
        self.expert_qat
    }

    #[must_use]
    pub const fn activation_qat(self) -> QuantHardness {
        self.activation_qat
    }

    #[must_use]
    pub const fn norm_qat(self) -> QuantHardness {
        self.norm_qat
    }

    #[must_use]
    pub const fn router_mode(self) -> RouterTrainMode {
        self.router_mode
    }
}

impl<'de> Deserialize<'de> for TrainPhaseSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawTrainPhaseSpec::deserialize(deserializer)?;
        Self::new(
            raw.kind,
            raw.start_step,
            raw.end_step,
            raw.expert_qat,
            raw.activation_qat,
            raw.norm_qat,
            raw.router_mode,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrainingPhaseSchedule {
    phases: Vec<TrainPhaseSpec>,
}

impl TrainingPhaseSchedule {
    pub fn new(phases: Vec<TrainPhaseSpec>) -> Result<Self, PhaseScheduleError> {
        validate_phases(&phases)?;
        Ok(Self { phases })
    }

    pub fn default_five_phase(steps_per_phase: u64) -> Result<Self, PhaseScheduleError> {
        let mut start_step: u64 = 0;
        let mut phases = Vec::with_capacity(TRAIN_PHASE_COUNT);

        for template in default_phase_templates() {
            let end_step = start_step.checked_add(steps_per_phase).ok_or(
                PhaseScheduleError::StepOverflow {
                    start_step,
                    added_steps: steps_per_phase,
                },
            )?;
            phases.push(TrainPhaseSpec::new(
                template.kind,
                start_step,
                end_step,
                template.expert_qat,
                template.activation_qat,
                template.norm_qat,
                template.router_mode,
            )?);
            start_step = end_step;
        }

        Self::new(phases)
    }

    /// F-S5 Pick-and-Fit phase ladder.
    ///
    /// The RFC numbers optimizer steps as 1-based ranges. This scheduler uses
    /// half-open ranges and retains step 0 for the existing restart/fixture
    /// convention, so Phase A covers `[0, 6001)` and Phase C begins at the
    /// unambiguous global step 6002.
    pub fn s5_pick_and_fit() -> Result<Self, PhaseScheduleError> {
        Self::new(vec![
            TrainPhaseSpec::new(
                TrainPhaseKind::DenseTeacherWarmup,
                0,
                6001,
                QuantHardness::Off,
                QuantHardness::Off,
                QuantHardness::Off,
                RouterTrainMode::SoftTop1,
            )?,
            TrainPhaseSpec::new(
                TrainPhaseKind::RouterWarmup,
                6001,
                6002,
                QuantHardness::Off,
                QuantHardness::Off,
                QuantHardness::Off,
                RouterTrainMode::SoftTop1,
            )?,
            TrainPhaseSpec::new(
                TrainPhaseKind::ExpertTernaryQat,
                6002,
                12001,
                QuantHardness::Hard,
                QuantHardness::Soft,
                QuantHardness::Soft,
                RouterTrainMode::SoftTop1,
            )?,
            TrainPhaseSpec::new(
                TrainPhaseKind::FullNumericQat,
                12001,
                20001,
                QuantHardness::Hard,
                QuantHardness::Hard,
                QuantHardness::Hard,
                RouterTrainMode::HardTop1,
            )?,
            TrainPhaseSpec::new(
                TrainPhaseKind::HardenAndSelect,
                20001,
                20002,
                QuantHardness::Hard,
                QuantHardness::Hard,
                QuantHardness::Hard,
                RouterTrainMode::HardTop1,
            )?,
        ])
    }

    #[must_use]
    pub fn phases(&self) -> &[TrainPhaseSpec] {
        &self.phases
    }

    #[must_use]
    pub fn into_phases(self) -> Vec<TrainPhaseSpec> {
        self.phases
    }
}

impl<'de> Deserialize<'de> for TrainingPhaseSchedule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawTrainingPhaseSchedule::deserialize(deserializer)?;
        Self::new(raw.phases).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingPhaseToml {
    pub training: TrainingPhaseSchedule,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RawTrainPhaseSpec {
    kind: TrainPhaseKind,
    start_step: u64,
    end_step: u64,
    expert_qat: QuantHardness,
    activation_qat: QuantHardness,
    norm_qat: QuantHardness,
    router_mode: RouterTrainMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RawTrainingPhaseSchedule {
    phases: Vec<TrainPhaseSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseScheduleError {
    WrongPhaseCount {
        expected: usize,
        actual: usize,
    },
    StartsAfterZero {
        actual_start: u64,
    },
    InvalidPhaseRange {
        kind: TrainPhaseKind,
        start_step: u64,
        end_step: u64,
    },
    UnexpectedPhaseKind {
        index: usize,
        expected_kind: TrainPhaseKind,
        actual_kind: TrainPhaseKind,
    },
    NonContiguous {
        previous_kind: TrainPhaseKind,
        next_kind: TrainPhaseKind,
        expected_start: u64,
        actual_start: u64,
    },
    StepOverflow {
        start_step: u64,
        added_steps: u64,
    },
}

impl fmt::Display for PhaseScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPhaseCount { expected, actual } => write!(
                f,
                "training phase schedule must contain exactly {expected} phases, got {actual}"
            ),
            Self::StartsAfterZero { actual_start } => write!(
                f,
                "training phase schedule must start at step 0, got {actual_start}"
            ),
            Self::InvalidPhaseRange {
                kind,
                start_step,
                end_step,
            } => write!(
                f,
                "training phase {kind} must have start_step < end_step, got {start_step}..{end_step}"
            ),
            Self::UnexpectedPhaseKind {
                index,
                expected_kind,
                actual_kind,
            } => write!(
                f,
                "training phase {index} must be {expected_kind}, got {actual_kind}"
            ),
            Self::NonContiguous {
                previous_kind,
                next_kind,
                expected_start,
                actual_start,
            } => write!(
                f,
                "training phase {next_kind} must start at step {expected_start} after {previous_kind}, got {actual_start}"
            ),
            Self::StepOverflow {
                start_step,
                added_steps,
            } => write!(
                f,
                "training phase step range overflowed when adding {added_steps} to {start_step}"
            ),
        }
    }
}

impl Error for PhaseScheduleError {}

fn validate_phases(phases: &[TrainPhaseSpec]) -> Result<(), PhaseScheduleError> {
    if phases.len() != TRAIN_PHASE_COUNT {
        return Err(PhaseScheduleError::WrongPhaseCount {
            expected: TRAIN_PHASE_COUNT,
            actual: phases.len(),
        });
    }

    if phases[0].start_step != 0 {
        return Err(PhaseScheduleError::StartsAfterZero {
            actual_start: phases[0].start_step,
        });
    }

    let canonical_order = TrainPhaseKind::canonical_order();
    for (index, (phase, expected_kind)) in phases.iter().zip(canonical_order).enumerate() {
        if phase.kind != expected_kind {
            return Err(PhaseScheduleError::UnexpectedPhaseKind {
                index,
                expected_kind,
                actual_kind: phase.kind,
            });
        }
    }

    for pair in phases.windows(2) {
        let previous = pair[0];
        let next = pair[1];
        if next.start_step != previous.end_step {
            return Err(PhaseScheduleError::NonContiguous {
                previous_kind: previous.kind,
                next_kind: next.kind,
                expected_start: previous.end_step,
                actual_start: next.start_step,
            });
        }
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct PhaseTemplate {
    kind: TrainPhaseKind,
    expert_qat: QuantHardness,
    activation_qat: QuantHardness,
    norm_qat: QuantHardness,
    router_mode: RouterTrainMode,
}

fn default_phase_templates() -> [PhaseTemplate; TRAIN_PHASE_COUNT] {
    [
        PhaseTemplate {
            kind: TrainPhaseKind::DenseTeacherWarmup,
            expert_qat: QuantHardness::Off,
            activation_qat: QuantHardness::Off,
            norm_qat: QuantHardness::Off,
            router_mode: RouterTrainMode::SoftTop1,
        },
        PhaseTemplate {
            kind: TrainPhaseKind::RouterWarmup,
            expert_qat: QuantHardness::Off,
            activation_qat: QuantHardness::Off,
            norm_qat: QuantHardness::Off,
            router_mode: RouterTrainMode::SoftTop1,
        },
        PhaseTemplate {
            kind: TrainPhaseKind::ExpertTernaryQat,
            expert_qat: QuantHardness::Hard,
            activation_qat: QuantHardness::Soft,
            norm_qat: QuantHardness::Soft,
            router_mode: RouterTrainMode::SoftTop1,
        },
        PhaseTemplate {
            kind: TrainPhaseKind::FullNumericQat,
            expert_qat: QuantHardness::Hard,
            activation_qat: QuantHardness::Hard,
            norm_qat: QuantHardness::Hard,
            router_mode: RouterTrainMode::HardTop1,
        },
        PhaseTemplate {
            kind: TrainPhaseKind::HardenAndSelect,
            expert_qat: QuantHardness::Hard,
            activation_qat: QuantHardness::Hard,
            norm_qat: QuantHardness::Hard,
            router_mode: RouterTrainMode::HardTop1,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_schedule_parses_from_training_toml_and_round_trips() {
        let config: TrainingPhaseToml = toml::from_str(VALID_PHASE_TOML).unwrap();
        let expected = literal_default_phase_specs(10);

        assert_eq!(config.training.phases(), expected.as_slice());

        let encoded = toml::to_string(&config).unwrap();
        let decoded: TrainingPhaseToml = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn phase_schedule_rejects_wrong_phase_count() {
        let phases = literal_default_phase_specs(10)[..4].to_vec();

        assert_eq!(
            TrainingPhaseSchedule::new(phases).unwrap_err(),
            PhaseScheduleError::WrongPhaseCount {
                expected: TRAIN_PHASE_COUNT,
                actual: 4,
            }
        );
    }

    #[test]
    fn phase_schedule_rejects_first_phase_after_zero() {
        let phases =
            literal_phase_specs_with_ranges([(10, 20), (20, 30), (30, 40), (40, 50), (50, 60)]);

        assert_eq!(
            TrainingPhaseSchedule::new(phases).unwrap_err(),
            PhaseScheduleError::StartsAfterZero { actual_start: 10 }
        );
    }

    #[test]
    fn phase_schedule_rejects_gaps() {
        let phases =
            literal_phase_specs_with_ranges([(0, 10), (10, 20), (21, 30), (30, 40), (40, 50)]);

        assert_eq!(
            TrainingPhaseSchedule::new(phases).unwrap_err(),
            PhaseScheduleError::NonContiguous {
                previous_kind: TrainPhaseKind::RouterWarmup,
                next_kind: TrainPhaseKind::ExpertTernaryQat,
                expected_start: 20,
                actual_start: 21,
            }
        );
    }

    #[test]
    fn phase_schedule_rejects_overlaps() {
        let phases =
            literal_phase_specs_with_ranges([(0, 10), (10, 20), (19, 30), (30, 40), (40, 50)]);

        assert_eq!(
            TrainingPhaseSchedule::new(phases).unwrap_err(),
            PhaseScheduleError::NonContiguous {
                previous_kind: TrainPhaseKind::RouterWarmup,
                next_kind: TrainPhaseKind::ExpertTernaryQat,
                expected_start: 20,
                actual_start: 19,
            }
        );
    }

    #[test]
    fn phase_schedule_rejects_noncanonical_phase_order() {
        let mut phases = literal_default_phase_specs(10).to_vec();
        phases.swap(1, 2);

        assert_eq!(
            TrainingPhaseSchedule::new(phases).unwrap_err(),
            PhaseScheduleError::UnexpectedPhaseKind {
                index: 1,
                expected_kind: TrainPhaseKind::RouterWarmup,
                actual_kind: TrainPhaseKind::ExpertTernaryQat,
            }
        );
    }

    #[test]
    fn phase_specs_reject_zero_length_ranges() {
        assert_eq!(
            TrainPhaseSpec::new(
                TrainPhaseKind::DenseTeacherWarmup,
                4,
                4,
                QuantHardness::Off,
                QuantHardness::Off,
                QuantHardness::Off,
                RouterTrainMode::SoftTop1,
            )
            .unwrap_err(),
            PhaseScheduleError::InvalidPhaseRange {
                kind: TrainPhaseKind::DenseTeacherWarmup,
                start_step: 4,
                end_step: 4,
            }
        );
    }

    #[test]
    fn default_five_phase_schedule_is_constructible() {
        let schedule = TrainingPhaseSchedule::default_five_phase(10).unwrap();
        let expected = literal_default_phase_specs(10);

        assert_eq!(schedule.phases(), expected.as_slice());
        assert_eq!(schedule.phases()[0].len_steps(), 10);
        assert_eq!(schedule.phases()[4].end_step(), 50);
    }

    #[test]
    fn s5_pick_and_fit_schedule_pins_phase_c_to_step_6002() {
        let schedule = TrainingPhaseSchedule::s5_pick_and_fit().unwrap();

        assert_eq!(
            schedule
                .phases()
                .iter()
                .map(|phase| (phase.kind(), phase.start_step(), phase.end_step()))
                .collect::<Vec<_>>(),
            vec![
                (TrainPhaseKind::DenseTeacherWarmup, 0, 6001),
                (TrainPhaseKind::RouterWarmup, 6001, 6002),
                (TrainPhaseKind::ExpertTernaryQat, 6002, 12001),
                (TrainPhaseKind::FullNumericQat, 12001, 20001),
                (TrainPhaseKind::HardenAndSelect, 20001, 20002),
            ]
        );
        assert_eq!(schedule.phases()[1].len_steps(), 1);
    }

    #[test]
    fn default_five_phase_schedule_rejects_step_overflow() {
        assert_eq!(
            TrainingPhaseSchedule::default_five_phase(u64::MAX / 5 + 1).unwrap_err(),
            PhaseScheduleError::StepOverflow {
                start_step: 14_757_395_258_967_641_296,
                added_steps: 3_689_348_814_741_910_324,
            }
        );
    }

    fn literal_default_phase_specs(steps_per_phase: u64) -> [TrainPhaseSpec; TRAIN_PHASE_COUNT] {
        assert_eq!(steps_per_phase, 10);
        [
            TrainPhaseSpec::new(
                TrainPhaseKind::DenseTeacherWarmup,
                0,
                10,
                QuantHardness::Off,
                QuantHardness::Off,
                QuantHardness::Off,
                RouterTrainMode::SoftTop1,
            )
            .unwrap(),
            TrainPhaseSpec::new(
                TrainPhaseKind::RouterWarmup,
                10,
                20,
                QuantHardness::Off,
                QuantHardness::Off,
                QuantHardness::Off,
                RouterTrainMode::SoftTop1,
            )
            .unwrap(),
            TrainPhaseSpec::new(
                TrainPhaseKind::ExpertTernaryQat,
                20,
                30,
                QuantHardness::Hard,
                QuantHardness::Soft,
                QuantHardness::Soft,
                RouterTrainMode::SoftTop1,
            )
            .unwrap(),
            TrainPhaseSpec::new(
                TrainPhaseKind::FullNumericQat,
                30,
                40,
                QuantHardness::Hard,
                QuantHardness::Hard,
                QuantHardness::Hard,
                RouterTrainMode::HardTop1,
            )
            .unwrap(),
            TrainPhaseSpec::new(
                TrainPhaseKind::HardenAndSelect,
                40,
                50,
                QuantHardness::Hard,
                QuantHardness::Hard,
                QuantHardness::Hard,
                RouterTrainMode::HardTop1,
            )
            .unwrap(),
        ]
    }

    fn literal_phase_specs_with_ranges(
        ranges: [(u64, u64); TRAIN_PHASE_COUNT],
    ) -> Vec<TrainPhaseSpec> {
        let mut phases = Vec::with_capacity(TRAIN_PHASE_COUNT);
        let defaults = literal_default_phase_specs(10);
        for (index, default) in defaults.iter().enumerate() {
            phases.push(
                TrainPhaseSpec::new(
                    default.kind(),
                    ranges[index].0,
                    ranges[index].1,
                    default.expert_qat(),
                    default.activation_qat(),
                    default.norm_qat(),
                    default.router_mode(),
                )
                .unwrap(),
            );
        }
        phases
    }

    const VALID_PHASE_TOML: &str = r#"
        [training]

        [[training.phases]]
        kind = "dense_teacher_warmup"
        start_step = 0
        end_step = 10
        expert_qat = "off"
        activation_qat = "off"
        norm_qat = "off"
        router_mode = "soft_top1"

        [[training.phases]]
        kind = "router_warmup"
        start_step = 10
        end_step = 20
        expert_qat = "off"
        activation_qat = "off"
        norm_qat = "off"
        router_mode = "soft_top1"

        [[training.phases]]
        kind = "expert_ternary_qat"
        start_step = 20
        end_step = 30
        expert_qat = "hard"
        activation_qat = "soft"
        norm_qat = "soft"
        router_mode = "soft_top1"

        [[training.phases]]
        kind = "full_numeric_qat"
        start_step = 30
        end_step = 40
        expert_qat = "hard"
        activation_qat = "hard"
        norm_qat = "hard"
        router_mode = "hard_top1"

        [[training.phases]]
        kind = "harden_and_select"
        start_step = 40
        end_step = 50
        expert_qat = "hard"
        activation_qat = "hard"
        norm_qat = "hard"
        router_mode = "hard_top1"
        "#;
}

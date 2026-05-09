//! Training-side deployability preflight helpers.

use std::error::Error;
use std::fmt;

use gbf_artifact::weight_plan::TernaryWeightPlan;
use gbf_foundation::ByteCost;
use gbf_model::budget::{
    ExpertBudgetError, ExpertSlotFit, StaticBudgetReport, compute_expert_bytes_checked,
};
use gbf_policy::model_profile::ModelSizeProfile;

use crate::logging::{ExpertSlotPreflightEvent, LoggingEventError, TrainingLogEmitter};

pub fn compute_preflight_expert_bytes(
    plan: &TernaryWeightPlan,
    d_model: u32,
    d_ff: u32,
) -> Result<ByteCost, ExpertBudgetError> {
    compute_expert_bytes_checked(plan, d_model, d_ff)
}

/// Narrow training-side view of the runtime chrome budget.
///
/// The source-of-truth `RuntimeChromeBudget` type is not yet present in this
/// workspace. This object models only the boundary this preflight owns: the
/// usable byte capacity of every `ExpertBank` slot emitted by a future runtime
/// budget artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpertBankBudgetSurface {
    expert_bank_usable_bytes: Vec<ByteCost>,
}

impl ExpertBankBudgetSurface {
    pub fn new(expert_bank_usable_bytes: Vec<ByteCost>) -> Result<Self, ProfilePreflightError> {
        if expert_bank_usable_bytes.is_empty() {
            return Err(ProfilePreflightError::MissingExpertBanks);
        }
        if expert_bank_usable_bytes.contains(&ByteCost::ZERO) {
            return Err(ProfilePreflightError::EmptyExpertBank);
        }

        Ok(Self {
            expert_bank_usable_bytes,
        })
    }

    #[must_use]
    pub fn expert_bank_usable_bytes(&self) -> &[ByteCost] {
        &self.expert_bank_usable_bytes
    }

    #[must_use]
    pub fn smallest_expert_bank_usable_bytes(&self) -> ByteCost {
        *self
            .expert_bank_usable_bytes
            .iter()
            .min()
            .expect("constructor requires at least one ExpertBank")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileExpertBankPreflightReport {
    profile: ModelSizeProfile,
    expert_bytes: ByteCost,
    smallest_expert_bank_usable_bytes: ByteCost,
    slack_bytes: ByteCost,
}

impl ProfileExpertBankPreflightReport {
    #[must_use]
    pub const fn profile(self) -> ModelSizeProfile {
        self.profile
    }

    #[must_use]
    pub const fn expert_bytes(self) -> ByteCost {
        self.expert_bytes
    }

    #[must_use]
    pub const fn smallest_expert_bank_usable_bytes(self) -> ByteCost {
        self.smallest_expert_bank_usable_bytes
    }

    #[must_use]
    pub const fn slack_bytes(self) -> ByteCost {
        self.slack_bytes
    }
}

pub fn preflight_profile_expert_bank_budget(
    budget: &ExpertBankBudgetSurface,
    profile: ModelSizeProfile,
) -> Result<ProfileExpertBankPreflightReport, ProfilePreflightError> {
    let expert_bytes = profile.expert_byte_cost();
    let smallest_expert_bank_usable_bytes = budget.smallest_expert_bank_usable_bytes();

    let Some(slack_bytes) = smallest_expert_bank_usable_bytes.checked_sub(expert_bytes) else {
        return Err(ProfilePreflightError::ExpertExceedsSmallestBank {
            profile,
            expert_bytes,
            smallest_expert_bank_usable_bytes,
            over_by: expert_bytes - smallest_expert_bank_usable_bytes,
        });
    };

    Ok(ProfileExpertBankPreflightReport {
        profile,
        expert_bytes,
        smallest_expert_bank_usable_bytes,
        slack_bytes,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpertBudgetPreflightReport {
    static_budget: StaticBudgetReport,
}

impl ExpertBudgetPreflightReport {
    pub fn check_expert_slot(
        plan: &TernaryWeightPlan,
        d_model: u32,
        d_ff: u32,
        expert_slot_usable_bytes: ByteCost,
    ) -> Result<Self, ExpertBudgetError> {
        Ok(Self {
            static_budget: StaticBudgetReport::for_expert_checked(
                plan,
                d_model,
                d_ff,
                Some(expert_slot_usable_bytes),
            )?,
        })
    }

    pub fn check_expert_slot_with_logging(
        plan: &TernaryWeightPlan,
        d_model: u32,
        d_ff: u32,
        expert_slot_usable_bytes: ByteCost,
        emitter: &TrainingLogEmitter,
    ) -> Result<Self, PreflightLoggingError> {
        let report = match Self::check_expert_slot(plan, d_model, d_ff, expert_slot_usable_bytes) {
            Ok(report) => report,
            Err(error) => {
                emit_budget_error(emitter, expert_slot_usable_bytes, error)?;
                return Err(PreflightLoggingError::Budget(error));
            }
        };
        report.emit_structured_log(emitter)?;
        Ok(report)
    }

    #[must_use]
    pub const fn static_budget(self) -> StaticBudgetReport {
        self.static_budget
    }

    #[must_use]
    pub fn expert_bytes(self) -> ByteCost {
        self.static_budget.expert_bytes()
    }

    #[must_use]
    pub fn expert_slot_fit(self) -> ExpertSlotFit {
        self.static_budget
            .expert_slot_fit()
            .expect("preflight report always has an expert slot budget")
    }

    #[must_use]
    pub fn fits_expert_slot(self) -> bool {
        self.expert_slot_fit().fits()
    }

    pub fn emit_structured_log(
        self,
        emitter: &TrainingLogEmitter,
    ) -> Result<(), LoggingEventError> {
        emitter.expert_slot_preflight(&self.to_preflight_event()?)
    }

    pub fn to_preflight_event(self) -> Result<ExpertSlotPreflightEvent, LoggingEventError> {
        let expert_bytes = self.expert_bytes();
        let slot_bytes = self
            .static_budget()
            .expert_slot_usable_bytes()
            .expect("preflight report always has an expert slot budget");
        let fit = self.expert_slot_fit();
        match fit {
            ExpertSlotFit::Fits { slack } => ExpertSlotPreflightEvent::fits(
                format!(
                    "expert payload fits slot with {} slack bytes",
                    slack.as_u64()
                ),
                expert_bytes.as_u64(),
                slot_bytes.as_u64(),
                slack.as_u64(),
            ),
            ExpertSlotFit::Exceeds { over_by } => ExpertSlotPreflightEvent::exceeds(
                format!("expert payload exceeds slot by {} bytes", over_by.as_u64()),
                expert_bytes.as_u64(),
                slot_bytes.as_u64(),
                over_by.as_u64(),
            ),
        }
    }
}

fn emit_budget_error(
    emitter: &TrainingLogEmitter,
    expert_slot_usable_bytes: ByteCost,
    error: ExpertBudgetError,
) -> Result<(), LoggingEventError> {
    emitter.expert_slot_preflight(&ExpertSlotPreflightEvent::invalid(
        format!("expert slot budget could not be computed: {error}"),
        expert_slot_usable_bytes.as_u64(),
    )?)
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreflightLoggingError {
    Budget(ExpertBudgetError),
    Logging(LoggingEventError),
}

impl fmt::Display for PreflightLoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(error) => write!(f, "{error}"),
            Self::Logging(error) => write!(f, "{error}"),
        }
    }
}

impl Error for PreflightLoggingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Budget(error) => Some(error),
            Self::Logging(error) => Some(error),
        }
    }
}

impl From<ExpertBudgetError> for PreflightLoggingError {
    fn from(error: ExpertBudgetError) -> Self {
        Self::Budget(error)
    }
}

impl From<LoggingEventError> for PreflightLoggingError {
    fn from(error: LoggingEventError) -> Self {
        Self::Logging(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfilePreflightError {
    MissingExpertBanks,
    EmptyExpertBank,
    ExpertExceedsSmallestBank {
        profile: ModelSizeProfile,
        expert_bytes: ByteCost,
        smallest_expert_bank_usable_bytes: ByteCost,
        over_by: ByteCost,
    },
}

impl fmt::Display for ProfilePreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingExpertBanks => {
                f.write_str("runtime chrome budget surface has no ExpertBank slots")
            }
            Self::EmptyExpertBank => {
                f.write_str("runtime chrome budget surface has an empty ExpertBank slot")
            }
            Self::ExpertExceedsSmallestBank {
                profile,
                expert_bytes,
                smallest_expert_bank_usable_bytes,
                over_by,
            } => write!(
                f,
                "{profile:?} expert payload {expert_bytes} exceeds smallest ExpertBank usable capacity {smallest_expert_bank_usable_bytes} by {over_by}"
            ),
        }
    }
}

impl Error for ProfilePreflightError {}

#[cfg(test)]
mod tests {
    use gbf_artifact::weight_plan::{
        ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
    };

    use crate::logging::{TestEventCollector, TestEventKind, TestFieldValue};

    use super::*;

    #[test]
    fn preflight_expert_budget_uses_model_compute_expert_bytes() {
        let plan = default_plan();
        let expected = compute_expert_bytes_checked(&plan, 128, 224).unwrap();

        assert_eq!(
            compute_preflight_expert_bytes(&plan, 128, 224),
            Ok(expected)
        );

        let report =
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 224, ByteCost::new(16_384))
                .unwrap();
        assert_eq!(report.expert_bytes(), expected);
        assert_eq!(report.static_budget().expert_bytes(), expected);
        assert_eq!(
            report.expert_slot_fit(),
            ExpertSlotFit::Fits {
                slack: ByteCost::new(1_294),
            }
        );
        assert!(report.fits_expert_slot());
    }

    #[test]
    fn preflight_logging_path_emits_pass_event_from_real_budget_report() {
        let plan = default_plan();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let report = ExpertBudgetPreflightReport::check_expert_slot_with_logging(
            &plan,
            128,
            224,
            ByteCost::new(16_384),
            &emitter,
        )
        .unwrap();

        assert!(report.fits_expert_slot());
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), TestEventKind::Preflight);
        assert_eq!(
            events[0].field("check_name"),
            Some(&TestFieldValue::String("expert_slot_budget".to_owned()))
        );
        assert_eq!(
            events[0].field("status"),
            Some(&TestFieldValue::String("pass".to_owned()))
        );
        assert_eq!(
            events[0].field("budget_computed"),
            Some(&TestFieldValue::Bool(true))
        );
        assert_eq!(
            events[0].field("expert_bytes"),
            Some(&TestFieldValue::U64(15_090))
        );
        assert_eq!(
            events[0].field("expert_slot_usable_bytes"),
            Some(&TestFieldValue::U64(16_384))
        );
        assert_eq!(
            events[0].field("slack_bytes"),
            Some(&TestFieldValue::U64(1_294))
        );
    }

    #[test]
    fn preflight_reports_over_budget_experts_before_training() {
        let plan = default_plan();

        let report =
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 224, ByteCost::new(15_000))
                .unwrap();

        assert_eq!(
            report.expert_slot_fit(),
            ExpertSlotFit::Exceeds {
                over_by: ByteCost::new(90),
            }
        );
        assert!(!report.fits_expert_slot());
    }

    #[test]
    fn preflight_logging_path_emits_fail_event_for_over_budget_report() {
        let plan = default_plan();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let report = ExpertBudgetPreflightReport::check_expert_slot_with_logging(
            &plan,
            128,
            224,
            ByteCost::new(15_000),
            &emitter,
        )
        .unwrap();

        assert!(!report.fits_expert_slot());
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].field("status"),
            Some(&TestFieldValue::String("fail".to_owned()))
        );
        assert_eq!(
            events[0].field("detail"),
            Some(&TestFieldValue::String(
                "expert payload exceeds slot by 90 bytes".to_owned()
            ))
        );
        assert_eq!(
            events[0].field("expert_bytes"),
            Some(&TestFieldValue::U64(15_090))
        );
        assert_eq!(
            events[0].field("over_by_bytes"),
            Some(&TestFieldValue::U64(90))
        );
    }

    #[test]
    fn preflight_rejects_zero_expert_dimensions() {
        let plan = default_plan();

        assert_eq!(
            compute_preflight_expert_bytes(&plan, 0, 224),
            Err(ExpertBudgetError::EmptyDimension { field: "d_model" })
        );
        assert_eq!(
            ExpertBudgetPreflightReport::check_expert_slot(&plan, 128, 0, ByteCost::new(16_384),),
            Err(ExpertBudgetError::EmptyDimension { field: "d_ff" })
        );
    }

    #[test]
    fn profile_expert_budget_passes_when_profile_fits_smallest_expert_bank() {
        let profile = ModelSizeProfile::moe_tiny(4).unwrap();
        let budget = ExpertBankBudgetSurface::new(vec![
            ByteCost::new(8_192),
            ByteCost::new(4_480),
            ByteCost::new(16_384),
        ])
        .unwrap();

        let report = preflight_profile_expert_bank_budget(&budget, profile).unwrap();

        assert_eq!(report.profile(), profile);
        assert_eq!(report.expert_bytes(), ByteCost::new(4_480));
        assert_eq!(
            report.smallest_expert_bank_usable_bytes(),
            ByteCost::new(4_480)
        );
        assert_eq!(report.slack_bytes(), ByteCost::ZERO);
    }

    #[test]
    fn profile_expert_budget_reports_positive_slack() {
        let profile = ModelSizeProfile::moe_tiny(2).unwrap();
        let budget =
            ExpertBankBudgetSurface::new(vec![ByteCost::new(16_384), ByteCost::new(8_192)])
                .unwrap();

        let report = preflight_profile_expert_bank_budget(&budget, profile).unwrap();

        assert_eq!(report.expert_bytes(), ByteCost::new(4_480));
        assert_eq!(
            report.smallest_expert_bank_usable_bytes(),
            ByteCost::new(8_192)
        );
        assert_eq!(report.slack_bytes(), ByteCost::new(3_712));
    }

    #[test]
    fn profile_expert_budget_rejects_profile_larger_than_smallest_expert_bank() {
        let profile = ModelSizeProfile::upper_bank_candidate(128, 4).unwrap();
        let budget =
            ExpertBankBudgetSurface::new(vec![ByteCost::new(16_384), ByteCost::new(12_000)])
                .unwrap();

        assert_eq!(
            preflight_profile_expert_bank_budget(&budget, profile),
            Err(ProfilePreflightError::ExpertExceedsSmallestBank {
                profile,
                expert_bytes: ByteCost::new(12_928),
                smallest_expert_bank_usable_bytes: ByteCost::new(12_000),
                over_by: ByteCost::new(928),
            })
        );
    }

    #[test]
    fn profile_expert_budget_selects_smallest_expert_bank() {
        let budget = ExpertBankBudgetSurface::new(vec![
            ByteCost::new(16_384),
            ByteCost::new(9_000),
            ByteCost::new(12_000),
        ])
        .unwrap();
        let profile = ModelSizeProfile::upper_bank_candidate(96, 4).unwrap();

        assert_eq!(
            preflight_profile_expert_bank_budget(&budget, profile),
            Err(ProfilePreflightError::ExpertExceedsSmallestBank {
                profile,
                expert_bytes: ByteCost::new(9_792),
                smallest_expert_bank_usable_bytes: ByteCost::new(9_000),
                over_by: ByteCost::new(792),
            })
        );
    }

    #[test]
    fn profile_expert_budget_surface_rejects_missing_or_empty_banks() {
        assert_eq!(
            ExpertBankBudgetSurface::new(vec![]),
            Err(ProfilePreflightError::MissingExpertBanks)
        );
        assert_eq!(
            ExpertBankBudgetSurface::new(vec![ByteCost::new(16_384), ByteCost::ZERO]),
            Err(ProfilePreflightError::EmptyExpertBank)
        );
    }

    #[test]
    fn preflight_logging_path_emits_fail_event_for_invalid_budget_input() {
        let plan = default_plan();
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        let error = ExpertBudgetPreflightReport::check_expert_slot_with_logging(
            &plan,
            0,
            224,
            ByteCost::new(16_384),
            &emitter,
        )
        .unwrap_err();

        assert_eq!(
            error,
            PreflightLoggingError::Budget(ExpertBudgetError::EmptyDimension { field: "d_model" })
        );
        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].field("status"),
            Some(&TestFieldValue::String("fail".to_owned()))
        );
        assert_eq!(
            events[0].field("budget_computed"),
            Some(&TestFieldValue::Bool(false))
        );
        assert_eq!(
            events[0].field("detail"),
            Some(&TestFieldValue::String(
                "expert slot budget could not be computed: d_model must be nonzero".to_owned()
            ))
        );
    }

    fn default_plan() -> TernaryWeightPlan {
        TernaryWeightPlan::new(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerOutputRow,
            ScaleFormat::Q8_8,
            ThresholdPlan::FixedQ8_8,
        )
    }
}

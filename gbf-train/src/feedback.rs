//! Compiler feedback ingestion for training.
//!
//! This module owns the config-gated reader and typed application boundary for
//! `compiler_feedback.json`. Concrete training-loop callers provide a target
//! that knows how to update activation range penalties and router affinity
//! hints.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gbf_foundation::{ExpertId, LayerId};
use gbf_policy::s5::{S5FeedbackApplyConfig, s5_apply_feedback_safe_bound};
use serde::{Deserialize, Serialize};

pub const COMPILER_FEEDBACK_SCHEMA: &str = "compiler_feedback.v1";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerFeedbackConfig {
    pub enabled: bool,
    pub safe_bound: S5FeedbackApplyConfig,
}

impl CompilerFeedbackConfig {
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            safe_bound: S5FeedbackApplyConfig::pinned(),
        }
    }

    #[must_use]
    pub const fn enabled() -> Self {
        Self {
            enabled: true,
            safe_bound: S5FeedbackApplyConfig::pinned(),
        }
    }
}

impl Default for CompilerFeedbackConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerFeedback {
    pub schema: String,
    #[serde(default)]
    pub range_hotspots: Vec<ActivationRangeFeedback>,
    #[serde(default)]
    pub expert_slot_affinities: Vec<ExpertSlotAffinityFeedback>,
    #[serde(default)]
    pub warnings: Vec<CompilerFeedbackWarning>,
}

impl CompilerFeedback {
    pub fn validate(&self) -> Result<(), CompilerFeedbackError> {
        if self.schema != COMPILER_FEEDBACK_SCHEMA {
            return Err(CompilerFeedbackError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        for hotspot in &self.range_hotspots {
            hotspot.validate()?;
        }
        for affinity in &self.expert_slot_affinities {
            affinity.validate()?;
        }
        for warning in &self.warnings {
            warning.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRangeFeedback {
    pub target: String,
    pub current_safe_bound: f64,
    pub observed_max_abs: f64,
}

impl ActivationRangeFeedback {
    fn validate(&self) -> Result<(), CompilerFeedbackError> {
        validate_nonempty("range_hotspots.target", &self.target)?;
        validate_finite_non_negative("range_hotspots.current_safe_bound", self.current_safe_bound)?;
        validate_finite_non_negative("range_hotspots.observed_max_abs", self.observed_max_abs)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRangeUpdate {
    pub target: String,
    pub previous_safe_bound: f64,
    pub observed_max_abs: f64,
    pub next_safe_bound: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertSlotAffinityFeedback {
    pub layer: LayerId,
    pub expert_a: ExpertId,
    pub expert_b: ExpertId,
    /// Q8.8 unit score; 256 encodes 1.0.
    pub affinity_score_q8_8: u16,
}

impl ExpertSlotAffinityFeedback {
    fn validate(&self) -> Result<(), CompilerFeedbackError> {
        if self.expert_a == self.expert_b {
            return Err(CompilerFeedbackError::InvalidAffinityPair {
                layer: self.layer,
                expert: self.expert_a,
            });
        }
        if self.affinity_score_q8_8 > 256 {
            return Err(CompilerFeedbackError::AffinityScoreOutOfRange {
                layer: self.layer,
                score: self.affinity_score_q8_8,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerFeedbackWarning {
    pub code: String,
    pub message: String,
}

impl CompilerFeedbackWarning {
    fn validate(&self) -> Result<(), CompilerFeedbackError> {
        validate_nonempty("warnings.code", &self.code)?;
        validate_nonempty("warnings.message", &self.message)
    }
}

pub trait CompilerFeedbackTarget {
    fn update_activation_range_target(
        &mut self,
        update: &ActivationRangeUpdate,
    ) -> Result<(), CompilerFeedbackError>;

    fn update_expert_slot_affinity(
        &mut self,
        affinity: &ExpertSlotAffinityFeedback,
    ) -> Result<(), CompilerFeedbackError>;

    fn record_feedback_warning(
        &mut self,
        _warning: &CompilerFeedbackWarning,
    ) -> Result<(), CompilerFeedbackError> {
        Ok(())
    }

    fn record_feedback_event(
        &mut self,
        _event: &FeedbackApplicationEvent,
    ) -> Result<(), CompilerFeedbackError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackApplicationStatus {
    Disabled,
    Missing,
    Applied,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackApplicationReport {
    pub status: FeedbackApplicationStatus,
    pub source_path: PathBuf,
    pub range_updates: Vec<ActivationRangeUpdate>,
    pub affinity_updates: Vec<ExpertSlotAffinityFeedback>,
    pub warnings: Vec<CompilerFeedbackWarning>,
    pub events: Vec<FeedbackApplicationEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeedbackApplicationEvent {
    Disabled {
        path: PathBuf,
    },
    MissingFile {
        path: PathBuf,
    },
    RangeTargetApplied {
        target: String,
        previous_safe_bound: f64,
        next_safe_bound: f64,
    },
    ExpertSlotAffinityApplied {
        layer: LayerId,
        expert_a: ExpertId,
        expert_b: ExpertId,
        affinity_score_q8_8: u16,
    },
    Warning {
        code: String,
        message: String,
    },
}

pub fn apply_compiler_feedback_file<T: CompilerFeedbackTarget>(
    path: impl AsRef<Path>,
    config: CompilerFeedbackConfig,
    target: &mut T,
) -> Result<FeedbackApplicationReport, CompilerFeedbackError> {
    let path = path.as_ref();
    if !config.enabled {
        let event = FeedbackApplicationEvent::Disabled {
            path: path.to_path_buf(),
        };
        target.record_feedback_event(&event)?;
        return Ok(FeedbackApplicationReport {
            status: FeedbackApplicationStatus::Disabled,
            source_path: path.to_path_buf(),
            range_updates: Vec::new(),
            affinity_updates: Vec::new(),
            warnings: Vec::new(),
            events: vec![event],
        });
    }

    match fs::read_to_string(path) {
        Ok(json) => {
            let feedback: CompilerFeedback =
                serde_json::from_str(&json).map_err(CompilerFeedbackError::Json)?;
            apply_compiler_feedback(path, feedback, config, target)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let warning = CompilerFeedbackWarning {
                code: "compiler_feedback_missing".to_owned(),
                message: format!("compiler feedback file {} was not found", path.display()),
            };
            target.record_feedback_warning(&warning)?;
            let missing_event = FeedbackApplicationEvent::MissingFile {
                path: path.to_path_buf(),
            };
            let warning_event = FeedbackApplicationEvent::Warning {
                code: warning.code.clone(),
                message: warning.message.clone(),
            };
            target.record_feedback_event(&missing_event)?;
            target.record_feedback_event(&warning_event)?;
            Ok(FeedbackApplicationReport {
                status: FeedbackApplicationStatus::Missing,
                source_path: path.to_path_buf(),
                range_updates: Vec::new(),
                affinity_updates: Vec::new(),
                warnings: vec![warning.clone()],
                events: vec![missing_event, warning_event],
            })
        }
        Err(error) => Err(CompilerFeedbackError::Io {
            path: path.to_path_buf(),
            error,
        }),
    }
}

pub fn apply_compiler_feedback<T: CompilerFeedbackTarget>(
    source_path: impl AsRef<Path>,
    feedback: CompilerFeedback,
    config: CompilerFeedbackConfig,
    target: &mut T,
) -> Result<FeedbackApplicationReport, CompilerFeedbackError> {
    let source_path = source_path.as_ref().to_path_buf();
    if !config.enabled {
        let event = FeedbackApplicationEvent::Disabled {
            path: source_path.clone(),
        };
        target.record_feedback_event(&event)?;
        return Ok(FeedbackApplicationReport {
            status: FeedbackApplicationStatus::Disabled,
            source_path,
            range_updates: Vec::new(),
            affinity_updates: Vec::new(),
            warnings: Vec::new(),
            events: vec![event],
        });
    }

    feedback.validate()?;
    let mut range_updates = Vec::with_capacity(feedback.range_hotspots.len());
    let mut affinity_updates = Vec::with_capacity(feedback.expert_slot_affinities.len());
    let mut events = Vec::new();

    for hotspot in feedback.range_hotspots {
        let next_safe_bound = s5_apply_feedback_safe_bound(
            hotspot.current_safe_bound,
            hotspot.observed_max_abs,
            config.safe_bound,
        )
        .ok_or(CompilerFeedbackError::InvalidSafeBoundUpdate {
            target: hotspot.target.clone(),
        })?;
        let update = ActivationRangeUpdate {
            target: hotspot.target,
            previous_safe_bound: hotspot.current_safe_bound,
            observed_max_abs: hotspot.observed_max_abs,
            next_safe_bound,
        };
        target.update_activation_range_target(&update)?;
        let event = FeedbackApplicationEvent::RangeTargetApplied {
            target: update.target.clone(),
            previous_safe_bound: update.previous_safe_bound,
            next_safe_bound: update.next_safe_bound,
        };
        target.record_feedback_event(&event)?;
        events.push(event);
        range_updates.push(update);
    }

    for affinity in feedback.expert_slot_affinities {
        target.update_expert_slot_affinity(&affinity)?;
        let event = FeedbackApplicationEvent::ExpertSlotAffinityApplied {
            layer: affinity.layer,
            expert_a: affinity.expert_a,
            expert_b: affinity.expert_b,
            affinity_score_q8_8: affinity.affinity_score_q8_8,
        };
        target.record_feedback_event(&event)?;
        events.push(event);
        affinity_updates.push(affinity);
    }

    for warning in &feedback.warnings {
        target.record_feedback_warning(warning)?;
        let event = FeedbackApplicationEvent::Warning {
            code: warning.code.clone(),
            message: warning.message.clone(),
        };
        target.record_feedback_event(&event)?;
        events.push(event);
    }

    Ok(FeedbackApplicationReport {
        status: FeedbackApplicationStatus::Applied,
        source_path,
        range_updates,
        affinity_updates,
        warnings: feedback.warnings,
        events,
    })
}

#[derive(Debug)]
pub enum CompilerFeedbackError {
    Io { path: PathBuf, error: io::Error },
    Json(serde_json::Error),
    InvalidSchema { observed: String },
    EmptyField { field: &'static str },
    NonFiniteMetric { field: &'static str, value: f64 },
    NegativeMetric { field: &'static str, value: f64 },
    InvalidSafeBoundUpdate { target: String },
    InvalidAffinityPair { layer: LayerId, expert: ExpertId },
    AffinityScoreOutOfRange { layer: LayerId, score: u16 },
    Target(String),
}

impl CompilerFeedbackError {
    #[must_use]
    pub fn target(message: impl Into<String>) -> Self {
        Self::Target(message.into())
    }
}

impl fmt::Display for CompilerFeedbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, error } => {
                write!(
                    f,
                    "failed to read compiler feedback {}: {error}",
                    path.display()
                )
            }
            Self::Json(error) => write!(f, "{error}"),
            Self::InvalidSchema { observed } => {
                write!(
                    f,
                    "expected compiler feedback schema {COMPILER_FEEDBACK_SCHEMA}, got {observed}"
                )
            }
            Self::EmptyField { field } => write!(f, "{field} must not be empty"),
            Self::NonFiniteMetric { field, value } => {
                write!(f, "{field} must be finite, got {value}")
            }
            Self::NegativeMetric { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
            Self::InvalidSafeBoundUpdate { target } => {
                write!(f, "feedback safe-bound update for {target} is invalid")
            }
            Self::InvalidAffinityPair { layer, expert } => {
                write!(
                    f,
                    "expert slot affinity for layer {} uses expert {} twice",
                    layer.get(),
                    expert.get()
                )
            }
            Self::AffinityScoreOutOfRange { layer, score } => {
                write!(
                    f,
                    "expert slot affinity for layer {} has q8.8 score {score}, expected <= 256",
                    layer.get()
                )
            }
            Self::Target(message) => f.write_str(message),
        }
    }
}

impl Error for CompilerFeedbackError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            Self::Json(error) => Some(error),
            Self::InvalidSchema { .. }
            | Self::EmptyField { .. }
            | Self::NonFiniteMetric { .. }
            | Self::NegativeMetric { .. }
            | Self::InvalidSafeBoundUpdate { .. }
            | Self::InvalidAffinityPair { .. }
            | Self::AffinityScoreOutOfRange { .. }
            | Self::Target(_) => None,
        }
    }
}

fn validate_nonempty(field: &'static str, value: &str) -> Result<(), CompilerFeedbackError> {
    if value.trim().is_empty() {
        return Err(CompilerFeedbackError::EmptyField { field });
    }
    Ok(())
}

fn validate_finite_non_negative(
    field: &'static str,
    value: f64,
) -> Result<(), CompilerFeedbackError> {
    if !value.is_finite() {
        return Err(CompilerFeedbackError::NonFiniteMetric { field, value });
    }
    if value < 0.0 {
        return Err(CompilerFeedbackError::NegativeMetric { field, value });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn compiler_feedback_disabled_by_default_does_not_read_missing_file() {
        let mut target = RecordingFeedbackTarget::default();
        let path = temp_feedback_path("disabled");

        let report =
            apply_compiler_feedback_file(&path, CompilerFeedbackConfig::default(), &mut target)
                .unwrap();

        assert_eq!(report.status, FeedbackApplicationStatus::Disabled);
        assert!(target.range_updates.is_empty());
        assert!(target.affinity_updates.is_empty());
        assert!(target.warnings.is_empty());
        assert_eq!(target.events, report.events);
        assert!(matches!(
            report.events.as_slice(),
            [FeedbackApplicationEvent::Disabled { .. }]
        ));

        let mut target = RecordingFeedbackTarget::default();
        let report = apply_compiler_feedback(
            "memory",
            CompilerFeedback {
                schema: "wrong-but-ignored-when-disabled".to_owned(),
                range_hotspots: Vec::new(),
                expert_slot_affinities: Vec::new(),
                warnings: Vec::new(),
            },
            CompilerFeedbackConfig::disabled(),
            &mut target,
        )
        .unwrap();
        assert_eq!(report.status, FeedbackApplicationStatus::Disabled);
        assert_eq!(target.events, report.events);
    }

    #[test]
    fn compiler_feedback_missing_file_is_warning_not_error() {
        let mut target = RecordingFeedbackTarget::default();
        let path = temp_feedback_path("missing");

        let report =
            apply_compiler_feedback_file(&path, CompilerFeedbackConfig::enabled(), &mut target)
                .unwrap();

        assert_eq!(report.status, FeedbackApplicationStatus::Missing);
        assert_eq!(target.warnings.len(), 1);
        assert_eq!(target.events, report.events);
        assert_eq!(report.warnings[0].code, "compiler_feedback_missing");
        assert!(report.range_updates.is_empty());
        assert!(report.affinity_updates.is_empty());
    }

    #[test]
    fn compiler_feedback_enabled_applies_range_affinity_and_warnings() {
        let mut target = RecordingFeedbackTarget::default();
        let path = write_feedback_json(json!({
            "schema": COMPILER_FEEDBACK_SCHEMA,
            "range_hotspots": [{
                "target": "layer.0.ffn.activation",
                "current_safe_bound": 8.0,
                "observed_max_abs": 16.0
            }],
            "expert_slot_affinities": [{
                "layer": 0,
                "expert_a": 0,
                "expert_b": 1,
                "affinity_score_q8_8": 192
            }],
            "warnings": [{
                "code": "tight_margin",
                "message": "expert bank has only 96 bytes of slack"
            }]
        }));

        let report =
            apply_compiler_feedback_file(&path, CompilerFeedbackConfig::enabled(), &mut target)
                .unwrap();

        assert_eq!(report.status, FeedbackApplicationStatus::Applied);
        assert_eq!(target.range_updates.len(), 1);
        assert_eq!(target.range_updates[0].target, "layer.0.ffn.activation");
        assert_eq!(target.range_updates[0].previous_safe_bound, 8.0);
        assert_eq!(target.range_updates[0].observed_max_abs, 16.0);
        assert_eq!(target.range_updates[0].next_safe_bound, 8.8);
        assert_eq!(target.affinity_updates.len(), 1);
        assert_eq!(target.affinity_updates[0].layer, LayerId::new(0));
        assert_eq!(target.affinity_updates[0].expert_a, ExpertId::new(0));
        assert_eq!(target.affinity_updates[0].expert_b, ExpertId::new(1));
        assert_eq!(target.affinity_updates[0].affinity_score_q8_8, 192);
        assert_eq!(target.warnings[0].code, "tight_margin");
        assert_eq!(report.events.len(), 3);
        assert_eq!(target.events, report.events);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn compiler_feedback_rejects_invalid_schema_and_affinity_pair() {
        let mut target = RecordingFeedbackTarget::default();
        let invalid_schema = CompilerFeedback {
            schema: "compiler_feedback.v2".to_owned(),
            range_hotspots: Vec::new(),
            expert_slot_affinities: Vec::new(),
            warnings: Vec::new(),
        };
        assert!(matches!(
            apply_compiler_feedback(
                "memory",
                invalid_schema,
                CompilerFeedbackConfig::enabled(),
                &mut target
            ),
            Err(CompilerFeedbackError::InvalidSchema { .. })
        ));

        let invalid_pair = CompilerFeedback {
            schema: COMPILER_FEEDBACK_SCHEMA.to_owned(),
            range_hotspots: Vec::new(),
            expert_slot_affinities: vec![ExpertSlotAffinityFeedback {
                layer: LayerId::new(1),
                expert_a: ExpertId::new(2),
                expert_b: ExpertId::new(2),
                affinity_score_q8_8: 128,
            }],
            warnings: Vec::new(),
        };
        assert!(matches!(
            apply_compiler_feedback(
                "memory",
                invalid_pair,
                CompilerFeedbackConfig::enabled(),
                &mut target
            ),
            Err(CompilerFeedbackError::InvalidAffinityPair { .. })
        ));

        let invalid_score = CompilerFeedback {
            schema: COMPILER_FEEDBACK_SCHEMA.to_owned(),
            range_hotspots: Vec::new(),
            expert_slot_affinities: vec![ExpertSlotAffinityFeedback {
                layer: LayerId::new(1),
                expert_a: ExpertId::new(2),
                expert_b: ExpertId::new(3),
                affinity_score_q8_8: 257,
            }],
            warnings: Vec::new(),
        };
        assert!(matches!(
            apply_compiler_feedback(
                "memory",
                invalid_score,
                CompilerFeedbackConfig::enabled(),
                &mut target
            ),
            Err(CompilerFeedbackError::AffinityScoreOutOfRange { .. })
        ));
    }

    #[derive(Default)]
    struct RecordingFeedbackTarget {
        range_updates: Vec<ActivationRangeUpdate>,
        affinity_updates: Vec<ExpertSlotAffinityFeedback>,
        warnings: Vec<CompilerFeedbackWarning>,
        events: Vec<FeedbackApplicationEvent>,
    }

    impl CompilerFeedbackTarget for RecordingFeedbackTarget {
        fn update_activation_range_target(
            &mut self,
            update: &ActivationRangeUpdate,
        ) -> Result<(), CompilerFeedbackError> {
            self.range_updates.push(update.clone());
            Ok(())
        }

        fn update_expert_slot_affinity(
            &mut self,
            affinity: &ExpertSlotAffinityFeedback,
        ) -> Result<(), CompilerFeedbackError> {
            self.affinity_updates.push(affinity.clone());
            Ok(())
        }

        fn record_feedback_warning(
            &mut self,
            warning: &CompilerFeedbackWarning,
        ) -> Result<(), CompilerFeedbackError> {
            self.warnings.push(warning.clone());
            Ok(())
        }

        fn record_feedback_event(
            &mut self,
            event: &FeedbackApplicationEvent,
        ) -> Result<(), CompilerFeedbackError> {
            self.events.push(event.clone());
            Ok(())
        }
    }

    fn write_feedback_json(value: serde_json::Value) -> PathBuf {
        let path = temp_feedback_path("enabled");
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        path
    }

    fn temp_feedback_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "gbf-train-feedback-{label}-{}-{nanos}.json",
            std::process::id()
        ))
    }
}

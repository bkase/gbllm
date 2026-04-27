//! Structured training logging contracts.
//!
//! The public surface in this module keeps load-bearing data in typed fields
//! before it reaches `tracing`, and mirrors every emitted event into an
//! optional test collector for unit-level assertions.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tracing::Span;
use tracing_subscriber::EnvFilter;

pub const EVENT_NAME_SCALAR_METRIC: &str = "scalar_metric";
pub const EVENT_NAME_LOSS_STEP: &str = "loss_step";
pub const EVENT_NAME_PHASE_TRANSITION: &str = "phase_transition";
pub const EVENT_NAME_TEACHER_FREEZE: &str = "teacher_freeze";
pub const EVENT_NAME_EXPORT_COMPLETE: &str = "export_complete";
pub const EVENT_NAME_PREFLIGHT: &str = "preflight";
pub const EVENT_NAME_SHADOW_COMPILE: &str = "shadow_compile";
pub const DEFAULT_LOGGING_OVERHEAD_LIMIT: f64 = 0.01;
pub const PREFLIGHT_CHECK_EXPERT_SLOT_BUDGET: &str = "expert_slot_budget";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
    Off,
}

impl LogLevel {
    fn as_filter_directive(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Off => "off",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_filter_directive())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOutputFormat {
    #[default]
    Json,
    Human,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogDestination {
    #[default]
    Stderr,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrainingLoggingConfig {
    level: LogLevel,
    format: LogOutputFormat,
    destination: LogDestination,
    file_path: Option<PathBuf>,
    module_levels: BTreeMap<String, LogLevel>,
}

impl TrainingLoggingConfig {
    pub fn from_toml_str(input: &str) -> Result<Self, LoggingConfigError> {
        let value: toml::Value = toml::from_str(input).map_err(LoggingConfigError::Toml)?;
        let logging_value = if let Some(training) = value.get("training") {
            match training.get("logging") {
                Some(logging) => logging.clone(),
                None => return Ok(Self::default()),
            }
        } else {
            value
        };
        let config: Self = logging_value.try_into().map_err(LoggingConfigError::Toml)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), LoggingConfigError> {
        if self.destination == LogDestination::File && self.file_path.is_none() {
            return Err(LoggingConfigError::MissingFilePath);
        }

        if self.destination == LogDestination::Stderr && self.file_path.is_some() {
            return Err(LoggingConfigError::UnexpectedFilePathForStderr);
        }

        for module in self.module_levels.keys() {
            if module.trim().is_empty() {
                return Err(LoggingConfigError::EmptyModuleName);
            }
        }

        EnvFilter::try_new(self.filter_spec()).map_err(|error| {
            LoggingConfigError::InvalidFilterDirective {
                detail: error.to_string(),
            }
        })?;

        Ok(())
    }

    pub fn filter_spec(&self) -> String {
        let mut directives = Vec::with_capacity(self.module_levels.len() + 1);
        directives.push(self.level.as_filter_directive().to_owned());
        directives.extend(
            self.module_levels
                .iter()
                .map(|(module, level)| format!("{module}={level}")),
        );
        directives.join(",")
    }

    pub fn level(&self) -> LogLevel {
        self.level
    }

    pub fn format(&self) -> LogOutputFormat {
        self.format
    }

    pub fn destination(&self) -> LogDestination {
        self.destination
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    pub fn module_levels(&self) -> &BTreeMap<String, LogLevel> {
        &self.module_levels
    }
}

impl Default for TrainingLoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogOutputFormat::Json,
            destination: LogDestination::Stderr,
            file_path: None,
            module_levels: BTreeMap::new(),
        }
    }
}

pub fn init_logging(config: &TrainingLoggingConfig) -> Result<(), LoggingInitError> {
    config.validate().map_err(LoggingInitError::Config)?;
    let filter =
        EnvFilter::try_new(config.filter_spec()).map_err(|error| LoggingInitError::Filter {
            detail: error.to_string(),
        })?;

    match (config.destination(), config.format()) {
        (LogDestination::Stderr, LogOutputFormat::Json) => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
            .map_err(LoggingInitError::Subscriber),
        (LogDestination::Stderr, LogOutputFormat::Human) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
            .map_err(LoggingInitError::Subscriber),
        (LogDestination::File, LogOutputFormat::Json) => {
            let file = open_log_file(config)?;
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filter)
                .with_writer(move || file.try_clone().expect("training log file is clonable"))
                .try_init()
                .map_err(LoggingInitError::Subscriber)
        }
        (LogDestination::File, LogOutputFormat::Human) => {
            let file = open_log_file(config)?;
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(move || file.try_clone().expect("training log file is clonable"))
                .try_init()
                .map_err(LoggingInitError::Subscriber)
        }
    }
}

fn open_log_file(config: &TrainingLoggingConfig) -> Result<std::fs::File, LoggingInitError> {
    let path = config.file_path().ok_or(LoggingInitError::Config(
        LoggingConfigError::MissingFilePath,
    ))?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| LoggingInitError::Io {
            path: path.to_path_buf(),
            source,
        })
}

#[derive(Debug)]
pub enum LoggingConfigError {
    Toml(toml::de::Error),
    MissingFilePath,
    UnexpectedFilePathForStderr,
    EmptyModuleName,
    InvalidFilterDirective { detail: String },
}

impl fmt::Display for LoggingConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(error) => write!(f, "invalid training logging TOML: {error}"),
            Self::MissingFilePath => f.write_str("file log destination requires file_path"),
            Self::UnexpectedFilePathForStderr => {
                f.write_str("stderr log destination must not set file_path")
            }
            Self::EmptyModuleName => f.write_str("module log level entry has an empty module name"),
            Self::InvalidFilterDirective { detail } => {
                write!(f, "invalid tracing filter directive: {detail}")
            }
        }
    }
}

impl Error for LoggingConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Toml(error) => Some(error),
            Self::MissingFilePath
            | Self::UnexpectedFilePathForStderr
            | Self::EmptyModuleName
            | Self::InvalidFilterDirective { .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum LoggingInitError {
    Config(LoggingConfigError),
    Filter {
        detail: String,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Subscriber(Box<dyn Error + Send + Sync + 'static>),
}

impl fmt::Display for LoggingInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "{error}"),
            Self::Filter { detail } => write!(f, "invalid tracing filter: {detail}"),
            Self::Io { path, source } => {
                write!(
                    f,
                    "failed to open training log file {}: {source}",
                    path.display()
                )
            }
            Self::Subscriber(error) => {
                write!(f, "failed to initialize tracing subscriber: {error}")
            }
        }
    }
}

impl Error for LoggingInitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Subscriber(error) => Some(error.as_ref()),
            Self::Filter { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QatHardnessLevels {
    ternary: f32,
    activation: f32,
    norm: f32,
    router: f32,
    expert: f32,
}

impl QatHardnessLevels {
    pub fn new(
        ternary: f32,
        activation: f32,
        norm: f32,
        router: f32,
        expert: f32,
    ) -> Result<Self, LoggingEventError> {
        validate_hardness("ternary_hardness", ternary)?;
        validate_hardness("activation_hardness", activation)?;
        validate_hardness("norm_hardness", norm)?;
        validate_hardness("router_hardness", router)?;
        validate_hardness("expert_hardness", expert)?;

        Ok(Self {
            ternary,
            activation,
            norm,
            router,
            expert,
        })
    }

    pub fn ternary(self) -> f32 {
        self.ternary
    }

    pub fn activation(self) -> f32 {
        self.activation
    }

    pub fn norm(self) -> f32 {
        self.norm
    }

    pub fn router(self) -> f32 {
        self.router
    }

    pub fn expert(self) -> f32 {
        self.expert
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrainingPhaseSpanFields {
    phase_name: String,
    step_start: u64,
    step_end: u64,
    learning_rate: f32,
    hardness: QatHardnessLevels,
}

impl TrainingPhaseSpanFields {
    pub fn new(
        phase_name: impl Into<String>,
        step_start: u64,
        step_end: u64,
        learning_rate: f32,
        hardness: QatHardnessLevels,
    ) -> Result<Self, LoggingEventError> {
        let phase_name = phase_name.into();
        validate_nonempty("phase_name", &phase_name)?;
        if step_start > step_end {
            return Err(LoggingEventError::InvalidStepRange {
                step_start,
                step_end,
            });
        }
        validate_nonnegative_finite("learning_rate", learning_rate)?;

        Ok(Self {
            phase_name,
            step_start,
            step_end,
            learning_rate,
            hardness,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LossBreakdown {
    pub step: u64,
    pub lm_loss: f32,
    pub distill_loss: f32,
    pub balance_loss: f32,
    pub zrouter_loss: f32,
    pub switch_loss: f32,
    pub range_loss: f32,
    pub zero_loss: f32,
    pub shape_loss: f32,
    pub overflow_loss: f32,
    pub total_loss: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhaseTransitionEvent {
    pub from_phase: String,
    pub to_phase: String,
    pub step: u64,
    pub before_hardness: QatHardnessLevels,
    pub after_hardness: QatHardnessLevels,
    pub checkpoint_saved: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TeacherFreezeEvent {
    pub step: u64,
    pub teacher_checkpoint_id: String,
    pub source_weight_fingerprint: String,
    pub frozen_weight_fingerprint: String,
    pub weights_match: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportEvent {
    pub step: u64,
    pub artifact_core_hash: String,
    pub total_bytes: u64,
    pub n_experts: u32,
    pub ternary_weight_plan_summary: String,
    pub scale_bytes_total: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightStatus {
    Pass,
    Warn,
    Fail,
}

impl PreflightStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreflightEvent {
    pub check_name: String,
    pub status: PreflightStatus,
    pub detail: String,
    pub numeric_value: f32,
    pub threshold: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpertSlotPreflightEvent {
    detail: String,
    status: PreflightStatus,
    budget_computed: bool,
    expert_bytes: u64,
    expert_slot_usable_bytes: u64,
    slack_bytes: u64,
    over_by_bytes: u64,
}

impl ExpertSlotPreflightEvent {
    pub fn fits(
        detail: impl Into<String>,
        expert_bytes: u64,
        expert_slot_usable_bytes: u64,
        slack_bytes: u64,
    ) -> Result<Self, LoggingEventError> {
        Self::new(
            detail,
            PreflightStatus::Pass,
            true,
            expert_bytes,
            expert_slot_usable_bytes,
            slack_bytes,
            0,
        )
    }

    pub fn exceeds(
        detail: impl Into<String>,
        expert_bytes: u64,
        expert_slot_usable_bytes: u64,
        over_by_bytes: u64,
    ) -> Result<Self, LoggingEventError> {
        Self::new(
            detail,
            PreflightStatus::Fail,
            true,
            expert_bytes,
            expert_slot_usable_bytes,
            0,
            over_by_bytes,
        )
    }

    pub fn invalid(
        detail: impl Into<String>,
        expert_slot_usable_bytes: u64,
    ) -> Result<Self, LoggingEventError> {
        Self::new(
            detail,
            PreflightStatus::Fail,
            false,
            0,
            expert_slot_usable_bytes,
            0,
            0,
        )
    }

    fn new(
        detail: impl Into<String>,
        status: PreflightStatus,
        budget_computed: bool,
        expert_bytes: u64,
        expert_slot_usable_bytes: u64,
        slack_bytes: u64,
        over_by_bytes: u64,
    ) -> Result<Self, LoggingEventError> {
        let detail = detail.into();
        validate_nonempty("detail", &detail)?;
        Ok(Self {
            detail,
            status,
            budget_computed,
            expert_bytes,
            expert_slot_usable_bytes,
            slack_bytes,
            over_by_bytes,
        })
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub const fn status(&self) -> PreflightStatus {
        self.status
    }

    pub const fn budget_computed(&self) -> bool {
        self.budget_computed
    }

    pub const fn expert_bytes(&self) -> u64 {
        self.expert_bytes
    }

    pub const fn expert_slot_usable_bytes(&self) -> u64 {
        self.expert_slot_usable_bytes
    }

    pub const fn slack_bytes(&self) -> u64 {
        self.slack_bytes
    }

    pub const fn over_by_bytes(&self) -> u64 {
        self.over_by_bytes
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadowCompileEvent {
    pub step: u64,
    pub checkpoint_id: String,
    pub compile_profile: String,
    pub fit_status: String,
    pub quality_summary: String,
    pub frontier_size: u32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TrainingLogEmitter {
    test_collector: Option<TestEventCollector>,
}

impl TrainingLogEmitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_test_collector(test_collector: TestEventCollector) -> Self {
        Self {
            test_collector: Some(test_collector),
        }
    }

    pub fn training_phase_span(
        &self,
        fields: &TrainingPhaseSpanFields,
    ) -> Result<Span, LoggingEventError> {
        let span = tracing::info_span!(
            "training_phase",
            phase_name = %fields.phase_name,
            step_start = fields.step_start,
            step_end = fields.step_end,
            learning_rate = fields.learning_rate,
            ternary_hardness = fields.hardness.ternary(),
            activation_hardness = fields.hardness.activation(),
            norm_hardness = fields.hardness.norm(),
            router_hardness = fields.hardness.router(),
            expert_hardness = fields.hardness.expert(),
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::TrainingPhaseSpan,
            LogLevel::Info,
            fields_map([
                (
                    "phase_name",
                    TestFieldValue::String(fields.phase_name.clone()),
                ),
                ("step_start", TestFieldValue::U64(fields.step_start)),
                ("step_end", TestFieldValue::U64(fields.step_end)),
                ("learning_rate", TestFieldValue::F32(fields.learning_rate)),
                (
                    "ternary_hardness",
                    TestFieldValue::F32(fields.hardness.ternary()),
                ),
                (
                    "activation_hardness",
                    TestFieldValue::F32(fields.hardness.activation()),
                ),
                ("norm_hardness", TestFieldValue::F32(fields.hardness.norm())),
                (
                    "router_hardness",
                    TestFieldValue::F32(fields.hardness.router()),
                ),
                (
                    "expert_hardness",
                    TestFieldValue::F32(fields.hardness.expert()),
                ),
            ]),
        ));

        Ok(span)
    }

    pub fn loss_step(&self, fields: &LossBreakdown) -> Result<(), LoggingEventError> {
        validate_loss_breakdown(fields)?;

        tracing::info!(
            event_name = EVENT_NAME_LOSS_STEP,
            step = fields.step,
            lm_loss = fields.lm_loss,
            distill_loss = fields.distill_loss,
            balance_loss = fields.balance_loss,
            zrouter_loss = fields.zrouter_loss,
            switch_loss = fields.switch_loss,
            range_loss = fields.range_loss,
            zero_loss = fields.zero_loss,
            shape_loss = fields.shape_loss,
            overflow_loss = fields.overflow_loss,
            total_loss = fields.total_loss,
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::LossStep,
            LogLevel::Info,
            fields_map([
                ("step", TestFieldValue::U64(fields.step)),
                ("lm_loss", TestFieldValue::F32(fields.lm_loss)),
                ("distill_loss", TestFieldValue::F32(fields.distill_loss)),
                ("balance_loss", TestFieldValue::F32(fields.balance_loss)),
                ("zrouter_loss", TestFieldValue::F32(fields.zrouter_loss)),
                ("switch_loss", TestFieldValue::F32(fields.switch_loss)),
                ("range_loss", TestFieldValue::F32(fields.range_loss)),
                ("zero_loss", TestFieldValue::F32(fields.zero_loss)),
                ("shape_loss", TestFieldValue::F32(fields.shape_loss)),
                ("overflow_loss", TestFieldValue::F32(fields.overflow_loss)),
                ("total_loss", TestFieldValue::F32(fields.total_loss)),
            ]),
        ));

        Ok(())
    }

    pub fn loss_breakdown(&self, fields: &LossBreakdown) -> Result<(), LoggingEventError> {
        self.loss_step(fields)
    }

    pub fn phase_transition(&self, fields: &PhaseTransitionEvent) -> Result<(), LoggingEventError> {
        validate_nonempty("from_phase", &fields.from_phase)?;
        validate_nonempty("to_phase", &fields.to_phase)?;

        tracing::info!(
            event_name = EVENT_NAME_PHASE_TRANSITION,
            from_phase = %fields.from_phase,
            to_phase = %fields.to_phase,
            step = fields.step,
            before_ternary_hardness = fields.before_hardness.ternary(),
            before_activation_hardness = fields.before_hardness.activation(),
            before_norm_hardness = fields.before_hardness.norm(),
            before_router_hardness = fields.before_hardness.router(),
            before_expert_hardness = fields.before_hardness.expert(),
            after_ternary_hardness = fields.after_hardness.ternary(),
            after_activation_hardness = fields.after_hardness.activation(),
            after_norm_hardness = fields.after_hardness.norm(),
            after_router_hardness = fields.after_hardness.router(),
            after_expert_hardness = fields.after_hardness.expert(),
            checkpoint_saved = fields.checkpoint_saved,
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::PhaseTransition,
            LogLevel::Info,
            fields_map([
                (
                    "from_phase",
                    TestFieldValue::String(fields.from_phase.clone()),
                ),
                ("to_phase", TestFieldValue::String(fields.to_phase.clone())),
                ("step", TestFieldValue::U64(fields.step)),
                (
                    "before_ternary_hardness",
                    TestFieldValue::F32(fields.before_hardness.ternary()),
                ),
                (
                    "before_activation_hardness",
                    TestFieldValue::F32(fields.before_hardness.activation()),
                ),
                (
                    "before_norm_hardness",
                    TestFieldValue::F32(fields.before_hardness.norm()),
                ),
                (
                    "before_router_hardness",
                    TestFieldValue::F32(fields.before_hardness.router()),
                ),
                (
                    "before_expert_hardness",
                    TestFieldValue::F32(fields.before_hardness.expert()),
                ),
                (
                    "after_ternary_hardness",
                    TestFieldValue::F32(fields.after_hardness.ternary()),
                ),
                (
                    "after_activation_hardness",
                    TestFieldValue::F32(fields.after_hardness.activation()),
                ),
                (
                    "after_norm_hardness",
                    TestFieldValue::F32(fields.after_hardness.norm()),
                ),
                (
                    "after_router_hardness",
                    TestFieldValue::F32(fields.after_hardness.router()),
                ),
                (
                    "after_expert_hardness",
                    TestFieldValue::F32(fields.after_hardness.expert()),
                ),
                (
                    "checkpoint_saved",
                    TestFieldValue::Bool(fields.checkpoint_saved),
                ),
            ]),
        ));

        Ok(())
    }

    pub fn teacher_freeze(&self, fields: &TeacherFreezeEvent) -> Result<(), LoggingEventError> {
        validate_nonempty("teacher_checkpoint_id", &fields.teacher_checkpoint_id)?;
        validate_nonempty(
            "source_weight_fingerprint",
            &fields.source_weight_fingerprint,
        )?;
        validate_nonempty(
            "frozen_weight_fingerprint",
            &fields.frozen_weight_fingerprint,
        )?;

        tracing::info!(
            event_name = EVENT_NAME_TEACHER_FREEZE,
            step = fields.step,
            teacher_checkpoint_id = %fields.teacher_checkpoint_id,
            source_weight_fingerprint = %fields.source_weight_fingerprint,
            frozen_weight_fingerprint = %fields.frozen_weight_fingerprint,
            weights_match = fields.weights_match,
            duration_ms = fields.duration_ms,
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::TeacherFreeze,
            LogLevel::Info,
            fields_map([
                ("step", TestFieldValue::U64(fields.step)),
                (
                    "teacher_checkpoint_id",
                    TestFieldValue::String(fields.teacher_checkpoint_id.clone()),
                ),
                (
                    "source_weight_fingerprint",
                    TestFieldValue::String(fields.source_weight_fingerprint.clone()),
                ),
                (
                    "frozen_weight_fingerprint",
                    TestFieldValue::String(fields.frozen_weight_fingerprint.clone()),
                ),
                ("weights_match", TestFieldValue::Bool(fields.weights_match)),
                ("duration_ms", TestFieldValue::U64(fields.duration_ms)),
            ]),
        ));

        Ok(())
    }

    pub fn export_complete(&self, fields: &ExportEvent) -> Result<(), LoggingEventError> {
        validate_nonempty("artifact_core_hash", &fields.artifact_core_hash)?;
        validate_nonempty(
            "ternary_weight_plan_summary",
            &fields.ternary_weight_plan_summary,
        )?;
        if fields.n_experts == 0 {
            return Err(LoggingEventError::ZeroField { name: "n_experts" });
        }

        tracing::info!(
            event_name = EVENT_NAME_EXPORT_COMPLETE,
            step = fields.step,
            artifact_core_hash = %fields.artifact_core_hash,
            total_bytes = fields.total_bytes,
            n_experts = fields.n_experts,
            ternary_weight_plan_summary = %fields.ternary_weight_plan_summary,
            scale_bytes_total = fields.scale_bytes_total,
            duration_ms = fields.duration_ms,
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::ExportComplete,
            LogLevel::Info,
            fields_map([
                ("step", TestFieldValue::U64(fields.step)),
                (
                    "artifact_core_hash",
                    TestFieldValue::String(fields.artifact_core_hash.clone()),
                ),
                ("total_bytes", TestFieldValue::U64(fields.total_bytes)),
                (
                    "n_experts",
                    TestFieldValue::U64(u64::from(fields.n_experts)),
                ),
                (
                    "ternary_weight_plan_summary",
                    TestFieldValue::String(fields.ternary_weight_plan_summary.clone()),
                ),
                (
                    "scale_bytes_total",
                    TestFieldValue::U64(fields.scale_bytes_total),
                ),
                ("duration_ms", TestFieldValue::U64(fields.duration_ms)),
            ]),
        ));

        Ok(())
    }

    pub fn export(&self, fields: &ExportEvent) -> Result<(), LoggingEventError> {
        self.export_complete(fields)
    }

    pub fn preflight(&self, fields: &PreflightEvent) -> Result<(), LoggingEventError> {
        validate_nonempty("check_name", &fields.check_name)?;
        validate_nonempty("detail", &fields.detail)?;
        validate_finite("numeric_value", fields.numeric_value)?;
        validate_finite("threshold", fields.threshold)?;

        tracing::info!(
            event_name = EVENT_NAME_PREFLIGHT,
            check_name = %fields.check_name,
            status = fields.status.as_str(),
            detail = %fields.detail,
            numeric_value = fields.numeric_value,
            threshold = fields.threshold,
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::Preflight,
            LogLevel::Info,
            fields_map([
                (
                    "check_name",
                    TestFieldValue::String(fields.check_name.clone()),
                ),
                (
                    "status",
                    TestFieldValue::String(fields.status.as_str().to_owned()),
                ),
                ("detail", TestFieldValue::String(fields.detail.clone())),
                ("numeric_value", TestFieldValue::F32(fields.numeric_value)),
                ("threshold", TestFieldValue::F32(fields.threshold)),
            ]),
        ));

        Ok(())
    }

    pub fn expert_slot_preflight(
        &self,
        fields: &ExpertSlotPreflightEvent,
    ) -> Result<(), LoggingEventError> {
        validate_nonempty("detail", fields.detail())?;

        tracing::info!(
            event_name = EVENT_NAME_PREFLIGHT,
            check_name = PREFLIGHT_CHECK_EXPERT_SLOT_BUDGET,
            status = fields.status().as_str(),
            detail = %fields.detail(),
            budget_computed = fields.budget_computed(),
            expert_bytes = fields.expert_bytes(),
            expert_slot_usable_bytes = fields.expert_slot_usable_bytes(),
            slack_bytes = fields.slack_bytes(),
            over_by_bytes = fields.over_by_bytes(),
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::Preflight,
            LogLevel::Info,
            fields_map([
                (
                    "check_name",
                    TestFieldValue::String(PREFLIGHT_CHECK_EXPERT_SLOT_BUDGET.to_owned()),
                ),
                (
                    "status",
                    TestFieldValue::String(fields.status().as_str().to_owned()),
                ),
                ("detail", TestFieldValue::String(fields.detail().to_owned())),
                (
                    "budget_computed",
                    TestFieldValue::Bool(fields.budget_computed()),
                ),
                ("expert_bytes", TestFieldValue::U64(fields.expert_bytes())),
                (
                    "expert_slot_usable_bytes",
                    TestFieldValue::U64(fields.expert_slot_usable_bytes()),
                ),
                ("slack_bytes", TestFieldValue::U64(fields.slack_bytes())),
                ("over_by_bytes", TestFieldValue::U64(fields.over_by_bytes())),
            ]),
        ));

        Ok(())
    }

    pub fn shadow_compile(&self, fields: &ShadowCompileEvent) -> Result<(), LoggingEventError> {
        validate_nonempty("checkpoint_id", &fields.checkpoint_id)?;
        validate_nonempty("compile_profile", &fields.compile_profile)?;
        validate_nonempty("fit_status", &fields.fit_status)?;
        validate_nonempty("quality_summary", &fields.quality_summary)?;

        tracing::info!(
            event_name = EVENT_NAME_SHADOW_COMPILE,
            step = fields.step,
            checkpoint_id = %fields.checkpoint_id,
            compile_profile = %fields.compile_profile,
            fit_status = %fields.fit_status,
            quality_summary = %fields.quality_summary,
            frontier_size = fields.frontier_size,
            duration_ms = fields.duration_ms,
        );

        self.record_test_event(TestEvent::new(
            TestEventKind::ShadowCompile,
            LogLevel::Info,
            fields_map([
                ("step", TestFieldValue::U64(fields.step)),
                (
                    "checkpoint_id",
                    TestFieldValue::String(fields.checkpoint_id.clone()),
                ),
                (
                    "compile_profile",
                    TestFieldValue::String(fields.compile_profile.clone()),
                ),
                (
                    "fit_status",
                    TestFieldValue::String(fields.fit_status.clone()),
                ),
                (
                    "quality_summary",
                    TestFieldValue::String(fields.quality_summary.clone()),
                ),
                (
                    "frontier_size",
                    TestFieldValue::U64(u64::from(fields.frontier_size)),
                ),
                ("duration_ms", TestFieldValue::U64(fields.duration_ms)),
            ]),
        ));

        Ok(())
    }

    fn record_test_event(&self, event: TestEvent) {
        if let Some(collector) = &self.test_collector {
            collector.record(event);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TestEventCollector {
    events: Arc<Mutex<Vec<TestEvent>>>,
}

impl TestEventCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<TestEvent> {
        self.events
            .lock()
            .expect("test event collector mutex is not poisoned")
            .clone()
    }

    pub fn clear(&self) {
        self.events
            .lock()
            .expect("test event collector mutex is not poisoned")
            .clear();
    }

    fn record(&self, event: TestEvent) {
        self.events
            .lock()
            .expect("test event collector mutex is not poisoned")
            .push(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestEventKind {
    TrainingPhaseSpan,
    LossStep,
    PhaseTransition,
    TeacherFreeze,
    ExportComplete,
    Preflight,
    ShadowCompile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TestFieldValue {
    String(String),
    Bool(bool),
    U64(u64),
    F32(f32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestEvent {
    kind: TestEventKind,
    level: LogLevel,
    fields: BTreeMap<String, TestFieldValue>,
}

impl TestEvent {
    fn new(kind: TestEventKind, level: LogLevel, fields: BTreeMap<String, TestFieldValue>) -> Self {
        Self {
            kind,
            level,
            fields,
        }
    }

    pub fn kind(&self) -> TestEventKind {
        self.kind
    }

    pub fn level(&self) -> LogLevel {
        self.level
    }

    pub fn fields(&self) -> &BTreeMap<String, TestFieldValue> {
        &self.fields
    }

    pub fn field(&self, name: &str) -> Option<&TestFieldValue> {
        self.fields.get(name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoggingEventError {
    EmptyField { name: &'static str },
    ZeroField { name: &'static str },
    NonFiniteField { name: &'static str, value: f32 },
    NegativeField { name: &'static str, value: f32 },
    HardnessOutOfRange { name: &'static str, value: f32 },
    InvalidStepRange { step_start: u64, step_end: u64 },
}

impl fmt::Display for LoggingEventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { name } => write!(f, "{name} must not be empty"),
            Self::ZeroField { name } => write!(f, "{name} must be greater than zero"),
            Self::NonFiniteField { name, value } => {
                write!(f, "{name} must be finite, got {value}")
            }
            Self::NegativeField { name, value } => {
                write!(f, "{name} must be non-negative, got {value}")
            }
            Self::HardnessOutOfRange { name, value } => {
                write!(f, "{name} must be within [0, 1], got {value}")
            }
            Self::InvalidStepRange {
                step_start,
                step_end,
            } => write!(
                f,
                "training phase step_start {step_start} must be <= step_end {step_end}"
            ),
        }
    }
}

impl Error for LoggingEventError {}

/// Explicit timing input for the logging overhead gate.
///
/// This type deliberately does not measure anything by itself. Callers must
/// feed it values from a real baseline-vs-instrumented workload before making
/// an overhead claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoggingOverheadMeasurement {
    baseline_step_ns: u64,
    instrumented_step_ns: u64,
}

impl LoggingOverheadMeasurement {
    pub fn new(
        baseline_step_ns: u64,
        instrumented_step_ns: u64,
    ) -> Result<Self, LoggingOverheadGateError> {
        if baseline_step_ns == 0 {
            return Err(LoggingOverheadGateError::ZeroBaseline);
        }

        Ok(Self {
            baseline_step_ns,
            instrumented_step_ns,
        })
    }

    pub fn baseline_step_ns(self) -> u64 {
        self.baseline_step_ns
    }

    pub fn instrumented_step_ns(self) -> u64 {
        self.instrumented_step_ns
    }

    pub fn overhead_fraction(self) -> f64 {
        if self.instrumented_step_ns <= self.baseline_step_ns {
            return 0.0;
        }

        (self.instrumented_step_ns - self.baseline_step_ns) as f64 / self.baseline_step_ns as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoggingOverheadStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoggingOverheadReport {
    measurement: LoggingOverheadMeasurement,
    overhead_fraction: f64,
    max_overhead_fraction: f64,
    status: LoggingOverheadStatus,
}

impl LoggingOverheadReport {
    pub fn measurement(self) -> LoggingOverheadMeasurement {
        self.measurement
    }

    pub fn overhead_fraction(self) -> f64 {
        self.overhead_fraction
    }

    pub fn max_overhead_fraction(self) -> f64 {
        self.max_overhead_fraction
    }

    pub fn status(self) -> LoggingOverheadStatus {
        self.status
    }

    pub fn passes(self) -> bool {
        self.status == LoggingOverheadStatus::Pass
    }
}

/// Compares measured logging overhead against a configured limit.
///
/// Unit tests cover the arithmetic contract; benchmark ownership belongs to
/// the workload that produces `LoggingOverheadMeasurement`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoggingOverheadGate {
    max_overhead_fraction: f64,
}

impl LoggingOverheadGate {
    pub fn new(max_overhead_fraction: f64) -> Result<Self, LoggingOverheadGateError> {
        if !max_overhead_fraction.is_finite() || max_overhead_fraction < 0.0 {
            return Err(LoggingOverheadGateError::InvalidMaxOverheadFraction {
                value: max_overhead_fraction,
            });
        }

        Ok(Self {
            max_overhead_fraction,
        })
    }

    pub fn constitution_one_percent() -> Self {
        Self::new(DEFAULT_LOGGING_OVERHEAD_LIMIT)
            .expect("default logging overhead limit is finite and non-negative")
    }

    pub fn max_overhead_fraction(self) -> f64 {
        self.max_overhead_fraction
    }

    pub fn evaluate(self, measurement: LoggingOverheadMeasurement) -> LoggingOverheadReport {
        let overhead_fraction = measurement.overhead_fraction();
        let status = if overhead_fraction <= self.max_overhead_fraction {
            LoggingOverheadStatus::Pass
        } else {
            LoggingOverheadStatus::Fail
        };

        LoggingOverheadReport {
            measurement,
            overhead_fraction,
            max_overhead_fraction: self.max_overhead_fraction,
            status,
        }
    }

    pub fn require_pass(
        self,
        measurement: LoggingOverheadMeasurement,
    ) -> Result<LoggingOverheadReport, LoggingOverheadGateError> {
        let report = self.evaluate(measurement);
        if report.passes() {
            return Ok(report);
        }

        Err(LoggingOverheadGateError::Exceeded {
            measured_fraction: report.overhead_fraction(),
            max_fraction: report.max_overhead_fraction(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoggingOverheadGateError {
    ZeroBaseline,
    InvalidMaxOverheadFraction {
        value: f64,
    },
    Exceeded {
        measured_fraction: f64,
        max_fraction: f64,
    },
}

impl fmt::Display for LoggingOverheadGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroBaseline => {
                f.write_str("logging overhead measurement baseline must be nonzero")
            }
            Self::InvalidMaxOverheadFraction { value } => write!(
                f,
                "logging overhead gate limit must be finite and non-negative, got {value}"
            ),
            Self::Exceeded {
                measured_fraction,
                max_fraction,
            } => write!(
                f,
                "logging overhead {measured_fraction:.6} exceeds limit {max_fraction:.6}"
            ),
        }
    }
}

impl Error for LoggingOverheadGateError {}

fn fields_map<const N: usize>(
    entries: [(&'static str, TestFieldValue); N],
) -> BTreeMap<String, TestFieldValue> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn validate_loss_breakdown(fields: &LossBreakdown) -> Result<(), LoggingEventError> {
    validate_finite("lm_loss", fields.lm_loss)?;
    validate_finite("distill_loss", fields.distill_loss)?;
    validate_finite("balance_loss", fields.balance_loss)?;
    validate_finite("zrouter_loss", fields.zrouter_loss)?;
    validate_finite("switch_loss", fields.switch_loss)?;
    validate_finite("range_loss", fields.range_loss)?;
    validate_finite("zero_loss", fields.zero_loss)?;
    validate_finite("shape_loss", fields.shape_loss)?;
    validate_finite("overflow_loss", fields.overflow_loss)?;
    validate_finite("total_loss", fields.total_loss)
}

fn validate_nonempty(name: &'static str, value: &str) -> Result<(), LoggingEventError> {
    if value.trim().is_empty() {
        return Err(LoggingEventError::EmptyField { name });
    }
    Ok(())
}

fn validate_hardness(name: &'static str, value: f32) -> Result<(), LoggingEventError> {
    validate_finite(name, value)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(LoggingEventError::HardnessOutOfRange { name, value });
    }
    Ok(())
}

fn validate_nonnegative_finite(name: &'static str, value: f32) -> Result<(), LoggingEventError> {
    validate_finite(name, value)?;
    if value < 0.0 {
        return Err(LoggingEventError::NegativeField { name, value });
    }
    Ok(())
}

fn validate_finite(name: &'static str, value: f32) -> Result<(), LoggingEventError> {
    if !value.is_finite() {
        return Err(LoggingEventError::NonFiniteField { name, value });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::prelude::*;

    #[test]
    fn logging_config_parses_training_toml_and_builds_filter_spec() {
        let config = TrainingLoggingConfig::from_toml_str(
            r#"
            [training.logging]
            level = "debug"
            format = "human"
            destination = "file"
            file_path = "logs/train.log"

            [training.logging.module_levels]
            "gbf_train::preflight" = "warn"
            "gbf_train::qat" = "trace"
            "#,
        )
        .unwrap();

        assert_eq!(config.level(), LogLevel::Debug);
        assert_eq!(config.format(), LogOutputFormat::Human);
        assert_eq!(config.destination(), LogDestination::File);
        assert_eq!(
            config.file_path(),
            Some(PathBuf::from("logs/train.log").as_path())
        );
        assert_eq!(
            config.filter_spec(),
            "debug,gbf_train::preflight=warn,gbf_train::qat=trace"
        );
    }

    #[test]
    fn logging_config_rejects_ambiguous_routing() {
        let missing_file = TrainingLoggingConfig::from_toml_str(
            r#"
            [training.logging]
            destination = "file"
            "#,
        )
        .unwrap_err();
        assert!(matches!(missing_file, LoggingConfigError::MissingFilePath));

        let unexpected_file = TrainingLoggingConfig::from_toml_str(
            r#"
            [training.logging]
            destination = "stderr"
            file_path = "train.log"
            "#,
        )
        .unwrap_err();
        assert!(matches!(
            unexpected_file,
            LoggingConfigError::UnexpectedFilePathForStderr
        ));
    }

    #[test]
    fn logging_config_defaults_when_training_toml_omits_logging_section() {
        let config = TrainingLoggingConfig::from_toml_str(
            r#"
            [training]
            seed = 42
            "#,
        )
        .unwrap();

        assert_eq!(config, TrainingLoggingConfig::default());
    }

    #[test]
    fn test_collector_captures_training_phase_span_fields() {
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());
        let hardness = QatHardnessLevels::new(0.0, 0.25, 0.4, 0.5, 1.0).unwrap();
        let span_fields =
            TrainingPhaseSpanFields::new("qat_warmup", 10, 20, 0.001, hardness).unwrap();

        let _span = emitter.training_phase_span(&span_fields).unwrap();

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), TestEventKind::TrainingPhaseSpan);
        assert_eq!(
            events[0].field("phase_name"),
            Some(&TestFieldValue::String("qat_warmup".to_owned()))
        );
        assert_eq!(
            events[0].field("router_hardness"),
            Some(&TestFieldValue::F32(0.5))
        );
        assert_eq!(
            events[0].field("norm_hardness"),
            Some(&TestFieldValue::F32(0.4))
        );
    }

    #[test]
    fn loss_step_is_structured_and_typed() {
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        emitter.loss_step(&sample_loss_breakdown()).unwrap();

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), TestEventKind::LossStep);
        assert_eq!(events[0].field("step"), Some(&TestFieldValue::U64(7)));
        assert_eq!(
            events[0].field("zrouter_loss"),
            Some(&TestFieldValue::F32(0.04))
        );
        assert_eq!(
            events[0].field("total_loss"),
            Some(&TestFieldValue::F32(1.33))
        );
    }

    #[test]
    fn event_validation_rejects_non_finite_loss_without_capture() {
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());
        let mut loss = sample_loss_breakdown();
        loss.overflow_loss = f32::INFINITY;

        let error = emitter.loss_step(&loss).unwrap_err();

        assert_eq!(
            error,
            LoggingEventError::NonFiniteField {
                name: "overflow_loss",
                value: f32::INFINITY,
            }
        );
        assert!(collector.events().is_empty());
    }

    #[test]
    fn phase_transition_records_before_after_hardness_and_checkpoint() {
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        emitter
            .phase_transition(&PhaseTransitionEvent {
                from_phase: "soft".to_owned(),
                to_phase: "hard".to_owned(),
                step: 99,
                before_hardness: QatHardnessLevels::new(0.2, 0.3, 0.35, 0.4, 0.5).unwrap(),
                after_hardness: QatHardnessLevels::new(1.0, 1.0, 0.9, 0.8, 0.8).unwrap(),
                checkpoint_saved: true,
            })
            .unwrap();

        let events = collector.events();
        assert_eq!(events[0].kind(), TestEventKind::PhaseTransition);
        assert_eq!(
            events[0].field("after_ternary_hardness"),
            Some(&TestFieldValue::F32(1.0))
        );
        assert_eq!(
            events[0].field("after_norm_hardness"),
            Some(&TestFieldValue::F32(0.9))
        );
        assert_eq!(
            events[0].field("checkpoint_saved"),
            Some(&TestFieldValue::Bool(true))
        );
    }

    #[test]
    fn teacher_freeze_export_preflight_and_shadow_compile_events_cover_required_fields() {
        let collector = TestEventCollector::new();
        let emitter = TrainingLogEmitter::with_test_collector(collector.clone());

        emitter
            .teacher_freeze(&TeacherFreezeEvent {
                step: 11,
                teacher_checkpoint_id: "teacher-11".to_owned(),
                source_weight_fingerprint: "010203".to_owned(),
                frozen_weight_fingerprint: "010203".to_owned(),
                weights_match: true,
                duration_ms: 5,
            })
            .unwrap();
        emitter
            .export_complete(&ExportEvent {
                step: 12,
                artifact_core_hash: "0123456789abcdef".to_owned(),
                total_bytes: 4096,
                n_experts: 2,
                ternary_weight_plan_summary: "ternary2/per_output_row/q8_8".to_owned(),
                scale_bytes_total: 128,
                duration_ms: 17,
            })
            .unwrap();
        emitter
            .preflight(&PreflightEvent {
                check_name: "expert_bank_fit".to_owned(),
                status: PreflightStatus::Warn,
                detail: "fits with low slack".to_owned(),
                numeric_value: 15_090.0,
                threshold: 16_384.0,
            })
            .unwrap();
        emitter
            .shadow_compile(&ShadowCompileEvent {
                step: 12,
                checkpoint_id: "ckpt-12".to_owned(),
                compile_profile: "tiny-ci".to_owned(),
                fit_status: "fits".to_owned(),
                quality_summary: "frontier stable".to_owned(),
                frontier_size: 3,
                duration_ms: 42,
            })
            .unwrap();

        let events = collector.events();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].kind(), TestEventKind::TeacherFreeze);
        assert_eq!(
            events[0].field("teacher_checkpoint_id"),
            Some(&TestFieldValue::String("teacher-11".to_owned()))
        );
        assert_eq!(
            events[0].field("weights_match"),
            Some(&TestFieldValue::Bool(true))
        );
        assert_eq!(events[1].kind(), TestEventKind::ExportComplete);
        assert_eq!(
            events[1].field("artifact_core_hash"),
            Some(&TestFieldValue::String("0123456789abcdef".to_owned()))
        );
        assert_eq!(events[2].kind(), TestEventKind::Preflight);
        assert_eq!(
            events[2].field("status"),
            Some(&TestFieldValue::String("warn".to_owned()))
        );
        assert_eq!(events[3].kind(), TestEventKind::ShadowCompile);
        assert_eq!(
            events[3].field("frontier_size"),
            Some(&TestFieldValue::U64(3))
        );
    }

    #[test]
    fn logging_overhead_gate_requires_measurement_and_enforces_one_percent_limit() {
        assert_eq!(
            LoggingOverheadMeasurement::new(0, 1).unwrap_err(),
            LoggingOverheadGateError::ZeroBaseline
        );
        assert!(matches!(
            LoggingOverheadGate::new(f64::NAN).unwrap_err(),
            LoggingOverheadGateError::InvalidMaxOverheadFraction { .. }
        ));

        let gate = LoggingOverheadGate::constitution_one_percent();
        let passing = gate
            .require_pass(LoggingOverheadMeasurement::new(10_000, 10_050).unwrap())
            .unwrap();
        assert_eq!(passing.status(), LoggingOverheadStatus::Pass);
        assert_eq!(passing.overhead_fraction(), 0.005);

        let failing = gate
            .require_pass(LoggingOverheadMeasurement::new(10_000, 10_200).unwrap())
            .unwrap_err();
        assert_eq!(
            failing,
            LoggingOverheadGateError::Exceeded {
                measured_fraction: 0.02,
                max_fraction: DEFAULT_LOGGING_OVERHEAD_LIMIT,
            }
        );
    }

    #[test]
    fn actual_tracing_subscriber_observes_training_phase_span_fields() {
        let capture = TraceCapture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        let hardness = QatHardnessLevels::new(0.0, 0.25, 0.4, 0.5, 1.0).unwrap();
        let span_fields =
            TrainingPhaseSpanFields::new("qat_warmup", 10, 20, 0.001, hardness).unwrap();

        tracing::subscriber::with_default(subscriber, || {
            let emitter = TrainingLogEmitter::new();
            let _span = emitter.training_phase_span(&span_fields).unwrap();
        });

        let records = capture.records();
        let phase_span = records
            .iter()
            .find(|record| record.kind == TraceRecordKind::Span && record.name == "training_phase")
            .expect("training_phase span should be captured by tracing subscriber");
        assert_eq!(phase_span.field("phase_name"), Some("qat_warmup"));
        assert_eq!(phase_span.field("step_start"), Some("10"));
        assert_eq!(phase_span.field("router_hardness"), Some("0.5"));

        // Structured event fields are covered by the integration test process,
        // where tracing callsites cannot be pre-registered by sibling unit
        // tests before the capture subscriber is installed.
    }

    #[test]
    fn structured_logging_source_avoids_message_strings_on_required_events() {
        let source = include_str!("logging.rs");
        let burn_source = include_str!("adapter/burn.rs");
        for event_name in [
            EVENT_NAME_SCALAR_METRIC,
            EVENT_NAME_LOSS_STEP,
            EVENT_NAME_PHASE_TRANSITION,
            EVENT_NAME_TEACHER_FREEZE,
            EVENT_NAME_EXPORT_COMPLETE,
            EVENT_NAME_PREFLIGHT,
            EVENT_NAME_SHADOW_COMPILE,
        ] {
            assert!(
                source.contains(event_name),
                "missing structured event name constant for {event_name}"
            );
        }
        assert!(burn_source.contains("EVENT_NAME_SCALAR_METRIC"));

        assert!(!source.contains("\"loss breakdown\""));
        assert!(!source.contains("\"phase transition\""));
        assert!(!source.contains("\"export event\""));
        assert!(!burn_source.contains("\"training scalar metric\""));
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TraceRecordKind {
        Event,
        Span,
    }

    #[derive(Debug, Clone)]
    struct TraceRecord {
        kind: TraceRecordKind,
        name: String,
        fields: BTreeMap<String, String>,
    }

    impl TraceRecord {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    #[derive(Debug, Clone, Default)]
    struct TraceCapture {
        records: Arc<Mutex<Vec<TraceRecord>>>,
    }

    impl TraceCapture {
        fn records(&self) -> Vec<TraceRecord> {
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .clone()
        }
    }

    impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
    where
        S: tracing::Subscriber,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            attrs.record(&mut visitor);
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .push(TraceRecord {
                    kind: TraceRecordKind::Span,
                    name: attrs.metadata().name().to_owned(),
                    fields: visitor.fields,
                });
        }

        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            event.record(&mut visitor);
            self.records
                .lock()
                .expect("trace capture mutex is not poisoned")
                .push(TraceRecord {
                    kind: TraceRecordKind::Event,
                    name: event.metadata().name().to_owned(),
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Debug, Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl TraceFieldVisitor {
        fn insert(&mut self, field: &tracing::field::Field, value: String) {
            self.fields.insert(field.name().to_owned(), value);
        }
    }

    impl tracing::field::Visit for TraceFieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.insert(field, format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.insert(field, value.to_owned());
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.insert(field, value.to_string());
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.insert(field, value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.insert(field, value.to_string());
        }

        fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
            self.insert(field, value.to_string());
        }
    }

    fn sample_loss_breakdown() -> LossBreakdown {
        LossBreakdown {
            step: 7,
            lm_loss: 1.0,
            distill_loss: 0.1,
            balance_loss: 0.02,
            zrouter_loss: 0.04,
            switch_loss: 0.03,
            range_loss: 0.01,
            zero_loss: 0.02,
            shape_loss: 0.03,
            overflow_loss: 0.08,
            total_loss: 1.33,
        }
    }
}

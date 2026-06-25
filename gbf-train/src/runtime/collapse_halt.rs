//! D16 router-collapse halt predicate.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

/// D16 grace period after Phase B begins.
pub const ROUTER_COLLAPSE_GRACE_STEPS: u64 = 500;

/// D16 rolling entropy window.
pub const ENTROPY_WINDOW_STEPS: usize = 100;

/// D16 entropy-floor ratio.
pub const ENTROPY_FLOOR_LOG2_RATIO: f32 = 0.5;

/// Router-collapse halt configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollapseHaltConfig {
    phase_b_start_step: u64,
    n_experts: usize,
    grace_steps: u64,
    window_steps: usize,
}

impl CollapseHaltConfig {
    /// Construct the D16 default halt config for a Phase B start and expert count.
    pub fn new(phase_b_start_step: u64, n_experts: usize) -> Result<Self, CollapseHaltError> {
        Self::with_window(
            phase_b_start_step,
            n_experts,
            ROUTER_COLLAPSE_GRACE_STEPS,
            ENTROPY_WINDOW_STEPS,
        )
    }

    /// Construct a halt config with explicit grace/window values.
    pub fn with_window(
        phase_b_start_step: u64,
        n_experts: usize,
        grace_steps: u64,
        window_steps: usize,
    ) -> Result<Self, CollapseHaltError> {
        if n_experts == 0 {
            return Err(CollapseHaltError::ZeroExperts);
        }
        if window_steps == 0 {
            return Err(CollapseHaltError::ZeroWindow);
        }

        let config = Self {
            phase_b_start_step,
            n_experts,
            grace_steps,
            window_steps,
        };
        config.first_checked_step()?;
        Ok(config)
    }

    /// Phase B start step.
    #[must_use]
    pub const fn phase_b_start_step(self) -> u64 {
        self.phase_b_start_step
    }

    /// Number of experts used to compute the entropy floor.
    #[must_use]
    pub const fn n_experts(self) -> usize {
        self.n_experts
    }

    /// Number of post-Phase-B grace steps.
    #[must_use]
    pub const fn grace_steps(self) -> u64 {
        self.grace_steps
    }

    /// Number of post-grace steps in the rolling entropy window.
    #[must_use]
    pub const fn window_steps(self) -> usize {
        self.window_steps
    }

    /// First step that may participate in the halt rolling window.
    pub fn first_checked_step(self) -> Result<u64, CollapseHaltError> {
        self.phase_b_start_step
            .checked_add(self.grace_steps)
            .ok_or(CollapseHaltError::GraceStepOverflow)
    }

    /// Entropy floor in bits: `0.5 * log2(n_experts)`.
    #[must_use]
    pub fn entropy_floor_bits(self) -> f32 {
        ENTROPY_FLOOR_LOG2_RATIO * (self.n_experts as f32).log2()
    }
}

/// Halt decision for the current step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseHaltDecision {
    /// Continue the run.
    Continue,
    /// Halt because the D16 rolling entropy mean dropped below the floor.
    CollapsedAt(u64),
}

/// Per-step collapse-halt observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollapseHaltObservation {
    step: u64,
    entropy_for_step_bits: f32,
    rolling_mean_bits: Option<f32>,
    decision: CollapseHaltDecision,
}

impl CollapseHaltObservation {
    /// Step observed.
    #[must_use]
    pub const fn step(self) -> u64 {
        self.step
    }

    /// D16 `entropy_for_step`: min over layers of `expert_usage_entropy_bits`.
    #[must_use]
    pub const fn entropy_for_step_bits(self) -> f32 {
        self.entropy_for_step_bits
    }

    /// Rolling mean once a post-grace window is available.
    #[must_use]
    pub const fn rolling_mean_bits(self) -> Option<f32> {
        self.rolling_mean_bits
    }

    /// Halt decision for this observation.
    #[must_use]
    pub const fn decision(self) -> CollapseHaltDecision {
        self.decision
    }
}

/// Stateful D16 collapse-halt monitor.
#[derive(Debug, Clone)]
pub struct CollapseHaltMonitor {
    config: CollapseHaltConfig,
    window: VecDeque<f32>,
    rolling_sum: f64,
    collapsed_at: Option<u64>,
}

impl CollapseHaltMonitor {
    /// Construct a monitor from a validated config.
    #[must_use]
    pub fn new(config: CollapseHaltConfig) -> Self {
        Self {
            config,
            window: VecDeque::with_capacity(config.window_steps()),
            rolling_sum: 0.0,
            collapsed_at: None,
        }
    }

    /// Config used by this monitor.
    #[must_use]
    pub const fn config(&self) -> CollapseHaltConfig {
        self.config
    }

    /// Observe a step and evaluate D16 when the post-grace window is full.
    pub fn observe_step(
        &mut self,
        step: u64,
        layer_entropy_bits: &[f32],
    ) -> Result<CollapseHaltObservation, CollapseHaltError> {
        let entropy_for_step_bits = entropy_for_step_bits(layer_entropy_bits)?;
        if let Some(collapsed_step) = self.collapsed_at {
            return Ok(CollapseHaltObservation {
                step,
                entropy_for_step_bits,
                rolling_mean_bits: self.rolling_mean_bits(),
                decision: CollapseHaltDecision::CollapsedAt(collapsed_step),
            });
        }

        let first_checked_step = self.config.first_checked_step()?;
        if step < first_checked_step {
            return Ok(CollapseHaltObservation {
                step,
                entropy_for_step_bits,
                rolling_mean_bits: None,
                decision: CollapseHaltDecision::Continue,
            });
        }

        self.push_entropy(entropy_for_step_bits);
        let rolling_mean_bits = self.rolling_mean_bits();
        let decision = if self.window.len() == self.config.window_steps()
            && rolling_mean_bits.expect("full window has rolling mean")
                < self.config.entropy_floor_bits()
        {
            self.collapsed_at = Some(step);
            CollapseHaltDecision::CollapsedAt(step)
        } else {
            CollapseHaltDecision::Continue
        };

        Ok(CollapseHaltObservation {
            step,
            entropy_for_step_bits,
            rolling_mean_bits,
            decision,
        })
    }

    fn push_entropy(&mut self, entropy_bits: f32) {
        if self.window.len() == self.config.window_steps()
            && let Some(removed) = self.window.pop_front()
        {
            self.rolling_sum -= f64::from(removed);
        }
        self.window.push_back(entropy_bits);
        self.rolling_sum += f64::from(entropy_bits);
    }

    fn rolling_mean_bits(&self) -> Option<f32> {
        if self.window.len() == self.config.window_steps() {
            Some((self.rolling_sum / self.config.window_steps() as f64) as f32)
        } else {
            None
        }
    }
}

/// Compute D16 `entropy_for_step` as the minimum layer entropy in bits.
pub fn entropy_for_step_bits(layer_entropy_bits: &[f32]) -> Result<f32, CollapseHaltError> {
    if layer_entropy_bits.is_empty() {
        return Err(CollapseHaltError::EmptyLayerEntropy);
    }

    let mut min_entropy = f32::INFINITY;
    for (index, &entropy) in layer_entropy_bits.iter().enumerate() {
        if !entropy.is_finite() {
            return Err(CollapseHaltError::NonFiniteLayerEntropy {
                index,
                value: entropy,
            });
        }
        min_entropy = min_entropy.min(entropy);
    }

    Ok(min_entropy)
}

/// Errors raised by collapse-halt validation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CollapseHaltError {
    /// The model must have at least one expert.
    ZeroExperts,
    /// The rolling window must contain at least one step.
    ZeroWindow,
    /// `phase_b_start_step + grace_steps` overflowed.
    GraceStepOverflow,
    /// No layer entropy values were supplied for a step.
    EmptyLayerEntropy,
    /// A layer entropy value was not finite.
    NonFiniteLayerEntropy {
        /// Index in the per-layer entropy slice.
        index: usize,
        /// Observed value.
        value: f32,
    },
}

impl fmt::Display for CollapseHaltError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroExperts => f.write_str("collapse halt requires at least one expert"),
            Self::ZeroWindow => f.write_str("collapse halt window must be non-zero"),
            Self::GraceStepOverflow => {
                f.write_str("collapse halt Phase B start plus grace steps overflowed")
            }
            Self::EmptyLayerEntropy => {
                f.write_str("collapse halt requires at least one layer entropy")
            }
            Self::NonFiniteLayerEntropy { index, value } => write!(
                f,
                "collapse halt layer entropy at index {index} must be finite, observed {value}"
            ),
        }
    }
}

impl Error for CollapseHaltError {}

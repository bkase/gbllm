//! Clock and frame timing constants for DMG-class targets.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Dot clock frequency in Hz. Pan Docs: Rendering / PPU timing.
pub const DOT_CLOCK_HZ: u32 = 4_194_304;

/// Normal-speed CPU M-cycles per second: four dots per M-cycle.
pub const NORMAL_M_CYCLES_PER_SECOND: u32 = DOT_CLOCK_HZ / 4;

/// CGB double-speed CPU M-cycles per second: two dots per M-cycle.
pub const DOUBLE_SPEED_M_CYCLES_PER_SECOND: u32 = DOT_CLOCK_HZ / 2;

/// Total PPU dots per frame.
pub const FRAME_DOTS: u32 = 70_224;

/// Total normal-speed M-cycles per frame.
pub const FRAME_M_CYCLES: u32 = FRAME_DOTS / 4;

/// PPU dots in the VBlank interval.
pub const VBLANK_DOTS: u32 = 4_560;

/// Normal-speed M-cycles in the VBlank interval.
pub const VBLANK_M_CYCLES: u32 = VBLANK_DOTS / 4;

/// Approximate normal-speed frames per second.
pub const FRAMES_PER_SECOND: f32 = NORMAL_M_CYCLES_PER_SECOND as f32 / FRAME_M_CYCLES as f32;

/// Timing profile carried by a resolved target profile.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(try_from = "TimingProfileRepr")]
pub struct TimingProfile {
    dot_clock_hz: u32,
    dots_per_m_cycle: u8,
    frame_dots: u32,
    vblank_dots: u32,
}

impl TimingProfile {
    pub fn try_new(
        dot_clock_hz: u32,
        dots_per_m_cycle: u8,
        frame_dots: u32,
        vblank_dots: u32,
    ) -> Result<Self, TimingProfileError> {
        if dots_per_m_cycle == 0 {
            return Err(TimingProfileError::ZeroDotsPerMCycle);
        }
        if !frame_dots.is_multiple_of(u32::from(dots_per_m_cycle)) {
            return Err(TimingProfileError::FrameDotsNotDivisible {
                frame_dots,
                dots_per_m_cycle,
            });
        }
        if !vblank_dots.is_multiple_of(u32::from(dots_per_m_cycle)) {
            return Err(TimingProfileError::VblankDotsNotDivisible {
                vblank_dots,
                dots_per_m_cycle,
            });
        }

        Ok(Self {
            dot_clock_hz,
            dots_per_m_cycle,
            frame_dots,
            vblank_dots,
        })
    }

    #[must_use]
    pub const fn dot_clock_hz(&self) -> u32 {
        self.dot_clock_hz
    }

    #[must_use]
    pub const fn dots_per_m_cycle(&self) -> u8 {
        self.dots_per_m_cycle
    }

    #[must_use]
    pub const fn frame_dots(&self) -> u32 {
        self.frame_dots
    }

    #[must_use]
    pub const fn vblank_dots(&self) -> u32 {
        self.vblank_dots
    }

    #[must_use]
    pub const fn frame_m_cycles(&self) -> u32 {
        self.frame_dots / self.dots_per_m_cycle as u32
    }

    #[must_use]
    pub const fn vblank_m_cycles(&self) -> u32 {
        self.vblank_dots / self.dots_per_m_cycle as u32
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
struct TimingProfileRepr {
    dot_clock_hz: u32,
    dots_per_m_cycle: u8,
    frame_dots: u32,
    vblank_dots: u32,
}

impl TryFrom<TimingProfileRepr> for TimingProfile {
    type Error = TimingProfileError;

    fn try_from(value: TimingProfileRepr) -> Result<Self, Self::Error> {
        Self::try_new(
            value.dot_clock_hz,
            value.dots_per_m_cycle,
            value.frame_dots,
            value.vblank_dots,
        )
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TimingProfileError {
    ZeroDotsPerMCycle,
    FrameDotsNotDivisible {
        frame_dots: u32,
        dots_per_m_cycle: u8,
    },
    VblankDotsNotDivisible {
        vblank_dots: u32,
        dots_per_m_cycle: u8,
    },
}

impl fmt::Display for TimingProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDotsPerMCycle => f.write_str("dots_per_m_cycle must be non-zero"),
            Self::FrameDotsNotDivisible {
                frame_dots,
                dots_per_m_cycle,
            } => write!(
                f,
                "frame_dots {frame_dots} must be divisible by dots_per_m_cycle {dots_per_m_cycle}"
            ),
            Self::VblankDotsNotDivisible {
                vblank_dots,
                dots_per_m_cycle,
            } => write!(
                f,
                "vblank_dots {vblank_dots} must be divisible by dots_per_m_cycle {dots_per_m_cycle}"
            ),
        }
    }
}

impl std::error::Error for TimingProfileError {}

#[must_use]
pub const fn dmg_timing() -> TimingProfile {
    TimingProfile {
        dot_clock_hz: DOT_CLOCK_HZ,
        dots_per_m_cycle: 4,
        frame_dots: FRAME_DOTS,
        vblank_dots: VBLANK_DOTS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_cycles() {
        assert_eq!(FRAME_M_CYCLES, 17_556);
        assert_eq!(FRAME_M_CYCLES, FRAME_DOTS / 4);
        assert_eq!(FRAME_DOTS, 154 * 456);
    }

    #[test]
    fn vblank_cycles() {
        assert_eq!(VBLANK_M_CYCLES, 1_140);
        assert_eq!(VBLANK_M_CYCLES, VBLANK_DOTS / 4);
    }

    #[test]
    fn fps_close_to_597() {
        assert!((59.72..=59.73).contains(&FRAMES_PER_SECOND));
    }

    #[test]
    fn dot_clock_constant_across_speeds() {
        assert_eq!(NORMAL_M_CYCLES_PER_SECOND * 4, DOT_CLOCK_HZ);
        assert_eq!(DOUBLE_SPEED_M_CYCLES_PER_SECOND * 2, DOT_CLOCK_HZ);
    }

    #[test]
    fn dmg_timing_round_trip() {
        let encoded = serde_json::to_string(&dmg_timing()).unwrap();
        let decoded: TimingProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, dmg_timing());
        assert_eq!(
            serde_json::to_value(dmg_timing()).unwrap(),
            serde_json::json!({
                "dot_clock_hz": DOT_CLOCK_HZ,
                "dots_per_m_cycle": 4,
                "frame_dots": FRAME_DOTS,
                "vblank_dots": VBLANK_DOTS
            })
        );
    }

    #[test]
    fn rejects_invalid_timing_profiles() {
        assert_eq!(
            TimingProfile::try_new(DOT_CLOCK_HZ, 0, FRAME_DOTS, VBLANK_DOTS),
            Err(TimingProfileError::ZeroDotsPerMCycle)
        );
        assert_eq!(
            TimingProfile::try_new(DOT_CLOCK_HZ, 4, FRAME_DOTS + 1, VBLANK_DOTS),
            Err(TimingProfileError::FrameDotsNotDivisible {
                frame_dots: FRAME_DOTS + 1,
                dots_per_m_cycle: 4,
            })
        );
    }

    #[test]
    fn serde_runs_validation() {
        let bad = r#"{
            "dot_clock_hz":4194304,
            "dots_per_m_cycle":0,
            "frame_dots":70224,
            "vblank_dots":4560
        }"#;
        assert!(serde_json::from_str::<TimingProfile>(bad).is_err());
    }

    #[test]
    fn vblank_smaller_than_frame() {
        assert!(VBLANK_M_CYCLES < FRAME_M_CYCLES);
    }

    #[test]
    fn profile_derives_m_cycles() {
        let timing = dmg_timing();
        assert_eq!(timing.frame_m_cycles(), FRAME_M_CYCLES);
        assert_eq!(timing.vblank_m_cycles(), VBLANK_M_CYCLES);
    }
}

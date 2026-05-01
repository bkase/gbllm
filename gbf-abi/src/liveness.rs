//! Liveness counters embedded in the inference-state header.

#[cfg(test)]
use core::mem::{align_of, size_of};

#[cfg(test)]
use memoffset::offset_of;
use serde::{Deserialize, Serialize};

use crate::checkpoint::CompactCheckpointId;

/// Runtime progress counters used to detect livelock.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessCounters {
    pub progress_epoch: u32,
    pub last_checkpoint: CompactCheckpointId,
    pub no_progress_frames: u16,
    pub livelock_threshold_frames: u16,
    pub _reserved: [u8; 2],
}

impl LivenessCounters {
    pub const SIZE: usize = 12;

    #[must_use]
    pub const fn new(threshold: u16) -> Self {
        Self {
            progress_epoch: 0,
            last_checkpoint: CompactCheckpointId::NONE,
            no_progress_frames: 0,
            livelock_threshold_frames: threshold,
            _reserved: [0, 0],
        }
    }

    pub fn record_progress(&mut self, cp: CompactCheckpointId) {
        self.progress_epoch = self.progress_epoch.saturating_add(1);
        self.last_checkpoint = cp;
        self.no_progress_frames = 0;
    }

    pub fn note_idle_frame(&mut self) {
        self.no_progress_frames = self.no_progress_frames.saturating_add(1);
    }

    #[must_use]
    pub const fn is_livelocked(&self) -> bool {
        self.livelock_threshold_frames != 0
            && self.no_progress_frames >= self.livelock_threshold_frames
    }

    pub fn reset(&mut self, threshold: u16) {
        *self = Self::new(threshold);
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.progress_epoch.to_le_bytes());
        out[4..6].copy_from_slice(&self.last_checkpoint.0.to_le_bytes());
        out[6..8].copy_from_slice(&self.no_progress_frames.to_le_bytes());
        out[8..10].copy_from_slice(&self.livelock_threshold_frames.to_le_bytes());
        out[10..12].copy_from_slice(&self._reserved);
        out
    }

    #[must_use]
    pub fn from_bytes(bytes: &[u8; Self::SIZE]) -> Self {
        Self {
            progress_epoch: u32::from_le_bytes(bytes[0..4].try_into().expect("fixed slice")),
            last_checkpoint: CompactCheckpointId(u16::from_le_bytes(
                bytes[4..6].try_into().expect("fixed slice"),
            )),
            no_progress_frames: u16::from_le_bytes(bytes[6..8].try_into().expect("fixed slice")),
            livelock_threshold_frames: u16::from_le_bytes(
                bytes[8..10].try_into().expect("fixed slice"),
            ),
            _reserved: bytes[10..12].try_into().expect("fixed slice"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(size_of::<LivenessCounters>(), 12);
        assert_eq!(align_of::<LivenessCounters>(), 4);
        assert_eq!(offset_of!(LivenessCounters, last_checkpoint), 4);
        assert_eq!(offset_of!(LivenessCounters, _reserved), 10);
    }

    #[test]
    fn progress_advance() {
        let mut counters = LivenessCounters::new(60);
        counters.no_progress_frames = 7;
        counters.record_progress(CompactCheckpointId(42));

        assert_eq!(counters.progress_epoch, 1);
        assert_eq!(counters.last_checkpoint, CompactCheckpointId(42));
        assert_eq!(counters.no_progress_frames, 0);
    }

    #[test]
    fn idle_frames_saturate() {
        let mut counters = LivenessCounters::new(60);
        counters.no_progress_frames = u16::MAX;
        counters.note_idle_frame();

        assert_eq!(counters.no_progress_frames, u16::MAX);
    }

    #[test]
    fn progress_epoch_saturates() {
        let mut counters = LivenessCounters::new(60);
        counters.progress_epoch = u32::MAX;
        counters.record_progress(CompactCheckpointId(1));

        assert_eq!(counters.progress_epoch, u32::MAX);
    }

    #[test]
    fn livelock_threshold_zero_disables() {
        let mut counters = LivenessCounters::new(0);
        counters.no_progress_frames = u16::MAX;

        assert!(!counters.is_livelocked());
    }

    #[test]
    fn livelock_threshold_fires_at_eq() {
        let mut counters = LivenessCounters::new(3);
        counters.no_progress_frames = 2;
        assert!(!counters.is_livelocked());

        counters.note_idle_frame();
        assert!(counters.is_livelocked());
    }

    #[test]
    fn constructor_zeroes_reserved() {
        let counters = LivenessCounters::new(60);

        assert_eq!(counters._reserved, [0, 0]);
    }

    #[test]
    fn serde_round_trip() {
        let mut counters = LivenessCounters::new(60);
        counters.record_progress(CompactCheckpointId(7));
        counters.note_idle_frame();

        let encoded = serde_json::to_string(&counters).expect("liveness serializes");
        let decoded: LivenessCounters =
            serde_json::from_str(&encoded).expect("liveness deserializes");

        assert_eq!(decoded, counters);
    }

    #[test]
    fn from_bytes_round_trip() {
        let mut counters = LivenessCounters::new(60);
        counters.record_progress(CompactCheckpointId(7));
        counters.note_idle_frame();

        let bytes = counters.to_bytes();
        let decoded = LivenessCounters::from_bytes(&bytes);

        assert_eq!(decoded, counters);
    }

    #[test]
    fn has_no_drop() {
        assert!(!core::mem::needs_drop::<LivenessCounters>());
    }
}

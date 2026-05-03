//! Deterministic execution policy shared by all emulator consumers.

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FIXED_CARTRIDGE_RTC_UNIX_MS: i64 = 946_684_800_000;
pub const FIXED_SAVE_STATE_UNIX_MS: u64 = 946_684_800_000;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CartridgeRtcMode {
    Fixed,
    RealTime,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum SaveStateMetadataMode {
    Fixed,
    None,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum PowerOnRamPolicy {
    GameroyDefault,
    FixedFill {
        wram: u8,
        hram: u8,
        cartridge_ram: u8,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum AudioOutputMode {
    Disabled,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct DeterminismPolicy {
    cartridge_rtc: CartridgeRtcMode,
    save_state_metadata: SaveStateMetadataMode,
    power_on_ram: PowerOnRamPolicy,
    audio_output: AudioOutputMode,
}

impl Default for DeterminismPolicy {
    fn default() -> Self {
        Self {
            cartridge_rtc: CartridgeRtcMode::Fixed,
            save_state_metadata: SaveStateMetadataMode::Fixed,
            power_on_ram: PowerOnRamPolicy::GameroyDefault,
            audio_output: AudioOutputMode::Disabled,
        }
    }
}

impl DeterminismPolicy {
    #[must_use]
    pub fn builder() -> DeterminismPolicyBuilder {
        DeterminismPolicyBuilder::new()
    }

    #[must_use]
    pub const fn cartridge_rtc(&self) -> CartridgeRtcMode {
        self.cartridge_rtc
    }

    #[must_use]
    pub const fn save_state_metadata(&self) -> SaveStateMetadataMode {
        self.save_state_metadata
    }

    #[must_use]
    pub const fn power_on_ram(&self) -> PowerOnRamPolicy {
        self.power_on_ram
    }

    #[must_use]
    pub const fn audio_output(&self) -> AudioOutputMode {
        self.audio_output
    }

    #[must_use]
    pub fn save_state_timestamp(&self) -> Option<u64> {
        match self.save_state_metadata {
            SaveStateMetadataMode::Fixed => Some(FIXED_SAVE_STATE_UNIX_MS),
            SaveStateMetadataMode::None => None,
        }
    }

    #[must_use]
    pub fn fingerprint(&self) -> Hash256 {
        let mut hasher = Sha256::new();
        hasher.update(b"gbf-emu-determinism-policy-v1");
        hasher.update([match self.cartridge_rtc {
            CartridgeRtcMode::Fixed => 0,
            CartridgeRtcMode::RealTime => 1,
        }]);
        hasher.update([match self.save_state_metadata {
            SaveStateMetadataMode::Fixed => 0,
            SaveStateMetadataMode::None => 1,
        }]);
        match self.power_on_ram {
            PowerOnRamPolicy::GameroyDefault => hasher.update([0]),
            PowerOnRamPolicy::FixedFill {
                wram,
                hram,
                cartridge_ram,
            } => {
                hasher.update([1, wram, hram, cartridge_ram]);
            }
        }
        hasher.update([match self.audio_output {
            AudioOutputMode::Disabled => 0,
        }]);
        Hash256::from_bytes(hasher.finalize().into())
    }
}

#[derive(Clone, Debug, Default)]
pub struct DeterminismPolicyBuilder {
    cartridge_rtc: Option<CartridgeRtcMode>,
    save_state_metadata: Option<SaveStateMetadataMode>,
    power_on_ram: Option<PowerOnRamPolicy>,
    audio_output: Option<AudioOutputMode>,
}

impl DeterminismPolicyBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_real_time_cartridge_rtc(mut self) -> Self {
        self.cartridge_rtc = Some(CartridgeRtcMode::RealTime);
        self
    }

    #[must_use]
    pub fn with_power_on_ram(mut self, policy: PowerOnRamPolicy) -> Self {
        self.power_on_ram = Some(policy);
        self
    }

    #[must_use]
    pub fn with_save_state_metadata(mut self, mode: SaveStateMetadataMode) -> Self {
        self.save_state_metadata = Some(mode);
        self
    }

    #[must_use]
    pub fn build(self) -> DeterminismPolicy {
        let default = DeterminismPolicy::default();
        DeterminismPolicy {
            cartridge_rtc: self.cartridge_rtc.unwrap_or(default.cartridge_rtc),
            save_state_metadata: self
                .save_state_metadata
                .unwrap_or(default.save_state_metadata),
            power_on_ram: self.power_on_ram.unwrap_or(default.power_on_ram),
            audio_output: self.audio_output.unwrap_or(default.audio_output),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_locked_down() {
        let policy = DeterminismPolicy::default();

        assert_eq!(policy.cartridge_rtc(), CartridgeRtcMode::Fixed);
        assert_eq!(policy.save_state_metadata(), SaveStateMetadataMode::Fixed);
        assert_eq!(policy.power_on_ram(), PowerOnRamPolicy::GameroyDefault);
        assert_eq!(policy.audio_output(), AudioOutputMode::Disabled);
    }

    #[test]
    fn fingerprint_distinguishes_non_default_values() {
        let default = DeterminismPolicy::default().fingerprint();
        let real_time = DeterminismPolicy::builder()
            .with_real_time_cartridge_rtc()
            .build()
            .fingerprint();
        let fixed_fill = DeterminismPolicy::builder()
            .with_power_on_ram(PowerOnRamPolicy::FixedFill {
                wram: 0x12,
                hram: 0x34,
                cartridge_ram: 0x56,
            })
            .build()
            .fingerprint();
        let no_timestamp = DeterminismPolicy::builder()
            .with_save_state_metadata(SaveStateMetadataMode::None)
            .build()
            .fingerprint();

        assert_ne!(default, real_time);
        assert_ne!(default, fixed_fill);
        assert_ne!(default, no_timestamp);
    }

    #[test]
    fn fingerprint_is_stable_for_same_policy() {
        assert_eq!(
            DeterminismPolicy::default().fingerprint(),
            DeterminismPolicy::default().fingerprint()
        );
    }

    #[test]
    fn audio_output_disabled_is_exhaustive_for_m0() {
        match DeterminismPolicy::default().audio_output() {
            AudioOutputMode::Disabled => {}
        }
    }
}

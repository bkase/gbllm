//! Runtime shell module vocabulary shared by runtime emitters and budget policy.

use serde::{Deserialize, Serialize};

/// Closed set of runtime shell modules tracked by the reference-shell budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeShellModule {
    Boot,
    Interrupts,
    Scheduler,
    Banking,
    Joypad,
    Text,
    Keyboard,
    VideoCommit,
    Panic,
    FuturePersistence,
    FutureTrace,
    FutureHarness,
}

impl RuntimeShellModule {
    pub const ALL: &'static [Self] = &[
        Self::Boot,
        Self::Interrupts,
        Self::Scheduler,
        Self::Banking,
        Self::Joypad,
        Self::Text,
        Self::Keyboard,
        Self::VideoCommit,
        Self::Panic,
        Self::FuturePersistence,
        Self::FutureTrace,
        Self::FutureHarness,
    ];

    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::Interrupts => "interrupts",
            Self::Scheduler => "scheduler",
            Self::Banking => "banking",
            Self::Joypad => "joypad",
            Self::Text => "text",
            Self::Keyboard => "keyboard",
            Self::VideoCommit => "video_commit",
            Self::Panic => "panic",
            Self::FuturePersistence => "future_persistence",
            Self::FutureTrace => "future_trace",
            Self::FutureHarness => "future_harness",
        }
    }

    #[must_use]
    pub const fn is_future_reservation(self) -> bool {
        matches!(
            self,
            Self::FuturePersistence | Self::FutureTrace | Self::FutureHarness
        )
    }
}

/// Trait for types that carry a shell-module annotation.
pub trait RuntimeShellAnnotated {
    fn runtime_shell_module(&self) -> RuntimeShellModule;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_closed_and_ordered() {
        assert_eq!(RuntimeShellModule::ALL.len(), 12);
        assert_eq!(RuntimeShellModule::ALL[0], RuntimeShellModule::Boot);
        assert_eq!(
            RuntimeShellModule::ALL[RuntimeShellModule::ALL.len() - 1],
            RuntimeShellModule::FutureHarness
        );
    }

    #[test]
    fn serde_uses_canonical_names() {
        let json = serde_json::to_string(&RuntimeShellModule::VideoCommit).expect("serializes");
        assert_eq!(json, r#""video_commit""#);
        assert_eq!(
            serde_json::from_str::<RuntimeShellModule>(&json).expect("deserializes"),
            RuntimeShellModule::VideoCommit
        );
    }

    #[test]
    fn future_reservation_set_is_explicit() {
        let futures: Vec<_> = RuntimeShellModule::ALL
            .iter()
            .copied()
            .filter(|module| module.is_future_reservation())
            .collect();
        assert_eq!(
            futures,
            vec![
                RuntimeShellModule::FuturePersistence,
                RuntimeShellModule::FutureTrace,
                RuntimeShellModule::FutureHarness,
            ]
        );
    }
}

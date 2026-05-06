//! Structured tracing helpers for runtime-owned events.

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tracing::field::{Field, Visit};
use tracing::subscriber::Interest;
use tracing::{Event, Id, Level, Metadata, Subscriber};

pub const FB1_TRACE_TARGET: &str = "gbf::fb1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum FB1TraceEvent {
    BankLeaseAcquired {
        bank: u16,
        mcycle: u64,
        lease_id: u32,
    },
    BankLeaseReleased {
        bank: u16,
        mcycle: u64,
        lease_id: u32,
    },
    MbcRegisterWrite {
        register: u16,
        value: u8,
        mcycle: u64,
    },
    YieldCheckpoint {
        mcycle: u64,
        quantum: YieldQuantumKind,
        products_completed: u32,
        active_leases: u16,
    },
    LivenessProgress {
        mcycle: u64,
        progress_epoch: u32,
    },
    TileEntered {
        mt: u16,
        nt: u16,
        mcycle: u64,
    },
    TileCompleteSafePoint {
        mt: u16,
        nt: u16,
        mcycle: u64,
        source_addr: u16,
        active_leases: u16,
    },
    HarnessDumpRequested {
        mcycle: u64,
        addr: u16,
        len: u16,
    },
    HarnessResumeAcknowledged {
        mcycle: u64,
    },
    VBlankFired {
        frame: u32,
        mcycle: u64,
    },
    WidgetTickDispatched {
        frame: u32,
        mcycle: u64,
    },
    SchedulerServicedFrame {
        frame: u32,
        mcycle: u64,
    },
    YieldReturnedToScheduler {
        frame: u32,
        mcycle: u64,
        remaining_frame_mcycles_i32: i32,
        completed_quantum_products: u16,
        compute_progress_epoch: u32,
    },
    ComputeProgressEpochAdvanced {
        frame: u32,
        mcycle: u64,
        compute_progress_epoch: u32,
    },
    ComputeReqImported {
        request_hash: String,
    },
    LowerToIrComplete {
        ir_hash: String,
    },
    LowerToAsmIrComplete {
        asmir_hash: String,
    },
    RomEmitted {
        rom_sha256: String,
        bytes: u32,
    },
    ValidatorAccepted {
        rule: String,
        scope: String,
    },
    ValidatorRejected {
        rule: String,
        scope: String,
        evidence: String,
    },
}

impl FB1TraceEvent {
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::BankLeaseAcquired { .. } => "BankLeaseAcquired",
            Self::BankLeaseReleased { .. } => "BankLeaseReleased",
            Self::MbcRegisterWrite { .. } => "MbcRegisterWrite",
            Self::YieldCheckpoint { .. } => "YieldCheckpoint",
            Self::LivenessProgress { .. } => "LivenessProgress",
            Self::TileEntered { .. } => "TileEntered",
            Self::TileCompleteSafePoint { .. } => "TileCompleteSafePoint",
            Self::HarnessDumpRequested { .. } => "HarnessDumpRequested",
            Self::HarnessResumeAcknowledged { .. } => "HarnessResumeAcknowledged",
            Self::VBlankFired { .. } => "VBlankFired",
            Self::WidgetTickDispatched { .. } => "WidgetTickDispatched",
            Self::SchedulerServicedFrame { .. } => "SchedulerServicedFrame",
            Self::YieldReturnedToScheduler { .. } => "YieldReturnedToScheduler",
            Self::ComputeProgressEpochAdvanced { .. } => "ComputeProgressEpochAdvanced",
            Self::ComputeReqImported { .. } => "ComputeReqImported",
            Self::LowerToIrComplete { .. } => "LowerToIrComplete",
            Self::LowerToAsmIrComplete { .. } => "LowerToAsmIrComplete",
            Self::RomEmitted { .. } => "RomEmitted",
            Self::ValidatorAccepted { .. } => "ValidatorAccepted",
            Self::ValidatorRejected { .. } => "ValidatorRejected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum YieldQuantumKind {
    KLaneFullTile,
    KLaneRows4,
    KLaneRow,
}

pub fn emit_fb1_event(event: &FB1TraceEvent) {
    let fb1_event_json = serde_json::to_string(event).expect("FB1TraceEvent serializes");
    tracing::info!(
        target: FB1_TRACE_TARGET,
        kind = event.kind_name(),
        fb1_event_json = %fb1_event_json,
        "fb1 trace event"
    );
}

#[derive(Debug, Clone, Default)]
pub struct CapturedTrace {
    events: Vec<FB1TraceEvent>,
}

impl CapturedTrace {
    #[must_use]
    pub fn capture(f: impl FnOnce()) -> Self {
        let subscriber = CaptureSubscriber::default();
        let events = Arc::clone(&subscriber.events);
        tracing::subscriber::with_default(subscriber, f);
        Self {
            events: events.lock().expect("capture lock").clone(),
        }
    }

    #[must_use]
    pub fn events(&self) -> &[FB1TraceEvent] {
        &self.events
    }

    #[must_use]
    pub fn count(&self, predicate: impl Fn(&FB1TraceEvent) -> bool) -> usize {
        self.events.iter().filter(|event| predicate(event)).count()
    }

    #[must_use]
    pub fn first(&self, predicate: impl Fn(&FB1TraceEvent) -> bool) -> Option<&FB1TraceEvent> {
        self.events.iter().find(|event| predicate(event))
    }

    pub fn assert_lease_acquire_release_balance(&self) -> Result<(), TraceAssertionError> {
        let mut balance = 0_i32;
        for event in &self.events {
            match event {
                FB1TraceEvent::BankLeaseAcquired { .. } => balance += 1,
                FB1TraceEvent::BankLeaseReleased { .. } => balance -= 1,
                _ => {}
            }
            if balance < 0 {
                return Err(TraceAssertionError::LeaseReleasedBeforeAcquire);
            }
        }
        if balance == 0 {
            Ok(())
        } else {
            Err(TraceAssertionError::UnbalancedLeases { balance })
        }
    }

    pub fn assert_no_yield_while_lease_active(&self) -> Result<(), TraceAssertionError> {
        let mut active = 0_u16;
        for event in &self.events {
            match event {
                FB1TraceEvent::BankLeaseAcquired { .. } => active = active.saturating_add(1),
                FB1TraceEvent::BankLeaseReleased { .. } => active = active.saturating_sub(1),
                FB1TraceEvent::YieldCheckpoint { active_leases, .. }
                    if active != 0 || *active_leases != 0 =>
                {
                    return Err(TraceAssertionError::YieldWithActiveLease {
                        active_leases: active.max(*active_leases),
                    });
                }
                FB1TraceEvent::TileCompleteSafePoint { active_leases, .. }
                    if active != 0 || *active_leases != 0 =>
                {
                    return Err(TraceAssertionError::HarnessPauseWithActiveLease {
                        active_leases: active.max(*active_leases),
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn assert_widget_tick_per_gated_frame(
        &self,
        gated: Range<u32>,
    ) -> Result<(), TraceAssertionError> {
        for frame in gated {
            let ticks = self.count(|event| {
                matches!(event, FB1TraceEvent::WidgetTickDispatched { frame: f, .. } if *f == frame)
            });
            if ticks != 1 {
                return Err(TraceAssertionError::WidgetTickCount { frame, ticks });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceAssertionError {
    LeaseReleasedBeforeAcquire,
    UnbalancedLeases { balance: i32 },
    YieldWithActiveLease { active_leases: u16 },
    HarnessPauseWithActiveLease { active_leases: u16 },
    WidgetTickCount { frame: u32, ticks: usize },
}

impl fmt::Display for TraceAssertionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LeaseReleasedBeforeAcquire => f.write_str("lease released before acquire"),
            Self::UnbalancedLeases { balance } => write!(f, "lease balance is {balance}"),
            Self::YieldWithActiveLease { active_leases } => {
                write!(f, "yield emitted with {active_leases} active lease(s)")
            }
            Self::HarnessPauseWithActiveLease { active_leases } => {
                write!(
                    f,
                    "harness pause emitted with {active_leases} active lease(s)"
                )
            }
            Self::WidgetTickCount { frame, ticks } => {
                write!(f, "frame {frame} has {ticks} widget tick events")
            }
        }
    }
}

impl std::error::Error for TraceAssertionError {}

#[derive(Clone, Default)]
struct CaptureSubscriber {
    events: Arc<Mutex<Vec<FB1TraceEvent>>>,
}

impl Subscriber for CaptureSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == FB1_TRACE_TARGET && *metadata.level() <= Level::INFO
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() != FB1_TRACE_TARGET {
            return;
        }
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let Some(json) = visitor.fb1_event_json else {
            return;
        };
        if let Ok(event) = serde_json::from_str::<FB1TraceEvent>(&json) {
            self.events.lock().expect("capture lock").push(event);
        }
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}

    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if metadata.target() == FB1_TRACE_TARGET {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::INFO)
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    fb1_event_json: Option<String>,
}

impl Visit for JsonFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "fb1_event_json" {
            self.fb1_event_json = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "fb1_event_json" {
            self.fb1_event_json = Some(value.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serde_round_trip() {
        let event = FB1TraceEvent::YieldReturnedToScheduler {
            frame: 7,
            mcycle: 123,
            remaining_frame_mcycles_i32: -4,
            completed_quantum_products: 256,
            compute_progress_epoch: 9,
        };
        let encoded = serde_json::to_string(&event).expect("serializes");
        let decoded = serde_json::from_str::<FB1TraceEvent>(&encoded).expect("deserializes");
        assert_eq!(decoded, event);
    }

    #[test]
    fn subscriber_captures_emitted_event_shape() {
        let trace = CapturedTrace::capture(|| {
            emit_fb1_event(&FB1TraceEvent::BankLeaseAcquired {
                bank: 2,
                mcycle: 10,
                lease_id: 1,
            });
        });
        assert_eq!(
            trace.events(),
            &[FB1TraceEvent::BankLeaseAcquired {
                bank: 2,
                mcycle: 10,
                lease_id: 1
            }]
        );
    }

    #[test]
    fn subscriber_helpers_assert_lease_balance() {
        let trace = CapturedTrace::capture(|| {
            emit_fb1_event(&FB1TraceEvent::BankLeaseAcquired {
                bank: 1,
                mcycle: 1,
                lease_id: 7,
            });
            emit_fb1_event(&FB1TraceEvent::BankLeaseReleased {
                bank: 1,
                mcycle: 2,
                lease_id: 7,
            });
        });
        trace
            .assert_lease_acquire_release_balance()
            .expect("balanced");
    }

    #[test]
    fn subscriber_helpers_detect_yield_under_lease() {
        let trace = CapturedTrace::capture(|| {
            emit_fb1_event(&FB1TraceEvent::BankLeaseAcquired {
                bank: 1,
                mcycle: 1,
                lease_id: 7,
            });
            emit_fb1_event(&FB1TraceEvent::YieldCheckpoint {
                mcycle: 2,
                quantum: YieldQuantumKind::KLaneFullTile,
                products_completed: 256,
                active_leases: 1,
            });
        });
        assert!(matches!(
            trace.assert_no_yield_while_lease_active(),
            Err(TraceAssertionError::YieldWithActiveLease { active_leases: 1 })
        ));
    }

    #[test]
    fn subscriber_helpers_detect_missed_widget_tick() {
        let trace = CapturedTrace::capture(|| {
            emit_fb1_event(&FB1TraceEvent::WidgetTickDispatched {
                frame: 4,
                mcycle: 100,
            });
        });
        assert!(matches!(
            trace.assert_widget_tick_per_gated_frame(4..6),
            Err(TraceAssertionError::WidgetTickCount { frame: 5, ticks: 0 })
        ));
    }

    #[test]
    fn f_b1_frame_widget_ticks_once_per_vblank() {
        let trace = CapturedTrace::capture(|| {
            for frame in 8..12 {
                emit_fb1_event(&FB1TraceEvent::VBlankFired {
                    frame,
                    mcycle: u64::from(frame) * u64::from(gbf_hw::timing::FRAME_M_CYCLES),
                });
                emit_fb1_event(&FB1TraceEvent::WidgetTickDispatched {
                    frame,
                    mcycle: u64::from(frame) * u64::from(gbf_hw::timing::FRAME_M_CYCLES) + 32,
                });
            }
        });
        trace
            .assert_widget_tick_per_gated_frame(8..12)
            .expect("one widget tick per gated frame");
    }

    #[test]
    fn f_b1_l2_banklease_balance() {
        let trace = CapturedTrace::capture(|| {
            for lease_id in 0_u32..4 {
                emit_fb1_event(&FB1TraceEvent::BankLeaseAcquired {
                    bank: 1 + lease_id as u16 % 2,
                    mcycle: u64::from(lease_id) * 10,
                    lease_id,
                });
                emit_fb1_event(&FB1TraceEvent::BankLeaseReleased {
                    bank: 1 + lease_id as u16 % 2,
                    mcycle: u64::from(lease_id) * 10 + 4,
                    lease_id,
                });
            }
        });
        trace
            .assert_lease_acquire_release_balance()
            .expect("balanced");
    }

    #[test]
    fn f_b1_l2_no_yield_while_banklease_active() {
        let trace = CapturedTrace::capture(|| {
            emit_fb1_event(&FB1TraceEvent::BankLeaseAcquired {
                bank: 1,
                mcycle: 1,
                lease_id: 1,
            });
            emit_fb1_event(&FB1TraceEvent::BankLeaseReleased {
                bank: 1,
                mcycle: 2,
                lease_id: 1,
            });
            emit_fb1_event(&FB1TraceEvent::YieldCheckpoint {
                mcycle: 3,
                quantum: YieldQuantumKind::KLaneRows4,
                products_completed: 64,
                active_leases: 0,
            });
            emit_fb1_event(&FB1TraceEvent::TileCompleteSafePoint {
                mt: 0,
                nt: 0,
                mcycle: 4,
                source_addr: 0xC000,
                active_leases: 0,
            });
        });
        trace.assert_no_yield_while_lease_active().expect("clean");
    }

    #[test]
    fn liveness_advances_per_yield_quantum() {
        let trace = CapturedTrace::capture(|| {
            for epoch in 1..=3 {
                emit_fb1_event(&FB1TraceEvent::YieldCheckpoint {
                    mcycle: u64::from(epoch) * 10,
                    quantum: YieldQuantumKind::KLaneRow,
                    products_completed: 16,
                    active_leases: 0,
                });
                emit_fb1_event(&FB1TraceEvent::LivenessProgress {
                    mcycle: u64::from(epoch) * 10 + 1,
                    progress_epoch: epoch,
                });
            }
        });
        assert_eq!(
            trace.count(|event| matches!(event, FB1TraceEvent::YieldCheckpoint { .. })),
            trace.count(|event| matches!(event, FB1TraceEvent::LivenessProgress { .. }))
        );
    }
}

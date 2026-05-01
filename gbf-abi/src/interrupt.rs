//! Interrupt policy and resource lease vocabulary.

use core::mem::size_of;

use serde::{Deserialize, Serialize};

/// Per-slice interrupt behavior declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptPolicy {
    Enabled,
    ShortCriticalSection,
    Disabled,
}

impl InterruptPolicy {
    pub const ALL: [Self; 3] = [Self::Enabled, Self::ShortCriticalSection, Self::Disabled];
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LeaseId(pub u32);

impl LeaseId {
    pub const ALL_VALUES_FIT: bool = size_of::<Self>() == size_of::<u32>();
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SliceId(pub u32);

impl SliceId {
    pub const ALL_VALUES_FIT: bool = size_of::<Self>() == size_of::<u32>();
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OverlayId(pub u16);

impl OverlayId {
    pub const ALL_VALUES_FIT: bool = size_of::<Self>() == size_of::<u16>();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomWindowBinding {
    pub bank: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SramPageBinding {
    pub page: u8,
    pub enabled: bool,
}

/// Typed lease vocabulary for validators and reports, not a ROM-resident layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceLeaseKind {
    RomWindow(RomWindowBinding),
    SramPage(SramPageBinding),
    Overlay(OverlayId),
    InterruptMask(InterruptPolicy),
}

impl ResourceLeaseKind {
    #[must_use]
    pub const fn yield_safe(&self) -> bool {
        match self {
            Self::RomWindow(_) => false,
            Self::SramPage(_) => true,
            Self::Overlay(_) => false,
            Self::InterruptMask(policy) => matches!(policy, InterruptPolicy::Enabled),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLease {
    pub id: LeaseId,
    pub kind: ResourceLeaseKind,
    pub acquired_in: SliceId,
    pub released_in: Option<SliceId>,
}

impl ResourceLease {
    #[must_use]
    pub const fn yield_safe(&self) -> bool {
        self.kind.yield_safe()
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.released_in.is_none()
    }

    #[must_use]
    pub const fn is_balanced(&self) -> bool {
        match self.released_in {
            Some(released) => released.0 >= self.acquired_in.0,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_enum_complete() {
        assert_eq!(InterruptPolicy::ALL.len(), 3);
        assert!(InterruptPolicy::ALL.contains(&InterruptPolicy::Enabled));
        assert!(InterruptPolicy::ALL.contains(&InterruptPolicy::ShortCriticalSection));
        assert!(InterruptPolicy::ALL.contains(&InterruptPolicy::Disabled));
    }

    #[test]
    fn lease_yield_safety_table() {
        let table = [
            (
                ResourceLeaseKind::RomWindow(RomWindowBinding { bank: 1 }),
                false,
            ),
            (
                ResourceLeaseKind::SramPage(SramPageBinding {
                    page: 0,
                    enabled: true,
                }),
                true,
            ),
            (ResourceLeaseKind::Overlay(OverlayId(7)), false),
            (
                ResourceLeaseKind::InterruptMask(InterruptPolicy::Enabled),
                true,
            ),
            (
                ResourceLeaseKind::InterruptMask(InterruptPolicy::ShortCriticalSection),
                false,
            ),
            (
                ResourceLeaseKind::InterruptMask(InterruptPolicy::Disabled),
                false,
            ),
        ];

        for (kind, expected) in table {
            assert_eq!(kind.yield_safe(), expected);
        }
    }

    #[test]
    fn lease_id_distinct_from_slice_id() {
        fn takes_lease(_: LeaseId) {}
        fn takes_slice(_: SliceId) {}
        fn takes_overlay(_: OverlayId) {}

        takes_lease(LeaseId(1));
        takes_slice(SliceId(1));
        takes_overlay(OverlayId(1));
        assert_ne!(
            core::any::type_name::<LeaseId>(),
            core::any::type_name::<SliceId>()
        );
        assert_ne!(
            core::any::type_name::<LeaseId>(),
            core::any::type_name::<OverlayId>()
        );
    }

    #[test]
    fn lease_id_uniqueness() {
        assert!(LeaseId::ALL_VALUES_FIT);
        assert!(SliceId::ALL_VALUES_FIT);
        assert!(OverlayId::ALL_VALUES_FIT);
    }

    #[test]
    fn serde_round_trip() {
        let leases = [
            ResourceLease {
                id: LeaseId(1),
                kind: ResourceLeaseKind::RomWindow(RomWindowBinding { bank: 0x01FF }),
                acquired_in: SliceId(9),
                released_in: Some(SliceId(10)),
            },
            ResourceLease {
                id: LeaseId(2),
                kind: ResourceLeaseKind::SramPage(SramPageBinding {
                    page: 2,
                    enabled: false,
                }),
                acquired_in: SliceId(9),
                released_in: Some(SliceId(10)),
            },
            ResourceLease {
                id: LeaseId(3),
                kind: ResourceLeaseKind::Overlay(OverlayId(7)),
                acquired_in: SliceId(9),
                released_in: None,
            },
            ResourceLease {
                id: LeaseId(4),
                kind: ResourceLeaseKind::InterruptMask(InterruptPolicy::Enabled),
                acquired_in: SliceId(9),
                released_in: Some(SliceId(9)),
            },
            ResourceLease {
                id: LeaseId(5),
                kind: ResourceLeaseKind::InterruptMask(InterruptPolicy::ShortCriticalSection),
                acquired_in: SliceId(9),
                released_in: Some(SliceId(10)),
            },
            ResourceLease {
                id: LeaseId(6),
                kind: ResourceLeaseKind::InterruptMask(InterruptPolicy::Disabled),
                acquired_in: SliceId(9),
                released_in: Some(SliceId(10)),
            },
        ];

        for lease in leases {
            let encoded = serde_json::to_value(lease).expect("lease serializes");
            let decoded: ResourceLease =
                serde_json::from_value(encoded.clone()).expect("lease deserializes");
            assert_eq!(decoded, lease);
            assert_eq!(
                serde_json::to_value(decoded).expect("lease reserializes"),
                encoded
            );
        }
    }

    #[test]
    fn balanced_predicate() {
        let released_later = ResourceLease {
            id: LeaseId(1),
            kind: ResourceLeaseKind::Overlay(OverlayId(1)),
            acquired_in: SliceId(4),
            released_in: Some(SliceId(5)),
        };
        let released_before = ResourceLease {
            released_in: Some(SliceId(3)),
            ..released_later
        };
        let active = ResourceLease {
            released_in: None,
            ..released_later
        };

        assert!(released_later.is_balanced());
        assert!(!released_before.is_balanced());
        assert!(!active.is_balanced());
    }

    #[test]
    fn active_predicate() {
        let active = ResourceLease {
            id: LeaseId(1),
            kind: ResourceLeaseKind::Overlay(OverlayId(1)),
            acquired_in: SliceId(1),
            released_in: None,
        };

        assert!(active.is_active());
        assert!(
            !ResourceLease {
                released_in: Some(SliceId(1)),
                ..active
            }
            .is_active()
        );
    }

    #[test]
    fn sram_page_disabled_state() {
        let kind = ResourceLeaseKind::SramPage(SramPageBinding {
            page: 3,
            enabled: false,
        });

        assert!(kind.yield_safe());
    }
}

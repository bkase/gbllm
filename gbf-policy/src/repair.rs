//! Feasibility repair policy schema.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepairProposalId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairPolicy {
    pub max_refinement_iters: u8,
    pub allow_placement_profile_fallback: bool,
    pub allow_trace_demotion: bool,
    pub allow_overlay_promotion: bool,
    pub allow_recompute_promotion: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RepairPolicyProfile {
    Bringup,
    Default,
    TraceInvariant,
    Recovery,
}

impl RepairPolicy {
    #[must_use]
    pub const fn for_profile(profile: RepairPolicyProfile) -> Self {
        match profile {
            RepairPolicyProfile::Bringup => Self {
                max_refinement_iters: 1,
                allow_placement_profile_fallback: false,
                allow_trace_demotion: false,
                allow_overlay_promotion: false,
                allow_recompute_promotion: false,
            },
            RepairPolicyProfile::Default => Self {
                max_refinement_iters: 4,
                allow_placement_profile_fallback: true,
                allow_trace_demotion: true,
                allow_overlay_promotion: true,
                allow_recompute_promotion: true,
            },
            RepairPolicyProfile::TraceInvariant => Self {
                max_refinement_iters: 2,
                allow_placement_profile_fallback: false,
                allow_trace_demotion: false,
                allow_overlay_promotion: false,
                allow_recompute_promotion: false,
            },
            RepairPolicyProfile::Recovery => Self {
                max_refinement_iters: 6,
                allow_placement_profile_fallback: true,
                allow_trace_demotion: true,
                allow_overlay_promotion: true,
                allow_recompute_promotion: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_policy_defaults_match_planv0_table() {
        assert_eq!(
            RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
            RepairPolicy {
                max_refinement_iters: 1,
                allow_placement_profile_fallback: false,
                allow_trace_demotion: false,
                allow_overlay_promotion: false,
                allow_recompute_promotion: false,
            }
        );
        assert_eq!(
            RepairPolicy::for_profile(RepairPolicyProfile::Default),
            RepairPolicy {
                max_refinement_iters: 4,
                allow_placement_profile_fallback: true,
                allow_trace_demotion: true,
                allow_overlay_promotion: true,
                allow_recompute_promotion: true,
            }
        );
        assert_eq!(
            RepairPolicy::for_profile(RepairPolicyProfile::TraceInvariant),
            RepairPolicy {
                max_refinement_iters: 2,
                allow_placement_profile_fallback: false,
                allow_trace_demotion: false,
                allow_overlay_promotion: false,
                allow_recompute_promotion: false,
            }
        );
        assert_eq!(
            RepairPolicy::for_profile(RepairPolicyProfile::Recovery),
            RepairPolicy {
                max_refinement_iters: 6,
                allow_placement_profile_fallback: true,
                allow_trace_demotion: true,
                allow_overlay_promotion: true,
                allow_recompute_promotion: true,
            }
        );
    }

    #[test]
    fn repair_policy_round_trip() {
        let policy = RepairPolicy::for_profile(RepairPolicyProfile::Default);
        let expected = serde_json::json!({
            "max_refinement_iters": 4,
            "allow_placement_profile_fallback": true,
            "allow_trace_demotion": true,
            "allow_overlay_promotion": true,
            "allow_recompute_promotion": true
        });

        let encoded = serde_json::to_string(&policy).expect("repair policy serializes");
        let decoded: RepairPolicy =
            serde_json::from_str(&encoded).expect("repair policy deserializes");

        assert_eq!(decoded, policy);
        assert_eq!(
            serde_json::to_value(policy).expect("repair policy serializes"),
            expected
        );
    }

    #[test]
    fn repair_policy_rejects_unknown_field() {
        let mut value =
            serde_json::to_value(RepairPolicy::for_profile(RepairPolicyProfile::Default))
                .expect("repair policy serializes");
        value["unexpected"] = serde_json::json!(true);

        assert!(serde_json::from_value::<RepairPolicy>(value).is_err());
    }
}

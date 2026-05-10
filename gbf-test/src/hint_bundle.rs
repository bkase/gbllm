//! Hint-bundle fixtures.

use gbf_artifact::export_facts::ExportFacts;
use gbf_artifact::hint_bundle::*;
use gbf_artifact::preferences::CompilePreferences;
use gbf_policy::TraceProbeId;
use gbf_policy::compile::{CompileKnobId, ConstraintValue, PlacementProfile};

pub struct HintBundleBuilder {
    bundle: HintBundle,
}

impl HintBundleBuilder {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            bundle: HintBundle::empty(),
        }
    }

    #[must_use]
    pub fn with_facts(mut self, facts: ExportFacts) -> Self {
        self.bundle.facts = facts;
        self
    }

    #[must_use]
    pub fn with_preferences(mut self, preferences: CompilePreferences) -> Self {
        self.bundle.preferences = preferences;
        self
    }

    #[must_use]
    pub fn with_constraint(mut self, entry: BuildConstraintEntry) -> Self {
        self.bundle.constraints.entries.push(entry);
        self
    }

    #[must_use]
    pub fn build(self) -> HintBundle {
        self.bundle
    }
}

#[must_use]
pub fn empty_hint_bundle_fixture() -> HintBundle {
    HintBundleBuilder::empty().build()
}

#[must_use]
pub fn build_constraint_entry_fixture() -> BuildConstraintEntry {
    BuildConstraintEntry {
        provenance_id: TraceProbeId(1),
        knob: CompileKnobId::Placement,
        path: None,
        value: ConstraintValue::PlacementProfile {
            value: PlacementProfile::Budgeted,
        },
        evidence: Vec::new(),
        scope: EvidenceScope::WholeArtifact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_empty_matches_fixture_constant() {
        assert_eq!(
            HintBundleBuilder::empty().build(),
            empty_hint_bundle_fixture()
        );
    }

    #[test]
    fn builder_supports_with_constraint_chaining() {
        let entry = build_constraint_entry_fixture();
        let bundle = HintBundleBuilder::empty()
            .with_constraint(entry.clone())
            .build();

        assert_eq!(bundle.constraints.entries, vec![entry]);
    }
}

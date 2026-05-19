//! Canonical encoding for sorted denotational observations.

use gbf_foundation::CanonicalJson;
use gbf_workload::PromptId;
use serde::Serialize;

use super::{
    DenotationalDeterminismClass, Observation, OracleError, ReferenceObservations,
    SemanticCheckpoint,
};

const OBSERVATIONS_SCHEMA: &str = "reference_observations.canonical.v1";

/// Encoder for `ReferenceObservations`.
#[derive(Debug, Clone, Copy)]
pub struct ReferenceObservationsCanonical;

impl ReferenceObservationsCanonical {
    /// Encode observations as S1 canonical JSON in sorted-key row order.
    pub fn to_vec(observations: &ReferenceObservations) -> Result<Vec<u8>, OracleError> {
        let payload = ObservationsPayload::from_observations(observations);
        CanonicalJson::to_vec(&payload).map_err(OracleError::CanonicalJson)
    }

    /// Return the canonical row payload used by tests and product hashing.
    #[must_use]
    pub fn rows(observations: &ReferenceObservations) -> Vec<ObservationRow> {
        ObservationsPayload::from_observations(observations).observations
    }
}

/// Product-hash payload; backend metadata is intentionally outside this hash so
/// real and fallback products share the same observation identity.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductHashPayload {
    schema: &'static str,
    determinism_class: DenotationalDeterminismClass,
    observations: Vec<ObservationRow>,
}

impl ProductHashPayload {
    pub(crate) fn from_observations(
        observations: &ReferenceObservations,
        determinism_class: DenotationalDeterminismClass,
    ) -> Self {
        Self {
            schema: "denotational_oracle_product_hash.v1",
            determinism_class,
            observations: ReferenceObservationsCanonical::rows(observations),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct ObservationsPayload {
    schema: &'static str,
    observations: Vec<ObservationRow>,
}

impl ObservationsPayload {
    fn from_observations(observations: &ReferenceObservations) -> Self {
        Self {
            schema: OBSERVATIONS_SCHEMA,
            observations: observations
                .iter()
                .map(|((prompt_id, checkpoint, step), observation)| {
                    ObservationRow::new(prompt_id, *checkpoint, *step, observation.clone())
                })
                .collect(),
        }
    }
}

/// Canonical row for one `(prompt, checkpoint, step)` observation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRow {
    /// Prompt id.
    pub prompt_id: String,
    /// Semantic checkpoint.
    pub checkpoint: SemanticCheckpoint,
    /// Generated-step index.
    pub step: u32,
    /// Checkpoint-specific observation.
    pub observation: Observation,
}

impl ObservationRow {
    fn new(
        prompt_id: &PromptId,
        checkpoint: SemanticCheckpoint,
        step: u32,
        observation: Observation,
    ) -> Self {
        Self {
            prompt_id: prompt_id.to_string(),
            checkpoint,
            step,
            observation,
        }
    }
}

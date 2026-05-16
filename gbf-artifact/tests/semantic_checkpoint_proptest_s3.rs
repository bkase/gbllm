use gbf_artifact::{CheckpointRole, SemanticCheckpointSchema};
use proptest::prelude::*;

proptest! {
    #[test]
    fn semantic_checkpoint_schema_variants_round_trip(checkpoint in semantic_checkpoint_strategy()) {
        let encoded = serde_json::to_string(&checkpoint).expect("checkpoint serializes");
        let decoded: SemanticCheckpointSchema =
            serde_json::from_str(&encoded).expect("checkpoint decodes");
        prop_assert_eq!(decoded, checkpoint);
    }

    #[test]
    fn semantic_checkpoint_role_variants_round_trip(role in checkpoint_role_strategy()) {
        let encoded = serde_json::to_string(&role).expect("role serializes");
        let decoded: CheckpointRole = serde_json::from_str(&encoded).expect("role decodes");
        prop_assert_eq!(decoded, role);
    }
}

fn semantic_checkpoint_strategy() -> impl Strategy<Value = SemanticCheckpointSchema> {
    prop_oneof![
        Just(SemanticCheckpointSchema::PostEmbedding),
        Just(SemanticCheckpointSchema::PostLogits),
        Just(SemanticCheckpointSchema::PostDecode),
    ]
}

fn checkpoint_role_strategy() -> impl Strategy<Value = CheckpointRole> {
    prop_oneof![
        Just(CheckpointRole::ObservationOnly),
        Just(CheckpointRole::AgreementGated),
    ]
}

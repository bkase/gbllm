mod denotational_support;

use gbf_artifact::VOCAB_SIZE;
use gbf_oracle::denotational::{
    Observation, ReferenceObservations, ReferenceObservationsCanonical, SemanticCheckpoint,
};
use gbf_workload::PromptId;
use serde_json::{Value, json};

#[test]
fn observations_canonical_sorts_keys_and_normalizes_floats() {
    let mut observations = ReferenceObservations::new();
    observations
        .insert(
            PromptId::from("prompt-b"),
            SemanticCheckpoint::PostDecode,
            2,
            Observation::post_decode(7).unwrap(),
        )
        .unwrap();
    observations
        .insert(
            PromptId::from("prompt-a"),
            SemanticCheckpoint::PostEmbedding,
            0,
            Observation::post_embedding(Some(vec![-0.0, 1.25])).unwrap(),
        )
        .unwrap();
    observations
        .insert(
            PromptId::from("prompt-a"),
            SemanticCheckpoint::PostLogits,
            0,
            Observation::post_logits(vec![0.0; VOCAB_SIZE]).unwrap(),
        )
        .unwrap();

    let rows = ReferenceObservationsCanonical::rows(&observations);
    assert_eq!(rows[0].prompt_id, "prompt-a");
    assert_eq!(rows[0].checkpoint, SemanticCheckpoint::PostEmbedding);
    assert_eq!(rows[1].prompt_id, "prompt-a");
    assert_eq!(rows[1].checkpoint, SemanticCheckpoint::PostLogits);
    assert_eq!(rows[2].prompt_id, "prompt-b");

    let encoded = observations
        .canonical_bytes()
        .expect("observations canonicalize");
    let payload: Value =
        serde_json::from_slice(&encoded).expect("canonical observations parse as JSON");
    assert_eq!(
        payload["schema"],
        json!("reference_observations.canonical.v1")
    );

    let rows = payload["observations"]
        .as_array()
        .expect("observations is an array");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        json!({
            "prompt_id": "prompt-a",
            "checkpoint": "post_embedding",
            "step": 0,
            "observation": {
                "checkpoint": "post_embedding",
                "hidden_state": [0.0, 1.25],
            },
        })
    );
    let hidden_zero = rows[0]["observation"]["hidden_state"][0]
        .as_f64()
        .expect("hidden_state[0] is numeric");
    assert_eq!(hidden_zero, 0.0);
    assert!(
        !hidden_zero.is_sign_negative(),
        "canonical float normalization must rewrite -0.0 to +0.0"
    );
    assert_eq!(
        rows[1],
        json!({
            "prompt_id": "prompt-a",
            "checkpoint": "post_logits",
            "step": 0,
            "observation": {
                "checkpoint": "post_logits",
                "logits": vec![0.0_f32; VOCAB_SIZE],
            },
        })
    );
    assert_eq!(
        rows[2],
        json!({
            "prompt_id": "prompt-b",
            "checkpoint": "post_decode",
            "step": 2,
            "observation": {
                "checkpoint": "post_decode",
                "token": 7,
            },
        })
    );
}

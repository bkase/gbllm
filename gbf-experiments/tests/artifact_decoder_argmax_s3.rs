#![cfg(all(feature = "s3", feature = "s3-oracle-real"))]

mod artifact_oracle_s3_support;

use artifact_oracle_s3_support::{argmax, eos_trigger_prompt, prompt_for_decode};
use gbf_artifact::{EOS_ID, ModelArtifact, PayloadRole, TextCharSeq};
use gbf_oracle::artifact::{ArtifactDecoder, OracleError, quant_spec_resolver_logits};

#[test]
fn artifact_decoder_argmax_s3() {
    let artifact = artifact_oracle_s3_support::fixture_artifact(0);
    let prompt = prompt_for_decode();
    let decoder = ArtifactDecoder::new(&artifact);
    let first = decoder.decode_argmax(&prompt, 16, false);

    for _ in 0..10 {
        assert_eq!(decoder.decode_argmax(&prompt, 16, false), first);
    }
    assert_eq!(first.decode_log.len(), 16);
    assert_eq!(first.generated.len(), 16);
    assert!(!first.terminal_eos_seen);

    let mut prefix = prompt.as_slice().to_vec();
    for step in &first.decode_log {
        let prefix_seq = TextCharSeq::new(prefix.clone()).expect("prefix remains valid");
        let logits = quant_spec_resolver_logits(&artifact, &prefix_seq).expect("logits resolve");
        let (expected_token, expected_logit_max) = argmax(&logits);
        assert_eq!(step.token, expected_token);
        assert_eq!(step.logit_max.to_bits(), expected_logit_max.to_bits());
        prefix.push(step.token);
    }

    let eos_result = decoder.decode_argmax(&eos_trigger_prompt(), 4, true);
    assert!(eos_result.terminal_eos_seen);
    assert_eq!(eos_result.generated.len(), 0);
    assert_eq!(eos_result.decode_log.len(), 1);

    let max_chars_result = decoder.decode_argmax(&prompt, 4, true);
    assert!(!max_chars_result.terminal_eos_seen);
    assert_eq!(max_chars_result.generated.len(), 4);
    assert_eq!(max_chars_result.decode_log.len(), 4);
}

#[test]
fn artifact_decoder_soft_stops_on_eos_with_stop_on_eos_false() {
    let artifact = artifact_oracle_s3_support::fixture_artifact(0);
    let decoder = ArtifactDecoder::new(&artifact);
    let result = decoder.decode_argmax(&eos_trigger_prompt(), 4, false);

    assert!(result.terminal_eos_seen);
    assert_eq!(result.generated.len(), 0);
    assert_eq!(result.decode_log.len(), 1);
    assert_eq!(result.decode_log[0].token, EOS_ID);
}

#[test]
fn artifact_decoder_try_new_reports_missing_quant_spec_coverage() {
    let mut artifact = artifact_oracle_s3_support::fixture_artifact(0);
    let missing_id = artifact
        .core
        .tensors
        .iter()
        .find(|tensor| tensor.payload_role == PayloadRole::DeployableWeight)
        .expect("fixture has deployable weight")
        .id
        .clone();
    artifact
        .core
        .quant
        .weight_quant
        .remove(&missing_id)
        .expect("fixture quant covers deployable weight");
    rehash_artifact(&mut artifact);

    let error = ArtifactDecoder::try_new(&artifact).expect_err("missing coverage is rejected");
    match error {
        OracleError::QuantSpecCoverageMissing { tensor_id } => assert_eq!(tensor_id, missing_id),
        other => panic!("expected QuantSpecCoverageMissing, got {other:?}"),
    }
}

#[test]
fn artifact_decoder_try_new_validates_tied_alias_contract() {
    let mut artifact = artifact_oracle_s3_support::fixture_artifact(0);
    let classifier_id = artifact
        .core
        .tensors
        .iter()
        .find(|tensor| tensor.id.as_str() == "tensor.linear.weight")
        .expect("fixture has classifier/linear weight")
        .id
        .clone();
    artifact
        .core
        .tied_embedding_alias
        .as_mut()
        .expect("fixture has tied alias")
        .classifier_canonical_id = classifier_id.clone();
    rehash_artifact(&mut artifact);

    let error = ArtifactDecoder::try_new(&artifact).expect_err("invalid tied alias is rejected");
    match error {
        OracleError::TiedAliasNotShared {
            classifier_id: observed,
            ..
        } => assert_eq!(observed, classifier_id),
        other => panic!("expected TiedAliasNotShared, got {other:?}"),
    }
}

fn rehash_artifact(artifact: &mut ModelArtifact) {
    artifact.artifact_self_hash = artifact
        .compute_self_hash()
        .expect("mutated fixture can be rehashed");
}

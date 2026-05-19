#![cfg(all(feature = "s3", feature = "s3-oracle-adversarial"))]

mod artifact_oracle_s3_support;

use artifact_oracle_s3_support::max_abs_diff;
use gbf_oracle::artifact::adversarial_fixture::{
    CANONICAL_LINEAR_WEIGHT_ID, SHADOW_LINEAR_WEIGHT_ID, adversarial_artifact_fixture,
    adversarial_fixture_is_structurally_separating, name_resolver_logits_for_fixture,
    separating_prompt,
};
use gbf_oracle::artifact::quant_spec_resolver_logits;

#[test]
fn oracle_adversarial_fixture_separating_s3() {
    let artifact = adversarial_artifact_fixture();
    assert!(
        artifact
            .core
            .tensors
            .iter()
            .any(|tensor| tensor.id.as_str() == CANONICAL_LINEAR_WEIGHT_ID)
    );
    assert!(
        artifact
            .core
            .tensors
            .iter()
            .any(|tensor| tensor.id.as_str() == SHADOW_LINEAR_WEIGHT_ID)
    );
    assert!(
        artifact
            .core
            .quant
            .weight_quant
            .keys()
            .any(|tensor_id| { tensor_id.as_str() == CANONICAL_LINEAR_WEIGHT_ID })
    );
    assert!(
        !artifact
            .core
            .quant
            .weight_quant
            .keys()
            .any(|tensor_id| { tensor_id.as_str() == SHADOW_LINEAR_WEIGHT_ID })
    );

    let prompt = separating_prompt();
    let quant = quant_spec_resolver_logits(&artifact, &prompt).expect("QuantSpec logits");
    let name = name_resolver_logits_for_fixture(&artifact, &prompt).expect("name logits");
    assert!(max_abs_diff(&quant, &name) > 0.0);
    assert!(adversarial_fixture_is_structurally_separating().expect("fixture self-test runs"));
}

use gbf_train::loss::composer::{
    InertClassification, LossTermApplicability, LossTerms, PhaseEffectiveLossWeights,
    PhaseEffectiveLossWeightsValues, TrainingLossUnit, compose,
};
use serde_json::{Value, json};

#[test]
fn inert_discipline_toy0_router_terms_are_structurally_inert() {
    let composed = compose(
        LossTerms {
            lm_loss_next_byte_nats: 0.5,
            distill_loss_raw_nats: Some(0.0),
            range_loss_raw: Some(0.0),
            zero_loss_raw: Some(0.0),
            ..LossTerms::default()
        },
        weights(0.0, 0.01, 0.001, 0.0, 0.01, 0.0001),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert_eq!(
        composed.inert_classification.balance,
        InertClassification::StructurallyInert
    );
    assert_eq!(
        composed.inert_classification.zrouter,
        InertClassification::StructurallyInert
    );
    assert_eq!(
        composed.inert_classification.switch,
        InertClassification::StructurallyInert
    );
}

#[test]
fn inert_discipline_nodistill_records_raw_but_zero_weight() {
    let composed = compose(
        LossTerms {
            lm_loss_next_byte_nats: 0.5,
            distill_loss_raw_nats: Some(2.5),
            range_loss_raw: Some(0.3),
            zero_loss_raw: Some(0.2),
            ..LossTerms::default()
        },
        weights(0.0, 0.0, 0.0, 0.0, 0.01, 0.0001),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert_eq!(
        composed.inert_classification.distill,
        InertClassification::ComputedDisabled {
            raw: 2.5,
            weighted: 0.0
        }
    );
}

#[test]
fn inert_discipline_phase_d_qat_terms_are_enabled() {
    let composed = compose(
        LossTerms {
            lm_loss_next_byte_nats: 0.5,
            distill_loss_raw_nats: Some(2.0),
            range_loss_raw: Some(0.3),
            zero_loss_raw: Some(0.2),
            ..LossTerms::default()
        },
        weights(1.0, 0.0, 0.0, 0.0, 0.01, 0.0001),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert_eq!(
        composed.inert_classification.range,
        InertClassification::Enabled {
            raw: 0.3,
            weighted: 0.003
        }
    );
    assert_eq!(
        composed.inert_classification.zero,
        InertClassification::Enabled {
            raw: 0.2,
            weighted: 0.00002
        }
    );
}

#[test]
fn toy0_classification_snapshot_serializes_null_null_for_router_terms() {
    let composed = compose(
        LossTerms {
            lm_loss_next_byte_nats: 0.5,
            distill_loss_raw_nats: Some(2.5),
            range_loss_raw: Some(0.3),
            zero_loss_raw: Some(0.2),
            ..LossTerms::default()
        },
        weights(0.0, 0.01, 0.001, 0.0, 0.01, 0.0001),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();
    let report = json!({
        "topology": "toy0",
        "build_kind": "s2_ternary_nodistill",
        "phase": "PhaseC",
        "total_loss": composed.total_loss,
        "eval_points": composed.inert_classification.eval_points(),
    });

    assert_eq!(report["eval_points"]["balance"]["raw"], Value::Null);
    assert_eq!(report["eval_points"]["balance"]["weighted"], Value::Null);
    assert_eq!(report["eval_points"]["zrouter"]["raw"], Value::Null);
    assert_eq!(report["eval_points"]["switch"]["weighted"], Value::Null);
    assert_eq!(report["eval_points"]["distill"]["weighted"], json!(0.0));
    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!("inert_s2__toy0_classification", pretty_json(&report));
    });
}

fn weights(
    lambda_distill: f32,
    lambda_balance: f32,
    lambda_zrouter: f32,
    lambda_switch: f32,
    lambda_range: f32,
    lambda_zero: f32,
) -> PhaseEffectiveLossWeights {
    PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
        lambda_distill,
        lambda_balance,
        lambda_zrouter,
        lambda_switch,
        lambda_range,
        lambda_zero,
        lambda_shape: 0.0,
        lambda_overflow: 0.0,
    })
    .unwrap()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot report serializes")
}

use gbf_experiments::s7::falsify::{
    S7FalsificationCase, S7FalsificationEvidence, f9_expert_block_qat_grad_dead,
};
use gbf_experiments::s7::outcome::{S7Decision, S7Outcome};
use gbf_model::qat::{
    ActFakeQuant, ActivationQuantFormat, ActivationRange, ActivationRangeMode, ExpertBlockQat,
    ExpertForwardOptions, ExpertQat, MatrixShape, Q8_8Scale, TernaryLinearQat, TernaryThreshold,
};
use gbf_train::adapter::burn::{
    BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::qat::ExpertBlockBurnQat;

type B = BurnNdArrayAutodiffBackend;

#[test]
fn f9_expert_block_qat_grad_dead_refutes_h8() {
    let device = BurnDevice::<B>::default();
    let layer = ExpertBlockBurnQat::<B>::from_core(fixture_block(), &device).expect("Burn expert");
    let input = float_tensor_from_vec::<B, 1>(vec![0.25, 0.5], [2], &device)
        .expect("input")
        .require_grad();

    let output = layer
        .forward(input, 0, ExpertForwardOptions::hard_quantized_train())
        .expect("expert forward");
    let gradients = output.sum().backward();
    let up_grad = float_tensor_into_vec(
        layer.experts()[0]
            .up_projection()
            .full_precision_weights()
            .grad(&gradients)
            .expect("up.weight should receive gradient"),
    )
    .expect("up.weight gradient values");
    assert!(
        up_grad.iter().any(|value| value.abs() > 0.0),
        "up.weight gradient must be non-zero: {up_grad:?}"
    );

    let evidence = f9_expert_block_qat_grad_dead::broken_substitute();
    assert!(matches!(
        evidence,
        S7FalsificationEvidence::ExpertBlockQatGradDead {
            up_weight_grad_nonzero: false,
        }
    ));
    assert!(evidence.refutes_expected());

    crate::assert_s7_case(
        S7FalsificationCase::ExpertBlockQatGradDead,
        S7Outcome::FailBurnGrad,
        S7Decision::Halt {
            reason: "burn-adapter-broken",
        },
        f9_expert_block_qat_grad_dead::run,
    );
}

fn fixture_block() -> ExpertBlockQat {
    ExpertBlockQat::new(vec![fixture_expert()], None).expect("fixture block")
}

fn fixture_expert() -> ExpertQat {
    ExpertQat::new(
        ternary_linear(
            3,
            2,
            vec![
                1.0, 0.0, //
                0.0, 1.0, //
                0.25, 0.25,
            ],
            None,
        ),
        activation(),
        ternary_linear(
            2,
            3,
            vec![
                1.0, 0.0, 0.0, //
                0.0, -1.0, 0.0,
            ],
            None,
        ),
    )
    .expect("fixture expert")
}

fn activation() -> ActFakeQuant {
    ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).expect("range")),
        ActivationQuantFormat::Int8,
    )
    .expect("activation")
}

fn ternary_linear(
    output_rows: usize,
    input_cols: usize,
    weights: Vec<f32>,
    bias: Option<Vec<f32>>,
) -> TernaryLinearQat {
    TernaryLinearQat::new(
        MatrixShape::new(output_rows, input_cols).expect("shape"),
        weights,
        bias,
        vec![TernaryThreshold::new(0.5).expect("threshold"); output_rows],
        vec![Q8_8Scale::from_f32(1.0).expect("scale"); output_rows],
    )
    .expect("ternary linear")
}

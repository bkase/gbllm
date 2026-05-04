use gbf_artifact::norm_plan::NormExportParams;
use gbf_model::qat::{
    ActFakeQuant, ActivationForwardMode, ActivationQuantFormat, ActivationRange,
    ActivationRangeMode, AffineParams, ClippedActivation, DenseBranchProjection, EmaDecay,
    ExpertBlockQat, ExpertForwardOptions, ExpertQat, ExpertQatForwardMode, LutSpec, MatrixShape,
    NormApproxPlan, NormApproxQat, NormClip, Q8_8Scale, RouterAuxLossWeights, RouterForwardOptions,
    RouterShape, SharedDenseBranch, TernaryLinearQat, TernaryThreshold, TernaryValue, TileRmsSpec,
    Top1RouterQat,
};
use gbf_test::fixtures::{TINY_D_FF, TINY_D_MODEL, make_tiny_model};
use gbf_test::helpers::assert_f32_slice_close;
use gbf_train::adapter::burn::{
    BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::qat::{ActFakeQuantBurnQat, NormApproxBurnQat, TernaryLinearBurnQat};

const TOL: f32 = 1.0e-5;

#[test]
fn qat_tests_ternary_known_4x4_forward_matches_manual_projection() {
    let layer = ternary_4x4_layer();
    let input = [1.0, 2.0, -1.0, 0.5];

    let actual = layer.inference_forward(&input).unwrap();
    let expected = [-0.25, -0.75, 2.25, 0.25];

    assert_f32_slice_close(&actual, &expected, TOL, TOL);
}

#[test]
fn qat_tests_ternary_export_round_trip_recomputes_forward() {
    let layer = ternary_4x4_layer();
    let export = layer.export_canonical();
    let input = [0.25, -0.5, 1.5, 2.0];

    let projected = project_export_weights(export.ternary_values(), export.scales());
    let round_trip = manual_linear(export.shape(), &projected, export.bias_values(), &input);
    let actual = layer.inference_forward(&input).unwrap();

    assert_eq!(export.plan(), TernaryLinearQat::canonical_weight_plan());
    assert_f32_slice_close(&actual, &round_trip, TOL, TOL);
    assert_eq!(
        export.ternary_values(),
        &[
            TernaryValue::Negative,
            TernaryValue::Zero,
            TernaryValue::Positive,
            TernaryValue::Zero,
            TernaryValue::Positive,
            TernaryValue::Negative,
            TernaryValue::Zero,
            TernaryValue::Positive,
            TernaryValue::Zero,
            TernaryValue::Positive,
            TernaryValue::Negative,
            TernaryValue::Zero,
            TernaryValue::Positive,
            TernaryValue::Zero,
            TernaryValue::Positive,
            TernaryValue::Negative,
        ]
    );
}

#[test]
fn qat_tests_ternary_per_row_scales_and_burn_gradients_are_owned() {
    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let core = TernaryLinearQat::new(
        MatrixShape::new(1, 3).unwrap(),
        vec![-2.0, -0.1, 0.6],
        None,
        vec![TernaryThreshold::new(0.5).unwrap()],
        vec![Q8_8Scale::from_f32(0.25).unwrap()],
    )
    .unwrap();
    let layer = TernaryLinearBurnQat::<B>::from_core(core, &device).unwrap();
    let input = float_tensor_from_vec::<B, 1>(vec![1.0, 2.0, 4.0], [3], &device).unwrap();

    let output = layer.fake_quant_forward(input).unwrap();
    let output_values = float_tensor_into_vec(output.clone().inner()).unwrap();
    let gradients = output.sum().backward();

    let weight_grad = float_tensor_into_vec(
        layer
            .full_precision_weights()
            .grad(&gradients)
            .expect("weight gradient should exist"),
    )
    .unwrap();
    let threshold_grad = float_tensor_into_vec(
        layer
            .thresholds()
            .grad(&gradients)
            .expect("threshold gradient should exist"),
    )
    .unwrap();
    let scale_grad = float_tensor_into_vec(
        layer
            .scale_factors()
            .grad(&gradients)
            .expect("scale gradient should exist"),
    )
    .unwrap();
    let export = layer.export_canonical().unwrap();

    assert_eq!(output_values, vec![0.75]);
    assert_eq!(weight_grad, vec![0.25, 0.5, 1.0]);
    assert_eq!(threshold_grad, vec![-0.25]);
    assert_eq!(scale_grad, vec![3.0]);
    assert_eq!(export.scales(), &[Q8_8Scale::from_f32(0.25).unwrap()]);
}

#[test]
fn qat_tests_activation_clips_and_quantizes_unsigned_values() {
    let quant = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
        ActivationQuantFormat::UInt4,
    )
    .unwrap();
    let input = [-2.0, -0.5, 0.0, 0.5, 2.0];

    let actual = quant
        .inference_forward(&input, ActivationForwardMode::Train)
        .unwrap();
    let expected = input
        .iter()
        .copied()
        .map(|value| manual_unsigned_fake_quant(value, -1.0, 1.0, 15.0))
        .collect::<Vec<_>>();

    assert_f32_slice_close(&actual, &expected, TOL, TOL);
}

#[test]
fn qat_tests_activation_eval_passthrough_and_export_range_are_explicit() {
    let quant = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
        ActivationQuantFormat::UInt8,
    )
    .unwrap()
    .with_eval_passthrough(true);
    let input = [-1.0, 0.25, 2.0];

    assert_eq!(
        quant.export_range(),
        ActivationRange::new(0.0, 1.0).unwrap()
    );
    assert_eq!(
        quant
            .inference_forward(&input, ActivationForwardMode::Eval)
            .unwrap(),
        input
    );
    assert!(!quant.forward_spec(ActivationForwardMode::Eval).enabled());
    assert!(quant.forward_spec(ActivationForwardMode::Train).enabled());
}

#[test]
fn qat_tests_activation_range_tracking_and_burn_input_gradients() {
    type B = BurnNdArrayAutodiffBackend;

    let mut ema = ActFakeQuant::new(
        ActivationRangeMode::Ema {
            range: ActivationRange::new(-1.0, 1.0).unwrap(),
            decay: EmaDecay::new(0.5).unwrap(),
        },
        ActivationQuantFormat::Int8,
    )
    .unwrap();
    ema.update_ema_range(ActivationRange::new(0.0, 2.0).unwrap())
        .unwrap();
    assert_eq!(ema.export_range(), ActivationRange::new(-0.5, 1.5).unwrap());

    let device = BurnDevice::<B>::default();
    let core = ActFakeQuant::new(
        ActivationRangeMode::Fixed(ActivationRange::new(-1.0, 1.0).unwrap()),
        ActivationQuantFormat::UInt8,
    )
    .unwrap();
    let layer = ActFakeQuantBurnQat::from_core(core).unwrap();
    let input = float_tensor_from_vec::<B, 1>(vec![-2.0, -0.25, 0.25, 2.0], [4], &device)
        .unwrap()
        .require_grad();

    let output = layer.fake_quant_forward(input.clone(), ActivationForwardMode::Train);
    let gradients = output.sum().backward();
    let input_grad = input.grad(&gradients).expect("input gradient should exist");

    assert_eq!(
        float_tensor_into_vec(input_grad).unwrap(),
        vec![0.0, 1.0, 1.0, 0.0]
    );
}

#[test]
fn qat_tests_norm_affine_clip_lut_matches_manual_lookup() {
    let norm = NormApproxQat::new(NormApproxPlan::AffineClipLut {
        affine: AffineParams::new(2.0, 0.5).unwrap(),
        clip: NormClip::new(0.0, 1.0).unwrap(),
        lut: LutSpec::new(-1.0, 1.0, 5).unwrap(),
    });
    let input = [-1.2, -0.75, 0.2, 0.76, 1.5];

    let actual = norm.forward(&input).unwrap();
    let expected = vec![0.0, 0.0, 0.5, 1.0, 1.0];

    assert_f32_slice_close(&actual, &expected, TOL, TOL);
}

#[test]
fn qat_tests_norm_tile_rms_affine_clip_matches_reference() {
    let tile = TileRmsSpec::new(2, 1.0).unwrap();
    let affine = AffineParams::new(2.0, 0.1).unwrap();
    let clip = NormClip::new(-1.0, 1.0).unwrap();
    let norm = NormApproxQat::new(NormApproxPlan::TileRmsThenAffineClip { tile, affine, clip });
    let input = [3.0, 4.0, 0.0, 2.0];

    let actual = norm.forward(&input).unwrap();
    let expected = manual_tile_rms_affine_clip(&input, 2, 1.0, 2.0, 0.1, -1.0, 1.0);

    assert_f32_slice_close(&actual, &expected, TOL, TOL);
}

#[test]
fn qat_tests_norm_export_params_and_burn_gradients_match_contract() {
    type B = BurnNdArrayAutodiffBackend;

    let core = NormApproxQat::new(NormApproxPlan::AffineClipLut {
        affine: AffineParams::new(1.0, 0.0).unwrap(),
        clip: NormClip::new(-1.0, 1.0).unwrap(),
        lut: LutSpec::new(-1.0, 1.0, 3).unwrap(),
    });
    let export = core.export_norm_params();
    let NormExportParams::AffineClipLut { plan, lut_values } = export.params() else {
        panic!("expected affine clip LUT export params");
    };
    assert_eq!(plan.lut.entries, 3);
    assert_eq!(lut_values, &vec![-1.0, 0.0, 1.0]);

    let device = BurnDevice::<B>::default();
    let layer = NormApproxBurnQat::<B>::from_core(core, &device).unwrap();
    let input = float_tensor_from_vec::<B, 1>(vec![-2.0, 0.25, 2.0, 3.0], [4], &device)
        .unwrap()
        .require_grad();
    let output = layer.forward(input.clone()).unwrap();
    let gradients = output.clone().sum().backward();

    assert_eq!(
        float_tensor_into_vec(output.inner()).unwrap(),
        vec![-1.0, 0.0, 1.0, 1.0]
    );
    assert_eq!(
        float_tensor_into_vec(input.grad(&gradients).unwrap()).unwrap(),
        vec![0.0, 1.0, 0.0, 0.0]
    );
}

#[test]
fn qat_tests_router_top1_and_aux_losses_match_manual_logits() {
    let router = Top1RouterQat::new(
        RouterShape::new(2, 3, 2).unwrap(),
        vec![1.0, 0.0, 0.0, 1.0],
        None,
        vec![1.0, 0.0, 0.0, 1.0, -1.0, 1.0],
        None,
    )
    .unwrap();
    let input = [1.0, 2.0];

    let output = router
        .forward_stateless(&input, None, &RouterForwardOptions::hard_top1(3))
        .unwrap();
    let soft = softmax(&[1.0, 2.0, 1.0]);
    let z = (1.0_f32.exp() + 2.0_f32.exp() + 1.0_f32.exp()).ln();

    assert_eq!(output.expert_index(), 1);
    assert_f32_slice_close(output.logits(), &[1.0, 2.0, 1.0], TOL, TOL);
    assert_eq!(output.routing_weights(), &[0.0, 1.0, 0.0]);
    assert_f32_slice_close(output.soft_probs(), &soft, TOL, TOL);
    assert!((output.aux_losses().z_loss() - z * z).abs() <= TOL);
    assert!((output.aux_losses().token_balance_proxy_loss() - soft[1] * 3.0).abs() <= TOL);
}

#[test]
fn qat_tests_router_low_rank_projection_matches_manual_factorization() {
    let router = Top1RouterQat::new(
        RouterShape::new(3, 2, 2).unwrap(),
        vec![1.0, -1.0, 0.5, 0.0, 2.0, 1.0],
        Some(vec![0.25, -0.5]),
        vec![2.0, -1.0, -0.5, 0.25],
        Some(vec![0.125, -0.25]),
    )
    .unwrap();
    let input = [2.0, -1.0, 0.5];
    let hidden = [
        1.0 * 2.0 + -1.0 * -1.0 + 0.5 * 0.5 + 0.25,
        0.0 * 2.0 - 2.0 + 1.0 * 0.5 - 0.5,
    ];
    let expected_logits = [
        2.0 * hidden[0] + -hidden[1] + 0.125,
        -0.5 * hidden[0] + 0.25 * hidden[1] - 0.25,
    ];

    let output = router
        .forward_stateless(&input, None, &RouterForwardOptions::soft_top1(2))
        .unwrap();

    assert_f32_slice_close(output.logits(), &expected_logits, TOL, TOL);
    assert_f32_slice_close(output.soft_probs(), &softmax(&expected_logits), TOL, TOL);
}

#[test]
fn qat_tests_router_soft_mode_dropout_and_jitter_are_explicit_inputs() {
    let router = router_for_logits([2.0, 1.0, 0.0]);
    let input = [1.0, 2.0];
    let options = RouterForwardOptions::soft_top1(3)
        .with_dropped_experts(vec![true, false, false])
        .with_logit_jitter(vec![0.0, 0.0, 3.0]);

    let output = router.forward_stateless(&input, None, &options).unwrap();

    assert_eq!(output.expert_index(), 2);
    assert_eq!(output.routing_weights(), output.soft_probs());
    assert_eq!(output.soft_probs()[0], 0.0);
    assert!(output.soft_probs()[2] > output.soft_probs()[1]);
}

#[test]
fn qat_tests_router_temporal_buffer_stores_soft_probs_and_resets() {
    let mut router = Top1RouterQat::new_with_aux_loss_weights(
        RouterShape::new(2, 3, 2).unwrap(),
        vec![1.0, 0.0, 0.0, 1.0],
        None,
        vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        Some(vec![0.0, 0.0, 0.0]),
        RouterAuxLossWeights::new(1.0, 1.0, 1.0).unwrap(),
    )
    .unwrap();
    let options = RouterForwardOptions::hard_top1(3);

    let first = router.forward_with_options(&[1.0, 2.0], &options).unwrap();
    assert_eq!(router.previous_distribution(), Some(first.soft_probs()));
    let second = router.forward_with_options(&[2.0, 1.0], &options).unwrap();
    let expected_temporal = 1.0
        - second
            .soft_probs()
            .iter()
            .zip(first.soft_probs())
            .map(|(&current, &previous)| current * previous)
            .sum::<f32>();

    assert!((second.aux_losses().temporal_smoothness_loss() - expected_temporal).abs() <= TOL);
    router.reset_sequence();
    assert_eq!(router.previous_distribution(), None);
}

#[test]
fn qat_tests_router_failed_forward_does_not_advance_temporal_state() {
    let mut router = router_for_logits([1.0, 2.0, 0.0]);
    let options = RouterForwardOptions::hard_top1(3);
    let first = router.forward_with_options(&[1.0, 2.0], &options).unwrap();
    let previous = first.soft_probs().to_vec();
    let invalid_options =
        RouterForwardOptions::hard_top1(3).with_dropped_experts(vec![true, true, true]);

    assert!(
        router
            .forward_with_options(&[1.0, 2.0], &invalid_options)
            .is_err()
    );
    assert_eq!(router.previous_distribution(), Some(previous.as_slice()));
}

#[test]
fn qat_tests_expert_block_composes_up_activation_down_and_residual() {
    let expert = simple_expert(vec![1.0, -1.0, 0.0, 1.0], vec![1.0, 0.0, 0.0, 1.0], None);
    let block = ExpertBlockQat::new(vec![expert], None).unwrap();
    let input = [2.0, 1.0];

    let actual = block.forward(&input, 0).unwrap();

    assert_f32_slice_close(&actual, &[3.0, 2.0], TOL, TOL);
}

#[test]
fn qat_tests_expert_block_phase_modes_change_projection_semantics() {
    let expert = simple_expert(vec![0.6, 0.0, 0.0, 0.6], vec![0.6, 0.0, 0.0, 0.6], None);
    let block = ExpertBlockQat::new(vec![expert], None).unwrap();
    let input = [1.0, 1.0];

    let hard = block
        .forward_with_options(
            &input,
            0,
            ExpertForwardOptions::hard_quantized_train()
                .with_expert_qat(ExpertQatForwardMode::HardQuantized),
        )
        .unwrap();
    let full = block
        .forward_with_options(
            &input,
            0,
            ExpertForwardOptions::full_precision_train()
                .with_expert_qat(ExpertQatForwardMode::FullPrecision),
        )
        .unwrap();

    assert_ne!(hard, full);
    assert_f32_slice_close(&hard, &[2.0, 2.0], TOL, TOL);
    assert_f32_slice_close(&full, &[1.6, 1.6], TOL, TOL);
}

#[test]
fn qat_tests_expert_shared_branch_honors_activation_mode_and_tiny_structure() {
    let expert = simple_expert(vec![0.0, 0.0, 0.0, 0.0], vec![0.0, 0.0, 0.0, 0.0], None);
    let shared = SharedDenseBranch::new(
        DenseBranchProjection::new(
            MatrixShape::new(2, 2).unwrap(),
            vec![1.0, 0.0, 0.0, 1.0],
            None,
        )
        .unwrap(),
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(0.0, 1.0).unwrap()),
            ActivationQuantFormat::UInt8,
        )
        .unwrap()
        .with_eval_passthrough(true),
        DenseBranchProjection::new(
            MatrixShape::new(2, 2).unwrap(),
            vec![1.0, 0.0, 0.0, 1.0],
            None,
        )
        .unwrap(),
        1.0,
    )
    .unwrap();
    let block = ExpertBlockQat::new(vec![expert], Some(shared)).unwrap();

    let train = block
        .forward_with_options(
            &[2.0, -1.0],
            0,
            ExpertForwardOptions::hard_quantized_train()
                .with_activation(ActivationForwardMode::Train),
        )
        .unwrap();
    let eval = block
        .forward_with_options(
            &[2.0, -1.0],
            0,
            ExpertForwardOptions::hard_quantized_train()
                .with_activation(ActivationForwardMode::Eval),
        )
        .unwrap();
    let tiny = make_tiny_model();

    assert_f32_slice_close(&train, &[3.0, -1.0], TOL, TOL);
    assert_f32_slice_close(&eval, &[4.0, -2.0], TOL, TOL);
    assert_eq!(tiny.expert_block().experts().len(), 2);
    assert!(
        tiny.expert_block()
            .experts()
            .iter()
            .all(|expert| expert.ternary_linear_count() == 2)
    );
    assert_eq!(tiny.config().d_model(), TINY_D_MODEL);
    assert_eq!(tiny.config().d_ff(), TINY_D_FF);
}

fn ternary_4x4_layer() -> TernaryLinearQat {
    TernaryLinearQat::new(
        MatrixShape::new(4, 4).unwrap(),
        vec![
            -0.8, -0.1, 0.9, 0.2, 0.6, -0.7, 0.0, 1.1, 0.4, 0.8, -1.2, -0.3, 1.4, 0.1, 0.6, -0.9,
        ],
        Some(vec![0.25, -0.5, 0.0, 0.75]),
        vec![
            TernaryThreshold::new(0.5).unwrap(),
            TernaryThreshold::new(0.5).unwrap(),
            TernaryThreshold::new(0.5).unwrap(),
            TernaryThreshold::new(0.5).unwrap(),
        ],
        vec![
            Q8_8Scale::from_f32(0.25).unwrap(),
            Q8_8Scale::from_f32(0.5).unwrap(),
            Q8_8Scale::from_f32(0.75).unwrap(),
            Q8_8Scale::from_f32(1.0).unwrap(),
        ],
    )
    .unwrap()
}

fn simple_expert(
    up_weights: Vec<f32>,
    down_weights: Vec<f32>,
    bias: Option<Vec<f32>>,
) -> ExpertQat {
    ExpertQat::new_with_clipped_activation(
        TernaryLinearQat::new(
            MatrixShape::new(2, 2).unwrap(),
            up_weights,
            None,
            vec![
                TernaryThreshold::new(0.5).unwrap(),
                TernaryThreshold::new(0.5).unwrap(),
            ],
            vec![
                Q8_8Scale::from_f32(1.0).unwrap(),
                Q8_8Scale::from_f32(1.0).unwrap(),
            ],
        )
        .unwrap(),
        ClippedActivation::relu(),
        ActFakeQuant::new(
            ActivationRangeMode::Fixed(ActivationRange::new(0.0, 255.0).unwrap()),
            ActivationQuantFormat::UInt8,
        )
        .unwrap(),
        TernaryLinearQat::new(
            MatrixShape::new(2, 2).unwrap(),
            down_weights,
            bias,
            vec![
                TernaryThreshold::new(0.5).unwrap(),
                TernaryThreshold::new(0.5).unwrap(),
            ],
            vec![
                Q8_8Scale::from_f32(1.0).unwrap(),
                Q8_8Scale::from_f32(1.0).unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap()
}

fn router_for_logits(logits: [f32; 3]) -> Top1RouterQat {
    Top1RouterQat::new(
        RouterShape::new(2, 3, 2).unwrap(),
        vec![1.0, 0.0, 0.0, 1.0],
        None,
        vec![0.0; 6],
        Some(logits.to_vec()),
    )
    .unwrap()
}

fn project_export_weights(ternary_values: &[TernaryValue], scales: &[Q8_8Scale]) -> Vec<f32> {
    ternary_values
        .chunks_exact(ternary_values.len() / scales.len())
        .zip(scales)
        .flat_map(|(row, &scale)| row.iter().map(move |&value| value.scaled(scale)))
        .collect()
}

fn manual_linear(
    shape: MatrixShape,
    weights: &[f32],
    bias: Option<&[f32]>,
    input: &[f32],
) -> Vec<f32> {
    weights
        .chunks_exact(shape.input_cols())
        .enumerate()
        .map(|(row_index, row)| {
            let weighted = row
                .iter()
                .zip(input)
                .map(|(&weight, &input)| weight * input)
                .sum::<f32>();
            weighted + bias.map_or(0.0, |bias| bias[row_index])
        })
        .collect()
}

fn manual_unsigned_fake_quant(value: f32, lo: f32, hi: f32, qmax: f32) -> f32 {
    let clamped = value.clamp(lo, hi);
    let quantized = ((clamped - lo) * qmax / (hi - lo)).round().clamp(0.0, qmax);
    (quantized * (hi - lo) / qmax + lo).clamp(lo, hi)
}

fn manual_tile_rms_affine_clip(
    input: &[f32],
    tile_width: usize,
    epsilon: f32,
    scale: f32,
    bias: f32,
    lo: f32,
    hi: f32,
) -> Vec<f32> {
    input
        .chunks_exact(tile_width)
        .flat_map(|chunk| {
            let mean_square =
                chunk.iter().map(|value| value * value).sum::<f32>() / chunk.len() as f32;
            let rms = (mean_square + epsilon).sqrt();
            chunk
                .iter()
                .map(move |value| ((value / rms) * scale + bias).clamp(lo, hi))
        })
        .collect()
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp = logits
        .iter()
        .map(|logit| (logit - max).exp())
        .collect::<Vec<_>>();
    let sum = exp.iter().sum::<f32>();
    exp.into_iter().map(|value| value / sum).collect()
}

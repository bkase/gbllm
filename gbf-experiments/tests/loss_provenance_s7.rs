#![cfg(all(feature = "s7", feature = "qat", feature = "burn-adapter"))]

use gbf_model::loss::temporal_smoothness::{
    SmoothnessWindow, s7_temporal_smoothness_with_boundaries,
};
use gbf_model::qat::{RouterForwardOptions, RouterShape, Top1RouterQat};
use gbf_train::adapter::burn::{
    BurnAutodiffBackend, BurnDevice, BurnFloatTensor, BurnNdArrayAutodiffBackend, burn_linear,
    burn_log_softmax, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::loss::distillation::{DEFAULT_DISTILLATION_TEMPERATURE, burn_distillation_loss};
use gbf_train::loss::router::{
    BurnRawRouterLogits, RawRouterLogits, burn_load_balance_loss,
    burn_load_balance_loss_with_stop_gradient_dispatch, burn_router_z_loss, router_z_loss,
};
use gbf_train::qat::Top1RouterBurnQat;

type B = BurnNdArrayAutodiffBackend;
type BurnGradients = <B as BurnAutodiffBackend>::Gradients;

const BATCH: usize = 2;
const SEQ: usize = 8;
const N_TOKENS: usize = BATCH * SEQ;
const N_BLOCKS: usize = 1;
const D_MODEL: usize = 3;
const N_EXPERTS: usize = 4;
const ROUTER_RANK: usize = 4;
const VOCAB: usize = 5;
const NONZERO_GRAD_EPS: f32 = 1.0e-6;

const TOKEN_IDS: [usize; N_TOKENS] = [0, 1, 2, 3, 4, 0, 1, 2, 3, 4, 0, 1, 2, 3, 4, 0];
const TARGET_IDS: [usize; N_TOKENS] = [1, 2, 3, 4, 0, 2, 3, 4, 0, 1, 3, 4, 0, 1, 2, 4];

#[test]
fn o13_lm_loss_reaches_task_path_but_not_low_rank_router_under_hard_dispatch() {
    assert_eq!(N_BLOCKS, 1, "O13 fixture is pinned to one router block");
    let device = BurnDevice::<B>::default();
    let fixture = task_fixture(&device);

    let loss =
        lm_loss(fixture.student_logits.clone(), &device) + router_zero_anchor(&fixture.layer);
    let gradients = loss.backward();

    assert_task_path_receives_gradient(&fixture, &gradients);
    assert_router_params_exact_zero(&fixture.layer, &gradients);
}

#[test]
fn o13_distill_loss_reaches_student_path_but_not_router_or_frozen_teacher() {
    let device = BurnDevice::<B>::default();
    let fixture = task_fixture(&device);

    let loss = burn_distillation_loss(
        fixture.student_logits.clone(),
        fixture.teacher_logits.clone(),
        1,
        DEFAULT_DISTILLATION_TEMPERATURE,
    )
    .unwrap()
        + router_zero_anchor(&fixture.layer)
        + fixture.teacher_logits.clone().sum() * 0.0;
    let gradients = loss.backward();

    assert_task_path_receives_gradient(&fixture, &gradients);
    assert_router_params_exact_zero(&fixture.layer, &gradients);
    assert_tensor_gradient_exact_zero("frozen_teacher_logits", &fixture.teacher_logits, &gradients);
}

#[test]
fn o13_centered_z_loss_reaches_raw_logits_and_low_rank_router_only_through_z() {
    let device = BurnDevice::<B>::default();
    let fixture = router_matrix_fixture(&device, RouterForwardOptions::soft_top1(N_EXPERTS));
    let task_probe = task_param_probe(&device);
    let raw_logits_probe =
        float_tensor_from_vec::<B, 2>(z_probe_values(), [N_TOKENS, N_EXPERTS], &device)
            .unwrap()
            .require_grad();

    let baseline_logits = vec![0.0; N_TOKENS * N_EXPERTS];
    let centered_baseline = router_z_loss(
        RawRouterLogits::from_raw_router_logits(&baseline_logits),
        N_EXPERTS,
    )
    .unwrap();
    assert!(
        f64::from(centered_baseline).abs() <= 1.0e-12,
        "centered z-loss baseline should be zero within 1e-12, got {centered_baseline}"
    );

    let probe_loss = burn_router_z_loss(BurnRawRouterLogits::from_raw_router_logits(
        raw_logits_probe.clone(),
    ))
    .unwrap();
    let router_loss = burn_router_z_loss(BurnRawRouterLogits::from_raw_router_logits(
        fixture.raw_router_logits.clone(),
    ))
    .unwrap();
    let loss = probe_loss + router_loss + task_params_zero_anchor(&task_probe);
    let gradients = loss.backward();

    assert_tensor_gradient_nonzero("raw_router_logits", &raw_logits_probe, &gradients);
    assert_router_params_nonzero(&fixture.layer, &gradients);
    assert_task_params_exact_zero(&task_probe, &gradients);
}

#[test]
fn o13_balance_loss_reaches_routing_probs_and_stop_gradient_dispatch_is_exact_zero() {
    let device = BurnDevice::<B>::default();
    let fixture = router_matrix_fixture(&device, RouterForwardOptions::soft_top1(N_EXPERTS));
    let task_probe = task_param_probe(&device);
    let routing_probs_probe =
        float_tensor_from_vec::<B, 2>(routing_prob_probe_values(), [N_TOKENS, N_EXPERTS], &device)
            .unwrap()
            .require_grad();
    let dispatch_probe =
        float_tensor_from_vec::<B, 2>(dispatch_probe_values(), [N_TOKENS, N_EXPERTS], &device)
            .unwrap()
            .require_grad();

    let production_loss = burn_load_balance_loss(
        fixture.routing_probs.clone(),
        &fixture.expert_assignments,
        &device,
    )
    .unwrap();
    let provenance_loss = burn_load_balance_loss_with_stop_gradient_dispatch(
        routing_probs_probe.clone(),
        dispatch_probe.clone(),
    )
    .unwrap();
    let loss = production_loss
        + provenance_loss
        + dispatch_probe.clone().sum() * 0.0
        + task_params_zero_anchor(&task_probe);
    let gradients = loss.backward();

    assert_tensor_gradient_nonzero("routing_probs", &routing_probs_probe, &gradients);
    assert_router_params_nonzero(&fixture.layer, &gradients);
    assert_tensor_gradient_exact_zero(
        "dispatch_indicator_stop_gradient_provenance",
        &dispatch_probe,
        &gradients,
    );
    assert_task_params_exact_zero(&task_probe, &gradients);
}

#[test]
fn o13_l_switch_full_window_reaches_router_and_masks_sequence_boundaries() {
    let device = BurnDevice::<B>::default();
    let fixture = router_matrix_fixture(&device, RouterForwardOptions::soft_top1(N_EXPERTS));
    let task_probe = task_param_probe(&device);
    let routing_probs_probe =
        float_tensor_from_vec::<B, 2>(routing_prob_probe_values(), [N_TOKENS, N_EXPERTS], &device)
            .unwrap()
            .require_grad();

    let loss = temporal_switch_loss(&routing_probs_probe)
        + boundary_zero_anchor(&routing_probs_probe)
        + temporal_switch_loss(&fixture.routing_probs)
        + task_params_zero_anchor(&task_probe);
    let gradients = loss.backward();
    let routing_grad = tensor_grad_vec("routing_probs", &routing_probs_probe, &gradients);

    for token in 0..SEQ {
        assert_row_has_nonzero_gradient(
            "L_switch full-window routing_probs row",
            &routing_grad,
            token,
            N_EXPERTS,
        );
    }
    assert_row_exact_zero(
        "L_switch boundary-masked routing_probs row before boundary",
        &routing_grad,
        SEQ + 3,
        N_EXPERTS,
    );
    assert_row_exact_zero(
        "L_switch boundary-masked routing_probs row after boundary",
        &routing_grad,
        SEQ + 4,
        N_EXPERTS,
    );
    assert_router_params_nonzero(&fixture.layer, &gradients);
    assert_task_params_exact_zero(&task_probe, &gradients);
}

struct TaskFixture {
    layer: Top1RouterBurnQat<B>,
    embedding_table: BurnFloatTensor<B, 2>,
    sequence_state_projection: BurnFloatTensor<B, 2>,
    norm_scale: BurnFloatTensor<B, 1>,
    norm_bias: BurnFloatTensor<B, 1>,
    expert_kernels: Vec<BurnFloatTensor<B, 2>>,
    student_logits: BurnFloatTensor<B, 2>,
    teacher_logits: BurnFloatTensor<B, 2>,
    selected_experts: Vec<usize>,
}

struct TaskParamProbe {
    embedding_table: BurnFloatTensor<B, 2>,
    sequence_state_projection: BurnFloatTensor<B, 2>,
    norm_scale: BurnFloatTensor<B, 1>,
    norm_bias: BurnFloatTensor<B, 1>,
    expert_kernels: Vec<BurnFloatTensor<B, 2>>,
}

struct RouterMatrixFixture {
    layer: Top1RouterBurnQat<B>,
    routing_probs: BurnFloatTensor<B, 2>,
    raw_router_logits: BurnFloatTensor<B, 2>,
    expert_assignments: Vec<usize>,
}

fn task_fixture(device: &BurnDevice<B>) -> TaskFixture {
    let layer = Top1RouterBurnQat::<B>::from_core(fixture_router(), device).unwrap();
    let probe = task_param_probe(device);
    let mut student_rows = Vec::with_capacity(N_TOKENS);
    let mut selected_experts = Vec::with_capacity(N_TOKENS);
    let options = RouterForwardOptions::hard_top1(N_EXPERTS);

    for batch in 0..BATCH {
        let mut state = BurnFloatTensor::<B, 1>::zeros([D_MODEL], device);
        for step in 0..SEQ {
            let token_index = batch * SEQ + step;
            let hidden = task_hidden_for_token(&probe, TOKEN_IDS[token_index], state, device);
            state = hidden.state;

            let router_output = layer
                .forward(hidden.activation.clone(), None, &options, device)
                .unwrap();
            let expert_rows = probe
                .expert_kernels
                .iter()
                .map(|kernel| burn_linear(hidden.activation.clone(), kernel.clone(), None))
                .collect::<Vec<_>>();
            let all_expert_logits = BurnFloatTensor::<B, 1>::stack::<2>(expert_rows, 0);
            let student_logits =
                burn_linear(router_output.routing_weights(), all_expert_logits, None);

            selected_experts.push(router_output.expert_index());
            student_rows.push(student_logits);
        }
    }

    TaskFixture {
        layer,
        embedding_table: probe.embedding_table,
        sequence_state_projection: probe.sequence_state_projection,
        norm_scale: probe.norm_scale,
        norm_bias: probe.norm_bias,
        expert_kernels: probe.expert_kernels,
        student_logits: BurnFloatTensor::<B, 1>::stack::<2>(student_rows, 0),
        teacher_logits: float_tensor_from_vec::<B, 2>(
            teacher_logits_values(),
            [N_TOKENS, VOCAB],
            device,
        )
        .unwrap()
        .require_grad(),
        selected_experts,
    }
}

struct TaskHidden {
    state: BurnFloatTensor<B, 1>,
    activation: BurnFloatTensor<B, 1>,
}

fn task_hidden_for_token(
    probe: &TaskParamProbe,
    token_id: usize,
    previous_state: BurnFloatTensor<B, 1>,
    device: &BurnDevice<B>,
) -> TaskHidden {
    let token = one_hot(token_id, VOCAB, device);
    let embedding = burn_linear(token, probe.embedding_table.clone(), None);
    let state_delta = burn_linear(
        embedding,
        probe.sequence_state_projection.clone(),
        None::<BurnFloatTensor<B, 1>>,
    );
    let state = previous_state * 0.5 + state_delta;
    let activation = state.clone() * probe.norm_scale.clone() + probe.norm_bias.clone();

    TaskHidden { state, activation }
}

fn task_param_probe(device: &BurnDevice<B>) -> TaskParamProbe {
    TaskParamProbe {
        embedding_table: float_tensor_from_vec::<B, 2>(
            matrix_values(VOCAB, D_MODEL, 0.07),
            [VOCAB, D_MODEL],
            device,
        )
        .unwrap()
        .require_grad(),
        sequence_state_projection: float_tensor_from_vec::<B, 2>(
            matrix_values(D_MODEL, D_MODEL, -0.11),
            [D_MODEL, D_MODEL],
            device,
        )
        .unwrap()
        .require_grad(),
        norm_scale: float_tensor_from_vec::<B, 1>(vec![1.1, 0.9, 1.2], [D_MODEL], device)
            .unwrap()
            .require_grad(),
        norm_bias: float_tensor_from_vec::<B, 1>(vec![0.05, -0.1, 0.025], [D_MODEL], device)
            .unwrap()
            .require_grad(),
        expert_kernels: (0..N_EXPERTS)
            .map(|expert| {
                float_tensor_from_vec::<B, 2>(
                    expert_kernel_values(expert),
                    [D_MODEL, VOCAB],
                    device,
                )
                .unwrap()
                .require_grad()
            })
            .collect(),
    }
}

fn router_matrix_fixture(
    device: &BurnDevice<B>,
    options: RouterForwardOptions,
) -> RouterMatrixFixture {
    let layer = Top1RouterBurnQat::<B>::from_core(fixture_router(), device).unwrap();
    let mut routing_rows = Vec::with_capacity(N_TOKENS);
    let mut raw_rows = Vec::with_capacity(N_TOKENS);
    let mut expert_assignments = Vec::with_capacity(N_TOKENS);

    for token_index in 0..N_TOKENS {
        let input = float_tensor_from_vec::<B, 1>(
            router_input_values(token_index).to_vec(),
            [D_MODEL],
            device,
        )
        .unwrap();
        let output = layer.forward(input, None, &options, device).unwrap();

        expert_assignments.push(output.expert_index());
        routing_rows.push(output.routing_probs());
        raw_rows.push(output.raw_router_logits());
    }

    RouterMatrixFixture {
        layer,
        routing_probs: BurnFloatTensor::<B, 1>::stack::<2>(routing_rows, 0),
        raw_router_logits: BurnFloatTensor::<B, 1>::stack::<2>(raw_rows, 0),
        expert_assignments,
    }
}

fn lm_loss(student_logits: BurnFloatTensor<B, 2>, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
    let target = target_distribution(device);
    let log_probs = burn_log_softmax(student_logits, 1);

    (log_probs * target).sum() * (-1.0 / N_TOKENS as f32)
}

fn temporal_switch_loss(routing_probs: &BurnFloatTensor<B, 2>) -> BurnFloatTensor<B, 1> {
    let sequence_masks = sequence_masks();
    let boundary_masks = boundary_masks();
    let window = SmoothnessWindow::s7_default();
    let mut total = None;
    let mut pair_count = 0usize;

    for batch in 0..BATCH {
        let pairs = s7_temporal_smoothness_with_boundaries(
            &sequence_masks[batch],
            &boundary_masks[batch],
            window,
        )
        .unwrap();
        for pair in pairs {
            let current = routing_row(routing_probs, batch, pair.t);
            let previous = routing_row(routing_probs, batch, pair.u);
            let dot = (current * previous).sum();
            let penalty = dot.ones_like() - dot;
            total = Some(match total {
                Some(acc) => acc + penalty,
                None => penalty,
            });
            pair_count += 1;
        }
    }

    total.expect("O13 L_switch fixture must contain valid same-sequence pairs") / pair_count as f32
}

fn routing_row(
    routing_probs: &BurnFloatTensor<B, 2>,
    batch: usize,
    step: usize,
) -> BurnFloatTensor<B, 1> {
    let row = batch * SEQ + step;
    routing_probs
        .clone()
        .slice([row..row + 1, 0..N_EXPERTS])
        .reshape([N_EXPERTS])
}

fn boundary_zero_anchor(routing_probs: &BurnFloatTensor<B, 2>) -> BurnFloatTensor<B, 1> {
    let before = routing_row(routing_probs, 1, 3).sum();
    let after = routing_row(routing_probs, 1, 4).sum();

    (before + after) * 0.0
}

fn sequence_masks() -> [[bool; SEQ]; BATCH] {
    [
        [true, true, true, true, true, true, true, true],
        [false, false, false, true, true, false, false, false],
    ]
}

fn boundary_masks() -> [[bool; SEQ]; BATCH] {
    [
        [false, false, false, false, false, false, false, false],
        [false, false, false, false, true, false, false, false],
    ]
}

fn fixture_router() -> Top1RouterQat {
    Top1RouterQat::new(
        RouterShape::new(D_MODEL, N_EXPERTS, ROUTER_RANK).unwrap(),
        vec![
            1.0, 0.0, -0.5, //
            0.0, 1.0, 0.25, //
            -0.5, 0.5, 1.0, //
            0.75, -0.25, 0.5,
        ],
        Some(vec![0.1, -0.2, 0.05, 0.0]),
        vec![
            0.5, -0.25, 0.75, 0.1, //
            -0.1, 0.6, -0.2, 0.5, //
            0.3, -0.4, 0.2, -0.6, //
            -0.7, 0.1, 0.4, 0.2,
        ],
        Some(vec![0.0, 0.2, -0.15, 0.05]),
    )
    .unwrap()
}

fn one_hot(index: usize, len: usize, device: &BurnDevice<B>) -> BurnFloatTensor<B, 1> {
    let mut values = vec![0.0; len];
    values[index] = 1.0;

    float_tensor_from_vec::<B, 1>(values, [len], device).unwrap()
}

fn target_distribution(device: &BurnDevice<B>) -> BurnFloatTensor<B, 2> {
    let mut values = vec![0.0; N_TOKENS * VOCAB];
    for (row, target) in TARGET_IDS.iter().copied().enumerate() {
        values[row * VOCAB + target] = 1.0;
    }

    float_tensor_from_vec::<B, 2>(values, [N_TOKENS, VOCAB], device).unwrap()
}

fn matrix_values(rows: usize, cols: usize, offset: f32) -> Vec<f32> {
    (0..rows * cols)
        .map(|index| {
            let row = index / cols;
            let col = index % cols;
            offset + row as f32 * 0.17 - col as f32 * 0.09 + (row + col) as f32 * 0.013
        })
        .collect()
}

fn expert_kernel_values(expert: usize) -> Vec<f32> {
    (0..D_MODEL * VOCAB)
        .map(|index| {
            let row = index / VOCAB;
            let col = index % VOCAB;
            expert as f32 * 0.19 + row as f32 * 0.11 - col as f32 * 0.07 + 0.03
        })
        .collect()
}

fn router_input_values(token_index: usize) -> [f32; D_MODEL] {
    let batch = token_index / SEQ;
    let step = token_index % SEQ;

    [
        0.25 + batch as f32 * 0.15 + step as f32 * 0.07,
        -0.5 + ((step * 3 + batch) % 5) as f32 * 0.2,
        0.3 - step as f32 * 0.04 + batch as f32 * 0.11,
    ]
}

fn z_probe_values() -> Vec<f32> {
    (0..N_TOKENS * N_EXPERTS)
        .map(|index| {
            let row = index / N_EXPERTS;
            let expert = index % N_EXPERTS;
            row as f32 * 0.03 - expert as f32 * 0.17 + 0.2
        })
        .collect()
}

fn routing_prob_probe_values() -> Vec<f32> {
    let base = [
        [0.55, 0.2, 0.15, 0.1],
        [0.1, 0.55, 0.2, 0.15],
        [0.15, 0.1, 0.55, 0.2],
        [0.2, 0.15, 0.1, 0.55],
    ];
    (0..N_TOKENS)
        .flat_map(|token| base[token % N_EXPERTS])
        .collect()
}

fn dispatch_probe_values() -> Vec<f32> {
    let mut values = vec![0.0; N_TOKENS * N_EXPERTS];
    for token in 0..N_TOKENS {
        values[token * N_EXPERTS + token % N_EXPERTS] = 1.0;
    }

    values
}

fn teacher_logits_values() -> Vec<f32> {
    (0..N_TOKENS * VOCAB)
        .map(|index| {
            let row = index / VOCAB;
            let col = index % VOCAB;
            row as f32 * 0.015 + col as f32 * 0.21 - 0.35
        })
        .collect()
}

fn router_zero_anchor(layer: &Top1RouterBurnQat<B>) -> BurnFloatTensor<B, 1> {
    let mut anchor = layer.input_projection().sum() + layer.expert_projection().sum();
    if let Some(input_bias) = layer.input_bias() {
        anchor = anchor + input_bias.sum();
    }
    if let Some(expert_bias) = layer.expert_bias() {
        anchor = anchor + expert_bias.sum();
    }

    anchor * 0.0
}

fn task_params_zero_anchor(probe: &TaskParamProbe) -> BurnFloatTensor<B, 1> {
    let mut anchor = probe.embedding_table.clone().sum()
        + probe.sequence_state_projection.clone().sum()
        + probe.norm_scale.clone().sum()
        + probe.norm_bias.clone().sum();
    for kernel in &probe.expert_kernels {
        anchor = anchor + kernel.clone().sum();
    }

    anchor * 0.0
}

fn assert_task_path_receives_gradient(fixture: &TaskFixture, gradients: &BurnGradients) {
    assert_tensor_gradient_nonzero("embedding_table", &fixture.embedding_table, gradients);
    assert_tensor_gradient_nonzero(
        "sequence_state_projection",
        &fixture.sequence_state_projection,
        gradients,
    );
    assert_tensor_gradient_nonzero("norm_scale", &fixture.norm_scale, gradients);
    assert_tensor_gradient_nonzero("norm_bias", &fixture.norm_bias, gradients);

    for expert in 0..N_EXPERTS {
        let name = format!("selected_expert_kernel[{expert}]");
        if fixture.selected_experts.contains(&expert) {
            assert_tensor_gradient_nonzero(&name, &fixture.expert_kernels[expert], gradients);
        } else {
            assert_tensor_gradient_exact_zero(&name, &fixture.expert_kernels[expert], gradients);
        }
    }
}

fn assert_task_params_exact_zero(probe: &TaskParamProbe, gradients: &BurnGradients) {
    assert_tensor_gradient_exact_zero("embedding_table", &probe.embedding_table, gradients);
    assert_tensor_gradient_exact_zero(
        "sequence_state_projection",
        &probe.sequence_state_projection,
        gradients,
    );
    assert_tensor_gradient_exact_zero("norm_scale", &probe.norm_scale, gradients);
    assert_tensor_gradient_exact_zero("norm_bias", &probe.norm_bias, gradients);
    for (index, kernel) in probe.expert_kernels.iter().enumerate() {
        assert_tensor_gradient_exact_zero(&format!("expert_kernel[{index}]"), kernel, gradients);
    }
}

fn assert_router_params_nonzero(layer: &Top1RouterBurnQat<B>, gradients: &BurnGradients) {
    assert_tensor_gradient_nonzero("input_projection", &layer.input_projection(), gradients);
    if let Some(input_bias) = layer.input_bias() {
        assert_tensor_gradient_nonzero("input_bias", &input_bias, gradients);
    }
    assert_tensor_gradient_nonzero("expert_projection", &layer.expert_projection(), gradients);
    if let Some(expert_bias) = layer.expert_bias() {
        assert_tensor_gradient_nonzero("expert_bias", &expert_bias, gradients);
    }
}

fn assert_router_params_exact_zero(layer: &Top1RouterBurnQat<B>, gradients: &BurnGradients) {
    assert_tensor_gradient_exact_zero("input_projection", &layer.input_projection(), gradients);
    if let Some(input_bias) = layer.input_bias() {
        assert_tensor_gradient_exact_zero("input_bias", &input_bias, gradients);
    }
    assert_tensor_gradient_exact_zero("expert_projection", &layer.expert_projection(), gradients);
    if let Some(expert_bias) = layer.expert_bias() {
        assert_tensor_gradient_exact_zero("expert_bias", &expert_bias, gradients);
    }
}

fn assert_tensor_gradient_nonzero<const D: usize>(
    name: &str,
    tensor: &BurnFloatTensor<B, D>,
    gradients: &BurnGradients,
) {
    let grad = tensor_grad_vec(name, tensor, gradients);
    assert!(
        grad.iter().any(|value| value.abs() >= NONZERO_GRAD_EPS),
        "{name} should receive a non-zero gradient >= {NONZERO_GRAD_EPS}, got {grad:?}"
    );
}

fn assert_tensor_gradient_exact_zero<const D: usize>(
    name: &str,
    tensor: &BurnFloatTensor<B, D>,
    gradients: &BurnGradients,
) {
    let grad = tensor
        .grad(gradients)
        .unwrap_or_else(|| panic!("{name} zero-gradient tensor should be materialized"));
    let grad = float_tensor_into_vec(grad).unwrap();
    assert!(
        grad.iter().all(|value| *value == 0.0),
        "{name} gradient should be exactly zero, got {grad:?}"
    );
}

fn tensor_grad_vec<const D: usize>(
    name: &str,
    tensor: &BurnFloatTensor<B, D>,
    gradients: &BurnGradients,
) -> Vec<f32> {
    let grad = tensor
        .grad(gradients)
        .unwrap_or_else(|| panic!("{name} should receive a gradient tensor"));

    float_tensor_into_vec(grad).unwrap()
}

fn assert_row_has_nonzero_gradient(name: &str, grad: &[f32], row: usize, width: usize) {
    let start = row * width;
    let row_grad = &grad[start..start + width];
    assert!(
        row_grad.iter().any(|value| value.abs() >= NONZERO_GRAD_EPS),
        "{name} {row} should receive a non-zero gradient, got {row_grad:?}"
    );
}

fn assert_row_exact_zero(name: &str, grad: &[f32], row: usize, width: usize) {
    let start = row * width;
    let row_grad = &grad[start..start + width];
    assert!(
        row_grad.iter().all(|value| *value == 0.0),
        "{name} {row} should be exactly zero, got {row_grad:?}"
    );
}

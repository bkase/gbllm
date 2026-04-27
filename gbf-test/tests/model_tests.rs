use gbf_artifact::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};
use gbf_foundation::ByteCost;
use gbf_model::block::{
    BlockExecutionStage, DenseFfnStage, FfnExecutionPath, ModelBlockTopology, MoeFfnStage,
    RouterStage, SharedDenseBranchStage, SharedSequenceStage,
};
use gbf_model::budget::{
    ExpertBudgetMetadata, compute_expert_byte_breakdown, compute_expert_bytes,
};
use gbf_model::config::{
    DenseFfnConfig, ModelTopologyConfig, MoeBlockConfig, MoeBlockSelection, MoeFfnConfig,
    SharedSequenceConfig, SharedSequenceKind,
};
use gbf_model::embeddings::{EmbeddingConfig, EmbeddingTied, EmbeddingUntied};
use gbf_model::expert::{
    ExpertBlockQatError, ExpertMlpConfig, ExpertMlpConfigEventCode, ExpertMlpConfigEventLevel,
    ExpertMlpVariant, SharedDenseBranchConfig,
};
use gbf_model::qat::MatrixShape;
use gbf_test::fixtures::{TINY_D_FF, TINY_D_MODEL, TINY_N_EXPERTS, tiny_expert, tiny_moe_config};
use gbf_test::helpers::assert_f32_slice_close;
use gbf_train::adapter::burn::{
    BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::embeddings::{EmbeddingTiedBurn, EmbeddingUntiedBurn};

const TOTAL_BLOCKS: usize = 4;
const TOL: f32 = 1.0e-6;

#[test]
fn model_tests_block_selection_patterns_route_only_ffn_path_and_share_sequence_state() {
    let cases = [
        (
            "alternating_start_0_stride_2",
            MoeBlockSelection::alternating(0, 2).unwrap(),
            vec![true, false, true, false],
        ),
        (
            "middle_1_through_2",
            MoeBlockSelection::middle(1, 2).unwrap(),
            vec![false, true, true, false],
        ),
        (
            "explicit_1_and_3",
            MoeBlockSelection::explicit(vec![1, 3]).unwrap(),
            vec![false, true, false, true],
        ),
    ];

    for (case_name, selection, expected_moe_mask) in cases {
        let topology = topology_for_selection(&selection, None);
        let actual_moe_mask = moe_mask(&topology);

        println!(
            "model_tests block_structure case={case_name} blocks={TOTAL_BLOCKS} moe_mask={actual_moe_mask:?} sequence_kind={:?}",
            topology.sequence_semantics()
        );

        assert_eq!(actual_moe_mask, expected_moe_mask);
        for (block_index, block) in topology.blocks().iter().enumerate() {
            assert_block_plan(block, expected_moe_mask[block_index]);
        }
    }
}

#[test]
fn model_tests_shared_dense_branch_is_config_gated_and_reported_by_topology() {
    let selection = MoeBlockSelection::explicit(vec![1]).unwrap();

    let default_topology = topology_for_selection(&selection, None);
    let default_plan = ModelBlockTopology::new(default_topology.blocks()[1].clone()).plan();
    let FfnExecutionPath::Moe { ffn, .. } = default_plan.ffn_path() else {
        panic!("selected block should use MoE FFN path");
    };
    assert_eq!(ffn.shared_dense, None);

    let shared_dense = SharedDenseBranchConfig::new(TINY_D_MODEL, 4).unwrap();
    let configured_topology = topology_for_selection(&selection, Some(shared_dense));
    let configured_plan = ModelBlockTopology::new(configured_topology.blocks()[1].clone()).plan();
    let FfnExecutionPath::Moe { ffn, .. } = configured_plan.ffn_path() else {
        panic!("selected block should use MoE FFN path");
    };

    println!(
        "model_tests shared_dense_branch d_model={} d_ff={} d_ff_shared={} n_experts={}",
        TINY_D_MODEL,
        TINY_D_FF,
        shared_dense.d_ff_shared(),
        TINY_N_EXPERTS
    );

    assert_eq!(
        ffn.shared_dense,
        Some(SharedDenseBranchStage {
            d_model: TINY_D_MODEL,
            d_ff_shared: 4,
        })
    );
}

#[test]
fn model_tests_tied_embeddings_share_storage_and_accumulate_path_contributions() {
    type B = BurnNdArrayAutodiffBackend;

    let config = EmbeddingConfig::tied(3, 2).unwrap();
    let layer = EmbeddingTied::from_config(
        config,
        vec![
            1.0, 2.0, //
            3.0, 4.0, //
            -1.0, 0.5,
        ],
    )
    .unwrap();

    assert!(std::ptr::eq(
        layer.embedding_weights().as_ptr(),
        layer.classifier_weights().as_ptr()
    ));
    assert_eq!(layer.parameter_count(), 6);

    let embeddings = layer.embed(&[2]).unwrap();
    let logits = layer.classify(&[2.0, -3.0]).unwrap();
    let device = BurnDevice::<B>::default();
    let burn_layer = EmbeddingTiedBurn::<B>::from_core(layer.clone(), &device).unwrap();
    let hidden = float_tensor_from_vec::<B, 1>(vec![2.0, -3.0], [2], &device).unwrap();
    let classifier_target =
        float_tensor_from_vec::<B, 1>(vec![0.0, 0.0, 1.0], [3], &device).unwrap();

    let embedding_loss = burn_layer.embed_one(2, &device).unwrap().sum();
    let classifier_logits = burn_layer.classify(hidden).unwrap();
    let classifier_loss = (classifier_logits * classifier_target).sum();
    let gradients = (embedding_loss + classifier_loss).backward();
    let embedding_weight_grad = float_tensor_into_vec(
        burn_layer
            .embedding_weights()
            .grad(&gradients)
            .expect("tied embedding weight gradient should exist"),
    )
    .unwrap();
    let classifier_weight_grad = float_tensor_into_vec(
        burn_layer
            .classifier_weights()
            .grad(&gradients)
            .expect("tied classifier weight gradient should share the embedding param"),
    )
    .unwrap();

    println!(
        "model_tests tied_embedding vocab={} d_model={} parameter_count={} shared_row_gradient={:?}",
        layer.vocab_size(),
        layer.d_model(),
        layer.parameter_count(),
        &embedding_weight_grad[4..6]
    );

    assert_eq!(embeddings.shape(), [1, 2]);
    assert_eq!(logits.shape(), [1, 3]);
    assert_f32_slice_close(embeddings.values(), &[-1.0, 0.5], TOL, TOL);
    assert_f32_slice_close(logits.values(), &[-4.0, -6.0, -3.5], TOL, TOL);
    assert_f32_slice_close(
        &embedding_weight_grad,
        &[0.0, 0.0, 0.0, 0.0, 3.0, -2.0],
        TOL,
        TOL,
    );
    assert_f32_slice_close(&classifier_weight_grad, &embedding_weight_grad, TOL, TOL);
}

#[test]
fn model_tests_untied_embeddings_use_separate_storage_and_parameter_counts() {
    type B = BurnNdArrayAutodiffBackend;

    let config = EmbeddingConfig::untied(3, 2).unwrap();
    let layer = EmbeddingUntied::from_config(
        config,
        vec![
            10.0, 20.0, //
            30.0, 40.0, //
            50.0, 60.0,
        ],
        vec![
            1.0, 0.0, //
            0.0, 1.0, //
            1.0, 1.0,
        ],
    )
    .unwrap();

    println!(
        "model_tests untied_embedding vocab={} d_model={} parameter_count={}",
        layer.vocab_size(),
        layer.d_model(),
        layer.parameter_count()
    );

    assert!(!std::ptr::eq(
        layer.embedding_weights().as_ptr(),
        layer.classifier_weights().as_ptr()
    ));
    assert_eq!(layer.parameter_count(), 12);
    assert_f32_slice_close(layer.embed(&[2]).unwrap().values(), &[50.0, 60.0], TOL, TOL);
    assert_f32_slice_close(
        layer.classify(&[2.0, -3.0]).unwrap().values(),
        &[2.0, -3.0, -1.0],
        TOL,
        TOL,
    );

    let device = BurnDevice::<B>::default();
    let burn_layer = EmbeddingUntiedBurn::<B>::from_core(layer.clone(), &device).unwrap();
    let hidden = float_tensor_from_vec::<B, 1>(vec![2.0, -3.0], [2], &device).unwrap();
    let classifier_target =
        float_tensor_from_vec::<B, 1>(vec![0.0, 0.0, 1.0], [3], &device).unwrap();
    let embedding_loss = burn_layer.embed_one(2, &device).unwrap().sum();
    let classifier_logits = burn_layer.classify(hidden).unwrap();
    let classifier_loss = (classifier_logits * classifier_target).sum();
    let gradients = (embedding_loss + classifier_loss).backward();
    let embedding_weight_grad = float_tensor_into_vec(
        burn_layer
            .embedding_weights()
            .grad(&gradients)
            .expect("untied embedding weight gradient should exist"),
    )
    .unwrap();
    let classifier_weight_grad = float_tensor_into_vec(
        burn_layer
            .classifier_weights()
            .grad(&gradients)
            .expect("untied classifier weight gradient should exist"),
    )
    .unwrap();

    assert_f32_slice_close(
        &embedding_weight_grad,
        &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0],
        TOL,
        TOL,
    );
    assert_f32_slice_close(
        &classifier_weight_grad,
        &[0.0, 0.0, 0.0, 0.0, 2.0, -3.0],
        TOL,
        TOL,
    );
}

#[test]
fn model_tests_expert_block_is_two_matrix_and_preserves_batch_shape() {
    let config = tiny_moe_config();
    let experts = vec![tiny_expert(&config, 0), tiny_expert(&config, 1)];
    let block = gbf_model::expert::ExpertBlockQat::without_shared_dense(experts).unwrap();

    println!(
        "model_tests expert_block n_experts={} d_model={} d_ff={} ternary_linear_count_per_expert={}",
        block.experts().len(),
        block.d_model(),
        block.experts()[0].d_ff(),
        block.experts()[0].ternary_linear_count()
    );

    assert_eq!(block.experts().len(), TINY_N_EXPERTS);
    assert_eq!(block.shared_dense(), None);
    for expert in block.experts() {
        assert_eq!(expert.ternary_linear_count(), 2);
        assert_eq!(expert.d_model(), TINY_D_MODEL);
        assert_eq!(expert.d_ff(), TINY_D_FF);
        assert_eq!(
            expert.up_projection().shape(),
            MatrixShape::new(TINY_D_FF, TINY_D_MODEL).unwrap()
        );
        assert_eq!(
            expert.down_projection().shape(),
            MatrixShape::new(TINY_D_MODEL, TINY_D_FF).unwrap()
        );
    }

    let input = vec![0.25; 2 * TINY_D_MODEL];
    let output = block.forward_batch(&input, &[0, 1]).unwrap();

    assert_eq!(output.shape(), [2, TINY_D_MODEL]);
    assert_eq!(output.values().len(), input.len());
}

#[test]
fn model_tests_glu_expert_variant_is_structured_warning_event_not_executable_structure() {
    let config = tiny_moe_config();
    let expert = tiny_expert(&config, 0);
    let two_matrix = ExpertMlpConfig::default_two_matrix(TINY_D_MODEL, TINY_D_FF).unwrap();
    let (glu, event) = ExpertMlpConfig::glu_explicit(TINY_D_MODEL, TINY_D_FF).unwrap();

    println!(
        "model_tests glu_warning d_model={} d_ff={} two_matrix_bytes={} glu_bytes={} message={}",
        event.d_model(),
        event.d_ff(),
        event.two_matrix_byte_cost(),
        event.glu_byte_cost(),
        event.message()
    );

    expert.validate_config(two_matrix).unwrap();
    assert_eq!(event.level(), ExpertMlpConfigEventLevel::Warning);
    assert_eq!(event.code(), ExpertMlpConfigEventCode::GluBankFitWarning);
    assert_eq!(event.variant(), ExpertMlpVariant::GatedLinearUnit);
    assert_eq!(event.ternary_linear_count(), 3);
    assert!(event.glu_byte_cost() > event.two_matrix_byte_cost());
    assert!(event.message().contains("third projection"));
    assert!(event.message().contains("ExpertBank budget"));
    assert_eq!(
        expert.validate_config(glu),
        Err(ExpertBlockQatError::UnsupportedExpertMlpVariant {
            variant: ExpertMlpVariant::GatedLinearUnit,
        })
    );
}

#[test]
fn model_tests_expert_byte_budget_formula_matches_independent_ternary_math() {
    let plan = default_plan();
    let breakdown = compute_expert_byte_breakdown(&plan, 128, 224);
    let manual_up = manual_ternary2_per_output_row_q8_8_bytes(224, 128);
    let manual_down = manual_ternary2_per_output_row_q8_8_bytes(128, 224);
    let manual_total = manual_up + manual_down + ExpertBudgetMetadata::default().total();

    println!(
        "model_tests expert_byte_budget d_model=128 d_ff=224 up={} down={} metadata={} total={}",
        breakdown.up_projection_bytes(),
        breakdown.down_projection_bytes(),
        breakdown.metadata().total(),
        breakdown.total()
    );

    assert_eq!(breakdown.up_projection_bytes(), ByteCost::new(7_616));
    assert_eq!(breakdown.down_projection_bytes(), ByteCost::new(7_424));
    assert_eq!(breakdown.projection_bytes(), ByteCost::new(15_040));
    assert_eq!(breakdown.metadata().total(), ByteCost::new(50));
    assert_eq!(breakdown.total(), ByteCost::new(15_090));
    assert_eq!(
        breakdown.up_projection_bytes(),
        plan.compute_byte_cost(224, 128)
    );
    assert_eq!(
        breakdown.down_projection_bytes(),
        plan.compute_byte_cost(128, 224)
    );
    assert_eq!(breakdown.total(), manual_total);
    assert_eq!(compute_expert_bytes(&plan, 128, 224), manual_total);
}

fn topology_for_selection(
    selection: &MoeBlockSelection,
    shared_dense_branch: Option<SharedDenseBranchConfig>,
) -> ModelTopologyConfig {
    let moe_ffn = MoeFfnConfig::with_shared_dense_branch(
        TINY_D_MODEL,
        TINY_D_FF,
        TINY_N_EXPERTS,
        shared_dense_branch,
    )
    .unwrap();

    ModelTopologyConfig::from_moe_block_selection(
        TOTAL_BLOCKS,
        selection,
        shared_sequence(),
        Some(DenseFfnConfig::new(TINY_D_MODEL, TINY_D_FF).unwrap()),
        moe_ffn,
    )
    .unwrap()
}

fn assert_block_plan(block: &MoeBlockConfig, expect_moe: bool) {
    let block_topology = ModelBlockTopology::new(block.clone());
    let plan = block_topology.plan();

    assert_eq!(block_topology.sequence_state_update_count(), 1);
    assert_eq!(plan.shared_sequence(), expected_shared_sequence_stage());
    assert_eq!(
        block_topology.stages()[0],
        BlockExecutionStage::SharedSequence(expected_shared_sequence_stage())
    );

    match (expect_moe, plan.ffn_path()) {
        (true, FfnExecutionPath::Moe { router, ffn }) => {
            assert_eq!(block_topology.router_invocations_per_token(), 1);
            assert_eq!(
                *router,
                RouterStage {
                    d_model: TINY_D_MODEL,
                    n_experts: TINY_N_EXPERTS,
                }
            );
            assert_eq!(
                *ffn,
                MoeFfnStage {
                    d_model: TINY_D_MODEL,
                    d_ff: TINY_D_FF,
                    n_experts: TINY_N_EXPERTS,
                    shared_dense: None,
                }
            );
        }
        (false, FfnExecutionPath::Dense(stage)) => {
            assert_eq!(block_topology.router_invocations_per_token(), 0);
            assert_eq!(
                *stage,
                DenseFfnStage {
                    d_model: TINY_D_MODEL,
                    d_ff: TINY_D_FF,
                }
            );
        }
        (true, FfnExecutionPath::Dense(_)) => panic!("expected MoE FFN path"),
        (false, FfnExecutionPath::Moe { .. }) => panic!("expected dense FFN path"),
    }
}

fn moe_mask(topology: &ModelTopologyConfig) -> Vec<bool> {
    topology
        .blocks()
        .iter()
        .map(MoeBlockConfig::has_moe_ffn)
        .collect()
}

fn shared_sequence() -> SharedSequenceConfig {
    SharedSequenceConfig::bounded_kv(TINY_D_MODEL, 16, 8).unwrap()
}

fn expected_shared_sequence_stage() -> SharedSequenceStage {
    SharedSequenceStage {
        kind: SharedSequenceKind::BoundedKv,
        d_model: TINY_D_MODEL,
        state_width: 8,
    }
}

fn default_plan() -> TernaryWeightPlan {
    TernaryWeightPlan::new(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerOutputRow,
        ScaleFormat::Q8_8,
        ThresholdPlan::FixedQ8_8,
    )
}

fn manual_ternary2_per_output_row_q8_8_bytes(rows: u32, cols: u32) -> ByteCost {
    let weights = u64::from(rows) * u64::from(cols);
    let weight_bytes = (weights * 2).div_ceil(8);
    let scale_bytes = u64::from(rows) * 2;
    ByteCost::new(weight_bytes + scale_bytes)
}

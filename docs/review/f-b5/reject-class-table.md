# F-B5 IIR Reject Class Table

Each row maps one RFC §15.2 IIR reject class to a checked fixture descriptor under `fixtures/infer_ir/reject/` and the typed diagnostic expected from that fixture.

| Class | Fixture | Typed diagnostic | Severity | Coverage note |
| --- | --- | --- | --- | --- |
| IIR-Reject-1 | `fixtures/infer_ir/reject/infer_ir_embedding_not_unique` | `InferIrEmbeddingNotUnique` | Hard | Single-token convention: embedding uniqueness. |
| IIR-Reject-2 | `fixtures/infer_ir/reject/infer_ir_decode_not_unique` | `InferIrDecodeNotUnique` | Hard | Single-token convention: decode uniqueness. |
| IIR-Reject-3 | `fixtures/infer_ir/reject/infer_ir_classify_not_unique` | `InferIrClassifyNotUnique` | Hard | Single-token convention: classify uniqueness. |
| IIR-Reject-4 | `fixtures/infer_ir/reject/infer_ir_expert_coverage_mismatch` | `InferIrExpertCoverageMismatch` | Hard | Expert section coverage. |
| IIR-Reject-5 | `fixtures/infer_ir/reject/infer_ir_route_coverage_mismatch` | `InferIrRouteCoverageMismatch` | Hard | Routed-layer route coverage. |
| IIR-Reject-6 | `fixtures/infer_ir/reject/infer_ir_semantic_checkpoint_emitted_here` | `InferIrSemanticCheckpointEmittedHere` | Hard | No semantic checkpoint emission in IIR v1. |
| IIR-Reject-7 | `fixtures/infer_ir/reject/infer_ir_effect_chain_not_linear` | `InferIrEffectChainNotLinear` | Hard | Effect-chain linearity. |
| IIR-Reject-8 | `fixtures/infer_ir/reject/infer_ir_effect_id_edge_token_violation` | `InferIrEffectIdEdgeTokenViolation` | Hard | Effect id is an edge token, not class instance. |
| IIR-Reject-9 | `fixtures/infer_ir/reject/infer_ir_topological_order_mismatch` | `InferIrTopologicalOrderMismatch` | Hard | Canonical topological order. |
| IIR-Reject-10 | `fixtures/infer_ir/reject/infer_ir_value_format_mismatch` | `InferIrValueFormatMismatch` | Hard | Value format consistency. |
| IIR-Reject-11 | `fixtures/infer_ir/reject/infer_ir_norm_format_mismatch` | `InferIrNormFormatMismatch` | Hard | Norm format chain. |
| IIR-Reject-12 | `fixtures/infer_ir/reject/infer_ir_decode_rng_binding_mismatch` | `InferIrDecodeRngBindingMismatch` | Hard | Decode RNG binding. |
| IIR-Reject-13 | `fixtures/infer_ir/reject/infer_ir_semantic_equivalence_failed` | `InferIrSemanticEquivalenceFailed` | Hard | Fixture-only semantic equivalence failure. |
| IIR-Reject-14 | `fixtures/infer_ir/reject/infer_ir_cycle_detected` | `InferIrCycleDetected` | Hard | DAG cycle detection. |
| IIR-Reject-15 | `fixtures/infer_ir/reject/infer_ir_unreachable_node` | `InferIrUnreachableNode` | Hard | Reachability. |
| IIR-Reject-16 | `fixtures/infer_ir/reject/infer_ir_disconnected_component` | `InferIrDisconnectedComponent` | Hard | Disconnected component detection. |
| IIR-Reject-17 | `fixtures/infer_ir/reject/infer_ir_forbidden_storage_metadata` | `InferIrForbiddenStorageMetadata` | Hard | Storage-free IIR contract. |
| IIR-Reject-18 | `fixtures/infer_ir/reject/infer_ir_non_v1_router_semantics` | `InferIrNonV1RouterSemantics` | Hard | Router semantics restricted to v1 Top1Hard. |
| IIR-Reject-19 | `fixtures/infer_ir/reject/infer_ir_semantic_anchor_missing` | `InferIrSemanticAnchorMissing` | Hard | Required semantic anchor absent. |
| IIR-Reject-20 | `fixtures/infer_ir/reject/infer_ir_ffn_activation_missing` | `InferIrFfnActivationMissing` | Hard | Routed FFN activation coverage. |
| IIR-Reject-21 | `fixtures/infer_ir/reject/infer_ir_expert_selection_missing` | `InferIrExpertSelectionMissing` | Hard | Routed expert selection node coverage. |
| IIR-Reject-22 | `fixtures/infer_ir/reject/infer_ir_gate_weight_not_consumed` | `InferIrGateWeightNotConsumed` | Hard | GateWeight consumption by SelectExpertTop1. |
| IIR-Reject-23 | `fixtures/infer_ir/reject/infer_ir_token_ingress_ambiguous` | `InferIrTokenIngressAmbiguous` | Hard | Token ingress must be unambiguous. |
| IIR-Reject-24 | `fixtures/infer_ir/reject/infer_ir_reduction_site_missing` | `InferIrReductionSiteMissing` | Hard | Reduction-bearing node missing Stage 2 site. |
| IIR-Reject-25 | `fixtures/infer_ir/reject/infer_ir_op_histogram_total_mismatch` | `InferIrOpHistogramTotalMismatch` | Hard | Op histogram total must match node count. |
| IIR-Reject-26 | `fixtures/infer_ir/reject/infer_ir_fault_boundary_emitted_v1_forbidden` | `InferIrFaultBoundaryEmittedV1Forbidden` | Hard | FaultBoundary reserved but not emitted in v1. |
| IIR-Reject-27 | `fixtures/infer_ir/reject/infer_ir_op_signature_mismatch` | `InferIrOpSignatureMismatch` | Hard | Closed op-signature predicate. |
| IIR-Reject-28 | `fixtures/infer_ir/reject/infer_ir_router_score_orphaned` | `InferIrRouterScoreOrphaned` | Hard | RouterScore reachability. |
| IIR-Reject-29 | `fixtures/infer_ir/reject/infer_ir_sequence_slot_coverage_mismatch` | `InferIrSequenceSlotCoverageMismatch` | Hard | Sequence slot coverage. |
| IIR-Reject-30 | `fixtures/infer_ir/reject/infer_ir_unexpected_rng_effect_on_pure_op` | `InferIrUnexpectedRngEffectOnPureOp` | Hard | Pure ops cannot touch RNG effects. |
| IIR-Reject-31 | `fixtures/infer_ir/reject/infer_ir_residual_boundary_mismatch` | `InferIrResidualBoundaryMismatch` | Hard | CombineResidual boundary semantics. |
| IIR-Reject-32 | `fixtures/infer_ir/reject/infer_ir_router_matvec_missing_for_routed_layer` | `InferIrRouterMatVecMissingForRoutedLayer` | Hard | Routed layer must emit RouterMatVec. |
| IIR-Reject-33 | `fixtures/infer_ir/reject/infer_ir_router_present_for_dense_layer` | `InferIrRouterPresentForDenseLayer` | Hard | Dense layer must not emit router ops. |
| IIR-Reject-34 | `fixtures/infer_ir/reject/infer_ir_input_token_value_id_mismatch` | `InferIrInputTokenValueIdMismatch` | Hard | Token input value id binding. |
| IIR-Reject-35 | `fixtures/infer_ir/reject/infer_ir_sequence_state_next_orphaned` | `InferIrSequenceStateNextOrphaned` | Hard | SequenceStateNext reachability. |
| IIR-Reject-36 | `fixtures/infer_ir/reject/infer_ir_sequence_semantics_unsupported_v1` | `InferIrSequenceSemanticsUnsupportedV1` | Hard | Non-identity sequence semantics unsupported in v1. |

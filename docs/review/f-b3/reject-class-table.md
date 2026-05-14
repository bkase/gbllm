# F-B3 Reject-Class Table

Each row is an RFC Section 15.1 reject class backed by a tiny
`fixtures/quant_graph/reject/**` fixture. The `inputs.toml` counterexample names
the programmatic builder used by the Stage 1 fixture test; the test runs
`build_quant_graph_core` and requires the typed hard diagnostic from
`expected.toml`.

| Class | Clause | Fixture path | Diagnostic code | Severity | Source class |
| --- | --- | --- | --- | --- | --- |
| QG-Reject-1 | 2.16 | `fixtures/quant_graph/reject/qg_reject_01_training_residue` | `QuantGraphTrainingResidue` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_training_residue_inputs` |
| QG-Reject-2 | 8.6 | `fixtures/quant_graph/reject/qg_reject_02_role_format_mismatch` | `QuantGraphRoleFormatMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_role_format_mismatch_inputs` |
| QG-Reject-3 | 2.11 | `fixtures/quant_graph/reject/qg_reject_03_routing_missing_for_routed_layer` | `QuantGraphRoutingMissingForRoutedLayer` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_routing_missing_for_routed_layer_inputs` |
| QG-Reject-4 | 2.11 | `fixtures/quant_graph/reject/qg_reject_04_routing_present_for_dense_layer` | `QuantGraphRoutingPresentForDenseLayer` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_routing_present_for_dense_layer_inputs` |
| QG-Reject-5 | 2.11 | `fixtures/quant_graph/reject/qg_reject_05_routing_expert_coverage_mismatch` | `QuantGraphRoutingExpertCoverageMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_mismatch_inputs` |
| QG-Reject-6 | 8.5 SC-1 | `fixtures/quant_graph/reject/qg_reject_06_tensor_id_not_unique` | `QuantGraphTensorIdNotUnique` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_tensor_id_not_unique_inputs` |
| QG-Reject-7 | QG-Ok-4 | `fixtures/quant_graph/reject/qg_reject_07_identity_hash_mismatch` | `QuantGraphIdentityHashMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_identity_hash_mismatch_inputs` |
| QG-Reject-8 | 8.5 SC-2 / 2.8 | `fixtures/quant_graph/reject/qg_reject_08_export_provenance_missing` | `QuantGraphExportProvenanceMissing` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_export_provenance_missing_inputs` |
| QG-Reject-9 | 8.5 SC-2 | `fixtures/quant_graph/reject/qg_reject_09_provenance_image_not_injective` | `QuantGraphProvenanceImageNotInjective` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_provenance_image_not_injective_inputs` |
| QG-Reject-10 | 8.5 SC-3 | `fixtures/quant_graph/reject/qg_reject_10_norm_plan_reference_unresolved` | `QuantGraphNormPlanReferenceUnresolved` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_norm_plan_reference_unresolved_inputs` |
| QG-Reject-11 | 8.5 SC-5 | `fixtures/quant_graph/reject/qg_reject_11_expert_section_weight_missing` | `QuantGraphExpertSectionWeightMissing` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_expert_section_weight_missing_inputs` |
| QG-Reject-12 | 8.5 SC-7 | `fixtures/quant_graph/reject/qg_reject_12_classify_head_tied_mismatch` | `QuantGraphClassifyHeadTiedMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_classify_head_tied_mismatch_inputs` |
| QG-Reject-13 | 8.5 SC-15 | `fixtures/quant_graph/reject/qg_reject_13_classify_head_format_mismatch` | `QuantGraphClassifyHeadFormatMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_classify_head_format_mismatch_inputs` |
| QG-Reject-14 | 8.5 SC-8 | `fixtures/quant_graph/reject/qg_reject_14_decode_spec_not_in_capability_set` | `QuantGraphDecodeSpecNotInCapabilitySet` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_decode_spec_not_in_capability_set_inputs` |
| QG-Reject-15 | 8.5 SC-11 | `fixtures/quant_graph/reject/qg_reject_15_sequence_semantics_tensor_mismatch` | `QuantGraphSequenceSemanticsTensorMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_sequence_semantics_tensor_mismatch_inputs` |
| QG-Reject-16 | 8.5 SC-9 | `fixtures/quant_graph/reject/qg_reject_16_layout_inconsistent_with_model_spec` | `QuantGraphLayoutInconsistentWithModelSpec` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_layout_inconsistent_with_model_spec_inputs` |
| QG-Reject-17 | 8.5 SC-13 | `fixtures/quant_graph/reject/qg_reject_17_blob_ref_unresolvable` | `QuantGraphBlobRefUnresolvable` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_blob_ref_unresolvable_inputs` |
| QG-Reject-18 | 8.5 SC-13 | `fixtures/quant_graph/reject/qg_reject_18_blob_ref_size_mismatch` | `QuantGraphBlobRefSizeMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_blob_ref_size_mismatch_inputs` |
| QG-Reject-19 | 8.5 SC-13 | `fixtures/quant_graph/reject/qg_reject_19_aux_blob_ref_size_mismatch` | `QuantGraphAuxBlobRefSizeMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_aux_blob_ref_size_mismatch_inputs` |
| QG-Reject-20 | 2.10 | `fixtures/quant_graph/reject/qg_reject_20_determinism_requires_enforced_reduction_order` | `QuantGraphDeterminismRequiresEnforcedReductionOrder` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_determinism_requires_enforced_reduction_order_inputs` |
| QG-Reject-21 | 8.5 SC-12 | `fixtures/quant_graph/reject/qg_reject_21_required_feature_unsupported` | `QuantGraphRequiredFeatureUnsupported` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_required_feature_unsupported_inputs` |
| QG-Reject-22 | 2.3 | `fixtures/quant_graph/reject/qg_reject_22_forbidden_storage_metadata` | `QuantGraphForbiddenStorageMetadata` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_forbidden_storage_metadata_inputs` |
| QG-Reject-23 | 8.5 SC-14 | `fixtures/quant_graph/reject/qg_reject_23_embedding_missing` | `QuantGraphEmbeddingMissing` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_embedding_missing_inputs` |
| QG-Reject-24 | 8.5 SC-14 | `fixtures/quant_graph/reject/qg_reject_24_embedding_not_unique` | `QuantGraphEmbeddingNotUnique` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_embedding_not_unique_inputs` |
| QG-Reject-25 | 8.5 SC-17 | `fixtures/quant_graph/reject/qg_reject_25_ffn_gate_presence_mismatch` | `QuantGraphFfnGatePresenceMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_ffn_gate_presence_mismatch_inputs` |
| QG-Reject-26 | 8.5 SC-18 | `fixtures/quant_graph/reject/qg_reject_26_layer_norms_incomplete` | `QuantGraphLayerNormsIncomplete` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_layer_norms_incomplete_inputs` |
| QG-Reject-27 | 8.5 SC-20 | `fixtures/quant_graph/reject/qg_reject_27_final_norm_missing` | `QuantGraphFinalNormMissing` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_final_norm_missing_inputs` |
| QG-Reject-28 | 8.5 SC-20 | `fixtures/quant_graph/reject/qg_reject_28_norm_site_duplicate` | `QuantGraphNormSiteDuplicate` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_norm_site_duplicate_inputs` |
| QG-Reject-29 | 8.5 SC-21 | `fixtures/quant_graph/reject/qg_reject_29_aux_blob_kind_mismatch` | `QuantGraphAuxBlobKindMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_aux_blob_kind_mismatch_inputs` |
| QG-Reject-30 | 8.5 SC-22 | `fixtures/quant_graph/reject/qg_reject_30_decode_requires_rng_mismatch` | `QuantGraphDecodeRequiresRngMismatch` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_decode_requires_rng_mismatch_inputs` |
| QG-Reject-31 | 8.1 | `fixtures/quant_graph/reject/qg_reject_31_router_gate_weight_semantics_unsupported` | `QuantGraphRouterGateWeightSemanticsUnsupported` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_router_gate_weight_semantics_unsupported_inputs` |
| QG-Reject-32 | 8.5 SC-23 | `fixtures/quant_graph/reject/qg_reject_32_residual_plan_invalid` | `QuantGraphResidualPlanInvalid` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_residual_plan_invalid_inputs` |
| QG-Reject-33 | 8.5 SC-4 missing | `fixtures/quant_graph/reject/qg_reject_33_routing_expert_coverage_gap` | `QuantGraphRoutingExpertCoverageGap` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_gap_inputs` |
| QG-Reject-34 | 8.5 SC-4 extra/out-of-range | `fixtures/quant_graph/reject/qg_reject_34_routing_expert_coverage_extra` | `QuantGraphRoutingExpertCoverageExtra` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_extra_inputs` |
| QG-Reject-35 | 2.10 | `fixtures/quant_graph/reject/qg_reject_35_bit_exact_mid_reduction_saturation_forbidden` | `QuantGraphBitExactMidReductionSaturationForbidden` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_bit_exact_mid_reduction_saturation_forbidden_inputs` |
| QG-Reject-36 | 8.1 / RouterSemantics | `fixtures/quant_graph/reject/qg_reject_36_router_tie_break_unsupported` | `QuantGraphRouterTieBreakUnsupported` | Hard | `gbf-codegen::s1::quant_graph::tests::reject_router_tie_break_unsupported_inputs` |

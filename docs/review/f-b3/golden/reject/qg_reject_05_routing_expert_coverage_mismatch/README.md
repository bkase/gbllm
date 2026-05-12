# QG-Reject-5: QuantGraphRoutingExpertCoverageMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: sets router n_experts different from the model summary.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRoutingExpertCoverageMismatch diagnostic matching expected.toml.

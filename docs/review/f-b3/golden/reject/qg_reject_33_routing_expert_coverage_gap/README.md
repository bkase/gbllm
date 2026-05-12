# QG-Reject-33: QuantGraphRoutingExpertCoverageGap

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_gap_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes one routed expert's weight bindings.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRoutingExpertCoverageGap diagnostic matching expected.toml.

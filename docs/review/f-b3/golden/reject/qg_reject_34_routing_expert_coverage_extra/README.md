# QG-Reject-34: QuantGraphRoutingExpertCoverageExtra

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_routing_expert_coverage_extra_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: adds an out-of-range dense expert section.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRoutingExpertCoverageExtra diagnostic matching expected.toml.

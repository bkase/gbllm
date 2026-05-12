# QG-Reject-11: QuantGraphExpertSectionWeightMissing

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_expert_section_weight_missing_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes the FFN down tensor for the dense expert section.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphExpertSectionWeightMissing diagnostic matching expected.toml.

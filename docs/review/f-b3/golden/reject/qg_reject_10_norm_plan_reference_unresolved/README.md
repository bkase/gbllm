# QG-Reject-10: QuantGraphNormPlanReferenceUnresolved

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_norm_plan_reference_unresolved_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: adds a norm tensor role pointing at an unknown norm plan id.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphNormPlanReferenceUnresolved diagnostic matching expected.toml.

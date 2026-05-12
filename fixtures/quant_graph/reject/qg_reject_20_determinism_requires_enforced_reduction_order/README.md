# QG-Reject-20: QuantGraphDeterminismRequiresEnforcedReductionOrder

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_determinism_requires_enforced_reduction_order_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: marks BitExact reduction order policy as not enforced.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphDeterminismRequiresEnforcedReductionOrder diagnostic matching expected.toml.

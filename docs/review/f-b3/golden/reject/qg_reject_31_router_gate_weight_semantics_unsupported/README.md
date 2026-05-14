# QG-Reject-31: QuantGraphRouterGateWeightSemanticsUnsupported

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_router_gate_weight_semantics_unsupported_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: uses an unsupported router gate-weight semantics tag.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRouterGateWeightSemanticsUnsupported diagnostic matching expected.toml.

# QG-Reject-4: QuantGraphRoutingPresentForDenseLayer

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_routing_present_for_dense_layer_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: adds a router layer to a dense model summary.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRoutingPresentForDenseLayer diagnostic matching expected.toml.

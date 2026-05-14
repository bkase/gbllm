# QG-Reject-3: QuantGraphRoutingMissingForRoutedLayer

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_routing_missing_for_routed_layer_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes the router layer from a routed fixture.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRoutingMissingForRoutedLayer diagnostic matching expected.toml.

# QG-Reject-26: QuantGraphLayerNormsIncomplete

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_layer_norms_incomplete_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes the layer FFN norm binding.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphLayerNormsIncomplete diagnostic matching expected.toml.

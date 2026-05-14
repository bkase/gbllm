# QG-Reject-6: QuantGraphTensorIdNotUnique

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_tensor_id_not_unique_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: duplicates a tensor id in the tensor bindings.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphTensorIdNotUnique diagnostic matching expected.toml.

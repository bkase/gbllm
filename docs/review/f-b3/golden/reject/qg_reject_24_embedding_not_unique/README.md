# QG-Reject-24: QuantGraphEmbeddingNotUnique

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_embedding_not_unique_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: adds a second embedding tensor binding.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphEmbeddingNotUnique diagnostic matching expected.toml.

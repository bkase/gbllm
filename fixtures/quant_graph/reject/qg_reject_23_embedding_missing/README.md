# QG-Reject-23: QuantGraphEmbeddingMissing

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_embedding_missing_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: changes the only embedding tensor role into an untied classify weight.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphEmbeddingMissing diagnostic matching expected.toml.

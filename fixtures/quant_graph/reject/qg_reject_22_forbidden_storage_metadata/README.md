# QG-Reject-22: QuantGraphForbiddenStorageMetadata

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_forbidden_storage_metadata_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: marks upstream artifact facts as leaking storage metadata into QuantGraph construction.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphForbiddenStorageMetadata diagnostic matching expected.toml.

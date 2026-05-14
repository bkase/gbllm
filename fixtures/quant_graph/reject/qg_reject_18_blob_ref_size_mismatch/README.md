# QG-Reject-18: QuantGraphBlobRefSizeMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_blob_ref_size_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: changes the embedding blob metadata decoded size.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphBlobRefSizeMismatch diagnostic matching expected.toml.

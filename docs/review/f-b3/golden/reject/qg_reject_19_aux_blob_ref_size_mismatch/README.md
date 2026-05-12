# QG-Reject-19: QuantGraphAuxBlobRefSizeMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_aux_blob_ref_size_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: changes an expert scale aux blob metadata decoded size.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphAuxBlobRefSizeMismatch diagnostic matching expected.toml.

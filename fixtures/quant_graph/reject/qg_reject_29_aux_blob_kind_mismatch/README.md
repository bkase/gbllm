# QG-Reject-29: QuantGraphAuxBlobKindMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_aux_blob_kind_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes the required scale aux blob from a Binary1 expert weight.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphAuxBlobKindMismatch diagnostic matching expected.toml.

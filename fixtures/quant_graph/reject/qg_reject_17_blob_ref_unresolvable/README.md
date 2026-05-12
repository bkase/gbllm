# QG-Reject-17: QuantGraphBlobRefUnresolvable

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_blob_ref_unresolvable_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes the embedding blob from the prebuilt ResolvedBlobIndex.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphBlobRefUnresolvable diagnostic matching expected.toml.

# QG-Reject-9: QuantGraphProvenanceImageNotInjective

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_provenance_image_not_injective_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: maps two tensor ids to the same export tensor id.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphProvenanceImageNotInjective diagnostic matching expected.toml.

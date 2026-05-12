# QG-Reject-8: QuantGraphExportProvenanceMissing

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_export_provenance_missing_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes one tensor export provenance entry.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphExportProvenanceMissing diagnostic matching expected.toml.

# QG-Reject-27: QuantGraphFinalNormMissing

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_final_norm_missing_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes the final norm binding.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphFinalNormMissing diagnostic matching expected.toml.

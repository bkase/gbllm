# QG-Reject-12: QuantGraphClassifyHeadTiedMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_classify_head_tied_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: points a tied classify head at a non-embedding tensor.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphClassifyHeadTiedMismatch diagnostic matching expected.toml.

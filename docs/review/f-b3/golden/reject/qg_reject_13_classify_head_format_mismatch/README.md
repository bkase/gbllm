# QG-Reject-13: QuantGraphClassifyHeadFormatMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_classify_head_format_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: uses an unsupported ternary logit format for the classify head.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphClassifyHeadFormatMismatch diagnostic matching expected.toml.

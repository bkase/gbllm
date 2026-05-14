# QG-Reject-28: QuantGraphNormSiteDuplicate

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_norm_site_duplicate_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: duplicates a norm site binding.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphNormSiteDuplicate diagnostic matching expected.toml.

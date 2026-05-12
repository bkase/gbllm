# QG-Reject-2: QuantGraphRoleFormatMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_role_format_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: changes the embedding tensor to a Binary1 expert-only format.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRoleFormatMismatch diagnostic matching expected.toml.

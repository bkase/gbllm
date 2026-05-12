# QG-Reject-30: QuantGraphDecodeRequiresRngMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_decode_requires_rng_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: marks decode requires-rng facts as mismatched.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphDecodeRequiresRngMismatch diagnostic matching expected.toml.

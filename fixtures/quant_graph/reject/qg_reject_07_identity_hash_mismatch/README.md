# QG-Reject-7: QuantGraphIdentityHashMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_identity_hash_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: changes semantic_core_hash away from the validated effective core hash.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphIdentityHashMismatch diagnostic matching expected.toml.

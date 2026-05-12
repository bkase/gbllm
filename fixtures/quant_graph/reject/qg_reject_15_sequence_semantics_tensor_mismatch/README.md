# QG-Reject-15: QuantGraphSequenceSemanticsTensorMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_sequence_semantics_tensor_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: marks sequence-semantic tensor facts as mismatched.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphSequenceSemanticsTensorMismatch diagnostic matching expected.toml.

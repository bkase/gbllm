# QG-Reject-25: QuantGraphFfnGatePresenceMismatch

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_ffn_gate_presence_mismatch_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: switches the FFN plan to SwiGLU without adding a gate weight.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphFfnGatePresenceMismatch diagnostic matching expected.toml.

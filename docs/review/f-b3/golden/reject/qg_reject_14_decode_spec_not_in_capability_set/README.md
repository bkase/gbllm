# QG-Reject-14: QuantGraphDecodeSpecNotInCapabilitySet

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_decode_spec_not_in_capability_set_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: removes Argmax from the decode capability set.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphDecodeSpecNotInCapabilitySet diagnostic matching expected.toml.

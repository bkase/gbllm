# QG-Reject-21: QuantGraphRequiredFeatureUnsupported

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_required_feature_unsupported_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: marks a required feature as unsupported by the target capability set.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRequiredFeatureUnsupported diagnostic matching expected.toml.

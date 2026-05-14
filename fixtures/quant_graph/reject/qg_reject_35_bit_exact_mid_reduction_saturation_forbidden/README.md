# QG-Reject-35: QuantGraphBitExactMidReductionSaturationForbidden

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_bit_exact_mid_reduction_saturation_forbidden_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: marks BitExact mid-reduction saturation as present.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphBitExactMidReductionSaturationForbidden diagnostic matching expected.toml.

# QG-Reject-32: QuantGraphResidualPlanInvalid

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_residual_plan_invalid_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: uses a ternary residual activation format outside the activation set.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphResidualPlanInvalid diagnostic matching expected.toml.

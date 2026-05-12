# QG-Reject-16: QuantGraphLayoutInconsistentWithModelSpec

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_layout_inconsistent_with_model_spec_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: changes embedding layout away from vocab_size by d_model.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphLayoutInconsistentWithModelSpec diagnostic matching expected.toml.

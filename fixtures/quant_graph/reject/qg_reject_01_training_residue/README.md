# QG-Reject-1: QuantGraphTrainingResidue

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_training_residue_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: marks upstream export-role facts as containing training-only residue.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphTrainingResidue diagnostic matching expected.toml.

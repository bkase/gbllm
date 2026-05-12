# QG-Reject-36: QuantGraphRouterTieBreakUnsupported

This fixture is pipeline-backed by gbf-codegen::s1::quant_graph::tests::reject_router_tie_break_unsupported_inputs.
The builder starts from a tiny synthetic QuantGraph input and applies this malformation: uses an unsupported router tie-break semantics tag.

The Stage 1 fixture test runs build_quant_graph_core and requires a Hard QuantGraphRouterTieBreakUnsupported diagnostic matching expected.toml.

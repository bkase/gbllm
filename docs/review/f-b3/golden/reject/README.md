Each subdirectory is one tiny QuantGraph reject fixture aligned to RFC Section 15.1.

`expected.toml` pins the numbered reject slot, typed Stage 1 diagnostic code,
Hard severity, and RFC clause. `inputs.toml` names the programmatic malformed
input builder used by the Stage 1 fixture tests; the test invokes
`build_quant_graph_core` and checks the emitted diagnostic rather than
constructing diagnostic enums directly.

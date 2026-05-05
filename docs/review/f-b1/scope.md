# F-B1 Scope

F-B1 implements the M0.5 compute bringup from `history/rfcs/F-B1-compute-bringup.md`: exact tiled `i8 x i8 -> i32` matmul, deterministic fixtures, BankLease-shaped lowering, streamed tile output, frame-service metrics, and a reproducible `realism_report.v1.json`. The measured emitted kernel is `fixed_8_step_shift_add_i8_i32`; quarter-square table support is kept as future-kernel reference coverage, not as a checked F-B1 timing claim.

Out of scope: quantization, oracles, conformance envelopes, M1 `CompileRequest`, range plans, observation plans, and new harness opcodes.

# Known Debt

No F-B1 closure blockers remain in this packet.

Follow-up notes that are intentionally outside F-B1 closure:

- The emitted streaming ROM uses the checked `fixed_8_step_shift_add_i8_i32`
  microkernel and reads no Bank0 multiply table. This is an RFC/closure
  amendment, not an implicit QST timing claim: the quarter-square generator and
  exhaustive parity tests remain in `gbf-codegen` as reference/future
  optimization coverage, while QST emission and measurement must land under a
  follow-up kernel-calibration bead (`bd-3akv`).
- Frame-service evidence is derived from emulator PC traps at the ROM VBlank
  handler generated through the runtime interrupt helper and at yield safe
  points, plus the existing tile-safe harness boundary. It is not a new
  `HarnessOp` or ROM ABI.
- The VBlank vector patch is typed and instruction-encoded, but still applied
  after ROM assembly. First-class interrupt-vector placement in `gbf-asm` is
  tracked as follow-up work (`bd-18bu`, `bd-3q2w`, `bd-3s0s`, `bd-1q40`) so
  future packets do not mutate vector bytes outside layout ownership.
- The L4 gate closed with the RFC's final fallback `KLaneRow` yield quantum.
  The realism report records this knob so future faster kernels can compare
  against the same packet contract.

# L4 Iterations

Initial checked streaming-ROM baseline used `KLaneFullTile`, with a tile-safe
checkpoint only. That proved L0-L3 byte-exactness but missed the cooperative
60 Hz frame-service gate because full-tile compute spans multiple frames.

The first `KLaneRow` ROM yielded per k-lane row but still used the
`signed_repeated_add_i8_i32` microkernel. Debug and bench evidence showed the
output remained byte-exact, but the worst unyielded interval was roughly
36,570 M-cycles, above the 17,428 M-cycle soft-deadline ceiling.

The closing packet uses the RFC's final fallback `KLaneRow` plus the checked
`fixed_8_step_shift_add_i8_i32` microkernel. Local heavy L4 evidence for
`N = 128` passed with zero frame-service misses, byte-exact output, bounded
no-progress frames, and max yield/bank-hold gaps under the
`FRAME_M_CYCLES - 128` deadline. The fast N=32 gate measured a max unyielded
gap of 17,000 M-cycles against the 17,428 M-cycle deadline after switching to
the RFC-shaped `kk -> mm -> nn` row-k-lane loop. The report records
`KLaneRow` in `runtime_knobs.yield_quantum`.

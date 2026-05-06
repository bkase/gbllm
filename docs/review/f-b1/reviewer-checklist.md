# Reviewer Checklist

- Confirm no production crate has `gbf-verify` as a non-dev dependency.
- Confirm no new `HarnessOp` variant was added for F-B1.
- Confirm `WramSmoke` appears only in smoke tests and not the checked report.
- Confirm `realism_report.v1.json` validates against `realism_report.v1` schema.
- Confirm `realism_report.v1.json` records `runtime_knobs.yield_quantum == "KLaneRow"` and `soft_deadline_margin_mcycles == 128`.
- Confirm `realism_report.v1.json` records `multiply_kernel == "fixed_8_step_shift_add_i8_i32"` and `rom_bank0_table_bytes_read == 0`; QST emission is follow-up `bd-3akv`, not an F-B1 closure claim.
- Confirm each checked run has `frame_service_misses == 0`, `widget_update_count == scheduler_service_count == gated_frame_count`, and bounded yield/bank-hold gaps.
- Confirm `frame_trace_n128.summary.md` records real transient `VBlankFired` events sourced from the ROM VBlank handler symbol and starts its gated interval after the startup frame. The raw 45 MiB trace is intentionally regenerated on demand, not checked in.
- Confirm `yield_while_bank_lease_active_count == 0` and `harness_pause_while_bank_lease_active_count == 0`.
- Confirm `scripts/review/f-b1/verify-packet.sh` reruns cleanly.

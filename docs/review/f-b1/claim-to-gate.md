# Claim To Gate

| Claim | Gate |
| --- | --- |
| Exact `i8 x i8 -> i32` reference | `cargo test -p gbf-verify -- matmul_reference_i8_known_fixture` |
| Production/reference fixtures agree | `cargo test -p gbf-codegen -- operand_fixture_matches_verify_for_review_sizes` |
| Emitted packet uses fixed shift/add and zero Bank0 table reads | `cargo test -p gbf-report -- realism_report_v1_accepts_checked_fixture` |
| Future QST table matches reference | `cargo test -p gbf-codegen -- quarter_square_table_matches_verify` |
| Future QST exhaustive i8 multiply parity | `cargo test -p gbf-codegen -- quarter_square_mul_exhaustive_i8` |
| Request rejects invalid bank/tile/layout | `cargo test -p gbf-codegen -- compute_bringup_request_` |
| BankLease-shaped lowering, no raw MBC writes | `cargo test -p gbf-codegen -- f_b1_l2_` |
| No yield while lease active | `cargo test -p gbf-runtime -- f_b1_l2_no_yield_while_banklease_active` |
| L0/L1/L2 output matches reference | `cargo test -p gbf-emu -- f_b1_l` |
| L3 output-tile ROM matches reference slice | `cargo test -p gbf-emu -- f_b1_l3_output_tile_rom_matches_reference_tile` |
| L3 streaming ROM reaches tile-safe dumps | `cargo test -p gbf-emu -- f_b1_l3_streaming_rom_matches_reference_n32` |
| N sweep streaming ROM packet regenerates | `cargo run -p gbf-test --bin f_b1_regen` |
| L4 cooperative runtime gate | `cargo test -p gbf-emu --lib f_b1_l4 -- --ignored`; `cargo test -p gbf-bench -- f_b1_l4_emulated_partial_run_services_frames_n32`; report validator requires zero misses and pinned `KLaneRow` knobs |
| Report schema and self hash | `cargo test -p gbf-report -- realism_report_v1` |

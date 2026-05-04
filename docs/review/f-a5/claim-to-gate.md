# Claim To Gate

| RFC claim | Implementation | Gate |
| --- | --- | --- |
| Boot/header/vector split is RFC-shaped | `gbf-runtime/src/boot.rs` | `cargo test -p gbf-runtime --lib boot::` |
| ISR handlers are Bank0, IME-disabled, and label-linked | `boot.rs`, `interrupts.rs`, linked-image hash tests | `cargo test -p gbf-runtime --lib interrupts::` |
| TIMA sets a cooperative yield flag, not preemption, and safe-point polling branches over compiler-owned yield code when the flag is clear | `scheduler.rs`, `interrupts.rs` | `cargo test -p gbf-runtime --lib scheduler::yield_check_emits_expected_sequence`; `cargo test -p gbf-runtime --lib scheduler::` |
| Joypad reader owns active-low decode and WRAM cache | `joypad.rs` | `cargo test -p gbf-runtime --lib joypad::` |
| Text does not write VRAM directly | `text.rs`, `video_commit.rs` | `cargo test -p gbf-runtime --lib text::` |
| Keyboard reads cached joypad state and stages glyph redraws, including backspace erase redraws | `keyboard.rs`, `video_commit.rs` | `cargo test -p gbf-runtime --lib keyboard_step_accepts_selected_layout_cell_in_real_emu`; `cargo test -p gbf-runtime --lib keyboard_backspace_enqueues_blank_glyph_in_real_emu` |
| video_commit is the normal sole VRAM/OAM writer and faults outside legal LCD modes | `video_commit.rs` | `cargo test -p gbf-runtime --lib video_commit::`; `cargo test -p gbf-runtime --lib video_commit_drains_glyph_cell_to_bg_map_in_real_emu`; `cargo test -p gbf-runtime --lib video_commit_illegal_mode_raises_fault_in_real_emu` |
| VBlank fill work is bounded by one tile row and rechecks VBlank before each cell write | `video_commit.rs` | `cargo test -p gbf-runtime --lib fill_run_wire_len_is_bounded`; `cargo test -p gbf-runtime --lib fill_run_runtime_len_is_clamped_before_loop` |
| Panic is the audited direct-VRAM bypass with 16-bit fault display | `panic.rs` | `cargo test -p gbf-runtime --lib panic::`; `cargo test -p gbf-runtime --lib panic_entry_renders_fault_code_in_real_emu` |
| runtime_nucleus_hash excludes compile profile and zeroes lineage hashes | `lib.rs` | `cargo test -p gbf-runtime --lib runtime_nucleus_hash` |

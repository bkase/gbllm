# Source To Artifact Map

| RFC area | Source artifact | Test / evidence |
|---|---|---|
| §3.1 session schema | `gbf-debug/src/session.rs` | `cargo test -p gbf-debug --test session_wire session_wire_rejects_bad_container_inputs`; `cargo test -p gbf-debug --test session_wire session_wire_rejects_schema_and_lineage_mismatches`; `session-wire.md` |
| §3.2 script host determinism | `gbf-debug/src/script.rs` | `cargo test -p gbf-debug --test e2e date_now_and_log_timestamps_are_relative_to_each_exec`; `correctness-dossier.md` |
| §3.3 `gb` object | `gbf-debug/src/script.rs` | `api-surface.md`; e2e recipe test |
| §3.4 stateless CLI | `gbf-debug/src/bin/gbf-debug.rs`, `gbf-debug/src/cli.rs` | `cargo test -p gbf-debug --test e2e cli_arg_errors_are_json_exit_one`; `cli-envelopes.md` |
| §3.6 symbols | `gbf-debug/src/session.rs` | `cargo test -p gbf-debug --test session_wire symbols_sorted_and_ambiguous` |
| §3.7 skill | `.agents/skills/gbf-debug-usage/` | `cargo test -p gbf-debug --test skill_recipes` |
| §14 review packet | `xtask/src/main.rs`, `docs/review/f-a8/` | `cargo xtask regen-review-packet --feature F-A8` |

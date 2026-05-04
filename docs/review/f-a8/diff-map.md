# Diff Map

| Path | Risk | Why review it | Gate |
|---|---:|---|---|
| `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml` | Medium | Adds the crate, pinned JS/compression deps, and `cargo xtask` alias. | `cargo tree -e features -p gbf-debug` |
| `gbf-debug/src/session.rs` | High | Defines the durable `.gbsess` wire format and all load-time refusal checks. | `cargo test -p gbf-debug --test session_wire` |
| `gbf-debug/src/script.rs` | High | Hosts QuickJS, predicate evaluation, the `gb` object, trace exposure, and deterministic stubs. | `cargo test -p gbf-debug --test e2e` |
| `gbf-debug/src/cli.rs` | High | Owns `init`/`exec`/`inspect` envelopes, deterministic error sessions, timeout partials, and lineage. | `cargo test -p gbf-debug --test e2e deterministic_script_errors_write_normal_error_session` |
| `gbf-debug/src/bin/gbf-debug.rs` | Medium | Converts clap outcomes into exactly one JSON object on stdout/stderr. | `cargo test -p gbf-debug --test e2e cli_arg_errors_are_json_exit_one` |
| `.agents/skills/gbf-debug-usage/` | Medium | Teaches future agents the actual CLI surface and determinism rules. | `cargo test -p gbf-debug --test skill_recipes` |
| `xtask/src/main.rs`, `docs/review/f-a8/` | Medium | Regenerates reviewer evidence and dependency audit artifacts. | `cargo xtask regen-review-packet --feature F-A8` |

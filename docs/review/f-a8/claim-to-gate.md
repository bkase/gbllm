# Claim To Gate

| Claim | Gate |
|---|---|
| Session container rejects bad magic, flags, truncation, zstd decode, JSON decode, schema mismatch, ROM hash mismatch, and snapshot lineage mismatch. | `cargo test -p gbf-debug --test session_wire session_wire_rejects_bad_container_inputs`; `cargo test -p gbf-debug --test session_wire session_wire_rejects_schema_and_lineage_mismatches` |
| Symbol ambiguity and watchpoint kind contracts are pinned. | `cargo test -p gbf-debug --test session_wire symbols_sorted_and_ambiguous`; `cargo test -p gbf-debug --test session_wire watchpoint_kind_parse_contract` |
| `init -> exec -> inspect` works over the tiny ROM fixture. | `cargo test -p gbf-debug --test e2e init_exec_inspect_tiny_rom` |
| String predicates persist and re-evaluate without becoming unconditional. | `cargo test -p gbf-debug --test e2e string_predicate_round_trips_without_becoming_unconditional` |
| Closure predicates see predicate scope, are invocation-local, and warn. | `cargo test -p gbf-debug --test e2e closure_predicate_is_invocation_local_and_warned` |
| Deterministic script errors write normal post-error sessions, not timeout partials. | `cargo test -p gbf-debug --test e2e deterministic_script_errors_write_normal_error_session` |
| CLI arg/help output is structured JSON with RFC exit codes. | `cargo test -p gbf-debug --test e2e cli_arg_errors_are_json_exit_one`; `cargo test -p gbf-debug --test e2e cli_help_is_json_exit_zero` |
| Virtual time is per script invocation. | `cargo test -p gbf-debug --test e2e date_now_and_log_timestamps_are_relative_to_each_exec` |
| Skill recipes match the actual CLI/API surface. | `cargo test -p gbf-debug --test skill_recipes` |
| Dependency feature audit is current. | `cargo tree -e features -p gbf-debug`; see `dependency-tree.txt` |
| Review packet is reproducible. | `cargo xtask regen-review-packet --feature F-A8` |
| Workspace integration remains healthy. | pre-commit hook: fmt, clippy, workspace tests |

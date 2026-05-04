# Error Shape Report

| Error family | Exit code | JSON `kind` |
|---|---:|---|
| Help | 0 | `help` |
| Clap / CLI argument errors | 1 | `cli_args` |
| Session load failures | 2 | `session_load` |
| Session write failures | 3 | `session_write` |
| Script syntax/runtime/host failures | 4 | `script_syntax`, `script_runtime`, `watchdog_timeout`, `host_binding` |
| Predicate compile failures | 5 | `predicate_compile` |
| Emulator / I/O / post-load PC failures | 6 | `io` or `post_load_pc` |
| Symbol hydration failures | 7 | `symbol_hydration` |

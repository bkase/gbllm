# Correctness Dossier

## Session Schema

Typed enums cover predicate kind, watchpoint kind, and trace event kind. `Session::load_bytes` validates container shape before serde and validates ROM/snapshot lineage after serde.

## JS Host Determinism

`Date.now()` and `log(...).ts_micros_since_script_start` are derived from emulator cycles relative to the current script invocation. `Math.random()` is fixed-seeded. `console` is removed. A QuickJS interrupt handler and bounded Rust host calls enforce liveness without making wall-clock time observable.

## Predicates

String predicates are compiled at registration or startup and evaluated in a restricted scope containing `regs`, `pc`, `access`, `cycle`, `symbol`, and `symbolInBank`. Closure predicates stay in the JS heap under an invocation-local key, see the same temporary predicate scope plus a read-only `gb` view, and are not written to the session.

## CLI

Every success path writes one JSON object to stdout. Every failure path writes one JSON `ErrorEnvelope` to stderr, including clap parse failures. Deterministic script failures write a normal post-error `session_path`; timeout partials use `partial_session_path` and `determinism: "nondeterministic_partial"` only when requested.

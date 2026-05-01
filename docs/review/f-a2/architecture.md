# Architecture

`gbf-hw` is a pure contract crate. It owns constants, exhaustive enums, value objects, and constructor-validated schema types. It does not perform I/O, maintain runtime state, emit assembly, or produce calibration bundles.

Dependency direction:

```text
gbf-foundation -> gbf-hw -> consumers
```

`gbf-hw` depends only on `gbf-foundation` and `serde` in production. `serde_json` is dev-only.

The target profile bundles identity, console, cartridge, timing, and capabilities. Memory, MBC5, LCD, interrupt, and joypad modules expose constants and predicates. Calibration carries schema only; measurement production belongs to `gbf-bench`.

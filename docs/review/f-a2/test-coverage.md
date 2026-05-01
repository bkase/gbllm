# Test Coverage

Focused command:

```bash
cargo test -p gbf-hw
```

Current groups:

- 92 unit tests across `cartridge_header`, `target`, `memory`, `mbc5`, `lcd`, `timing`, `interrupts`, `joypad`, and `calibration`.
- 4 cross-module conformance tests.
- 1 ignored single-source smoke test.
- 1 `compile_fail` doctest proving no loose MBC5 RAM-enable predicate is exported.

Compatibility command:

```bash
cargo test -p gbf-asm
```

This verifies the `gbf-asm::rom` public API remains source-compatible after the re-export migration.

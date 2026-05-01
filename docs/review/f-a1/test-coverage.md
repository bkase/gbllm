# Test Coverage

Current focused gate:

```bash
cargo test -p gbf-asm --all-features
```

This runs unit tests for builder, ISA, effect typing, cycle model, encoder, layout, lowering, relax, listing, ROM assembly, and symbol output. Packet verification also runs the tiny ROM example and compares regenerated artifacts against the checked packet files.

The full workspace gate is still enforced by the repository pre-commit hook.

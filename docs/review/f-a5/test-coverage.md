# Test Coverage

Focused gates run during implementation:

- `cargo test -p gbf-runtime --lib` — 129 tests, including F-A7 `gbf-emu` execution of boot, joypad, keyboard, video commit, illegal-mode fault, and panic paths
- `cargo run -p gbf-runtime --example demo_bank0_rom -- target/review/f-a5`

Expected final PR gates:

- `cargo test -p gbf-abi`
- `cargo test -p gbf-asm`
- `cargo test -p gbf-runtime`
- `scripts/review/f-a5/verify-packet.sh`
- workspace pre-commit hook on commit: fmt, clippy, tests

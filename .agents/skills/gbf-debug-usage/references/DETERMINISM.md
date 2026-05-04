# Determinism

- `.gbsess` files use `GBSE` magic, zero flags, zstd level 3, and JSON.
- The session embeds ROM bytes and symbols, so later `exec` calls do not need sidecars.
- `Date.now()` is virtual emulator time.
- `Math.random()` is fixed-seeded.
- Default envelopes omit host timing.
- Watchdog timeouts are liveness failures; partial sessions are marked nondeterministic.


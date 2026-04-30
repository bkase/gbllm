# Reproducibility

Build or refresh packet artifacts:

```bash
./scripts/review/f-a1/build-packet.sh
```

Verify checked artifacts from a clean tree:

```bash
./scripts/review/f-a1/verify-packet.sh
```

The verifier runs `cargo test -p gbf-asm --all-features`, regenerates `target/review/f-a1/tiny_rom.gb`, `.lst`, and `.sym`, compares them byte-for-byte with `docs/review/f-a1/artifacts/`, and checks the SHA-256 manifest.

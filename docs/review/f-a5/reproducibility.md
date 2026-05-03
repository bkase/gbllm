# Reproducibility

Regenerate the review artifacts with:

```bash
cargo run -p gbf-runtime --example demo_bank0_rom -- target/review/f-a5
```

The normalized Bank0 image is built from the same section builders as the runtime API, with fixed F-A5 placement anchors, branch relaxation, the Nintendo logo/header region stamped, global checksum bytes zeroed, and the F-A3 `BuildIdentityBlock` lineage hash fields zeroed.


# F-A4 Reproducibility

Run `scripts/review/f-a4/verify-packet.sh` from the repository root. The
script verifies formatting, focused F-A4 runtime and asm gates, the banking
demo byte stream against `artifacts/acquire_release.bin`, the diagram/listing
artifacts, the no-unsafe grep, and the review packet's required files.

The final PR gate additionally runs:

```bash
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
```

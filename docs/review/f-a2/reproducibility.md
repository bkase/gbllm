# Reproducibility

One command verifies the packet:

```bash
scripts/review/f-a2/verify-packet.sh
```

The verifier runs:

- `cargo fmt --check --all`
- `cargo test -p gbf-hw`
- `cargo test -p gbf-asm`
- `cargo clippy -p gbf-hw -- -D warnings`
- `scripts/lints/no-hw-literal-redeclarations.py`
- `cargo tree -p gbf-hw`
- `git diff --check`

It does not run the ignored single-source smoke test; that gate is intentionally deferred.

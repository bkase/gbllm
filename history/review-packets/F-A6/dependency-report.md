# F-A6 Dependency Report

Resolved dependency graph:

```text
gbf-store v0.1.0 (/Users/bkase/.codex/worktrees/d5a4/gbllm/gbf-store)
├── gbf-foundation v0.1.0 (/Users/bkase/.codex/worktrees/d5a4/gbllm/gbf-foundation)
├── serde v1.0.228
├── sha2 v0.10.9
└── tempfile v3.27.0
[dev-dependencies]
└── serde_json v1.0.149
```

Production dependency posture:

- `gbf-store` depends on `gbf-foundation`, `serde`, `sha2`, and `tempfile`.
- `serde_json` is dev-only for JSON shape tests.
- `gbf-artifact` is removed from `gbf-store/Cargo.toml`.
- No `gbf-abi`, `gbf-codegen`, `gbf-report`, or other contract/product crate dependency leaks into `gbf-store`.
- `gbf-migrate/Cargo.toml` is unchanged; its existing deferred dependency cleanup belongs to F-A6b.

Verification commands:

```bash
cargo tree -p gbf-store --depth 1
git diff --exit-code -- gbf-migrate
```

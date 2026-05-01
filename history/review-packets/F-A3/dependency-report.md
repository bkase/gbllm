# F-A3 Dependency Report

## Feature Gates

- `default = ["host"]`
- `host = ["std", "alloc", "dep:gbf-foundation"]`
- `std = ["serde/std"]`
- `alloc = ["serde/alloc"]`

## No-Upward-Dependency Evidence

PASS: no upward contract dependency appears in `cargo tree -p gbf-abi --edges normal --features host`.

Forbidden upward dependencies checked: gbf-runtime, gbf-codegen, gbf-asm, gbf-artifact, gbf-hw.

## Cargo Tree

```text
gbf-abi v0.1.0 (./gbf-abi)
├── gbf-foundation v0.1.0 (./gbf-foundation)
│   └── serde v1.0.228
│       ├── serde_core v1.0.228
│       └── serde_derive v1.0.228 (proc-macro)
│           ├── proc-macro2 v1.0.106
│           │   └── unicode-ident v1.0.24
│           ├── quote v1.0.45
│           │   └── proc-macro2 v1.0.106 (*)
│           └── syn v2.0.117
│               ├── proc-macro2 v1.0.106 (*)
│               ├── quote v1.0.45 (*)
│               └── unicode-ident v1.0.24
├── memoffset v0.9.1
└── serde v1.0.228 (*)
```

## License Summary

Workspace path crates in this tree do not declare package licenses. Registry dependencies are standard Rust ecosystem crates used for serde derives, JSON test support, property tests, and field offsets.

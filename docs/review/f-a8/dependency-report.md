# Dependency Report

Direct dependencies: `gbf-emu`, `gbf-asm`, `gbf-foundation`, `gbf-abi`, `gbf-hw`, `rquickjs`, `rquickjs-serde`, `zstd`, `base64`, `sha2`, `clap`, `serde`, `serde_json`, `tempfile`. `gameroy-core` enters only through `gbf-emu`.

`rquickjs` is pinned with default features disabled and only `std` enabled. Module loading, loader features, custom allocator features, and async/futures features are not part of the F-A8 surface.

See `dependency-tree.txt` for `cargo tree -e features -p gbf-debug`.

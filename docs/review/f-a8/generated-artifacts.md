# Generated Artifacts

`cargo xtask regen-review-packet --feature F-A8` writes this directory, `dependency-tree.txt`, `cargo-metadata.json`, `unsafe-grep.txt`, and reproducibility hashes generated from an `init -> exec` tiny-ROM session pair.

No checked-in `.gbsess` binary fixture is required for F-A8; the deterministic session bytes are regenerated into a temp directory and recorded by hash in `reproducibility.md`.

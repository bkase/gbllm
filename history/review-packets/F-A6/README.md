# F-A6 Review Packet: gbf-store

## Scope Statement

This packet reviews RFC `history/rfcs/F-A6-gbf-store-migrate.md` for feature bead `bd-3ll` after the RFC's deferral pass. The PR closes `gbf-store` only: foundation `BlobRef`/`BlobCodec`, the content-addressed `BlobStore`, integrity checks, `StageCache`, pinsets, GC, and archive transport.

Deferred: `gbf-migrate` remains a stub under bead `bd-n9i`, now deferred to the future F-A6b feature. This PR intentionally does not modify `gbf-migrate/`.

## Reading Order

1. Read `gbf-foundation/src/blob.rs` and `gbf-store/src/blob.rs` first; they establish the content-addressed identity invariant.
2. Read `gbf-store/src/stage_cache.rs` next; review `try_compose_key` byte encoding closely.
3. Read `gbf-store/src/gc.rs` and `gbf-store/src/pinset.rs` together; GC is pinset-driven and registry-decoded.
4. Read `gbf-store/src/archive.rs` last; it ties pinsets, transitive references, and content-addressed records together.
5. Use `gbf-store/tests/f_a6.rs`, `gbf-foundation/tests/blob.rs`, and `claim-to-gate.md` as the proof index.

## Changed File Disposition

| File | Risk | Disposition | Primary gate |
| --- | --- | --- | --- |
| `.beads/issues.jsonl` | Medium | Generated bead metadata; scope/status source-of-truth updates. | br sync --flush-only + beads_pr_scope_check |
| `Cargo.lock` | Medium | Config/dependency review. | cargo tree -p gbf-store --depth 1 |
| `Cargo.toml` | Medium | Config/dependency review. | cargo tree -p gbf-store --depth 1 |
| `gbf-foundation/src/blob.rs` | Medium | Boundary review for durable BlobRef shape. | cargo test -p gbf-foundation |
| `gbf-foundation/src/lib.rs` | Medium | Mechanical re-export. | cargo test -p gbf-foundation |
| `gbf-foundation/tests/blob.rs` | Medium | Fixture and proof review. | cargo test -p gbf-foundation |
| `gbf-store/Cargo.toml` | Medium | Config/dependency review. | cargo tree -p gbf-store --depth 1 |
| `gbf-store/src/archive.rs` | High | Deep implementation review. | cargo test -p gbf-store |
| `gbf-store/src/blob.rs` | High | Deep implementation review. | cargo test -p gbf-store |
| `gbf-store/src/gc.rs` | High | Deep implementation review. | cargo test -p gbf-store |
| `gbf-store/src/integrity.rs` | High | Deep implementation review. | cargo test -p gbf-store |
| `gbf-store/src/lib.rs` | High | Deep implementation review. | cargo test -p gbf-store |
| `gbf-store/src/pinset.rs` | High | Deep implementation review. | cargo test -p gbf-store |
| `gbf-store/src/stage_cache.rs` | High | Deep implementation review. | cargo test -p gbf-store |
| `gbf-store/tests/f_a6.rs` | High | Fixture and proof review. | cargo test -p gbf-store |
| `history/review-packets/F-A6/README.md` | Low | Generated review packet artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/artifact-manifest.json` | Low | Generated review packet artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/claim-to-gate.md` | Low | Generated review packet artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/dependency-report.md` | Low | Generated review packet artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/archive-layout.mmd` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/archive-layout.svg` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/atomic-write.mmd` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/atomic-write.svg` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/directory-layout.mmd` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/directory-layout.svg` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/gc-keep-walk.mmd` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/gc-keep-walk.svg` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/stage-key.mmd` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/diagrams/stage-key.svg` | Low | Generated diagram artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `history/review-packets/F-A6/reviewer-checklist.md` | Low | Generated review packet artifact. | python3 scripts/generate_f_a6_review_packet.py --check |
| `scripts/generate_f_a6_review_packet.py` | Medium | Review-packet reproducibility script. | python3 scripts/generate_f_a6_review_packet.py --check |

## Architecture Brief

`gbf-store` is intentionally filesystem-shaped infrastructure. Blob bytes live under `blobs/sha256/<prefix>/<hash>`. The stage cache is a weak index layered over the blob store: cache files map `StageCacheKey` to payload `Hash256`, but payload bytes still live in the CAS. Pinsets are explicit GC roots. GC walks pinsets transitively through `BlobReferencesRegistry`, then removes unpinned blobs in deterministic hash order. Archives carry pinsets plus sorted content-addressed blob records for cross-machine transport.

Dependency direction is one-way: `gbf-store` depends on `gbf-foundation`, `serde`, `sha2`, and `tempfile`; it does not depend on `gbf-artifact`, `gbf-abi`, `gbf-codegen`, or any contract/product crate. `gbf-migrate` stays unchanged because no production schema bump exists yet.

## Correctness Dossier

- Atomic writes: `BlobStore::commit_bytes` writes `tmp/blob-*.tmp`, syncs the file, then renames into the canonical path. `DurabilityMode::Full` adds best-effort parent-directory sync. Gates: `blob::atomic_writes`, `blob::open_preserves_tmp_files`, `blob::idempotent_verifies_existing`.
- Content-addressed determinism: `put` hashes bytes before commit; `put_expect` rejects mismatches before commit; existing canonical files are stream-verified before reuse. Gates: `blob::round_trip`, `blob::content_addressed_idempotent`, `blob::put_expect_hash_mismatch`, `integrity::*`.
- `BlobRef` populate: `gbf-foundation::blob::BlobRef { hash, len, codec }` is serde-tested and consumed by `BlobStore::put_as` / `get_ref`. Gates: `gbf-foundation/tests/blob.rs`, `blob::put_as_returns_blob_ref`, `blob::get_ref_validates_len`.
- `compose_key` determinism: all variable fields are u32 length-prefixed; maps and sets are `BTree*`; SemVer components are checked into u32. Gates: `stage_cache::deterministic_keys`, `stage_cache::compose_key_length_prefix_safe`, `stage_cache::pass_version_component_overflow_rejected`.
- GC keep-set walk: roots are traversed breadth-first through registered byte recognizers. Unknown blobs default to abort. Gates: `gc::pinset_protection`, `gc::transitive_refs_via_registry`, `gc::unknown_reference_policy_abort`, `gc::unknown_reference_policy_treat_as_leaf`.
- Archive byte stability: fixed 24-byte header, sorted pinsets, sorted blob hashes, preflight source verification, declared-byte accounting, trailing-byte rejection. Gates: `archive::header_size_24_bytes`, `archive::deterministic_bytes`, `archive::extract_rejects_record_exceeding_declared_total_before_commit`, `archive::trailing_bytes_rejected`.
- Cargo cleanup: `gbf-store/Cargo.toml` drops the placeholder `gbf-artifact` dependency. Gate: `cargo tree -p gbf-store --depth 1` in `dependency-report.md`.

## Filesystem-Semantics Citation Table

| Property | Implementation reliance | Gate |
| --- | --- | --- |
| Same-filesystem rename is atomic for readers of the final path | temp file is created under `store/tmp`, then renamed into `blobs/sha256` | `blob::atomic_writes` |
| A synced file has bytes durable enough for read-after-return on the same machine | `NamedTempFile` is written and `sync_all` runs before rename | code inspection plus `DurabilityMode` docs |
| Directory-entry power-loss durability is stronger than read-after-return | `DurabilityMode::Full` best-effort syncs the destination directory after rename | `DurabilityMode` docs |
| `read_dir` is an observation, not a lock | GC tolerates missing candidates and index files that disappear after listing | `gc::*` plus code path in `run_gc` |
| Metadata length is advisory unless bytes hash back to the path hash | archive creation preflights and rechecks source hashes while streaming | `archive::create_rejects_corrupt_existing_blob_before_writing` |

## Test Coverage Report

Run:

```bash
cargo test -p gbf-foundation -p gbf-store
cargo test -p gbf-migrate
cargo test -p gbf-store --doc
cargo clippy -p gbf-store --all-features --tests -- -D warnings
python3 scripts/generate_f_a6_review_packet.py --check --run-tests
```

`gbf-store/tests/f_a6.rs` groups the behavioral proof by module: blob, integrity, stage cache, pinset, GC, and archive. The atomic-write crash test is a fixture-local simulation: it writes only the temp file, reopens the store, and proves the canonical path did not appear. `cargo test -p gbf-migrate` confirms the deferred crate remains stub-only and contributes no F-A6 tests.

## Reproducibility Report

One command checks the packet and the packet-owned gates:

```bash
python3 scripts/generate_f_a6_review_packet.py --check --run-tests
```

Regenerate checked-in packet artifacts with:

```bash
python3 scripts/generate_f_a6_review_packet.py
```

## Generated Artifacts Manifest

`artifact-manifest.json` records a SHA-256 fingerprint for every generated packet artifact except itself. The generator recomputes markdown, Mermaid sources, SVG companion renders, the dependency report, and the manifest, then `--check` byte-compares all outputs.

## Dependency Report

See `dependency-report.md`. The short form is: production deps are `gbf-foundation`, `serde`, `sha2`, and `tempfile`; dev-only `serde_json`; no `gbf-artifact`; no `gbf-migrate` edit.

## Known Debt Ledger

| Debt | Owner |
| --- | --- |
| `gbf-migrate` schema DAG, epochs, and reports are absent by design | F-A6b / `bd-n9i` |
| Archive extraction is still record-buffered and capped at 256 MiB per record | future streaming archive extractor bead when blobs exceed this size |
| StageCache is a library surface only; no compiler stage calls it yet | F-B15 StageCache integration |
| No CLI for `store gc`, `store verify`, or `archive list` | future `gbf-cli` store front-end bead |

## Out-Of-Scope Ledger

| Looks like F-A6 | Actual owner |
| --- | --- |
| Concrete artifact/report/reference decoders for `BlobReferencesRegistry` | owning schema crates |
| Production pinset policy such as "latest 5 builds" | future build/report orchestration |
| Async I/O or multi-process locking | future scale/concurrency bead if needed |
| Runtime SRAM state migration | `gbf-runtime::persistence`, not `gbf-migrate` |
| Schema migration fixtures | F-A6b |

## API Guide

- `gbf_foundation::blob`: `BlobCodec`, `BlobRef`.
- `gbf_store::blob`: `BlobStore`, `DurabilityMode`, `BlobStoreError`, `put`, `put_expect`, `put_streaming`, `put_as`, `get`, `get_ref`, `exists`, `remove`, `list_blobs`, `cleanup_tmp`.
- `gbf_store::integrity`: `verify_integrity`, `verify_all`, `verify_reachable`, `IntegrityReport`, `IntegrityError`.
- `gbf_store::stage_cache`: `StageKey`, `ComponentDigestSet`, `StageCacheKey`, `StageCacheEntry`, `StageCache`, `compose_key`, `try_compose_key`, `StageCacheError`.
- `gbf_store::pinset`: `PinsetName`, `Pinset`, typed in-memory `BlobReferences`.
- `gbf_store::gc`: `BlobReferencesRegistry`, `BlobReferenceReader`, `UnknownReferencePolicy`, `GcOptions`, `GcReport`, `run_gc`.
- `gbf_store::archive`: `ArchiveHeader`, `ArchiveContents`, `ExtractedArchive`, `create_archive`, `list_archive`, `extract_archive`, `ArchiveError`.

## Cleanliness Audit

`gbf-store/src/lib.rs` has `#![forbid(unsafe_code)]`. No `unsafe` is introduced in `gbf-store/src`. No async runtime or coordination library was added. `gbf-store/Cargo.toml` no longer depends on `gbf-artifact`. `gbf-migrate/Cargo.toml` is unchanged.

## Source-To-Artifact Traceability

Archive magic/version path:

| Source | Implementation | Gate |
| --- | --- | --- |
| RFC F-A6 §10: magic `GBLM\0ARC`, version 1, 24-byte header | `ARCHIVE_MAGIC`, `ARCHIVE_VERSION`, `ARCHIVE_HEADER_LEN`, `write_header`, `read_header` | `archive::magic_bytes`, `archive::version_byte`, `archive::header_size_24_bytes` |

StageKey encoding path:

| Source | Implementation | Gate |
| --- | --- | --- |
| RFC F-A6 §7: stage id, shard-local map, global hash, feature flags, pass version | `try_compose_key`, `update_string`, `semver_u32`, `len_u32` | `stage_cache::deterministic_keys`, `stage_cache::compose_key_length_prefix_safe` |

## Diagrams

Mermaid sources and SVG companion renders live in `diagrams/`:

- `directory-layout`
- `stage-key`
- `gc-keep-walk`
- `archive-layout`
- `atomic-write`

## BlobRef Populate Evidence

`gbf-foundation/src/blob.rs` defines the durable reference shape and `gbf-foundation/src/lib.rs` re-exports it. `gbf-store::BlobStore::put_as` is the first consumer, returning a `BlobRef` for bytes stored in the CAS.

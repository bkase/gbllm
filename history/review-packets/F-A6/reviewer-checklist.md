# F-A6 Reviewer Checklist

- [ ] The PR touches no `gbf-migrate/` paths.
- [ ] Every file in `gh pr diff --name-only` appears exactly once in the Changed File Disposition table.
- [ ] `gbf-store/src/lib.rs` forbids unsafe code.
- [ ] `gbf-store/Cargo.toml` has no `gbf-artifact` dependency.
- [ ] `BlobRef` is implemented in foundation and consumed by `BlobStore::put_as`.
- [ ] `BlobStore::put_expect` rejects a bad claimed hash before commit.
- [ ] Existing canonical blobs are stream-verified before reuse.
- [ ] `StageKey` construction order does not affect `compose_key`.
- [ ] `StageCacheKey` is distinct from content `Hash256` at the API boundary.
- [ ] GC default unknown-reference behavior is abort, not leaf.
- [ ] Dry-run GC reports candidates without removing blobs or indexes.
- [ ] Archive creation is deterministic and preflights source hash integrity.
- [ ] Archive extraction rejects over-budget records before committing them.
- [ ] The review packet generator passes with `--check --run-tests`.

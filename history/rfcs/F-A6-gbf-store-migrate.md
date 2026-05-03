# RFC F-A6: `gbf-store` — content-addressed storage, two-level StageCache, pinsets/GC, archive transport (`gbf-migrate` scaffolding **deferred**)

| Field          | Value                                                                                  |
|----------------|----------------------------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A6)                                                       |
| Status         | Draft (single-PR closure of T-A6.0–T-A6.4; precondition: populate `gbf-foundation::blob::BlobRef`). **T-A6.5 (`gbf-migrate` scaffolding) is deferred — see §0.0.0.** |
| Feature bead   | `bd-3ll` (re-scoped to `gbf-store` only)                                               |
| Open tasks     | T-A6.0 (foundation `BlobRef`, precondition), T-A6.1 (blob + integrity), T-A6.2 (StageCache), T-A6.3 (pinsets + GC), T-A6.4 (archive transport) |
| Deferred tasks | T-A6.5 (`gbf-migrate::epochs` + `dag` + `report`, bead `bd-n9i`) — see §0.0.0          |
| Closed tasks   | none — every `gbf-store` module is still `//! Module stub.`                            |
| Plan reference | `history/planv0.md` line 137 (`gbf-foundation` owns `BlobRef`), line 138 (`gbf-store` modules), line 139 (`gbf-migrate` modules), line 269 (`gbf-store` is the dedicated CAS/StageCache crate, exists separate from `gbf-artifact`), line 311 (`gbf-codegen` consumes the always-on `StageCache`), line 1126 ("core crates define current schemas only; host-side migration logic lives in `gbf-migrate`"), line 1504 (`CompatibilityEpochs`), line 1511 (`MigrationReport`), line 1519 (`MigrationLossClass`), line 2160 (runtime `StateMigrator` is *not* gbf-migrate's concern), line 2511 (two-level `StageCache`: shard-local + global), line 2863 (always-on `StageCache` keys), line 2940 (rule 20: shard-local + global), line 2986 (`BlobRef` / `BlobCodec` / `ComponentDigestSet` / `BuildShardManifest` / `CompatibilityEpochs` / `MigrationReport` are the first shared types) |
| Glossary       | `history/glossary.md` (uses existing terms; introduces no new RFC vocabulary)          |
| Constitution   | `CONSTITUTION.md` §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds), §V.3 (silence on success, loud on failure), §VI.1 (single source of truth) |

## 0.0.0 Deferral notice — `gbf-migrate` is OUT OF SCOPE for F-A6

> **F-A6 ships `gbf-store` only. `gbf-migrate` stays a stub. Do not implement it under this RFC.**
>
> The original F-A6 scope bundled a `gbf-migrate` scaffolding (epochs, the `Migrator` trait, the `MigrationGraph`, `MigrationReport`, and identity/bump-minor test fixtures). After review, that work has been **deferred** to a future feature bead. The motivation is recorded inline below; the practical effect is:
>
> - **The PR that closes F-A6 must not touch `gbf-migrate/`.** `gbf-migrate/src/{dag.rs, epochs.rs, report.rs}` stay exactly `//! Module stub.`. `gbf-migrate/Cargo.toml` is left as-is. `gbf-migrate/tests/` is not created.
> - **Bead `bd-n9i` (T-A6.5) is deferred** (`br defer bd-n9i`). It is no longer a closure-blocker for the parent feature bead `bd-3ll`.
> - **No migrators ship.** Not even identity / bump-minor test fixtures. Until a real schema bump forces the issue, the workspace assumes "schema mismatch ⇒ rebuild from sources" is acceptable.
> - **Sections §§ 11, 12, 13 of this RFC are preserved as design notes** for the future bead, but each carries its own `DEFERRED` banner. Reviewers of the F-A6 PR should skip those sections.
> - **The claim-to-gate matrix (§19) marks every `gbf-migrate::*` row as deferred.** Closure does not require any migrate test to exist.
> - **The file-by-file change set (§21) drops every `gbf-migrate/*` row from the F-A6 PR.**
>
> ### Why deferred
>
> The `gbf-migrate` scaffolding is *infrastructure for handling future schema bumps*. As of this RFC, the workspace has shipped zero versioned schemas in production: no real artifacts, no measured calibration bundles, no captured failure capsules, no run reports. The cost of "wipe the store and rebuild from sources on schema bump" is therefore essentially zero today, while the cost of carrying ~600 LOC of unused scaffolding plus a permanent maintenance surface (DAG planner, loss-class enum, family scoping, tests) is paid every day F-A6 onward.
>
> Per family, the cost of *eventually* needing a migrator becomes non-zero at different points in the project's life:
>
> - **`calibration`** — costliest because regenerating means physical lab access to the target Game Boy hardware (`gbf-bench` measures on real DMG/MBC5). The first calibration bundle that needs to survive a schema bump is the bead that re-opens migration work.
> - **`artifact`** — cost = recompile time. May be acceptable to wipe-and-rebuild for the lifetime of M0/M1; decide at the first artifact-schema bump.
> - **`abi`** / **`reports`** — cost = lost replay/audit data. Likely acceptable to wipe; revisit if external users start preserving these across schema bumps.
>
> Because none of these triggers exist today, F-A6 is re-scoped to `gbf-store` only. The first real schema bump opens a new feature bead (provisional name **F-A6b: `gbf-migrate` scaffolding**) which inherits §§ 11–13 of this RFC as its starting design.
>
> ### What stays in F-A6
>
> Everything `gbf-store`: `gbf-foundation::blob::{BlobRef, BlobCodec}` populate (T-A6.0), the content-addressed `BlobStore` + integrity (T-A6.1), the two-level `StageCache` (T-A6.2), pinsets + GC (T-A6.3), archive transport (T-A6.4). The `gbf-store/Cargo.toml` cleanup (drop the placeholder `gbf-artifact` dep) stays. None of those depend on `gbf-migrate`, so the deferral is clean.
>
> ### Reading guide for the rest of this RFC
>
> Sections §0.0.1 through §10 describe the in-scope `gbf-store` work and remain authoritative. Sections §§ 11–13 are preserved as design notes for the deferred bead and carry inline `DEFERRED` banners. Sections §§ 14–22 have been edited to drop migrate-specific rows / claims / file changes, keeping only `gbf-store` content.

## Project orientation: where this feature sits

### 0.0.1 The big picture

`gbllm2` is a hardware-aware compiler plus cooperative runtime that targets a real DMG Game Boy with an MBC5 cartridge. The end goal is to run a quantized transformer on a Game Boy Color-class device with a reproducible, agent-debuggable build. The architecture decomposes into five product crates and three shared-contract crates (`gbf-foundation`, `gbf-hw`, `gbf-abi`). Sitting alongside the contracts are two **infrastructure** crates — `gbf-store` and `gbf-migrate` — that are not contracts, not products, and not consumed in the hot path of inference, but are load-bearing for everything else. **F-A6 fills `gbf-store` only; `gbf-migrate` is deferred to a future feature bead (see §0.0.0).**

`gbf-store` is the **dedicated content-addressed storage crate**. Every transformative compiler pass writes its output blob into a `blobs/sha256/` layout; the `StageCache` is the keyed lookup over that layout; the `pinset`/`gc` machinery is what keeps the store from growing unbounded; the `archive` machinery is what lets a build move between machines. Its deliberate isolation from `gbf-artifact` is one of the architectural decisions named explicitly in `planv0.md` line 269 and engineering rule 20: "`gbf-store` is the dedicated CAS/transport/stage-cache crate so `gbf-artifact` stays schema-only and `gbf-codegen` does not grow its own blob store."

`gbf-migrate` is *intended* to be the **dedicated host-side schema-evolution crate**: when a schema bumps, register a migrator N→N+1; when an old artifact arrives, walk the DAG to bring it to current schema. **F-A6 does not build it.** As of this RFC there are zero versioned schemas in production, so the cost of "wipe and rebuild from sources on schema bump" is essentially zero — and that becomes the workspace's working policy until the first real schema bump justifies opening a follow-up bead. The crate stays as a pinned stub (`gbf-migrate/src/{dag,epochs,report}.rs` exactly `//! Module stub.`).

`gbf-store` depends only on `gbf-foundation`. It is not in any inference hot path. It ships no model logic, target logic, ABI logic, or compile logic. Its **single responsibility** is filesystem-shaped infrastructure: a deterministic content-addressed object store.

### 0.0.2 What this feature is for

F-A6 fills the `gbf-store` crate with verified, deterministic, atomic implementations of:

- a content-addressed blob store with atomic write semantics, hash-fan-out directory layout, and integrity verification;
- a two-level `StageCache` whose keys decompose into shard-local digests (where legality is local) and a whole-build hash (where legality is global), with deterministic `compose_key`;
- named `Pinset`s plus a GC that walks pinsets, follows transitive `BlobReferences`, and can run dry/limited; emits a structured `GcReport`;
- a single-file `Archive` format with a fixed magic header and round-trippable `(hash, length, bytes)` records, so a build or calibration cohort can be moved between machines.

**Deferred (NOT in F-A6, see §0.0.0):** the `gbf-migrate` scaffolding (`CompatibilityEpochs`, the `Migrator` trait, the `MigrationGraph` with `plan` + `execute`, and the `MigrationReport` with `MigrationLossClass`, plus identity + bump-minor migrator test fixtures). The first real schema bump opens a follow-up bead (provisional **F-A6b**) that picks this up; until then, `gbf-migrate/src/{dag,epochs,report}.rs` stay `//! Module stub.`.

Concretely, F-A6 unblocks:

- **F-B15 (`StageCache` integration across all stages)** — the wire-up bead that makes every transformative stage in `gbf-codegen` consult the cache. Cannot start until `StageKey`, `compose_key`, and the `BlobStore::put`/`get` primitives exist.
- **F-F1 (build reports — JSON schemas + emit hooks)** — the report-emission infrastructure. Reports go through the same blob store as stage outputs.
- **Cross-machine build/calibration transport.** `gbf-bench` (Epic E) measures calibration on one machine and ships it to another; `gbf-cli` agents may export a build for review. Both need `archive::create_archive` / `extract_archive`.

F-A6 does **not** unblock artifact-schema bump beads. Those are blocked by the deferred `gbf-migrate` bead (F-A6b), which inherits §§ 11–13 of this RFC as its starting design.

F-A6 is **independent** of F-A1 (`gbf-asm`, shipped), F-A2 (`gbf-hw`), F-A3 (`gbf-abi`), F-A4 (`BankLease`), F-A5 (Bank0 runtime), F-A7 (`gbf-emu`), and F-A8 (`gbf-debug`). It depends only on `gbf-foundation`. It is also independent of Epic B (compiler pipeline) and Epic C (oracle stack); those depend on it but it does not depend on them.

### 0.0.3 Why content-addressed determinism is the load-bearing property

The dominant correctness invariant this feature exists to maintain is `hash(write(blob)) == hash(blob)`. Every other property — cache reuse, archive integrity, migration safety — derives from that single equality. A blob store that ever returned different bytes for the same hash would silently corrupt every downstream consumer; a `StageCache` that ever produced different keys for equivalent inputs would either miss legitimate hits (a nuisance) or report false hits (catastrophic — a stale stage output gets fed to the next stage). F-A6's mechanisms for keeping the property are not policy, they are mechanical:

1. **The hash is the path.** Blob `h` is stored at `blobs/sha256/<ab>/<hash>` where `ab` is the first two hex chars of the hash. The split creates 256 shard directories, each containing roughly `total_blobs / 256` files under a uniform hash distribution. There is no other lookup.
2. **Writes are atomic.** Every blob write goes to `tmp/<random>` first, fsyncs, then renames to its final path. A torn write never produces a half-blob at the canonical path.
3. **`StageKey` serialization is canonical.** `BTreeMap` for ordered fields, fixed-encoding for fixed-size fields, sorted feature flags. `compose_key` is a closed-form deterministic serialization, not a hash of `Debug` output.
4. **The archive format is byte-stable.** Magic bytes, fixed-size header fields, sequential records in pinset-then-blob order; same inputs produce identical archive bytes.
5. **Integrity is checkable post-hoc.** `verify_integrity(store, hash)` re-hashes a blob's contents and compares against its filename; `verify_all` walks the store and produces an `IntegrityReport`.

Determinism here is not a performance optimization — it is the contract. A `StageCache` hit is only safe if the cache key is canonical and the stored blob is byte-identical to the one originally written. F-A6's tests treat both properties as hard invariants.

### 0.0.4 What this feature deliberately does *not* do

F-A6 is filesystem-shaped infrastructure. No compiler logic. No artifact schema. No runtime state. No inference. Specifically:

- F-A6 does **not** define `PlanningStage` or any concrete compiler-stage enum. Those variants are owned by `gbf-asm` (`PlanningStage`) and `gbf-codegen`. F-A6 defines an opaque `StageId(string_id)` newtype inside `gbf-store::stage_cache`; consumer crates map their own stage enums to this newtype at the boundary. Only `BlobRef`/`BlobCodec` live in `gbf-foundation`; `StageId`, `ComponentId`, and `FeatureFlag` live in `gbf-store::stage_cache`.
- F-A6 does **not** wire the cache into compiler stages. That is F-B15.
- F-A6 does **not** ship `gbf-migrate` scaffolding (epochs, `Migrator` trait, `MigrationGraph`, `MigrationReport`, `MigrationLossClass`). **All deferred** to a follow-up bead (provisional F-A6b) that opens when the first real schema bump demands it. See §0.0.0.
- F-A6 does **not** ship real schema migrators or even identity/bump-minor test fixtures. The first real migrator is owned by whichever bead first bumps a schema; that same bead will land the `gbf-migrate` scaffolding it needs.
- F-A6 does **not** own the runtime persistence `StateMigrator`. That is `gbf-runtime::persistence` (M0/M1, separate ownership). The (eventual) `gbf-migrate` is for *offline host-side* migration; the runtime's SRAM-record `StateMigrator` is `gbf-runtime` territory per `planv0.md` line 2160.
- F-A6 does **not** define `BlobReferences` for any specific schema (artifact, report, etc.). `gbf-store` defines the *trait*; each blob format implements it inside its own crate.
- F-A6 does **not** depend on `tokio`, `async-std`, or any async runtime. All operations are synchronous `std::io`.
- F-A6 does **not** depend on a specific compression library. The `BlobCodec::Zstd` variant lives in `gbf-foundation`; `gbf-store` reads bytes opaquely. Compression decisions belong to whoever is *producing* the blob, not the storage layer.
- F-A6 does **not** depend on a specific hashing implementation outside of what `gbf-foundation::Hash256` already commits to. `BlobStore::put` re-hashes the input to confirm the caller's claimed hash matches; the hashing primitive is `sha2` (already in the workspace).
- F-A6 does **not** ship a CLI. `gbf-cli` may eventually provide `gbf-cli store gc`, `gbf-cli store verify`, `gbf-cli archive list <path>`; F-A6 ships only the library API.
- F-A6 does **not** make `gbf-store` `no_std`. Filesystem operations are fundamentally `std::io`-bound. The "`no_std + alloc` capable" engineering rule (rule 11) applies to `gbf-foundation`, `gbf-artifact`, `gbf-abi`, `gbf-ir`, and `gbf-asm`; `gbf-store` is explicitly outside that list.

### 0.0.5 What changed since this feature was first scoped

Three pieces of context have moved since the feature bead was filed in April 2026:

1. **`gbf-foundation::blob` is still `//! Module stub.`** The original bead description claimed `BlobRef` was "already CLOSED in T0.3"; the source of truth on `main` shows `gbf-foundation/src/blob.rs` is one line: `//! Module stub.`. F-A6 closure therefore includes T-A6.0 (a precondition step inside the same PR): populate `gbf-foundation::blob::{BlobRef, BlobCodec}` with the shape defined in `planv0.md` line 746 (`hash`/`len`/`codec` and `Raw`/`Zstd`). This is purely additive, mechanical, and unblocks `gbf-store`'s use of `BlobRef` for sidecar references. Without this step, `gbf-store` cannot return a `BlobRef` to its callers, and `gbf-codegen`/`gbf-artifact` cannot reference store-resident blobs by their canonical foundation type.
2. **The current `gbf-store/Cargo.toml` lists `gbf-artifact` as a dependency.** That dependency is wrong. The architectural intent is that *artifact* depends on *store* (artifact references blobs by `BlobRef`; the store dereferences `BlobRef` into bytes), not the other way around. F-A6 closure removes `gbf-artifact` from `gbf-store/Cargo.toml`. The `gbf-store` library only depends on `gbf-foundation`. (No production consumer depends on `gbf-store` today, so the dep-flip is invisible to the rest of the tree.)
3. **The current `gbf-migrate/Cargo.toml` lists `gbf-abi` and `gbf-artifact` as dependencies.** Those dependencies are wrong for the eventual scaffolding (the `Migrator` trait operates on opaque `&[u8]`; real migrators live in their owning crates), **but F-A6 leaves `gbf-migrate/Cargo.toml` untouched** because the entire `gbf-migrate` crate is now deferred (§0.0.0). The dep cleanup will land in the same follow-up bead (F-A6b) that fills the scaffolding.

The migration plan is detailed in §2.X "Closing the Cargo.toml stubs and populating BlobRef" — restricted to the `gbf-store` and `gbf-foundation` edits; the `gbf-migrate/Cargo.toml` diff there is preserved as a design note for F-A6b but **must not** be applied in the F-A6 PR. The closure summary (§19 claim-to-gate matrix) names the in-scope dep-flip and the BlobRef precondition explicitly.

## 0. TL;DR

### 0.1 RFC self-check before implementation

This RFC is ready to implement only if the following are true:

- `gbf-foundation::blob::BlobRef` is populated with the `planv0.md` line 746 shape (`hash: Hash256`, `len: u32`, `codec: BlobCodec`) and `BlobCodec` has the two variants `Raw` and `Zstd`. T-A6.0 is part of the F-A6 PR; closure does not require any other crate to consume it yet.
- `gbf-store/Cargo.toml` depends only on `gbf-foundation` plus `serde`/`sha2`; the existing `gbf-artifact` dep is removed.
- `gbf-migrate/Cargo.toml` is **left untouched** by F-A6. Its existing `gbf-abi`/`gbf-artifact` deps are not corrected here; they belong to the deferred F-A6b bead. F-A6 closure does not gate on this file.
- `gbf-migrate/src/{dag.rs, epochs.rs, report.rs}` are **left as `//! Module stub.`** by F-A6. F-A6 closure does not gate on `gbf-migrate` types existing.
- `BlobStore::put(&self, bytes: &[u8]) -> Result<Hash256, BlobStoreError>` hashes the input and stores the blob at `blobs/sha256/<ab>/<hash>`; `<ab>` is the first two hex characters of the hash. A separate `put_expect(expected, bytes)` is provided for callers (notably archive extraction) that have an external claimed hash and want it verified before commit.
- `BlobStore::put` is atomic against single-process crash: a torn file write never lands at the canonical path. The implementation is `tmp/<random>` → fsync(file) → rename. Durability claims are scoped to read-after-return on the same machine; full power-loss durability of the renamed directory entry requires the optional `DurabilityMode::Full` variant.
- `BlobStore::put` is content-addressed-idempotent: writing the same bytes twice returns the same `Hash256` and does not error on the second write. The early-out path verifies the existing canonical file's contents (re-hashes and compares) before treating it as a hit; a corrupt file at the correct path surfaces as `BlobStoreError::ExistingBlobCorrupt`.
- `verify_integrity(store, hash)` re-reads the blob and compares its hash against the filename; `verify_all(store)` walks the store and returns an `IntegrityReport { blobs_checked, mismatches, missing }`.
- `StageKey { stage_id, shard_local: ComponentDigestSet, global: Hash256, feature_flags: BTreeSet<FeatureFlag>, pass_version: SemVer }` exists, is `Serialize + Deserialize`, and `compose_key` is deterministic across struct construction order.
- `compose_key` uses a canonical encoding: `BTreeMap` for the digest set, sorted feature flags, fixed encoding for `Hash256` and `SemVer`. Two equivalent keys produced via different construction paths hash identically.
- `Pinset { name: PinsetName, roots: BTreeSet<Hash256>, annotation: Option<String> }` exists and is `Serialize + Deserialize`. `PinsetName` wraps an owned `String`, validated at construction (no NUL, no path separators, no `..`, non-empty).
- The typed `BlobReferences` trait exposes `fn referenced_blobs(&self) -> Vec<Hash256>` for in-memory typed values. GC operates on opaque bytes, so it consults a separate `BlobReferenceReader` recognizer trait through `BlobReferencesRegistry` rather than calling `BlobReferences` directly.
- `run_gc(store, pinsets, refs, opts) -> Result<GcReport, GcError>` walks pinsets transitively. Unknown reference-bearing formats are governed by `GcOptions::unknown_reference_policy`; the default is `Abort`, *not* `TreatAsLeaf`. Removal order is `Hash256` ascending so `max_remove_per_run` is deterministic. GC also sweeps stale stage-cache index files (`GcOptions::sweep_stage_cache_indexes`).
- `GcReport` is structured JSON with `pinsets_walked: u64`, `blobs_kept: u64`, `candidate_blobs: u64`, `candidate_bytes: u64`, `blobs_removed: u64`, `bytes_freed: u64`, `removed: Vec<Hash256>`. In dry-run mode the `candidate_*` counters describe what *would* be removed; `blobs_removed`/`bytes_freed`/`removed` remain zero/empty.
- `ArchiveHeader` has magic bytes `b"GBLM\0ARC"` (eight bytes; the `\0` is the fifth byte), a one-byte version, `pinset_count: u16`, `blob_count: u32`, `total_bytes: u64`. The header layout is byte-stable.
- `create_archive(store, pinsets, refs, out)` accepts a `BlobReferencesRegistry` so it can walk transitive references; then `extract_archive(in, store) -> ExtractedArchive` round-trips: extracted blobs are byte-identical to the originals, and the returned `ExtractedArchive { header, pinsets }` carries the pinsets back to the caller. Extraction is *not* transactional: a malformed later record may leave earlier-extracted blobs in the store.
- `list_archive(in)` reads the header, pinset table, and each blob record's `(hash, len)`; it does not write into a store. With a plain `Read`, listing is `O(total archive bytes)` because blob bodies must be read and discarded between records. A future `Read + Seek` overload may skip bodies.
- *(deferred to F-A6b — see §0.0.0)* `gbf-migrate::epochs::CompatibilityEpochs`, the `Migrator` trait, `MigrationGraph::plan`/`execute`, `MigrationLossClass`, `MigrationReport`. None of these are required for F-A6 closure.
- `gbf-store` uses `#![forbid(unsafe_code)]` at the crate root.
- `gbf-migrate`'s `#![forbid(unsafe_code)]` is **not** added by F-A6; it lands when the deferred bead populates the crate.
- F-A6 ships **no migrators** (real or fixture). Migrator scaffolding and test fixtures are deferred to F-A6b.
- Single-file archive transport is mandatory; the directory form (just walking `blobs/sha256/`) is the canonical on-disk layout and is not an "alternative" but the universal layout.

### 0.2 Summary

`gbf-store` is the workspace's only legitimate home for content-addressed blob storage and the always-on `StageCache`. (`gbf-migrate` is intended to be the workspace's only legitimate home for offline schema migration, but its scaffolding is **deferred** — see §0.0.0.) Today the in-scope modules are pinned in the workspace (`gbf-store/Cargo.toml`, `gbf-store/src/{blob,integrity,stage_cache,pinset,gc,archive}.rs`) with every file exactly `//! Module stub.`. F-A6 fills all six `gbf-store` modules in a single PR, populates the precondition `BlobRef`/`BlobCodec` in `gbf-foundation::blob`, removes the placeholder `gbf-artifact` dep from `gbf-store/Cargo.toml`, and ships table-driven tests that close the parent feature bead `bd-3ll` together with the four in-scope child task beads `bd-1jy`, `bd-2ab`, `bd-1e9`, `bd-1yd`. The fifth child bead `bd-n9i` (T-A6.5, `gbf-migrate`) is **deferred** (`br defer bd-n9i`) and removed from `bd-3ll`'s closure path.

```
gbf-foundation/src/
  blob.rs              (T-A6.0 precondition: BlobRef + BlobCodec)

gbf-store/src/
  lib.rs              ──┐
  blob.rs               │  T-A6.1 part 1: BlobStore + atomic writes + put/get/exists/remove/streaming
  integrity.rs          │  T-A6.1 part 2: verify_integrity, verify_all, IntegrityReport
  stage_cache.rs        │  T-A6.2:        StageKey, ComponentDigestSet, compose_key (canonical)
  pinset.rs             │  T-A6.3 part 1: PinsetName, Pinset, BlobReferences trait
  gc.rs                 │  T-A6.3 part 2: run_gc, GcOptions, GcReport (transitive walk)
  archive.rs          ──┘  T-A6.4:        ArchiveHeader (b"GBLM\0ARC"), create/extract/list

gbf-migrate/src/        ── DEFERRED to F-A6b (see §0.0.0); files stay `//! Module stub.`
  lib.rs                   not modified
  epochs.rs                not modified
  dag.rs                   not modified
  report.rs                not modified
```

The new in-scope modules add roughly 1,100 LOC of production code plus about 1.3 KLOC of table-driven tests, atomic-write crash tests, archive round-trip tests, and GC dry-run/limit tests. There is no example binary — `gbf-store` is a library consumed by the rest of the workspace once F-B15 wires the StageCache into compiler stages.

The seven most load-bearing decisions in this RFC are:

1. **The hash is the path; nothing else.** A blob lives at `blobs/sha256/<ab>/<hash>` and there is no parallel index, no manifest, no metadata file. Lookups are filesystem operations against a deterministic path. The `<ab>` first-two-hex split keeps directory entries bounded at ~256 sub-dirs even with millions of blobs.
2. **Atomic writes are non-negotiable, with explicit durability scope.** Every blob write is `tmp/<random>` → fsync(file) → rename-onto-canonical-path. A crash during write leaves at most a stale tmp file (cleaned up by an explicit `cleanup_tmp` maintenance call, *not* by `open`, because another process may be mid-write). The canonical path either holds the complete blob or it does not exist. Read-after-return on the same machine is guaranteed; full power-loss durability of the directory entry requires `DurabilityMode::Full` (which best-effort-fsyncs the destination directory after rename on platforms that support it).
3. **`StageKey` is one canonical key with two components, not two physical caches.** `planv0.md` line 2940 (engineering rule 20) is explicit: "two levels: shard-local keys derived from named component digests where legality is local, and whole-build keys where legality is global." A flat single-input key over-invalidates; a purely shard-local key produces false hits when a global invariant changes. F-A6 implements one physical cache whose canonical key combines both components via `compose_key`, surfaced as the `StageCacheKey` newtype so it cannot be confused with a content hash.
4. **Pinsets are the GC root set; transitive walk uses a recognizer registry, not on-blob trait calls.** GC operates on opaque bytes, so the keep-set walk consults `BlobReferencesRegistry` — a list of `BlobReferenceReader`s, each of which inspects bytes and either returns `Some(refs)` (recognized) or `None` (skipped). The default `UnknownReferencePolicy::Abort` refuses to GC if any pinned reference-bearing blob cannot be decoded; callers must opt into `TreatAsLeaf` for formats known to have no outgoing references or for cache-like data where losing descendants is acceptable. The typed `BlobReferences` trait remains for in-memory consumers that already hold the deserialized value.
5. **`BlobCodec` is observed by the store but not chosen by it.** `BlobCodec::Raw` and `BlobCodec::Zstd` are encoded into the `BlobRef` produced by the *blob author*; `BlobStore::put` stores bytes opaquely. Decompression is the consumer's job, not the store's. This keeps `gbf-store` independent of compression library choices.
6. **`[F-A6b design note — DEFERRED, see §0.0.0]` Migrators take opaque bytes.** The `Migrator` trait is `fn migrate(&self, input: &[u8]) -> Result<MigrateResult, MigrateError>`. The scaffolding does not depend on any specific schema; real migrators (artifact, ABI, calibration, report) implement the trait in their owning crates and register with a `MigrationGraph` instance built per-consumer. This is what will justify removing `gbf-abi` and `gbf-artifact` from `gbf-migrate/Cargo.toml` when F-A6b lands.
7. **`[F-A6b design note — DEFERRED, see §0.0.0]` `gbf-migrate` ships `Lossless` / `LossyButAccepted` / `Rejected` as a closed enum.** Lossy migrations are not silent — they are typed. Policy resolution downstream may choose to refuse `LossyButAccepted` migrations under strict profiles; refusal is a typed `Rejected` outcome, not an `Option<>` or a `Result<_, &str>`.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A `gbf-foundation::blob` module that exports `BlobRef { hash: Hash256, len: u32, codec: BlobCodec }` and `BlobCodec { Raw, Zstd }`, both `Copy + Clone + Debug + Eq + PartialEq + Hash + Serialize + Deserialize`. Public re-exports in `gbf-foundation/src/lib.rs`. `gbf-foundation::Hash256` gains explicit `from_bytes([u8; 32])` and `as_bytes(&self) -> &[u8; 32]` accessors (today's stub already has these; F-A6 documents them as a precondition).
- A `gbf-store::blob` module that exports `BlobStore`, `BlobStore::open`, `put`, `put_expect`, `put_streaming`, `put_as`, `get`, `get_streaming`, `get_ref`, `exists`, `remove`, `list_blobs`, `path_for`, `cleanup_tmp`, plus `DurabilityMode { ReadAfterReturn, Full }`. `put` is atomic, content-addressed-idempotent (with verification of any pre-existing canonical file), and hashes its input. Streaming variants exist for blobs above a configurable inline limit. `put_expect(expected, bytes)` is the verifying form used by archive extraction.
- A `gbf-store::integrity` module that exports `verify_integrity(store, hash)`, `verify_all(store) -> Result<IntegrityReport, IntegrityError>`, `verify_reachable(store, roots, refs)`, and an `IntegrityReport { blobs_checked: u64, mismatches: Vec<Hash256>, missing: Vec<Hash256> }`.
- A `gbf-store::stage_cache` module that exports `StageCache`, `StageKey { stage_id, shard_local, global, feature_flags, pass_version }`, `ComponentDigestSet`, `StageId`, `ComponentId`, `FeatureFlag` (the three IDs are defined here, not in `gbf-foundation`), the `StageCacheKey` newtype, `StageCacheEntry { key, payload_hash }`, and `compose_key`. The cache is one physical cache layered over `BlobStore`, keyed by `compose_key(StageKey) -> StageCacheKey`; values are arbitrary bytes (the consumer decides their internal format).
- A `gbf-store::pinset` module that exports `PinsetName(String)` (validated at construction), `Pinset { name, roots, annotation }`, and the typed `BlobReferences` trait with one method `fn referenced_blobs(&self) -> Vec<Hash256>` for in-memory consumers.
- A `gbf-store::gc` module that exports `run_gc(store, pinsets, refs, opts) -> Result<GcReport, GcError>`, `GcOptions { dry_run, max_remove_per_run, sweep_stage_cache_indexes, unknown_reference_policy }`, `UnknownReferencePolicy { Abort, TreatAsLeaf }`, `GcReport`, `BlobReferencesRegistry`, `BlobReferenceReader`, `BlobReferenceError`. Removal order is `Hash256` ascending. The default `UnknownReferencePolicy` is `Abort`.
- A `gbf-store::archive` module that exports `ArchiveHeader { magic: [u8; 8], version: u8, pinset_count: u16, blob_count: u32, total_bytes: u64 }`, `create_archive(store, pinsets, refs, out)`, `extract_archive(in, store) -> ExtractedArchive`, `list_archive(in) -> ArchiveContents`, `ExtractedArchive { header, pinsets }`, `ARCHIVE_MAGIC: [u8; 8] = *b"GBLM\0ARC"`, `ARCHIVE_VERSION: u8 = 1`. Archive bytes are deterministic for the same inputs (pinsets are sorted by `PinsetName` inside `create_archive`; blobs by `Hash256`).
- *(Deferred to F-A6b — see §0.0.0)* Goals previously listed for `gbf-migrate::epochs`, `gbf-migrate::dag`, and `gbf-migrate::report` (the `CompatibilityEpochs` shape, the `Migrator` trait, `MigrationGraph`, `MigrationPlan`, `MigrationExecution`, `MigrationLossClass`, `MigrationReport`, `FieldPath`, `MigrationWarning`). The full design is preserved in §§ 11–13 as a starting point for the deferred bead but is **not implemented** by F-A6.
- A test matrix that proves: round-trip put/get; atomic-write crash safety via simulated crash; content-addressed idempotence; integrity check detects manual corruption; deterministic `StageKey` hashing across construction order; shard-local invalidation does not invalidate the global cache; GC pinset protection; GC transitive walk via `BlobReferences`; GC dry-run removes nothing; GC `max_remove_per_run` honored; archive round-trip; archive listing without extraction. (Migration-DAG / loss-class / migrator tests are deferred to F-A6b.)
- A `cargo test -p gbf-store` matrix that, by itself, proves every in-scope claim in §19. It runs in the workspace pre-commit hook. (`cargo test -p gbf-migrate` is not part of F-A6 closure.)

### 1.2 Non-goals (deferred)

- **The entire `gbf-migrate` scaffolding.** Epochs, the `Migrator` trait, `MigrationGraph::plan`/`execute`, `MigrationLossClass`, `MigrationReport`, identity / bump-minor migrator fixtures, and the `gbf-migrate/Cargo.toml` cleanup are all deferred to a follow-up feature bead (provisional **F-A6b**). See §0.0.0 for motivation. The first real schema bump opens that bead; until then, `gbf-migrate/src/{dag,epochs,report}.rs` stay `//! Module stub.` and the workspace policy is "schema mismatch ⇒ rebuild from sources."
- **`StageCache` integration into compiler stages.** Owned by F-B15 (`bd-1g7k`). F-A6 ships only the library; the per-stage call sites (`QuantGraph` cache, `StoragePlan` cache, `RomWindowPlan` cache, etc.) are wired in their owning passes.
- **Real schema migrators.** Deferred (see above).
- **The runtime `StateMigrator`.** Per `planv0.md` line 2160, the runtime SRAM-record `StateMigrator` is `gbf-runtime::persistence` territory, not `gbf-migrate`. F-A6 explicitly does not touch runtime persistence; the two migrator concepts are deliberately distinct (host-side schema vs on-device versioned record).
- **Compression.** `BlobCodec::Zstd` is enumerated; `gbf-store` does not perform compression. The author of a blob writes the `BlobCodec` it chose into the `BlobRef`; the consumer decompresses on read. F-A6 ships no compression dependency.
- **Async I/O.** All `BlobStore`/`StageCache`/`run_gc`/`create_archive`/`extract_archive` operations are synchronous `std::io`. Async wrappers are a follow-up bead if and when a consumer needs them.
- **Multi-process safe coordination.** `BlobStore::put` is atomic against single-process crash; concurrent writes from two processes are safe-but-redundant (each writes its own tmp, both rename onto the same canonical path; the second rename overwrites the first byte-for-byte because the bytes are identical). F-A6 does not ship a lockfile or coordination protocol; concurrent GC from two processes is undefined behavior at the protocol level (consumers must serialize GC).
- **`StageId` enumeration.** F-A6 defines `StageId(string_id)` as an opaque newtype inside `gbf-store::stage_cache`. Concrete stage variants live in their owning crate (`gbf-asm::PlanningStage`, `gbf-codegen` stage variants). Mapping a domain stage enum to `StageId` is a one-line conversion in the consumer.
- **`FeatureFlag` enumeration.** Same shape: `FeatureFlag(string_id)` is an opaque newtype; concrete flag values live in `gbf-policy` or wherever the flag originates.
- **Workspace-wide cache eviction policy.** `gbf-store::gc` ships `run_gc` with a pinset root set; choosing *what to pin* is policy that lives in `gbf-cli`, `gbf-codegen`, or `gbf-bench`. F-A6 does not decide retention policy.
- **CLI front-ends.** `gbf-cli store gc`, `gbf-cli store verify`, `gbf-cli archive list` are all reasonable future tools; they live in `gbf-cli`.
- **`no_std + alloc`.** Filesystem ops are `std::io`-bound. F-A6 does not declare `#![no_std]` on either crate.

## 2. Background and existing state

### 2.1 What is already in tree

`gbf-store` and `gbf-migrate` exist as scaffolds; `gbf-foundation::blob` exists as a stub.

**`gbf-foundation` (mostly stable, blob is stubbed):**

- `gbf-foundation/src/blob.rs` — exactly `//! Module stub.`. The crate `lib.rs` declares `pub mod blob;` but does not re-export anything from it.
- `gbf-foundation/src/hash.rs` — populated with `Hash256` (32-byte hex newtype, parse + display), `Hash256ParseError` (invalid length / invalid hex). `Hash256::ZERO` exists as a sentinel; `Hash256::from_bytes([u8; 32]) -> Hash256` and `Hash256::as_bytes(&self) -> &[u8; 32]` are public accessors used pervasively by F-A6 (these are already present in `main`; the F-A6 RFC documents them as a precondition for the pseudocode in §5–§10).
- `gbf-foundation/src/semver.rs` — populated with `SemVer { major, minor, patch }`, `SemVerParseError`.
- `gbf-foundation/src/ids.rs` — populated with `string_id!` and `numeric_id!` macros plus existing IDs (`TargetProfileId`, `CompileProfileId`, `TargetFamilyId`, `CheckpointId`, `WorkloadId`, `CalibrationSetRef` (string-id placeholder, will be replaced by F-A2's struct), `LayerId`, `ExpertId`, `BudgetSlotId`).
- `gbf-foundation/src/cost.rs` — populated with `ByteCost`.
- `gbf-foundation/src/lib.rs` — re-exports `BudgetSlotId`, `CalibrationSetRef`, `CheckpointId`, `CompileProfileId`, `ExpertId`, `LayerId`, `TargetFamilyId`, `TargetProfileId`, `WorkloadId`, `Hash256`, `Hash256ParseError`, `SemVer`, `SemVerParseError`, `ByteCost`. **Does not re-export anything from `blob`** because `blob.rs` is empty.

**`gbf-store` (this crate, mostly stubbed):**

- `gbf-store/Cargo.toml` — pinned, `publish = false`. Currently lists `gbf-artifact = { path = "../gbf-artifact" }` in addition to `gbf-foundation`, `serde`, `serde_json`. F-A6 closure removes the `gbf-artifact` dependency and adds `sha2` (for `Hash256` re-hashing) and `tempfile` (for atomic-write tmp paths) under `[dependencies]`; moves `serde_json` to `[dev-dependencies]`.
- `gbf-store/src/lib.rs` — declares the six modules and contains a single doc comment ("Content-addressed storage, stage cache, archive transport, pinsets, GC, and integrity verification."). F-A6 closure adds `#![forbid(unsafe_code)]` at the crate root.
- `gbf-store/src/{blob, integrity, stage_cache, pinset, gc, archive}.rs` — every file is exactly `//! Module stub.`.

**`gbf-migrate` (this crate, **left as-is — DEFERRED to F-A6b, see §0.0.0**):**

- `gbf-migrate/Cargo.toml` — pinned, `publish = false`. Currently lists `gbf-abi = { path = "../gbf-abi" }` and `gbf-artifact = { path = "../gbf-artifact" }` in addition to `gbf-foundation`, `serde`, `serde_json`. **F-A6 does not modify this file.** The wrong-direction `gbf-abi`/`gbf-artifact` deps stay in place until the deferred F-A6b bead picks up scaffolding work.
- `gbf-migrate/src/lib.rs` — declares the three modules and contains a single doc comment. **F-A6 does not modify this file.** `#![forbid(unsafe_code)]` is added by F-A6b along with the rest of the crate's contents.
- `gbf-migrate/src/{dag, epochs, report}.rs` — every file is exactly `//! Module stub.`. **F-A6 leaves them as stubs.**

**No production consumer.** Neither crate is depended on by `gbf-asm`, `gbf-hw`, `gbf-codegen`, `gbf-runtime`, `gbf-policy`, `gbf-report`, `gbf-bench`, or any other crate. F-A6 is therefore additive at the API surface: nothing breaks, because nothing currently uses these crates.

### 2.2 What is stubbed

All six `gbf-store` modules. All three `gbf-migrate` modules. `gbf-foundation::blob` (one line, empty). There is no test file under `gbf-store/tests/` or `gbf-migrate/tests/`. **F-A6 fills the six `gbf-store` stubs and `gbf-foundation::blob`, adds `gbf-store/tests/` with one integration test per module plus a cross-module conformance test, and updates `gbf-store/Cargo.toml`. The three `gbf-migrate` stubs and `gbf-migrate/Cargo.toml` are left untouched** (deferred to F-A6b — see §0.0.0).

### 2.2.1 Migration plan: closing the Cargo.toml stubs and populating BlobRef

The F-A6 single-PR closure includes two small Cargo-level changes plus the BlobRef populate. (A third diff for `gbf-migrate/Cargo.toml` is preserved below as a design note for the deferred F-A6b bead — **do not apply it in the F-A6 PR**.)

```diff
# gbf-foundation/src/blob.rs (was: "//! Module stub.")
+ //! `BlobRef` and `BlobCodec` — the foundation types that thread through every artifact, sidecar, and stage cache entry.
+ //!
+ //! `BlobRef` is a (`hash`, `len`, `codec`) handle that points into a content-addressed blob store
+ //! (typically `gbf-store::BlobStore`). Per `planv0.md` line 137, the type lives in `gbf-foundation`
+ //! so every contract crate (artifact, hw, abi, ir, asm) can reference blobs without depending on
+ //! the storage crate.
+
+ use serde::{Deserialize, Serialize};
+
+ use crate::Hash256;
+
+ #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
+ pub struct BlobRef {
+     pub hash: Hash256,
+     pub len: u32,
+     pub codec: BlobCodec,
+ }
+
+ #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
+ #[serde(rename_all = "snake_case")]
+ pub enum BlobCodec {
+     Raw,
+     Zstd,
+ }
```

```diff
# gbf-foundation/src/lib.rs
+ pub use blob::{BlobCodec, BlobRef};
```

```diff
# gbf-store/Cargo.toml
  [dependencies]
- gbf-artifact = { path = "../gbf-artifact" }
  gbf-foundation = { path = "../gbf-foundation" }
  serde.workspace = true
- serde_json.workspace = true
+ sha2.workspace = true
+ tempfile.workspace = true
+
+ [dev-dependencies]
+ serde_json.workspace = true
```

`tempfile` is a production dependency because `BlobStore::put` writes to `tmp/<random>` in production code paths. `sha2` is a production dependency because every `put` re-hashes its input. Both deps are pinned in `[workspace.dependencies]` (added if not already there) so version drift is centralized.

**DEFERRED — design note for F-A6b, NOT applied by F-A6:**

```diff
# gbf-migrate/Cargo.toml  (NOT TOUCHED BY F-A6 — applied by deferred F-A6b bead)
  [dependencies]
- gbf-abi = { path = "../gbf-abi" }
- gbf-artifact = { path = "../gbf-artifact" }
  gbf-foundation = { path = "../gbf-foundation" }
  serde.workspace = true
- serde_json.workspace = true
+
+ [dev-dependencies]
+ serde_json.workspace = true
```

The three in-scope diffs (foundation `blob.rs`, foundation `lib.rs`, `gbf-store/Cargo.toml`) are the only Cargo/foundation edits in the F-A6 PR. The reverse migration — anyone in the workspace wanting to consume `BlobRef` — is now possible (it was impossible before because `gbf-foundation::blob` was empty). No existing crate is broken because none use these types yet.

### 2.3 Downstream pressure on this design

```
gbf-store  ──▶ gbf-codegen      (Stage cache for every transformative pass; F-B15)
           ──▶ gbf-bench        (Calibration-bundle production stores measurement blobs;
                                 archive transport ships bundles between machines)
           ──▶ gbf-cli          (Future: `gbf-cli store gc`, `gbf-cli archive list`)
           ──▶ gbf-report       (Build reports written through blob store; F-F1)
           ──▶ gbf-artifact     (Artifact lineage references store-resident blobs by BlobRef)
           ──▶ gbf-train        (Shadow compile artifacts; checkpoint frontiers)

gbf-migrate (DEFERRED — see §0.0.0; consumers below are blocked on F-A6b)
            ──▶ gbf-artifact    (artifact 1.x → 1.y migrators register here)
            ──▶ gbf-abi          (ABI bumps register here)
            ──▶ gbf-policy       (Calibration-schema bumps register here, indirectly)
            ──▶ gbf-report       (Report-schema bumps register here)
            ──▶ gbf-cli          (Future: `gbf-cli artifact migrate <from> <to>`)
```

Every consumer of `gbf-store` assumes:

- The blob store is content-addressed-deterministic. `hash(write(b)) == hash(b)` for all `b`.
- The store's directory layout is stable: `blobs/sha256/<ab>/<hash>` is the canonical location.
- `StageKey`s with structurally equivalent contents produce identical hashes regardless of construction order.
- `Pinset`s survive process restart (they are persisted JSON; the format is part of the contract).
- An archive built on machine A and extracted on machine B produces a byte-identical set of blobs.

**Until F-A6b lands**, the workspace policy for schema mismatch is "rebuild from sources" — there is no `MigrationReport`, no `MigrationLossClass` enforcement, and no migrator chain. Loaders that today assert "current schema in memory" must error loudly on a version mismatch rather than silently transforming.

### 2.4 Engineering-rule grounding (`planv0.md` §"Engineering rules")

This RFC threads the rules tightly:

- **Rule 5** (deterministic, hashed builds). The blob store's content-addressed property is what makes "deterministic, hashed" mechanically checkable. Two compiles of identical inputs must produce identical blob sets, which means identical `Hash256`s, which means identical `blobs/sha256/...` paths. The integrity-verify pass converts that property into an audit.
- **Rule 5** (deterministic, hashed builds). The blob store's content-addressed property is what makes "deterministic, hashed" mechanically checkable. Two compiles of identical inputs must produce identical blob sets, which means identical `Hash256`s, which means identical `blobs/sha256/...` paths.
- **Rule 11** (`no_std + alloc` capable where practical). Rule 11 lists `gbf-hw`, `gbf-artifact`, `gbf-abi`, `gbf-ir`, and `gbf-asm` — *not* `gbf-store` or `gbf-migrate`. F-A6 takes the rule literally: the `BlobRef` populate in `gbf-foundation::blob` stays `no_std`-shaped (no `std::io`); `gbf-store` and `gbf-migrate` use `std::io` freely.
- **Rule 12** (`unsafe` is forbidden by default). Both crates use `#![forbid(unsafe_code)]` at the crate root.
- **Rule 20** (always-on `StageCache`, two-component canonical key). F-A6 implements exactly this rule's required shape: `StageKey` with shard-local digest set + global digest, `compose_key` deterministic, `--resume-from <stage>` is *not* in scope (it's a CLI control layered on top, not a storage primitive).
- **Rule 24** (deterministic builds need pinned toolchain + lockfile + host triple). The `ReproducibilityManifest` is owned elsewhere; F-A6 supplies the storage primitive that makes manifest contents hash-stable.
- **Single source of truth** (a constitutional invariant rather than a numbered engineering rule). The "one place for content-addressed storage; one place for offline schema migration" property is the entire point of these two crates.

### 2.5 Constitutional grounding (`CONSTITUTION.md`)

- **§I.1 (correctness by construction).** `BlobRef` and `Pinset` are typed value objects, not bag-of-strings. `MigrationLossClass` is a closed enum, not a `&str`. `StageKey` decomposes into typed sub-fields, not a flat hash blob. `ComponentDigestSet` uses `BTreeMap<ComponentId, Hash256>` so the canonical order is structural.
- **§III (shift left).** The atomic-write contract is enforced by `tempfile`'s rename semantics and tested by a deliberately-killed write that confirms no half-blob lands. `StageKey` determinism is tested by constructing the same logical key in two different orders and asserting hash equality.
- **§IV.3 (reproducible builds).** Content-addressed storage is precisely what Rule 5 requires. Tests prove byte-identity on repeat writes and on archive round-trip.
- **§V.3 (silence on success, loud on failure).** Every error type is a typed `enum` with reproducible state. `IntegrityError`, `MigrateError`, `MigrationGraphError`, `GcError` (only emitted for IO failures; the GC itself never silently drops blobs), `ArchiveError` all carry enough state to retry or diagnose.
- **§VI.1 (single source of truth).** Two crates, two responsibilities. No other crate is allowed to write its own blob store or its own migration DAG.

### 2.6 Filesystem semantics as the primary specification

Where Pan Docs is the spec for `gbf-hw`, POSIX filesystem semantics are the spec for `gbf-store`. The load-bearing properties:

1. **`rename(src, dst)` is atomic when src and dst are on the same filesystem.** This is what makes `tmp/<random> → fsync → rename` safe. F-A6 enforces same-filesystem by placing `tmp/` inside the store root, not under `/tmp` or `$TMPDIR`.
2. **`fsync(fd)` flushes a file's contents to durable storage before returning.** F-A6 calls `fsync` after writing the blob's bytes and before renaming, so a power-loss between rename and consumer read produces consistent contents. F-A6 does *not* `fsync` the directory after rename, on the theory that consumer read-after-write on the same machine is the only ordering F-A6 guarantees; durable cross-process visibility is the consumer's responsibility (the ROM build is not in this crate).
3. **Filesystems vary on case sensitivity.** F-A6 uses lowercase hex for paths so the canonical path is unambiguous on case-insensitive filesystems (macOS APFS by default, Windows NTFS).
4. **Directory entries scale poorly past ~10K entries on most filesystems.** The two-character prefix split (`<ab>`) creates 256 directories containing roughly `total_blobs / 256` files each. A future two-level split (`<ab>/<cd>`) would create 65,536 leaves if the store ever needs that fan-out.

Where filesystem semantics and the workspace's pre-commit hook conflict (e.g., the hook runs `cargo test --workspace --all-features`, and `gbf-store`'s atomic-write test uses `tempfile`), the hook is authoritative. F-A6's tests honor the workspace test runner exactly.

### 2.7 Relationship to other M0 features

```
                        ┌─────────────────────────────────────────────┐
                        │  F-A6: gbf-store                            │   ← this RFC
                        │  (CAS, StageCache, archive)                 │
                        └────────────┬────────────────────────────────┘
                                     │
        ┌────────────────────────────┼────────────────────────────────┐
        ▼                            ▼                                ▼
┌──────────────────┐       ┌──────────────────┐           ┌──────────────────────────────┐
│  F-B15:          │       │  F-F1: build     │           │  F-A6b (DEFERRED):           │
│  StageCache      │       │  reports + emit  │           │  gbf-migrate scaffolding     │
│  integration     │       │  hooks           │           │  — opens when first real     │
│  across all      │       │  (writes through │           │    schema bump demands it.   │
│  Epic B stages   │       │  blob store)     │           │  Inherits §§ 11–13 of this   │
└──────────────────┘       └──────────────────┘           │  RFC as starting design.     │
                                     │                    └──────────────────────────────┘
                                     ▼
                        ┌──────────────────────────────┐
                        │  F-E2 / F-E3:                │
                        │  PlatformCalibrationBundle / │
                        │  KernelCalibrationBundle     │
                        │  production via gbf-bench    │
                        │  (archive transport ships    │
                        │   bundles between machines)  │
                        └──────────────────────────────┘
```

F-A6 is a **leaf** dependency in the M0 graph — nothing it depends on is changing during M0. It blocks F-B15 (StageCache integration) and F-F1 (build reports). Future schema-bump beads are blocked on F-A6b (deferred), not on F-A6. Closing F-A6 unlocks F-B15 and F-F1.

### 2.8 Beads under this feature

The four in-scope child tasks under `bd-3ll` are:

| Bead     | Task     | Module(s)                                      | Priority | Status                           |
|----------|----------|------------------------------------------------|----------|----------------------------------|
| `bd-1jy` | T-A6.1   | `gbf-store::blob`, `gbf-store::integrity`      | P1       | open (in scope)                  |
| `bd-2ab` | T-A6.2   | `gbf-store::stage_cache`                       | P1       | open (in scope)                  |
| `bd-1e9` | T-A6.3   | `gbf-store::pinset`, `gbf-store::gc`           | P2       | open (in scope)                  |
| `bd-1yd` | T-A6.4   | `gbf-store::archive`                           | P2       | open (in scope)                  |
| `bd-n9i` | T-A6.5   | `gbf-migrate::epochs`, `dag`, `report`         | P2       | **DEFERRED** (`br defer bd-n9i`) |

T-A6.0 is a precondition step that has no existing bead; the F-A6 single-PR closure creates a bead for it (or folds it into `bd-1jy` as a sub-step) and closes it together with T-A6.1..T-A6.4.

T-A6.0 (foundation `BlobRef`) is independent of all `gbf-store` tasks but is a precondition for `gbf-store::blob` because `BlobStore::put` returns a `Hash256` and the consumer typically wants a `BlobRef` (which adds `len` and `codec`). T-A6.1 blocks T-A6.2 (StageCache stores values via BlobStore) and T-A6.3 (GC walks BlobStore) and T-A6.4 (archive reads from / writes to BlobStore). T-A6.5 is **deferred** (see §0.0.0); its work is moved to a future feature bead (provisional **F-A6b**) that opens when the first real schema bump demands it. The closure shape for F-A6 is one PR closing T-A6.0..T-A6.4 and the parent feature bead `bd-3ll` (re-scoped to `gbf-store` only).

## 3. Architecture

### 3.1 Crate-level shape: one crate in scope, one deferred

`gbf-store` is **not** a contract crate. It is *infrastructure* — it ships code that does work (atomic writes, hash verification) rather than data types other crates carry. The in-scope decomposition is:

```rust
// gbf-store: filesystem-shaped infrastructure (IN SCOPE for F-A6)
pub mod blob;          // BlobStore, atomic put/get, content-addressed paths
pub mod integrity;     // verify_integrity, verify_all, IntegrityReport
pub mod stage_cache;   // StageKey, ComponentDigestSet, compose_key, StageCache
pub mod pinset;        // PinsetName, Pinset, BlobReferences trait
pub mod gc;            // run_gc, GcOptions, GcReport (transitive walk)
pub mod archive;       // ArchiveHeader, create_archive, extract_archive, list_archive

// gbf-migrate: schema-evolution infrastructure (DEFERRED — see §0.0.0)
// pub mod epochs;     // CompatibilityEpochs, CURRENT_EPOCHS                     ← stays //! Module stub.
// pub mod dag;        // Migrator trait, MigrationGraph, MigrationPlan          ← stays //! Module stub.
// pub mod report;     // MigrationReport, MigrationLossClass, FieldPath, ...    ← stays //! Module stub.
```

`gbf-store` uses `std::io` and `std::fs`. It does not use `unsafe`. It does not spawn threads, open sockets, or run background work; every operation is synchronous and returns when complete.

### 3.2 Module responsibility table

| Module                       | Owns                                                                                  | Public surface |
|------------------------------|---------------------------------------------------------------------------------------|----------------|
| `gbf-foundation::blob`       | `BlobRef`, `BlobCodec`                                                                | ~3 items       |
| `gbf-store::blob`            | `BlobStore`, atomic put/get/exists/remove, streaming variants, `BlobStoreError`       | ~12 items      |
| `gbf-store::integrity`       | `verify_integrity`, `verify_all`, `IntegrityReport`, `IntegrityError`                 | ~5 items       |
| `gbf-store::stage_cache`     | `StageCache`, `StageKey`, `ComponentDigestSet`, `ComponentId`, `FeatureFlag`, `compose_key`, `StageId`, `StageCacheError` | ~14 items |
| `gbf-store::pinset`          | `PinsetName`, `Pinset`, `BlobReferences` trait, `PinsetError`                         | ~6 items       |
| `gbf-store::gc`              | `run_gc`, `GcOptions`, `GcReport`, `GcError`, `BlobReferencesRegistry`                | ~7 items       |
| `gbf-store::archive`         | `ArchiveHeader`, `create_archive`, `extract_archive`, `list_archive`, `ArchiveContents`, `ArchiveError` | ~10 items |
| ~~`gbf-migrate::epochs`~~    | DEFERRED to F-A6b (see §0.0.0)                                                        | 0 items in F-A6 |
| ~~`gbf-migrate::dag`~~       | DEFERRED to F-A6b (see §0.0.0)                                                        | 0 items in F-A6 |
| ~~`gbf-migrate::report`~~    | DEFERRED to F-A6b (see §0.0.0)                                                        | 0 items in F-A6 |

Total in-scope public surface: ~58 items (the three deferred `gbf-migrate` modules are not counted).

### 3.3 The dependency graph

```
gbf-foundation
   ▲
   │  (Hash256, SemVer, BlobRef, BlobCodec, string_id! / numeric_id!)
   │
   ├────────────────┬──────────────────────────┐
   │                │                          │
gbf-store      gbf-migrate (stub,         everyone else
                deferred to F-A6b)        (artifact, abi, hw, ...)
```

`gbf-store` depends on `gbf-foundation` only (plus `serde` and `sha2`).
`gbf-migrate` is **not** populated by F-A6; its current `Cargo.toml` lists wrong-direction `gbf-abi`/`gbf-artifact` deps that F-A6b will clean up.
A future consumer that needs both `gbf-store` and `gbf-migrate` (e.g., `gbf-codegen` wiring a stage cache that operates over migrated artifacts) will import both directly once F-A6b lands.

### 3.4 The `BlobRef` hand-off with `gbf-foundation`

`BlobRef` lives in `gbf-foundation::blob` per `planv0.md` line 137, *not* in `gbf-store`. This is structural — it is what lets `gbf-artifact`, `gbf-policy`, `gbf-report`, and every other contract crate carry `BlobRef`-typed fields without depending on `gbf-store`. A blob's *identity* (hash + len + codec) is foundation-typed; only its *resolution* into bytes is store-typed.

The handshake at the API boundary:

```rust
// in a consumer (gbf-artifact, etc.):
pub struct ArtifactCore {
    pub canonical_payload: BlobRef,      // foundation type
    // ...
}

// in gbf-store::blob:
impl BlobStore {
    pub fn put(&self, bytes: &[u8]) -> io::Result<Hash256> { ... }

    /// Convenience: store and return a BlobRef tagged with codec.
    pub fn put_as(&self, bytes: &[u8], codec: BlobCodec) -> io::Result<BlobRef> {
        let hash = self.put(bytes)?;
        Ok(BlobRef { hash, len: bytes.len() as u32, codec })
    }

    pub fn get_ref(&self, r: BlobRef) -> io::Result<Vec<u8>> {
        // Optionally verifies r.len matches the stored blob's actual length.
        let bytes = self.get(r.hash)?;
        if bytes.len() as u32 != r.len {
            return Err(BlobStoreError::LenMismatch { expected: r.len, actual: bytes.len() as u32 }.into());
        }
        Ok(bytes)
    }
}
```

`BlobStore` does not interpret `codec`. It stores bytes opaquely. The consumer is responsible for decompression. This keeps `gbf-store` independent of any specific compression library.

### 3.5 Why these are two crates, not one

A naive design would fold `gbf-migrate` into `gbf-store` (both are "infrastructure"; both are foundation-only deps; both ship synchronous APIs). Two reasons not to (these still hold for the F-A6b bead that will eventually populate `gbf-migrate`):

1. **Different change cadences.** `gbf-store`'s API is approximately frozen once T-A6.4 lands — atomic writes don't drift. `gbf-migrate`'s `MigrationGraph` will gain new migrator registrations on every schema bump, which is steady-state churn. Keeping them separate means a churning crate doesn't force re-builds of every consumer of the stable crate.
2. **Different consumer sets.** `gbf-store` is consumed by anything that reads/writes blobs (codegen, bench, cli, report). `gbf-migrate` is consumed by anything that loads versioned schemas (artifact loaders, abi loaders, calibration loaders). The intersection is small: `gbf-cli` is the only crate that obviously wants both. Splitting keeps each consumer's transitive surface tighter.

(F-A6 keeps `gbf-migrate` as a pinned-but-stubbed crate so the workspace topology is stable for the deferred bead. Removing the crate now and reintroducing it later would churn the workspace `Cargo.toml`.)

### 3.6 What `gbf-store` deliberately does not own (and what `gbf-migrate` will not own once F-A6b lands)

- **`StageId` discriminants.** `string_id!(StageId)` lives in `gbf-store::stage_cache`; concrete values like `"asm.layout"` or `"codegen.range_plan"` live in their owning crate.
- **`FeatureFlag` discriminants.** `string_id!(FeatureFlag)` lives in `gbf-store::stage_cache`; concrete flag names live in `gbf-policy` or wherever the flag is defined.
- **`ComponentId` semantics.** `ComponentId` is a `string_id` defined in `gbf-store::stage_cache`; what counts as a "component" (a kernel? an expert? a layer?) is consumer-decided.
- **Compression.** `BlobStore` reads/writes opaque bytes. `BlobCodec::Zstd` is a hint to consumers, not a compression directive.
- **Garbage-collection policy.** `run_gc` accepts pinsets; choosing what to pin is policy. F-A6 ships no built-in pinning policy.
- *(F-A6b non-ownership)* **Schema definitions.** When `gbf-migrate` is eventually populated, it will migrate bytes between two versioned shapes; the shapes themselves live in their owning crate (`gbf-artifact`, `gbf-abi`, etc.).
- *(F-A6b non-ownership)* **Real migrators.** Identity + bump-minor will be *test fixtures* in F-A6b; the first real migrator is owned by the first schema bump.
- **Runtime SRAM `StateMigrator`.** `gbf-runtime::persistence` owns it. `gbf-migrate` (when populated) is offline-only.
- **Concurrent multi-process coordination.** Single-process atomicity is honored; cross-process locking is the consumer's responsibility.
- **CLI front-ends.** `gbf-cli` may layer commands on top in a follow-up bead.

The boundary between `gbf-store` and these concerns is enforced by the dependency graph: `gbf-store` depends only on `gbf-foundation`. There is no path back the other way. (The same property will hold for `gbf-migrate` once F-A6b lands.)

## 4. Foundation `BlobRef` populate (T-A6.0, `gbf-foundation::blob`)

**Reference**: `planv0.md` line 137 (`gbf-foundation` owns `BlobRef`); line 746 (the canonical `BlobRef` shape).

### 4.1 Why this populate is a precondition

`gbf-store::BlobStore` returns `Hash256` from `put` and accepts `Hash256` in `get`. But every contract-crate consumer (`gbf-artifact`, `gbf-policy`, etc.) carries fields typed as `BlobRef`, not `Hash256`, because a `BlobRef` also names the length and codec the producer chose. Without `BlobRef` in `gbf-foundation`, `gbf-store` can't expose a `put_as(bytes, codec) -> BlobRef` convenience and consumers have to mint their own (`hash, len, codec`) tuple at every call site. That destroys the "single source of truth" property at the foundation-type level.

### 4.2 The types

```rust
//! `BlobRef` and `BlobCodec` — handles into a content-addressed blob store.

use serde::{Deserialize, Serialize};

use crate::Hash256;

/// A handle to a content-addressed blob.
///
/// `BlobRef` carries the hash (canonical identity), the byte length (so consumers
/// can validate a fetched blob without re-hashing), and the codec the producer
/// chose (so consumers know whether to decompress). The actual bytes live in a
/// content-addressed store (typically `gbf-store::BlobStore`).
///
/// Provenance: `planv0.md` line 746.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BlobRef {
    pub hash: Hash256,
    pub len: u32,
    pub codec: BlobCodec,
}

/// How blob bytes are encoded on disk.
///
/// `Raw` is an uncompressed byte stream; `Zstd` is a zstd-compressed byte stream.
/// `gbf-store::BlobStore` does not interpret the codec — it stores bytes opaquely.
/// Decompression is the consumer's responsibility.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobCodec {
    Raw,
    Zstd,
}
```

### 4.3 Tests

```bash
cargo test -p gbf-foundation -- blob::blob_ref_serde_round_trip
cargo test -p gbf-foundation -- blob::blob_codec_serde_snake_case   # "raw" / "zstd"
cargo test -p gbf-foundation -- blob::blob_ref_size_is_compact      # size_of::<BlobRef>() <= 40 (Hash256=32 + u32 + u8 + padding)
```

The `serde_snake_case` test pins JSON shape: `{"hash":"...","len":1234,"codec":"zstd"}`. Future drift (e.g., someone adds `#[serde(rename = "Zstd")]`) breaks the test.

### 4.4 Constitution checkpoints

- §I.1: `BlobCodec` is closed; consumers `match` exhaustively.
- §VI.1: `BlobRef` lives only here; every other crate re-uses the foundation type.

## 5. Blob store and atomic writes (T-A6.1 part 1, `gbf-store::blob`)

**Bead**: `bd-1jy` (P1). **Reference**: `planv0.md` line 138, line 2511 (always-on content-addressed `StageCache`).

### 5.1 The directory layout

```
<store_root>/
  blobs/
    sha256/
      00/   01/   02/   ...   fe/   ff/
        <hash>      ← 64-character lowercase hex; the file is the blob's bytes.
  tmp/
    <random>      ← in-flight writes; renamed onto blobs/sha256/<ab>/<hash> on success.
```

Every blob lives at `blobs/sha256/<ab>/<hash>` where `<ab>` is the first two hex characters of the lowercase 64-character SHA-256 hex digest. The two-character split creates 256 shard directories; each shard holds roughly `total_blobs / 256` files under a uniform hash distribution.

### 5.2 `BlobStore`

```rust
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use gbf_foundation::{BlobCodec, BlobRef, Hash256};

pub struct BlobStore {
    root: PathBuf,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum DurabilityMode {
    /// fsync the file before rename. Sufficient for read-after-return on the
    /// same machine. Does *not* fully guarantee durability of the renamed
    /// directory entry across OS crash or power loss.
    #[default]
    ReadAfterReturn,
    /// fsync the file before rename, plus best-effort fsync of the destination
    /// directory after rename on platforms that support it.
    Full,
}

impl BlobStore {
    /// Open an existing store, or create one at `root` if absent.
    /// Creates `root/blobs/sha256/` and `root/tmp/`. Does *not* delete arbitrary
    /// files in `tmp/` during open (another process may have an in-flight
    /// write); use `cleanup_tmp` as an explicit maintenance operation.
    pub fn open(root: PathBuf) -> Result<Self, BlobStoreError>;

    /// Hash `bytes` and store atomically. Returns the hash.
    /// Idempotent: putting identical bytes twice returns the same hash and
    /// leaves the canonical path byte-identical. The early-out path verifies
    /// the existing canonical file's contents (re-hash + compare); a corrupt
    /// canonical file surfaces as `ExistingBlobCorrupt`.
    /// Atomic against single-process crash: writes go to a store-local
    /// temporary, fsync, then rename onto the canonical path.
    pub fn put(&self, bytes: &[u8]) -> Result<Hash256, BlobStoreError>;

    /// Store `bytes` only if they hash to `expected`. Used by callers
    /// (notably archive extraction) that have an external claimed hash.
    pub fn put_expect(&self, expected: Hash256, bytes: &[u8])
        -> Result<Hash256, BlobStoreError>;

    /// Convenience: store `bytes`, return a `BlobRef` carrying `codec`.
    /// Returns `BlobTooLarge` if `bytes.len() > u32::MAX`.
    pub fn put_as(&self, bytes: &[u8], codec: BlobCodec)
        -> Result<BlobRef, BlobStoreError>;

    /// Streaming variant. Hashes-as-it-reads, writes to tmp, fsyncs, renames.
    pub fn put_streaming<R: Read>(&self, reader: R)
        -> Result<Hash256, BlobStoreError>;

    /// Read the blob at `hash` into memory.
    pub fn get(&self, hash: Hash256) -> Result<Vec<u8>, BlobStoreError>;

    /// Open the blob at `hash` for streaming read (no full materialization).
    pub fn get_streaming(&self, hash: Hash256)
        -> Result<impl Read, BlobStoreError>;

    /// Read via `BlobRef`; validates `r.len` matches the on-disk blob length.
    pub fn get_ref(&self, r: BlobRef) -> Result<Vec<u8>, BlobStoreError>;

    /// True iff `blobs/sha256/<ab>/<hash>` exists. Does not verify contents.
    pub fn exists(&self, hash: Hash256) -> bool;

    /// Remove a blob. Used by GC; not generally called by user code.
    pub fn remove(&self, hash: Hash256) -> Result<(), BlobStoreError>;

    /// Collect every blob hash present in the store. Used by integrity
    /// verification and GC. Prefers a `Vec` over an iterator type so the
    /// public API stays simple; the store walks the directory once.
    pub fn list_blobs(&self) -> Result<Vec<Hash256>, BlobStoreError>;

    /// Compute the on-disk path for a hash. Public for diagnostic tools.
    pub fn path_for(&self, hash: Hash256) -> PathBuf;

    /// Explicit maintenance: remove tmp files older than `max_age`. Callers
    /// must coordinate (no other process should be mid-write).
    pub fn cleanup_tmp(&self, max_age: std::time::Duration)
        -> Result<u32, BlobStoreError>;
}

#[derive(Debug)]
pub enum BlobStoreError {
    Io(io::Error),
    LenMismatch { expected: u32, actual: u32 },
    HashMismatch { expected: Hash256, actual: Hash256 },
    NotFound { hash: Hash256 },
    BlobTooLarge { len: u64, max: u64 },
    ExistingBlobCorrupt { hash: Hash256, actual: Hash256 },
}

impl core::fmt::Display for BlobStoreError { /* explicit */ }
impl core::error::Error for BlobStoreError {}
impl From<io::Error> for BlobStoreError { fn from(e: io::Error) -> Self { Self::Io(e) } }
```

### 5.3 The atomic write protocol

```rust
impl BlobStore {
    pub fn put(&self, bytes: &[u8]) -> Result<Hash256, BlobStoreError> {
        // 1. hash the input
        let hash = sha256_of(bytes);
        let canonical = self.path_for(hash);

        // 2. early-out on idempotent re-write — but verify, don't trust.
        if canonical.exists() {
            let existing = fs::read(&canonical)?;
            let actual = sha256_of(&existing);
            if actual != hash {
                return Err(BlobStoreError::ExistingBlobCorrupt { hash, actual });
            }
            return Ok(hash);
        }

        // 3. write to tmp (store-local, same filesystem as canonical)
        let tmp_path = self.tmp_path();              // root/tmp/<random>
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;                                 // fsync(file)

        // 4. ensure parent dir exists
        fs::create_dir_all(canonical.parent().unwrap())?;

        // 5. rename. On platforms where rename cannot overwrite, AlreadyExists
        //    is treated as success only after re-hashing the existing file.
        match fs::rename(&tmp_path, &canonical) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let existing = fs::read(&canonical)?;
                let actual = sha256_of(&existing);
                if actual != hash {
                    return Err(BlobStoreError::ExistingBlobCorrupt { hash, actual });
                }
                let _ = fs::remove_file(&tmp_path);
            }
            Err(e) => return Err(e.into()),
        }

        // 6. (DurabilityMode::Full only) best-effort fsync(parent dir)
        Ok(hash)
    }
}
```

The protocol guarantees:

- A crash *before* step 5 leaves the tmp file but not the canonical path. Tmp cleanup is an explicit maintenance operation (`cleanup_tmp`), not a side effect of `open`, because another process may have an in-flight write.
- A crash *during* step 5 is impossible: `rename(2)` is atomic on the same filesystem; F-A6 puts `tmp/` inside the store root specifically to ensure same-filesystem rename.
- File fsync before rename guarantees read-after-return on the same machine. Full durability of the renamed directory entry across OS crash or power loss requires `DurabilityMode::Full` (which best-effort-fsyncs the destination directory after rename).
- A concurrent `put` of identical bytes from two processes is permitted but not used as a coordination primitive. Implementations must handle the race where the canonical path appears between the existence check and the commit step. On platforms whose `rename` cannot overwrite an existing destination, `AlreadyExists` is treated as success only after re-hashing the existing canonical file and confirming the bytes match.
- `put` of *different* bytes at the same hash is impossible under SHA-256 collision resistance; an `ExistingBlobCorrupt` surfacing here means external on-disk damage, not a hash collision.

### 5.4 Streaming: `put_streaming` / `get_streaming`

For blobs above an inline threshold (say, 64 KiB), the consumer prefers streaming. `put_streaming` hashes-as-it-reads, writing to tmp incrementally, so memory stays bounded. `get_streaming` returns a `BufReader` over the canonical file.

```rust
pub fn put_streaming<R: Read>(&self, mut reader: R) -> Result<Hash256, BlobStoreError> {
    let tmp_path = self.tmp_path();
    let mut f = fs::File::create(&tmp_path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
        f.write_all(&buf[..n])?;
    }
    f.sync_all()?;
    let hash = Hash256::from_bytes(hasher.finalize().into());
    let canonical = self.path_for(hash);
    if canonical.exists() {
        // idempotent path: verify existing then discard tmp
        let existing = fs::read(&canonical)?;
        let actual = sha256_of(&existing);
        if actual != hash {
            let _ = fs::remove_file(&tmp_path);
            return Err(BlobStoreError::ExistingBlobCorrupt { hash, actual });
        }
        let _ = fs::remove_file(&tmp_path);
    } else {
        fs::create_dir_all(canonical.parent().unwrap())?;
        fs::rename(&tmp_path, &canonical)?;
    }
    Ok(hash)
}
```

### 5.5 Tests

```bash
cargo test -p gbf-store -- blob::round_trip                          # put then get returns identical bytes
cargo test -p gbf-store -- blob::content_addressed_idempotent         # put(x) twice returns same hash
cargo test -p gbf-store -- blob::atomic_writes                        # injected mid-write failure leaves no canonical-path file
cargo test -p gbf-store -- blob::streaming_round_trip                 # put_streaming + get_streaming match
cargo test -p gbf-store -- blob::streaming_hash_matches_inline        # put_streaming(bytes) == put(bytes) hash-wise
cargo test -p gbf-store -- blob::two_char_prefix_layout               # path_for(h) lives at sha256/<h[..2]>/<h>
cargo test -p gbf-store -- blob::open_preserves_tmp_files             # open() does not delete possible in-flight writes
cargo test -p gbf-store -- blob::cleanup_tmp_removes_old_orphans      # explicit cleanup operation removes stale tmp files
cargo test -p gbf-store -- blob::open_creates_dirs                    # fresh root has blobs/sha256 + tmp after open
cargo test -p gbf-store -- blob::put_as_returns_blob_ref              # BlobRef.hash/len/codec are correct
cargo test -p gbf-store -- blob::put_as_rejects_oversize              # bytes.len() > u32::MAX -> BlobTooLarge
cargo test -p gbf-store -- blob::put_expect_hash_match                # put_expect with correct hash succeeds
cargo test -p gbf-store -- blob::put_expect_hash_mismatch             # put_expect with wrong hash -> HashMismatch, no commit
cargo test -p gbf-store -- blob::idempotent_verifies_existing         # corrupt canonical file -> ExistingBlobCorrupt
cargo test -p gbf-store -- blob::get_ref_validates_len                # mismatched len -> BlobStoreError::LenMismatch
cargo test -p gbf-store -- blob::list_blobs_walks_all                 # list_blobs() returns each blob exactly once
cargo test -p gbf-store -- blob::remove_then_exists_false             # removed blob is no longer present
cargo test -p gbf-store -- blob::concurrent_writes_idempotent         # two threads put(x) -> same hash, no error
```

The `atomic_writes` test simulates crash by writing to tmp, calling fsync, but *not* renaming, then re-opening the store — the canonical path must not exist and the tmp must be cleaned. Implementation uses `tempfile`'s `NamedTempFile::keep` plus a manual `remove_file` to simulate the partial-write window.

### 5.6 Constitution checkpoints

- §I.1: `BlobStoreError` is enumerated; consumers `match` exhaustively.
- §III: Atomic writes are tested by injected-failure, not by hope.
- §IV.3: Content-addressed-idempotent is a test invariant.
- §VI.1: `BlobStore` is the workspace's only blob storage.

## 6. Integrity (T-A6.1 part 2, `gbf-store::integrity`)

**Bead**: `bd-1jy` (P1). **Reference**: `planv0.md` line 138 (integrity verification).

### 6.1 `IntegrityReport`

```rust
use serde::{Deserialize, Serialize};

use gbf_foundation::Hash256;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IntegrityReport {
    pub blobs_checked: u64,
    pub mismatches: Vec<Hash256>,        // blob's actual contents hashed differently than its filename
    pub missing: Vec<Hash256>,           // referenced but not on disk; populated by verify_reachable, not verify_all
}

#[derive(Debug)]
pub enum IntegrityError {
    Io(io::Error),
    BlobStore(BlobStoreError),
    HashMismatch { expected: Hash256, actual: Hash256 },
    NotFound { hash: Hash256 },
    BlobReferenceDecode(BlobReferenceError),
}
impl core::fmt::Display for IntegrityError { /* explicit */ }
impl core::error::Error for IntegrityError {}
impl From<io::Error> for IntegrityError { fn from(e: io::Error) -> Self { Self::Io(e) } }
impl From<BlobStoreError> for IntegrityError { fn from(e: BlobStoreError) -> Self { Self::BlobStore(e) } }
```

### 6.2 `verify_integrity` and `verify_all`

```rust
/// Re-read the blob at `hash` and confirm its contents hash to `hash`.
pub fn verify_integrity(store: &BlobStore, hash: Hash256) -> Result<(), IntegrityError> {
    if !store.exists(hash) {
        return Err(IntegrityError::NotFound { hash });
    }
    let bytes = store.get(hash)?;
    let actual = Hash256::from_bytes(sha2::Sha256::digest(&bytes).into());
    if actual != hash {
        return Err(IntegrityError::HashMismatch { expected: hash, actual });
    }
    Ok(())
}

/// Walk every blob in the store and verify each one. The report aggregates
/// mismatches; `missing` is always empty for `verify_all` because every walked
/// blob exists by construction. Use `verify_reachable` to detect missing
/// references against a known root set.
pub fn verify_all(store: &BlobStore) -> Result<IntegrityReport, IntegrityError> {
    let mut report = IntegrityReport {
        blobs_checked: 0,
        mismatches: Vec::new(),
        missing: Vec::new(),
    };
    for h in store.list_blobs()? {
        report.blobs_checked += 1;
        match verify_integrity(store, h) {
            Ok(()) => {}
            Err(IntegrityError::HashMismatch { expected, .. }) => {
                report.mismatches.push(expected);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(report)
}

/// Walk a known reference graph and check for missing-but-referenced blobs.
/// Use this when integrity-verification needs to discover dangling references
/// (e.g., for archive validation or for confirming a pinset still resolves
/// after GC).
pub fn verify_reachable(
    store: &BlobStore,
    roots: impl IntoIterator<Item = Hash256>,
    refs: &BlobReferencesRegistry,
) -> Result<IntegrityReport, IntegrityError>;
```

`verify_all` propagates IO errors as `Err`; it does not panic. A corrupted blob is recorded as a `mismatch` and the walk continues, because the report is the diagnostic — `verify_all` is meant to scan the whole store and report all problems at once.

### 6.3 Tests

```bash
cargo test -p gbf-store -- integrity::detects_corruption           # tamper a blob -> verify_integrity returns HashMismatch
cargo test -p gbf-store -- integrity::detects_missing               # remove a referenced hash -> NotFound
cargo test -p gbf-store -- integrity::verify_all_no_mismatches      # clean store -> report.mismatches is empty
cargo test -p gbf-store -- integrity::verify_all_counts_blobs       # report.blobs_checked == number of put() calls
cargo test -p gbf-store -- integrity::report_serde_round_trip       # IntegrityReport JSON round-trip
```

The `detects_corruption` test directly opens the canonical file with write access and flips a byte mid-content; `verify_integrity` then returns `HashMismatch { expected: <original>, actual: <tampered> }`.

### 6.4 Constitution checkpoints

- §V.3: `IntegrityReport` is structured; `IntegrityError` is enumerated.
- §VI.1: All integrity verification flows through these two functions.

## 7. Two-level StageCache (T-A6.2, `gbf-store::stage_cache`)

**Bead**: `bd-2ab` (P1). **Reference**: `planv0.md` line 2511 (two-level: shard-local + whole-build); line 2940 (engineering rule 20: shard-local + global).

### 7.1 Why two levels

A flat cache key — "hash of every input to this stage" — has a fatal property: changing one tile dim invalidates the cache for every stage that consumed *any* tile dim. That's correct for *global* legality concerns (the whole-build hash), but wrong for *local* concerns (a kernel re-tile only invalidates that kernel's shard).

The two-level decomposition:

- **Shard-local digest set**: hashes-of-just-this-component-and-its-direct-inputs. A kernel's range plan keys on the kernel's tensor digests, not on every other kernel's.
- **Global digest**: a build-wide hash that captures whole-build legality concerns (target profile, compile request, calibration set, feature flags). When a global invariant changes, every cache entry's composed key shifts; when it doesn't, shard-local-only changes affect only the entries whose digest set names the changed component.

`compose_key` is a closed-form function over a `StageKey` value and yields one canonical `StageCacheKey` used as the cache index. The shard-local digest set and the global digest are two components of one canonical cache key; F-A6 does not implement two physical cache tiers or a fallback hierarchy.

### 7.2 Types

```rust
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use gbf_foundation::{Hash256, SemVer};
use gbf_foundation::string_id;

string_id!(StageId);                 // defined in gbf-store: e.g., "asm.layout", "codegen.range_plan"
string_id!(ComponentId);             // defined in gbf-store: e.g., "kernel.matvec", "expert.42"
string_id!(FeatureFlag);             // defined in gbf-store: e.g., "trace_observability", "double_speed"

/// Hashes of named components used by the stage. Empty for stages whose
/// legality is purely global.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ComponentDigestSet {
    pub components: BTreeMap<ComponentId, Hash256>,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct StageKey {
    pub stage_id: StageId,
    pub shard_local: ComponentDigestSet,
    pub global: Hash256,
    pub feature_flags: BTreeSet<FeatureFlag>,
    pub pass_version: SemVer,
}

/// Composed cache-key digest. A newtype distinct from `Hash256` (a content
/// hash) so `blob_store.get(stage_cache_key.0)` is a type error rather than a
/// silent category bug.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct StageCacheKey(pub Hash256);

/// Returned by `StageCache::put`: the composed key under which the entry was
/// indexed plus the content hash of the stored payload blob.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct StageCacheEntry {
    pub key: StageCacheKey,
    pub payload_hash: Hash256,
}

#[derive(Debug)]
pub enum StageCacheError {
    Io(io::Error),
    BlobStore(BlobStoreError),
    KeyEncodingFailed,
}

pub struct StageCache<'s> {
    blob: &'s BlobStore,
}

impl<'s> StageCache<'s> {
    pub fn new(blob: &'s BlobStore) -> Self;
    pub fn get(&self, key: &StageKey) -> Result<Option<Vec<u8>>, StageCacheError>;
    pub fn put(&self, key: &StageKey, payload: &[u8])
        -> Result<StageCacheEntry, StageCacheError>;
}

/// Canonical digest of a StageKey. Two structurally-equivalent StageKeys
/// produce identical `StageCacheKey`s regardless of construction order.
pub fn compose_key(key: &StageKey) -> StageCacheKey;
```

### 7.3 Canonical encoding for `compose_key`

```rust
pub fn compose_key(key: &StageKey) -> StageCacheKey {
    let mut hasher = sha2::Sha256::new();

    // Stage id: length-prefixed UTF-8.
    let s = key.stage_id.as_str().as_bytes();
    hasher.update(&(s.len() as u32).to_le_bytes());
    hasher.update(s);

    // Shard-local: BTreeMap iter is sorted by key.
    hasher.update(&(key.shard_local.components.len() as u32).to_le_bytes());
    for (cid, h) in &key.shard_local.components {
        let cb = cid.as_str().as_bytes();
        hasher.update(&(cb.len() as u32).to_le_bytes());
        hasher.update(cb);
        hasher.update(h.as_bytes());
    }

    // Global hash.
    hasher.update(key.global.as_bytes());

    // Feature flags: BTreeSet iter is sorted.
    hasher.update(&(key.feature_flags.len() as u32).to_le_bytes());
    for f in &key.feature_flags {
        let fb = f.as_str().as_bytes();
        hasher.update(&(fb.len() as u32).to_le_bytes());
        hasher.update(fb);
    }

    // Pass version: 3 × u32.
    let v = key.pass_version;
    hasher.update(&v.major.to_le_bytes());
    hasher.update(&v.minor.to_le_bytes());
    hasher.update(&v.patch.to_le_bytes());

    StageCacheKey(Hash256::from_bytes(hasher.finalize().into()))
}
```

The encoding is byte-exact and order-stable. `BTreeMap` and `BTreeSet` guarantee iteration order. Length prefixes prevent the "string boundary" attack where two different pairs `("ab", "cd")` and `("a", "bcd")` would otherwise hash identically.

### 7.4 `get` / `put`

```rust
impl<'s> StageCache<'s> {
    pub fn put(&self, key: &StageKey, payload: &[u8])
        -> Result<StageCacheEntry, StageCacheError>
    {
        let composed = compose_key(key);
        // Cache naming: an index entry maps StageCacheKey -> payload-blob hash;
        // the payload itself lives in the BlobStore. The index file path is
        // `cache/<composed[..2]>/<composed>` containing the payload hash.
        let payload_hash = self.blob.put(payload)?;
        let index_path = self.cache_index_path(composed);
        atomic_write_index(&index_path, payload_hash)?;
        Ok(StageCacheEntry { key: composed, payload_hash })
    }

    pub fn get(&self, key: &StageKey) -> Result<Option<Vec<u8>>, StageCacheError> {
        let composed = compose_key(key);
        let index_path = self.cache_index_path(composed);
        if !index_path.exists() {
            return Ok(None);
        }
        let payload_hash = read_index(&index_path)?;
        match self.blob.get(payload_hash) {
            Ok(b) => Ok(Some(b)),
            Err(BlobStoreError::NotFound { .. }) => Ok(None),  // index points at a GC'd blob
            Err(e) => Err(StageCacheError::BlobStore(e)),
        }
    }
}
```

The implementation deliberately keeps the cache index simple: a tiny file containing the payload's `Hash256`. Three consequences:

- A cache miss is a missing index file, not a missing blob. (Missing-but-referenced blobs are an integrity concern surfaced by `verify_all` / `verify_reachable`.)
- A blob can be GC'd out from under a stale index; `get` treats that as a cache miss and returns `Ok(None)`, so the consumer transparently recomputes. Stage-cache payload blobs are weak cache entries, *not* GC roots — pinning them is the consumer's choice.
- Stale index files are GC's responsibility to sweep when `GcOptions::sweep_stage_cache_indexes` is set; without that sweep the index directory grows monotonically.

### 7.5 Tests

```bash
cargo test -p gbf-store -- stage_cache::deterministic_keys                   # same logical key -> same StageCacheKey regardless of construction order
cargo test -p gbf-store -- stage_cache::shard_invalidation                   # changing one component's digest changes the key
cargo test -p gbf-store -- stage_cache::feature_flag_sensitivity             # different flag set -> different key
cargo test -p gbf-store -- stage_cache::pass_version_sensitivity             # different pass_version -> different key
cargo test -p gbf-store -- stage_cache::round_trip                           # put then get returns identical payload
cargo test -p gbf-store -- stage_cache::put_returns_entry                    # StageCacheEntry { key, payload_hash } is correct
cargo test -p gbf-store -- stage_cache::miss_returns_none                    # absent key -> Ok(None)
cargo test -p gbf-store -- stage_cache::stale_index_treated_as_miss          # blob removed under index -> Ok(None)
cargo test -p gbf-store -- stage_cache::compose_key_length_prefix_safe       # ("ab","cd") and ("a","bcd") hash differently
cargo test -p gbf-store -- stage_cache::component_digest_btreemap_canonical  # insertion order doesn't affect hash
```

The `compose_key_length_prefix_safe` test is the key safety invariant: without length prefixes, two structurally-different keys with identical concatenated byte strings would hash identically. The test constructs both shapes and asserts distinct hashes.

### 7.6 Constitution checkpoints

- §I.1: `StageKey` is structured; no untyped strings as cache keys.
- §IV.3: Determinism is asserted by the deterministic-keys test.
- §VI.1: One stage cache implementation in the workspace.

## 8. Pinsets and the `BlobReferences` trait (T-A6.3 part 1, `gbf-store::pinset`)

**Bead**: `bd-1e9` (P2). **Reference**: `planv0.md` line 138 (pinsets/GC/eviction).

### 8.1 What a pinset is

A `Pinset` is a *named root set* of blob hashes. Pinsets are persisted (typically as `pinsets/<name>.json` under the store root, but the exact filesystem layout is private to the `gbf-store::gc` module — pinsets are passed by value to `run_gc`, so storage policy is consumer-decided). Common pinsets:

- `latest` — the latest N successful builds.
- `current_calibration` — the calibration cohort the current compile is using.
- `frontier` — Pareto-frontier checkpoints from `gbf-train::shadow_compile`.
- `pinned_for_review` — blobs an agent or reviewer has manually held.

A blob is GC-safe iff it is reachable from at least one pinset's roots via the `BlobReferences` trait.

### 8.2 Types

```rust
use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};
use gbf_foundation::Hash256;

/// A pinset name. Owned `String`; deserialization always produces an owned
/// value (`Cow<'static, str>` cannot borrow from arbitrary deserialized
/// input). Validated at construction: non-empty, no NUL, no `/` or `\`,
/// no `..` segment, no leading `.`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PinsetName(String);

#[derive(Debug)]
pub enum PinsetNameError {
    Empty,
    ContainsNul,
    ContainsPathSeparator,
    ParentSegment,
    LeadingDot,
}

impl PinsetName {
    pub fn new(s: impl Into<String>) -> Result<Self, PinsetNameError>;
    pub fn as_str(&self) -> &str;
}

impl<'de> serde::Deserialize<'de> for PinsetName {
    /// Custom Deserialize that runs the same validation as `new`.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s: String = String::deserialize(d)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Pinset {
    pub name: PinsetName,
    pub roots: BTreeSet<Hash256>,
    pub annotation: Option<String>,
}

/// Typed blob references. Implemented by in-memory consumers that already
/// hold a deserialized value. GC operates on opaque bytes and therefore does
/// not call this trait directly; see `gbf-store::gc::BlobReferenceReader` for
/// the recognizer-style trait the GC uses.
pub trait BlobReferences {
    fn referenced_blobs(&self) -> Vec<Hash256>;
}
```

### 8.3 `Pinset` invariants

- `name` is unique within the consumer's pinset registry. Two pinsets with the same name are considered equivalent for GC purposes.
- `roots` is a `BTreeSet`, so duplicates are impossible by construction.
- `annotation` is for humans (e.g., "build of 2026-04-26 14:30 UTC"); it does not affect GC behavior.

### 8.4 The `BlobReferences` trait

The trait is the *only* way for the GC to learn about transitive references inside a blob. Each blob format implements the trait inside its own crate:

```rust
// example: gbf-artifact (not part of F-A6, illustrative)
impl BlobReferences for ArtifactCore {
    fn referenced_blobs(&self) -> Vec<Hash256> {
        let mut refs = vec![self.canonical_payload.hash];
        for tensor in &self.tensors {
            refs.push(tensor.payload.hash);
        }
        refs
    }
}
```

`gbf-store::pinset` does not provide implementations for any specific format. It defines the *trait* and the GC consumes it (via a registry passed at GC time — see §9).

### 8.5 Tests

```bash
cargo test -p gbf-store -- pinset::serde_round_trip                  # JSON round-trip preserves structure
cargo test -p gbf-store -- pinset::name_validation_rejects_empty
cargo test -p gbf-store -- pinset::name_validation_rejects_path_separator
cargo test -p gbf-store -- pinset::name_validation_rejects_parent_segment
cargo test -p gbf-store -- pinset::name_validation_rejects_nul
cargo test -p gbf-store -- pinset::deserialize_runs_validation       # invalid JSON name -> deserialize error
cargo test -p gbf-store -- pinset::roots_dedup_via_btreeset          # inserting same hash twice gives one entry
cargo test -p gbf-store -- pinset::annotation_optional_round_trip
cargo test -p gbf-store -- pinset::blob_references_trait_compiles    # typed trait is object-safe
```

### 8.6 Constitution checkpoints

- §I.1: `Pinset` is typed; `BlobReferences` is a trait, not a `Box<dyn Any>`.
- §VI.1: All pinset semantics live here.

## 9. Garbage collection (T-A6.3 part 2, `gbf-store::gc`)

**Bead**: `bd-1e9` (P2). **Reference**: `planv0.md` line 138.

### 9.1 The keep-set walk

```
keep_set = empty
for pinset in pinsets:
    for root in pinset.roots:
        keep_set.insert(root)
        bytes = store.get(root)
        match registry.read_references(root, &bytes):
            Some(refs):
                for child in refs:
                    if child not in keep_set:
                        keep_set.insert(child)
                        queue child for transitive walk
            None and opts.unknown_reference_policy == TreatAsLeaf:
                (skip; child references intentionally not tracked)
            None and opts.unknown_reference_policy == Abort:
                return Err(GcError::UndecodableReferenceBearingBlob { hash: root })

remove_set = blobs_in_store - keep_set
sort remove_set by Hash256 ascending           # determinism for max_remove_per_run
if opts.sweep_stage_cache_indexes:
    also collect stale cache-index files whose payload_hash is in remove_set
if opts.dry_run:
    populate report.candidate_* fields; remove nothing
else:
    for h in remove_set (limited by max_remove_per_run):
        store.remove(h)
        record h in report.removed
```

### 9.2 Types

```rust
use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};
use gbf_foundation::Hash256;

#[derive(Clone, Debug)]
pub struct GcOptions {
    pub dry_run: bool,
    pub max_remove_per_run: Option<usize>,
    pub sweep_stage_cache_indexes: bool,
    pub unknown_reference_policy: UnknownReferencePolicy,
}

impl Default for GcOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            max_remove_per_run: None,
            sweep_stage_cache_indexes: false,
            unknown_reference_policy: UnknownReferencePolicy::Abort,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UnknownReferencePolicy {
    /// Default. Abort the walk if a pinned blob is expected to be reference-bearing
    /// but no registered reader can decode it. Forces consumers to register every
    /// reference-bearing format before running GC.
    Abort,
    /// Treat undecodable blobs as leaves. Children that are reachable only through
    /// the undecodable parent may be removed. Use only for formats known to have
    /// no outgoing references or for cache-like data where losing descendants is
    /// acceptable.
    TreatAsLeaf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GcReport {
    pub pinsets_walked: u64,
    pub blobs_kept: u64,
    /// Number of blobs identified for removal (whether or not they were actually removed).
    pub candidate_blobs: u64,
    /// Total bytes that would be freed by removing all candidates.
    pub candidate_bytes: u64,
    /// In dry-run mode this stays 0; otherwise it is the number actually removed.
    pub blobs_removed: u64,
    /// In dry-run mode this stays 0; otherwise it is the bytes actually freed.
    pub bytes_freed: u64,
    /// In dry-run mode this stays empty; otherwise it lists the removed hashes.
    pub removed: Vec<Hash256>,
}

#[derive(Debug)]
pub enum GcError {
    Io(io::Error),
    BlobStore(BlobStoreError),
    UndecodableReferenceBearingBlob { hash: Hash256 },
}
impl core::fmt::Display for GcError { /* explicit */ }
impl core::error::Error for GcError {}

/// Registry of recognizer-style byte readers. Each reader either recognizes a
/// blob's format and returns `Some(refs)`, or returns `None` to delegate to the
/// next reader. If every reader returns `None`, the GC consults
/// `GcOptions::unknown_reference_policy` to decide whether to abort or treat the
/// blob as a leaf.
pub struct BlobReferencesRegistry {
    readers: Vec<Box<dyn BlobReferenceReader>>,
}

#[derive(Debug)]
pub enum BlobReferenceError {
    Decode(String),
}

pub trait BlobReferenceReader: Send + Sync {
    /// Return `Some(refs)` if this reader recognizes the blob format and decodes
    /// successfully. Return `None` to indicate "this reader does not handle this
    /// format" (the registry will try the next reader). Return `Err` if the
    /// reader recognizes the format but the bytes are invalid.
    fn referenced_blobs(
        &self,
        hash: Hash256,
        bytes: &[u8],
    ) -> Result<Option<Vec<Hash256>>, BlobReferenceError>;
}

impl BlobReferencesRegistry {
    pub fn new() -> Self;
    /// An empty registry. Combined with `UnknownReferencePolicy::Abort`, GC will
    /// refuse to run if any pinset roots may contain references; combined with
    /// `TreatAsLeaf`, GC treats every pinned blob as a leaf.
    pub fn empty() -> Self;
    pub fn register<R>(&mut self, reader: R)
    where
        R: BlobReferenceReader + 'static;
}

pub fn run_gc(
    store: &BlobStore,
    pinsets: &[Pinset],
    refs: &BlobReferencesRegistry,
    opts: &GcOptions,
) -> Result<GcReport, GcError>;
```

### 9.3 Why the registry is a parameter, not global

Two reasons. First, different consumers care about different formats: a `gbf-cli archive verify` caller doesn't need every artifact-format reader linked in. Second, "global registry" patterns invariably leak ordering bugs (which thread registered first?) and break tests' independence. F-A6 takes a registry as an argument; the consumer constructs it.

For tests in F-A6 that pin only leaf blobs, `BlobReferencesRegistry::empty()` plus `UnknownReferencePolicy::TreatAsLeaf` suffices; tests of the abort policy use the same empty registry but assert the abort path fires when a pinned blob is intended to be reference-bearing. The default `Abort` policy is the safer option for production, even though it forces consumers to register every reference-bearing format before GC.

### 9.4 Tests

```bash
cargo test -p gbf-store -- gc::pinset_protection                          # pinned blob survives GC
cargo test -p gbf-store -- gc::unpinned_blob_removed                      # blob with no pinset reference is removed
cargo test -p gbf-store -- gc::transitive_refs_via_registry               # registered reader -> child blobs are kept
cargo test -p gbf-store -- gc::unknown_reference_policy_abort             # no registered reader for a pinned blob -> Err(UndecodableReferenceBearingBlob)
cargo test -p gbf-store -- gc::unknown_reference_policy_treat_as_leaf      # opt-in TreatAsLeaf -> walk treats blob as leaf, removes children
cargo test -p gbf-store -- gc::dry_run_populates_candidates                # dry_run=true -> candidate_blobs/candidate_bytes set, blobs_removed=0
cargo test -p gbf-store -- gc::max_remove_per_run_honored                 # max_remove=N -> at most N blobs removed; order is Hash256 ascending
cargo test -p gbf-store -- gc::removal_order_is_hash_ascending             # max_remove=N selects the same N hashes regardless of store walk order
cargo test -p gbf-store -- gc::report_counts_correct                      # blobs_kept + candidate_blobs == total in store before
cargo test -p gbf-store -- gc::bytes_freed_matches_sum_of_lens            # report.bytes_freed equals sum of actually-removed blob sizes
cargo test -p gbf-store -- gc::sweep_stage_cache_indexes_removes_stale     # opt-in sweep removes index files whose payload was removed
cargo test -p gbf-store -- gc::report_serde_round_trip                    # GcReport JSON round-trip
cargo test -p gbf-store -- gc::two_pinsets_union                          # blob in either pinset is kept
```

### 9.5 Constitution checkpoints

- §V.3: `GcReport` is structured JSON; `GcError` is enumerated.
- §I.1: `GcOptions` is typed; `dry_run` is `bool`, not a magic flag string.
- §VI.1: GC lives only here.

## 10. Archive transport (T-A6.4, `gbf-store::archive`)

**Bead**: `bd-1yd` (P2). **Reference**: `planv0.md` line 138 (archive/directory transport).

### 10.1 The archive format

```
[ArchiveHeader: 24 bytes]
  magic:        b"GBLM\0ARC"      (8 bytes; the \0 is the fifth byte)
  version:      u8                (1 byte; current = 1)
  pinset_count: u16 (LE)          (2 bytes)
  reserved:     u8                (1 byte; must be 0)
  blob_count:   u32 (LE)          (4 bytes)
  total_bytes:  u64 (LE)          (8 bytes; sum of all blob body lengths,
                                   excluding header and pinset metadata)

[Pinset table]
  for each pinset (pinset_count repeats):
    name_len:    u16 (LE)
    name:        bytes (UTF-8)
    annotation_present: u8 (0 or 1)
    annotation_len: u16 (LE; only if annotation_present == 1)
    annotation:  bytes (UTF-8; only if annotation_present == 1)
    root_count:  u32 (LE)
    roots:       [Hash256; root_count]

[Blob records]
  for each blob (blob_count repeats):
    hash:   Hash256 (32 bytes)
    length: u64 (LE)
    bytes:  [u8; length]
```

The format is **byte-stable** for identical inputs. `create_archive` sorts pinsets by `PinsetName` ascending before writing the pinset table, and sorts blobs by `Hash256` ascending before writing the blob records. Callers do not have to pre-sort.

Because each blob record's body immediately follows its `(hash, length)` pair, listing an archive with a plain `Read` requires reading and discarding each body to advance to the next record — `list_archive` is `O(total archive bytes)` when given `Read`. A future overload accepting `Read + Seek` may skip bodies; `list_archive` is documented as best-effort O(total) for the streaming case.

### 10.2 Types

```rust
use std::io::{Read, Write};
use serde::{Deserialize, Serialize};
use gbf_foundation::Hash256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveHeader {
    pub magic: [u8; 8],          // b"GBLM\0ARC"
    pub version: u8,
    pub pinset_count: u16,
    pub blob_count: u32,
    pub total_bytes: u64,        // sum of blob body lengths only
}

pub const ARCHIVE_MAGIC: [u8; 8] = *b"GBLM\0ARC";
pub const ARCHIVE_VERSION: u8 = 1;

/// What `extract_archive` returns to its caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedArchive {
    pub header: ArchiveHeader,
    pub pinsets: Vec<Pinset>,
}

#[derive(Clone, Debug)]
pub struct ArchiveContents {
    pub header: ArchiveHeader,
    pub pinsets: Vec<Pinset>,
    pub blobs: Vec<(Hash256, u64)>,    // hash + length, no body
}

#[derive(Debug)]
pub enum ArchiveError {
    Io(io::Error),
    BadMagic { found: [u8; 8] },
    UnsupportedVersion { found: u8 },
    Truncated,
    Utf8(core::str::Utf8Error),
    BlobStore(BlobStoreError),
    TooManyPinsets { count: usize, max: u16 },
    PinsetNameTooLong { len: usize, max: u16 },
    AnnotationTooLong { len: usize, max: u16 },
    TooManyRoots { count: usize, max: u32 },
    BlobTooLarge { len: u64, max: u64 },
    TotalBytesMismatch { header: u64, actual: u64 },
    HashMismatch { expected: Hash256, actual: Hash256 },
}
impl core::fmt::Display for ArchiveError { /* explicit */ }
impl core::error::Error for ArchiveError {}

/// Walk pinsets transitively (via the registry), then write the archive.
/// Pinsets are sorted by name, blobs by hash, internally.
pub fn create_archive(
    store: &BlobStore,
    pinsets: &[Pinset],
    refs: &BlobReferencesRegistry,
    out: &mut dyn Write,
) -> Result<ArchiveHeader, ArchiveError>;

/// Read header + pinset table + blob records, validating each blob's hash
/// before committing it to the store. Extraction is *not* transactional: a
/// malformed later record may leave earlier-extracted blobs in `store`.
pub fn extract_archive(
    in_: &mut dyn Read,
    store: &BlobStore,
) -> Result<ExtractedArchive, ArchiveError>;

/// Read header + pinset table + each blob's `(hash, length)`. With a plain
/// `Read`, this is O(total archive bytes) because blob bodies must be
/// consumed between records.
pub fn list_archive(in_: &mut dyn Read) -> Result<ArchiveContents, ArchiveError>;
```

### 10.3 Determinism

`create_archive` enforces deterministic output by:

1. Sorting pinsets by `PinsetName` ascending before writing the pinset table (so callers do not need to pre-sort).
2. Within each pinset, `roots` is already a `BTreeSet`, so iteration is sorted.
3. Building the keep-set transitively (using the same `BlobReferencesRegistry` walk as GC), then writing blobs in `Hash256`-ascending order.
4. Writing all integer fields little-endian, fixed-width.
5. Not embedding any timestamp, host name, user, or environment information.

### 10.4 Round-trip property

```rust
fn round_trip(pinsets: &[Pinset], blobs_in_store_a: BTreeSet<Hash256>) -> bool {
    let store_a = make_store_with(blobs_in_store_a);
    let registry = BlobReferencesRegistry::empty();   // adjust per format set
    let mut buf = Vec::new();
    let header = create_archive(&store_a, pinsets, &registry, &mut buf).unwrap();

    let store_b = make_empty_store();
    let extracted = extract_archive(&mut Cursor::new(&buf), &store_b).unwrap();

    extracted.header == header
        && extracted.pinsets == sorted_by_name(pinsets)
        && store_b.list_blobs().unwrap() == store_a.list_blobs().unwrap()
}
```

The round-trip is the load-bearing acceptance test. It captures every property simultaneously: header parses, pinsets are returned to the caller in sorted order, blobs are preserved and verified by hash on extract.

### 10.5 Tests

```bash
cargo test -p gbf-store -- archive::magic_bytes                          # ARCHIVE_MAGIC == b"GBLM\0ARC" exactly (8 bytes)
cargo test -p gbf-store -- archive::version_byte                          # ARCHIVE_VERSION == 1
cargo test -p gbf-store -- archive::round_trip                            # create + extract preserves blobs and pinsets
cargo test -p gbf-store -- archive::pinsets_sorted_by_name_internally     # caller order doesn't affect archive bytes
cargo test -p gbf-store -- archive::list_without_extract                  # list_archive doesn't write to a store
cargo test -p gbf-store -- archive::deterministic_bytes                   # same inputs -> same archive bytes
cargo test -p gbf-store -- archive::blobs_sorted_by_hash                  # written body order matches Hash256 ascending
cargo test -p gbf-store -- archive::extract_validates_each_record_hash    # tampered blob bytes -> HashMismatch before commit
cargo test -p gbf-store -- archive::extract_not_transactional             # later malformed record leaves earlier blobs in store
cargo test -p gbf-store -- archive::bad_magic_rejected                    # truncated or wrong magic -> BadMagic
cargo test -p gbf-store -- archive::unsupported_version_rejected          # version > 1 -> UnsupportedVersion
cargo test -p gbf-store -- archive::truncated_input_detected              # trimmed bytes -> Truncated
cargo test -p gbf-store -- archive::pinset_with_annotation_round_trip
cargo test -p gbf-store -- archive::pinset_without_annotation_round_trip
cargo test -p gbf-store -- archive::overflow_too_many_pinsets             # > u16::MAX pinsets -> TooManyPinsets
cargo test -p gbf-store -- archive::overflow_pinset_name_too_long         # name > u16::MAX bytes -> PinsetNameTooLong
cargo test -p gbf-store -- archive::overflow_too_many_roots               # > u32::MAX roots -> TooManyRoots
cargo test -p gbf-store -- archive::header_size_24_bytes                  # serialized header is exactly 24 bytes
cargo test -p gbf-store -- archive::extracted_archive_returns_pinsets      # ExtractedArchive { header, pinsets } populated
```

### 10.6 Constitution checkpoints

- §IV.3: Archive bit-stability is a test invariant.
- §V.3: `ArchiveError` is enumerated.
- §VI.1: One archive format in the workspace.

## 11. Compatibility epochs (T-A6.5 part 1, `gbf-migrate::epochs`) — **DEFERRED**

> **DEFERRED — DO NOT IMPLEMENT under F-A6.** This section is preserved as design-notes for the follow-up bead **F-A6b** (which opens when the first real schema bump demands `gbf-migrate`). The F-A6 PR must not modify `gbf-migrate/src/epochs.rs`. See §0.0.0.

**Bead**: `bd-n9i` (P2, **deferred**). **Reference**: `planv0.md` line 1126 ("core crates define current schemas only"); line 1504 (`CompatibilityEpochs` shape).

### 11.1 What an epoch is

An *epoch* is a major version number for a contract family. Bumping an epoch means "the host-side format incremented in a way that requires a migrator to bridge old to new." Epochs are coarser than `SemVer` (which tracks within-format changes); an epoch bump is a flag day for migrators.

The four epochs F-A6 commits to:

- **`artifact`** — `ArtifactCore`, `ArtifactManifest`, `TargetDataLoweringArtifact`, `ArtifactAux`, `ReferenceModelBundle`. Bumped when artifact-lineage shape changes.
- **`abi`** — `InferenceState`, `HarnessCommandBlock`, `HarnessResultBlock`, `FaultCode`, `BuildIdentityBlock`, `CompatibilityEnvelope`, `TraceEvent`. Bumped when the live-execution ABI changes.
- **`calibration`** — `PlatformCalibrationBundle`, `KernelCalibrationBundle`, `RuntimeCalibrationBundle`. Bumped when calibration-bundle shape changes.
- **`reports`** — `RunManifest`, `FailureCapsule`, all `*.json` schemas. Bumped when report shape changes.

The four are independent: bumping `artifact` does not require bumping any other. This isolation is why M0 ships with `CompatibilityEpochs { artifact: 1, abi: 1, calibration: 1, reports: 1 }`.

### 11.2 Types

```rust
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct CompatibilityEpochs {
    pub artifact: u16,
    pub abi: u16,
    pub calibration: u16,
    pub reports: u16,
}

pub const CURRENT_EPOCHS: CompatibilityEpochs = CompatibilityEpochs {
    artifact: 1,
    abi: 1,
    calibration: 1,
    reports: 1,
};
```

The constant lives in this crate so every consumer (host loader, archive validator, migration planner) reads the same canonical "current epoch" value.

### 11.3 Tests

```bash
cargo test -p gbf-migrate -- epochs::current_constants_set         # all four fields >= 1
cargo test -p gbf-migrate -- epochs::serde_round_trip              # JSON round-trip
cargo test -p gbf-migrate -- epochs::field_independence            # bumping one doesn't auto-bump others
```

### 11.4 Constitution checkpoints

- §I.1: `CompatibilityEpochs` is a typed struct, not a free integer.
- §VI.1: Epochs live only here.

## 12. Migrator trait + DAG (T-A6.5 part 2, `gbf-migrate::dag`) — **DEFERRED**

> **DEFERRED — DO NOT IMPLEMENT under F-A6.** This section is preserved as design-notes for the follow-up bead **F-A6b**. The F-A6 PR must not modify `gbf-migrate/src/dag.rs`. See §0.0.0.

**Bead**: `bd-n9i` (P2, **deferred**). **Reference**: `planv0.md` line 1126.

### 12.1 The trait

```rust
use serde::{Deserialize, Serialize};
use gbf_foundation::SemVer;
use gbf_foundation::string_id;

string_id!(MigratorId);       // defined here: e.g., "artifact.1.0->1.1"
string_id!(SchemaFamilyId);   // defined here: e.g., "artifact", "abi", "calibration", "reports"

pub trait Migrator: Send + Sync {
    fn id(&self) -> MigratorId;
    fn from_version(&self) -> SemVer;
    fn to_version(&self) -> SemVer;
    fn loss_class(&self) -> MigrationLossClass;
    fn migrate(&self, input: &[u8]) -> Result<MigrateResult, MigrateError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrateResult {
    pub bytes: Vec<u8>,
    pub lossy_fields: Vec<crate::report::FieldPath>,
    pub warnings: Vec<crate::report::MigrationWarning>,
}

#[derive(Debug)]
pub enum MigrateError {
    InvalidInput { detail: String },
    SchemaMismatch { expected: SemVer, found: SemVer },
    Rejected { reason: String },         // migrator decided this input cannot be migrated
}
impl core::fmt::Display for MigrateError { /* explicit */ }
impl core::error::Error for MigrateError {}
```

The trait is byte-in / byte-out. No specific schema is referenced. Real migrators (e.g., "artifact 1.0 → 1.1") deserialize, transform, re-serialize, and return the result.

### 12.2 The DAG and the planner

```rust
use std::collections::BTreeMap;

pub struct MigrationGraph {
    family: SchemaFamilyId,
    migrators: Vec<Box<dyn Migrator>>,
    by_from: BTreeMap<SemVer, Vec<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub steps: Vec<MigratorId>,
    pub loss_class: crate::report::MigrationLossClass,    // worst loss across steps
    pub input_schema: SemVer,
    pub output_schema: SemVer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationExecution {
    pub bytes: Vec<u8>,
    pub report: crate::report::MigrationReport,
}

#[derive(Debug)]
pub enum MigrationGraphError {
    NoPath { from: SemVer, to: SemVer },
    /// Every candidate path crosses a Rejected migrator.
    NoAcceptablePath { from: SemVer, to: SemVer },
    AmbiguousPath { from: SemVer, to: SemVer, candidates: Vec<Vec<MigratorId>> },
    UnknownMigrator { id: MigratorId },
    DuplicateMigratorId { id: MigratorId },
    /// Identity self-edges (`from_version == to_version`) are rejected; use an empty plan instead.
    IdentityEdgeRejected { version: SemVer },
}
impl core::fmt::Display for MigrationGraphError { /* explicit */ }
impl core::error::Error for MigrationGraphError {}

impl MigrationGraph {
    pub fn new(family: SchemaFamilyId) -> Self;
    pub fn register(&mut self, m: Box<dyn Migrator>) -> Result<(), MigrationGraphError>;
    /// `plan(v, v)` returns an empty plan with `Lossless`.
    pub fn plan(&self, from: SemVer, to: SemVer) -> Result<MigrationPlan, MigrationGraphError>;
    pub fn execute(&self, plan: &MigrationPlan, input: &[u8])
        -> Result<MigrationExecution, MigrateError>;
}
```

`plan` searches from `from` toward `to` over the registered migrators' `from_version` → `to_version` edges. Candidate paths are ranked by:

1. aggregate loss class (`Lossless` < `LossyButAccepted` < `Rejected`) — a two-step lossless path beats a one-step lossy path;
2. hop count;
3. deterministic migrator-id sequence as the final tiebreaker.

If every candidate path contains a `Rejected` edge, `plan` returns `NoAcceptablePath`. Remaining ties after all three keys are an `AmbiguousPath` error (the consumer must register a distinguishing migrator or pick a path explicitly).

`execute` chains the migrators in order; on the first `Err(MigrateError::*)`, it stops and propagates. Successful execution returns a `MigrationExecution { bytes, report }` carrying the aggregate `MigrationReport`.

### 12.3 The scaffolding migrators

F-A6 ships two trivial migrators in `tests/`. Note that `IdentityEdgeRejected` forbids self-edges, so the "identity" fixture below is a *byte-preserving migrator across versions*, not a `(v, v)` self-edge:

```rust
// IdentityBytes: 1.X.0 -> 1.(X+1).0, lossless, byte-preserving.
// Used for tests that need a known-good migrator without exercising any field
// transformation. Its from/to versions are distinct so the graph accepts it.
pub struct IdentityBytesMigrator {
    id: MigratorId,
    from: SemVer,
    to: SemVer,
}
impl Migrator for IdentityBytesMigrator {
    fn id(&self) -> MigratorId { self.id.clone() }
    fn from_version(&self) -> SemVer { self.from }
    fn to_version(&self) -> SemVer { self.to }
    fn loss_class(&self) -> MigrationLossClass { MigrationLossClass::Lossless }
    fn migrate(&self, input: &[u8]) -> Result<MigrateResult, MigrateError> {
        Ok(MigrateResult { bytes: input.to_vec(), lossy_fields: vec![], warnings: vec![] })
    }
}

// BumpMinor: 1.X.0 -> 1.(X+1).0, lossless, byte-preserving (chains arbitrarily).
pub struct BumpMinorMigrator { id: MigratorId, from: SemVer }
```

These are *test fixtures*. F-A6 closure does not require any real migrator to ship. `plan(v, v)` is exercised by the `dag::plan_v_v_returns_empty` test, which never registers a self-edge.

### 12.4 Tests

```bash
cargo test -p gbf-migrate -- dag::single_step_migration             # 1.0 -> 1.1 via one bump-minor migrator
cargo test -p gbf-migrate -- dag::transitive_migration              # 1.0 -> 1.1 -> 1.2 chains correctly
cargo test -p gbf-migrate -- dag::no_path_returns_error             # missing migrator -> NoPath
cargo test -p gbf-migrate -- dag::no_acceptable_path                # every path contains Rejected -> NoAcceptablePath
cargo test -p gbf-migrate -- dag::ambiguous_path_detected           # remaining ties after loss/hops/id -> AmbiguousPath
cargo test -p gbf-migrate -- dag::loss_first_then_hops              # 2-hop Lossless beats 1-hop LossyButAccepted
cargo test -p gbf-migrate -- dag::duplicate_migrator_id_rejected    # register same id twice -> DuplicateMigratorId
cargo test -p gbf-migrate -- dag::identity_edge_rejected            # register from==to -> IdentityEdgeRejected
cargo test -p gbf-migrate -- dag::plan_v_v_returns_empty            # plan(v, v) -> empty plan, Lossless
cargo test -p gbf-migrate -- dag::loss_class_aggregation            # Lossless * Lossless = Lossless; Lossless * LossyButAccepted = LossyButAccepted
cargo test -p gbf-migrate -- dag::execute_returns_report            # MigrationExecution carries a MigrationReport
cargo test -p gbf-migrate -- dag::execute_propagates_error          # mid-chain MigrateError stops execution
cargo test -p gbf-migrate -- dag::register_then_plan_works          # registered migrator is reachable
cargo test -p gbf-migrate -- dag::identity_test_fixture_round_trip  # IdentityMigrator (test fixture only, not a self-edge) preserves bytes
```

### 12.5 Constitution checkpoints

- §I.1: `Migrator` is a trait; `MigratorId` is typed; `MigrationLossClass` is closed.
- §V.3: `MigrationGraphError` and `MigrateError` carry reproducible state.
- §VI.1: Host-side migration logic lives only here.

## 13. Migration report (T-A6.5 part 3, `gbf-migrate::report`) — **DEFERRED**

> **DEFERRED — DO NOT IMPLEMENT under F-A6.** This section is preserved as design-notes for the follow-up bead **F-A6b**. The F-A6 PR must not modify `gbf-migrate/src/report.rs`. See §0.0.0.

**Bead**: `bd-n9i` (P2, **deferred**). **Reference**: `planv0.md` line 1511 (`MigrationReport`); line 1519 (`MigrationLossClass`).

### 13.1 Types

```rust
use serde::{Deserialize, Serialize};
use gbf_foundation::SemVer;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationLossClass {
    Lossless,
    LossyButAccepted,
    Rejected,
}

/// Owned `String`. Field paths are JSON-facing report data; deserialization
/// always produces an owned value.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(pub String);     // e.g., "artifact.core.tensors[42].layout"

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MigrationWarning {
    pub field: FieldPath,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub input_schema: SemVer,
    pub output_schema: SemVer,
    pub migrators: Vec<crate::dag::MigratorId>,
    /// Aggregate loss class across `migrators`. Included so consumers do not
    /// have to recompute it from the graph.
    pub loss_class: MigrationLossClass,
    pub lossy_fields: Vec<FieldPath>,
    pub warnings: Vec<MigrationWarning>,
}
```

### 13.2 The loss class ordering

```rust
impl MigrationLossClass {
    /// Aggregate two loss classes by taking the worse.
    pub const fn worse(self, other: Self) -> Self {
        match (self, other) {
            (Self::Rejected, _) | (_, Self::Rejected) => Self::Rejected,
            (Self::LossyButAccepted, _) | (_, Self::LossyButAccepted) => Self::LossyButAccepted,
            _ => Self::Lossless,
        }
    }

    /// True iff `self` is no worse than `max_allowed`.
    /// `Lossless < LossyButAccepted < Rejected` (smaller is better).
    pub const fn is_no_worse_than(self, max_allowed: Self) -> bool {
        self.rank() <= max_allowed.rank()
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Lossless => 0,
            Self::LossyButAccepted => 1,
            Self::Rejected => 2,
        }
    }
}
```

`worse` is the aggregator the planner uses. `is_no_worse_than` is a policy helper for downstream consumers (e.g., a strict profile that refuses anything looser than `Lossless` calls `loss.is_no_worse_than(Lossless)`).

### 13.3 Tests

```bash
cargo test -p gbf-migrate -- report::loss_class_serde_snake_case        # "lossy_but_accepted"
cargo test -p gbf-migrate -- report::loss_class_worse_is_associative
cargo test -p gbf-migrate -- report::loss_class_worse_lossless_identity
cargo test -p gbf-migrate -- report::loss_class_rejected_absorbs
cargo test -p gbf-migrate -- report::field_path_static_zero_alloc
cargo test -p gbf-migrate -- report::migration_report_serde_round_trip
cargo test -p gbf-migrate -- report::warning_carries_field_path
```

### 13.4 Constitution checkpoints

- §I.1: `MigrationLossClass` is closed; downstream code `match`es exhaustively.
- §V.3: `MigrationReport` is structured; warnings carry typed field paths.
- §VI.1: One migration-reporting shape in the workspace.

## 14. Cross-cutting concerns

### 14.1 `std::io` usage

`gbf-store` depends on `std::io` and `std::fs`. It is fundamentally filesystem-shaped; abandoning `std` would mean shipping our own `Read`/`Write` traits and atomic-rename primitives, which is not a good use of the workspace's complexity budget. F-A6 takes the pragmatic position: `gbf-store` (and eventually `gbf-migrate` once F-A6b lands) is the workspace's `std`-using crate, and engineering rule 11 (`no_std + alloc` capable where practical) explicitly does not list it. Foundation's `BlobRef` populate stays `no_std`-shaped (it imports only `serde` and `gbf-foundation::Hash256`).

### 14.2 `serde` policy

`gbf-store` uses `serde` via the workspace pin. JSON is the canonical inspection format for `IntegrityReport`, `GcReport`, and `Pinset`. `serde_json` is dev-only. The archive format is *not* serde-driven; it is hand-rolled little-endian binary because deterministic-byte output is the contract and serde's JSON doesn't give us byte stability without extra work.

`BlobCodec` carries `#[serde(rename_all = "snake_case")]` to match the workspace style. Negative-deserialization tests pin against future drift. (`MigrationLossClass`'s serde policy is documented in §13 as a design note for the deferred F-A6b bead.)

### 14.3 Error style

Every error type:

- Is a `#[derive(Debug)]` enum (with derived `Clone, Eq, PartialEq` where the variants permit).
- Has explicit `impl core::fmt::Display` and `impl core::error::Error` (not `thiserror`, not `anyhow`).
- Carries enough state to fully reproduce the failing input.
- Is exposed in the module's public API.

This matches F-A1's and F-A2's error style and `CONSTITUTION.md` §V.3.

### 14.4 `unsafe` policy

`gbf-store` uses `#![forbid(unsafe_code)]` at the crate root. Compiler-enforced. No exceptions. (`gbf-migrate` will gain the same attribute when F-A6b lands; F-A6 does not touch the file.)

### 14.5 Performance targets

- `BlobStore::put(N bytes)` — bounded by hashing throughput (~1 GB/s on commodity hardware) plus one fsync. Not in any inference hot path.
- `BlobStore::get(N bytes)` — bounded by read throughput. Not in any inference hot path.
- `compose_key(&StageKey)` — single SHA-256 over a few hundred bytes; sub-microsecond.
- `run_gc(M blobs)` — O(M) walk; each blob touched at most twice.
- `create_archive(N blobs, total bytes B)` — O(N + B) write; one pass.

The crates are consumed at compile time, build time, and ops time, never at inference time.

### 14.6 Versioning

`gbf-store` is `0.1.0` for M0; it stabilizes at `1.0.0` once F-B15 (StageCache integration) has validated the API in production. Pre-`1.0.0`, breaking changes to public types are coordinated through the bead graph. (`gbf-migrate` versioning is owned by F-A6b.)

`ArchiveHeader::version = 1` is its own concern: archives produced by M0 will be readable by `extract_archive` for the lifetime of the project. Bumping this is a flag day; F-A6 does not anticipate doing so.

### 14.7 Documentation

Every public item carries a doc comment with at minimum:

- A short description.
- A `# Provenance` block citing the `planv0.md` line that motivated the item, where applicable.
- A `# Examples` block if the item has non-trivial usage.

Doc comments are linted via `rustdoc`'s `--deny broken-intra-doc-links` and `--deny missing-docs` (workspace-level).

## 15. Implementation order

Within F-A6, the open tasks have a real DAG:

```
T-A6.0 foundation BlobRef       ─── independent (precondition)
T-A6.1 blob + integrity         ─── depends on T-A6.0 (BlobStore::put_as returns BlobRef)
T-A6.2 stage_cache              ─── depends on T-A6.1 (StageCache wraps BlobStore)
T-A6.3 pinset + gc              ─── depends on T-A6.1 (GC walks BlobStore::list_blobs)
T-A6.4 archive                  ─── depends on T-A6.1 + T-A6.3 (archive uses pinsets + blobstore)
T-A6.5 gbf-migrate scaffolding ─── DEFERRED (see §0.0.0; moved to follow-up bead F-A6b)
```

**Recommended order:**

1. **T-A6.0 `gbf-foundation::blob` populate**. Half a day. Mechanical; unblocks T-A6.1.
2. **T-A6.1 `gbf-store::blob` + `gbf-store::integrity`**. One day. The largest single piece; atomic-write protocol + tests.
3. **T-A6.2 `gbf-store::stage_cache`**. One day. `compose_key` canonical encoding + cache index.
4. **T-A6.3 `gbf-store::pinset` + `gbf-store::gc`**. One day. Trait + GC walk + dry-run/limit honoring.
5. **T-A6.4 `gbf-store::archive`**. Half a day. Hand-rolled binary format + round-trip tests.

**Total: ~3.5 days of focused work.** The critical path is T-A6.0 → T-A6.1 → (T-A6.2 || T-A6.3 || T-A6.4). (T-A6.5 was previously parallelizable; it is now deferred to F-A6b — see §0.0.0.)

**PR shape (closure):** F-A6 ships in a single PR that lands every in-scope module together plus the foundation `BlobRef` populate plus the `gbf-store/Cargo.toml` cleanup. Splitting into multiple PRs adds review overhead without reducing per-PR risk. However, the single PR is structured into reviewable commits in dependency order:

1. foundation `BlobRef`/`BlobCodec` populate (T-A6.0);
2. `gbf-store::blob` + `integrity` (T-A6.1) plus the `gbf-store/Cargo.toml` cleanup;
3. `gbf-store::stage_cache` (T-A6.2);
4. `gbf-store::pinset` + `gc` (T-A6.3);
5. `gbf-store::archive` (T-A6.4);
6. review packet + generated artifacts.

The review packet's diff disposition table follows these commits. A single PR closes T-A6.0 through T-A6.4 and the parent feature bead `bd-3ll` (re-scoped to `gbf-store` only) in one step. T-A6.5/`bd-n9i` remains deferred and is **not** closed by this PR.

## 16. Testing strategy summary

| Layer                        | Coverage                                                                                                   |
|------------------------------|------------------------------------------------------------------------------------------------------------|
| Type-level (compile-time)    | `BlobCodec` is an exhaustive enum. `BlobReferences` is a trait. `Pinset`, `StageKey`, `GcReport`, `IntegrityReport`, `ArchiveHeader` are typed structs. `#![forbid(unsafe_code)]` is compiler-enforced. |
| Atomic-write                 | `blob::atomic_writes` simulates crash mid-write; canonical path either complete or absent. `blob::tmp_cleanup_on_open` cleans orphan tmp files. |
| Determinism                  | `blob::content_addressed_idempotent` (put twice -> same hash). `stage_cache::deterministic_keys` (logical equality -> hash equality). `archive::deterministic_bytes` (same inputs -> same archive bytes). `archive::blobs_sorted_by_hash` (write order is canonical). |
| Round-trip                   | `blob::round_trip`, `blob::streaming_round_trip`, `stage_cache::round_trip`, `archive::round_trip`, `pinset::serde_round_trip`. |
| Negative                     | `blob::get_ref_validates_len` (LenMismatch). `integrity::detects_corruption` (HashMismatch). `archive::bad_magic_rejected`, `unsupported_version_rejected`, `truncated_input_detected`. |
| Boundary                     | `blob::two_char_prefix_layout`. `stage_cache::compose_key_length_prefix_safe`. `archive::header_size_24_bytes`. |
| GC behavior                  | `gc::pinset_protection`, `unpinned_blob_removed`, `transitive_refs_via_registry`, `unknown_format_conservative`, `dry_run_removes_nothing`, `max_remove_per_run_honored`, `bytes_freed_matches_sum_of_lens`. |
| Cross-module conformance     | StageCache writes flow through BlobStore; GC removes only blobs not pinned and not transitively referenced; archive extracts blobs that pass `verify_integrity`. |
| Snapshot                     | `IntegrityReport`, `GcReport`, `Pinset` JSON-shape snapshots for downstream report consumers. |
| Skill checklist              | Constructor-validated newtypes have negative deserialization tests (e.g., archive magic). Effect-classifier-style traversals (here: `compose_key` over StageKey, `run_gc` over pinsets) have totality tests. JSON-facing schemas have round-trip tests. |
| **DEFERRED to F-A6b**        | All `gbf-migrate::*` test layers (`MigrationLossClass`, `MigrationReport`, `CompatibilityEpochs` round-trips; `dag::no_path_returns_error`, `dag::ambiguous_path_detected`; `epochs::serde_round_trip`). |

All in-scope tests run as part of the workspace pre-commit hook (`cargo test --workspace --all-features`). The `gbf-migrate` crate stays stub-only after F-A6, so `cargo test -p gbf-migrate` is a no-op until F-A6b.

## 17. Resolved questions

These were the questions I planned to surface in PR review. Each is now resolved; the decisions below are load-bearing for closure. **Items marked `[F-A6b design note]`** are preserved for the deferred bead and are not gating for F-A6.

1. **`BlobRef` lives in `gbf-foundation`, not `gbf-store`.** Per `planv0.md` line 137, contract crates (artifact, hw, abi) carry `BlobRef`-typed fields; if `BlobRef` lived in `gbf-store`, every contract crate would have to depend on `gbf-store`, which inverts the architectural intent. F-A6 closure populates `gbf-foundation::blob` as a precondition step.
2. **`gbf-store::Cargo.toml` should not depend on `gbf-artifact`.** The current scaffold's dep is wrong. Closure removes it. The architectural intent is artifact → store, not store → artifact.
3. **`[F-A6b design note]` `gbf-migrate::Cargo.toml` should not depend on `gbf-abi` or `gbf-artifact`.** The trait operates on `&[u8]`; real migrators that know schemas live in their owning crates and register with a `MigrationGraph` instance. Scaffolding doesn't need either dep. **F-A6 leaves the file untouched; the cleanup lands in F-A6b.**
4. **Only `BlobRef`/`BlobCodec` live in `gbf-foundation`. `StageId`, `ComponentId`, `FeatureFlag` live in `gbf-store::stage_cache`.** The earlier draft mixed these locations and contradicted itself; this RFC pins ownership at the module level. (`MigratorId` and `SchemaFamilyId` ownership in `gbf-migrate::dag` is documented in §12 as a design note for F-A6b.)
5. **`StageKey` is a single canonical key with two components, not two physical caches.** Engineering rule 20 is about the canonical key's *inputs*, not about a fallback hierarchy. F-A6 implements one physical cache; `compose_key` returns a `StageCacheKey` newtype distinct from `Hash256` so consumers cannot confuse a cache key with a content hash.
6. **`compose_key` uses canonical length-prefixed encoding.** Without length prefixes, two structurally-different keys with identical concatenated byte strings would hash identically. The test `compose_key_length_prefix_safe` pins this.
7. **`Pinset.roots` is `BTreeSet<Hash256>`, not `Vec<Hash256>`.** Dedup is structural; iteration order is sorted.
8. **`PinsetName` is a validated owned `String`, not `Cow<'static, str>`.** `Cow<'static, str>` deserialization cannot borrow from arbitrary input, and pinset names become filenames — they need validation (no NUL, no path separators, no `..`, non-empty). The earlier zero-alloc claim was misplaced; pinset names are not in any hot path.
9. **The typed `BlobReferences` trait and the `BlobReferenceReader` recognizer trait are separate.** `BlobReferences` is for in-memory consumers that already hold a deserialized value; GC works on opaque bytes and uses `BlobReferenceReader` (returning `Option<Vec<Hash256>>`) through `BlobReferencesRegistry`. Pinsets carry only `Hash256`, so the GC-side trait must recognize-from-bytes rather than rely on a format tag.
10. **`run_gc` takes a `BlobReferencesRegistry` parameter.** Different consumers care about different formats; passing the registry as an argument lets each consumer build the right registry without leaking dependencies.
11. **Unknown reference-bearing formats default to `UnknownReferencePolicy::Abort`, not silent-leaf.** "Treating unknown formats as leaves" can delete blobs that are reachable only through an undecodable parent — which is *not* conservative for GC safety. The default refuses to GC; consumers opt into `TreatAsLeaf` only for formats known to have no outgoing references or for cache-like data where losing descendants is acceptable.
12. **`GcOptions` carries `sweep_stage_cache_indexes`.** Stage-cache index files are weak references (a stale index → cache miss → recompute), but the index directory must not grow without bound. GC sweeps stale indexes when this option is set.
13. **`GcReport` carries `candidate_*` counters separately from `blobs_removed`/`bytes_freed`.** Dry-run mode populates `candidate_*` and leaves `blobs_removed`/`bytes_freed`/`removed` zero/empty. Counts are `u64` (these are durable JSON-facing telemetry; no artificial `u32` ceiling).
14. **GC removal order is `Hash256` ascending.** Determinism for `max_remove_per_run` selection.
15. **Archive magic is `b"GBLM\0ARC"` (exactly 8 bytes; the `\0` is the fifth byte).** The earlier draft wrote `b"GBLM\0ARCH"` (9 bytes) which contradicted the 8-byte header field; this RFC fixes the magic to a literal 8 bytes.
16. **Archive header is exactly 24 bytes including a 1-byte reserved field.** `magic[8] + version[1] + pinset_count[2] + reserved[1] + blob_count[4] + total_bytes[8] = 24`. `total_bytes` is the sum of blob *body* lengths only, not including header or pinset metadata.
17. **`create_archive` accepts `BlobReferencesRegistry` and sorts pinsets internally.** Round-trip determinism depends on internal sorting (by `PinsetName` then by `Hash256`), not on caller order. `extract_archive` returns `ExtractedArchive { header, pinsets }` so pinsets actually round-trip; the original signature returning only the header silently dropped them.
18. **Archive extraction is *not* transactional.** A malformed later record may leave earlier-extracted blobs in the store. The contract is documented and tested explicitly so consumers do not assume rollback semantics.
19. **`BlobStore::put` has a verifying idempotent path; corrupt canonical files surface as `ExistingBlobCorrupt`.** Trusting "the path exists" is unsafe — the bytes at that path could be damaged. Re-hash before treating as a hit.
20. **`BlobStore::put_expect(expected, bytes)` exists for callers with an external claimed hash.** Archive extract uses it; the bare `put` does not have a claimed-hash parameter.
21. **`BlobStore` uses `Result<_, BlobStoreError>`, not `io::Result<_>`.** The error enum carries `LenMismatch`, `HashMismatch`, `BlobTooLarge`, `ExistingBlobCorrupt`, `NotFound` in addition to `Io`; `io::Result` cannot express those.
22. **Atomic-write durability has two modes: `ReadAfterReturn` (default, file-fsync-then-rename) and `Full` (also best-effort destination-directory fsync).** "Atomic" is not a single property; the RFC distinguishes single-process crash safety from full power-loss directory-entry durability.
23. **`open()` does not delete arbitrary tmp files.** Another process may have an in-flight write. Tmp cleanup is the explicit `cleanup_tmp(max_age)` operation.
24. **`fs::rename` can fail with `AlreadyExists` on some platforms.** F-A6 treats that as success only after re-hashing the existing canonical file and confirming the bytes match.
25. **`tempfile` and `sha2` are production deps for `gbf-store`** and pinned via `[workspace.dependencies]`. `BlobStore::put` uses `tempfile` for `tmp/<random>` and `sha2` for re-hashing, both in production paths.
26. **`[F-A6b design note]` `MigrationLossClass` is exactly `Lossless`/`LossyButAccepted`/`Rejected`.** Three classes are sufficient: lossless migrations always pass; lossy ones may be policy-rejected; explicitly-rejected migrations stop. Adding a fourth class (e.g., `LossyButRequiresReview`) would split `LossyButAccepted` along a dimension that downstream policy already controls.
27. **`[F-A6b design note]` `Migrator::migrate` takes `&[u8]` and returns `Vec<u8>`.** The trait is schema-agnostic. Real migrators deserialize, transform, re-serialize. This is what will justify removing `gbf-abi`/`gbf-artifact` from `gbf-migrate/Cargo.toml` when F-A6b lands.
28. **`[F-A6b design note]` `MigrationGraph::plan` ranks paths by loss class first, then hop count, then deterministic id sequence.** A two-step lossless path beats a one-step lossy path. `Rejected` edges exclude a path; if all paths contain Rejected → `NoAcceptablePath`.
29. **`[F-A6b design note]` `MigrationPlan` carries `input_schema` and `output_schema`.** This makes plans self-describing and prevents accidental execution in the wrong context.
30. **`[F-A6b design note]` `MigrationGraph::execute` returns `MigrationExecution { bytes, report }`.** The earlier draft discarded `lossy_fields` and `warnings`; the new shape always returns a `MigrationReport`.
31. **`[F-A6b design note]` `MigrationGraph::new(SchemaFamilyId)` is family-scoped.** One graph cannot mix migrators across families (artifact `1.0->1.1` and abi `1.0->1.1` cannot coexist in one graph).
32. **`[F-A6b design note]` `MigrationGraph::register` returns `Result<(), MigrationGraphError>`.** Duplicate ids are rejected (`DuplicateMigratorId`); identity self-edges (`from_version == to_version`) are rejected (`IdentityEdgeRejected`); `plan(v, v)` returns an empty Lossless plan instead.
33. **`[F-A6b design note]` `is_no_worse_than(max_allowed)`, not `is_at_least(threshold)`.** The earlier name was confusing and its implementation made `Lossless.is_at_least(Rejected)` true, which is the opposite of what policy code wants.
34. **F-A6 ships *no* migrators (real or fixture).** All migrator scaffolding (including `IdentityBytesMigrator` and `BumpMinorMigrator` test fixtures previously planned) is deferred to F-A6b — see §0.0.0.
35. **`gbf-store` uses `#![forbid(unsafe_code)]` as the *primary* unsafe gate.** Compiler-enforced. Grep is at most an advisory smoke check, never the contract. (`gbf-migrate` will gain the same attribute when F-A6b lands.)
36. **`StageCache` cache misses are silent.** A missing index → `Ok(None)`; a stale index pointing at a GC'd blob → `Ok(None)`. Stage-cache payload blobs are weak entries, not GC roots.
37. **Both crates are `std`-using.** Per engineering rule 11's literal text, the `no_std + alloc` mandate names `gbf-foundation`, `gbf-artifact`, `gbf-abi`, `gbf-ir`, and `gbf-asm`. `gbf-store` and `gbf-migrate` are explicitly outside that list.
38. **The PR is shipped as one PR but with reviewable commits in dependency order.** Reviewers absorb CAS, archive format, GC, cache index, migration DAG, and review packet across 7 commits rather than one mega-diff (see §15 for the commit ladder).

## 18. Risks

| Risk                                                                       | Likelihood | Mitigation                                                                                                                                              |
|----------------------------------------------------------------------------|------------|---------------------------------------------------------------------------------------------------------------------------------------------------------|
| Atomic-write protocol fails on a non-POSIX filesystem                      | Low        | F-A6 places `tmp/` inside the store root, ensuring same-filesystem rename atomicity on any reasonable filesystem. CI runs on Linux; macOS APFS is also tested. Windows-NTFS support is verified by a smoke test in `gbf-test` once that crate exists. |
| `compose_key` non-determinism due to encoding bug                           | Low        | The `deterministic_keys`, `compose_key_length_prefix_safe`, and `component_digest_btreemap_canonical` tests pin this. Length prefixes are explicit. |
| GC removes a blob that's actually referenced                                | Medium     | `BlobReferences`-driven walk is conservative for unknown formats (unknown format → no transitive walk → only direct pinning protects descendants). Dry-run mode is the recommended first invocation; `max_remove_per_run` bounds blast radius. |
| Archive format drift breaks old archives                                    | Low        | `ARCHIVE_VERSION` byte exists; F-A6 ships v1 only. A version bump requires explicit migration tooling, not just code changes. |
| ~~Migrator DAG has an `AmbiguousPath` that no one notices~~                 | n/a        | DEFERRED to F-A6b — `gbf-migrate` is not built by F-A6 (see §0.0.0). |
| Concurrent multi-process GC corrupts the store                               | Low (M0)   | M0 is single-process for both build and ops. `BlobStore::put` is multi-process-safe (idempotent rename); GC is not. F-A6 ships no lock; consumers serialize GC. A follow-up bead can add a lockfile if needed. |
| `StageCache` index file format drifts                                       | Low        | The index is intentionally trivial: a single `Hash256` per file. Bumping the format is a flag day; if it happens, a `MigrationGraph` migrator is the right tool. |
| `gbf-foundation::blob` populate is forgotten                                | Very Low   | The RFC self-check (§0.1) lists it as the first item. The PR includes `lib.rs` re-exports that fail to compile if the populate is missing. |
| `Cargo.toml` cleanup misses a transitive consumer                            | Very Low   | No production consumer of `gbf-store` exists today. The PR is mechanically additive. (`gbf-migrate/Cargo.toml` is not modified — its cleanup is deferred to F-A6b.) |

## 19. Claim-to-gate matrix (closure-style)

| Claim                                                                                | Gating test / artifact                                                                                       |
|--------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------|
| `BlobRef` exists in `gbf-foundation::blob`                                           | `cargo check -p gbf-foundation`; `gbf-foundation/src/lib.rs` re-exports `BlobRef`/`BlobCodec`.               |
| `BlobRef` JSON round-trips with `snake_case` codec                                   | `gbf-foundation/tests/blob.rs::blob_ref_serde_round_trip`                                                    |
| `BlobStore::put` is content-addressed-idempotent (with verification of any pre-existing canonical file) | `gbf-store::blob::content_addressed_idempotent`, `idempotent_verifies_existing` |
| `BlobStore::put` is atomic against single-process crash                              | `gbf-store::blob::atomic_writes`                                                                             |
| `BlobStore::put_expect` rejects mismatched hashes without committing                  | `gbf-store::blob::put_expect_hash_match`, `put_expect_hash_mismatch`                                         |
| `BlobStore::put_as` returns a correct `BlobRef` and rejects oversize input            | `gbf-store::blob::put_as_returns_blob_ref`, `put_as_rejects_oversize`                                        |
| Streaming put + inline put produce the same hash                                     | `gbf-store::blob::streaming_hash_matches_inline`                                                             |
| Two-character prefix layout: `blobs/sha256/<ab>/<hash>`                              | `gbf-store::blob::two_char_prefix_layout`                                                                    |
| `open()` does not delete tmp files; explicit `cleanup_tmp` does                        | `gbf-store::blob::open_preserves_tmp_files`, `cleanup_tmp_removes_old_orphans`                              |
| `verify_integrity` detects byte-level corruption                                      | `gbf-store::integrity::detects_corruption`                                                                   |
| `verify_all` reports mismatches without aborting (returns Result; no panic)           | `gbf-store::integrity::verify_all_no_mismatches`, `verify_all_counts_blobs`                                  |
| `verify_reachable` discovers missing referenced blobs                                  | `gbf-store::integrity::verify_reachable_finds_missing`                                                       |
| `IntegrityReport` JSON round-trips                                                    | `gbf-store::integrity::report_serde_round_trip`                                                              |
| `compose_key` is deterministic across construction order                              | `gbf-store::stage_cache::deterministic_keys`                                                                 |
| `compose_key` returns `StageCacheKey` (not raw `Hash256`)                              | type-level: `pub fn compose_key(...) -> StageCacheKey;` and the `stage_cache_key_distinct_from_hash` test    |
| `compose_key` length-prefix encoding prevents string-boundary collisions              | `gbf-store::stage_cache::compose_key_length_prefix_safe`                                                     |
| Shard-local digest changes change the key                                             | `gbf-store::stage_cache::shard_invalidation`                                                                 |
| Feature flags affect the key                                                          | `gbf-store::stage_cache::feature_flag_sensitivity`                                                            |
| Pass version affects the key                                                          | `gbf-store::stage_cache::pass_version_sensitivity`                                                            |
| StageCache returns `Ok(None)` on miss and on stale index                              | `gbf-store::stage_cache::miss_returns_none`, `stale_index_treated_as_miss`                                   |
| `StageCache::put` returns `StageCacheEntry { key, payload_hash }`                      | `gbf-store::stage_cache::put_returns_entry`                                                                  |
| `Pinset` JSON round-trips                                                             | `gbf-store::pinset::serde_round_trip`                                                                        |
| `PinsetName` validates input and runs validation on deserialize                       | `gbf-store::pinset::name_validation_*`, `pinset::deserialize_runs_validation`                                |
| `BlobReferences` typed trait is object-safe                                            | `gbf-store::pinset::blob_references_trait_compiles`                                                          |
| GC keeps pinned blobs                                                                 | `gbf-store::gc::pinset_protection`                                                                            |
| GC removes unpinned blobs                                                              | `gbf-store::gc::unpinned_blob_removed`                                                                        |
| GC walks transitive refs via registered recognizers                                   | `gbf-store::gc::transitive_refs_via_registry`                                                                 |
| Default `UnknownReferencePolicy::Abort` refuses to GC unrecognized reference-bearing pinned blobs | `gbf-store::gc::unknown_reference_policy_abort`                                                  |
| Opt-in `UnknownReferencePolicy::TreatAsLeaf` allows leaf treatment                    | `gbf-store::gc::unknown_reference_policy_treat_as_leaf`                                                      |
| GC dry-run populates `candidate_*` and removes nothing                                | `gbf-store::gc::dry_run_populates_candidates`                                                                |
| GC `max_remove_per_run` is honored deterministically (Hash256 ascending)              | `gbf-store::gc::max_remove_per_run_honored`, `removal_order_is_hash_ascending`                               |
| `sweep_stage_cache_indexes` removes stale index files                                  | `gbf-store::gc::sweep_stage_cache_indexes_removes_stale`                                                     |
| `GcReport` JSON round-trips                                                           | `gbf-store::gc::report_serde_round_trip`                                                                      |
| `GcReport.bytes_freed` equals sum of actually-removed blob sizes                       | `gbf-store::gc::bytes_freed_matches_sum_of_lens`                                                              |
| Archive magic bytes are exactly `b"GBLM\0ARC"` (8 bytes)                              | `gbf-store::archive::magic_bytes`                                                                             |
| Archive header is exactly 24 bytes                                                    | `gbf-store::archive::header_size_24_bytes`                                                                    |
| Archive round-trips through create + extract (pinsets returned via `ExtractedArchive`) | `gbf-store::archive::round_trip`, `extracted_archive_returns_pinsets`                                        |
| `create_archive` sorts pinsets internally                                              | `gbf-store::archive::pinsets_sorted_by_name_internally`                                                      |
| Archive bytes are deterministic for identical inputs                                  | `gbf-store::archive::deterministic_bytes`                                                                     |
| Archive blob order is `Hash256` ascending                                             | `gbf-store::archive::blobs_sorted_by_hash`                                                                    |
| `list_archive` works without writing a store                                          | `gbf-store::archive::list_without_extract`                                                                    |
| Archive extract validates each record's claimed hash before commit                     | `gbf-store::archive::extract_validates_each_record_hash`                                                     |
| Archive extract is *not* transactional (documented + tested)                           | `gbf-store::archive::extract_not_transactional`                                                              |
| Archive overflow paths return typed errors                                            | `gbf-store::archive::overflow_too_many_pinsets`, `overflow_pinset_name_too_long`, `overflow_too_many_roots`  |
| Archive bad magic / wrong version / truncation are typed errors                       | `gbf-store::archive::bad_magic_rejected`, `unsupported_version_rejected`, `truncated_input_detected`         |
| **DEFERRED to F-A6b — see §0.0.0:** `CompatibilityEpochs`, `Migrator` trait, single-/multi-step migration, missing-path errors, `NoAcceptablePath`, plan ranking, duplicate-migrator-id rejection, identity-self-edge rejection, `plan(v, v)`, `AmbiguousPath`, `MigrationExecution.report`, `MigrationLossClass` (worse/associativity/`Lossless`-identity/`Rejected` absorbs/`is_no_worse_than`/`snake_case`), `MigrationReport` round-trip. None of these are gating for F-A6 closure. | (no F-A6 test) |
| `gbf-store/Cargo.toml` does not depend on `gbf-artifact`                              | `Cargo.toml` diff in PR; dependency report                                                                   |
| `gbf-migrate/Cargo.toml` is **unchanged** by F-A6 (the `gbf-abi`/`gbf-artifact` cleanup is owned by F-A6b) | inspection of `git diff origin/main -- gbf-migrate/Cargo.toml` shows no changes        |
| `gbf-store` uses `#![forbid(unsafe_code)]` (compiler-enforced primary gate)            | compiler-enforced via the attribute; grep is at most an advisory smoke check                                |
| F-A6 ships no migrators (real or fixture) and `gbf-migrate/src/{dag,epochs,report}.rs` stay `//! Module stub.` | inspection of `git diff origin/main -- gbf-migrate/`; expected: zero changes |
| `BlobReferencesRegistry::empty()` is usable in tests                                  | `gbf-store::gc::pinset_protection` runs with empty registry plus `TreatAsLeaf`                              |
| `gbf-store` depends only on `gbf-foundation` (after closure)                           | `cargo tree -p gbf-store --depth 1` matches packet                                                          |
| `gbf-foundation::Hash256` exposes `from_bytes`/`as_bytes` accessors                    | type-level: `Hash256::from_bytes([u8; 32])` and `Hash256::as_bytes(&self) -> &[u8; 32]`                      |

## 20. References

### Internal

- `history/planv0.md` — line 137 (`gbf-foundation::BlobRef`), line 138 (`gbf-store` modules), line 139 (`gbf-migrate` modules), line 269 (`gbf-store` is the dedicated CAS crate), line 311 (`gbf-codegen` consumes `StageCache`), line 1126 (host-side migration logic lives in `gbf-migrate`), line 1504 (`CompatibilityEpochs`), line 1511 (`MigrationReport`), line 1519 (`MigrationLossClass`), line 1472 (`ComponentDigestSet`), line 1476 (`BuildShardManifest`), line 2160 (runtime `StateMigrator` is *not* `gbf-migrate`), line 2511 (two-level `StageCache`), line 2863 (always-on `StageCache` keys), line 2940 (engineering rule 20).
- `history/glossary.md` — uses existing terms (`Bead`, `Feature`, `Task`, `Contract`, `Owner`, `Build product`); introduces no new RFC vocabulary.
- `CONSTITUTION.md` — §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds), §V.3 (silence on success, loud on failure), §VI.1 (single source of truth).
- `bd-3ll` (F-A6 feature bead) and child tasks `bd-1jy`, `bd-2ab`, `bd-1e9`, `bd-1yd`, `bd-n9i`.
- F-A1 RFC `history/rfcs/F-A1-gbf-asm.md` — defines `PlanningStage`, the eventual concrete domain for `StageId`.
- F-A2 RFC `history/rfcs/F-A2-gbf-hw.md` — analogous "fill the stubs in a single PR" closure shape; F-A6 mirrors its structure.
- F-B15 task `bd-1g7k` (StageCache integration across all stages) — the first major consumer of T-A6.2.
- F-F1 task `bd-ow2e` (build reports — JSON schemas + emit hooks) — depends on `gbf-store` for blob storage.

### External

- POSIX `rename(2)` semantics: <https://pubs.opengroup.org/onlinepubs/9699919799/functions/rename.html>
- `std::fs::rename` documentation: <https://doc.rust-lang.org/std/fs/fn.rename.html>
- `tempfile` crate: <https://crates.io/crates/tempfile>
- `sha2` crate: <https://crates.io/crates/sha2>
- SHA-256 specification (FIPS 180-4): <https://nvlpubs.nist.gov/nistpubs/FIPS/NIST.FIPS.180-4.pdf>
- Content-addressable storage prior art (Git's object store): <https://git-scm.com/book/en/v2/Git-Internals-Git-Objects>

## 21. Appendix: file-by-file change set

| File                                          | Change                                                                                                            | Lines (est.) |
|-----------------------------------------------|-------------------------------------------------------------------------------------------------------------------|--------------|
| `gbf-foundation/src/blob.rs`                  | New (replace stub) — `BlobRef`, `BlobCodec`                                                                       | ~50          |
| `gbf-foundation/src/lib.rs`                   | Add `pub use blob::{BlobRef, BlobCodec};`                                                                          | +1           |
| `gbf-foundation/tests/blob.rs`                | New                                                                                                                | ~40          |
| `gbf-store/src/lib.rs`                        | Add `#![forbid(unsafe_code)]`, doc                                                                                 | +3           |
| `gbf-store/src/blob.rs`                       | New (replace stub) — `BlobStore`, atomic put/get, streaming, errors                                                | ~250         |
| `gbf-store/src/integrity.rs`                  | New (replace stub) — `verify_integrity`, `verify_all`, `IntegrityReport`, `IntegrityError`                         | ~80          |
| `gbf-store/src/stage_cache.rs`                | New (replace stub) — `StageKey`, `ComponentDigestSet`, `compose_key`, `StageCache`                                 | ~200         |
| `gbf-store/src/pinset.rs`                     | New (replace stub) — `PinsetName`, `Pinset`, `BlobReferences`                                                      | ~70          |
| `gbf-store/src/gc.rs`                         | New (replace stub) — `run_gc`, `GcOptions`, `GcReport`, `BlobReferencesRegistry`                                   | ~180         |
| `gbf-store/src/archive.rs`                    | New (replace stub) — `ArchiveHeader`, `create_archive`, `extract_archive`, `list_archive`                          | ~280         |
| `gbf-store/tests/blob.rs`                     | New                                                                                                                | ~200         |
| `gbf-store/tests/integrity.rs`                | New                                                                                                                | ~80          |
| `gbf-store/tests/stage_cache.rs`              | New                                                                                                                | ~150         |
| `gbf-store/tests/pinset.rs`                   | New                                                                                                                | ~70          |
| `gbf-store/tests/gc.rs`                       | New                                                                                                                | ~180         |
| `gbf-store/tests/archive.rs`                  | New                                                                                                                | ~200         |
| `gbf-store/tests/cross_module_conformance.rs` | New                                                                                                                | ~100         |
| `gbf-store/Cargo.toml`                        | Remove `gbf-artifact` dep; add `sha2`, `tempfile` ([dev]); move `serde_json` to `[dev-dependencies]`               | ~6           |
| ~~`gbf-migrate/src/lib.rs`~~                  | **DEFERRED to F-A6b — not modified by F-A6**                                                                       | 0            |
| ~~`gbf-migrate/src/epochs.rs`~~               | **DEFERRED to F-A6b — stays `//! Module stub.`**                                                                   | 0            |
| ~~`gbf-migrate/src/dag.rs`~~                  | **DEFERRED to F-A6b — stays `//! Module stub.`**                                                                   | 0            |
| ~~`gbf-migrate/src/report.rs`~~               | **DEFERRED to F-A6b — stays `//! Module stub.`**                                                                   | 0            |
| ~~`gbf-migrate/tests/epochs.rs`~~             | **DEFERRED to F-A6b — not created by F-A6**                                                                        | 0            |
| ~~`gbf-migrate/tests/dag.rs`~~                | **DEFERRED to F-A6b — not created by F-A6**                                                                        | 0            |
| ~~`gbf-migrate/tests/report.rs`~~             | **DEFERRED to F-A6b — not created by F-A6**                                                                        | 0            |
| ~~`gbf-migrate/Cargo.toml`~~                  | **DEFERRED to F-A6b — not modified by F-A6** (the `gbf-abi`/`gbf-artifact` cleanup ships with F-A6b)               | 0            |

**Total in scope for F-A6: ~2,400 LOC, ~58% of which is tests.** (~600 LOC of `gbf-migrate` work is deferred to F-A6b — see §0.0.0.)

## 22. Review packet requirements

The F-A6 PR ships with a **review packet** as a first-class artifact in the repository, alongside the implementation. The packet is authored *after* implementation (so it can describe real decisions, real surprises, and real measured costs rather than the RFC's predictions). This RFC specifies only the *topics* the engineer must cover in the packet; the packet's directory layout, file names, prose, diagrams, and exact contents are decided at packet-creation time once the implementation has shipped.

### 22.1 What the packet must let the reviewer do

A reviewer who is otherwise unfamiliar with F-A6 should be able to answer four questions in one sitting:

1. **Is the implementation correct?** — Including the riskiest invariants for this feature: atomic writes, content-addressed identity, `compose_key` determinism, archive byte stability. (Migration-DAG correctness is deferred to F-A6b — see §0.0.0.)
2. **Is it clear and maintainable?** — Including the dependency posture, `unsafe` posture, module decomposition, and the in-scope `gbf-store/Cargo.toml` cleanup.
3. **Are the riskiest invariants actually proved?** — By tests, types, or citations of filesystem semantics — not by prose alone.
4. **Can I reproduce every claimed output locally?** — Tests, generated artifacts, dependency reports, and any cross-crate workspace effects.

### 22.2 Required topics

The packet, however structured, must include sections covering at least:

- **Scope statement** — what is in scope for this PR, what is intentionally deferred, and (when the deferred path has an owner) which downstream feature/bead picks it up.
- **Reading order** — a recommended sequence: which file or topic to read first, which to read deeply, which to skim, which to ignore.
- **Diff disposition** — for every file in the PR diff, a one-line classification (deep review / boundary review / skim / mechanical re-export / generated / fixture / config). The list must be exhaustive over `gh pr diff --name-only`, including the modified `Cargo.toml` files.
- **Architecture brief** — how `gbf-store` decomposes, why each module is where it is, why `gbf-migrate` stays a stub (F-A6b deferral), and what the dependency direction is. Reuses material from §3 of this RFC.
- **Correctness dossier** — for each of the highest-risk surfaces of F-A6 (atomic writes, content-addressed determinism, `compose_key` determinism, archive byte stability, GC keep-set walk, the foundation `BlobRef` populate, the `gbf-store/Cargo.toml` cleanup), the packet records the invariant, the test or type or filesystem-semantics citation that proves it, and the failure mode if it ever drifts. (Migration-DAG planning is **out of scope** — deferred to F-A6b.)
- **Filesystem-semantics citation table** — every load-bearing filesystem property the implementation depends on, mapped to the implementation choice it justifies. The "documents resolve at the cited level" check is structural (e.g., POSIX function references resolve to a stable specification), not just HTTP 200.
- **Claim-to-gate matrix** — every load-bearing claim from §19 (and any new claims that surfaced during implementation) mapped to its gating test or artifact. New rows are added if the implementation discovered new invariants.
- **Test coverage report** — what `cargo test -p gbf-foundation -p gbf-store` runs, how it groups, what it asserts, and any portions deliberately not covered (with reasoning). The atomic-write crash test in particular deserves explicit treatment. (`gbf-migrate` is stub-only after F-A6 — confirm `cargo test -p gbf-migrate` runs zero tests.)
- **Reproducibility report** — the exact command set a reviewer runs to regenerate every checked-in artifact. One top-level script invocation should reproduce all of it.
- **Generated artifacts manifest** — what artifacts (test logs, cargo trees, dependency snapshots, sample serialized outputs, diagrams) ship with the packet, and a reproducibility-fingerprint per artifact so the reviewer can confirm the file matches what the script produces.
- **Dependency report** — the resolved dependency graph for `gbf-store` after closure, with explicit confirmation that no production-dep on any contract crate leaks in. Plus a note confirming `gbf-migrate/Cargo.toml` is unchanged (its existing wrong-direction deps remain until F-A6b).
- **Known-debt ledger** — every TODO, FIXME, deferred decision, or known-imperfect aspect introduced or carried by F-A6, with the bead/feature that owns the resolution.
- **Out-of-scope ledger** — items that look like F-A6's job but explicitly are not, each with the owning feature.
- **API guide** — the public surface of each module, the typed errors with their reproducible state, the trait shapes (`BlobReferences`, `Migrator`), the byte-stable archive format, and the breaking-vs-additive policy.
- **Reviewer checklist** — the binary questions the reviewer should be able to mark off (one for each load-bearing claim).
- **Cleanliness audit** — confirmation that `#![forbid(unsafe_code)]` is at both crate roots (the compiler-enforced attribute is the gate; grep is at most advisory because comments, docs, and dependency code may contain the string `unsafe`), that no async or coordination libraries were introduced, that no contract-crate dependencies leak in, and that the `Cargo.toml` cleanups are explicit in the diff.
- **Source-to-artifact traceability** — for at least one representative invariant (e.g., the archive magic + version, or `compose_key`'s length-prefix encoding), a worked example showing the spec source, the implementation, and the gating test.
- **Diagrams** — covering at minimum the directory layout, the StageKey two-level decomposition, the GC keep-set walk, the archive byte layout, and the atomic-write protocol with crash arrows. Mermaid sources plus rendered SVGs; the rendering is reproducible. (The migration-DAG planning diagram is deferred to F-A6b's packet.)
- **`gbf-store/Cargo.toml` cleanup evidence** — the specific diff lines that remove the placeholder `gbf-artifact` dep, plus a dependency-graph excerpt confirming the resulting transitive surface. (`gbf-migrate/Cargo.toml` is **not** modified by F-A6.)
- **`BlobRef` populate evidence** — the diff in `gbf-foundation/src/blob.rs`, the new re-export in `gbf-foundation/src/lib.rs`, and at least one example consumer.

The packet may add other sections that turn out to be useful at implementation time (e.g., a discussion of a cross-cutting decision that surfaced during coding), but it must not omit the topics above.

### 22.3 Reproducibility property

The packet contains a single top-level `verify-packet` script (or equivalent). Running it in a fresh checkout regenerates every artifact-the-packet-references and fails loudly if any checked-in artifact is stale relative to the current source. The exact script name, location, and output format are decided at packet-creation time; the contract is that one command suffices.

### 22.4 Acceptance bar

The packet is complete only when:

- a fresh-checkout reviewer can run the verify script, all tests pass, all reproducible artifacts match;
- every claim in §19 (claim-to-gate matrix) maps to a concrete gate (test, type, citation, or generated artifact);
- every file in the PR diff appears exactly once in the diff disposition table;
- the atomic-write protocol is documented (diagram + tests) and the diagram resolves to the actual implementation;
- the `gbf-foundation::blob` populate is explicitly documented and tested;
- the in-scope `gbf-store/Cargo.toml` cleanup is visible in the diff disposition table and in the dependency report; `gbf-migrate/Cargo.toml` is confirmed unchanged;
- the cleanliness audit shows zero introduced `unsafe` and zero contract-crate deps in `gbf-store`;
- known-debt and out-of-scope ledgers are present and entries point at owning beads or RFCs.

### 22.5 What the packet should explicitly not pre-commit to in *this* RFC

This RFC deliberately stops short of specifying packet directory structure, file names, README templates, exact diff-map columns, or the exact reviewer-question wording. The reasons:

- The packet describes the *implementation that actually shipped*. Pre-committing to a fixed structure here would force the implementation to match an a-priori shape that may not fit what was built.
- Implementation decisions (e.g., a redesign of an error variant, a reorganisation of test fixtures, an additive helper that didn't exist when this RFC was drafted) need to be reflected in the packet without first amending this RFC.
- The packet is itself a deliverable; locking its structure here makes the RFC into a packet template, which is the wrong level of abstraction.

The reviewer asks under §23 are part of *this* RFC's deliverable, not the packet's; they remain in this document.

## 23. End

This RFC stays inside the (re-scoped) F-A6 boundary: **`gbf-store` only**. The entire `gbf-migrate` scaffolding (epochs, `Migrator` trait, `MigrationGraph`, `MigrationReport`, `MigrationLossClass`, identity/bump-minor migrator fixtures) is **deferred** to a follow-up bead (provisional **F-A6b**) that opens when the first real schema bump demands it (see §0.0.0). Anything that requires F-B15's stage wiring, real schema migrators, async I/O, multi-process coordination, the runtime SRAM `StateMigrator`, or `gbf-cli` front-ends is also explicitly deferred. The proposal lets F-A6 close without `gbf-migrate` work existing, while leaving every seam (`BlobRef`/`BlobCodec` in foundation, `BlobStore::put_as` for tagging codec, `StageKey`'s shard-local + global split, the `BlobReferences` trait) shaped for the eventual deferred bead to plug in cleanly. Sections §§ 11–13 are preserved as design notes so F-A6b inherits a finished design rather than a blank page.

Reviewer asks I would value most:

1. Is the **atomic-write protocol** correct and minimal? Specifically: is `tmp/<random>` → `fsync` → `rename` (without `fsync`-on-parent-dir) sufficient for single-process crash safety, given that consumers within the same machine read after we return from `put`? Cross-check with a filesystem expert is welcome.
2. Is the **`compose_key` canonical encoding** right? In particular, length-prefixed UTF-8 for `StageId` / `ComponentId` / `FeatureFlag`, `BTreeMap`/`BTreeSet` iteration for sets, fixed little-endian for integers, raw bytes for `Hash256`. Are there any structural cases where two distinct `StageKey`s could collide?
3. Is the **archive format** at the right level of detail? Magic + version + 24-byte header + pinset table + blob table; deterministic blob order by `Hash256`; little-endian integers. Is there a missing field (e.g., compression of the archive itself) that should be in v1?
4. Is the **GC keep-set walk** the right shape? Specifically: passing `BlobReferencesRegistry` as a parameter (not a global) and treating unknown formats conservatively (no transitive walk for them). The alternative (require every consumer to register a format reader before GC) is stricter but more correct.
5. Is the **`gbf-migrate` deferral** correctly scoped? The motivation in §0.0.0 is "zero versioned schemas exist today, wipe-and-rebuild is the cheapest policy until a real schema bump forces the issue." Is there any consumer that would be silently broken by leaving `gbf-migrate` stubbed (i.e., should F-A6b be sooner than the first artifact bump)?
6. Is the **`BlobRef` populate** in the right place? It lives in `gbf-foundation::blob` per planv0; the alternative (`gbf-store::BlobRef`) would force every contract crate to depend on `gbf-store`. Cross-check the architectural intent welcome.
7. Is **closing the `gbf-store` → `gbf-artifact` dep** correct? The current scaffold has it; this RFC removes it on the theory that store should not know about any specific schema. Is there any consumer pattern I'm missing where store would legitimately need an artifact type?

If those land cleanly, F-A6 closes and unblocks F-B15 (StageCache integration) and F-F1 (build reports). Future schema-bump beads remain blocked on F-A6b (deferred).

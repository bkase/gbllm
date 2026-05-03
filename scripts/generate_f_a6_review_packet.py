#!/usr/bin/env python3
"""Generate and verify the F-A6 gbf-store review packet.

Regenerate:
    python3 scripts/generate_f_a6_review_packet.py

Verify packet staleness and optional gates:
    python3 scripts/generate_f_a6_review_packet.py --check --run-tests
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from textwrap import dedent


ROOT = Path(__file__).resolve().parents[1]
PACKET = ROOT / "history" / "review-packets" / "F-A6"
DIAGRAMS = PACKET / "diagrams"

EXPECTED_CHANGED_FILES = [
    ".beads/issues.jsonl",
    "Cargo.lock",
    "Cargo.toml",
    "gbf-foundation/src/blob.rs",
    "gbf-foundation/src/lib.rs",
    "gbf-foundation/tests/blob.rs",
    "gbf-store/Cargo.toml",
    "gbf-store/src/archive.rs",
    "gbf-store/src/blob.rs",
    "gbf-store/src/gc.rs",
    "gbf-store/src/integrity.rs",
    "gbf-store/src/lib.rs",
    "gbf-store/src/pinset.rs",
    "gbf-store/src/stage_cache.rs",
    "gbf-store/tests/f_a6.rs",
    "history/review-packets/F-A6/README.md",
    "history/review-packets/F-A6/artifact-manifest.json",
    "history/review-packets/F-A6/claim-to-gate.md",
    "history/review-packets/F-A6/dependency-report.md",
    "history/review-packets/F-A6/diagrams/archive-layout.mmd",
    "history/review-packets/F-A6/diagrams/archive-layout.svg",
    "history/review-packets/F-A6/diagrams/atomic-write.mmd",
    "history/review-packets/F-A6/diagrams/atomic-write.svg",
    "history/review-packets/F-A6/diagrams/directory-layout.mmd",
    "history/review-packets/F-A6/diagrams/directory-layout.svg",
    "history/review-packets/F-A6/diagrams/gc-keep-walk.mmd",
    "history/review-packets/F-A6/diagrams/gc-keep-walk.svg",
    "history/review-packets/F-A6/diagrams/stage-key.mmd",
    "history/review-packets/F-A6/diagrams/stage-key.svg",
    "history/review-packets/F-A6/reviewer-checklist.md",
    "scripts/generate_f_a6_review_packet.py",
]


DIAGRAM_MMD = {
    "directory-layout": """\
flowchart TB
  root["store root"]
  root --> blobs["blobs/sha256"]
  blobs --> shard["ab/"]
  shard --> blob["ab...64 hex content hash"]
  root --> tmp["tmp/blob-*.tmp"]
  root --> cache["cache/"]
  cache --> cshard["cd/"]
  cshard --> index["cd...64 hex stage-cache key"]
  index --> payload["file body: payload Hash256"]
""",
    "stage-key": """\
flowchart LR
  stage["StageId"] --> canon["canonical length-prefixed bytes"]
  local["ComponentDigestSet (BTreeMap)"] --> canon
  global["global Hash256"] --> canon
  flags["FeatureFlag set (BTreeSet)"] --> canon
  version["SemVer as u32 triplet"] --> canon
  canon --> digest["StageCacheKey"]
  digest --> index["cache/<prefix>/<key>"]
  index --> blob["payload Hash256 in BlobStore"]
""",
    "gc-keep-walk": """\
flowchart TD
  pinsets["Pinset roots"] --> queue["work queue"]
  queue --> load["BlobStore::get"]
  load --> refs["BlobReferencesRegistry"]
  refs --> children["child Hash256 values"]
  children --> queue
  load --> keep["keep set"]
  all["BlobStore::list_blobs"] --> candidates["all - keep"]
  candidates --> sweep["remove in Hash256 order"]
  sweep --> indexes["optional stage-cache index sweep"]
""",
    "archive-layout": """\
flowchart TB
  header["24-byte header: magic, version, counts, total_bytes"]
  pinsets["pinset table: name, annotation, roots"]
  records["blob records in Hash256 order"]
  record["hash[32] | len[u64 LE] | body[len]"]
  header --> pinsets --> records --> record
""",
    "atomic-write": """\
sequenceDiagram
  participant Caller
  participant Temp as tmp/blob-*.tmp
  participant Final as blobs/sha256/ab/hash
  Caller->>Temp: write bytes
  Caller->>Temp: sync_all
  Temp->>Final: rename
  alt existing final path
    Caller->>Final: stream-verify existing hash
  end
  opt DurabilityMode::Full
    Caller->>Final: best-effort parent directory sync
  end
""",
}


def main() -> int:
    args = parse_args()
    files = build_files()
    if args.check:
        failures = check_files(files)
        failures.extend(check_changed_files())
        if args.run_tests:
            failures.extend(run_verification_commands())
        if failures:
            print("F-A6 review packet check failed:", file=sys.stderr)
            for failure in failures:
                print(f"- {failure}", file=sys.stderr)
            return 1
        print("F-A6 review packet check passed")
        return 0

    write_files(files)
    print(f"wrote {len(files)} F-A6 review packet files")
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if generated files are stale")
    parser.add_argument(
        "--run-tests",
        action="store_true",
        help="with --check, run the packet's cargo/test verification commands",
    )
    return parser.parse_args()


def build_files() -> dict[Path, str]:
    tree = run(["cargo", "tree", "-p", "gbf-store", "--depth", "1"])
    files: dict[Path, str] = {}
    files[PACKET / "README.md"] = readme()
    files[PACKET / "claim-to-gate.md"] = claim_to_gate()
    files[PACKET / "dependency-report.md"] = dependency_report(tree)
    files[PACKET / "reviewer-checklist.md"] = reviewer_checklist()
    for name, source in DIAGRAM_MMD.items():
        files[DIAGRAMS / f"{name}.mmd"] = source
        files[DIAGRAMS / f"{name}.svg"] = svg_render(name, source)

    manifest = {
        str(path.relative_to(ROOT)): sha256_text(text)
        for path, text in sorted(files.items(), key=lambda item: str(item[0]))
    }
    files[PACKET / "artifact-manifest.json"] = (
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    return files


def readme() -> str:
    disposition = "\n".join(
        f"| `{path}` | {risk_for(path)} | {disposition_for(path)} | {gate_for(path)} |"
        for path in EXPECTED_CHANGED_FILES
    )
    return clean(
        dedent(
        f"""\
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
        {disposition}

        ## Architecture Brief

        `gbf-store` is intentionally filesystem-shaped infrastructure. Blob bytes live under `blobs/sha256/<prefix>/<hash>`. The stage cache is a weak index layered over the blob store: cache files map `StageCacheKey` to payload `Hash256`, but payload bytes still live in the CAS. Pinsets are explicit GC roots. GC walks pinsets transitively through `BlobReferencesRegistry`, then removes unpinned blobs in deterministic hash order. Archives carry pinsets plus sorted content-addressed blob records for cross-machine transport.

        Dependency direction is one-way: `gbf-store` depends on `gbf-foundation`, `serde`, `sha2`, and `tempfile`; it does not depend on `gbf-artifact`, `gbf-abi`, `gbf-codegen`, or any contract/product crate. `gbf-migrate` stays unchanged because no production schema bump exists yet.

        ## Correctness Dossier

        - Atomic writes: `BlobStore::commit_bytes` writes `tmp/blob-*.tmp`, syncs the file, then renames into the canonical path. `DurabilityMode::Full` adds best-effort parent-directory sync. Gates: `blob::atomic_writes`, `blob::open_preserves_tmp_files`, `blob::idempotent_verifies_existing`.
        - Content-addressed determinism: `put` hashes bytes before commit; `put_expect` rejects mismatches before commit; existing canonical files are stream-verified before reuse. Gates: `blob::round_trip`, `blob::content_addressed_idempotent`, `blob::put_expect_hash_mismatch`, `integrity::*`.
        - `BlobRef` populate: `gbf-foundation::blob::BlobRef {{ hash, len, codec }}` is serde-tested and consumed by `BlobStore::put_as` / `get_ref`. Gates: `gbf-foundation/tests/blob.rs`, `blob::put_as_returns_blob_ref`, `blob::get_ref_validates_len`.
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
        | RFC F-A6 §10: magic `GBLM\\0ARC`, version 1, 24-byte header | `ARCHIVE_MAGIC`, `ARCHIVE_VERSION`, `ARCHIVE_HEADER_LEN`, `write_header`, `read_header` | `archive::magic_bytes`, `archive::version_byte`, `archive::header_size_24_bytes` |

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
        """
        )
    )


def claim_to_gate() -> str:
    rows = [
        ("BlobRef has hash/len/codec and serde-stable codec names", "gbf-foundation/tests/blob.rs::blob_ref_serde_round_trip + blob_codec_serde_snake_case"),
        ("gbf-store has no local unsafe", "gbf-store/src/lib.rs contains #![forbid(unsafe_code)] and cargo clippy passes"),
        ("BlobStore writes are content-addressed and idempotent", "blob::round_trip + blob::content_addressed_idempotent"),
        ("put_expect rejects mismatched claimed hashes before commit", "blob::put_expect_hash_mismatch + archive::extract_rejects_record_exceeding_declared_total_before_commit"),
        ("Canonical existing files are rehashed before idempotent reuse", "blob::idempotent_verifies_existing"),
        ("BlobRef length is validated on read", "blob::get_ref_validates_len"),
        ("Integrity check detects missing and corrupt blobs", "integrity::verify_missing_blob + integrity::verify_corrupt_blob"),
        ("StageCache compose_key is deterministic across construction order", "stage_cache::deterministic_keys"),
        ("StageCache key encoding is boundary-safe and SemVer checked", "stage_cache::compose_key_length_prefix_safe + pass_version_component_overflow_rejected"),
        ("StageCache stale payloads are cache misses", "stage_cache::stale_index_treated_as_miss"),
        ("Pinset names reject path-shaped names", "pinset::name_validation_rejects_bad_forms + name_validation_rejects_leading_dot"),
        ("GC protects pinsets and walks transitive references", "gc::pinset_protection + gc::transitive_refs_via_registry"),
        ("GC default unknown-reference policy aborts", "gc::unknown_reference_policy_abort"),
        ("GC dry-run and removal limits are deterministic", "gc::dry_run_populates_candidates + gc::max_remove_per_run_honored + removal_order_is_hash_ascending"),
        ("GC sweeps only indexes for blobs removed in the run", "gc::sweep_stage_cache_indexes_removes_stale + sweep_stage_cache_indexes_respects_remove_limit"),
        ("Archive header is byte-stable", "archive::magic_bytes + version_byte + header_size_24_bytes"),
        ("Archive bytes are deterministic for equivalent inputs", "archive::deterministic_bytes + blobs_sorted_by_hash + pinsets_sorted_by_name_internally"),
        ("Archive extraction is hash-checked and non-transactional", "archive::extract_validates_each_record_hash + extract_not_transactional"),
        ("Archive rejects malformed accounting before commit", "archive::extract_rejects_record_exceeding_declared_total_before_commit + total_bytes_mismatch_rejected + trailing_bytes_rejected"),
        ("gbf-store Cargo cleanup removes gbf-artifact", "cargo tree -p gbf-store --depth 1 in dependency-report.md"),
        ("gbf-migrate is untouched and deferred", "git diff --exit-code -- gbf-migrate + cargo test -p gbf-migrate"),
    ]
    body = "\n".join(f"| {claim} | {gate} |" for claim, gate in rows)
    return clean(
        dedent(
        f"""\
        # F-A6 Claim-To-Gate Matrix

        | Claim | Gate |
        | --- | --- |
        {body}
        """
        )
    )


def dependency_report(tree: str) -> str:
    return clean(
        dedent(
        f"""\
        # F-A6 Dependency Report

        Resolved dependency graph:

        ```text
        {tree}
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
        """
        )
    )


def reviewer_checklist() -> str:
    checks = [
        "The PR touches no `gbf-migrate/` paths.",
        "Every file in `gh pr diff --name-only` appears exactly once in the Changed File Disposition table.",
        "`gbf-store/src/lib.rs` forbids unsafe code.",
        "`gbf-store/Cargo.toml` has no `gbf-artifact` dependency.",
        "`BlobRef` is implemented in foundation and consumed by `BlobStore::put_as`.",
        "`BlobStore::put_expect` rejects a bad claimed hash before commit.",
        "Existing canonical blobs are stream-verified before reuse.",
        "`StageKey` construction order does not affect `compose_key`.",
        "`StageCacheKey` is distinct from content `Hash256` at the API boundary.",
        "GC default unknown-reference behavior is abort, not leaf.",
        "Dry-run GC reports candidates without removing blobs or indexes.",
        "Archive creation is deterministic and preflights source hash integrity.",
        "Archive extraction rejects over-budget records before committing them.",
        "The review packet generator passes with `--check --run-tests`.",
    ]
    body = "\n".join(f"- [ ] {check}" for check in checks)
    return f"# F-A6 Reviewer Checklist\n\n{body}\n"


def risk_for(path: str) -> str:
    if path.endswith((".rs", "Cargo.toml")) and "review-packets" not in path:
        return "High" if "gbf-store/src" in path or "tests/f_a6" in path else "Medium"
    if path.endswith(".jsonl"):
        return "Medium"
    if path.endswith((".svg", ".mmd", ".json", ".md")):
        return "Low"
    return "Medium"


def disposition_for(path: str) -> str:
    if path == ".beads/issues.jsonl":
        return "Generated bead metadata; scope/status source-of-truth updates."
    if path in {"Cargo.toml", "Cargo.lock", "gbf-store/Cargo.toml"}:
        return "Config/dependency review."
    if path == "gbf-foundation/src/lib.rs":
        return "Mechanical re-export."
    if path == "gbf-foundation/src/blob.rs":
        return "Boundary review for durable BlobRef shape."
    if path.startswith("gbf-store/src/"):
        return "Deep implementation review."
    if path.endswith("tests/f_a6.rs") or path.endswith("tests/blob.rs"):
        return "Fixture and proof review."
    if path.startswith("history/review-packets/F-A6/diagrams/"):
        return "Generated diagram artifact."
    if path.startswith("history/review-packets/F-A6/"):
        return "Generated review packet artifact."
    if path.startswith("scripts/"):
        return "Review-packet reproducibility script."
    return "Skim."


def gate_for(path: str) -> str:
    if path.startswith("gbf-store/src/") or path == "gbf-store/tests/f_a6.rs":
        return "cargo test -p gbf-store"
    if path.startswith("gbf-foundation/"):
        return "cargo test -p gbf-foundation"
    if path in {"Cargo.toml", "Cargo.lock", "gbf-store/Cargo.toml"}:
        return "cargo tree -p gbf-store --depth 1"
    if path == ".beads/issues.jsonl":
        return "br sync --flush-only + beads_pr_scope_check"
    if path.startswith("history/review-packets/") or path.startswith("scripts/"):
        return "python3 scripts/generate_f_a6_review_packet.py --check"
    return "manual review"


def svg_render(name: str, source: str) -> str:
    lines = [name.replace("-", " ").title()] + [
        line.strip().replace('"', "'") for line in source.splitlines() if line.strip()
    ]
    width = 980
    height = 40 + 24 * len(lines)
    text = "\n".join(
        f'  <text x="24" y="{34 + index * 24}" font-family="monospace" '
        f'font-size="{18 if index == 0 else 14}">{escape_xml(line)}</text>'
        for index, line in enumerate(lines)
    )
    return dedent(
        f"""\
        <svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
          <rect x="0" y="0" width="{width}" height="{height}" fill="#ffffff"/>
          <rect x="12" y="12" width="{width - 24}" height="{height - 24}" fill="#f8fafc" stroke="#334155" stroke-width="1"/>
        {text}
        </svg>
        """
    )


def escape_xml(value: str) -> str:
    return (
        value.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
    )


def clean(text: str) -> str:
    return text.replace("\n        ", "\n").lstrip()


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def write_files(files: dict[Path, str]) -> None:
    for path, text in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")


def check_files(files: dict[Path, str]) -> list[str]:
    failures: list[str] = []
    for path, expected in files.items():
        if not path.exists():
            failures.append(f"missing generated file {path.relative_to(ROOT)}")
            continue
        actual = path.read_text(encoding="utf-8")
        if actual != expected:
            diff = "\n".join(
                difflib.unified_diff(
                    actual.splitlines(),
                    expected.splitlines(),
                    fromfile=f"{path.relative_to(ROOT)} (actual)",
                    tofile=f"{path.relative_to(ROOT)} (expected)",
                    lineterm="",
                )
            )
            failures.append(f"stale generated file {path.relative_to(ROOT)}\n{diff}")
    return failures


def check_changed_files() -> list[str]:
    changed = pr_changed_files()
    if not changed:
        changed = EXPECTED_CHANGED_FILES
    expected = sorted(EXPECTED_CHANGED_FILES)
    actual = sorted(changed)
    if actual == expected:
        return []
    return [
        "changed-file table mismatch: expected "
        + ", ".join(expected)
        + "; actual "
        + ", ".join(actual)
    ]


def pr_changed_files() -> list[str]:
    try:
        result = subprocess.run(
            ["git", "diff", "--name-only", "origin/main...HEAD"],
            cwd=ROOT,
            check=True,
            text=True,
            capture_output=True,
        )
    except subprocess.CalledProcessError:
        return []
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def run_verification_commands() -> list[str]:
    commands = [
        ["cargo", "test", "-p", "gbf-foundation", "-p", "gbf-store"],
        ["cargo", "test", "-p", "gbf-migrate"],
        ["cargo", "test", "-p", "gbf-store", "--doc"],
        ["cargo", "clippy", "-p", "gbf-store", "--all-features", "--tests", "--", "-D", "warnings"],
        ["git", "diff", "--exit-code", "--", "gbf-migrate"],
    ]
    failures: list[str] = []
    for cmd in commands:
        try:
            subprocess.run(cmd, cwd=ROOT, check=True)
        except subprocess.CalledProcessError as exc:
            failures.append(f"command failed ({exc.returncode}): {' '.join(cmd)}")
    return failures


def run(cmd: list[str]) -> str:
    result = subprocess.run(cmd, cwd=ROOT, check=True, text=True, capture_output=True)
    return result.stdout.strip()


if __name__ == "__main__":
    raise SystemExit(main())

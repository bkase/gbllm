#!/usr/bin/env python3
"""Generate the F-A3 gbf-abi review packet.

The RFC requires the packet to be reproducible from one command. Run:

    python3 scripts/generate_f_a3_review_packet.py

Use --check in CI/review to fail if checked-in artifacts are stale.
"""

from __future__ import annotations

import argparse
import difflib
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PACKET = ROOT / "history" / "review-packets" / "F-A3"


LAYOUTS = [
    {
        "type": "AbiVersion",
        "module": "version",
        "size": 3,
        "align": 1,
        "rom_resident": True,
        "fields": [
            {"name": "major", "offset": 0, "size": 1},
            {"name": "minor", "offset": 1, "size": 1},
            {"name": "patch", "offset": 2, "size": 1},
        ],
    },
    {
        "type": "BuildIdentityBlock",
        "module": "version",
        "size": 152,
        "align": 8,
        "rom_resident": True,
        "fields": [
            {"name": "magic", "offset": 0, "size": 4},
            {"name": "abi", "offset": 4, "size": 3},
            {"name": "_reserved0", "offset": 7, "size": 1},
            {"name": "build_hash", "offset": 8, "size": 32},
            {"name": "artifact_core_hash", "offset": 40, "size": 32},
            {"name": "runtime_nucleus_hash", "offset": 72, "size": 32},
            {"name": "compile_request_hash", "offset": 104, "size": 32},
            {"name": "timestamp_unix", "offset": 136, "size": 8},
            {"name": "continuation_tail_bytes", "offset": 144, "size": 4},
            {"name": "semantic_schema_version", "offset": 148, "size": 2},
            {"name": "_reserved1", "offset": 150, "size": 2},
        ],
    },
    {
        "type": "LivenessCounters",
        "module": "liveness",
        "size": 12,
        "align": 4,
        "rom_resident": True,
        "fields": [
            {"name": "progress_epoch", "offset": 0, "size": 4},
            {"name": "last_checkpoint", "offset": 4, "size": 2},
            {"name": "no_progress_frames", "offset": 6, "size": 2},
            {"name": "livelock_threshold_frames", "offset": 8, "size": 2},
            {"name": "_reserved", "offset": 10, "size": 2},
        ],
    },
    {
        "type": "InferenceStateHeader",
        "module": "continuation",
        "size": 32,
        "align": 4,
        "rom_resident": True,
        "fields": [
            {"name": "abi", "offset": 0, "size": 3},
            {"name": "_reserved0", "offset": 3, "size": 1},
            {"name": "schema", "offset": 4, "size": 2},
            {"name": "last_fault", "offset": 6, "size": 2},
            {"name": "session_id", "offset": 8, "size": 4},
            {"name": "token_count", "offset": 12, "size": 4},
            {"name": "slice_id", "offset": 16, "size": 4},
            {"name": "liveness", "offset": 20, "size": 12},
        ],
    },
    {
        "type": "FaultCodeOptional",
        "module": "continuation",
        "size": 2,
        "align": 2,
        "rom_resident": True,
        "fields": [{"name": "raw", "offset": 0, "size": 2}],
    },
    {
        "type": "RegisterSnapshot",
        "module": "fault",
        "size": 10,
        "align": 2,
        "rom_resident": True,
        "fields": [
            {"name": "a", "offset": 0, "size": 1},
            {"name": "f", "offset": 1, "size": 1},
            {"name": "b", "offset": 2, "size": 1},
            {"name": "c", "offset": 3, "size": 1},
            {"name": "d", "offset": 4, "size": 1},
            {"name": "e", "offset": 5, "size": 1},
            {"name": "h", "offset": 6, "size": 1},
            {"name": "l", "offset": 7, "size": 1},
            {"name": "sp", "offset": 8, "size": 2},
        ],
    },
    {
        "type": "FaultSnapshot",
        "module": "fault",
        "size": 36,
        "align": 4,
        "rom_resident": True,
        "fields": [
            {"name": "code", "offset": 0, "size": 2},
            {"name": "domain", "offset": 2, "size": 2},
            {"name": "at_pc", "offset": 4, "size": 2},
            {"name": "at_bank", "offset": 6, "size": 2},
            {"name": "at_checkpoint", "offset": 8, "size": 2},
            {"name": "_resv", "offset": 10, "size": 2},
            {"name": "regs", "offset": 12, "size": 10},
            {"name": "_resv1", "offset": 22, "size": 2},
            {"name": "liveness", "offset": 24, "size": 12},
        ],
    },
    {
        "type": "HarnessCommandBlock",
        "module": "harness",
        "size": 44,
        "align": 4,
        "rom_resident": True,
        "fields": [
            {"name": "magic", "offset": 0, "size": 4},
            {"name": "seq", "offset": 4, "size": 4},
            {"name": "op", "offset": 8, "size": 2},
            {"name": "doorbell", "offset": 10, "size": 1},
            {"name": "_resv", "offset": 11, "size": 1},
            {"name": "args", "offset": 12, "size": 32},
        ],
    },
    {
        "type": "HarnessResultBlock",
        "module": "harness",
        "size": 44,
        "align": 4,
        "rom_resident": True,
        "fields": [
            {"name": "magic", "offset": 0, "size": 4},
            {"name": "seq", "offset": 4, "size": 4},
            {"name": "kind", "offset": 8, "size": 2},
            {"name": "ready", "offset": 10, "size": 1},
            {"name": "_resv", "offset": 11, "size": 1},
            {"name": "data", "offset": 12, "size": 32},
        ],
    },
    {
        "type": "TraceEvent",
        "module": "trace",
        "size": 32,
        "align": 4,
        "rom_resident": True,
        "fields": [
            {"name": "seq", "offset": 0, "size": 4},
            {"name": "timestamp_m_cycles", "offset": 4, "size": 4},
            {"name": "slice", "offset": 8, "size": 4},
            {"name": "probe", "offset": 12, "size": 2},
            {"name": "checkpoint", "offset": 14, "size": 2},
            {"name": "data", "offset": 16, "size": 16},
        ],
    },
]


CLAIMS = [
    ("CURRENT_ABI is non-zero", "version::current_constant_set"),
    ("AbiVersion ordering is lexicographic", "version::ord_total"),
    ("AbiVersion converts to/from bounded SemVer", "version::semver_round_trip"),
    ("BuildIdentityBlock is 152 bytes with pinned offsets", "version::build_identity_layout + version::build_identity_offsets"),
    ("BuildIdentityBlock constructors stamp magic and zero reserved bytes", "version::build_identity_constructor_sets_magic + version::build_identity_constructor_zeroes_reserved"),
    ("BuildIdentityBlock rejects bad magic/nonzero reserved bytes", "version::build_identity_validate_rejects_bad_magic + version::build_identity_validate_rejects_nonzero_reserved"),
    ("BuildIdentityBlock byte parser round-trips", "version::build_identity_from_bytes_round_trip"),
    ("CompatibilityEnvelope rejects self-reference and invalid ranges", "version::compatibility_envelope_no_self + version::compatibility_envelope_validate"),
    ("Liveness counters saturate and threshold at >= unless disabled by zero", "liveness::progress_advance + liveness::idle_frames_saturate + liveness::progress_epoch_saturates + liveness::livelock_threshold_zero_disables + liveness::livelock_threshold_fires_at_eq + property::record_progress_then_idle_frame_property"),
    ("InferenceStateHeader is 32 bytes with pinned offsets and split tail sizing", "continuation::header_layout + continuation::split_header_tail_validates_size + property::continuation_header_from_to_bytes_round_trip_property"),
    ("Harness op/result discriminants are covered and reject unknowns", "harness::op_kind_complete + harness::op_from_u16_rejects_unknown + harness::result_kind_from_u16_rejects_unknown"),
    ("Harness blocks are 44-byte magic/reserved/signal/seq-checked control blocks", "harness::layout + harness::constructor_sets_magic + harness::constructor_stages_signals_clear + harness::validate_rejects_bad_magic + harness::validate_rejects_invalid_signal_value + harness::seq_mismatch_rejected"),
    ("FaultCode maps totally into FaultDomain ranges", "fault::all_unique_discriminants + fault::code_to_domain_total + fault::range_partition"),
    ("FaultSnapshot captures registers and liveness in a 36-byte layout", "fault::register_snapshot_layout + fault::snapshot_layout + fault::snapshot_domain_matches_code + property::fault_snapshot_from_to_bytes_round_trip_property"),
    ("FaultPolicy default action cannot be BootValidationOnly", "fault::policy_default_action_validation + fault::policy_action_for_falls_back_to_default"),
    ("ResourceLeaseKind yield safety table is pinned", "interrupt::lease_yield_safety_table"),
    ("ResourceLease active/balanced semantics use Option<SliceId>", "interrupt::active_predicate + interrupt::balanced_predicate"),
    ("SemanticCheckpointId parser rejects malformed ids", "checkpoint::semantic_id_validation_basic + checkpoint::*rejects*"),
    ("CompactCheckpointId(0) is reserved in schemas", "checkpoint::compact_none_sentinel + checkpoint::schema_rejects_compact_zero"),
    ("SemanticCheckpointSchema enforces nonzero schema version, unique compact/semantic ids, and resolves both ways", "checkpoint::schema_rejects_zero_schema_version + checkpoint::schema_validates_unique_compact + checkpoint::schema_validates_unique_semantic + checkpoint::schema_resolve_round_trip + property::schema_resolve_round_trip_property"),
    ("TraceEvent is exactly 32 bytes with pinned offsets", "trace::event_layout"),
    ("TraceBudget rejects inconsistent zero/nonzero settings", "trace::trace_budget_constructor_rejects_inconsistent + trace::trace_budget_constructor_accepts_zero_zero"),
    ("Trace probe/drop enums are exhaustive", "trace::probe_level_exhaustive + trace::probe_budget_class_exhaustive + trace::drop_policy_exhaustive"),
    ("gbf-abi forbids local unsafe", "src/lib.rs contains #![forbid(unsafe_code)]; grep -R \"unsafe\" gbf-abi/src only finds that lint"),
]


def run(cmd: list[str]) -> str:
    result = subprocess.run(cmd, cwd=ROOT, check=True, text=True, capture_output=True)
    return result.stdout.strip()


def normalized_cargo_tree() -> str:
    tree = run(["cargo", "tree", "-p", "gbf-abi", "--edges", "normal", "--features", "host"])
    return tree.replace(str(ROOT), ".")


def toolchain_report() -> str:
    return run(["rustc", "-Vv"]) + "\n" + run(["cargo", "-V"])


def markdown_table(headers: list[str], rows: list[tuple[str, ...]]) -> str:
    out = ["| " + " | ".join(headers) + " |"]
    out.append("| " + " | ".join("---" for _ in headers) + " |")
    out.extend("| " + " | ".join(row) + " |" for row in rows)
    return "\n".join(out)


def render_readme() -> str:
    diff_rows = [
        ("AGENTS.md", "Low", "Adds Gemini --skip-trust review rule requested during F-A3.", "n/a"),
        ("gbf-abi/Cargo.toml", "Medium", "Defines host/alloc/std features and keeps serde no-default for ABI builds.", "cargo check/test feature matrix"),
        ("gbf-abi/src/version.rs", "High", "ABI version and ROM build identity handshake.", "version::*"),
        ("gbf-abi/src/checkpoint.rs", "High", "Durable semantic ids and build-local compact schema.", "checkpoint::*"),
        ("gbf-abi/src/continuation.rs", "High", "32-byte continuation prefix and opaque tail helpers.", "continuation::*"),
        ("gbf-abi/src/liveness.rs", "High", "Non-optional liveness state and saturation semantics.", "liveness::*"),
        ("gbf-abi/src/fault.rs", "High", "Pinned fault ranges, snapshots, and host recovery policy.", "fault::*"),
        ("gbf-abi/src/interrupt.rs", "Medium", "Lease/interrupt validation vocabulary for F-A4/F-B11.", "interrupt::*"),
        ("gbf-abi/src/harness.rs", "High", "Bidirectional harness control-plane blocks.", "harness::*"),
        ("gbf-abi/src/trace.rs", "High", "32-byte trace events, budgets, and probe registry shell.", "trace::*"),
        ("gbf-abi/tests/property.rs", "Medium", "Deterministic RFC property tests for byte round trips, liveness, and checkpoint resolution.", "property::*"),
        ("scripts/generate_f_a3_review_packet.py", "Medium", "Single-command packet regeneration/staleness gate.", "python3 scripts/generate_f_a3_review_packet.py --check"),
    ]

    return f"""# F-A3 Review Packet: gbf-abi

## Orientation

This packet pre-digests RFC `history/rfcs/F-A3-gbf-abi.md` for feature bead `bd-2k2` and child beads `bd-2ul`, `bd-2qx`, `bd-2m8`, `bd-1si`, `bd-30s`, `bd-1ml`, and `bd-34v`.

Recommended pass order:

1. Read the scope and non-goals below.
2. Check `layout-report.json` against the `repr(C)` source definitions and targeted layout tests.
3. Read `claim-to-gate.md` to map every load-bearing claim to tests.
4. Inspect `dependency-report.md` for the no-upward-dependency invariant.
5. Use `reviewer-checklist.md` as the final approval pass.

## Scope Ledger

In scope:

- `AbiVersion`, host `CompatibilityEnvelope`, and 152-byte `BuildIdentityBlock`.
- `InferenceStateHeader`, `FaultCodeOptional`, and `LivenessCounters`.
- `HarnessCommandBlock` / `HarnessResultBlock` with raw discriminant decoding.
- `FaultCode`, `FaultDomain`, `FaultSnapshot`, host `FaultPolicy`, and `BootValidationPlan`.
- `InterruptPolicy`, `ResourceLeaseKind`, and `ResourceLease` validation/report vocabulary.
- `SemanticCheckpointId`, `CompactCheckpointId`, `SemanticCheckpointSchema`, and `CheckpointResolver`.
- `TraceEvent`, trace budget/drop policy, and host `TraceProbeRegistry`.
- Targeted layout tests, serde round trips, deterministic property tests, negative discriminant/magic/reserved tests, and no local unsafe.

Out of scope and owners:

- Runtime persistence protocol: F-D1/T-A6.x. F-A3 guard: only identity/fault field shapes exist here.
- Full BankLease/BankGuard runtime API: F-A4. F-A3 guard: `ResourceLeaseKind` is vocabulary, not a call ABI.
- Semantic checkpoint schema production: F-F1/gbf-codegen. F-A3 guard: schema validates and resolves; it does not mint ids.
- Trace transport and payload schemas: F-D3. F-A3 guard: `TraceEvent` has fixed bytes; payload tags are named only.
- Emulator glue: F-H3. F-A3 guard: explicit parsers/layouts, no memory casting helpers.
- Schema migration: F-A6. F-A3 guard: versions and validation errors are typed, migration DAG absent.

## Reading Guide

- Layout pass: `version.rs`, `liveness.rs`, `continuation.rs`, `fault.rs`, `harness.rs`, `trace.rs`; compare with `layout-report.json`.
- Enum pass: `FaultCode`, `FaultDomain`, `HarnessOp`, `HarnessResultKind`, `ProbeLevel`, `ProbeBudgetClass`, `TraceDropPolicy`, `InterruptPolicy`, `RecoveryAction`, `SemanticStratum`.
- Liveness pass: `LivenessCounters::record_progress`, `note_idle_frame`, `is_livelocked`, and `InferenceStateHeader::validate`.
- Checkpoint pass: `SemanticCheckpointId` parser and `SemanticCheckpointSchema::validate`.
- Harness/trace pass: raw `u16` op/kind fields and `TraceEvent` slot layout.
- Host surface pass: cfg-gated `CompatibilityEnvelope`, `FaultPolicy`, checkpoint schema, and trace registry.

## Diff Map

{markdown_table(["File", "Risk", "Why reviewers should care", "Primary gates"], diff_rows)}

## Architecture Diagrams

### Crate Relationship

```mermaid
flowchart LR
  gbf_foundation["gbf-foundation::SemVer"] --> gbf_abi["gbf-abi"]
  gbf_abi --> runtime["F-A5 Bank0 runtime"]
  gbf_abi --> banking["F-A4 BankLease ABI"]
  gbf_abi --> emu["F-H3 gbf-emu/debug"]
  gbf_abi --> oracles["F-C1/F-C2/F-C3 oracles"]
  gbf_abi --> reports["F-F reports/failure capsules"]
```

### BuildIdentityBlock Bytes

```text
0..4 magic | 4..7 abi | 7 reserved | 8..136 four hashes | 136..144 timestamp | 144..152 tail/schema/reserved
```

### InferenceStateHeader Bytes

```text
0..4 abi/reserved | 4..8 schema/last_fault | 8..20 session/token/slice | 20..32 liveness
```

### Harness Doorbell

```mermaid
sequenceDiagram
  participant Host
  participant ROM
  Host->>ROM: write command fields and args
  Host->>ROM: set doorbell=1 last
  ROM->>ROM: poll at safe point and copy command
  ROM->>Host: write result fields and set ready=1
  ROM->>Host: clear command doorbell=0
```

### Fault Partition

```text
0x0000 None; 0x0001..0x000F Boot; 0x0010..0x001F Persistence; 0x0020..0x002F Banking;
0x0030..0x0031 Scheduling; 0x0032..0x003F Liveness; 0x0040..0x004F UI;
0x0050..0x005F Schema; 0x0060..0x006F Harness; 0x0070..0x007F Trace;
0x0080..0x008F Calibration; 0xFF00..0xFFFF Internal.
```

### Checkpoint Strata

```mermaid
flowchart TB
  den["Denotation"] --> art["Artifact"] --> op["Operational"]
```

## Correctness Dossier

- `layout-report.json` pins the expected size, alignment, and field offsets for review; source keeps targeted runtime layout tests.
- Byte parsers are explicit little-endian helpers; `gbf-abi` does not expose transmute/bytemuck casting.
- Raw enum fields in memory blocks are stored as `u16` and decoded through `from_u16`; unknown values become typed errors.
- `SemanticCheckpointId` permits only non-empty dotted `[a-z0-9_]+` segments up to 128 bytes.
- `SemanticCheckpointSchema` rejects schema version zero, duplicate semantic ids, duplicate compact ids, and `CompactCheckpointId::NONE`.
- `LivenessCounters` uses saturating epoch/frame arithmetic; threshold zero disables timeout; equality triggers livelock.
- Constructors stamp magic, zero reserved bytes, and stage harness signal bytes clear; validators reject bad magic, nonzero reserved bytes, and invalid harness signal values.

## Test Coverage Report

Run:

```bash
cargo test -p gbf-abi --no-default-features
cargo test -p gbf-abi --no-default-features --features alloc
cargo test -p gbf-abi --features host
cargo clippy -p gbf-abi --all-features -- -D warnings
python3 scripts/generate_f_a3_review_packet.py --check
```

The test names intentionally mirror the RFC acceptance gates so filtered runs remain meaningful.

## Reproducibility Report

Packet regeneration command:

```bash
python3 scripts/generate_f_a3_review_packet.py
```

Staleness check:

```bash
python3 scripts/generate_f_a3_review_packet.py --check
```

Toolchain captured during generation:

```text
{toolchain_report()}
```

`BuildIdentityBlock::timestamp_unix` is caller-provided; reproducible builds feed it from `SOURCE_DATE_EPOCH` or use `0` in deterministic mode. The ABI crate itself never reads wall-clock time.

## Generated Artifacts

- `README.md`: human review guide and mandatory content ledger.
- `layout-report.json`: byte layout evidence for every `repr(C)` ABI type.
- `claim-to-gate.md`: RFC claim-to-test mapping.
- `dependency-report.md`: dependency tree, feature notes, and no-upward-dependency evidence.
- `reviewer-checklist.md`: final approval checklist.

## Known Debt

No TODO/FIXME debt is introduced in `gbf-abi`. Deferred production paths are listed in the scope ledger with owning feature beads.

## API Guide

- Bare/no-default: fixed ABI layouts, enums, ids, liveness, harness blocks, trace events, interrupt/lease vocabulary.
- `alloc`: adds validated `SemanticCheckpointId`.
- `host`: adds compatibility envelope, checkpoint schema/resolver, fault policy, and trace probe registry.

## Error Shape Report

- `AbiVersionError`: zero, unsupported, or semver out of u8 range.
- `BuildIdentityError`: bad magic, bad ABI, truncated bytes, nonzero reserved bytes, bad schema version.
- `ContinuationError`: truncated header/tail, size overflow, bad ABI/schema, unknown last fault, nonzero reserved byte.
- `HarnessProtocolError`: bad magic, nonzero reserved byte, unknown op/result kind, sequence mismatch.
- `SnapshotDecodeError`: unknown fault code/domain, domain mismatch, nonzero reserved byte.
- `CheckpointIdError`: empty, too long, invalid char, leading/trailing/double dot.
- `SchemaValidationError`: duplicate/reserved ids, identity hash/version mismatch.
- `FaultPolicyError`: illegal default action.
- `TraceBudgetError`: inconsistent zero/nonzero budget settings.
- `TraceProbeRegistryError`: duplicate probe or identity mismatch.

## Source-To-Artifact Traceability

This file and sibling artifacts are produced by `scripts/generate_f_a3_review_packet.py`. Layout constants in the script mirror the RFC/source ABI contract; stale artifacts fail with `--check`.
"""


def render_layout_report() -> str:
    payload = {
        "rfc": "history/rfcs/F-A3-gbf-abi.md",
        "crate": "gbf-abi",
        "endianness": "little",
        "layouts": LAYOUTS,
    }
    return json.dumps(payload, indent=2, sort_keys=True) + "\n"


def render_claim_to_gate() -> str:
    rows = [(claim, gate) for claim, gate in CLAIMS]
    return "# F-A3 Claim-To-Gate Matrix\n\n" + markdown_table(["Claim", "Gate"], rows) + "\n"


def render_dependency_report() -> str:
    upward = ["gbf-runtime", "gbf-codegen", "gbf-asm", "gbf-artifact", "gbf-hw"]
    tree = normalized_cargo_tree()
    present = [name for name in upward if name in tree]
    status = "PASS: no upward contract dependency appears in `cargo tree -p gbf-abi --edges normal --features host`."
    if present:
        status = "FAIL: upward dependency name(s) present: " + ", ".join(present)

    return f"""# F-A3 Dependency Report

## Feature Gates

- `default = ["host"]`
- `host = ["std", "alloc", "dep:gbf-foundation"]`
- `std = ["serde/std"]`
- `alloc = ["serde/alloc"]`

## No-Upward-Dependency Evidence

{status}

Forbidden upward dependencies checked: {", ".join(upward)}.

## Cargo Tree

```text
{tree}
```

## License Summary

Workspace path crates in this tree do not declare package licenses. Registry dependencies are standard Rust ecosystem crates used for serde derives, JSON test support, property tests, and field offsets.
"""


def render_reviewer_checklist() -> str:
    checks = [
        "Every `repr(C)` type in `layout-report.json` matches the source ABI definitions and targeted layout tests.",
        "No fixed layout stores Rust data enums directly where unknown discriminants could be UB.",
        "All magic/reserved-byte validators reject malformed input.",
        "`BuildIdentityBlock` carries the four lineage hashes plus tail/schema fields.",
        "`LivenessCounters` is non-optional in `InferenceStateHeader` and uses saturating arithmetic.",
        "`FaultCode::ALL` is complete and `classify_fault` is total.",
        "`SemanticCheckpointSchema` rejects duplicate ids, compact zero, and schema version zero.",
        "`TraceEvent` is exactly 32 bytes and trace budget zero/nonzero rules are tested.",
        "Host-only Vec/Cow/BTreeMap types are cfg-gated behind `host`/`alloc`.",
        "`gbf-abi` has no upward dependency on runtime/codegen/asm/artifact/hw.",
        "`#![forbid(unsafe_code)]` is present in `gbf-abi/src/lib.rs`.",
        "The full feature matrix and packet staleness checks have been run.",
    ]
    return "# F-A3 Reviewer Checklist\n\n" + "\n".join(f"- [ ] {check}" for check in checks) + "\n"


def generated_files() -> dict[Path, str]:
    return {
        PACKET / "README.md": render_readme(),
        PACKET / "layout-report.json": render_layout_report(),
        PACKET / "claim-to-gate.md": render_claim_to_gate(),
        PACKET / "dependency-report.md": render_dependency_report(),
        PACKET / "reviewer-checklist.md": render_reviewer_checklist(),
    }


def check_files(files: dict[Path, str]) -> int:
    failed = False
    for path, content in files.items():
        existing = path.read_text() if path.exists() else ""
        if existing != content:
            failed = True
            rel = path.relative_to(ROOT)
            diff = difflib.unified_diff(
                existing.splitlines(keepends=True),
                content.splitlines(keepends=True),
                fromfile=f"{rel} (checked in)",
                tofile=f"{rel} (generated)",
            )
            sys.stderr.writelines(diff)
    return 1 if failed else 0


def write_files(files: dict[Path, str]) -> None:
    for path, content in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="fail if packet artifacts are stale")
    args = parser.parse_args()

    files = generated_files()
    if args.check:
        return check_files(files)

    write_files(files)
    print(f"generated {len(files)} F-A3 review packet artifacts under {PACKET.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

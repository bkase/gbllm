# Session Wire Evidence

The on-disk layout is fixed `GBSE` magic, four zero flag bytes, then a zstd-compressed UTF-8 JSON `Session`.

Load-time hard refusals cover: truncated headers, bad magic, non-zero flags, zstd decode failure, JSON decode failure, schema mismatch, ROM hash mismatch, snapshot ROM mismatch, and unsupported non-PostBootDmg lineage.

The cross-checks are intentionally redundant: `Session::rom_sha256` pins the embedded `RomBlob`, while `SnapshotLineage::rom_sha256` proves the F-A7 snapshot belongs to the same ROM. Breakpoints/watchpoints persist only `PersistedPredicate::{None,StringifiedSource}`; closure predicates never enter the wire shape.

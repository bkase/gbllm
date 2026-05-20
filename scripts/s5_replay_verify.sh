#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="${repo_root}/fixtures/s5"
manifest="${fixture_root}/MANIFEST.toml"

python3 - "$fixture_root" "$manifest" <<'PY'
import hashlib
import json
import pathlib
import sys

fixture_root = pathlib.Path(sys.argv[1])
manifest_path = pathlib.Path(sys.argv[2])

def parse_manifest(path: pathlib.Path):
    metadata = {}
    entries = []
    current = None
    for raw_line in path.read_text().splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line == "[[files]]":
            if current is not None:
                entries.append(current)
            current = {}
            continue
        if current is None or "=" not in line:
            if "=" not in line:
                continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if value.startswith('"') and value.endswith('"'):
            if current is None:
                metadata[key] = value[1:-1]
            else:
                current[key] = value[1:-1]
    if current is not None:
        entries.append(current)
    return metadata, entries

metadata, entries = parse_manifest(manifest_path)
expected_metadata = {
    "schema": "s5_fixture_manifest.v1",
    "corpus": "fixtures/s5",
    "owner_bead": "bd-u4fh",
    "fixture_scope": "SUBSTRATE_ONLY",
    "spec_sha": "UNSET_PLACEHOLDER_F_S5_RFC_SHA",
    "spec_sha_status": "placeholder_unset",
    "producer_replay_owner": "bd-q3zo",
}
for key, expected in expected_metadata.items():
    observed = metadata.get(key)
    if observed != expected:
        raise SystemExit(
            f"fixtures/s5/MANIFEST.toml {key} must be {expected!r}; observed {observed!r}"
        )
if not entries:
    raise SystemExit("fixtures/s5/MANIFEST.toml has no [[files]] entries")

manifest_paths = set()
for entry in entries:
    rel = pathlib.PurePosixPath(entry["path"])
    if rel.is_absolute() or ".." in rel.parts:
        raise SystemExit(f"unsafe manifest path: {entry['path']}")
    path = fixture_root / pathlib.Path(*rel.parts)
    manifest_paths.add(rel.as_posix())
    if not path.is_file():
        raise SystemExit(f"manifest entry missing file: {rel}")
    observed = hashlib.sha256(path.read_bytes()).hexdigest()
    if observed != entry["sha256"]:
        raise SystemExit(
            f"sha256 mismatch for {rel}: manifest={entry['sha256']} observed={observed}"
        )

orphans = []
for path in fixture_root.rglob("*"):
    if path.is_file() and path.name != "MANIFEST.toml":
        rel = path.relative_to(fixture_root).as_posix()
        if rel not in manifest_paths:
            orphans.append(rel)
if orphans:
    raise SystemExit("fixture files missing from manifest: " + ", ".join(sorted(orphans)))

build_path = fixture_root / "encoded_rom/seed_0_canonical/build.json"
build = json.loads(build_path.read_text())
rom_digest = (fixture_root / "encoded_rom/seed_0_canonical/rom.gb.sha256").read_text()
if build["rom_sha256"] not in rom_digest:
    raise SystemExit("encoded_rom/seed_0_canonical rom digest does not match build.json")
for cert in build["certs"]:
    cert_path = fixture_root / "encoded_rom/seed_0_canonical" / cert
    if not cert_path.is_file():
        raise SystemExit(f"encoded ROM cert listed by build.json is missing: {cert}")
    cert_json = json.loads(cert_path.read_text())
    if cert_json.get("valid") is not True:
        raise SystemExit(f"encoded ROM cert is not valid=true: {cert}")

def normalize_log(path: pathlib.Path) -> bytes:
    return normalize_log_rows(json.loads(line) for line in path.read_text().splitlines() if line.strip())

def normalize_log_rows(raw_rows) -> bytes:
    rows = []
    for raw_row in raw_rows:
        row = dict(raw_row)
        row.pop("timestamp", None)
        row.pop("run_id", None)
        rows.append(row)
    return ("\n".join(json.dumps(row, sort_keys=True, separators=(",", ":")) for row in rows) + "\n").encode()

def read_log_rows(path: pathlib.Path):
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        yield json.loads(line)

log_path = fixture_root / "log_streams/seed_0_canonical_run.ndjson"
first = hashlib.sha256(normalize_log(log_path)).hexdigest()
mutated_rows = []
for index, row in enumerate(read_log_rows(log_path)):
    row["timestamp"] = f"2099-01-01T00:00:0{index}Z"
    row["run_id"] = f"mutated-run-{index}"
    mutated_rows.append(row)
mutated = hashlib.sha256(normalize_log_rows(mutated_rows)).hexdigest()
if first != mutated:
    raise SystemExit("normalized seed-0 log stream changed under timestamp/run_id mutation")

frontier_hashes = []
for rel in [
    "frontier/recommendation_a.json",
    "frontier/recommendation_b_l_mt4.json",
    "frontier/recommendation_b_l_fix1.json",
    "frontier/recommendation_tie.json",
]:
    frontier_hashes.append(hashlib.sha256((fixture_root / rel).read_bytes()).hexdigest())

print("s5 replay fixture verification passed SUBSTRATE_ONLY")
print("SUBSTRATE_ONLY: fixtures/s5 replay verifies committed fixture hashes and normalization only; live producer replay is owned by bd-q3zo.")
print(f"manifest_entries={len(entries)}")
print(f"normalized_log_sha256={first}")
print("frontier_fixture_sha256s=" + ",".join(frontier_hashes))
PY

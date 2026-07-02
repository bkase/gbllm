#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PINNED_HISTORY="$ROOT/artifacts/calibration/PINNED_HASH_HISTORY.md"
PINNED_BUDGET="$ROOT/artifacts/calibration/pinned_runtime_chrome_budget.json"
OUT_DIR="${S5_NUCLEUS_DRIFT_OUT_DIR:-$ROOT/target/runtime-nucleus-drift}"

usage() {
  cat <<'USAGE'
Usage: scripts/check-nucleus-drift.sh [--self-test]

Re-emits the current gbf-runtime shell RuntimeChromeBudget and fails if it
drifts from the pinned runtime_nucleus_hash / pinned budget artifact. Promoting
a drift requires updating artifacts/calibration/PINNED_HASH_HISTORY.md and
artifacts/calibration/pinned_runtime_chrome_budget.json in the same PR.
USAGE
}

self_test=0
while (($#)); do
  case "$1" in
    --self-test)
      self_test=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "check-nucleus-drift.sh: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

pinned_raw_hash() {
  python3 - "$PINNED_HISTORY" <<'PY'
import re
import sys

for line in open(sys.argv[1], encoding="utf-8"):
    match = re.search(r"`([0-9a-f]{64})`", line)
    if match:
        print(match.group(1))
        raise SystemExit(0)
raise SystemExit("no pinned runtime_nucleus_hash found")
PY
}

if [[ "$self_test" == "1" ]]; then
  bash -n "$0"
  test -f "$PINNED_HISTORY"
  test -f "$PINNED_BUDGET"
  pinned_raw_hash >/dev/null
  python3 - "$PINNED_BUDGET" <<'PY'
import json
import sys

budget = json.load(open(sys.argv[1], encoding="utf-8"))
assert budget["runtime_nucleus_hash"].startswith("sha256:"), budget["runtime_nucleus_hash"]
assert budget["target"] == "dmg-mbc5-8mib-128kib"
assert budget["profile"] == "Bringup"
assert any(slot["class"]["kind"] == "Bank0Free" for slot in budget["rom_slots"])
assert any(slot["class"]["kind"] == "CommonBank" for slot in budget["rom_slots"])
assert any(slot["class"]["kind"] == "ExpertBank" for slot in budget["rom_slots"])
PY
  echo "[S5 NUCLEUS DRIFT] self-test PASS"
  exit 0
fi

expected_raw_hash="$(pinned_raw_hash)"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

(
  cd "$ROOT"
  cargo run -p gbf-runtime --example demo_bank0_rom -- "$OUT_DIR/current" >/dev/null
)

CURRENT_RAW_HASH_FILE="$OUT_DIR/current/runtime_nucleus_hash.txt"
CURRENT_BUDGET="$OUT_DIR/current/chrome_budget.json"
test -f "$CURRENT_RAW_HASH_FILE"
test -f "$CURRENT_BUDGET"

observed_raw_hash="$(tr -d '\n' < "$CURRENT_RAW_HASH_FILE")"
observed_raw_hash="${observed_raw_hash#sha256:}"

python3 - \
  "$PINNED_BUDGET" \
  "$CURRENT_BUDGET" \
  "$expected_raw_hash" \
  "$observed_raw_hash" \
  "$PINNED_HISTORY" <<'PY'
import json
import sys

pinned_path, current_path, expected_raw, observed_raw, history_path = sys.argv[1:6]
pinned = json.load(open(pinned_path, encoding="utf-8"))
current = json.load(open(current_path, encoding="utf-8"))

def slot_key(slot):
    return (slot["class"]["kind"], int(slot["id"]))

def slot_map(budget):
    return {slot_key(slot): slot for slot in budget["rom_slots"]}

def int_field(slot, field):
    if slot is None:
        return 0
    return int(slot[field])

def effective_bytes(slot):
    return max(0, int_field(slot, "usable_bytes") - int_field(slot, "reserved_slack"))

pinned_slots = slot_map(pinned)
current_slots = slot_map(current)
all_keys = sorted(set(pinned_slots) | set(current_slots))
deltas = [
    {
        "slot_class": slot_class,
        "slot_id": slot_id,
        "pinned_usable_bytes": int_field(pinned_slots.get((slot_class, slot_id)), "usable_bytes"),
        "current_usable_bytes": int_field(current_slots.get((slot_class, slot_id)), "usable_bytes"),
        "delta_bytes": int_field(current_slots.get((slot_class, slot_id)), "usable_bytes")
        - int_field(pinned_slots.get((slot_class, slot_id)), "usable_bytes"),
        "pinned_reserved_slack_bytes": int_field(
            pinned_slots.get((slot_class, slot_id)), "reserved_slack"
        ),
        "current_reserved_slack_bytes": int_field(
            current_slots.get((slot_class, slot_id)), "reserved_slack"
        ),
        "reserved_slack_delta_bytes": int_field(
            current_slots.get((slot_class, slot_id)), "reserved_slack"
        )
        - int_field(pinned_slots.get((slot_class, slot_id)), "reserved_slack"),
        "pinned_effective_bytes": effective_bytes(pinned_slots.get((slot_class, slot_id))),
        "current_effective_bytes": effective_bytes(current_slots.get((slot_class, slot_id))),
        "effective_delta_bytes": effective_bytes(current_slots.get((slot_class, slot_id)))
        - effective_bytes(pinned_slots.get((slot_class, slot_id))),
        "slot_contract_matches": pinned_slots.get((slot_class, slot_id))
        == current_slots.get((slot_class, slot_id)),
    }
    for slot_class, slot_id in all_keys
]

raw_hashes_match = expected_raw == observed_raw
budget_hashes_match = pinned["runtime_nucleus_hash"] == current["runtime_nucleus_hash"]
slots_match = all(delta["slot_contract_matches"] for delta in deltas)
budget_section_matches = {
    "target": pinned["target"] == current["target"],
    "profile": pinned["profile"] == current["profile"],
    "reference_shell_modules": pinned["reference_shell_modules"]
    == current["reference_shell_modules"],
    "rom_slots": slots_match,
    "memory_caps": pinned["memory_caps"] == current["memory_caps"],
    "wram_reserved": pinned["wram_reserved"] == current["wram_reserved"],
    "sram_reserved": pinned["sram_reserved"] == current["sram_reserved"],
}
budget_artifacts_match = pinned == current

report = {
    "schema": "s5_runtime_nucleus_drift.v1",
    "raw_runtime_nucleus_hashes_match": raw_hashes_match,
    "budget_runtime_nucleus_hashes_match": budget_hashes_match,
    "budget_artifacts_match": budget_artifacts_match,
    "rom_slots_match": slots_match,
    "budget_section_matches": budget_section_matches,
    "changed_budget_sections": [
        name for name, matches in budget_section_matches.items() if not matches
    ],
    "expected_raw_runtime_nucleus_hash": f"sha256:{expected_raw}",
    "observed_raw_runtime_nucleus_hash": f"sha256:{observed_raw}",
    "pinned_budget_runtime_nucleus_hash": pinned["runtime_nucleus_hash"],
    "current_budget_runtime_nucleus_hash": current["runtime_nucleus_hash"],
    "per_slot_byte_deltas": deltas,
    "promotion_instructions": (
        f"Update {history_path} and {pinned_path} intentionally in the same PR "
        "when promoting a runtime nucleus drift."
    ),
}

print(json.dumps(report, indent=2, sort_keys=True))
if not (raw_hashes_match and budget_hashes_match and budget_artifacts_match):
    raise SystemExit(1)
PY

echo "S5 nucleus drift gate PASS emitter=gbf-runtime/demo_bank0_rom pinned_budget=$PINNED_BUDGET"

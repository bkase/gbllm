#!/usr/bin/env bash

chunk2_utc_now() {
  date -u '+%Y-%m-%dT%H:%M:%SZ'
}

chunk2_log_init() {
  local log_file="$1"
  mkdir -p "$(dirname "$log_file")"
  : > "$log_file"
}

chunk2_log_event() {
  local log_file="$1"
  local event="$2"
  shift 2
  local timestamp
  timestamp="$(chunk2_utc_now)"
  python3 - "$log_file" "$timestamp" "$event" "$@" <<'PY'
import json
import sys

log_file = sys.argv[1]
timestamp = sys.argv[2]
event = sys.argv[3]
pairs = sys.argv[4:]
record = {"ts": timestamp, "event": event}
if len(pairs) % 2:
    raise SystemExit("chunk2_log_event requires key/value pairs")
for key, value in zip(pairs[0::2], pairs[1::2]):
    if value == "true":
        parsed = True
    elif value == "false":
        parsed = False
    elif value.isdecimal():
        parsed = int(value)
    else:
        parsed = value
    record[key] = parsed
with open(log_file, "a", encoding="utf-8") as handle:
    handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")
PY
}

chunk2_read_toml_string() {
  local key="$1"
  local path="$2"
  sed -n "s/^${key} = \"\\(.*\\)\"/\\1/p" "$path"
}

chunk2_sha256_file() {
  local path="$1"
  shasum -a 256 "$path" | awk '{print $1}'
}

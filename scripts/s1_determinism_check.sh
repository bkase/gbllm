#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s1_determinism_check.sh [--fast] [--json] [--self-test] [--manifest PATH] [--gbf-bin PATH]

Runs the F-S1 O2 same-machine determinism check for seed 0.

Modes:
  default      Production budget against fixtures/corpora/tinystories.toml.
  --fast       Tiny fixture corpus with the explicit IntegrationFixture budget.
  --self-test  Exercise the script byte-diff diagnostics without invoking gbf.

Environment:
  GBF_S1_GBF_BIN       gbf-cli binary path override.
  GBF_S1_PASS_VERSION  pass version override (default: 0.1.0).

The gbf subprocess is launched with only the S1CpuDeterministic environment
variables so F-S1.04 enforcement runs before replay allocation.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode=production
json=0
self_test=0
manifest=""
gbf_bin="${GBF_S1_GBF_BIN:-}"
pass_version="${GBF_S1_PASS_VERSION:-0.1.0}"

while (($#)); do
    case "$1" in
        --fast)
            mode=fast
            ;;
        --json)
            json=1
            ;;
        --self-test)
            self_test=1
            ;;
        --manifest)
            shift
            if (($# == 0)); then
                echo "error: --manifest requires a path" >&2
                exit 2
            fi
            manifest="$1"
            ;;
        --gbf-bin)
            shift
            if (($# == 0)); then
                echo "error: --gbf-bin requires a path" >&2
                exit 2
            fi
            gbf_bin="$1"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

log() {
    printf '[DETERMINISM] %s\n' "$*" >&2
}

json_escape() {
    python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

sha256_file() {
    python3 - "$1" <<'PY'
import hashlib
import sys
from pathlib import Path

print("sha256:" + hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest())
PY
}

self_test_first_diff_report() {
    python3 - "$1" "$2" <<'PY'
import sys
from pathlib import Path

left = Path(sys.argv[1]).read_bytes()
right = Path(sys.argv[2]).read_bytes()
limit = min(len(left), len(right))
for index in range(limit):
    if left[index] != right[index]:
        print(f"byte_offset={index} expected=0x{left[index]:02X} observed=0x{right[index]:02X}")
        raise SystemExit(1)
if len(left) != len(right):
    side = "expected_eof" if len(left) < len(right) else "observed_eof"
    value = right[limit] if len(left) < len(right) else left[limit]
    print(f"byte_offset={limit} {side}=true byte=0x{value:02X}")
    raise SystemExit(1)
print("identical")
PY
}

run_self_test() {
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/s1-determinism-self-test.XXXXXX")"
    trap 'rm -rf "$tmp"' RETURN

    printf '\x00\xAB\x02' > "$tmp/left.bin"
    printf '\x00\xAB\x02' > "$tmp/right.bin"
    if [[ "$(self_test_first_diff_report "$tmp/left.bin" "$tmp/right.bin")" != "identical" ]]; then
        echo "self-test failed: identical files were reported different" >&2
        exit 1
    fi

    printf '\x00\xAB\x02' > "$tmp/left.bin"
    printf '\x00\xCD\x02' > "$tmp/right.bin"
    set +e
    report="$(self_test_first_diff_report "$tmp/left.bin" "$tmp/right.bin")"
    status=$?
    set -e
    if [[ "$status" -eq 0 || "$report" != "byte_offset=1 expected=0xAB observed=0xCD" ]]; then
        echo "self-test failed: mismatch diagnostic was '$report' with status $status" >&2
        exit 1
    fi

    log "self-test PASS"
}

if ((self_test)); then
    run_self_test
    exit 0
fi

if [[ -z "$manifest" ]]; then
    case "$mode" in
        fast)
            manifest="$repo_root/gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml"
            ;;
        production)
            manifest="$repo_root/fixtures/corpora/tinystories.toml"
            ;;
        *)
            echo "error: unknown mode $mode" >&2
            exit 2
            ;;
    esac
fi

if [[ -z "$gbf_bin" ]]; then
    log "building gbf-cli"
    cargo build -p gbf-cli >/dev/null
    gbf_bin="$repo_root/target/debug/gbf-cli"
fi

if [[ ! -x "$gbf_bin" ]]; then
    echo "error: gbf binary is not executable: $gbf_bin" >&2
    exit 2
fi

args=(
    s1 verify-determinism
    --manifest "$manifest"
    --pass-version "$pass_version"
    --seed 0
    --device-profile S1CpuDeterministic
)

if [[ "$mode" == "fast" ]]; then
    args+=(--budget-profile integration-fixture --allow-noncanonical-integration-fixture)
fi

log "starting replay 1 of seed 0"
log "starting replay 2 of seed 0"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/s1-determinism-check.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
stdout="$tmp/stdout.json"
stderr="$tmp/stderr.log"

set +e
env -i \
    BURN_NDARRAY_NUM_THREADS=1 \
    BURN_DETERMINISTIC=1 \
    OMP_NUM_THREADS=1 \
    RAYON_NUM_THREADS=1 \
    "$gbf_bin" "${args[@]}" >"$stdout" 2>"$stderr"
status=$?
set -e

if [[ "$status" -ne 0 ]]; then
    failure_detail="$(cat "$stderr")"
    if ((json)); then
        escaped="$(json_escape "$failure_detail")"
        printf '{"deterministic":false,"seed":0,"mode":"%s","error":%s,"failure_detail":%s}\n' "$mode" "$escaped" "$escaped"
    fi
    printf '%s\n' "$failure_detail" >&2
    log "FAIL  gbf s1 verify-determinism exited with status $status"
    exit "$status"
fi

checkpoint_sha="$(python3 - "$stdout" <<'PY'
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text())
print(payload["checkpoint_sha"])
PY
)"

log "PASS  safetensors byte-identical (sha256=$checkpoint_sha)"

if ((json)); then
    cat "$stdout"
else
    printf 'S1 determinism PASS seed=0 mode=%s checkpoint_sha=%s\n' "$mode" "$checkpoint_sha"
fi

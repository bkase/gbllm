#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s1_isolation_check.sh [--fast] [--json] [--self-test] [--manifest PATH] [--allow-fast-custom-manifest] [--gbf-bin PATH]

Runs the F-S1 O9 per-seed isolation smoke check.

Checks:
  1. seeds 0..=4 produce at least two distinct final_checkpoint_sha values.
  2. replaying [0,1] and [1,0] produces identical per-seed hashes.

Modes:
  default      Production budget against fixtures/corpora/tinystories.toml.
  --fast       Tiny fixture corpus with the explicit IntegrationFixture budget.
  --self-test  Exercise the script validation diagnostics without invoking gbf.

Safety:
  --fast --manifest PATH is rejected unless --allow-fast-custom-manifest is
  also supplied. This prevents accidentally running a custom or canonical
  corpus under the IntegrationFixture budget. Use the opt-in only for tiny
  fixture-like manifests whose short budget is intentional.

Environment:
  GBF_S1_GBF_BIN       gbf-cli binary path override.
  GBF_S1_PASS_VERSION  pass version override (default: 0.1.0).

The gbf subprocesses are launched with only the S1CpuDeterministic environment
variables so F-S1.04 enforcement runs before replay allocation.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode=production
json=0
self_test=0
manifest=""
manifest_supplied=0
allow_fast_custom_manifest=0
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
            manifest_supplied=1
            ;;
        --allow-fast-custom-manifest)
            allow_fast_custom_manifest=1
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
    printf '[ISOLATION] %s\n' "$*" >&2
}

json_escape() {
    python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

hash_for_seed() {
    python3 - "$1" "$2" <<'PY'
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text())
seed = int(sys.argv[2])
for product in payload["products"]:
    if int(product["seed"]) == seed:
        print(product["checkpoint_sha"])
        raise SystemExit(0)
raise SystemExit(f"seed {seed} missing from replay summary")
PY
}

distinct_count() {
    python3 - "$1" <<'PY'
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text())
print(len({product["checkpoint_sha"] for product in payload["products"]}))
PY
}

validate_distinct_seeds() {
    local summary="$1"
    local count
    count="$(distinct_count "$summary")"
    log "distinct final_checkpoint_sha count=$count (expected >= 2)"
    if ((count < 2)); then
        echo "[ISOLATION] FAIL  A17 all-identical: all seeds produced identical checkpoint sha; rng not actually consumed" >&2
        return 1
    fi
}

validate_order_invariant() {
    local order_0_1="$1"
    local order_1_0="$2"
    local failed=0

    for seed in 0 1; do
        local left
        local right
        left="$(hash_for_seed "$order_0_1" "$seed")"
        right="$(hash_for_seed "$order_1_0" "$seed")"
        if [[ "$left" != "$right" ]]; then
            echo "[ISOLATION] FAIL  seed $seed hash differs across orderings order_0_1=$left order_1_0=$right" >&2
            failed=1
        fi
    done

    if ((failed)); then
        echo "[ISOLATION] FAIL  Rep-7 order-dependence: seed-order dependence detected; global mutable state suspected" >&2
        return 1
    fi

    log "PASS  per-seed hashes order-invariant"
}

write_summary_fixture() {
    local path="$1"
    shift
    python3 - "$path" "$@" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
pairs = [arg.split("=", 1) for arg in sys.argv[2:]]
payload = {
    "pass_version": "0.1.0",
    "budget_profile": "IntegrationFixture",
    "products": [
        {
            "seed": int(seed),
            "completion": "completed",
            "checkpoint_sha": checkpoint_sha,
            "run_log_self_hash": "sha256:" + f"{int(seed):064x}",
        }
        for seed, checkpoint_sha in pairs
    ],
}
path.write_text(json.dumps(payload), encoding="utf-8")
PY
}

run_self_test() {
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/s1-isolation-self-test.XXXXXX")"
    trap 'rm -rf "$tmp"' RETURN

    write_summary_fixture "$tmp/distinct.json" \
        0=sha256:0000000000000000000000000000000000000000000000000000000000000000 \
        1=sha256:1111111111111111111111111111111111111111111111111111111111111111 \
        2=sha256:2222222222222222222222222222222222222222222222222222222222222222 \
        3=sha256:3333333333333333333333333333333333333333333333333333333333333333 \
        4=sha256:4444444444444444444444444444444444444444444444444444444444444444
    validate_distinct_seeds "$tmp/distinct.json"

    write_summary_fixture "$tmp/constant.json" \
        0=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
        1=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
        2=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
        3=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
        4=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    set +e
    validate_distinct_seeds "$tmp/constant.json" >/dev/null 2>"$tmp/constant.err"
    status=$?
    set -e
    cat "$tmp/constant.err" >&2
    if [[ "$status" -eq 0 ]] || ! grep -q "all seeds produced identical checkpoint sha" "$tmp/constant.err"; then
        echo "self-test failed: constant-output diagnostic missing" >&2
        exit 1
    fi

    write_summary_fixture "$tmp/order-01.json" \
        0=sha256:0000000000000000000000000000000000000000000000000000000000000000 \
        1=sha256:1111111111111111111111111111111111111111111111111111111111111111
    write_summary_fixture "$tmp/order-10-pass.json" \
        1=sha256:1111111111111111111111111111111111111111111111111111111111111111 \
        0=sha256:0000000000000000000000000000000000000000000000000000000000000000
    validate_order_invariant "$tmp/order-01.json" "$tmp/order-10-pass.json"

    write_summary_fixture "$tmp/order-10-fail.json" \
        1=sha256:9999999999999999999999999999999999999999999999999999999999999999 \
        0=sha256:0000000000000000000000000000000000000000000000000000000000000000
    set +e
    validate_order_invariant "$tmp/order-01.json" "$tmp/order-10-fail.json" >/dev/null 2>"$tmp/order.err"
    status=$?
    set -e
    cat "$tmp/order.err" >&2
    if [[ "$status" -eq 0 ]] || ! grep -q "seed-order dependence detected" "$tmp/order.err"; then
        echo "self-test failed: order-dependence diagnostic missing" >&2
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

default_fast_manifest="$repo_root/gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml"
if [[ "$mode" == "fast" ]] && ((manifest_supplied)) && ((allow_fast_custom_manifest == 0)); then
    manifest_abs="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' "$manifest")"
    default_fast_manifest_abs="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve())' "$default_fast_manifest")"
    if [[ "$manifest_abs" != "$default_fast_manifest_abs" ]]; then
        cat >&2 <<'ERROR'
error: --fast uses the IntegrationFixture budget and only accepts the default tiny fixture manifest.
       Custom manifests under --fast require explicit opt-in:
       scripts/s1_isolation_check.sh --fast --manifest PATH --allow-fast-custom-manifest

       Do not use this opt-in for the canonical TinyStories manifest; omit --fast for production-budget isolation.
ERROR
        exit 2
    fi
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

tmp="$(mktemp -d "${TMPDIR:-/tmp}/s1-isolation-check.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

replay_args=(
    --manifest "$manifest"
    --pass-version "$pass_version"
    --device-profile S1CpuDeterministic
)

if [[ "$mode" == "fast" ]]; then
    replay_args+=(--budget-profile integration-fixture --allow-noncanonical-integration-fixture)
fi

run_replay() {
    local seed_list="$1"
    local out_dir="$2"
    local stdout="$3"
    local stderr="$4"

    set +e
    env -i \
        BURN_NDARRAY_NUM_THREADS=1 \
        BURN_DETERMINISTIC=1 \
        OMP_NUM_THREADS=1 \
        RAYON_NUM_THREADS=1 \
        "$gbf_bin" s1 replay \
        "${replay_args[@]}" \
        --seed-list "$seed_list" \
        --out-dir "$out_dir" >"$stdout" 2>"$stderr"
    local status=$?
    set -e
    if [[ "$status" -ne 0 ]]; then
        cat "$stderr" >&2
        log "FAIL  gbf s1 replay --seed-list $seed_list exited with status $status"
        return "$status"
    fi
}

log "running seeds 0..=4"
all_summary="$tmp/all.json"
run_replay "0,1,2,3,4" "$tmp/all-out" "$all_summary" "$tmp/all.err"
if ! validate_distinct_seeds "$all_summary"; then
    if ((json)); then
        printf '{"isolated":false,"mode":"%s","reason":"all_seeds_identical"}\n' "$mode"
    fi
    exit 1
fi

log "running [0,1] then [1,0]"
order_0_1="$tmp/order-0-1.json"
order_1_0="$tmp/order-1-0.json"
run_replay "0,1" "$tmp/order-0-1-out" "$order_0_1" "$tmp/order-0-1.err"
run_replay "1,0" "$tmp/order-1-0-out" "$order_1_0" "$tmp/order-1-0.err"
if ! validate_order_invariant "$order_0_1" "$order_1_0"; then
    if ((json)); then
        printf '{"isolated":false,"mode":"%s","reason":"seed_order_dependence"}\n' "$mode"
    fi
    exit 1
fi

count="$(distinct_count "$all_summary")"
if ((json)); then
    seed0="$(hash_for_seed "$order_0_1" 0)"
    seed1="$(hash_for_seed "$order_0_1" 1)"
    printf '{"isolated":true,"mode":"%s","distinct_final_checkpoint_sha_count":%s,"seed_hashes":{"0":"%s","1":"%s"}}\n' \
        "$mode" "$count" "$seed0" "$seed1"
else
    printf 'S1 isolation PASS mode=%s distinct_final_checkpoint_sha_count=%s\n' "$mode" "$count"
fi

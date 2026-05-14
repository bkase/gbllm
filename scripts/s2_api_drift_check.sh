#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: scripts/s2_api_drift_check.sh [--dry-run] [--report-path PATH] [--report-dir DIR]

Runs the F-S2 O11 public API drift gate. By default the structured report is
written to /tmp/s2-api-drift.json. Dry-run validates the pinned S1 API snapshot
files and emits the same report schema without claiming live API evidence. The
default /tmp report path is for serial local use; use --report-path/--report-dir
for parallel jobs.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dry_run=0
report_path="/tmp/s2-api-drift.json"

while (($#)); do
    case "$1" in
        --dry-run)
            dry_run=1
            ;;
        --report-path)
            shift
            if [[ $# -eq 0 ]]; then
                echo "error: --report-path requires a value" >&2
                exit 2
            fi
            report_path="$1"
            ;;
        --report-dir)
            shift
            if [[ $# -eq 0 ]]; then
                echo "error: --report-dir requires a value" >&2
                exit 2
            fi
            report_path="${1%/}/s2-api-drift.json"
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

python3 - "$repo_root" "$dry_run" "$report_path" <<'PY'
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

repo = Path(sys.argv[1])
dry_run = sys.argv[2] == "1"
report_path = Path(sys.argv[3])
script = "s2_api_drift_check"
snapshots = repo / "gbf-experiments" / "snapshots"
stages = []

def emit(payload):
    print(json.dumps(payload, sort_keys=True, separators=(",", ":")), file=sys.stderr)

def file_hash(path):
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()

def stage_start(index, description):
    emit({"event": f"{script}_stage_start", "stage": index, "description": description})

def stage_done(name, index, passed, detail):
    stages.append({"name": name, "passed": passed, "detail": detail})
    emit({"event": f"{script}_stage_done", "stage": index, "passed": passed, "detail": detail})

def finish(passed, code, summary):
    report = {
        "script": script,
        "passed": passed,
        "stages": stages,
        "exit_code": code,
        "dry_run": dry_run,
        "evidence_mode": "dry_run" if dry_run else "live",
        "live_evidence": not dry_run,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
    emit({"event": f"{script}_exit", "exit_code": code, "passed": passed, "summary": summary})
    print(summary)
    raise SystemExit(code)

def fail(stage, reason, remediation):
    if stages and stages[-1]["passed"]:
        stages[-1]["passed"] = False
        stages[-1]["detail"] = {**stages[-1]["detail"], "reason": reason}
    emit({"event": f"{script}_failure", "stage": stage, "reason": reason, "remediation": remediation})
    finish(False, 1, f"S2 api-drift FAIL stage={stage} reason={reason} report={report_path}")

def snapshot_hashes():
    return {
        "qat_public_api_snapshot_hash": file_hash(qat_path),
        "linearstate_public_api_snapshot_hash": file_hash(linearstate_path),
    }

def normalize(symbols):
    return sorted({symbol.strip() for symbol in symbols if symbol.strip() and not symbol.strip().startswith("#")})

def cargo_public_api_symbols():
    if os.environ.get("S2_API_DRIFT_FORCE_TEXT_FALLBACK") == "1":
        return None
    if shutil.which("cargo-public-api") is None:
        return None
    try:
        output = subprocess.run(
            ["cargo", "public-api", "-p", "gbf-model"],
            cwd=repo,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
            timeout=180,
        )
    except Exception:
        return None
    qat = []
    linearstate = []
    for line in output.stdout.splitlines():
        line = line.strip()
        if "::qat::" in line:
            qat.append(public_api_line_name(line))
        if "::sequence::" in line and "::LinearState" in line:
            linearstate.append(public_api_line_name(line))
    if qat and linearstate:
        return {"qat": normalize(qat), "linearstate": normalize(linearstate), "source": "cargo-public-api"}
    return None

def public_api_line_name(line):
    token = line.split()[-1]
    name = token.rsplit("::", 1)[-1]
    return re.split(r"[<(]", name, maxsplit=1)[0]

def fallback_pub_use_symbols():
    root = repo / "gbf-model" / "src"
    qat = parse_public_surface(root / "qat" / "mod.rs")
    linearstate = parse_public_surface(
        root / "sequence" / "mod.rs",
        only_reexport_module="linear_state",
    )
    if not qat or not linearstate:
        fail(
            2,
            "API drift text fallback could not extract required public symbols",
            "install cargo-public-api or keep gbf-model module re-exports/direct public items parseable by scripts/s2_api_drift_check.sh",
        )
    return {
        "qat": qat,
        "linearstate": linearstate,
        "source": "live-workspace-text-fallback",
        "diagnostic": "cargo-public-api unavailable or explicitly bypassed; used deterministic Rust text extraction",
    }

def strip_rust_comments(text):
    without_blocks = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//.*", "", without_blocks)

def split_symbols(body):
    symbols = []
    current = []
    depth = 0
    for char in body:
        if char in "([{<":
            depth += 1
        elif char in ")]}>":
            depth = max(0, depth - 1)
        if char == "," and depth == 0:
            symbol = "".join(current).strip()
            if symbol:
                symbols.append(symbol)
            current = []
        else:
            current.append(char)
    symbol = "".join(current).strip()
    if symbol:
        symbols.append(symbol)
    return symbols

def exported_name(raw):
    symbol = raw.strip()
    if not symbol or symbol == "self":
        return None
    if " as " in symbol:
        symbol = symbol.rsplit(" as ", 1)[-1].strip()
    symbol = symbol.rsplit("::", 1)[-1].strip()
    return re.split(r"[<(]", symbol, maxsplit=1)[0].strip() or None

def module_path_for(parent, module):
    inline = parent.parent / f"{module}.rs"
    nested = parent.parent / module / "mod.rs"
    if inline.exists():
        return inline
    if nested.exists():
        return nested
    return None

def direct_public_items(path):
    text = strip_rust_comments(path.read_text(encoding="utf-8"))
    names = []
    depth = 0
    pattern = re.compile(
        r"^\s*pub\s+(?:async\s+)?(?:struct|enum|fn|type|trait|const|static|union)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )
    for line in text.splitlines():
        if depth == 0 and (match := pattern.match(line)):
            names.append(match.group("name"))
        depth += line.count("{") - line.count("}")
        depth = max(0, depth)
    return names

def parse_public_surface(path, only_reexport_module=None):
    text = path.read_text(encoding="utf-8")
    text = strip_rust_comments(text)
    names = []
    names.extend(direct_public_items(path))
    for match in re.finditer(r"pub\s+use\s+([a-zA-Z0-9_:]+)::\{(?P<body>.*?)\};", text, re.S):
        module = match.group(1).rsplit("::", 1)[-1]
        if only_reexport_module is not None and module != only_reexport_module:
            continue
        names.extend(name for raw in split_symbols(match.group("body").replace("\n", " ")) if (name := exported_name(raw)))
    for match in re.finditer(r"(?m)^\s*pub\s+use\s+([a-zA-Z0-9_:]+)::(?P<name>[A-Za-z_][A-Za-z0-9_]*|\*);", text):
        module_path = match.group(1)
        module = module_path.rsplit("::", 1)[-1]
        if only_reexport_module is not None and module != only_reexport_module:
            continue
        if match.group("name") == "*":
            child = module_path_for(path, module)
            if child is not None:
                names.extend(direct_public_items(child))
        else:
            names.append(match.group("name"))
    return normalize(names)

def run_text_fallback_self_test():
    with tempfile.TemporaryDirectory(prefix="s2-api-fallback-self-test-") as tmp:
        root = Path(tmp)
        qat = root / "qat"
        sequence = root / "sequence"
        qat.mkdir()
        sequence.mkdir()
        (qat / "mod.rs").write_text(
            """
            /* pub struct BlockCommentGhost; */
            pub static QAT_SENTINEL: u8 = 1;
            pub union QatUnion { value: u32 }
            pub use direct::DirectThing;
            pub use nested::{NestedAlias as RenamedNested, NestedThing};
            mod direct { pub struct DirectThing; }
            mod nested { pub struct NestedThing; pub struct NestedAlias; }
            """,
            encoding="utf-8",
        )
        (sequence / "mod.rs").write_text(
            """
            pub mod linear_state;
            pub use linear_state::{LinearStateBlock, LinearStateBlockConfig};
            """,
            encoding="utf-8",
        )
        (sequence / "linear_state.rs").write_text(
            """
            /* pub struct CommentedLinearState; */
            pub static LINEAR_STATE_SENTINEL: u8 = 1;
            pub union LinearStateUnion { value: u32 }
            pub struct LinearStateBlock;
            pub struct LinearStateBlockConfig;
            """,
            encoding="utf-8",
        )
        qat_symbols = parse_public_surface(qat / "mod.rs")
        linearstate_symbols = parse_public_surface(
            sequence / "mod.rs",
            only_reexport_module="linear_state",
        )
    required_qat = {"QAT_SENTINEL", "QatUnion", "DirectThing", "RenamedNested", "NestedThing"}
    required_linearstate = {"LinearStateBlock", "LinearStateBlockConfig"}
    if not required_qat.issubset(qat_symbols) or not required_linearstate.issubset(linearstate_symbols):
        fail(
            0,
            "API drift text fallback self-test failed",
            "update scripts/s2_api_drift_check.sh fallback parser coverage",
        )
    if "BlockCommentGhost" in qat_symbols or "CommentedLinearState" in linearstate_symbols:
        fail(
            0,
            "API drift text fallback parsed block-commented symbols",
            "strip Rust block comments before text fallback extraction",
        )
    stage_done(
        "text_fallback_self_test",
        0,
        True,
        {
            "evidence_source": "live-workspace-text-fallback-self-test",
            "qat_symbols": qat_symbols,
            "linearstate_symbols": linearstate_symbols,
        },
    )
    finish(True, 0, f"S2 api-drift PASS text_fallback_self_test=true report={report_path}")

def extract_current_symbols():
    if os.environ.get("S2_API_DRIFT_FORCE_SELF_ORACLE") == "1":
        fail(2, "self-oracle current symbols rejected", "extract current symbols from the live workspace instead of snapshots")
    extracted = cargo_public_api_symbols() or fallback_pub_use_symbols()
    if os.environ.get("S2_SCRIPT_INJECT_FAILURE") == "api_drift_added":
        extracted["qat"] = normalize(extracted["qat"] + ["SyntheticFutureQatSymbol"])
    return extracted

def rust_check(current):
    with tempfile.TemporaryDirectory(prefix="s2-api-drift-") as tmp:
        tmp = Path(tmp)
        current_path = tmp / "current.json"
        result_path = tmp / "result.json"
        current_path.write_text(json.dumps({"qat": current["qat"], "linearstate": current["linearstate"]}), encoding="utf-8")
        env = os.environ.copy()
        env.update({
            "S2_API_DRIFT_CURRENT_JSON": str(current_path),
            "S2_API_DRIFT_SNAPSHOTS_DIR": str(snapshots),
            "S2_API_DRIFT_RESULT_JSON": str(result_path),
        })
        command = [
            "cargo", "test", "-p", "gbf-experiments", "--features", "s2-full",
            "--test", "cli_scripts_s2", "__s2_api_drift_probe", "--", "--ignored", "--exact",
        ]
        output = subprocess.run(command, cwd=repo, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if output.returncode != 0:
            stage_done("rust_api_drift_wrapper", 3, False, {"status": output.returncode, "stderr_tail": output.stderr[-2000:]})
            fail(3, "Rust API drift wrapper failed", "inspect gbf-experiments/tests/cli_scripts_s2.rs::__s2_api_drift_probe")
        return json.loads(result_path.read_text(encoding="utf-8"))

if os.environ.get("S2_API_DRIFT_TEXT_FALLBACK_SELF_TEST") == "1":
    stage_start(0, "Self-test deterministic Rust text fallback extraction")
    run_text_fallback_self_test()

qat_path = snapshots / "s1_qat_public_api.txt"
linearstate_path = snapshots / "s1_linearstate_public_api.txt"

stage_start(1, "Validate pinned S1 API snapshot files")
missing = [str(path) for path in (qat_path, linearstate_path) if not path.exists()]
if missing:
    stage_done("snapshot_inputs", 1, False, {"missing": missing})
    fail(1, "missing API snapshot", "restore gbf-experiments/snapshots S1 public API files")
hashes = snapshot_hashes()
stage_done("snapshot_inputs", 1, True, {**hashes, "dry_run": dry_run})

if dry_run:
    finish(True, 0, f"S2 api-drift PASS dry_run=true report={report_path}")

stage_start(2, "Extract current public symbols from the live workspace")
current = extract_current_symbols()
stage_done(
    "extract_current_symbols",
    2,
    True,
    {
        "evidence_source": current["source"],
        "qat_count": len(current["qat"]),
        "linearstate_count": len(current["linearstate"]),
        "diagnostic": current.get("diagnostic"),
        "dry_run": False,
    },
)

stage_start(3, "Diff current symbols against pinned S1 snapshots through Rust semantics")
result = rust_check(current)
passed = bool(result["passed"])
stage_done(
    "diff_snapshots",
    3,
    passed,
    {
        **hashes,
        "evidence_source": "gbf_experiments::s2::api_drift::check_api_drift",
        "drift_count": result["drift_count"],
        "drifts": result["drifts"],
    },
)
if not passed:
    fail(3, "public API drift", "update the RFC allow-list and snapshot only through an approved S2 API drift bead")

finish(True, 0, f"S2 api-drift PASS dry_run=false drift_count=0 report={report_path}")
PY

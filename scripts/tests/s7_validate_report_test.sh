#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-validate-report-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp/valid.md" <<'PY'
from pathlib import Path
import hashlib
import json
import re
import sys

path = Path(sys.argv[1])
h = "sha256:" + "1" * 64


def domain_bytes_hash(domain, payload: bytes) -> str:
    crate_name, type_name, schema_id, schema_version = domain
    material = (
        f"gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}".encode("utf-8")
        + b"\0"
        + payload
    )
    return f"sha256:{hashlib.sha256(material).hexdigest()}"


def domain_json_hash(domain, payload) -> str:
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)
    return domain_bytes_hash(domain, canonical.encode("utf-8"))


switch_hash = domain_json_hash(
    ("gbf-experiments", "S7SwitchStatsBundleManifest", "s7_switch_stats_bundle_manifest.v1", "1"),
    {
        "schema": "s7_switch_stats_bundle_manifest.v1",
        "seed_bundle_self_hashes": [
            {"seed": seed, "bundle_self_hash": h} for seed in range(5)
        ],
    },
)


def with_report_self_hash(text: str) -> str:
    expected = domain_bytes_hash(
        ("gbf-experiments", "S7ReportMarkdown", "s7_report.v1", "1"),
        text.encode("utf-8"),
    )
    return re.sub(r"^report_self_hash: null$", f'report_self_hash: "{expected}"', text, flags=re.MULTILINE)

rows = []
for topology in ["MoeTiny", "MoeTinyDenseMatched"]:
    for seed in range(5):
        rows.append(
            f"""  - seed: {seed}
    topology: "{topology}"
    completion: Completed
    checkpoint_self_hash: "{h}"
    run_log_self_hash: "{h}"
    score_self_hash: "{h}"
"""
        )
front_rows = "".join(rows)
body = "\n".join(
    [
        "## Pre-registered predictions",
        "Pinned before results.",
        "## Observed (per-seed, per-topology table)",
        "All rows recorded.",
        "## Hypothesis verdicts",
        "H1 Confirmed\nH2 Confirmed\nH3 Confirmed\nH4 Confirmed\nH5 Confirmed\nH6 Confirmed\nH7 Confirmed\nH8 Confirmed\nH9 Confirmed\nH10 Confirmed",
        "## Falsification analysis",
        "No refutation.",
        "## Switch statistics summary",
        "See artifact.",
        "## lambda_switch sweep summary",
        "See artifact.",
        "## Pareto verdict",
        "MoE dominates.",
        "## Surprises",
        "None.",
        "## Decision",
        "ProceedToS8.",
        "## Reproducibility statement",
        "Replay command pinned.",
        "",
    ]
)
report = f"""---
schema: "s7_report.v1"
s7_outcome: PassClean
decision: ProceedToS8
matched_bytes_self_hash: "{h}"
per_seed_artifacts:
{front_rows}switch_stats_self_hash: "{switch_hash}"
router_collapse_sweep_self_hash: "{h}"
dense_vs_moe_self_hash: "{h}"
frontier_self_hash: "{h}"
burn_grad_smoke_self_hash: "{h}"
oracle_routed_self_hash: "{h}"
emulator_one_token_moe_self_hash: "{h}"
emulator_one_token_dense_self_hash: null
rfc_revision: "{"a" * 40}"
predictions_section_hash: "{h}"
predictions_commit: "{"b" * 40}"
first_result_commit: "{"c" * 40}"
report_self_hash: null
---
{body}"""
path.write_text(with_report_self_hash(report), encoding="utf-8")
PY

scripts/review/f-s7/validate-report.py --report "$tmp/valid.md" >/tmp/s7-validate-report-ok.out

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
h = "sha256:" + "1" * 64


def write(rel: str, payload) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")


for topology in ["MoeTiny", "MoeTinyDenseMatched"]:
    for seed in range(5):
        write(f"experiments/S7/runs/{topology}/seed-{seed}/run-log.json", {"run_log_self_hash": h})
        write(
            f"experiments/S7/scores/{topology}/seed-{seed}/score.json",
            {"checkpoint_sha": h, "score_self_hash": h},
        )
for seed in range(5):
    write(f"experiments/S7/switch-stats/seed-{seed}/switch-stats.json", {"bundle_self_hash": h})
write(
    "experiments/S7/dense-vs-moe/comparison.json",
    {
        "matched_bytes_pin": {"matched_bytes_self_hash": h},
        "comparison_self_hash": h,
    },
)
write("experiments/S7/router-collapse/seed-0/sweep.json", {"sweep_self_hash": h})
write("experiments/S7/frontier/frontier.json", {"frontier_self_hash": h})
write("experiments/S7/burn-grad-smoke/expert_block_qat.json", {"smoke_self_hash": h})
write("experiments/S7/oracle-routed/seed-0/oracle.json", {"oracle_self_hash": h})
write("experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json", {"emulator_self_hash": h})
PY

scripts/review/f-s7/validate-report.py --report "$tmp/valid.md" --root "$tmp" >/tmp/s7-validate-report-root-ok.out

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
h = "sha256:" + "1" * 64
path.write_text(
    '{"checkpoint_sha":"%s","score_self_hash":"%s","score_self_hash":"%s"}\n' % (h, h, h),
    encoding="utf-8",
)
PY

if scripts/review/f-s7/validate-report.py --report "$tmp/valid.md" --root "$tmp" >/tmp/s7-validate-report-bad.out 2>&1; then
  echo "expected report duplicate artifact-key validation failure" >&2
  exit 1
fi
rg -n "report artifact reference has duplicate JSON key" /tmp/s7-validate-report-bad.out >/dev/null

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
h = "sha256:" + "1" * 64
path.write_text(
    json.dumps({"checkpoint_sha": h, "score_self_hash": h}, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
PY

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["bpc"] = float("nan")
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-report.py --report "$tmp/valid.md" --root "$tmp" >/tmp/s7-validate-report-bad.out 2>&1; then
  echo "expected report non-finite artifact-reference validation failure" >&2
  exit 1
fi
rg -n "report artifact reference has non-canonical JSON value" /tmp/s7-validate-report-bad.out >/dev/null

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
h = "sha256:" + "1" * 64
path.write_text(
    json.dumps({"checkpoint_sha": h, "score_self_hash": h}, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
PY

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
h = "sha256:" + "1" * 64
path.write_text(
    json.dumps({"checkpoint_sha": h, "score_self_hash": h}, sort_keys=True, indent=2) + "\n",
    encoding="utf-8",
)
PY

if scripts/review/f-s7/validate-report.py --report "$tmp/valid.md" --root "$tmp" >/tmp/s7-validate-report-bad.out 2>&1; then
  echo "expected report noncanonical artifact-reference validation failure" >&2
  exit 1
fi
rg -n "report artifact reference must use canonical JSON bytes" /tmp/s7-validate-report-bad.out >/dev/null

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
h = "sha256:" + "1" * 64
path.write_text(
    json.dumps({"checkpoint_sha": h, "score_self_hash": h}, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
PY

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["checkpoint_sha"] = "sha256:" + "8" * 64
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-report.py --report "$tmp/valid.md" --root "$tmp" >/tmp/s7-validate-report-bad.out 2>&1; then
  echo "expected report checkpoint artifact-reference validation failure" >&2
  exit 1
fi
rg -n "checkpoint_self_hash must match artifact self-hash" /tmp/s7-validate-report-bad.out >/dev/null

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["checkpoint_sha"] = "sha256:" + "1" * 64
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

python3 - "$tmp/experiments/S7/scores/MoeTiny/seed-0/score.json" <<'PY'
from pathlib import Path
import json
import sys

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
data["score_self_hash"] = "sha256:" + "9" * 64
path.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

if scripts/review/f-s7/validate-report.py --report "$tmp/valid.md" --root "$tmp" >/tmp/s7-validate-report-bad.out 2>&1; then
  echo "expected report artifact-reference validation failure" >&2
  exit 1
fi
rg -n "score_self_hash must match artifact self-hash" /tmp/s7-validate-report-bad.out >/dev/null

python3 - "$tmp/valid.md" "$tmp/dense-hyphen-ok.md" "$tmp/bad-dense.md" "$tmp/bad-row.md" "$tmp/bad-hypothesis.md" "$tmp/bad-decision.md" "$tmp/bad-flow.md" "$tmp/bad-body-decision.md" <<'PY'
from pathlib import Path
import hashlib
import re
import sys

valid = Path(sys.argv[1]).read_text(encoding="utf-8")


def with_report_self_hash(text: str) -> str:
    normalized = re.sub(
        r'(?m)^report_self_hash:\s*(?:"sha256:[0-9a-f]{64}"|sha256:[0-9a-f]{64}|null)\s*$',
        "report_self_hash: null",
        text,
        count=1,
    )
    material = (
        b"gbf:gbf-experiments:S7ReportMarkdown:s7_report.v1:1"
        + b"\0"
        + normalized.encode("utf-8")
    )
    expected = "sha256:" + hashlib.sha256(material).hexdigest()
    return re.sub(
        r'(?m)^report_self_hash:\s*(?:"sha256:[0-9a-f]{64}"|sha256:[0-9a-f]{64}|null)\s*$',
        f'report_self_hash: "{expected}"',
        normalized,
        count=1,
    )


dense = valid.replace("s7_outcome: PassClean", "s7_outcome: FailParity")
dense = dense.replace("decision: ProceedToS8", "decision: Proceed-To-S8-DenseOnly")
dense = dense.replace(
    "emulator_one_token_dense_self_hash: null",
    'emulator_one_token_dense_self_hash: "sha256:' + "1" * 64 + '"',
)
dense = dense.replace("ProceedToS8.", "Proceed-To-S8-DenseOnly.")
Path(sys.argv[2]).write_text(with_report_self_hash(dense), encoding="utf-8")

Path(sys.argv[3]).write_text(
    valid.replace("decision: ProceedToS8", "decision: ProceedToS8DenseOnly"),
    encoding="utf-8",
)
Path(sys.argv[4]).write_text(
    valid.replace('score_self_hash: "sha256:' + "1" * 64 + '"', "score_self_hash: null", 1),
    encoding="utf-8",
)
Path(sys.argv[5]).write_text(
    valid.replace("H10 Confirmed", "H10 NotEvaluatedDueToPriorGate(foo)"),
    encoding="utf-8",
)
Path(sys.argv[6]).write_text(
    valid.replace("decision: ProceedToS8", "decision: Proceed-T-o-S8"),
    encoding="utf-8",
)
Path(sys.argv[7]).write_text(
    valid.replace("per_seed_artifacts:\n", "per_seed_artifacts: []\n", 1),
    encoding="utf-8",
)
Path(sys.argv[8]).write_text(
    valid.replace("ProceedToS8.", "ProceedToS8DenseOnly."),
    encoding="utf-8",
)
PY

scripts/review/f-s7/validate-report.py --report "$tmp/dense-hyphen-ok.md" >/tmp/s7-validate-report-dense-ok.out

for bad in "$tmp/bad-dense.md" "$tmp/bad-row.md" "$tmp/bad-hypothesis.md" "$tmp/bad-decision.md" "$tmp/bad-flow.md" "$tmp/bad-body-decision.md"; do
  if scripts/review/f-s7/validate-report.py --report "$bad" >/tmp/s7-validate-report-bad.out 2>&1; then
    echo "expected validator failure for $bad" >&2
    exit 1
  fi
  if ! rg -n "NEEDS_CHANGES" /tmp/s7-validate-report-bad.out >/dev/null; then
    echo "validator failure did not print NEEDS_CHANGES for $bad" >&2
    cat /tmp/s7-validate-report-bad.out >&2
    exit 1
  fi
done
scripts/review/f-s7/validate-report.py --report "$tmp/bad-body-decision.md" >/tmp/s7-validate-report-bad.out 2>&1 || true
rg -n "Decision body must match front matter decision" /tmp/s7-validate-report-bad.out >/dev/null

echo "s7_validate_report_test: ok"

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/s7-emit-report-test.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

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
        final_lm_loss = 1.0
        if topology == "MoeTiny" and seed == 1:
            final_lm_loss = 9.5
        elif topology == "MoeTiny" and seed == 4:
            final_lm_loss = 0.5
        write(
            f"experiments/S7/runs/{topology}/seed-{seed}/run-log.json",
            {
                "completion": {"kind": "completed"},
                "run_log_self_hash": h,
                "losses": [[20000, {"lm_loss_raw": final_lm_loss}]],
            },
        )
        write(
            f"experiments/S7/scores/{topology}/seed-{seed}/score.json",
            {"checkpoint_sha": h, "score_self_hash": h, "bpc": 1.0 + seed / 100.0},
        )
for seed in range(5):
    write(f"experiments/S7/switch-stats/seed-{seed}/switch-stats.json", {"bundle_self_hash": h})
write(
    "experiments/S7/dense-vs-moe/comparison.json",
    {
        "matched_bytes_pin": {"matched_bytes_self_hash": h},
        "comparison_self_hash": h,
        "pareto_verdict": "MoE-dominates",
    },
)
write("experiments/S7/router-collapse/seed-0/sweep.json", {"sweep_self_hash": h})
write("experiments/S7/frontier/frontier.json", {"frontier_self_hash": h})
write("experiments/S7/burn-grad-smoke/expert_block_qat.json", {"smoke_self_hash": h})
write("experiments/S7/oracle-routed/seed-0/oracle.json", {"oracle_self_hash": h})
write("experiments/S7/emulator-one-token/seed-0/MoeTiny/result.json", {"emulator_self_hash": h})
write("experiments/S7/emulator-one-token/seed-0/MoeTinyDenseMatched/result.json", {"emulator_self_hash": h})
PY

scripts/review/f-s7/emit-report.py \
  --root "$tmp" \
  --output docs/experiments/S7-report.md \
  --s7-outcome PassClean \
  --rfc-revision "$(printf 'a%.0s' {1..40})" \
  --predictions-section-hash "sha256:$(printf '2%.0s' {1..64})" \
  --predictions-commit "$(printf 'b%.0s' {1..40})" \
  --first-result-commit "$(printf 'c%.0s' {1..40})" \
  --generated-at "2026-06-25T00:00:00Z" \
  >/tmp/s7-emit-report-ok.out

scripts/review/f-s7/validate-report.py \
  --report "$tmp/docs/experiments/S7-report.md" \
  --root "$tmp" \
  >/tmp/s7-emit-report-validate-ok.out

rg -n 'generated_at: "2026-06-25T00:00:00Z"' "$tmp/docs/experiments/S7-report.md" >/dev/null
rg -n 'H10 Confirmed' "$tmp/docs/experiments/S7-report.md" >/dev/null
rg -n 'H4 Confirmed' "$tmp/docs/experiments/S7-report.md" >/dev/null
rg -n 'MoE final-step lm_loss_raw was noisy across seeds' "$tmp/docs/experiments/S7-report.md" >/dev/null

python3 - "$tmp" <<'PY'
from pathlib import Path
import json
import sys

root = Path(sys.argv[1])
path = root / "experiments/S7/dense-vs-moe/comparison.json"
payload = json.loads(path.read_text(encoding="utf-8"))
payload["pareto_verdict"] = "Dense-wins-under-byte-equivalence"
path.write_text(json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

scripts/review/f-s7/emit-report.py \
  --root "$tmp" \
  --output "$tmp/fail-parity.md" \
  --s7-outcome FailParity \
  --decision ProceedToS8DenseOnly \
  --rfc-revision "$(printf 'a%.0s' {1..40})" \
  --predictions-section-hash "sha256:$(printf '2%.0s' {1..64})" \
  --predictions-commit "$(printf 'b%.0s' {1..40})" \
  --first-result-commit "$(printf 'c%.0s' {1..40})" \
  --generated-at "2026-06-25T00:00:00Z" \
  >/tmp/s7-emit-report-fail-parity.out
rg -n 'H3 Refuted' "$tmp/fail-parity.md" >/dev/null
rg -n 'H4 Refuted' "$tmp/fail-parity.md" >/dev/null
rg -n 'H4 was refuted by the Pareto verdict \(Dense-wins-under-byte-equivalence\)' "$tmp/fail-parity.md" >/dev/null

python3 - "$tmp/docs/experiments/S7-report.md" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
path.write_text(
    text.replace('generated_at: "2026-06-25T00:00:00Z"', 'generated_at: "2099-01-01T00:00:00Z"'),
    encoding="utf-8",
)
PY

scripts/review/f-s7/validate-report.py \
  --report "$tmp/docs/experiments/S7-report.md" \
  --root "$tmp" \
  >/tmp/s7-emit-report-generated-at-ok.out

if scripts/review/f-s7/emit-report.py \
  --root "$tmp" \
  --output "$tmp/bad.md" \
  --s7-outcome PassClean \
  --decision ProceedToS8DenseOnly \
  --rfc-revision "$(printf 'a%.0s' {1..40})" \
  --predictions-section-hash "sha256:$(printf '2%.0s' {1..64})" \
  --predictions-commit "$(printf 'b%.0s' {1..40})" \
  --first-result-commit "$(printf 'c%.0s' {1..40})" \
  >/tmp/s7-emit-report-bad.out 2>&1; then
  echo "expected outcome/decision mismatch to fail" >&2
  exit 1
fi
rg -n "PassClean must emit decision ProceedToS8" /tmp/s7-emit-report-bad.out >/dev/null

rm "$tmp/experiments/S7/frontier/frontier.json"
if scripts/review/f-s7/emit-report.py \
  --root "$tmp" \
  --output "$tmp/missing.md" \
  --s7-outcome PassClean \
  --rfc-revision "$(printf 'a%.0s' {1..40})" \
  --predictions-section-hash "sha256:$(printf '2%.0s' {1..64})" \
  --predictions-commit "$(printf 'b%.0s' {1..40})" \
  --first-result-commit "$(printf 'c%.0s' {1..40})" \
  >/tmp/s7-emit-report-bad.out 2>&1; then
  echo "expected missing artifact to fail" >&2
  exit 1
fi
rg -n "missing artifact: .*experiments/S7/frontier/frontier.json" /tmp/s7-emit-report-bad.out >/dev/null

echo "s7_emit_report_test: ok"

#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

packet="docs/review/f-b5"

for required in \
  "$packet/SUMMARY.md" \
  "$packet/reject-class-table.md" \
  "$packet/op-signature-table.md" \
  "$packet/reduction-site-join.md" \
  "$packet/bit_exact_equivalence.toml" \
  "$packet/golden/driver_evidence.toml"
do
  if [[ ! -f "$required" ]]; then
    echo "missing review-packet file: $required" >&2
    exit 1
  fi
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

scripts/review/f-b5/regen.sh "$tmp"
diff -ru "$packet/golden" "$tmp/golden"
diff -u "$packet/bit_exact_equivalence.toml" "$tmp/bit_exact_equivalence.toml"

python3 - <<'PY'
from pathlib import Path
import json
import re
import sys

root = Path(".")
packet = root / "docs" / "review" / "f-b5"
reject_table = (packet / "reject-class-table.md").read_text()
op_table = (packet / "op-signature-table.md").read_text()
join_doc = (packet / "reduction-site-join.md").read_text()
summary = (packet / "SUMMARY.md").read_text()
bit_exact = (packet / "bit_exact_equivalence.toml").read_text()
driver_evidence = (packet / "golden" / "driver_evidence.toml").read_text()

def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)

reject_rows = [line for line in reject_table.splitlines() if line.startswith("| IIR-Reject-")]
if len(reject_rows) != 36:
    fail(f"reject-class-table.md has {len(reject_rows)} reject rows, expected 36")

reject_dirs = sorted((root / "fixtures" / "infer_ir" / "reject").iterdir())
if len([d for d in reject_dirs if d.is_dir()]) != 36:
    fail("fixtures/infer_ir/reject does not contain 36 reject fixture directories")

for fixture_dir in reject_dirs:
    if not fixture_dir.is_dir():
        continue
    expected = fixture_dir / "expected.toml"
    text = expected.read_text()
    code_match = re.search(r'^code = "([^"]+)"$', text, re.MULTILINE)
    severity_match = re.search(r'^severity = "([^"]+)"$', text, re.MULTILINE)
    if code_match is None:
        fail(f"{expected} does not declare code")
    if severity_match is None:
        fail(f"{expected} does not declare severity")
    rel = fixture_dir.as_posix()
    code = code_match.group(1)
    severity = severity_match.group(1)
    if rel not in reject_table:
        fail(f"reject-class-table.md is missing fixture {rel}")
    if code not in reject_table:
        fail(f"reject-class-table.md is missing diagnostic {code}")
    if severity != "Hard":
        fail(f"{expected} expected Hard severity, got {severity}")

infer_ops = [
    "Classify",
    "CombineResidual",
    "DecodeToken",
    "Embedding",
    "ExpertMatVec",
    "FfnActivation",
    "Norm",
    "RouteTop1",
    "RouterMatVec",
    "SelectExpertTop1",
    "SequenceRead",
    "SequenceStep",
    "SequenceWrite",
]
op_rows = [line for line in op_table.splitlines() if line.startswith("| `")]
if len(op_rows) != len(infer_ops):
    fail(f"op-signature-table.md has {len(op_rows)} op rows, expected {len(infer_ops)}")
for op in infer_ops:
    if f"| `{op}` |" not in op_table:
        fail(f"op-signature-table.md is missing {op}")

for needle in [
    "router.<layer>",
    "expert.<layer>.<expert>.<slot>",
    "norm.<plan>",
    "classify",
    "InferIrReductionSiteMissing",
    "F-B5 does not independently invent",
]:
    if needle not in join_doc:
        fail(f"reduction-site-join.md is missing {needle}")

for fixture in ["dense_toy0", "routed_basic", "mixed_topology"]:
    fixture_dir = packet / "golden" / fixture
    for filename in ["infer_ir.json", "hashes.toml", "anchor_ids.toml"]:
        path = packet / "golden" / fixture / filename
        if not path.exists():
            fail(f"missing golden file {path}")
    report = json.loads((fixture_dir / "infer_ir.json").read_text())
    try:
        result = report["result"]
        product = result["product"]
    except KeyError as error:
        fail(f"{fixture}/infer_ir.json is not product-bearing: missing {error}")
    if report.get("outcome") != "Passed":
        fail(f"{fixture}/infer_ir.json outcome is not Passed")
    if result.get("infer_ir_self_hash") not in (fixture_dir / "hashes.toml").read_text():
        fail(f"{fixture}/hashes.toml does not pin report infer_ir_self_hash")
    anchors = product.get("anchors")
    nodes = product.get("nodes")
    if not isinstance(anchors, dict) or not isinstance(nodes, list) or not anchors:
        fail(f"{fixture}/infer_ir.json has no product anchors/nodes")
    anchor_text = (fixture_dir / "anchor_ids.toml").read_text()
    if 'status = "exported"' not in anchor_text:
        fail(f"{fixture}/anchor_ids.toml is not marked exported")
    if anchor_text.count("[[anchors]]") != len(nodes):
        fail(f"{fixture}/anchor_ids.toml anchor count does not match node count")
    for node in nodes:
        node_id = str(node["node_id"])
        anchor = anchors.get(node_id)
        if anchor is None:
            fail(f"{fixture}/infer_ir.json missing anchor for node {node_id}")
        if anchor["anchor_id"] not in anchor_text:
            fail(f"{fixture}/anchor_ids.toml missing anchor id for node {node_id}")
    if fixture not in bit_exact:
        fail(f"bit_exact_equivalence.toml is missing {fixture}")
    if fixture not in summary:
        fail(f"SUMMARY.md is missing {fixture}")

for name, text in {
    "SUMMARY.md": summary,
    "bit_exact_equivalence.toml": bit_exact,
    "golden/driver_evidence.toml": driver_evidence,
}.items():
    for forbidden in [
        "blocked_by_" + "bd_37d1",
        "bd-" + "37d1 has not yet landed",
        "missing_" + "export_script",
        "pending_fixture_" + "export_script",
        "Remaining " + "Export Gap",
    ]:
        if forbidden in text:
            fail(f"{name} still carries blocker language: {forbidden}")

if 'driver_materialization = "exported"' not in bit_exact:
    fail("bit_exact_equivalence.toml must record exported driver materialization")
if bit_exact.count("[[samples]]") != 6:
    fail("bit_exact_equivalence.toml must include two BitExact samples per fixture")
if "run_stage3_emits_report_and_writes_cache" not in driver_evidence:
    fail("driver_evidence.toml must name the Stage 3 report emission test")
if 'fixture_export_status = "exported"' not in driver_evidence:
    fail("driver_evidence.toml must mark fixture export as exported")
PY

if [[ "${GBF_REVIEW_F_B5_RUN_CARGO:-0}" == "1" ]]; then
  scripts/e2e/stage3.sh
  cargo test -q -p gbf-codegen --lib fixture_infer_ir
  cargo test -q -p gbf-codegen --lib reject_infer_ir
  cargo test -q -p gbf-codegen --features semantic_equivalence_check --lib fixture_infer_ir_fixture_semantic_equivalence_bit_exact
fi

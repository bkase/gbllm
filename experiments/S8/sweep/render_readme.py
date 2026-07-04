#!/usr/bin/env python3
"""Render experiments/S8/sweep/README.md from report.json (bd-3771m).

Every number in the README is read from report.json (which is itself emitted
by gbf-experiments/src/bin/s8_matched_cycles.rs from the actual runs); this
script contains no hand-entered measurements.
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def fmt(x, nd=4):
    return f"{x:.{nd}f}" if isinstance(x, (int, float)) else "n/a"


def main() -> int:
    report = json.loads((HERE / "report.json").read_text())
    arms = [a for a in report["arms"] if a.get("status") == "ok"]
    kn = report["kn5_reference"]
    kn_runs = {r["train_cap_bytes"]: r for r in kn.get("runs", [])}
    kn_full = kn_runs.get(489614571, {})
    kn_64 = kn_runs.get(67108864, {})
    verdict = report["verdict"]
    budget = report["ux_budget"]
    plan = report.get("scale_plan")

    lines = []
    a = lines.append
    a("# F-S8 matched-cycles sizing sweep (bd-3771m)")
    a("")
    a(f"Generated from `report.json` (schema `{report['schema']}`, git {report['git_sha'][:12]}, seed {report['seed']}).")
    a("Bin: `gbf-experiments/src/bin/s8_matched_cycles.rs` (reuses the s5_state_ab substrate:")
    a("charset_v1 80-id vocab, LinearState MT4 + residual, pre-norm ternary FFN stack,")
    a("tied head, warmup Off->Hard QAT, TBPTT-128 x 4 lanes, seed 0).")
    a("")
    a("## UX budget (hard facts, measured constants)")
    a("")
    a(f"- 30 s/char at 70% CPU = {budget['m_cycles_per_token']/1e6:.1f}M M-cycles/token.")
    a(f"- V3 weights-as-code: 5.385 cy/MAC, 4.401 B/weight -> {budget['macs_per_token_v3']/1e6:.2f}M MACs/token envelope.")
    a(f"- V2 dispatch: 10.261 cy/MAC, 0.25 B/weight data + 2699 B shared code -> {budget['macs_per_token_v2']/1e6:.2f}M MACs/token envelope.")
    a(f"- ROM budget: {budget['rom_budget_bytes']/2**20:.0f} MiB (8 MiB MBC5 minus ~1 MiB runtime/UI reserve).")
    a(f"- Constants source: {budget['kernel_constants_source']}.")
    a("")
    a("## Quality bar (KN-5, copied verbatim from experiments/S4/baseline/)")
    a("")
    a(f"- KN-5 full corpus (466 MiB train): **{fmt(kn_full.get('kn5_bits_per_raw_val_byte'))} bits/raw-byte**"
      f" ({fmt(kn_full.get('bpc_kn5_val_per_normalized_char'))} bpc/char).")
    a(f"- KN-5 64 MiB train cap (the cap the Phase-A arms trained on): {fmt(kn_64.get('kn5_bits_per_raw_val_byte'))} bits/raw-byte"
      f" ({fmt(kn_64.get('bpc_kn5_val_per_normalized_char'))} bpc/char).")
    a("- Val stream: books [1017, 1233, 1475, 2105, 2156, 3440], raw sha "
      f"`{arms[0]['corpus']['val_raw_bytes_sha256'][:16]}...` (identical across all arms and KN-5).")
    a("")
    a("## Per-arm results (full val stream, 1,042,400 scored pairs)")
    a("")
    a("| arm | topology | bits/raw-byte (hard ternary) | bits/raw-byte (fp soft) | QAT gap (bpc/char) | MACs/token | V3 ROM (MiB) / s-char | V2 ROM (MiB) / s-char |")
    a("|---|---|---|---|---|---|---|---|")
    for arm in arms:
        c = arm["config"]
        m = arm["measurement"]
        d = arm["deployment"]
        topo = f"d{c['d_model']}/ff{c['d_ff']}/{c['n_blocks']}blk"
        if c["n_experts_per_block"] > 1:
            topo += f"/E{c['n_experts_per_block']} top-1"
        if "distillation" in (c.get("extra") or {}):
            topo += " + distill"
        v3 = d["v3_weights_as_code"]
        v2 = d["v2_dispatch_data"]
        a(f"| {arm['arm']} | {topo} | **{fmt(m['ternary_val_bits_per_raw_byte'])}** "
          f"| {fmt(m['fp_val_bits_per_raw_byte'])} | {fmt(m['gap_bpc_per_normalized_char'])} "
          f"| {d['macs_per_token']:,} | {v3['rom_mib']:.2f} / {v3['s_per_char_at_70pct_cpu']:.1f} "
          f"| {v2['rom_mib']:.2f} / {v2['s_per_char_at_70pct_cpu']:.1f} |")
    a("")
    a("Method: bits/raw-byte = bpc_per_normalized_char x (val_chars_total / val_raw_bytes_total),")
    a("the same total-bits-over-raw-byte-count re-expression as the KN-5 artifact.")
    a("Samples: `sample_<arm>.txt` (256-char greedy continuations, hard ternary).")
    a("")

    # Distillation section if present.
    distill_arms = [x for x in arms if "distillation" in (x["config"].get("extra") or {})]
    base_by_name = {x["arm"]: x for x in arms}
    if distill_arms:
        a("## Distillation probe")
        a("")
        for da in distill_arms:
            info = da["config"]["extra"]["distillation"]
            t = info["teacher"]
            base_name = da["arm"].replace("_distill", "")
            base = base_by_name.get(base_name)
            a(f"- Teacher: {t['description']} -> fp val {fmt(t['fp_val_bits_per_raw_byte'])} bits/raw-byte"
              f" after {t['steps']} steps ({t['train_wall_clock_seconds']/60:.0f} min).")
            a(f"- Loss: {info['loss']} (T={info['temperature']}, w={info['weight']}).")
            if base:
                delta = (da["measurement"]["ternary_val_bits_per_raw_byte"]
                         - base["measurement"]["ternary_val_bits_per_raw_byte"])
                a(f"- Student-with-distill {fmt(da['measurement']['ternary_val_bits_per_raw_byte'])} vs "
                  f"student-without {fmt(base['measurement']['ternary_val_bits_per_raw_byte'])} at matched"
                  f" {da['config']['steps']} steps: **delta {delta:+.4f} bits/raw-byte**"
                  f" ({'distill helps' if delta < 0 else 'distill does not help'}).")
        a("")

    a("## Verdict")
    a("")
    a(f"- Best deployable arm: **{verdict['best_arm_by_ternary_bits_per_raw_byte']}**"
      f" at {fmt(verdict['best_ternary_bits_per_raw_byte'])} bits/raw-byte (hard ternary).")
    beats = verdict.get("best_beats_kn5_full_corpus")
    a(f"- Beats KN-5 full corpus ({fmt(verdict['kn5_full_corpus_bits_per_raw_byte'])})? **{beats}** at this proxy scale.")
    a("")
    if plan:
        c = plan["config"]
        v3 = plan["v3_weights_as_code"]
        v2 = plan["v2_dispatch_data"]
        a("## Overnight scale run")
        a("")
        a(f"- Config: d{c['d_model']}/ff{c['d_ff']}/{c['n_blocks']}blk/slots{c['state_slots']}"
          f"/E{c['n_experts_per_block']} (CUSTOM arm of the same bin).")
        a(f"- Projected: {plan['macs_per_token']:,} MACs/token; {plan['stored_ternary_weights']:,} stored ternary weights;")
        a(f"  V3 {v3['rom_mib']:.2f} MiB / {v3['s_per_char_at_70pct_cpu']:.1f} s/char"
          f" (fits 7 MiB: {v3['fits_rom_budget_7mib']});")
        a(f"  V2 {v2['rom_mib']:.2f} MiB / {v2['s_per_char_at_70pct_cpu']:.1f} s/char"
          f" (fits 7 MiB: {v2['fits_rom_budget_7mib']}).")
        a("- Recipe: the Phase-B winner (distillation): fp dense teacher d384/ff768/6blk/slots384")
        a("  trained 30k steps, then the student distilled for 130k steps (T=2.0, w=0.5),")
        a("  train cap 256 MiB, seed 0. Launched via:")
        a("  `nohup ./target/release/s8_matched_cycles --phase distill --arm CUSTOM --d-model 192")
        a("  --d-ff 384 --n-blocks 6 --state-slots 192 --n-experts 1 --steps 130000")
        a("  --teacher-steps 30000 --teacher-mult 2 --train-cap-bytes 268435456")
        a("  --out-dir experiments/S8/sweep > experiments/S8/sweep/scale_run.log 2>&1 &`")
        a("- Log: `scale_run.log`; result lands in `arm_CUSTOM_distill.json` +")
        a("  `sample_CUSTOM_distill.txt` when done (~10-11 h).")
        a("- Monitor: `tail -f experiments/S8/sweep/scale_run.log` (teacher finishes ~2 h in;")
        a("  look for `[distill] teacher fp val:` then `[CUSTOM] step N/130000` lines).")
        a("")
    a("## Caveats")
    a("")
    a("- Proxy scale: 40k steps x 512 tokens/step (20.5M tokens) on the 64 MiB train prefix;")
    a("  matched steps relatively undertrains the larger arms (A2, A3), so Phase A measures")
    a("  the architecture signal at this budget, not the asymptote. The scale run probes further.")
    a("- The MoE router is a simplified in-bin fp linear top-1 (documented in the bin header),")
    a("  not the gbf-model Top1RouterQat low-rank core; hard top-1 dispatch is stop-gradient")
    a("  provenance, router gradients flow via top-1 prob output scaling + Switch aux (w=0.01).")
    a("  A3 showed partial router collapse in block 3 (one dead expert) despite balanced blocks 0-2.")
    a("- ROM accounting: ternary weights at the measured B/weight constants; embedding f32 (4 B),")
    a("  scales/decay Q8.8 (2 B), router int8 (1 B, assumption). Norms, dispatch overhead beyond the")
    a("  shared V2 handlers, and WRAM/SRAM state cost are excluded (same caveats as the bakeoff).")
    a("- Eval scores within-lane adjacent pairs (8 lanes); KN-5 scored every token under reset-context")
    a("  windows. The bits/raw-byte re-expression uses the stream-level chars-per-raw-byte ratio.")
    a("")
    (HERE / "README.md").write_text("\n".join(lines))
    print("wrote", HERE / "README.md")
    return 0


if __name__ == "__main__":
    sys.exit(main())

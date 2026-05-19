F-B6/F-B7 closure scope note
============================

This review packet proves the F-B6/F-B7 closure surface only: Stage 4
observation-plan outputs, Stage 5 range-plan/certificate outputs, diagnostic
coverage metadata, packet verification, and independent `gbf-verify` range
certificate checks.

The bd-1t48 §17 item 4 wording is intentionally narrowed here. The packet may
name the typed `PlanningReady(g, o, r)` join state, but executable F-B8/Stage 6
consumption is deferred to `bd-2k0`. No Stage 6 driver, `PlanningReady`
consumer, or F-B8 smoke test is claimed by this packet.

CI artifact attachment and `.github` workflow wiring are out of scope for this
review-fix bead. `verify-packet.sh` emits a local `closure-packet.tar.gz`; a
future CI integration bead owns attaching that packet as a CI artifact.

Workspace `cargo test --workspace --all-features` remains blocked by the known
`gbf-experiments` S2 feature mutex. The accepted gates for this packet are the
focused `gbf-codegen` coverage-matrix test and `scripts/review/f-b6-f-b7/verify-packet.sh`.

Historical amendment comments mentioned standalone `cache-replay.sh` and
`check-§20-conformance.sh` scripts. Those names are superseded by the current
durable packet entry points: cache/rewrap evidence and §20 reconciliation are
checked inline by `check-checklist.sh` and validated through `verify-packet.sh`.

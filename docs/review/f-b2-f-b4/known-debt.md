# Known Debt

| Item | Status | Owner / removal condition |
| --- | --- | --- |
| `RepairProposal(_)` provenance | Deferred | Forbidden in F-B2. F-B16 (`FeasibilityRefinementLoop`) adds proposal IDs, accept/reject logic, and `repair_report.json`. |
| `gbf-migrate` lossy/on-disk migration | Deferred | F-A6b owns migration. F-B2 accepts only registered, lossless, in-memory adapters and otherwise fails closed. |
| Real `QuantGraphBudgetSource` implementation | Deferred | F-B3 owns the real Stage 1 producer. F-B4 tests may use synthetic sources until then. |
| `static_budget.v1` review artifacts and final chunk scripts | Deferred | T-B4.13 (`bd-3fug`) extends this packet with static-budget goldens and full shared regen/verify. |
| RuntimeChromeBudget production | Out of this bead | Runtime-shell build owns production; F-B2 only references the Bringup fixture path for review context. |
| Cycle envelopes in budget reports | Deferred | F-B14 owns `schedule_cost.json`; F-B4 static budgets must not claim measured cycles. |
| Soft diagnostics | Accepted schema variant, unused here | F-B2/F-B4 report validators reject Soft diagnostics for these v1 review artifacts. |

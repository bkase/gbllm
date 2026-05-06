# N=128 Frame Trace Summary

The full N=128 frame trace is generated in memory during packet regeneration and consumed by the report/build checks, but it is not checked in. The pretty JSON trace is about 45 MiB and adds little review value beyond the aggregate invariants below.

## What This Means

- The L4 frame-service proof does not depend on a committed raw trace blob.
- `realism_report.v1.json` remains the machine-checked source for the hard gate.
- This summary pins the reviewer-facing shape: real VBlank events, scheduler/widget service for every gated frame, and bounded liveness gaps.

## Counts

| Metric | Value |
| --- | ---: |
| Matrix size | 128 |
| Transient frame events generated | 258397 |
| VBlankFired events | 51687 |
| WidgetTickDispatched events | 51686 |
| SchedulerServicedFrame events | 51686 |
| YieldReturnedToScheduler events | 51686 |
| ComputeProgressEpochAdvanced events | 51652 |
| Frame service misses | 0 |
| Widget updates | 51686 |
| Scheduler services | 51686 |
| Max no-progress frames | 1 |
| Yield while BankLease active | 0 |
| Harness pause while BankLease active | 0 |
| First VBlank frame in trace | 2 |
| Last VBlank frame in trace | 51688 |
| First event M-cycle | 5876522 |
| Last event M-cycle | 913275939 |

## Frame-Service Margin

| Metric | Value |
| --- | ---: |
| Remaining-frame samples | 51686 |
| Min remaining M-cycles | 602 |
| Mean remaining M-cycles | 14084.51 |
| P99 remaining M-cycles | 17439 |
| Max remaining M-cycles | 17509 |

## Reproduction

```bash
cargo run -p gbf-test --bin f_b1_regen
scripts/review/f-b1/verify-packet.sh
```

For interactive inspection, initialize the checked ROM with `gbf-debug` and run `scripts/review/f-b1/debug-safe-point.js`; that script checks the VBlank handler, copy/compute/tile safe points, and HRAM service counters.

---
name: logging-bead-closure
description: Use when implementing, reviewing, or closing gbllm structured logging beads, including tracing event schemas, training/export/preflight logs, subscriber capture tests, and logging adoption claims.
---

# Logging Bead Closure

Use this for structured logging schema/helper work and adoption by training, data, model, CLI, export, compiler, or runtime producers.

## Claim Boundaries

- Closure must distinguish a logging schema/helper contract from adoption by real producers.
- If producer adoption is incomplete, create or cite a named follow-up bead before closing.
- Do not claim observability performance targets, such as logging overhead percentage, unless a benchmark or explicit gate measures them.

## Event Shape Rules

- Tests for logging event shape should include at least one subscriber-level capture of actual `tracing` fields, not only a mirrored test collector or source grep.
- Canonical event names used by downstream tests should be constants in code.
- Do not introduce direct `tracing::*` call sites with ad hoc event names or load-bearing message strings.

## Closure Evidence

- Use a support matrix when a logging bead covers both schema helpers and producer adoption.
- Every claimed producer path needs either a subscriber capture test, an integration test, or an explicit moved-to bead.

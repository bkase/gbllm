---
name: feedback-harness-reflection
description: "Reflect on recent user feedback, review findings, transcript mistakes, bead closure problems, or repeated agent issues to extract reusable project guides and automated sensors. Use when the user asks to turn feedback into guardrails, harness checks, skills, AGENTS instructions, tests, pre-commit/CI checks, or next-time prevention mechanisms."
---

# Feedback Harness Reflection

## Workflow

Start from the concrete feedback, not from a generic best-practices list. Identify each item that exposed an agent failure, project-process gap, missing acceptance gate, unsupported public behavior, brittle test method, or manual review burden.

For each item, write one row:

```markdown
| Feedback item | Root cause | Guide | Sensor | Owner/gate |
| --- | --- | --- | --- | --- |
```

Classify the output:

- **Guide**: an instruction, checklist, skill update, AGENTS rule, or closure template that changes how future agents think and decide.
- **Sensor**: an executable check, unit test, integration test, linter, CI/pre-commit step, or script that fails before the same mistake reaches review.
- **Owner/gate**: the bead, file, test command, CI job, or skill responsible for keeping the guide or sensor alive.
- **No action**: use only when the feedback is wrong, obsolete, not reproducible, or too subjective to automate; explain why.

## Extraction Prompts

Ask these questions against the transcript or review:

- What claim did the agent make that was not proven by a test or command?
- What behavior was advertised by a public API but unsupported in a path such as training, export, CLI, or runtime?
- What acceptance criterion moved to another bead without a verifiable owner?
- What reviewer-only observation could have been detected by a deterministic grep, AST check, schema check, or focused test?
- What distinction did the agent blur, such as guide versus implementation, scalar core versus adapter, fixed behavior versus future variant, or artifact contract versus pre-export shape?
- What command, closure format, or workflow step was easy to misuse and should become a template or script?

## Guide Rules

Make guides short, imperative, and local to the workflow they protect. Prefer updating an existing project-local skill when the issue belongs to that domain; create a new project-local skill only when the reflection workflow is reusable across domains.

If the guide changes skill behavior, also use `skill-creator`. If it changes QAT bead closure, also use `qat-bead-closure`.

Good guides include:

- A support matrix for multi-path behavior.
- A claim-to-gate matrix for closure claims.
- A no-future-variant rule for public enums, modes, strategies, or schedules.
- A comment or closure workflow when shell quoting, markdown, or bead metadata can corrupt evidence.

## Sensor Rules

Prefer sensors that are deterministic, cheap, and close to the mistake:

- Architecture invariants belong in architecture tests.
- Closure and bead metadata checks belong in a script that reads `.beads/issues.jsonl`.
- Code semantics belong in unit or integration tests.
- Workflow checks belong in pre-commit and CI only after they run locally and have low false-positive risk.

Every sensor should have:

- A precise failure message that tells the next agent what to fix.
- At least one realistic bad pattern it would catch.
- A narrow scan scope to avoid policing unrelated code.
- A stable command listed in the guide or closure matrix.

Avoid sensors for subjective taste, broad style preferences, or speculative future architecture. Turn those into guides or beads instead.

## Implementation Loop

1. Create or claim a bead when working in a beads-managed repo.
2. Draft the feedback table and decide which rows get guides, sensors, both, or no action.
3. Patch the smallest local guide surface: usually `.agents/skills/<skill>/SKILL.md`, `AGENTS.md`, or a closure template.
4. Add executable sensors where they naturally fit: tests, scripts, pre-commit, or CI.
5. Run the new sensor directly, then the nearest existing gate.
6. Close/sync the bead and commit using the repo workflow.

## Response Shape

When reporting back, lead with what was added or intentionally skipped. Include the commands that proved the sensors work and name any residual manual judgment that remains.

# AI Agent Engineering Organization

A hierarchical multi-agent system for executing software engineering work in a monorepo, modeled after the Linux kernel lieutenant tree. Humans create beads (issues/specs), the agent org does everything else.

**Design sources**: CAID paper (arXiv 2603.21489), OpenAI harness engineering, Martin Fowler's guides+sensors framework, Jamon Holmgren's Night Shift workflow, large engineering org practices (Google, Linux kernel), Erlang/OTP supervision model, ACP (Agent Client Protocol).

**Runtime**: Gleam application on BEAM. All agent runtimes sit behind a common `SessionAdapter` boundary owned by the control plane.

- **Claude managers**: Claude Code `SessionAdapter` over ACP
- **Engineers**: Pi `SessionAdapter` with Codex OAuth backend
- **Gemini reviewers/meta-agents**: one-shot `gemini -p` CLI invocations (`gemini-3.1-pro-preview`)

SessionAdapter remains the control-plane abstraction, but this project intentionally pins exact runtime surfaces per role. Runtime surface changes happen only via daytime control-plane releases and shadow/replay validation. See §6 for the full runtime architecture.

### Subsystem Status

| Subsystem                        | Status          | Notes                                    |
| -------------------------------- | --------------- | ---------------------------------------- |
| Philosophy / principles          | **Planned**     | Ready to implement                       |
| CODEOWNERS / team boundaries     | **Planned**     | Awaiting actual crate scaffolding        |
| Harness guides (AGENTS.md, ADRs) | **Planned**     | Write during Phase 0                     |
| Harness sensors (Tier 1-2)       | **Planned**     | Build in Phase 0                         |
| Risk classes / autonomy ladder   | **Planned**     | Enforce from day 1                       |
| Task manifests                   | **Planned**     | Build in Phase 0                         |
| Gleam supervision tree           | **Planned**     | Phase 0: flat; Phase 1: full tree        |
| Claude Code ACP integration      | **Planned**     | Phase 0                                  |
| Pi + Codex engineer harness      | **Planned**     | Phase 0                                  |
| Gemini CLI review agents         | **Speculative** | Phase 2-3                                |
| Deterministic review checks      | **Planned**     | Phase 1                                  |
| Inferential review (Gemini)      | **Speculative** | Phase 2-3                                |
| Merge trains                     | **Speculative** | Phase 3                                  |
| Epoch tagging                    | **Planned**     | Phase 1                                  |
| Lieutenant tree                  | **Planned**     | Phase 1                                  |
| Meta-agents (retro, CI health)   | **Speculative** | Phase 2                                  |
| Entropy GC agents                | **Speculative** | Phase 2                                  |
| Benchmark suite                  | **Planned**     | Build in Phase 0, use to gate promotions |
| Command ledger / idempotency     | **Planned**     | Phase 0                                  |
| Event-driven monitors            | **Planned**     | Phase 1                                  |

---

## 1. Philosophy

### Core Principles

1. **Human attention is scarce; token spend is justified only when it reduces escaped defects, human review time, or cycle time.** Burn tokens on quality — but measure whether the spend actually pays off.
2. **Constrain the solution space.** Agents in a tight harness outperform agents in open space. More constraints = more productive, not less.
3. **Designed for unattended execution.** The system runs overnight. If it needs babysitting, the harness is broken.
4. **When defects recur, fix the system that permits them; when they are local, fix the code and record the class of failure.** Not every mistake is a harness gap. Use the failure taxonomy (below) to decide where to invest.
5. **The system gets better every day.** Each morning the human improves the harness. Each night the agents run better than the night before. The human's job converges toward pure specification and architecture.
6. **Repo-first knowledge.** From the agent's perspective, anything it can't access in-context doesn't exist. All decisions live in the repo as ADRs, golden path docs, and structural tests — never in Slack, memory, or people's heads.
7. **Degrade for low-severity local failures; fail closed for integrity, safety, or trust-boundary violations.** Not everything should be retried. Some things should halt.
8. **Evidence-first, risk-bounded automation.** Routine mechanical issues should be caught automatically. Human review should mostly arbitrate ambiguity, architecture, and risk acceptance. Autonomy is earned per risk class, not granted globally.
9. **LLMs propose; the control plane decides.** Any output that changes scheduling, risk class, review scope, or side effects must be emitted as typed proposal IR and validated by deterministic services before execution. Managers and reviewers generate proposals; only deterministic control-plane services may allocate leases, change risk/autonomy ceilings, enqueue side effects, or merge branches.
10. **Pin control-plane and credential semantics per epoch.** Do not vary requested model, resolved runtime model, auth mode, effort level, extended context flags, harness, prompts, rules, provider tool versions, and org topology at the same time. Hold one variable constant per phase so you can infer which variable caused what.
11. **Prefer Git-native provenance over parallel durability planes.** Durable, human-facing evidence for merged or archived attempts lives on Git objects via `git notes`. SQLite coordinates live work; it is not a second historical source of truth for merged beads.

### Failure Taxonomy

When something goes wrong, classify it before acting:

| Class                       | Example                                             | Response                                                     |
| --------------------------- | --------------------------------------------------- | ------------------------------------------------------------ |
| Local implementation defect | Off-by-one in byte math                             | Fix the code. No systemic change needed.                     |
| Missing sensor/invariant    | Budget overflow not caught until epoch              | Add sensor to Tier 2.                                        |
| Missing guide/documentation | Agent reinvented a pattern that exists              | Add to AGENTS.md golden path.                                |
| Bad task split              | Bead too large, agent ran out of context            | Split the bead. Improve manifest heuristics.                 |
| Bad autonomy assignment     | High-risk change auto-merged, introduced regression | Tighten risk class for that bead type.                       |
| Model/harness weakness      | Agent can't handle Burn derive syntax               | Add examples to context or switch model for that task class. |
| Spec ambiguity              | Bead description was underspecified                 | Improve bead, flag for human review.                         |

### Rule Budget

Every new rule added to AGENTS.md, INCIDENTS.md, or harness templates must:

- Cite at least 2 recurring incidents (not one-offs)
- Have an explicit owner (which team added it)
- Have a success metric (what does "this rule is working" look like?)
- Sunset after 3 epochs if it never fires or never prevents a recurrence

Without this, the golden path becomes a swamp of hyper-specific edge cases.

### Agent = Model + Harness

The harness has two control systems (Martin Fowler framing):

- **Guides (feedforward)**: Anticipate behavior before action. Architecture docs, golden paths, dependency rules, task manifests, ADRs.
- **Sensors (feedback)**: Observe post-action results and enable self-correction. Tests, linters, structural checks, review agents.

Neither alone is sufficient. Guides without sensors means agents produce plausible but unvalidated code. Sensors without guides means agents waste cycles exploring dead ends.

---

## 2. Team Structure

### The Lieutenant Tree

Modeled after the Linux kernel maintainer hierarchy. Four teams aligned to crate boundaries (Conway's Law — org structure matches system architecture exactly).

```
                       YOU (Human)
                    Create beads, review morning reports,
                    fix harness, write ADRs
                            |
                       TOP MANAGER [opus]
                    Owns: main, Cargo.toml, workspace config
                    Merges team branches into main
                    Declares epochs, handles cross-crate splits
                            |
            +---------------+---------------+---------------+
            |               |               |               |
       LT-MODEL [opus] LT-CONTRACTS [opus] LT-TRAIN [opus] LT-QA [opus]
       team/model       team/contracts      team/train       team/qa
       2-5 eng [sonnet] 2-3 eng [sonnet]   3-5 eng [sonnet] 2-3 eng [sonnet]
       +monitor agent   +monitor agent      +monitor agent   +monitor agent
```

### Team Ownership (CODEOWNERS)

Every file has one **primary owner** and zero or more **required reviewers**. Primary owner can merge; required reviewers must approve but cannot block indefinitely. This avoids false exclusivity while preserving clarity.

The project scaffolds 23 crates (bead T0.1), but only ~5 are active in the training-contract epic. The remaining crates are mapped to teams for when work reaches them.

```
# Active (training-contract epic)
# Format: path  primary-owner  [required-reviewers]
/gbf-model/**          @lt-model
/gbf-artifact/**       @lt-contracts
/gbf-policy/**         @lt-contracts
/gbf-train/**          @lt-train

# Tests: feature teams may modify tests relevant to their area.
# QA owns test infrastructure and cross-cutting harnesses.
# QA review required for broad integration-test changes.
/tests/infrastructure/**  @lt-qa
/tests/fixtures/**        @lt-qa
/tests/e2e/**             @lt-qa     [@lt-model, @lt-train]
/tests/unit/model/**      @lt-model  [@lt-qa]
/tests/unit/train/**      @lt-train  [@lt-qa]
/tests/unit/artifact/**   @lt-contracts [@lt-qa]

# Per-crate Cargo.toml — primary owner is the crate's team,
# top-manager reviews to catch workspace-level breakage
/gbf-model/Cargo.toml     @lt-model  [@top-manager]
/gbf-artifact/Cargo.toml  @lt-contracts [@top-manager]
/gbf-train/Cargo.toml     @lt-train  [@top-manager]

# Foundation / shared (owned by Contracts until volume justifies LT-PLATFORM)
/gbf-foundation/**     @lt-contracts
/gbf-hw/**             @lt-contracts
/gbf-abi/**            @lt-contracts
/gbf-verify/**         @lt-contracts

# Runtime / compiler (dormant until those milestones; top-manager holds)
/gbf-runtime/**        @top-manager
/gbf-codegen/**        @top-manager
/gbf-ir/**             @top-manager
/gbf-asm/**            @top-manager

# Workspace root
/Cargo.toml            @top-manager
/Cargo.lock            @top-manager
/AGENTS.md             @top-manager
/docs/decisions/**     @top-manager
```

**When to split further:** If foundation/hw/abi crates accumulate >10 active beads, split into a dedicated LT-PLATFORM team. If runtime/compiler work begins (M3+), create LT-COMPILER and LT-RUNTIME teams.

### Team Assignments

| Team          | Crate Ownership                                                                                     | Features                                     | Focus                                           |
| ------------- | --------------------------------------------------------------------------------------------------- | -------------------------------------------- | ----------------------------------------------- |
| **Model**     | `gbf-model/**`                                                                                      | F1 (QAT), F6 (MoE), F9.1 (ExpertShapePolicy) | QAT modules, model architecture                 |
| **Contracts** | `gbf-artifact/**`, `gbf-policy/**`, `gbf-foundation/**`, `gbf-hw/**`, `gbf-abi/**`, `gbf-verify/**` | F2 (Budget), F3 (Ternary), T0.3, T3.5        | Types, contracts, byte math, shared foundations |
| **Train**     | `gbf-train/**`                                                                                      | F4, F5, F7, F8, T1.7-T1.8, F9.2-F9.3         | Training loop, loss, router, export             |
| **QA**        | `tests/**`                                                                                          | F10                                          | Test infrastructure, fixtures, E2E              |

---

## 3. The Harness

### 3.1 Guides (Feedforward)

Documents and rules that agents read BEFORE acting:

| Guide                    | Location                                      | Purpose                                                       |
| ------------------------ | --------------------------------------------- | ------------------------------------------------------------- |
| Canonical context graph  | `/context-packs/epoch-<N>/canonical/**`        | Source of truth for reusable instructions per epoch            |
| Provider context bundles | `/context-packs/epoch-<N>/{claude,gemini,pi}/**` | Generated provider-specific instruction surfaces             |
| AGENTS.md                | `/AGENTS.md`                                  | Human-readable golden path derived from canonical pack        |
| ADRs                     | `/docs/decisions/`                            | Why past decisions were made                                  |
| Task manifests           | Generated per delegation                      | Allowed paths, deps, tests, acceptance criteria               |
| Harness templates        | Per task type                                 | Guide+sensor bundles (see §3.5)                               |
| planv0.md excerpts       | Referenced in manifests                       | Domain context (only relevant sections, not full doc)         |
| INCIDENTS.md             | `/INCIDENTS.md`                               | Past failures to avoid                                        |
| Dependency layer rules   | Structural tests                              | What can import what                                          |

Context packs are compiled from typed `ContextCard`s:

- `ADRCard` — architectural decision records
- `ExampleCard` — golden path code examples
- `IncidentCard` — past failures and root causes
- `InvariantCard` — mechanical invariants and structural rules
- `ChecklistCard` — step-by-step task procedures
- `GlossaryCard` — domain-specific terminology and definitions

Manifests reference cards, not whole documents. Provider bundles are rendered views over the same selected card set (`CLAUDE.md` for Claude lanes, `GEMINI.md` for Gemini lanes, `AGENTS.md`/`SYSTEM.md` for Pi lanes). The compiler prunes low-value cards using trace-derived usefulness metrics (which cards reduced retries, lowered token burn, or improved first-pass green rate).

Ambient provider memory/settings (Claude Code's auto memory, Codex's `~/.codex/config.toml`, Gemini CLI's layered config) are not treated as repo knowledge unless explicitly imported into the epoch bundle and hashed into provenance. The compiler emits a bundle-equivalence artifact showing which required cards appear in each provider surface.

### 3.2 Sensors (Feedback)

Verification checks that run AFTER the agent acts, organized by timing and cost:

**Tier 0 — Admission / preflight (< 2s, before session start)**
- Bead schema lint (`definition_of_ready` — is the bead well-formed?)
- Manifest completeness check (all required fields present)
- Requirement-to-evidence matrix lint:
  - every acceptance requirement has an ID
  - every requirement maps to at least one sensor or proof artifact
  - negative cases declared for API/contract changes
  - rollback scope and observability expectations present
- OAuth lane health check:
  - provider reachable
  - cached login valid and not expired
  - config root writable and isolated
  - account/quota entitlement present
  - session resumable where supported
  - no browser/interactive prompt required
- Quota / budget check (tokens remaining, wall-clock budget)
- Worktree + build-slot reservation
- Context-pack availability (epoch pack materialized for this provider)
- Test command existence (does the declared test_command actually exist?)

**Tier 1 — Pre-commit (< 5s, every edit, computational)**

- `cargo fmt --check`
- Restricted-file guard: reject edits outside task manifest's `allowed_paths`
- Forbidden-import check: dependency layer structural test
- Golden fixture guard: reject edits to `fixtures/golden/**` and block `REGENERATE_GOLDEN=1` unless task manifest explicitly grants permission (prevents agents from cheating golden tests)
- Loop detection: same error hash recurring = stuck (see §7.3)

**Tier 2 — Self-correction (< 60s, per commit attempt, computational)**

- `cargo clippy -p <crate>`
- `cargo test -p <crate>` (targeted tests only)
- Structural tests (dependency layers, CODEOWNERS compliance)
- Proof adequacy gate: type / compile-fail / property / Kani / loom evidence may satisfy the gate with lower raw line coverage. Unit-test-only tasks keep a changed-line coverage target (`cargo llvm-cov`, >90%). Review packets record evidence class, not just a percentage.
- Budget preflight: if task touches expert sizing or byte math, run `StaticBudgetReport` check — fail if expert exceeds bank capacity (catches T6.6/T2.3 violations in worktree, not at epoch merge)

**Tier 3 — Review (per PR, deterministic-first)**
- Build deterministic review packet (§5.1)
- Select 0-2 inferential reviewers from reviewer pool based on diff risk (§5.2)
- Style lieutenant check (conventions, naming)

**Tier 4 — Epoch integration (expensive, per epoch)**

- `cargo test --workspace`
- Cross-crate integration tests
- Full structural audit

**Tier 5 — Continuous monitoring (periodic, background)**

- Dead code detection
- Doc-code consistency
- Test coverage drift
- Dependency vulnerability scan
- Constraint drift scanning

### 3.3 Mechanical Enforcement

Hard gates that agents literally cannot bypass:

**Dependency layer rules** — enforced by structural test in CI:

```
Allowed imports:
  gbf-hw        -> (leaf, no deps)
  gbf-artifact  -> gbf-hw
  gbf-policy    -> gbf-hw, gbf-artifact
  gbf-model     -> gbf-artifact, gbf-hw
  gbf-train     -> gbf-model, gbf-artifact, gbf-policy, gbf-hw
  tests          -> everything

Forbidden:
  gbf-artifact  -> gbf-model, gbf-train, gbf-policy
  gbf-model     -> gbf-train, gbf-policy
  gbf-hw        -> anything
```

**LLM-optimized sensor output** — error messages include remediation:

```
Bad:  "error[E0433]: failed to resolve: use of undeclared crate"
Good: "error: gbf-model cannot import gbf-train (forbidden by
       dependency layer rule — see docs/decisions/adr-002.md).
       gbf-model may only import: gbf-artifact, gbf-hw."
```

### 3.4 Risk Model, Execution Boundaries, and Autonomy Ladder

Not all beads deserve the same autonomy. A docs-only change with unrestricted shell/network access is not low risk. A contract change with no network/secrets access is risky in a different way. Risk has two axes.

**CodeRisk** (API/semantic blast radius):

| Class  | Description                                                       | Examples                                                         |
| ------ | ----------------------------------------------------------------- | ---------------------------------------------------------------- |
| **R0** | Docs, comments, test-only, local non-exported code                | T10.1 (test fixtures), T10.2 (logging setup)                     |
| **R1** | Internal crate logic with clear proof obligations                 | T1.2 (TernaryLinearQat), T6.3 (tied embeddings)                  |
| **R2** | Public APIs, shared contracts, foundational crates                | T3.1 (TernaryWeightPlan types), T2.1 (RuntimeChromeBudget types) |
| **R3** | Workspace config, CI, cross-crate interfaces, destructive changes | T1.8 (Burn version pinning), Cargo.toml changes                  |

**ExecRisk** (trust boundary and side-effect danger):

| Class  | Description                                                  |
| ------ | ------------------------------------------------------------ |
| **X0** | Read-only local analysis                                      |
| **X1** | Local repo writes only                                        |
| **X2** | Shell/network to approved build/package endpoints              |
| **X3** | Secrets, external systems, destructive commands, cloud effects |

**Autonomy ladder:**

| Level  | Permission                       | Allowed For                                       |
| ------ | -------------------------------- | ------------------------------------------------- |
| **L0** | Analyze only                     | All risk classes initially                        |
| **L1** | Propose patch (branch, no merge) | All risk classes                                  |
| **L2** | Commit to engineer branch        | All risk classes                                  |
| **L3** | Auto-merge to team branch        | R0/X0-X1 and R1/X0-X1 (after Phase 1 gate)       |
| **L4** | Promote to integration branch    | R0-R1/X0-X1 (after Phase 2)                      |
| **L5** | Promote to main                  | R0/X0 only (after Phase 3, stable error budget)   |

**Binding:** The task manifest includes `code_risk`, `exec_risk`, `max_autonomy_level`, and `circuit_breaker_scope`. Effective autonomy is bounded by the **minimum** of:
- code-risk ceiling
- exec-risk ceiling
- provider capability ceiling
- current circuit-breaker state

**ExecRisk enforcement** is realized by the Security Supervisor (§6.2):
- Short-lived task-scoped credentials from SecretBroker
- Egress allowlists per manifest from NetworkPolicyWorker
- Signed commits / tags / merge decisions by SigningWorker
- Secret scanning / SBOM / vulnerability audit in Tier 4-5 by SBOMScannerWorker

X2/X3 authority never follows from code risk alone; it requires explicit execution policy in the manifest. A low-code-risk docs change with outbound network or secrets access is still operationally dangerous.

**Circuit breakers:** The system preserves progress on safe lanes and freezes unsafe lanes. Breakers may trip per provider lane, team, risk class, side-effect type, or workspace integrity state. Unrelated safe lanes continue when an unsafe lane freezes — this replaces the earlier "system never halts" principle with a more precise guarantee.

Each lane has an explicit operating mode, and breaker transitions downgrade mode before escalating to full freeze:
- `Full` — all dispatch and promotions allowed
- `Reduced` — dispatch allowed, optional review/meta jobs skipped, promotions require extra review
- `DeterministicOnly` — only deterministic checks run, inferential review paused
- `ReadOnly` — no new dispatch, preserve in-flight checkpoints only

### 3.5 Harness Templates (Per Task Type)

Different task types get different guide+sensor bundles:

**new-module:**

- Guides: AGENTS.md §module-creation-checklist, scaffold template, relevant ADRs
- Sensors: All 5 tiers + "does mod.rs export the new module?" + naming convention check

**bug-fix:**

- Guides: INCIDENTS.md (similar past failures), failing test output (LLM-optimized)
- Sensors: Tier 1-2 only (fast turnaround) + "does the fix include a regression test?"

**cross-crate-interface:**

- Guides: Dependency layer diagram, both crates' ADRs, RFC document
- Sensors: All 5 tiers + dependency layer compliance + "does public API have doc comments?"

**test:**

- Guides: AGENTS.md §test-conventions, existing test patterns in target crate
- Sensors: Tier 1-2 + "do tests assert something meaningful?" + "are tests hermetic?"

**mechanical-transform:**

- Guides: transform spec, affected symbol list, rollback plan
- Sensors: AST round-trip / parser round-trip, formatting, targeted compile/test, semantic diff allowlist
- Execution: deterministic `CodemodWorker` first; LLM engineer only designs or repairs the transform if deterministic execution fails

**experiment (training/ML tasks):**

- Guides: planv0.md relevant sections, existing training configs, eval methodology docs
- Manifest includes: dataset refs, seed policy, max runtime/cost budget, expected metric delta, eval script path, artifact destination, rollback criteria
- Proof obligation: seeded eval plan + metric threshold + published artifact lineage (NOT just `cargo test`)
- Sensors: eval script passes threshold, cost stayed within budget, artifacts written to correct destination, seed is recorded for reproducibility
- Note: A green `cargo test` is NOT evidence that a training change was good. The eval script is the primary sensor.

---

## 4. Engineer Protocol

### 4.0 Decision IR

All load-bearing managerial actions are represented as typed proposals:

| Proposal Type | Produced By | Validated By |
|---|---|---|
| `ManifestDraft` | Lieutenant / Manager | PolicyEngineGenServer (admission, satisfiability) |
| `SplitPlan` | Manager | PolicyEngineGenServer (dep graph consistency) |
| `ReviewPlan` | ReviewerSelector | PolicyEngineGenServer (risk-class routing) |
| `MergeProposal` | MergeTrainWorker | SideEffectExecutorGenServer (snapshot validity, command ledger) |
| `EscalationReport` | Any agent | PolicyEngineGenServer (integrity state check) |

Managers and reviewers may generate proposals, but only deterministic control-plane services may: allocate leases, change risk/autonomy ceilings, enqueue side effects, merge branches, or tag epochs. Every proposal includes `schema_version`, `inputs_hash`, `evidence_refs`, `author_agent_id`, and `generated_at`, and is persisted to `state.db` before evaluation.

### 4.1 Task Manifest

Every delegation is a structured JSON sandbox:

```json
{
  "schema_version": "v3",
  "attempt_id": "att-bd-2q4-03",
  "bead_id": "bd-2q4",
  "attempt_no": 3,
  "team": "model",
  "harness_template": "new-module",
  "proof_kind": "compile-and-unit-tests",
  "code_risk": "R1",
  "exec_risk": "X1",
  "max_autonomy_level": "L2",
  "baseline": {
    "epoch_tag": "epoch-7",
    "base_commit": "abc123...",
    "target_branch": "team/model"
  },
  "sandbox": {
    "allowed_paths": ["gbf-model/src/qat/**"],
    "restricted_paths": ["Cargo.toml", "gbf-model/src/lib.rs"],
    "conflict_domains": ["api:gbf-model::qat", "fixture:tests/fixtures/tensor_helpers"],
    "allowed_tools": ["read_file", "edit_file", "run_command", "git_commit"],
    "allowed_deps": ["burn-core", "burn-tensor"]
  },
  "test_command": "cargo test -p gbf-model --lib qat",
  "deliverables": [
    { "id": "DELIV-1", "statement": "qat module exists and compiles" },
    { "id": "DELIV-2", "statement": "exports TernaryLinearQat and ActFakeQuant" }
  ],
  "requirements": [
    { "id": "REQ-1", "statement": "module exports TernaryLinearQat", "evidence_refs": ["compile-test:qat_exports"], "negative_cases": [] },
    { "id": "REQ-2", "statement": "module exports ActFakeQuant", "evidence_refs": ["compile-test:qat_exports"], "negative_cases": [] }
  ],
  "observability_expectations": ["no new warn/error logs in green-path unit tests"],
  "test_impact": ["tests/unit/model/qat_exports.rs"],
  "context": {
    "context_pack_hash": "ctx-epoch-7-abc123",
    "provider_bundle_hash": "pi-epoch-7-def456",
    "context_refs": [
      "br show bd-2q4",
      "docs/decisions/adr-001-burn-as-frontend.md"
    ]
  },
  "budgets": {
    "change_budget": { "max_files": 8, "max_changed_lines": 400 },
    "resource_budget": { "max_tokens": 120000, "max_wall_clock_sec": 1800, "build_slots": 1 }
  },
  "rollback_plan": "revert stacked commits 2..N, preserve proof commit",
  "execution_fingerprint": {
    "engineer_adapter": "PiSessionAdapter@1.0.0",
    "requested_model": "codex",
    "resolved_model": "codex-<resolved>",
    "auth_mode": "oauth",
    "pi_extensions_hash": "ext-789"
  }
}
```

The engineer's attempt repo has Pi tool hooks that reject changes outside `allowed_paths`.

### 4.2 Proof-Obligation-First Implementation

Engineers establish proof obligations BEFORE implementing. Failing tests are the default form for code tasks, but not the only form. The key principle: **establish what "done" means in machine-checkable terms before writing production code.**

**Proof obligations by task class:**

| Task Class            | Proof Obligation                                | Form                                                                          |
| --------------------- | ----------------------------------------------- | ----------------------------------------------------------------------------- |
| Bug fix               | Failing regression test                         | `#[test]` that reproduces the bug                                             |
| New leaf module       | Unit tests + compile-fail tests                 | Standard `cargo test`                                                         |
| Cross-crate interface | Interface contract tests + example consumer     | Trait + compile test + usage example                                          |
| Refactor              | Semantic diff + non-regression                  | Snapshot tests, API surface diff, benchmark baseline                          |
| Domain modeling       | Type-driven bounds                              | Compile-fail tests demonstrating illegal states are unrepresentable            |
| Critical byte-math    | Generative invariants / formal bounds           | `proptest` invariants, or Kani bounded model checking harnesses               |
| Concurrent structures | Permutation bounds                              | `loom` test harnesses for thread interleaving                                 |
| Performance           | Benchmark envelope                              | `cargo bench` baseline, threshold in manifest                                 |
| Experiment/training   | Eval plan + metric threshold + artifact lineage | Eval script, seed, dataset ref, expected delta (see §3.5 experiment template) |
| Infra/config          | Structural invariant                            | CI check, workspace test                                                      |

**Proof strength hierarchy:** A green `cargo test` is weak evidence compared to an un-instantiatable illegal state. Prefer stronger proof forms when available:

1. **Types** — If the compiler rejects the illegal state, no test is needed. (e.g., `TernaryWeightPlan` that cannot be constructed with invalid scale granularity.)
2. **Generative/property-based** — `proptest` invariants force the logic to hold over the entire input domain, not just cherry-picked examples. Harder for an agent to game with `if input == "test" { return true; }`.
3. **Bounded model checking** — Kani harnesses for critical byte-math (T6.6 expert budget formula, T3.4 byte cost) prove correctness within bounds.
4. **Empirical tests** — Standard unit tests. Necessary but weakest. Always supplement with stronger forms when the domain allows.

```
ENGINEER LOOP:

1. READ task manifest + harness template + ADRs
2. READ existing code in target area
3. ESTABLISH PROOF OBLIGATIONS
   - Write the appropriate proof artifact for this task class
   - For code tasks: tests that FAIL (nothing implemented yet)
   - For refactors: snapshot current API surface / benchmark baseline
   - For experiments: eval script + seed + expected metric threshold
   - For core domain logic: property-based invariant frameworks (`proptest`)
   - For state constraints: type signatures + compile_fail tests
   - For critical byte-math: Kani bounded model checking harnesses
   - Run proof artifacts to confirm they fail for the RIGHT reason
4. COMMIT proof obligations (stacked commit 1: "test: ..." or "bench: ..." etc.)

--- PROOF REVIEW GATE ---
5. Run `ProofLint` (deterministic, <10s) on the proof commit BEFORE any model
   reviewer is invoked.

   `ProofLint` rejects:
   - proof commits that modify production code for non-refactor task classes
   - tests that do not fail before implementation
   - ignored / TODO / placeholder / empty-assertion tests
   - missing negative cases when required by manifest
   - missing compile-fail evidence for illegal-state claims
   - missing perf baselines for performance tasks
   - property test present but case count below policy threshold

6. Lieutenant invokes the Epistemologist (Gemini one-shot via `gemini -p`) on
   the proof commit only after `ProofLint` is green.

   Reviewer Persona — The Epistemologist:
   Assumes the implementation agent (Pi/Codex) is lazy and will attempt to
   cheat. Evaluates proofs adversarially. Does NOT care about code style,
   architecture, or performance — only proof strength.

   Checks:
   - If the implementation is a hardcoded `return true;`, will these proofs
     still pass? If yes, the proofs are invalid — reject.
   - Does the type signature allow illegal states that should be
     unrepresentable? If so, demand compile-fail tests or tighter types.
   - Are the proofs verifying the specification, or just locking in
     arbitrary behavior (tautology hunting)?
   - Is a type-driven constraint or property-based test more appropriate
     than a point-test? If so, demand the stronger form.
   - Do the proof obligations push the exact mathematical or typological
     boundaries of the spec?

   --- PROOF REVIEW LOOP ---
   Reviewers emit typed findings: `MustFix | ShouldFix | Question | Nit`,
   each with confidence level and evidence refs.

   Continue until no unresolved `MustFix` remain or `proof_review_budget`
   is exhausted. `ShouldFix` items may be deferred with rationale recorded
   in the review packet.

   On budget exhaustion: tighten autonomy, checkpoint/handoff, or escalate
   based on risk. Do not loop indefinitely.
---

7. IMPLEMENT to satisfy proof obligations
8. COMMIT implementation (stacked commit 2: "feat: ...")
9. SELF-VERIFY through all tier 1-2 sensors (including coverage check)
10. ITERATE until all sensors pass (burn tokens — but detect stuck loops)
11. COMMIT fixes if needed (stacked commit 3: "fix: ...")
12. COMMIT docs/ADR if non-obvious decisions were made (commit 4: "docs: ...")
13. SIGNAL lieutenant
```

### 4.3 Stacked Commits (Not Squash)

Each task produces stacked commits on the same branch:

```
commit 4: "docs: add ADR for ternary threshold decision"
commit 3: "fix: handle edge case in ternary threshold"
commit 2: "feat: implement TernaryLinearQat core"
commit 1: "test: add tests for TernaryLinearQat (all failing)"
```

Why stacked:

- Human can review test intent separately from implementation
- Git bisect works at finer granularity
- Monitor agent can give feedback on tests before implementation starts
- If implementation is wrong, tests are still valid — fresh agent restarts from commit 1

### 4.4 What Engineers Cannot Do

- Modify files outside `allowed_paths` (pre-commit rejects)
- Import forbidden crates (structural test rejects)
- Skip test-first (lieutenant enforces via review)
- Spend >80% context (lieutenant forces checkpoint)
- Ignore test failures (self-verify gate blocks commit)

---

## 5. Review: Deterministic Checks + Inferential Review

Six similar model reviewers reading the same context with slightly different prompts is not six independent reviewers — it's one correlated blind spot, multiplied. Instead, we lead with strong machine-checkable evidence and use 1-2 model reviewers for the residue.

### 5.1 Deterministic Review Checks (run first, always)

These are computed, not inferred. They produce a canonical JSON review packet with `deterministic.checks[]`, `requirements_matrix[]`, `review_rounds[]`, `final_verdict`, and evidence refs that the model reviewer consumes:

| Check                            | Tool                              | Output                               |
| -------------------------------- | --------------------------------- | ------------------------------------ |
| Proof-lint gate                  | `ProofLint`                       | Pass/fail + invalid-proof diagnostics |
| Dependency layer compliance      | Structural test                   | Pass/fail + violation details        |
| Public API diff                  | `cargo public-api diff` or custom | Added/removed/changed items          |
| Proof-obligation status          | Coverage check + test results     | Lines covered, tests passing/failing |
| Benchmark delta                  | `cargo bench` vs baseline         | Regression/improvement numbers       |
| Invariant checklist              | Manifest-derived                  | Which acceptance criteria pass       |
| Mutation score (when applicable) | `cargo-mutants` on new code       | Surviving mutants list               |

### 5.2 Inferential Review (0-2 selected reviewers via Gemini CLI one-shot)

After deterministic checks, Gemini reviewers handle what machines can't. Each reviewer is a one-shot `gemini -p` invocation with `--model gemini-3.1-pro-preview`, not a persistent session. The Epistemologist runs at the Proof Gate (§4.2); post-implementation reviewers are **selected by risk**, not applied uniformly.

**Reviewer pool:**

| Reviewer                 | When | Checks | Input |
| ------------------------ | ---- | ------ | ----- |
| **The Epistemologist**   | Proof Gate (before impl) | Are proofs tautological? Can illegal states be represented? Can a lazy agent cheat? Is a stronger proof form available? | Bead description, proof obligation commit, coverage data |
| **Semantic Reviewer**    | Post-impl | Does this correctly implement the spec? Subtle bugs? | Bead description, planv0.md excerpt, API diff, coverage report, diff |
| **Integration Reviewer** | Post-impl | Architectural intent? Merge cleanly? Morning-reviewable? | ADRs, AGENTS.md golden path, dependency diff, invariant checklist |
| **Performance Reviewer** | Post-impl | Allocations, hot paths, benchmark regressions? | Benchmark delta, profile data |
| **Operability Reviewer** | Post-impl | Error handling, observability, operational concerns? | Change diff, logging patterns |

**`ReviewerSelector` chooses post-impl reviewers by risk:**
- R0/X0 docs/test-only: deterministic checks only, 0 inferential reviewers
- R1/X0-X1 internal code: 1 reviewer (Semantic)
- R2+ or X2+ public API/perf/cross-crate: 2 reviewers (Semantic + Integration or Performance)

### 5.3 Protocol

```
1. Run all deterministic checks in parallel (< 60s)
2. If any hard check fails (dependency layer, coverage < 90%):
   → Return to engineer with structured failure. No model review yet.
3. Collect deterministic outputs into a structured review packet.
4. Invoke 1-2 Gemini reviewers (via `gemini -p --model gemini-3.1-pro-preview --approval-mode plan`) with the review packet.
5. Each returns APPROVE or REQUEST_CHANGES with specific feedback.
6. If changes requested: feedback to engineer (Pi/Codex), who fixes.
7. Re-run only failed checks + re-invoke model reviewers.
8. Loop until all hard checks are green and no reviewer has unresolved `MustFix`.
   `ShouldFix` items may be deferred with rationale recorded in the review packet.
   Non-convergence after budget triggers escalation, not infinite churn.
   Once approved, merge to team branch.
```

### 5.4 Reviewer Calibration

Track reviewer precision and recall over time (via trace data):

- If a reviewer mostly generates churn (requests changes that don't improve outcomes), demote or remove it.
- If escaped defects cluster in an area a reviewer should have caught, add a deterministic check for that class.
- Review the reviewer: periodically compare model review comments against human review decisions from morning protocol.
- Human audit sampling lane:
  - Random 10% audit of R0/R1 auto-approvals
  - Mandatory audit of first N beads after any harness/model/prompt change
  - Audit labels feed reviewer calibration and autonomy promotion decisions

---

## 6. Runtime Architecture: BEAM + Gleam + ACP + Custom Harness

The org runs as a Gleam application on the BEAM VM. Supervision trees mirror the lieutenant tree exactly. Managers (top manager, lieutenants) are Claude Code instances steered via ACP. Engineers are custom API harnesses wrapped by Gleam processes. All traces aggregate centrally.

### 6.0 SessionAdapter and Capability Registry

Each provider has a different native surface: Claude Code uses ACP, Pi has its own RPC surface, and Gemini CLI is invoked as one-shot `gemini -p` subprocesses. Hard-wiring the control plane to any single protocol would make the runtime brittle. SessionAdapter remains the control-plane abstraction, but this project intentionally pins exact runtime surfaces per role. Runtime surface changes happen only via daytime control-plane releases and shadow/replay validation.

Every runtime advertises a machine-readable capability record:

```gleam
pub type SessionCapability {
  SessionCapability(
    transport: Transport,         // Acp | Rpc | Sdk
    supports_resume: Bool,
    supports_live_steering: Bool,
    supports_tool_interception: Bool,
    model_pin_strength: PinStrength,  // Exact | AliasResolved | BestEffort
    checkpoint_semantics: CheckpointSemantic,  // Full | Summary | None
  )
}

pub type Transport { Acp  Rpc  Sdk }
```

The Gleam control plane routes work by **capability**, not by provider name. A `SessionAdapter` trait wraps each provider:

```gleam
pub type SessionAdapter {
  fn start(config) -> Session
  fn steer(session, instruction) -> Result
  fn interrupt(session) -> Checkpoint
  fn capabilities() -> SessionCapability
}
```

Concrete adapters: `ClaudeSessionAdapter`, `GeminiCliAdapter`, `PiSessionAdapter`. The control plane talks to one interface. Underneath, each provider uses its native surface. Gemini reviewers/meta-jobs use one-shot `gemini -p` invocations (not ACP sessions) — each call is a single prompt→response with structured output, which is the natural fit for ephemeral review/analysis jobs.

**Authentication:** There is exactly one auth mode: **OAuth login through each provider's own first-party CLI.** Each provider's login flow is used within its own supported surface, which is within the ToS of each provider:
- Claude Code owns its own OAuth login → Claude managers authenticate via `claude` CLI login
- Pi owns its Codex/OpenAI OAuth login → Pi engineers authenticate via `pi` login with Codex provider
- Gemini CLI owns its own Google OAuth login → Gemini reviewers authenticate via `gemini` CLI login (one-shot `-p` mode uses the same cached credentials)

No API keys, no credential proxying, no third-party token harvesting. Each provider's OAuth session is managed by that provider's own CLI. The Gleam control plane holds session handles, not credentials.

Because auth is OAuth-only, lane session health is a first-class runtime concern. Each provider lane has a `SessionCustodian` responsible for:
- Isolated provider config roots (no shared `~/.config` between lanes)
- Cached-session smoke tests before night shift starts
- Resume-handle capture where the provider supports it
- Detection of reauth-required / interactive-prompt states
- Lane-local freeze without stopping unrelated lanes

### 6.1 Why BEAM

The problems we need to solve — process isolation, supervision, message passing, failure recovery, introspection — are exactly what BEAM was designed for 30 years ago. Every pattern we reinvented in prose maps to an existing OTP primitive:

| Our design problem                   | BEAM primitive                                    |
| ------------------------------------ | ------------------------------------------------- |
| Agents should fail independently     | Process isolation (each process has its own heap) |
| Parent watches child for crashes     | Supervisor with `one_for_one` strategy            |
| Team of workers shares a strategy    | Supervisor with worker pool                       |
| Structured messages between agents   | `send`/`receive` with pattern matching            |
| Parent interrupts child              | Process linking, `exit(pid, reason)`              |
| Restart child on failure             | Supervisor restart intensity/period               |
| Observe any running agent            | `:observer.start()`, process introspection        |
| Hot-reload agent logic for daytime debugging | Not used during pinned night-shift epochs |
| "Let it crash" failure philosophy    | Built into the VM                                 |

Gleam provides static typing on top of BEAM, which matters here because the message protocol between agents is load-bearing — we want compile-time verification that we're sending the right message shapes.

### 6.2 The Supervision Tree

```
Application Supervisor (one_for_one)
├── ControlPlane Supervisor (one_for_one)
│   ├── SchedulerGenServer (dependency-aware bead dispatch)
│   ├── PolicyEngineGenServer (admission, risk gates, circuit breakers)
│   ├── ResourceManagerGenServer (build slots, quotas, attempt-repo + capsule allocation)
│   ├── SideEffectExecutorGenServer (owns all git/br mutations via command ledger)
│   └── BeadGraphProjection (materialized DAG from state store)
│
├── Security Supervisor (one_for_one)
│   ├── SecretBrokerWorker (short-lived task-scoped credentials)
│   ├── NetworkPolicyWorker (egress allowlists per manifest)
│   ├── SigningWorker (signed commits / tags / merge decisions)
│   └── SBOMScannerWorker (secret scanning / vulnerability audit in Tier 4-5)
│
├── TopManager Supervisor (one_for_one)
│   ├── TopManagerGenServer (wraps Claude Code via SessionAdapter)
│   └── MergeQueueGenServer
│
├── Team Supervisors (one_for_one, one per team)
│   ├── team_model Supervisor
│   │   ├── LieutenantGenServer (wraps Claude Code via ACP)
│   │   ├── MonitorGenServer (watches engineer processes + git)
│   │   ├── MergeTrainWorker (runs git merge + cargo test async)
│   │   └── EngineerPool Supervisor (simple_one_for_one)
│   │       ├── EngineerWorker (Pi + Codex harness)
│   │       ├── EngineerWorker
│   │       └── EngineerWorker
│   ├── team_contracts Supervisor (same shape)
│   ├── team_train Supervisor
│   └── team_qa Supervisor
│
├── ReviewJobs Supervisor (simple_one_for_one)
│   └── ReviewJobWorker (ephemeral, persona-selected per invocation)
│
├── MetaJobs Supervisor (simple_one_for_one)
│   └── MetaJobWorker (ephemeral, persona-selected per invocation)
│
└── Observability Supervisor (rest_for_one)
    ├── TraceAggregator (collects all API traces)
    ├── MetricsCollector (latency, tokens, durations)
    ├── DashboardServer (exposes live state)
    └── EventLog (persistent append-only log)
```

**Why this shape:**

- `one_for_one` at the top: if the top manager crashes, only restart the top manager — don't restart the whole org.
- `simple_one_for_one` for engineers: dynamic worker pool, spawn/kill engineers on demand.
- `simple_one_for_one` for review/meta jobs: ephemeral workers spawned per invocation, not persistent processes. "Epistemologist", "Semantic Reviewer", "Retro", "CI Health", etc. are persona labels (prompt + config), not always-on processes.
- `rest_for_one` for observability: if the trace aggregator crashes, restart everything downstream that depends on it (metrics, dashboard).
- Team supervisors are independent: if team_model's lieutenant crashes, team_contracts keeps running.

**Resource constraints:** Each Claude Code manager is a Bun process; each Pi engineer is a Bun process. With 1 top manager + 4 lieutenants + 12 engineers = 17 Bun instances plus the BEAM VM, plus concurrent `cargo test`/`rustc` invocations. **The EngineerPool Supervisor MUST limit concurrency based on available RAM.** On a 64GB machine, cap at ~8-10 concurrent engineers. Set `CARGO_BUILD_JOBS=2` per attempt repo to avoid CPU starvation.

**Attempt-repo isolation:** Each autonomous engineer gets an isolated attempt repo — not a shared worktree. Do not use shared worktrees for autonomous agents once concurrency >1. Attempt repos are created from a local read-only bare mirror of the authoritative repo. Use sparse checkout for `allowed_paths` + required build files + referenced docs. Where supported, prefer partial/blobless clone (`--filter=blob:none`) to reduce startup cost.

Each attempt repo has its own `.git`, index, refs, temp dir, provider config roots, and target dir. On failure or completion, the entire attempt repo is destroyed cleanly. This eliminates `.git/index.lock` contention, shared-config corruption, and dirty-worktree recovery complexity by design.

A shared compiler cache (`sccache` or equivalent) is mandatory from Phase 0a so isolated target dirs do not forfeit dependency-build reuse.

### 6.3 Message Protocol (Gleam Types)

All communication between processes is via typed messages. No files, no git polling for status, no branch-reading for observation.

```gleam
// Messages the top manager receives
pub type TopManagerMsg {
  TeamReadyForIntegration(team: Team, branch: String, beads_done: List(BeadId))
  TeamBlocked(team: Team, waiting_on: BeadId, reason: String)
  EpochCITick
  HumanNewBead(bead_id: BeadId)
  CrossCrateBeadDetected(bead_id: BeadId, teams_touched: List(Team))
  IncidentReported(team: Team, severity: Severity, details: Incident)
  ShutdownNight
}

// Messages a lieutenant receives
pub type LieutenantMsg {
  RebaseOnEpoch(tag: String)
  PriorityBoost(bead_id: BeadId)
  EngineerCompleted(eng_id: EngineerId, branch: String, final_state: EngineerState)
  EngineerStuck(eng_id: EngineerId, checkpoint: Checkpoint)
  EngineerCrashed(eng_id: EngineerId, reason: String)
  MonitorFlagged(eng_id: EngineerId, issue: QualityIssue)
  TopManagerSteering(instruction: SteeringMsg)
  CollectStatus(reply_to: Subject(TeamStatus))
}

// Messages an engineer receives
pub type EngineerMsg {
  TaskAssigned(manifest: TaskManifest)
  ReviewFeedback(comments: List(Comment))
  ContextPrune(keep_only: List(ContextRef))
  InterruptAndCheckpoint
  LieutenantSteering(instruction: SteeringMsg)
}

// Responses / upward reports
pub type EngineerState {
  WritingTests(tests_planned: Int)
  Implementing(tests_passing: Int, tests_failing: Int)
  SelfVerifying
  Done(commit: String, branch: String, tests_passed: Int)
  Blocked(reason: String, findings: String, attempted: List(String))
  Checkpoint(progress: String, remaining: String)
}
```

**Key property:** Gleam's type system forces every message path to be explicit. A lieutenant can't accidentally send a wrong-shape message to a worker. Protocol violations are compile-time errors, not runtime bugs.

### 6.4 Managers Are Claude Code Instances Steered via ACP

The top manager and each lieutenant is a real Claude Code instance (opus), running as a subprocess managed by its Gleam GenServer. Communication uses ACP (Agent Client Protocol).

**Why ACP:** ACP is designed for exactly this — programmatic, stateful, steerable communication with AI coding agents. It's not a chat API; it's a structured protocol with:

- Session lifecycle (start, interrupt, resume, kill)
- Structured turn-based messages (not free-form text)
- Tool use introspection
- Real-time steering (inject context mid-session without restart)
- Event streaming (observe every action the agent takes)

**Session rotation policy:** Persistent sessions are rotated after configurable thresholds (`max_tokens`, `max_tasks`, `max_idle_minutes`, `drift_score`). Rehydration uses typed state from `state.db` (proposals, leases, event log summaries), not opaque conversation history. Provider-native checkpoints are safety nets, not the authoritative state. Warm spare sessions may be pre-initialized where budget allows.

**GenServer wrapping Claude Code:**

```gleam
pub type ManagerState {
  ManagerState(
    acp_session: AcpSession,
    role: Role,  // TopManager or Lieutenant(team)
    current_task: Option(Task),
    trace_sink: Subject(Trace),
  )
}

pub fn handle_message(state: ManagerState, msg: LieutenantMsg) -> ManagerState {
  case msg {
    RebaseOnEpoch(tag) -> {
      // Steer the Claude Code session via ACP:
      // "New epoch available at tag X. Rebase your team branch before
      //  dispatching any more engineers."
      acp.steer(state.acp_session, RebaseInstruction(tag))
      state
    }

    EngineerStuck(eng_id, checkpoint) -> {
      // Kill the stuck engineer process (BEAM-level)
      process.exit(engineer_pid(eng_id), "stuck")
      // Tell the Claude Code lieutenant to reassign the bead
      acp.steer(state.acp_session, ReassignBead(checkpoint.bead_id))
      state
    }

    TopManagerSteering(instruction) -> {
      // Top manager is interrupting us. Pass through to Claude Code.
      acp.steer(state.acp_session, instruction)
      state
    }

    // ... other message handlers
  }
}
```

**What this buys us over branch-based comms:**

- **Real-time interruption.** Top manager can say "stop what you're doing, cross-team dep X just landed, prioritize T6.6 now" — and the lieutenant's Claude Code instance receives that mid-session.
- **Context injection.** When review feedback comes in, it's pushed into the Claude Code session, not committed to a file the agent has to re-read.
- **Clean restarts.** If a Claude Code session hangs, the Gleam supervisor kills and restarts it with fresh context summarized from the event log.
- **Observable actions.** Every tool call the Claude Code instance makes is visible to the supervising Gleam process via ACP events.

### 6.5 Engineers Are Custom Pi Harnesses (Codex Backend)

Engineers are built on [Pi](https://github.com/badlogic/pi-mono), a minimalist, extensible coding harness framework. We auth Pi against Codex (OpenAI) so the underlying model is Codex, not Claude. Each engineer is a Gleam process that spawns and steers a Pi subprocess configured with our custom TypeScript extensions.

**Why Pi + Codex rather than raw API or Claude Code:**

- **Pi is already a proper harness.** Compaction, AGENTS.md loading, SYSTEM.md, skills, prompt templates, and custom TypeScript extensions are built in. We don't reinvent context management.
- **Pi is extensible at the harness layer.** Custom TypeScript extensions let us enforce manifest sandboxing, inject LLM-optimized sensor feedback, and hook every tool call for tracing — exactly what we need.
- **Codex for engineers, Claude for managers** = mixed-model org. Managers (coordination, semantic review) run on Claude via ACP. Engineers (bounded implementation) run on Codex via Pi. Different models for different cognitive loads; can compare quality and cost per role.
- **Auth with Codex** gives access to the Codex CLI's backend without running the full Codex agent — Pi provides the harness, Codex provides the model.
- **Observable by construction.** Pi's architecture emits events we can capture. No monkey-patching.
- **Skills mechanism** in Pi maps naturally to our harness templates (§3.4). Each harness template is a Pi skill.

**Custom Pi extensions we ship:**

```typescript
// pi-extensions/manifest-sandbox.ts
// Enforces task manifest allowed_paths at the harness layer.
// Rejects file edits outside allowed_paths before they execute.
export const manifestSandboxExtension = {
  name: "manifest-sandbox",
  interceptToolUse: (tool, args, ctx) => {
    if (tool === "edit_file" || tool === "write_file") {
      if (!matchesManifest(args.path, ctx.manifest.allowed_paths)) {
        return {
          reject: true,
          message:
            `Path ${args.path} is outside allowed_paths. ` +
            `Your manifest allows: ${ctx.manifest.allowed_paths.join(", ")}. ` +
            `This is enforced at the harness layer — find another approach.`,
        };
      }
    }
    return { allow: true };
  },
};

// pi-extensions/sensor-injection.ts
// Wraps cargo test / clippy output with LLM-optimized error messages.
export const sensorInjectionExtension = {
  name: "sensor-injection",
  postToolResult: (tool, result, ctx) => {
    if (tool === "run_command" && result.command.startsWith("cargo test")) {
      return reformatCargoErrors(result.output, ctx.manifest);
    }
    return result;
  },
};

// pi-extensions/loop-detection.ts
// Tracks error hashes, forces checkpoint on recurring errors.
export const loopDetectionExtension = {
  /* ... */
};

// pi-extensions/trace-emitter.ts
// Streams every tool call + API call to the Gleam TraceAggregator.
export const traceEmitterExtension = {
  /* ... */
};

// pi-extensions/br-integration.ts
// Exposes br (beads) commands as Pi tools so engineers can
// create sub-beads for discovered work.
export const brIntegrationExtension = {
  /* ... */
};
```

**Engineer process shape (Gleam side):**

```gleam
pub type EngineerState {
  EngineerState(
    manifest: TaskManifest,
    attempt_repo: AttemptRepo,
    capsule: ExecutionCapsule,
    pi_subprocess: PiSession,           // running Pi with our extensions
    phase: EngineerPhase,
    trace_sink: Subject(Trace),
    supervisor: Subject(LieutenantMsg),
  )
}

pub type AttemptRepo {
  AttemptRepo(
    attempt_id: String,
    repo_dir: String,
    baseline_ref: String,
    base_commit: String,
    clone_strategy: CloneStrategy,     // Sparse | Partial | Full
  )
}

pub type ExecutionCapsule {
  ExecutionCapsule(
    home_dir: String,
    provider_config_roots: ProviderConfigRoots,
    temp_dir: String,
    cargo_target_dir: String,
    network_policy: NetworkPolicy,
  )
}

pub fn spawn_engineer(manifest: TaskManifest, team_branch: String) {
  // 1. Create isolated attempt repo from bare mirror
  let attempt_repo = resource_manager.request(
    CreateAttemptRepo(team_branch, branch_for(manifest), manifest.sandbox.allowed_paths)
  )

  // 2. Allocate minimal execution capsule
  let capsule = resource_manager.request(
    CreateCapsule(attempt_repo.attempt_id)
  )

  // 3. Materialize provider-specific context bundle for this task
  context_pack.materialize(
    epoch: current_epoch,
    provider: Pi,
    repo_dir: attempt_repo.repo_dir,
    manifest: manifest,
  )

  // 4. Spawn provider runtime inside capsule
  let pi = pi.spawn(
    cwd: attempt_repo.repo_dir,
    home: capsule.home_dir,
    tmp: capsule.temp_dir,
    auth: CodexAuth,
    extensions: [
      manifest_sandbox_extension(manifest),
      sensor_injection_extension(),
      loop_detection_extension(),
      trace_emitter_extension(self()),
      br_integration_extension(),
    ],
    initial_prompt: task_brief_from_manifest(manifest),
  )

  // 5. Pi now drives the engineer loop autonomously with Codex
  //    as the backend. The Gleam process watches for:
  //    - Completion (Pi exits with state=done)
  //    - Tool events (streamed via trace extension)
  //    - Checkpoint requests (from loop detection extension)
  //    - External messages (InterruptAndCheckpoint, ReviewFeedback, ...)
}
```

**What Pi's built-in machinery gives us for free:**

- **Compaction** — Pi summarizes older messages as context fills. We configure the trigger threshold.
- **AGENTS.md / SYSTEM.md loading** — per-team golden path docs are picked up automatically.
- **Skill installation** — harness templates ship as installable Pi skills.
- **No mid-task backend switching in night-shift runs** — even if Pi supports it, engineer lanes stay fixed to Codex for the duration of an epoch. Model changes are evaluated in shadow/replay lanes and adopted only at epoch boundaries.

**What our custom extensions add on top:**

- **Enforced sandboxing.** Pi lets extensions veto tool calls. We veto any edit outside `allowed_paths`. Not "we told the agent" — the harness literally won't execute it.
- **Sensor injection.** We intercept `cargo test` output and reformat compiler errors via the LLM-optimized template (§3.3) before Pi shows them to the model.
- **Loop detection.** We hash error signatures. On repeat within window, we reject further retries and emit a Checkpoint message upward to the Gleam supervisor.
- **Full trace capture.** Every tool call, every model exchange, every token count — streamed to the central TraceAggregator.
- **Steering hooks.** When the Gleam supervisor wants to interrupt (`InterruptAndCheckpoint`), the trace-emitter extension also accepts downward messages that get injected into Pi's next turn.

### 6.6 Observability: Central Trace Aggregation

Every API call from every agent flows to the `TraceAggregator` GenServer:

```gleam
pub type Trace {
  ApiCall(
    agent_id: String,
    role: Role,
    bead_id: Option(BeadId),
    timestamp: Int,
    tokens_in: Int,
    tokens_out: Int,
    latency_ms: Int,
    tool_calls: List(ToolUse),
    requested_model: String,
    resolved_model: String,
    auth_mode: String,  // always "oauth" — via provider's own CLI login
    effort_level: Option(String),
    extended_context: Bool,
    context_pack_hash: String,
    provider_version: String,
  )
  ToolResult(
    agent_id: String,
    tool: String,
    duration_ms: Int,
    success: Bool,
    error: Option(String),
  )
  PrecommitHook(
    agent_id: String,
    hook: String,
    duration_ms: Int,
    outcome: HookOutcome,
  )
  StateTransition(
    agent_id: String,
    from: String,
    to: String,
    timestamp: Int,
  )
}
```

The aggregator writes operational metadata to transactional local `state.db` (SQLite WAL by default). `state.db` holds events, commands, checkpoints, leases, outbox entries, and rolling metrics inputs. It does NOT serve as the long-lived provenance store for merged work.

Merged or archived attempts are compacted into canonical JSON git notes at land time (see §13.11). Large raw logs and full transcripts remain short-retention operational data with TTL-based cleanup. Rolling aggregates are maintained as materialized views that the dashboard and meta-agents query.

**Metrics derived from traces:**

| Metric              | Source                                | Used By                         |
| ------------------- | ------------------------------------- | ------------------------------- |
| Tokens per bead     | Sum of all ApiCall traces for bead_id | Cost tracking, retro            |
| Latency P50/P95     | ApiCall latency_ms distribution       | CI Health agent                 |
| Precommit duration  | PrecommitHook duration_ms             | CI Health agent                 |
| Tool use frequency  | ToolResult counts per tool            | Retro (detect useless tools)    |
| Error recurrence    | ApiCall + ToolResult error patterns   | Loop detection, retro           |
| Context efficiency  | tokens_in vs. useful_completion       | Retro (detect wasteful prompts) |
| Engineer stuck rate | Checkpoint state transitions          | Harness quality signal          |
| Model tier cost     | tokens × model price, grouped by role | Cost optimization               |

**Live introspection:**

Because everything runs on BEAM, we can introspect any running agent at any time:

- See the current message queue of any GenServer
- See the current state (prompts in flight, context used, phase)
- Attach a tracer to any process to watch its activity in real-time
- Hot-swap a GenServer's code without killing it (useful for harness tweaks mid-night)

The `DashboardServer` exposes this as a live view. During a night shift, the human can (if they want) watch any agent's current activity, any queue depth, any metric.

### 6.7 How Downward Steering Works (Example)

Concrete scenario: cross-team dep lands, top manager wants LT-Train to reprioritize.

```
1. LT-Model's lieutenant process sends to TopManager GenServer:
     TeamReadyForIntegration(team: Model, branch: "team/model",
                             beads_done: ["bd-g90"])   // T1.6 ExportVisitor done!

2. TopManager handler:
   - Runs workspace CI (spawned as a Task)
   - CI passes → merge team/model into main
   - Tags new epoch
   - Looks up dependents of bd-g90: T3.3, T4.3b, T8.2, T7.6, T10.13, T10.14

3. TopManager sends to each affected lieutenant:
     LieutenantGenServer ! RebaseOnEpoch("epoch-7")
     LT-Train ! PriorityBoost("bd-7lu")     // T4.3b now unblocked
     LT-Train ! PriorityBoost("bd-2ba")     // T8.2 now unblocked

4. Each LieutenantGenServer handles PriorityBoost:
   - Forwards to its Claude Code instance via ACP steering:
     acp.steer(session, "Priority change: bd-7lu is now unblocked
       and should be picked up in the next delegation round.
       Context: T1.6 (ExportVisitor) landed in epoch-7.")
   - Also updates local state so next delegation round sees the boost

5. Claude Code lieutenant receives the ACP steering event and
   incorporates it into its next reasoning step — not at some
   future rebase time, right now.
```

Compare to branch-based: top manager would commit a file somewhere, and the lieutenant would eventually read it. ACP makes it immediate.

### 6.8 How Upward Reporting Works (Example)

Engineer (Pi + Codex) discovers an unrelated bug while working.

```
1. Codex inside Pi calls the `br_create` tool exposed by the
   br-integration Pi extension, with title + description.

2. The br-integration extension intercepts the tool call. Instead
   of immediately shelling out, it reports upward via the trace-
   emitter extension's bidirectional channel:
     EngineerWorker (Gleam) receives:
       DiscoveredIssue(
         found_by: engineer_id,
         title: "...",
         details: "...",
         not_related_to: current_bead_id,
       )

3. EngineerWorker forwards to its supervisor:
     LieutenantGenServer ! DiscoveredIssue(...)

4. LieutenantGenServer handler:
   - Runs `br create` (shell command) with the structured details
   - Records in its state that this engineer produced a side-effect bead
   - Returns success to the Pi extension, which returns success to Codex
   - No steering needed — engineer continues its original task

5. On next projection refresh, the new bead appears in the
   state-store-backed team status projection and the morning report.
```

### 6.9 What We Keep from Git

Git remains the durable artifact layer:

- Engineer code changes → committed to branches (same as before)
- Beads → stored in `.beads/`, synced with `br sync --flush-only`
- ADRs, AGENTS.md, INCIDENTS.md → committed to main
- Epoch tags → git tags (the supervisor system uses these)
- Morning report / CHANGELOG → committed to main

What's gone:

- `.status.json` on engineer branches (replaced by GenServer state + traces)
- `.team-status.json` on team branches (replaced by Gleam process state)
- `.comms/` directory (never existed — deprecated before implementation)
- `REVIEW_FEEDBACK.json` files (replaced by direct ACP/message injection)
- `.dashboard.json` as a commit (replaced by live DashboardServer)

The Gleam application can persist its state to disk periodically so a crash of the BEAM node doesn't lose history. But the normal operating mode is in-memory actor state + central event log.

### 6.10 Transactional State Store, Command Ledger, and Idempotency

`state.db` is the authoritative operational source of truth. GenServers are controllers and caches over durable projections, not the sole record of bead ownership, review state, or worker liveness.

**Durable attempt records:**

```gleam
pub type BeadAttempt {
  BeadAttempt(
    bead_id: BeadId,
    attempt_no: Int,
    status: AttemptStatus,      // Planned | Leased | Running | Succeeded | Failed
    worker_id: Option(String),
    lease_expires_at: Option(Int),
    checkpoint: Option(Checkpoint),
    created_at: Int,
    updated_at: Int,
  )
}

pub type AttemptStatus { Planned  Leased  Running  Succeeded  Failed }

pub type ExecutionLease {
  ExecutionLease(
    lease_id: String,
    bead_id: BeadId,
    attempt_no: Int,
    worker_id: String,
    granted_at: Int,
    expires_at: Int,
    last_heartbeat: Int,
  )
}
```

**Dispatch protocol:**

1. Scheduler writes `PlannedAttempt` to `state.db`
2. Worker atomically claims lease (CAS on attempt status)
3. Only then may attempt-repo/capsule/session be created
4. Heartbeats extend lease ownership
5. Recovery reclaims expired leases exactly once

**Command ledger:** Every external side-effect goes through a persisted command ledger before execution:

```gleam
pub type CommandEntry {
  CommandEntry(
    id: String,               // idempotency key (e.g., "merge-epoch-7-team-model")
    state: CommandState,       // Planned | Started | Succeeded | Failed
    action: ExternalAction,    // GitMerge | GitTag | BeadClose | CargoTest | ...
    args: String,
    started_at: Option(Int),
    completed_at: Option(Int),
    result: Option(String),
  )
}

pub type CommandState { Planned  Started  Succeeded  Failed }
```

**Semantic change leases:** ResourceManager leases not only attempt repos and build slots, but logical conflict domains (public APIs, shared fixtures, workspace files, generated artifacts). Two attempts may run concurrently only if their `conflict_domains` are disjoint or explicitly declared compatible. Conflict domains are declared in manifests and computed from the code intelligence projection where available.

**Transactional outbox:** Side effects that must be visible to external systems (git tags, bead status changes, notifications) are written to a `transactional_outbox` table atomically with the state change. A background worker drains the outbox, ensuring at-least-once delivery with idempotency keys.

**Replay on recovery:** When the BEAM node restarts from a snapshot:

1. Read the command ledger and lease table.
2. For `Started` entries: check the actual external state (did the git tag get created? did the bead close?).
3. If the action completed but wasn't recorded: mark `Succeeded`.
4. If the action didn't complete: replay it (idempotent by key).
5. Never re-execute a `Succeeded` entry.
6. Expire stale leases (no heartbeat within TTL). Reclaim and reschedule exactly once.

This answers the question: "If the node crashes after a merge but before the event is recorded, what is the source of truth?" The ledger is.

### 6.11 Implementation Stack

```
Orchestration language:   Gleam (compiles to BEAM bytecode)
Runtime:                  BEAM VM (Erlang/OTP 27+)
Supervisor framework:     OTP built-ins (gen_server, supervisor)

Manager agents:
  Harness:                Claude Code
  Protocol:               ACP (Agent Client Protocol) — JSON-RPC 2.0 / stdio
  Model:                  Claude Opus (top manager, lieutenants)

Engineer agents:
  Harness:                Pi (github.com/badlogic/pi-mono)
  Extensions:             Custom TypeScript (manifest sandbox,
                          sensor injection, loop detection, trace
                          emitter, br integration)
  Auth / model backend:   Codex (OpenAI) — `pi --auth codex`
  Driver:                 Gleam EngineerWorker spawns and steers
                          Pi as a subprocess

Review / meta agents:
  Harness:                Gemini CLI (geminicli.com)
  Mode:                   One-shot `gemini -p` with structured prompt
  Invocation:             `gemini -p --model gemini-3.1-pro-preview --approval-mode plan` (per-job subprocess)
  Model:                  gemini-3.1-pro-preview (pinned per epoch)
  Driver:                 Gleam job workers spawn, capture stdout, parse
                          structured output, record trace

Git interaction:          Shell out to git CLI from Gleam
Bead interaction:          Shell out to br CLI from Gleam (and
                          exposed as a tool via br-integration
                          Pi extension for engineers)

Observability:            TraceAggregator GenServer; event log on
                          disk; optional OTLP export to external
                          backend (Honeycomb / Grafana Tempo)
Dashboard:                Gleam server-rendered (Lustre on JS target)
                          or OTP equivalent of Phoenix LiveView
Persistence:              Append-only event log + periodic
                          GenServer state snapshots
```

**What runs as a long-lived Gleam release:**

- The supervision tree
- TopManager + Lieutenant GenServers (each holding an ACP session to Claude Code)
- EngineerWorkers (one per active engineer; each holds a Pi subprocess handle)
- SessionCustodians (one per provider lane — OAuth health monitoring)
- Observability stack (TraceAggregator, DashboardServer, MetricsCollector, EventLog)

**What runs as subprocesses driven by Gleam:**

- Claude Code instances (one per manager, communicating via ACP)
- Pi instances (one per engineer, authenticated to Codex, configured with our custom extensions)
- Gemini CLI instances (ephemeral, one per review/meta job invocation via `gemini -p --model gemini-3.1-pro-preview --approval-mode plan`)
- `cargo`, `git`, `br` commands (shelled out by workers)
- Pre-commit hooks

**Why the three-provider stack (Claude managers + Codex engineers + Gemini reviewers):**

- **Managers** (Claude Code via ACP): coordination, delegation, bead-splitting — benefits from Claude Opus's reasoning. ACP provides real-time steering.
- **Engineers** (Pi + Codex): bounded implementation within a tight harness — Pi provides excellent harness primitives, Codex is strong at focused coding, per-token cost is lower at engineer volume.
- **Reviewers/meta-agents** (Gemini CLI one-shot): review personas, retro analysis, monitoring — Gemini's large context handles full diffs and epoch trace data efficiently. One-shot `gemini -p` is the natural fit: each review/analysis job is a single prompt→structured-response, no session state needed.
- We can A/B any of these: swap Pi's model backend between Codex / Sonnet / local models, swap review agents between Gemini / Haiku, without rewriting the Gleam layer. The trace aggregator gives us the data to choose.

---

## 7. Operational Loops

### 7.1 Lieutenant CAID Loop

Each lieutenant runs this independently and concurrently:

```
1. REBASE team branch onto latest epoch tag
2. QUERY: `ReadyBeads(team, capability_filter)` from BeadGraphProjection
3. RANK by critical-path score from SchedulerGenServer:
   - downstream blocker count + fan-out
   - merge-queue pressure
   - risk/autonomy ceiling
   - provider lane availability
4. DELEGATE: build task manifest, select harness template.
   --- MANIFEST REVIEW GATE ---
   - Lieutenant proposes manifest.
   - Architect persona critiques with typed findings (`MustFix | ShouldFix | Question | Nit`):
     Is the scope bounded? Are proof obligations clear?
   - Continue until no unresolved `MustFix` remain or manifest review budget is exhausted.
   - Finalize the manifest and spawn engineers in isolated attempt repos (2-5 per team). Do not block.
5. MONITOR: context utilization, retry count, stuck agents
   - >80% context or same-error loop → checkpoint & handoff
   - manager session past rotation threshold → hand off to warm spare session
6. COLLECT: await engineer completions
7. REVIEW: build deterministic review packet, then invoke selected reviewers (§5)
8. MERGE TRAIN: send StartMergeTrain([branches]) to MergeTrainWorker
   - MergeTrainWorker runs git merge + cargo test ASYNCHRONOUSLY
     (keeps the Lieutenant's ACP session responsive to top-down steering
     while CI runs — cargo test can take minutes)
   - Green: MergeTrainWorker replies MergeTrainResult(ok),
     lieutenant lands batch, br close <ids>
   - Red: MergeTrainWorker bisects, rejects breaker, relands rest
9. SIGNAL top manager: team branch ready for integration
10. Loop to step 1
```

### 7.2 Top Manager Loop

```
1. STEP 0: Verify clean state (see §9.1)
2. Dispatch all lieutenants
3. Await any lieutenant ready-signal
4. Pull team branch, run cargo check --workspace
5. Submit merge proposal to PolicyEngine + SideEffectExecutor
   (priority: Model > Contracts > Train > QA, from BeadGraphProjection)
6. Every K merges: declare epoch
   - Shell out cargo test --workspace to a SEPARATE PROCESS
     (do NOT run this as a Task inside the TopManager GenServer —
     long-running cargo builds will starve BEAM schedulers and
     block the TopManager from handling ACP steering events)
   - If passes: tag epoch-N, broadcast to all lieutenants
   - If fails: freeze, bisect, route P0 to responsible lieutenant
7. Handle cross-crate bead splits (see §8)
8. Loop to step 3
9. At end of night: generate morning report (see §9.2)
   --- REPORT REVIEW GATE ---
   - Top Manager drafts the report.
   - Integration Reviewer critiques with typed findings (`MustFix | ShouldFix | Nit`):
     Is it concise? Are failures actionable? Is the decision queue correctly ranked?
   - Continue until no unresolved `MustFix` remain or report review budget is exhausted.
   - Persist the report for the human to read. Do not block.
```

### 7.3 Loop Detection

Distinguish productive iteration from stuck loops:

```
Track: {file_path -> [(edit_count, error_hash)]} per agent session

PRODUCTIVE (allowed — burn the tokens):
  - Different files being edited (making progress)
  - Same file, different error each time (converging)
  - Test results improving (more tests passing)

UNPRODUCTIVE (detected — stop):
  - Same file, same error hash, 3+ times (spinning)
  - Test results not improving after 3 iterations
  - Agent generating and reverting the same change

Heuristic:
  if error_hash(attempt_N) == error_hash(attempt_N-2):
    STUCK → emit typed Checkpoint, handoff to fresh agent
  else:
    PROGRESSING → continue (up to context budget)
```

**Typed checkpoint schema:** Fresh-agent handoff is central to this design. A checkpoint is a typed artifact, not a vague summary:

```gleam
pub type Checkpoint {
  Checkpoint(
    bead_id: BeadId,
    manifest_version: String,
    branch: String,
    current_diff_hash: String,
    commands_run: List(CommandRecord),
    failing_sensors: List(SensorResult),
    error_signatures: List(ErrorHash),
    attempted_hypotheses: List(String),
    files_touched: List(String),
    remaining_uncertainty: String,
    recommended_next_move: String,
    context_used_pct: Int,
  )
}
```

When a fresh engineer (Pi/Codex) picks up from a checkpoint, it receives the full typed struct — not a prose summary. This makes restarts dramatically more reliable because the new agent knows exactly what was tried, what failed, and what to try next.

### 7.4 Monitor Agent (Per Team)

One `MonitorGenServer` per team watches engineer activity. **Event-driven, not git-polling** — the runtime is the live source of truth, so monitors subscribe to events rather than inferring liveness by polling branches.

```
MonitorGenServer subscribes to the event stream for its team.

on CommitEvent(eng_id, commit_hash, branch):
  # Engineer's trace-emitter extension emits this when a commit lands
  read diff (git show <commit_hash>)
  check against: bead acceptance, AGENTS.md conventions,
    dependency layers, test quality heuristics
  if issues found:
    send MonitorFlagged(eng_id, QualityIssue { ... })
      to LieutenantGenServer
    Lieutenant injects feedback into Pi/Codex engineer via
      BEAM message → EngineerWorker → Pi extension

on StateTransition(eng_id, from, to):
  # Track phase changes; detect stalls
  update last_activity[eng_id] = now()

on periodic_check (every 5 min):
  for each active engineer:
    if last_activity[eng_id] > 15 min ago:
      send MonitorFlagged(eng_id, Stale)
```

Monitors only fall back to `git log` polling if the event stream is unavailable (e.g., engineer subprocess not emitting traces). All feedback flows through BEAM messages → EngineerWorker → Pi's trace-emitter extension (see §6.5).

---

## 8. Cross-Crate Task Protocol

Tasks that touch files in 2+ CODEOWNERS areas follow one of three patterns:

### Pattern A: Consumer Task (most common)

Task lives in one crate, imports API from another. No cross-crate coordination needed — just rebase onto latest epoch.

Example: T4.5 (gbf-train uses QAT modules from gbf-model)

### Pattern B: Compatibility-Window Interface Migration

A new type/trait needs to exist in one crate, and a consumer in another needs it. Many cross-crate changes fail not because implementation is hard, but because the interface is underspecified. For breaking or widely consumed interface changes, use a staged **expand / migrate / contract** protocol.

Top manager splits the bead into **four** sub-beads:

```
Original: "T6.6: Update expert byte budget formula"
Split:
  T6.6-introduce: "Define ByteCostComputable trait + adapter layer + invariants"
    → Team Contracts
    Lands: new contract + adapter supporting both old and new paths,
           invariant docs, example consumer, compile-fail tests
    This is the EXPAND phase.

  T6.6a-producer: "Implement producer support for both old and new contract"
    → Team Contracts
    Dep: blocked-by T6.6-introduce

  T6.6b-consumers: "Migrate consumers to the new contract"
    → Model / Train / QA (may be parallel sub-beads per team)
    Dep: blocked-by T6.6-introduce

  T6.6c-cleanup: "Remove compatibility layer after all consumers migrate"
    → Owning team
    Dep: blocked-by T6.6a-producer AND T6.6b-consumers
    Deadline: must not remove old path in the same epoch unless all
    consumers migrated in the same reviewed batch.
```

This reduces stop-the-world merges and lets multiple teams move in parallel. The compatibility window gives teams time to migrate without thrash.

Each sub-bead enters normal CAID flow within its team. Cross-team dep resolves through epoch merges.

### Pattern C: Workspace Changes

Shared files (Cargo.toml, CI config). Top manager executes directly with its own small engineer pool (1-2 agents).

### Cross-Team Dependency Flow

```
1. Team Model engineer finishes T1.6 (ExportVisitor)
2. LT-Model merges to team/model
3. Top manager merges team/model → main
4. Top manager declares epoch, broadcasts update
5. LT-Contracts rebases team/contracts onto new epoch
   → T3.3 (needs ExportVisitor) now unblocked
6. LT-Train rebases team/train onto new epoch
   → T4.3b, T8.2, T7.6 now unblocked
```

### Key Cross-Team Chokepoints (This Project)

- **T1.6 (ExportVisitor)** — biggest bottleneck. Blocks 3 Train tasks, 1 Contracts task, 2 QA tasks. Prioritize.
- **T3.1 (TernaryWeightPlan types)** — blocks 1 Model task, 1 Train task, 1 QA task.
- **Circular dep**: Model needs Contracts (T6.6 ← T3.1) AND Contracts needs Model (T3.3 ← T1.6). Resolves naturally through sequenced epoch merges.

### Bead Splitting Recommendations

- **Split T1.5** into T1.5a (Top1RouterQat) and T1.5b (ExpertBlockQat). These are independent modules that agents can implement in parallel. Bundling them into one task forces a single agent to handle too much context across multiple complex files.

---

## 9. Day/Night Protocol

### 9.1 Step 0: Clean State (Start of Night Shift)

Before dispatching any work:

```
1. Verify latest epoch tag exists and tests pass
   cargo test --workspace (at epoch tag)
   If fails: this is a P0. Fix before dispatching ANY work.

2. Verify br ready is populated
   If empty: nothing to do. Stop.

3. Verify all team branches rebased on latest epoch
   For stale branches: rebase automatically.

4. Verify harness consistency
   All structural tests pass.
   AGENTS.md golden path sections exist for each crate.
   docs/decisions/ are indexed.

5. ONLY THEN: begin dispatching to lieutenants.
```

### 9.2 Morning Report

Generated by top manager at end of night shift. Emits both `morning_report.md` (human-readable) and `morning_report.json` (machine-readable, with a ranked `DecisionQueue`).

```markdown
## Night Shift Summary — {date}

### Decision Queue (ranked by unblock impact)

| Blocked Bead | Downstream Unblock Count | Recommended Options | Evidence Refs | Suggested Action |
| ------------ | ------------------------ | ------------------- | ------------- | ---------------- |
| T2.2         | 4                        | 1) Clarify PlacedRom type 2) Split bead 3) Defer | checkpoint-id-xyz, review-packet-abc | `split` |
| T6.3         | 1                        | 1) Add Burn derive examples 2) Tighten manifest | checkpoint-id-def | `write-ADR` |

Suggested action types: `accept | defer | split | tighten-risk | write-ADR`

### Completed ({N} beads)

- check T1.1: Create qat module structure [team/model]
- check T3.1: Define TernaryWeightPlan types [team/contracts]
- ...

### Blocked ({N} beads — need human input)

- T2.2: Emit RuntimeChromeBudget from runtime shell
  REASON: Bead references "PlacedRom" type that doesn't exist.
  AGENT FINDINGS: [checkpoint summary]

### Harness Issues (retro agent findings)

- 3 engineers re-derived byte_math helpers independently
  -> PR #42: Extract to gbf-artifact::byte_math
- Loop detection fired on T6.3 — Burn Module derive confusion
  -> PR #43: Add Burn derive examples to AGENTS.md

### Epoch Status

- Epoch {N} tagged at commit {hash}
- All workspace tests passing
- {N} beads remain in br ready

### CHANGELOG

[structured entries for each completed bead]
```

### 9.3 Human Morning Protocol (30-60 min)

```
1. READ decision queue first (5 min)
   What needs decisions, what's blocked, what failed

2. REVIEW commits (15-20 min)
   Skim commit messages (human advocate ensured readability)
   Spot-check diffs for anything suspicious
   Focus on blocked beads — make decisions

3. FIX THE HARNESS FIRST (10-15 min)
   Review retro agent's PRs
   If agents made mistakes: fix docs/hooks/manifests BEFORE code
   "The code fix is a symptom; the harness gap is the disease"

4. WRITE NEW SPECS (remaining time)
   Create new beads for next night's work
   Update blocked beads with human decisions
   Write ADRs for any decisions made during review

5. LOCK COMPUTER
   Night shift agents pick up from br ready
```

---

## 10. Failure Modes and Recovery

Every failure mode has an automatic recovery path via OTP supervision strategies. Unsafe lanes may freeze; unrelated safe lanes continue.

### 10.1 "Let It Crash" at the Process Level

BEAM processes are cheap. Instead of defensive programming inside each agent, we let agents crash and let supervisors restart them clean. Each supervisor has a restart intensity/period (e.g., "max 3 restarts per 60 seconds") — if a process crashes faster than that, the supervisor escalates upward.

### 10.2 Recovery Strategies by Failure Type

| Failure                               | Detected by                  | Recovery                                                                                                                                                 |
| ------------------------------------- | ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Git index.lock collision in authoritative repo | Git error in shell-out | Eliminated for engineers by design: each autonomous attempt uses its own clone. Per-repo lock queue remains only for authoritative repo mutations via SideEffectExecutor. |
| Cargo target dir lock                 | Cargo error in shell-out     | Each attempt repo uses unique `CARGO_TARGET_DIR`; if lock still occurs, fail the attempt repo and recreate it cleanly                                     |
| Engineer API call fails               | Engineer harness (Gleam)     | Retry with backoff; on persistent fail, emit `Checkpoint` msg upward                                                                                     |
| Engineer crashes (uncaught exception) | EngineerPool supervisor      | `simple_one_for_one` restart; if repeats, supervisor gives up and reports to lieutenant                                                                  |
| Engineer hits context limit           | Harness detects at ApiCall   | Prune context; if near 80%, send `InterruptAndCheckpoint`                                                                                                |
| Engineer stuck (same error hash 3x)   | Harness loop detection       | Send `Checkpoint` upward; lieutenant spawns fresh engineer with checkpoint summary                                                                       |
| Test failure                          | Engineer's `cargo test` tool | Harness injects LLM-optimized error as next message; agent iterates                                                                                      |
| Merge conflict at team level          | Lieutenant's merge worker    | Lieutenant invokes Claude Code via ACP to resolve; if can't, skip branch                                                                                 |
| Lieutenant's Claude Code hangs        | LieutenantGenServer timeout  | ACP kill + restart; supervisor counts restart                                                                                                            |
| Lieutenant crashes                    | Team supervisor              | `one_for_one` restart; context rebuilt from event log                                                                                                    |
| Epoch CI fails                        | TopManager's CI worker       | Freeze merge queue; spawn IncidentAgent; bisect; route P0                                                                                                |
| TopManager crashes                    | Application supervisor       | `one_for_one` restart; rebuild state from event log + git                                                                                                |
| BEAM node crashes                     | Persistent state snapshots   | Reboot supervision tree from last snapshot + replay events                                                                                               |
| OAuth session expired / interactive re-login required | SessionCustodian | Freeze affected lane, preserve in-flight checkpoints, emit S0 for morning re-auth. Unrelated lanes continue.                                            |
| Bead underspecified                   | Agent or lieutenant detects  | Mark bead blocked with findings; continue other work                                                                                                     |
| Unknown exception                     | Parent supervisor            | Restart within intensity limit; escalate if exceeded                                                                                                     |

### 10.3 Supervision Restart Intensity

Set per-supervisor to catch runaway crash loops:

```gleam
// Engineer pool: engineers may crash often, that's OK
simple_one_for_one_spec(max_restarts: 10, period: 60)

// Lieutenants: should not crash often
one_for_one_spec(max_restarts: 3, period: 300)

// Top manager: should effectively never crash
one_for_one_spec(max_restarts: 1, period: 600)

// If a supervisor exceeds its intensity, it crashes itself
// and its parent handles the escalation. Eventually the
// Application supervisor may decide to shut down the night
// shift and emit an emergency morning report.
```

### 10.4 Escalation to Human (S0)

The system targets partial availability, not absolute liveness:
- Safe lanes continue when isolated unsafe lanes freeze
- Global halt is correct when workspace integrity, credential integrity, or provenance integrity is in doubt

S0 flags raised into the morning report:

- Bead fundamentally unclear after multiple agent attempts
- All approaches failed (engineer, checkpoint handoff, fresh engineer — all failed)
- Supervision tree emergency shutdown (runaway restart loops)
- Cross-team architectural ambiguity (e.g., type should live in crate A or B?)
- Workspace integrity violation (unexpected state, unsigned artifacts)
- Credential integrity violation (leaked secret, expired service credential)

Human reviews these in the morning protocol (§9.3).

---

## 11. Meta-Agent Personas

Meta-agents are personas (prompt + config), not persistent processes. Each is executed as an ephemeral `MetaJobWorker` spawned by the `MetaJobs Supervisor` when scheduled or triggered. The worker loads the persona config, runs the analysis, writes outputs, and exits.

### 11.1 Retro Persona [opus] — Per Epoch

Reads completed work and proposes system improvements. **Does NOT directly edit AGENTS.md or harness config** — outputs proposals for human review to prevent rule explosion. Retro is Git-first: it analyzes exact code changes side-by-side with the manifest, review packet, execution summary, and attempt summary that caused them.

**Inputs (durable, primary):**
- `git log --show-notes=refs/notes/gbllm/* --since=<epoch_window>` on main/team refs
- Archived failed attempts under `refs/gbllm/archive/*` (with manifest/review/attempt-trace notes)
- Diffs, CODEOWNERS, ADRs, AGENTS.md

**Inputs (optional sidecar):**
- Aggregated metrics snapshots from `state.db` for coarse throughput/cost trend context

**Analyzes for:**

- Prompt failures: agent misunderstood task → propose manifest template update
- Golden path gaps: multiple agents invented different patterns → propose convention
- Hook/lint gaps: code passed precommit but broke at integration → propose new check
- Context waste: agents reading irrelevant docs → propose targeted refs
- Task description gaps: agent asked clarifying questions → propose bead template improvement
- Stuck patterns: common failure modes → propose beads or golden path additions

**Outputs:** A structured `retro_proposals.md` committed to main. **NOT** direct edits to AGENTS.md, pre-commit hooks, or manifest schemas.

--- RETRO REVIEW GATE ---
Before committing `retro_proposals.md`, a secondary Opus persona critiques the proposals
with typed findings (`MustFix | ShouldFix | Question | Nit`):
"Are these rules too specific? Do they violate the Rule Budget? Will they bloat the context?"
Continue until no unresolved `MustFix` remain or retro review budget is exhausted.
Save and commit the proposals. Do not block.

The human reviews proposals during the Morning Protocol (§9.3) and manually applies the ones that make architectural sense.

**Why not direct PRs:** Left unconstrained, the retro agent produces hyper-specific edge-case rules that bloat the golden path ("never use `clone()` on line 42 of router.rs"). Human judgment is needed to distinguish systemic patterns from one-off incidents. After the human gains confidence in the retro agent's judgment (weeks, not days), this constraint can be relaxed for specific categories.

All proposals are subject to the Rule Budget (§1): must cite 2+ recurring incidents, have an owner, have a success metric, and sunset after 3 epochs if unused. The retro agent must classify each proposal by failure taxonomy class (§1) so the human can prioritize systemic fixes over one-offs.

### 11.2 CI Health Persona [sonnet] — Weekly

Monitors and optimizes precommit duration, queueing health, lane health, and cost trends from runtime metrics (`state.db` / rolling aggregates):

**Actions:**

- Parallelize independent precommit steps
- Prune checks that haven't caught issues in N epochs → move to postcommit
- Shard tests so agents only run tests relevant to modified files
- Quarantine flaky tests (>2% flake rate) → create fix beads
- Configure shared build cache (sccache) for worktrees

**Target:** Precommit P50 under 15 seconds.

### 11.3 Entropy Management Personas [haiku] — Weekly/Per-Epoch

**Doc-Sync Agent (weekly):**

- Verify ADRs reference things that still exist in code
- Verify AGENTS.md golden path matches actual code patterns
- PR to update or archive stale docs

**Dead-Code Agent (weekly):**

- `cargo-udeps` for unused dependencies
- Scan for pub functions with zero callers
- PR to remove dead code, or create bead if non-trivial

**Constraint-Drift Agent (per epoch):**

- Run all structural tests
- Scan for patterns that pass CI but violate conventions
- PR to fix, or create bead if complex

---

## 12. Model Tiers (Reasoning Sandwich, Mixed Provider)

Different harnesses and providers for different roles. Role→provider bindings are fixed during operational runs to preserve budget accounting and model-feel comparability. Runtime surface changes are evaluated only in replay/shadow lanes and adopted only at epoch boundaries.

| Role              | Harness                | Provider/Model      | Process Type | Why                                                                                                 |
| ----------------- | ---------------------- | ------------------- | ------------ | --------------------------------------------------------------------------------------------------- |
| Top Manager       | Claude Code (ACP)      | Claude Opus         | Persistent   | Cross-team coordination, bead splitting, epoch decisions — needs highest reasoning + real-time steering |
| Lieutenants       | Claude Code (ACP)      | Claude Opus         | Persistent   | Task delegation, semantic review, dependency analysis — steered live via ACP                          |
| Engineers         | Pi + custom extensions | Codex (OpenAI auth) | Persistent   | Bounded implementation; Pi's harness primitives + Codex's coding strength at engineer per-token cost  |
| Style Reviewer    | Ephemeral review job   | Codex or Haiku               | Ephemeral    | Mechanical pattern matching when policy requests it                                                   |
| Review Personas   | Gemini CLI (`-p`)      | gemini-3.1-pro-preview       | Ephemeral    | Risk-selected review jobs; usually 0-2 post-impl plus Epistemologist at proof gate                    |
| Retro             | Gemini CLI (`-p`)      | gemini-3.1-pro-preview       | Ephemeral    | Per-epoch meta-analysis over trace data; benefits from Gemini's large context                         |
| CI Health         | Gemini CLI (`-p`)      | gemini-3.1-pro-preview       | Ephemeral    | Weekly metrics analysis, precommit optimization                                                       |
| Entropy GC        | Gemini CLI (`-p`)      | gemini-3.1-pro-preview       | Ephemeral    | Weekly/per-epoch: doc-sync, dead-code, constraint-drift                                               |
| Monitor           | Gemini CLI (`-p`)      | gemini-3.1-pro-preview       | Ephemeral    | Per-commit diff review against known rules                                                            |

**Three-provider reasoning sandwich:**

- **Claude (ACP)** at top — managers, lieutenants. Highest reasoning, real-time ACP steering for coordination and delegation.
- **Codex (Pi)** in middle — engineers. Bounded implementation within a tight harness. Pi provides harness primitives; Codex provides focused coding.
- **Gemini (`-p` one-shot)** at edges — review personas, retro, CI health, entropy GC, monitors. Each job is a single `gemini -p --model gemini-3.1-pro-preview --approval-mode plan` invocation with a structured prompt and structured output parsing. No session state, no ACP — one-shot is the natural fit for ephemeral review/analysis work. Gemini's large context window is well-suited for reviewing diffs and reading trace data. Usually 0-2 risk-selected post-impl reviewers plus the Epistemologist at the proof gate.

**Why three providers:**

- Each role gets the best model for its cognitive load and cost profile.
- Managers need Claude's reasoning + ACP steering for real-time coordination decisions.
- Engineers need Codex's coding strength within Pi's proven harness framework.
- Reviewers/scanners need fast, cheap inference over large context — Gemini excels here.
- The Gleam layer abstracts process lifecycle, tracing, and supervision. This project intentionally keeps role→provider bindings fixed during operational runs to preserve budget accounting and model-feel comparability. Runtime surface changes are evaluated only in replay/shadow lanes and adopted only at epoch boundaries.

---

## 13. Phased Rollout

**Do not build the full supervision tree immediately.** Conway's Law applies: you don't have 4 teams of code yet, so don't build 4 teams of agents. Phase transitions are **criteria-driven, not time-driven**.

### 13.1 Benchmark Suite

#### Shadow Lanes and Replay

Every benchmark bead can run in three execution modes:
- **Authoritative mode** — normal execution, writes land
- **Shadow mode** — writes diverted or disabled; same manifest and context pack through an alternate runtime or review policy
- **Replay mode** — frozen context pack + frozen artifact inputs; reruns a frozen attempt against the same inputs

Shadow mode is the default mechanism for evaluating:
- New providers before making them hard dependencies
- New review personas or prompt/context changes
- Autonomy promotions and risk threshold adjustments

This lets you compare providers, prompt compilers, review personas, context policies, and autonomy thresholds without confounding them with different tasks.

#### Adapter Conformance Suite

Every `SessionAdapter` must pass a deterministic conformance + fault-injection suite before its lane is promoted to production:
- Transcript replay (known input → expected output sequence)
- Malformed or partial frame injection (graceful error, no crash)
- Stray stdout/stderr injection (does not corrupt JSON-RPC stream)
- Missing usage/cost fields (handled gracefully, does not break trace)
- Auth-expired / reauth-required transitions (freeze lane, emit S0)
- Resume rejection (checkpoint and fresh restart)
- Duplicate or out-of-order events (idempotent handling)
- Abrupt subprocess exit mid-turn (detect, checkpoint, report)

Provider-lane promotion requires green benchmark beads AND green adapter conformance. Conformance results are stored as artifacts.

#### Benchmark Beads

A fixed set of representative beads, stratified by task class, used to evaluate each phase:

| Task Class            | Representative Beads   | What It Tests                                 |
| --------------------- | ---------------------- | --------------------------------------------- |
| Leaf new module       | T1.1, T3.1, T7.1, T9.1 | Scaffolding, type definitions                 |
| Leaf implementation   | T1.2, T6.1, T6.3       | Bounded coding within a crate                 |
| Cross-crate interface | T6.6, T3.3             | Contract-first split, multi-team coordination |
| Test-only             | T10.1, T10.2, T10.4    | Proof-obligation quality, hermetic testing    |
| Infra/config          | T1.8, T2.2             | Workspace changes, CI integration             |
| Experiment/training   | (future M3+ beads)     | Eval plans, metric thresholds                 |

### 13.2 Core Metrics

| Metric                                      | What It Measures                            | Tracked Per            |
| ------------------------------------------- | ------------------------------------------- | ---------------------- |
| Merged beads per night                      | Throughput                                  | Session                |
| Escaped defect rate                         | Quality (defects found after merge to main) | Epoch                  |
| Revert rate within 7 days                   | Stability                                   | Rolling window         |
| Human review minutes per merged bead        | Human load                                  | Session                |
| Token cost per merged bead                  | Efficiency                                  | Session, by task class |
| Mean time to green (proof obligations pass) | Cycle time                                  | Per bead               |
| Integration conflict rate                   | Isolation quality                           | Epoch                  |
| Stuck/checkpoint rate                       | Harness quality                             | Per task class         |

### 13.3 Phase 0a: Single Vertical Slice

**Goal:** Validate one complete loop end-to-end before mixed-provider orchestration.

#### 0a Build Manifest

**Gleam processes (supervision tree):**

```
Application Supervisor (one_for_one)
├── SchedulerGenServer         — dep-aware ready-set; priority + blocker-count ranking; no cost model
├── PolicyEngineGenServer      — manifest admission, risk classification, 1 circuit breaker per engineer
├── ResourceManagerGenServer   — attempt-repo + capsule alloc/cleanup, build-slot cap = 1
├── SideEffectExecutorGenServer — full command ledger (load-bearing from day 1)
├── BeadGraphProjection        — DAG materialized from `br` CLI output
├── ManagerGenServer           — combined top-manager + lieutenant; serial dispatch loop
├── EngineerWorker (×1)        — spawns Pi subprocess, monitors via trace channel
└── TraceAggregator            — events → state.db events table; no rolling aggregates
```

**State store (`state.db`):**

| Table | Purpose |
|---|---|
| `events` | Append-only event log (agent_id, event_type, payload, timestamp) |
| `commands` | Command ledger (id, state, action, args, timestamps, result) |
| `bead_attempts` | Durable attempt lifecycle (bead_id, attempt_no, status, worker_id, lease_expires_at) |
| `execution_leases` | Lease ownership (lease_id, bead_id, worker_id, granted_at, expires_at, last_heartbeat) |
| `transactional_outbox` | At-least-once side-effect delivery (action, payload, processed_at) |
Full lease claim/heartbeat/expire. Full command replay on BEAM restart. Git-notes provenance written at land time (review packets, proof commits, execution summary → git notes on landed commits).

**Session adapters:**

| Adapter | Transport | Role |
|---|---|---|
| `SessionAdapter` trait | — | Full interface: start, steer, interrupt, capabilities |
| `ClaudeSessionAdapter` | ACP only              | Manager lane |
| `PiSessionAdapter` | Subprocess + trace channel | Engineer lane |

**Pi extensions:** `manifest-sandbox.ts` (path enforcement), `trace-emitter.ts` (event streaming to Gleam), `loop-detection.ts` (stuck detection — error hash tracking, Checkpoint emission).

**Review:** Deterministic only — `cargo fmt --check`, `cargo clippy`, `cargo test`, path compliance. Structured review packet (JSON). No inferential reviewers.

**Context:** Flat file injection — AGENTS.md + bead description + ADR refs into Pi's context loading. No typed ContextCards, no compiler, no provider-specific rendering.

**Morning report:** `morning_report.md` only. Sections: completed, blocked, failed. No JSON, no decision queue.

**Gleam types needed:** `TaskManifest`, `EngineerMsg`, `EngineerState`, `Trace`, `CommandEntry`, `BeadAttempt`, `ExecutionLease`. Full `TopManagerMsg`/`LieutenantMsg` protocol deferred.

#### 0a Deferred

Security Supervisor · LieutenantGenServer · Team Supervisors · MonitorGenServer · MergeTrainWorker · MergeQueueGenServer · Meta personas · GeminiCliAdapter · inferential review · typed ContextCards · context compiler · shadow/replay lanes · QueueSnapshot · DashboardServer · MetricsCollector · cost-aware scheduling · `sensor-injection.ts` · `br-integration.ts` · morning report JSON/decision queue.

#### Phase 0a Bead Slice (7 beads, 3-layer dependency chain)

```
Layer 0 (no deps):
  T0.1 (bd-1bi) ─── Scaffold Cargo workspace
                     R3/X1, infra/config task class
                     Unblocks everything

Layer 1 (blocked by T0.1, all independent):
  T0.2 (bd-yb3) ─── Set up pre-commit hook (fmt, clippy, test gates)
  │                  R1/X1, infra task class
  │                  Establishes Tier 1/2 sensor baseline. Without this the
  │                  engineer operates without its mechanical straightjacket.
  │                  [currently IN_PROGRESS — verify/complete in 0a]
  │
  T0.3 (bd-1z3) ─── Define foundation types (gbf-foundation)
  │                  R2/X1, type-definition task class
  │                  Validates: newtype proof obligations, compile-time type safety
  │
  T1.1 (bd-2q4) ─── Create QAT module structure (gbf-model)
                     R0/X1, scaffolding task class
                     Validates: lowest-risk auto-merge candidate

Layer 2 (blocked by Layer 1):
  T10.1 (bd-mov) ── Create test fixtures + helpers (gbf-test)
  │                  R0/X1, test-infrastructure task class
  │                  Provides: deterministic_tensor(), assert_tensor_close(),
  │                  assert_bytes_equal(), tiny model config. Without this
  │                  T1.2 engineer invents bespoke test utilities that
  │                  pollute the repo with non-standard fixtures.
  │
  T3.1 (bd-1jl) ─── Define TernaryWeightPlan + byte math (gbf-artifact)
                     R2/X1, implementation-with-logic task class
                     Validates: proof-first with property tests (byte cost monotonicity),
                     cross-crate type consumption (uses foundation types)

Layer 3 (blocked by T1.1; uses T10.1 fixtures and T3.1 types):
  T1.2 (bd-3p5) ─── Implement TernaryLinearQat (gbf-model)
                     R1/X1, bounded implementation task class
                     Validates: real proof obligations (STE numerics, ternary projection),
                     stacked commits, review gate with substance.
                     Manifest context_refs MUST include T10.1 fixtures path so
                     engineer uses assert_tensor_close, not ad-hoc helpers.
```

**Why this slice:**

| Harness Capability Under Test         | Bead(s) That Exercise It                                  |
| ------------------------------------- | --------------------------------------------------------- |
| Tier 1/2 sensor baseline              | T0.2 (pre-commit hook enforces fmt/clippy/test)           |
| Dependency-aware dispatch             | T0.3+T1.1 unblock after T0.1; T3.1 after T0.3            |
| Parallel dispatch                     | T0.2, T0.3, T1.1 are independent after T0.1              |
| Proof-first workflow                  | T3.1 (property tests), T1.2 (STE/ternary numerics)       |
| Standard test infrastructure          | T10.1 fixtures consumed by T1.2 (no ad-hoc test helpers)  |
| Manifest path sandboxing              | 4 crates (foundation, model, artifact, test)              |
| Risk class range                      | R0 (T1.1, T10.1) → R1 (T0.2, T1.2) → R2 (T0.3, T3.1) → R3 (T0.1) |
| Task class diversity                  | infra, scaffold, type-def, test-infra, impl-with-logic    |
| State store / lease lifecycle         | 7 attempts across 4 dispatch rounds                       |
| Stacked commits                       | T1.2 (test → impl → fix pattern)                         |

**Dispatch order:**

1. Dispatch T0.1. Wait for completion. Merge to integration.
2. Dispatch T0.2, T0.3, and T1.1 (serial with cap=1, scheduler orders by priority). Merge each on completion.
3. Dispatch T10.1 and T3.1 (serial). Merge each on completion.
4. Dispatch T1.2 (manifest `context_refs` includes T10.1 fixtures). Merge on completion.
5. Tag `epoch-0a`. Run `cargo test --workspace`. Human reviews morning report.

#### 0a Exit Criteria

- All 7 beads complete the full loop: dispatch → proof-first → implement → deterministic review → merge to integration.
- Pre-commit hook (T0.2) rejects at least one engineer commit during the run (proves the sensor gate works).
- T1.2 implementation uses T10.1's `assert_tensor_close` and `deterministic_tensor` — not ad-hoc helpers.
- At least one merge is reverted and re-landed to validate revert workflow.
- State store survives a simulated BEAM restart between rounds 2 and 3 (leases reclaimed, dispatch resumes).
- Human promotes integration → main and verifies `cargo test --workspace` passes.

### 13.4 Phase 0b: Adapter + Replay Validation

**Promotion gate from 0a:** All 0a exit criteria met. At least one full loop (dispatch → implement → review → merge to integration → human promote to main) succeeds cleanly.

#### 0b Build Manifest (delta from 0a)

**Supervision tree changes:** No new process types. `ManagerGenServer` upgraded to invoke Gemini in shadow mode. Engineer concurrency cap raised from 1 to 2.

**New session adapter:**

| Adapter | Transport | Role |
|---|---|---|
| `GeminiCliAdapter` | One-shot `gemini -p --model gemini-3.1-pro-preview --approval-mode plan` | Review personas (shadow-only in 0b) |

`GeminiCliAdapter` must pass the adapter conformance suite (§13.1) before any lane usage.

**Review in 0b (shadow only — no authoritative Gemini merge gating yet):**
- Run Epistemologist and Semantic Reviewer in replay/shadow mode against 0b beads
- Collect typed findings (`MustFix | ShouldFix | Question | Nit`), disagreement data, and convergence metrics
- Compare shadow findings against deterministic-only review outcomes — measure precision/recall
- Deterministic review remains the authoritative merge gate in 0b
- Epistemologist calibration: tune aggressively against 0a beads (T0.3, T3.1 proof commits) — reject proofs that rely on runtime checks instead of compile-time type constraints

**New Pi extension:** `sensor-injection.ts` — LLM-optimized error reformatting for cargo output.

**Context (upgrade):** Provider-specific bundles — Claude bundle for manager, Pi bundle for engineer, Gemini bundle for reviewer. Basic card extraction from AGENTS.md sections.

**State store (new tables):**

| Table | Purpose |
|---|---|
| `review_packets` | Typed findings, reviewer ID, evidence refs, convergence state |
| `provenance` | Per-attempt: adapter, provider, model, auth lane, context_pack_hash |

**Morning report (upgrade):** Markdown + JSON. Basic decision queue (blocked beads with recommended options).

#### 0b Bead Slice (7 beads, 3 crates)

All unblocked by Phase 0a outputs:

| Bead | Crate | Risk | Why in 0b |
|---|---|---|---|
| T1.3 (bd-pk3) | gbf-model | R1 | ActFakeQuant — ML numerics, rich proof obligations for shadow Epistemologist |
| T1.4 (bd-2fi) | gbf-model | R1 | NormApproxQat — norm approximation, real proof target |
| T6.1 (bd-a8k) | gbf-model | R1 | MoE block config — architecture logic |
| T6.3 (bd-2v4) | gbf-model | R1 | Tied embeddings — implementation with weight sharing |
| T2.1 (bd-yfv) | gbf-policy | R2 | RuntimeChromeBudget types — tests that R2 gets heavier shadow review than R0 |
| T4.1 (bd-ptk) | gbf-train | R1 | Training phase types + TOML config — different crate, different task class |
| T10.2 (bd-3aq) | gbf-train | R1 | Logging framework — infra task, tests sensor-injection extension |

Dispatch with 2 concurrent engineers, ~4 rounds:
1. T1.3 + T1.4 (parallel, same crate, independent)
2. T6.1 + T6.3 (parallel, same crate, independent)
3. T2.1 + T4.1 (parallel, different crates)
4. T10.2

#### 0b Deferred

Security Supervisor · Lieutenant tree · Team Supervisors · MonitorGenServer · MergeTrainWorker · Meta personas · typed ContextCards · context compiler · QueueSnapshot · DashboardServer · cost-aware scheduling · `br-integration.ts` · **live** inferential review gating.

#### 0b Exit Criteria

- All 7 beads complete with deterministic review (authoritative) + Gemini review (shadow).
- Adapter conformance suite green for `GeminiCliAdapter`.
- Multi-provider traces complete and correlated: same `bead_id` across Claude (manager), Codex (engineer), Gemini (shadow reviewer) events in `state.db`.
- Shadow Epistemologist findings show acceptable precision (>70% of `MustFix` findings are genuine, validated by human spot-check).
- At least 2 beads dispatched concurrently in one round (validates parallel resource management + conflict domain leasing).
- Fallback if Epistemologist calibration fails: inject better context (ADRs, CONSTITUTION.md type-safety excerpts) into the Gemini bundle and re-run in shadow mode.

### 13.5 Phase 0c: Live Mixed-Provider Review

**Promotion gate from 0b:** Adapter conformance suite green. Shadow reviewers show acceptable precision/recall on 0a+0b beads. No adapter crashes or stream corruption during 0b.

#### 0c Build Manifest (delta from 0b)

**Key change:** Gemini review moves from shadow to authoritative. This is the only variable that changes — per the epoch-pinning philosophy, do not also change provider versions, context packs, or org topology.

**Review (live):**
- Epistemologist mandatory at proof gate for all beads
- Semantic Reviewer mandatory post-impl for R1+ beads
- R0 beads use reviewer shadow-sampling only (not mandatory)
- `ReviewerSelector` — risk-based routing per §5.2
- Review feedback loop with convergence budget

#### 0c Bead Slice

Run 5-7 additional beads from `br ready` — prioritize beads that depend on 0b outputs (T3.2, T6.2, etc.) to test cross-phase dependency flow with live review in the loop.

#### 0c Exit Criteria

- All beads complete with live Gemini review in the authoritative loop.
- Epistemologist catches at least one genuine proof weakness (not just style nits) — validates the review gate adds value beyond deterministic checks.
- No review-induced infinite churn (convergence budget works).
- Combined 0a+0b+0c throughput matches or exceeds single-agent baseline WITHOUT worse escaped defect rate, over at least 19 total beads.

### 13.6 Phase 1: Flat CAID

**Promotion gate from 0c:** All 0c exit criteria met.

#### Phase 1 Build Manifest (delta from 0c)

**Supervision tree (full Phase 1 shape):**

```
Application Supervisor (one_for_one)
├── ControlPlane Supervisor (one_for_one)
│   ├── SchedulerGenServer          — priority + fan-out + merge-pressure ranking
│   ├── PolicyEngineGenServer       — full risk gates, per-risk-class circuit breakers,
│   │                                 token + wall-clock budget enforcement
│   ├── ResourceManagerGenServer    — concurrent attempt-repo + capsule management, memory-based engineer cap
│   ├── SideEffectExecutorGenServer — unchanged
│   └── BeadGraphProjection         — unchanged
│
├── TopManagerGenServer              — full autonomous CAID loop (§7.2)
├── MergeQueueGenServer              — ordered merge queue, serial but prioritized
│
├── EngineerPool Supervisor (simple_one_for_one)
│   ├── EngineerWorker (×N, cap 3-4 based on RAM)
│   └── ...                          — each with unique CARGO_TARGET_DIR
│
└── Observability Supervisor (rest_for_one)
    ├── TraceAggregator              — unchanged
    └── MetricsCollector             — rolling aggregates (tokens/bead, latency, stuck rate)
```

**Key upgrades from 0b:**
- `TopManagerGenServer` separated from `ManagerGenServer` — runs full CAID loop autonomously (query → rank → delegate → monitor → review → merge → morning report)
- `MergeQueueGenServer` — ordered merge queue with priority ranking
- `EngineerPool` — `simple_one_for_one` supervisor, dynamic spawn/kill, 3-4 concurrent workers, each with isolated attempt repo + minimal `ExecutionCapsule` (unique HOME, provider config roots, TMPDIR, CARGO_TARGET_DIR; shared read-only dep caches)
- `CodemodWorker` — routes `mechanical-transform` tasks away from EngineerWorker where possible; executes deterministic transforms and hands results to normal review flow
- `PolicyEngineGenServer` — full risk-class admission, per-risk-class circuit breakers, token + wall-clock budget enforcement
- `MetricsCollector` — rolling aggregates from raw events (tokens/bead, latency P50/P95, stuck rate, cost per bead)

**Review (upgrade):**
- Full deterministic packet: API diff, coverage check, structural tests, invariant checklist
- Full reviewer pool: Epistemologist (proof gate), Semantic (post-impl), Integration (for R2+)
- `ReviewerSelector` with risk-based routing per §5.2
- Review convergence budget (no infinite loops — `MustFix`-based termination)

**Context (upgrade):**
- Typed `ContextCard`s: `ADRCard`, `ExampleCard`, `IncidentCard`, `InvariantCard`, `ChecklistCard`, `GlossaryCard`
- Card selection per manifest (relevant cards only)
- Provider-specific rendering from card set

**Pi extensions (full set):** Add `br-integration.ts` (engineers can create sub-beads for discovered work). All 5 extensions operational.

**Morning report (full):** `morning_report.md` + `morning_report.json`. Decision queue ranked by unblock impact with evidence refs and suggested actions.

**Task manifest (upgrade):** Harness template selection from task type (new-module, bug-fix, cross-crate-interface, test, experiment). Manifest generation from bead + template.

**State store (new table):**

| Table | Purpose |
|---|---|
| (no new tables) | Git-notes provenance handles review packets and proof commits directly |

#### Phase 1 Throughput Target

Run any beads from `br ready`. Target: **>5 merged beads per night** for 3 consecutive nights. Integration branch, not main. Human promotes during morning review.

#### Phase 1 Deferred

Lieutenant tree · Team Supervisors · MonitorGenServer · MergeTrainWorker · Meta personas · Security Supervisor · QueueSnapshot / snapshot-based merge validity · DashboardServer (web) · cost-aware scheduling with duration prediction · hardened capsules with cgroups/IO quotas · shadow/replay beyond adapter validation.

#### Phase 1 Exit Criteria

- >5 merged beads per night for 3 consecutive nights
- Escaped defect rate < 10% over 15+ merged beads
- No stuck loops requiring human intervention to recover (loop detection + checkpoint handoff works)
- Morning report is actionable: human morning protocol completes in < 45 min
- Flat orchestration is NOT yet the bottleneck (if it is, skip to Phase 2)

### 13.7 Phase 2: Lieutenant Tree

**Promotion gate:** Flat orchestration is the measured bottleneck (top manager spending >50% of tokens on merge ordering rather than delegation), AND Phase 1 metrics stable for 5+ consecutive nights.

Symptoms that trigger Phase 2:
- Git index.lock contention between concurrent engineers
- Engineers stepping on each other's crate boundaries
- Top manager context saturated by merge coordination rather than delegation

#### Phase 2 Build Manifest (delta from Phase 1)

**Supervision tree (full §6.2 shape):**

```
Application Supervisor (one_for_one)
├── ControlPlane Supervisor (one_for_one)
│   ├── SchedulerGenServer          — per-team dispatch queues, cross-team dep resolution,
│   │                                 starvation aging for hard tasks
│   ├── PolicyEngineGenServer       — per-team + per-lane circuit breakers
│   ├── ResourceManagerGenServer    — multi-team attempt-repo management, shared build cache
│   ├── SideEffectExecutorGenServer
│   └── BeadGraphProjection
│
├── Security Supervisor (one_for_one)
│   ├── NetworkPolicyWorker          — egress allowlists per manifest
│   └── SigningWorker                — signed epoch tags and merge decisions
│
├── TopManager Supervisor (one_for_one)
│   ├── TopManagerGenServer          — cross-team coordination only, epoch declaration,
│   │                                 cross-crate bead splitting (§8 Pattern B)
│   └── MergeQueueGenServer          — epoch-level merge ordering with QueueSnapshot
│
├── Team Supervisors (×4, one_for_one)
│   ├── team_model Supervisor
│   │   ├── LieutenantGenServer      — Claude Code via ACP, per-team CAID loop (§7.1)
│   │   ├── MonitorGenServer         — event-driven engineer monitoring (§7.4)
│   │   ├── MergeTrainWorker         — batch merge + CI + bisect on failure
│   │   └── EngineerPool (simple_one_for_one, 2-5 workers)
│   ├── team_contracts Supervisor (same shape)
│   ├── team_train Supervisor
│   └── team_qa Supervisor
│
└── Observability Supervisor (rest_for_one)
    ├── TraceAggregator
    ├── MetricsCollector
    ├── DashboardServer              — live web view (HTTP, localhost:4000)
    └── EventLog
```

**Key new components:**
- 4 `LieutenantGenServer`s — Claude Code via ACP, each runs independent CAID loop per team (§7.1)
- 4 `MonitorGenServer`s — event-driven, subscribe to team event streams, detect stalls, flag quality issues (§7.4)
- 4 `MergeTrainWorker`s — batch merge of completed branches, snapshot-based CI validation, bisect on failure
- Full Gleam message protocol: all types from §6.3 now active (`TopManagerMsg`, `LieutenantMsg`, `EngineerMsg`, `EngineerState`)
- `TopManagerGenServer` restructured — no longer dispatches to engineers directly; coordinates lieutenants, declares epochs, handles cross-crate splits
- `QueueSnapshot` builder — merge validity pinned to base SHA + candidate list + dependency closure + hashes; invalidate and requeue on base move
- `DashboardServer` — live web view of all teams, engineer states, merge queues, rolling metrics
- `SigningWorker` — signed epoch tags and merge decisions
- `NetworkPolicyWorker` — egress allowlists per manifest

**Merge system (new):**
- Team branches: `team/model`, `team/contracts`, `team/train`, `team/qa`
- Epoch tagging on main after workspace CI passes
- Snapshot-based merge validity (CI results valid only for exact `QueueSnapshot`)
- Merge priority derived from live dependency graph (bootstrap: Model > Contracts > Train > QA)

**Context (upgrade):** Card usefulness tracking from traces. Begin pruning low-value cards based on retry/green-rate correlation.

**Autonomy promotion:** R0/R1 beads may auto-merge to team branches. R2/R3 still require human promotion to main.

#### Phase 2 Throughput Target

Target: **>10 merged beads per night** across all teams.

#### Phase 2 Deferred

Meta personas (retro, CI health, entropy GC) · SecretBrokerWorker, SBOMScannerWorker (no X2/X3 tasks yet) · cost-aware scheduling with ML-based prediction · hardened capsules with cgroups/IO quotas · full shadow/replay lab.

#### Phase 2 Exit Criteria

- >10 merged beads per night for 5+ consecutive nights
- Cross-team dependency resolved via epoch tagging at least once (team A's output unblocks team B)
- Merge train successfully batches and lands 3+ branches in a single CI run
- Merge conflict rate < 5%
- Human morning review < 45 min

### 13.8 Phase 3: Meta-Agents + Cost-Aware Scheduling

**Promotion gate:** Human spending >30 min/morning on retro analysis that could be automated, AND harness stable for 5+ epochs, AND Phase 2 metrics stable.

#### Phase 3 Build Manifest (delta from Phase 2)

**New processes:**

```
MetaJobs Supervisor (simple_one_for_one)  — now activated with scheduled meta personas:
│   └── MetaJobWorker              — ephemeral; spawned per invocation with persona config:
│                                     Retro (per-epoch), CIHealth (weekly), EntropyGC (weekly/per-epoch)
│
├── Security Supervisor (upgrade, all 4 workers)
│   ├── NetworkPolicyWorker        — unchanged
│   ├── SigningWorker              — unchanged
│   ├── SecretBrokerWorker         — task-scoped credentials for X2 tasks
│   └── SBOMScannerWorker          — secret scanning, vulnerability audit
│
│   (new worker under ControlPlane)
├── DurationModelWorker            — predicts wall-clock / token / build-slot cost from traces
```

**Key upgrades:**
- `SchedulerGenServer` — cost-aware ranking: expected value per constrained resource (fan-out × success probability / predicted cost), provider-lane backpressure modeling, starvation aging
- `DurationModelWorker` — simple prediction model trained from historical trace data
- Full git-notes provenance pipeline: all note refs active, signed receipts, archive refs for failed attempts
- Morning report auto-populated from meta-agent outputs (retro proposals, CI health findings)

**State store (upgrade):**

| Table | Purpose |
|---|---|
| `provenance_index` | Index of git-notes provenance for fast querying (bead_id → commit → note digests) |
| `duration_predictions` | Historical task durations, costs, success rates for scheduling model |

**Autonomy promotion:** R0 beads auto-merge to main (docs, comments, test-only). R1 auto-merge to team branches.

#### Phase 3 Throughput Target

Target: **>15 merged beads per night.** Human morning review **< 30 min** (meta-agents handle retro analysis).

#### Phase 3 Deferred

Hardened capsules with cgroups/IO quotas · full shadow/replay lab · full reviewer calibration from outcome labels · R1 auto-merge to main · additional teams beyond 4.

#### Phase 3 Exit Criteria

- Retro agent produces actionable proposals (>50% acceptance rate by human)
- CI Health agent maintains precommit P50 < 15s
- Human morning review < 30 min for 10+ consecutive sessions
- R0 auto-merge to main for 3+ epochs without revert
- Cost per merged bead trending down (scheduling efficiency improving)

### 13.9 Phase 4: Full Scaling

**Promotion gate:** Escaped defect rate < 5% over 20+ epochs, AND R0/R1 auto-merge stable for 10+ epochs.

#### Phase 4 Build Manifest (delta from Phase 3)

**Key additions:**

- **Shadow lanes (full):** Shadow execution mode for A/B testing harness changes, prompt compiler tweaks, review persona adjustments. Replay lab for rerunning frozen attempts against same inputs. Shadow mode as default mechanism for evaluating new providers, personas, and autonomy promotions before production use.
- **Hardened execution capsules:** Phase 1's minimal capsules (unique HOME, config roots, TMPDIR, target dir) upgraded with: cgroup CPU/RAM/IO quotas, read-only base mounts, explicit network egress policy enforcement, content-addressed build cache keyed by toolchain + lockfile + feature set.
- **Full review pipeline:** All 5 personas active (Epistemologist, Semantic, Integration, Performance, Operability). Reviewer calibration from outcome labels (`accepted`, `reverted`, `escaped_defect`, `reviewer_false_positive`). Mutation testing (`cargo-mutants`) as default review check for R1+ beads.
- **Scaling infrastructure:** Additional teams (LT-PLATFORM, LT-COMPILER) when crate volume warrants. Dedicated merge coordinator for >6 teams. Codemod dispatch for mechanical refactors (AST transforms over agent swarms).

**Autonomy promotion:** R1 beads auto-merge to main. R2 beads may auto-merge to team branches with 2+ reviewer consensus.

#### Phase 4 Throughput Target

Throughput scales with team count. Per-team throughput should be stable; total scales linearly with teams added.

#### Phase 4 Exit Criteria

Steady-state operational phase. Continuous improvement via:
- Shadow-lane experiments validate harness changes before deployment
- Escaped defect rate maintained < 5%
- Cost per merged bead trending down
- Reviewer calibration data drives persona improvements
- Capsule isolation prevents cross-engineer interference

### 13.10 Control Plane Isolation

The control plane (`/control-plane/` subtree or separate repo) must not be self-editable by the agent org during night shifts. A self-editing agent org that can mutate its own scheduler, auth policy, and harness during the same operational cadence is much harder to reason about than a control plane with its own release train.

```
/control-plane/
  /scheduler/
  /policy/
  /adapters/
  /context-pack-compiler/
  /provenance/
  /benchmarks/
```

Night shifts must not self-edit `/control-plane/` except through a breakglass workflow.

### 13.11 Git-Notes Provenance and Outcome Labels

#### Git-Notes Provenance

All merged beads publish compact, canonical JSON git notes under dedicated `refs/notes/gbllm/*` refs. Git is the durable provenance layer for merged work. `state.db` remains the operational source of truth for in-flight work only.

**Note refs:**

| Ref | Content | Writer |
|---|---|---|
| `refs/notes/gbllm/manifest` | Exact `TaskManifest` used for the landed attempt | SideEffectExecutor |
| `refs/notes/gbllm/review` | Deterministic review packet + inferential verdicts | SideEffectExecutor |
| `refs/notes/gbllm/execution` | Tokens, duration, models, adapters, queue snapshot | SideEffectExecutor |
| `refs/notes/gbllm/attempt-trace` | Compact summary of failed iterations/checkpoints | SideEffectExecutor |
| `refs/notes/gbllm/receipt` | Signed note-digest receipt (tamper-evident) | SigningWorker |

Only `SideEffectExecutorGenServer` may write authoritative provenance notes. Only `SigningWorker` may write `refs/notes/gbllm/receipt`. Engineer lanes may read notes but may not mutate note refs directly.

**Notes configuration (all authoritative repos and attempt repos):**

```
git config --add notes.displayRef refs/notes/gbllm/manifest
git config --add notes.displayRef refs/notes/gbllm/review
git config --add notes.displayRef refs/notes/gbllm/execution
git config --add notes.displayRef refs/notes/gbllm/attempt-trace
git config --add notes.displayRef refs/notes/gbllm/receipt
git config notes.rewrite.rebase true
git config notes.rewrite.amend true
git config --add notes.rewriteRef refs/notes/gbllm/*
git config notes.rewriteMode concatenate
```

**Signed provenance receipt:** At land time, `SigningWorker` signs a receipt containing: annotated commit SHA, stack head SHA, note ref → canonical payload digest map, queue snapshot hash, and command ledger IDs used for the merge/promotion. This makes note-based provenance tamper-evident even though notes live outside the commit object.

**Land-time provenance flow:**

1. `PolicyEngineGenServer` approves manifest, persists to `state.db` for live coordination.
2. `ResourceManagerGenServer` allocates attempt repo + capsule.
3. `EngineerWorker` runs. `ReviewJobWorker` produces structured review data.
4. `TraceAggregator` compacts raw attempt events into attempt-trace summary.
5. `MergeTrainWorker` gets green candidate, asks `SideEffectExecutorGenServer` to write provenance.
6. `SideEffectExecutorGenServer` writes manifest, review, execution, and attempt-trace notes.
7. `SigningWorker` computes digests and writes receipt note.
8. Only after that does promotion/merge finalize.

**Failed/blocked attempts:** Archived under `refs/gbllm/archive/<attempt_id>` with the same manifest/review/attempt-trace notes attached. Retro scans main/team refs plus `refs/gbllm/archive/*`.

**Note size policy:** Notes store structured summaries, not raw transcripts. If a payload would exceed the note budget, compact it. Raw traces/logs remain TTL-bound operational data in `state.db` unless explicitly promoted.

**Note schemas** (canonical JSON, one object per note):

`refs/notes/gbllm/manifest` — the full `TaskManifest` (§4.1 schema v3).

`refs/notes/gbllm/review`:
```
{ schema_version, attempt_id, bead_id, attempt_no, annotated_commit,
  proof_commit,
  deterministic: { checks[], requirements_matrix[], coverage, api_diff,
                   benchmark_delta, mutation_score },
  inferential: { reviewers[{ persona, provider, model, verdict, findings[] }],
                 final_verdict, unresolved_mustfix_count,
                 deferred_shouldfix_count },
  review_rounds[] }
```

`refs/notes/gbllm/execution`:
```
{ schema_version, attempt_id, bead_id, attempt_no, annotated_commit,
  outcome: "merged"|"archived_failed"|"archived_blocked",
  timing: { started_at, completed_at, wall_clock_sec },
  cost: { tokens_in, tokens_out, total_tokens, estimated_cost_usd,
          breakdown[{ role, provider, model, tokens_in, tokens_out }] },
  runtime: { adapters, models, auth_mode, provider_versions,
             context_pack_hash, provider_bundle_hash },
  merge: { target_branch, epoch_tag, queue_snapshot_hash, command_ids[] } }
```

`refs/notes/gbllm/attempt-trace`:
```
{ schema_version, attempt_id, bead_id, final_attempt_no,
  attempts[{ attempt_no, outcome, stop_reason, top_error_hashes[],
             failing_sensors[], attempted_hypotheses[], files_touched[],
             tokens_total, duration_sec, failure_taxonomy[] }],
  retro_tags[], recommended_harness_updates[] }
```

`refs/notes/gbllm/receipt`:
```
{ schema_version, bead_id, attempt_id, annotated_commit, stack_head_commit,
  note_digests: { manifest, review, execution, attempt_trace },
  signed_by, signed_at, signature }
```

#### Outcome Labels

Morning review records outcome labels (structured, not just markdown):
- `accepted` — merged and staying
- `reverted` — merged then rolled back
- `escaped_defect` — defect found after merge
- `reviewer_false_positive` — reviewer flagged but human overrode
- `harness_gap` — failure caused by missing guide/sensor

These labels power: reviewer calibration, benchmark comparisons, rollback analysis, and error-budget calculations for autonomy promotion.

---

## 14. Scaling Properties

### What Scales

- **Teams scale horizontally**: New crate → new lieutenant + engineer pool. Linear cost.
- **Engineers per team**: 2 → 5 is free if the merge train handles batching.
- **Top manager stays at 1**: Coordinates lieutenants (bounded), not engineers.
- **Meta-agents scale with epoch size**, not team count.
- **Mechanical refactors prefer deterministic codemods over agent swarms.** For broad mechanical edits (cross-crate renames, derive additions, error type migrations), prefer AST transforms, `cargo fix`, `rust-analyzer` refactors, or scripted codemods. Use agents to _design, parameterize, and verify_ those tools, not to manually simulate them across many worktrees. Agent swarm dispatch is reserved for changes that genuinely require judgment per-crate (e.g., adapting callers to a new API with different semantics).

### What Doesn't Scale

- Over-parallelization within a team degrades quality (CAID paper §4.2). Cap at 5 engineers per team.
- Cross-team merge ordering gets complex with >6 teams. May need a dedicated merge coordinator.
- Review persona cost is O(6 \* num_PRs) — manageable with haiku but watch if PR volume explodes.

### Approximate Agent Count by Project Scale

| Project Scale      | Persistent              | Ephemeral          | Total Concurrent |
| ------------------ | ----------------------- | ------------------ | ---------------- |
| Current (4 crates) | 6 (top + 4 LTs + style) | 10-20 eng + 3 meta | ~25              |
| Medium (8 crates)  | 10                      | 20-40 eng + 5 meta | ~50              |
| Large (16 crates)  | 18                      | 40-80 eng + 8 meta | ~100             |

---

## 15. Merge Flow

### Within a Team: Merge Train

Every merge decision operates on a `QueueSnapshot`:
- `base_sha` / epoch tag
- Ordered candidate list
- Dependency closure
- Manifest hashes
- Proof commit hashes
- Review packet hashes

CI results are valid only for the exact snapshot that produced them. If the base moves, the candidate invalidates and requeues.

Lieutenants validate candidate snapshots as batches, but land approved engineer series by fast-forward or rebase+fast-forward whenever possible so the original code-bearing commits — and their attached git notes — survive unchanged:

```
eng-1 done -+
eng-2 done -+-> synthesize queue snapshot -> candidate branch -> CI -> land snapshot
eng-3 done -+   (or bisect within that snapshot on failure)
```

If CI fails on the batch, binary-search bisect within the snapshot to find the breaker. Reject that branch, reland the rest.

### Team to Main: Epoch Merges

```
main: --- epoch-1 --- merge --- merge --- epoch-2 --- ...
              |                                |
              v                                v
         full workspace CI                full workspace CI
         passes -> tag epoch-1            passes -> tag epoch-2
                                          fails -> freeze, bisect, P0
```

Lieutenants always rebase onto the latest epoch tag, never raw HEAD of main.

### Control-Plane Version Pinning

Each epoch tag records the exact versions of all control-plane artifacts in effect.

**Runtime BOM (Bill of Materials):**

- Harness templates (hash of templates dir)
- Context pack hash (canonical + provider bundles)
- Bundle-equivalence artifact (which cards appear in each provider surface)
- Pi extension versions
- Requested model aliases per role
- Resolved runtime models actually used
- Auth modes per lane
- Effort level / extended-context flags
- Sensor configuration
- Claude Code version + binary digest
- Gemini CLI version + binary digest
- Gemini model pin (currently `gemini-3.1-pro-preview`)
- Pi version / git SHA
- Codex CLI version + binary digest
- Bun version
- rustc/cargo version
- br version
- Hashes of any explicitly imported provider memory/config

**Mid-epoch changes to harness, prompts, rules, model resolution semantics, or auth mode are not allowed** except for emergency safety fixes. If you change the harness AND the model AND the org topology at the same time, you lose causal clarity about what improved or regressed. Pin per epoch, compare across epochs.

**Hot-reload is a daytime/debugging capability only.** Night shifts run immutable control-plane builds per epoch except for breakglass patches.

### Merge Priority

Based on cross-team dependency flow (who produces for whom):

```
1. Model (biggest producer — 7+ downstream cross-team edges)
2. Contracts (unblocks Train and QA)
3. Train (biggest consumer)
4. QA (consumes everything)
```

---

## 16. Information Architecture

Information splits cleanly into two layers: the **git layer** holds durable artifacts, the **runtime layer** holds live state.

### 16.1 Git Layer (Durable Artifacts)

```
/AGENTS.md                    # Golden path, conventions, per-crate sections
/CODEOWNERS                   # Machine-readable team ownership
/INCIDENTS.md                 # Past failures with root causes
/DEPRECATED.md                # Deprecated patterns with migration paths
/CHANGELOG.md                 # Human-readable per-bead entries
/docs/decisions/              # ADRs (why decisions were made)
/docs/rfcs/                   # Cross-team change proposals
/.beads/                      # Issue tracker database
/gbf-model/                   # Team Model
/gbf-artifact/                # Team Contracts
/gbf-policy/                  # Team Contracts
/gbf-train/                   # Team Train
/tests/                       # Team QA
```

Git tags: `epoch-N` mark known-good workspace states.
Git branches: `main`, `team/<team>`, `team/<team>/eng-<id>/<bead>` for engineer attempt repos.
Git note refs:
- `refs/notes/gbllm/manifest` — exact TaskManifest for each landed attempt
- `refs/notes/gbllm/review` — deterministic review packet + inferential verdicts
- `refs/notes/gbllm/execution` — tokens, duration, models, adapters, queue snapshot
- `refs/notes/gbllm/attempt-trace` — compact summary of failed iterations/checkpoints
- `refs/notes/gbllm/receipt` — signed note-digest receipt
Git archive refs: `refs/gbllm/archive/<attempt_id>` — tips of failed/blocked attempts with notes attached.

### 16.2 Runtime Layer (BEAM State + Event Log)

```
GenServer state (reconstructable caches over authoritative DB projections):
  TopManagerGenServer     # current epoch, merge queue, team statuses
  LieutenantGenServer     # active engineers, bead assignments, ACP session
  EngineerWorker          # manifest, phase, messages, context used
  TraceAggregator         # rolling metrics

Event log (append-only, on disk):
  /var/lib/gbllm/state.db     # SQLite WAL: events, commands, checkpoints,
                              # review packets, session metadata, projections

Dashboard (live HTTP):
  http://localhost:4000       # real-time view of runtime state
```

### 16.3 What Lives Where

| Information                         | Layer   | Location                                                |
| ----------------------------------- | ------- | ------------------------------------------------------- |
| How to create a module              | Git     | AGENTS.md §module-creation                              |
| Why we chose Burn                   | Git     | docs/decisions/adr-001.md                               |
| What broke last time                | Git     | INCIDENTS.md                                            |
| What can import what                | Git     | Structural test in CI (code)                            |
| What files a team owns              | Git     | CODEOWNERS                                              |
| What the domain requires            | Git     | Bead descriptions + planv0.md                           |
| What changed last night             | Git     | CHANGELOG.md on main (committed by top manager)         |
| What an engineer is doing RIGHT NOW | Runtime | EngineerWorker GenServer state                          |
| What a team is doing RIGHT NOW      | Runtime | LieutenantGenServer state + TeamSupervisor children     |
| Current review feedback             | Runtime | Injected into EngineerWorker via message, not committed |
| Every API call ever made            | Runtime | Event log (append-only)                                 |
| Live metrics                        | Runtime | TraceAggregator rolling aggregates → Dashboard          |

**Rule of thumb:** if the information outlives the night shift, it's in git. If it's operational state that matters only while agents are running, it's in the BEAM runtime and the event log.

---

## 17. Observability

Observability is built into the runtime (§6.6). The TraceAggregator GenServer collects every API call, tool use, state transition, and hook execution from every agent in the org.

### Key Metrics (derived from traces)

| Metric                        | Source                                 | Alert Threshold | Action                           |
| ----------------------------- | -------------------------------------- | --------------- | -------------------------------- |
| Agent context utilization     | ApiCall tokens_in / budget             | >80%            | Harness forces checkpoint        |
| Precommit P50 duration        | PrecommitHook.duration_ms distribution | >20s            | CI Health agent triggers         |
| Precommit P95 duration        | PrecommitHook.duration_ms distribution | >60s            | P0 bead created                  |
| Engineer stuck rate           | Checkpoint StateTransition count       | >15% of tasks   | Retro agent priority             |
| Merge conflict rate           | Git merge errors in workers            | >5% of merges   | Review CODEOWNERS/manifests      |
| Review persona rejection rate | Review worker outcomes                 | >40% first-pass | Tighten guides                   |
| Beads completed per night     | Done StateTransition count             | Trending down   | Investigate harness regression   |
| Epoch CI failure rate         | EpochCI worker outcomes                | >10%            | Tighten self-verify sensors      |
| Tokens per bead (by team)     | Sum ApiCall tokens grouped by bead     | Trending up     | Prune context, improve manifests |
| Tool call success rate        | ToolResult.success                     | <90%            | Harness validation broken        |
| Latency P50/P95 per model     | ApiCall.latency_ms                     | Regression >20% | API/network issue                |

### Live Introspection

Because the system runs on BEAM, anything currently happening is inspectable:

```
Observer tools (BEAM built-in):
  :observer.start()
    - See every process, its memory, its message queue depth
    - Drill into any GenServer's state
    - Watch messages flow between processes in real-time

Custom DashboardServer:
  http://localhost:4000/dashboard
    - Live view of every active agent
    - Current phase, current bead, tokens used
    - Tool call stream per agent
    - Team status, merge queue depth
    - Current epoch, blocked beads
    - Cost so far this session (tokens × model price)
```

### Traces Power the Meta-Agents

- **Retro agent** queries: "show me all ApiCall traces where the next message was a loop_detected transition — what error hashes recurred?"
- **CI Health agent** queries: "PrecommitHook durations grouped by hook name, P50/P95 for the last epoch — which are slowest and which never fail?"
- **Entropy GC agent** queries: "ToolResult traces for file edits — which files have had the most churn across engineers?"

Meta-agents read from the same event log as the dashboard. No separate instrumentation needed.

### Persistence

The event log is append-only. Writes are local disk by default, with optional OTLP export to an external observability backend (Honeycomb, Grafana Tempo, etc.) for long-term retention and cross-session analysis.

State snapshots of key GenServers are persisted every N minutes so a BEAM node crash doesn't lose the night's work. On restart, the supervision tree reboots from the last snapshot + replays events forward.

---

## 18. Practices That Do NOT Apply

Human org practices that waste compute if applied to AI agents:

- **Standup meetings, retros as ceremonies** — replaced by retro agent analyzing transcripts
- **Career ladders, mentorship** — agents don't grow
- **On-call rotations** — agents don't sleep
- **Consensus-building** — the dependency graph IS the plan
- **1:1 meetings** — agents have no social dynamics
- **Sprint planning ceremonies** — `br ready` is the sprint backlog

The equivalent functions (prioritization, quality improvement, knowledge sharing) are handled by: the dependency graph, the retro agent, and repo-first documentation.

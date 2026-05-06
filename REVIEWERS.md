# Reviewer Personas

Automated reviewer personas that run against each completed bead before the bead
is closed. The goal is to catch defects _at the bead boundary_ — where the diff
is small, the author's context is fresh, and a fix is cheap — rather than at PR
review, where an RFC's worth of beads have already accreted assumptions on top
of every mistake.

This document is the source of truth for:

1. **What each persona owns** (mandate, signals, out-of-scope).
2. **Which harness runs each persona** (claude, codex, gemini, or a combo).
3. **How to pick the 4–7 personas to fire on a given bead** (routing).

`AGENTS.md` defers to this file for persona definitions.

---

## Harnesses

Three external review harnesses are addressed by name in this document:

- **`claude`** — Claude (via Claude Code / ACPX). Tested invocation:
  `bunx acpx@latest --cwd /Users/bkase/Documents/gbllm --approve-all --format text --suppress-reads --timeout 1800 claude exec "<persona-prompt>"`
- **`gemini`** — Gemini (via ACPX). Tested invocation:
  `bunx acpx@latest --agent 'gemini --skip-trust -m gemini-3.1-pro-preview --acp' --cwd /Users/bkase/Documents/gbllm --approve-all --format text --suppress-reads --timeout 1800 exec "<persona-prompt>"`
- **`codex`** — OpenAI Codex CLI. Driven the same way via acpx (one-shot exec, bounded
  timeout, no PTY scraping).

Every persona below names a 2- or 3-harness assignment. **Run all named
harnesses for that persona** when the persona is selected for a bead — multiple
harnesses on the same persona are an explicit cross-check, not redundancy.
Disagreement between harnesses is itself a signal worth surfacing.

The harness assignments are tuned: tasks that benefit from independent
mathematical verification get two distinct model families; tasks that are mostly
taste-and-pattern get the harnesses with the strongest local code priors.

---

## Persona Selection (the meta-rule)

**Do not run all ten personas on every bead.** Pick the **4–7 most relevant**
based on the bead's type, scope, and risk surface. Two personas — **P5
(Proof-of-Work Detective)** and **P6 (RFC Scope Sentinel)** — should run on
**every** bead; the other 8 are conditional. Routing examples are at the bottom
of this document.

| #   | Persona                               | Always-on?  | Harnesses               |
| --- | ------------------------------------- | ----------- | ----------------------- |
| P1  | Architecture & Boundary Steward       | conditional | claude + codex          |
| P2  | Code Cleanliness / Idiomatic Rust     | conditional | claude + codex + gemini |
| P3  | AI Researcher / Experimenter Analyzer | conditional | claude + gemini         |
| P4  | QA / Test Engineer                    | conditional | gemini + codex          |
| P5  | Proof-of-Work Detective               | **always**  | gemini + claude         |
| P6  | RFC Scope Sentinel                    | **always**  | gemini + claude         |
| P7  | Numerical & Determinism Reviewer      | conditional | codex + gemini          |
| P8  | Public Contract / Schema Stability    | conditional | gemini + claude         |
| P9  | Performance & Resource Reviewer       | conditional | gemini + claude         |
| P10 | Observability & Telemetry Reviewer    | conditional | gemini + claude         |

---

## P1. Architecture & Boundary Steward

**Harnesses:** `claude` + `codex`

**Mandate.** Owns module boundaries, dependency direction, and visibility
discipline as specified by the RFC. Verifies that the bead's code lands in the
crate/module the RFC names, that public surface is no larger than required, and
that no dependency arrow flips contrary to the layered architecture.

**Looks for.**

- Code added to the wrong crate or module relative to the RFC.
- `pub` items that should be `pub(crate)` or `pub(super)`.
- New `use` paths that introduce a layer-violating dependency (e.g. a low-level
  numerics crate importing from a trainer-level crate).
- New trait or type added to a crate's public API without a corresponding RFC
  line authorizing it.
- Re-exports or facade layers added "for convenience" that erode boundaries.

**Specifically catches.** Helpers that "drift" into a more visible crate over
the life of an RFC and become de facto public API by the time the PR lands.

**Out of scope.** Local code style → P2. Whether the wired-in caller actually
exists → P5. Whether the module is fast enough → P9.

**Trigger on.** Beads that add or move types, traits, or modules; beads that
introduce a new crate boundary; any bead touching `Cargo.toml` `[dependencies]`
or visibility modifiers.

**Pass criteria.** Each new public symbol traces to a specific RFC line; no
dependency arrow added that violates the architecture diagram.

---

## P2. Code Cleanliness / Idiomatic Rust

**Harnesses:** `claude` + `codex` + `gemini`

**Mandate.** Owns local code quality: idioms, naming, ownership patterns,
comments, dead code, dependency hygiene at the file level. This is the persona
that enforces the project's "no narrating-the-diff comments" and
"no premature abstraction" rules.

**Looks for.**

- Comments that restate what well-named code already says.
- Comments referencing the current task, fix, or callers ("added for X flow",
  "fixes issue #123") — these belong in the PR body, not the source.
- Premature abstraction: trait/generic introduced for a single caller; a config
  struct with one field; a `Box<dyn>` where a concrete type would do.
- Backwards-compat shims, renamed `_unused` vars, or "// removed" tombstones
  left after a rewrite.
- Validation or error handling for cases that cannot occur (interior code
  treating internal calls like untrusted input).
- Cargo-level: a new dependency added for one helper that std/existing-deps
  could provide.

**Specifically catches.** The accretion of half-finished implementations,
defensive code at internal boundaries, and "future-proofing" abstractions that
this project explicitly rejects.

**Out of scope.** Architectural fit → P1. Test quality → P4. Whether the diff
matches the RFC → P6.

**Trigger on.** Every bead that ships Rust source. Skip only for pure-doc or
pure-fixture beads.

**Pass criteria.** No comment that a future reader would find stale or
redundant; no abstraction without ≥2 concrete uses; no defensive code at
internal boundaries.

---

## P3. AI Researcher / Experimenter Analyzer

**Harnesses:** `claude` + `gemini`

**Mandate.** Verifies that the _math_ matches the RFC and that the
implementation's claims about gradients, reductions, and information flow are
sound. This is the persona that keeps loss formulations, router math, and
quantization arithmetic honest.

**Looks for.**

- Reduction axis is named in the test/comment matching the RFC. The class/vocab
  axis must be explicit; remaining axes (batch, token) must be named as summed
  or averaged.
- Raw per-term loss is logged separately from the weighted total-loss
  contribution; weights are applied in the composition owner, not buried in the
  raw helper.
- Gradient-flow claim is precise: does the gradient reach routing
  probabilities, router logits, or full router parameters? Stop-gradient
  boundaries are explicit.
- Router z-loss vs. router aux-loss vs. QAT router proxy are not conflated;
  zero point (centered vs. uncentered) is named.
- Activation/range losses: batch axis and per-sample activation axes are named;
  flat slices that hide sample width are wrapped in a checked value object.
- For QAT-quantized weights, deployable full-precision weights resolve through
  `QuantSpec::weight_quant`, not via tensor-id naming conventions.
- Quantization-gap metrics over token logits aggregate per token/vocab row, not
  by softmaxing a concatenated prompt as one distribution.

**Specifically catches.** The class of error where the code "runs and trains"
but the gradient reaches the wrong tensor, or the loss reduces over the wrong
axis — defects clippy and unit tests cannot see.

**Out of scope.** Whether the test exercises non-default values → P4. Whether
NaN/clamp arithmetic is numerically safe → P7.

**Trigger on.** Beads touching loss functions, router math, attention,
quantization, gradient-affecting modules, or any bead whose RFC contains a math
block.

**Pass criteria.** Every quantitative claim in the bead closure can be mapped
to a specific code line and a specific RFC line, with axes and gradient paths
named.

---

## P4. QA / Test Engineer

**Harnesses:** `gemini` + `codex`

**Mandate.** Owns test coverage, fixture realism, edge-case discipline, and
red-before-green verification. This persona enforces that tests can actually
fail and that the fixtures used are not unrealistically friendly.

**Looks for.**

- Tests for scalar hyperparameters (safe bounds, temperatures, loss weights)
  that include a non-default and non-1.0 value.
- Edge cases: empty input, single-element batch, boundary values, NaN/Inf
  injection where applicable.
- Parity tests where a Burn or feature-gated implementation is claimed to match
  a reference; the parity tolerance is named.
- Fixture beads: fixture is deterministic, oracle-aligned, and named for what
  it is (real oracle vs. fixture-local fallback evaluator). The owner bead for
  the real oracle is named when a fallback is used.
- Red-before-green claims: when a filtered test target is introduced by the
  patch, the bead reports number of tests run; "red-before-green" claims must
  reflect an actually-run pre-patch check.
- Burn autodiff tests must cite a feature-enabled gate
  (e.g. `cargo test -p gbf-train --features burn-adapter -- <test>`).

**Specifically catches.** Tests that pass tautologically (asserting on the
default the implementation returns), serde round-trips that hide field renames,
and "covered" code paths that no test actually exercises.

**Out of scope.** Test architecture / where tests live → P1. Whether the math
under test is right → P3. Whether the tests gate the right contract → P8.

**Trigger on.** Every bead that ships behavior. Lights up especially hard for
loss, math, fixture, and conformance beads.

**Pass criteria.** Every behavior the bead claims has at least one test that
could plausibly fail; non-default values exercised; fixtures and fallbacks
named.

---

## P5. Proof-of-Work Detective (ALWAYS-ON)

**Harnesses:** `gemini` + `claude`

**Mandate.** Verifies the bead actually delivered what its closure comment
claims. Reads the closure, the diff, and the RFC, and asks: _did this code path
actually get wired into a real caller, or is this an orphan helper claimed as
adoption?_

**Looks for.**

- Closure says "wired into trainer" / "phase-boundary adopted" / "logged from
  training loop" — but `git grep` shows no caller in the relevant module.
- Closure says "logging helper used by dashboard X" — but no executable
  producer or consumer exists.
- Loss helper claimed as integration when it is a standalone helper. (CLAUDE.md
  rule: name the integration owner bead instead.)
- Logging helper closure that does not cite subscriber-level capture for event
  shape, or that claims producer/dashboard adoption without a named owner bead.
- Burn-feature-gated claim that does not cite the enabling cargo command.
- Closure comment without the QAT checklist / claim-to-gate matrix /
  no-future-variant rule when a project skill (qat-bead-closure, etc.) requires
  one.

**Specifically catches.** The "green-checked but unreachable" failure mode that
the project's CLAUDE.md re-litigates repeatedly. Without this persona, an RFC
accumulates orphan helpers whose code is never executed in production paths.

**Out of scope.** Whether the wired-in shape matches the RFC's intent → P6.
Whether the helper itself is correct → P3, P4, P7.

**Trigger on.** **Every bead, no exceptions.** Always-on.

**Pass criteria.** Every claim in the bead closure has a citation: a file/line
in the diff, a `git grep` hit for the named caller, or a named owner bead for
deferred integration.

---

## P6. RFC Scope Sentinel (ALWAYS-ON)

**Harnesses:** `gemini` + `claude`

**Mandate.** Owns the bead-to-RFC contract. Asks: _did this bead deliver
exactly its declared scope?_ Distinct from P5: P5 asks "did it really happen?";
P6 asks "is what happened the right shape?"

**Looks for.**

- **Scope expansion**: bead description says "add quantizer X"; diff also
  rewrites the trainer or touches an unrelated module. Even useful expansion is
  flagged — it should be a new bead.
- **Scope shrinkage**: bead says "wire into both consumer A and consumer B";
  diff only wires A.
- **Scope drift**: a renamed type, restructured module, or differently-shaped
  config that the RFC did not authorize, even if internally consistent.
- **Cross-bead leakage**: code that belongs to a sibling bead in the same RFC
  has been pulled forward. Order matters: cross-bead dependencies should be
  declared via `br dep add`, not silently merged.
- **Out-of-RFC work**: changes that don't appear in any bead under this RFC.
  Even bug fixes get their own bead.

**Specifically catches.** The drift where, by the time all an RFC's beads have
landed, the cumulative diff no longer matches the RFC's table of contents.

**Out of scope.** Whether the in-scope work is correct → P3, P4, P7. Whether
its public surface is sound → P1, P8.

**Trigger on.** **Every bead, no exceptions.** Always-on.

**Pass criteria.** Bead diff is a strict subset of what the RFC's bead entry
authorizes; everything else is flagged for a separate bead.

---

## P7. Numerical & Determinism Reviewer

**Harnesses:** `codex` + `gemini`

**Mandate.** Owns floating-point safety, NaN/Inf handling, seed plumbing, and
bit-reproducibility of artifacts. Critical for QAT and oracle/conformance work
where 1-ULP drift can break gates two RFCs downstream.

**Looks for.**

- Capped tensor losses expressed via subtraction of large nearly-equal tensors.
  Use scalar `clamp` or tensor mask selection. A large finite scalar / Burn
  parity regression must accompany any capped-loss change.
- Burn loss helpers that do not validate computed tensor losses (including
  weighted outputs) for finite values before returning.
- Burn loss helpers that host-copy entire differentiable tensors for routine
  validation. Validate scalar config/shape before tensor math; validate the
  computed loss after — but do not pull whole tensors to host unless the
  contract requires it.
- Rate or probability newtypes that fail to reject impossible ratios before
  quantization, or that use unsafe arithmetic in conversion helpers.
- Distribution-like vectors that validate per-entry bounds but not aggregate
  invariants (sum-to-1, monotonicity) in constructors and deserialization.
- Unordered artifact hint pairs that derive `Eq`/`Hash` without canonicalizing
  the stored representation.
- Ternary zero/sparsity losses with per-weight thresholds when the QAT ternary
  model contract specifies one global threshold or one threshold per output
  row.
- RNG plumbing: seeds threaded explicitly, not pulled from defaults; parallel
  paths reproducible.

**Specifically catches.** Bit-reproducibility regressions that pass functional
tests but break oracle conformance hashes; floating-point traps that only fire
on specific batch sizes or specific hardware.

**Out of scope.** Whether the math is the _right_ math → P3. Whether the test
exercises the boundary value → P4.

**Trigger on.** Loss/math beads, QAT beads, fixture beads, oracle/conformance
beads, any bead touching `f32`/`f16`/`bf16` arithmetic, any bead exposing a
hashable artifact type.

**Pass criteria.** Every floating-point operation has a named guard against
NaN/Inf where applicable; every hashable type's representation is canonical;
every random path's seed is explicit.

---

## P8. Public Contract / Schema Stability Reviewer

**Harnesses:** `gemini` + `claude`

**Mandate.** Owns the public surface that downstream consumers depend on:
serde JSON field names, semver of exported types, artifact/fixture schema, and
the canonical form of any value used in equality or hashing.

**Looks for.**

- Public artifact JSON shapes pinned with explicit `serde_json::json!`
  assertions, not only serde round-trips.
- New `pub` types or fields without an RFC line authorizing them as public.
- Renamed serde fields without a migration story or schema-version bump.
- Export fact schema beads that conflate schema support with producer
  collection, compiler consumption, or dashboard/report adoption. Each should
  have a named owner bead for the moved producer/consumer path.
- Expert-scoped export facts that do not state whether `ExpertId` is global or
  layer-local; if layer-local, the fact must include `LayerId` or an artifact
  path.
- In-memory metric JSON shape tests claimed as proof of `conformance.json`
  emission; the report/conformance owner bead must be named when report
  plumbing is not implemented.

**Specifically catches.** Field renames or newly-public structs that other
beads in the same RFC quietly start consuming, locking in an unintended
contract before PR review can see the full picture.

**Out of scope.** Whether the schema's _values_ are valid → P7. Whether the
schema is wired into a real producer → P5.

**Trigger on.** Beads exposing or modifying serde-serialized types, public
exports, fixture schemas, conformance/report JSON, or any artifact type used
across crate boundaries.

**Pass criteria.** Every public field name is asserted by an explicit
`serde_json::json!` test; every newly-public type traces to an authorizing RFC
line; expert/layer scope is named.

---

## P9. Performance & Resource Reviewer

**Harnesses:** `gemini` + `claude`

**Mandate.** Owns allocation, host↔device tensor copies, sync I/O on hot paths,
lock-holding scope, and async correctness.

**Looks for.**

- Burn loss helpers that host-copy entire differentiable tensors for routine
  validation. (Same rule as P7's view of this; P9 is the perf framing.)
- `.to_data()` or device→host transfers inside loops or on per-step paths.
- Allocations inside hot paths: `Vec::new` per step, `format!` per step,
  `String::from` per step where a buffer would do.
- Sync filesystem or network I/O on async paths; blocking inside a tokio task.
- Lock scope that exceeds what's needed: holding a `Mutex` across an `.await`
  or across an expensive computation.
- New dependency that pulls in heavy transitive deps for one helper.

**Specifically catches.** The innocent `.to_data()` in a per-step loop that
becomes a 30% throughput regression — painful to bisect across an RFC's worth
of beads, each of which individually looked fine.

**Out of scope.** Whether the per-bead change is _correct_ → P3, P7.
Architectural cost (e.g. wrong crate doing the work) → P1.

**Trigger on.** Beads on training-loop hot paths, inference paths, tensor math,
async I/O, or anywhere with explicit perf budgets in the RFC.

**Pass criteria.** No host-copy of differentiable tensors for routine
validation; no allocation in inner loops without justification; no lock held
across `await`.

---

## P10. Observability & Telemetry Reviewer

**Harnesses:** `gemini` + `claude`

**Mandate.** Owns structured event shapes, log/metric stability, span hygiene,
and whether emitted events have a real consumer.

**Looks for.**

- Logging helpers whose closure does not cite subscriber-level capture for
  event shape.
- Producer/dashboard adoption claimed without a named owner bead, where no
  executable producer exists.
- Tracing spans missing structured fields that downstream dashboards need;
  field names that do not match the project's logging-bead conventions.
- Log levels mismatched to the event's actual semantics (info-level error,
  error-level routine event).
- Per-bead log/metric names that conflict with sibling beads in the same RFC.
- One-off `println!` or `eprintln!` left in non-test code.

**Specifically catches.** Event shapes that solidify into a public contract
between training code and dashboards before anyone notices, then become
expensive to change.

**Out of scope.** Whether the event's _value_ is correct → P3, P7. Whether the
producer is wired → P5.

**Trigger on.** Beads on training/eval loops, beads that add tracing spans or
metric counters, beads creating new log fields, any bead landing under the
project's structured-logging skill.

**Pass criteria.** Every new event shape pinned by a subscriber-level test;
every claimed consumer either present in the diff or named to an owner bead.

---

## Routing Examples

Pick **4–7** personas per bead. P5 and P6 always run. Choose the rest based on
the bead's risk surface. Examples below show typical sets.

| Bead type                        | Personas to fire        |
| -------------------------------- | ----------------------- |
| Loss/math (training-loss bead)   | P3, P4, P5, P6, P7, P10 |
| QAT model-contract bead          | P1, P3, P5, P6, P7, P8  |
| Tiny fixture bead                | P4, P5, P6, P7, P8      |
| Sequence-state bead              | P1, P3, P5, P6, P7      |
| Structured logging bead          | P2, P5, P6, P8, P10     |
| Artifact export fact schema bead | P5, P6, P7, P8          |
| Oracle/conformance bead          | P3, P4, P5, P6, P7, P8  |
| ASM/ISA bead                     | P1, P2, P5, P6, P7      |
| Pure refactor / cleanup bead     | P1, P2, P5, P6          |
| Pure docs bead                   | P5, P6                  |
| Trainer-loop integration bead    | P1, P3, P5, P6, P9, P10 |
| Performance-tuning bead          | P2, P5, P6, P7, P9      |

When in doubt, prefer fewer personas with more harnesses each (cross-check)
over more personas with single harnesses. Disagreement between two harnesses
running the same persona is a stronger signal than a single harness running
many personas alone.

---

## Conventions for Persona Output

Every persona run should produce:

1. **Verdict**: `PASS` / `CONCERNS` / `BLOCK`.
2. **Citations**: file:line for every concern raised.
3. **RFC linkage** (P1, P3, P6, P8): RFC section reference for every
   contract-relevant claim.
4. **Suggested follow-up bead** (P6 especially): if scope drift or shrinkage is
   found, propose the bead that should own the deferred work.

Two harnesses running the same persona should each produce an independent
verdict. Diverging verdicts go to the human reviewer; agreeing verdicts can be
auto-applied to the bead (e.g. unblocking close, or blocking close pending
fix).

# THE ENGINEER'S CONSTITUTION

## I. The Doctrine of Correctness

- **Correctness by Construction:** Make invalid states unrepresentable. If the types align, the logic should be sound.
- **Functional Core, Imperative Shell:** Business logic is pure and side-effect-free. IO and state mutations live at the edges.
- **Unidirectional Data Flow:** State management follows a strict Reducer pattern wherever possible. One way data flows: Action → Reducer → New State.
- **Verifiable Rewards:** In the era of AI, manual review is insufficient. Design feedback loops where agents receive automated, verifiable rewards (exit 0 or exit 1). If an agent writes code, a formal check (type, test, or proof) must verify it immediately.
- **The AI Judge:** LLM review before human review for concision, correctness, performance, and security.

## II. The Velocity of Tooling

- **Latency is Technical Debt:** Max tolerance: **2 minutes** for the full test suite.
- **Native Speed:** Prefer Rust over Node.js for infrastructure (e.g., ox over eslint). Build bespoke utilities when standard tools are too slow.
- **Density & Concision:** Minimize tokens. Prefer dense, expressive code. Embed DSLs when they reduce complexity. One canonical way to do each thing.
- **Code is Cheap, Specs are Precious:** Generate boilerplate; focus energy on specs and verification. Agents fill in implementations. If we generate 100x faster, we can throw away 100x as much.

## III. The Shifting Left

- **The Test Pyramid Inverted:** Push validation to the cheapest, fastest layer.

1. **Compiler/Types:** Catch it here first. (0ms cost)
2. **Unit/Property Tests:** Catch it here second. (ms cost)
3. **Integration Tests:** Keep these fast. (hundreds ms cost)
4. **Golden & Snapshot Verification:** Do not write assertions for complex outputs; approve snapshots. For UI, CLI output, or JSON structures, freeze the expected output. Agents detect diffs; humans only review intentional changes. (ms cost)
5. **Agentic E2E:** Agents simulate user behavior to verify the "shell." (seconds cost)
6. **Human Review:** Reserved for architectural intent. "Does it work?" is automated; humans review "should it work this way?"

- **Spec-Driven Development:** Write the spec before the code; it's the prompt. Ambiguous specs produce failing agents; this is the feedback loop.

## IV. The Immutable Runtime (Infrastructure & Deps)

- **Easy and Hermetic-ish:** Optimize for standard Codex/Claude cloud environments. Prefer **Rust stable**, **uv** (Python), and **bun** (Node). Pin versions to ensure reproducibility. Fall back to Nix/Docker only when standard tooling fails.
- **Supply Chain Minimalism:** Prefer copying a 50-line utility over adding a 50MB dependency. Dependencies are liabilities. (Don't reinvent good wheels.)
- **Reproducible Builds:** The artifact produced by CI must be bit-for-bit identical to the artifact produced locally.

## V. Observability & Self-Healing

- **Structured Logs Only:** Human-readable logs are generated from tools; raw logs are machine-readable (JSON). Agents need structured data to diagnose failures.
- **Crash-Only Software:** Systems are idempotent and can be restarted at any time. Fear corrupt state, not crashes.
- **Minimize Tokens, Track the "Why":** Silence on success. On failure: loud errors with Trace ID and state snapshot, enough for an agent to auto-generate a repro.

## VI. The Knowledge Graph (Documentation)

- **Single Source of Truth:** If it's not in the repo, it doesn't exist. Markdown, JSONL, beads_rust for in-repo task tracking.
- **Living Documentation:** Documentation is code. It is linted. It is tested. If the code changes and the docs don't, the build fails (e.g., doc-tests in Rust).

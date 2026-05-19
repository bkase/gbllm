#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")/../../.." rev-parse --show-toplevel)"
cd "$REPO_ROOT"

rg -n 'pub trait StaticBudgetReductionSiteFacts' gbf-codegen/src/budget.rs >/dev/null
rg -n 'pub reduction_site_facts: Vec<ReductionSiteProjection>' gbf-codegen/src/budget.rs >/dev/null
rg -n 'stage2\.static_budget\.reduction_site_facts\.bound' gbf-codegen/src/budget.rs >/dev/null

cargo test -p gbf-codegen static_budget_reduction_site_facts_
cargo test -p gbf-codegen static_budget_validation_failure_preserves_non_empty_reduction_site_facts

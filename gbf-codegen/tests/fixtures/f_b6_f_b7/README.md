F-B6/F-B7 shared fixture corpus.

This corpus gives the F-B6/F-B7 construction and driver beads stable
accept/reject/replay/tampered/golden paths. Reject fixtures pin the closed
diagnostic algebra even when a specific input remains a locator for a later
driver/verifier promotion.

Reject `inputs.json` files with `"fixture_status":"placeholder"` are
non-executable locator records only. They must contain only `stage`, `code`,
and `fixture_status`, and scripts/tests must not treat them as driver-ready
inputs.

`expected_diag.json` is the executable coverage contract: every active fixture
pins `code`, `origin`, `severity`, `stage`, `reserved=false`, and
`producer_evidence`. Locator `inputs.json` records are allowed only when the
diagnostic is exercised by a constructor/verifier harness rather than by a
standalone Stage 4/Stage 5 input payload.

When a downstream bead promotes a reject fixture by removing the placeholder
marker, Stage 4 payloads must deserialize as landed `ObservationPlanInputs`,
Stage 5 payloads must deserialize as landed `RangePlanInputs`, and the file
must already be canonical JSON.

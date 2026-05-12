QuantGraph synthetic fixtures for F-B3.

These are tiny, generated fixtures used by gbf-codegen Stage 1 tests. Passing
fixtures pin QuantGraph/report hashes. Reject fixtures pin the typed diagnostic
class and Hard severity for the current Stage 1 diagnostic surface.

The two routed single-layer fixtures intentionally split router score handling:
`routed_basic_one` exercises a top-1 hard route whose gate contribution is the
constant one path, while `routed_basic_selected_score` exercises the selected
router score path. Keeping both tiny fixtures makes the route-weight semantic
choice visible without importing a real exported model.

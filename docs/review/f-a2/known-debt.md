# Known Debt

- Declared `no_std + alloc` remains deferred until `gbf-foundation` switches away from `std`. Owner: `bd-3f3d`.
- The single-source literal smoke test is ignored until `gbf-test` owns a workspace gate. Owner: `bd-iggu`.
- Existing F-A1 source still contains legacy fixture literals. The F-A2 script skips test-only sections and allowlists intentional layout/fixture paths instead of pretending those are fixed by this PR.

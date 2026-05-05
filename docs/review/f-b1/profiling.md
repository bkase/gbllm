# Profiling

Seed estimate from the local 2026-05-04 review run on aarch64 macOS:

| Command | Observed wall time | Notes |
| --- | ---: | --- |
| `scripts/review/f-b1/smoke.sh` | ~11 s | Fast iteration gate. |
| `cargo clippy --workspace --all-features -- -D warnings` | ~1 s | Incremental, warm target dir. |
| `scripts/review/f-b1/regen.sh` | 196 s | Current row-k-lane/runtime-VBlank-handler packet on warm target dir. |
| `cargo run -p gbf-test --bin f_b1_regen` | 183 s | Current dominant artifact sweep phase. Historical repeated-add baseline was ~390-420 s. |
| `cargo test -p gbf-emu --lib f_b1_l4 -- --ignored --nocapture` | 117 s | Current heavy L4 gate suite, 3 ignored N=128 tests. |
| `scripts/review/f-b1/verify-packet.sh` | ~421 s | Historical repeated-add baseline reached post-regeneration diff in 7 min 1 s. Current verification emits the per-phase TSV below. |

Future runs write machine-readable timing logs under `target/f-b1/`:

```bash
scripts/review/f-b1/regen.sh
cat target/f-b1/regen-profile.latest.tsv

scripts/review/f-b1/verify-packet.sh
cat target/f-b1/verify-packet-profile.latest.tsv
```

The logs are intentionally outside `docs/review/f-b1/` so profiling does not
make the review packet nondeterministic.

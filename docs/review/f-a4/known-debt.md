# F-A4 Known Debt

| Deferred item | Why not F-A4 | Owner | F-A4 guard |
|---|---|---|---|
| Whole-program ISR reachability | Requires call graph and placement pipeline | Epic B Stage 12 (`bd-18d`) | `InterruptSafetyTable`, local ISR rejection |
| F-A5 boot zero-init call | Boot nucleus is separate feature | F-A5 (`bd-2r1`) | `lower_banking_shadow_zero_init` exists and is byte-gated |
| Normal payload far-call helper materialization | Requires Bank0 runtime helper composition | F-A5/F-B13 | Normal/switchable inline lowering is rejected |
| MBC5 rumble SRAM semantics | Rumble profiles are not in shipped `gbf-hw` subset | Future cart-profile bead | `RumbleCartProfileRejected` variant reserved |
| ResumeWindow/Token restoration | Scheduler restoration belongs outside F-A4 | F-A5 scheduler | Production lowering rejects both lifetimes |
| Keep-current proof producer | Scheduler proof is not available in F-A4 | F-A5 scheduler | Production lowering rejects forged keep-current |

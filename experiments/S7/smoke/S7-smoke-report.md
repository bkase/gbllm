S7 SMOKE - fixture=tiny_v1 pass_version=1
  H1 MoE-train .................. Fixture [synthetic score path topology=MoeTiny seed=0]
  H2 Dense-train ................ Fixture [synthetic score path topology=MoeTinyDenseMatched seed=0]
  H3 Parity gate ................ Confirmed [Pass-clean]
  H4 Pareto ..................... Confirmed [MoE-dominates]
  H5 Switch-stats ............... Carried [fixture digest; aggregate producer owner bd-2v9r]
  H6 Guardrail .................. Confirmed [Pass]
  H7 Gradient provenance ........ Moved [diagnostic fields captured; full evidence bd-2v9r]
  H8 Burn ExpertBlockQat grad ... Moved [bd-2v9r]
  H9 Oracle (routed FFN) ........ Moved [bd-2v9r]
  H10 Emulator one-token ........ Separate [integration_s7 H10 schema test; not smoke-measured]
Outcome: Pass-clean -> Decision: ProceedToS8
Scope: production Gutenberg training/report adoption remains with bd-2v9r
Diagnostics: DEBUG roots are fixture-limited to bytes/parity/collapse; Pareto and switch-stats producer diagnostics remain with bd-2v9r

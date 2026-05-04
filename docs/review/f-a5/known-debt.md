# Known Debt

- The M0 scheduler emits the cooperative deadline and UI loop shape. Full inference slice dispatch/resume, continuation save/restore, `KeepCurrentProof` production, and live liveness-counter mutation remain with the compiler/lowering owner that emits safe points into inference code.
- Panic stores and renders the full `FaultCode` word, but full `FaultSnapshot` persistence is F-D1.
- Persistence, trace, and harness modules remain stubs by RFC design and are Epic D owners.

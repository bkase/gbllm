# F-B5 Op Signature Table

This table mirrors the closed `InferOp` signature predicate. It is meant for reviewers to diff against `IIR-SC-18` without re-reading the Stage 3 source.

| InferOpTag | Inputs | Outputs | Effects in/out | Reduction site | Notes |
| --- | --- | --- | --- | --- | --- |
| `Classify` | `NormalizedActivation` | `LogitVector` | None | Required | Final classify logits projection. |
| `CombineResidual` | `PostSequence`: `NormalizedActivation`; `PostFfn` dense: `Activation`, `ExpertCandidate`; `PostFfn` routed: `Activation`, `ExpertOutput` | `Activation` | None | Forbidden | Named residual boundary; no reduction site. |
| `DecodeToken` | `LogitVector` | `DecodedToken` | `Rng{Decode}` iff decode plan requires RNG; otherwise none | Forbidden | Only decode may bind the decode RNG effect. |
| `Embedding` | `InputToken` | `EmbeddingOutput` | None | Forbidden | Single-token input head. |
| `ExpertMatVec` | `FfnGate`/`FfnUp`: `NormalizedActivation`; `FfnDown`: `ExpertIntermediate` | `FfnGate`/`FfnUp`: `ExpertIntermediate`; `FfnDown`: `ExpertCandidate` | None | Required | One node per `(layer, expert, slot)` expert weight. |
| `FfnActivation` | `ExpertIntermediate`; plus second `ExpertIntermediate` for `SwiGLU` | `ExpertIntermediate` | None | Forbidden | Per-expert activation between up/gate and down projections. |
| `Norm` | `Activation` | `NormalizedActivation` | None | Required iff the plan is `TileRmsThenAffineClip`; otherwise forbidden | Reduction-bearing only for tile RMS plans. |
| `RouteTop1` | `RouterScore` | `RouterDecision`, `GateWeight` | None | Forbidden | Top-1 hard route decision and selected gate weight. |
| `RouterMatVec` | `NormalizedActivation` | `RouterScore` | None | Required | Routed-layer router score projection. |
| `SelectExpertTop1` | `RouterDecision`, `GateWeight`, and `ExpertCandidate` for each expert in the layer | `ExpertOutput` | None | Forbidden | Consumes the selected candidate and gate weight. |
| `SequenceRead` | None | `SequenceStateRead` | `SequenceState{slot}` in and out | Forbidden | Reserved shape; not emitted for non-empty sequence semantics in v1. |
| `SequenceStep` | `NormalizedActivation` plus one `SequenceStateRead` per slot in the layer | `SequenceBlockOutput` plus one `SequenceStateNext` per slot in the layer | None | Forbidden | Reserved shape; unsupported sequence semantics reject in v1. |
| `SequenceWrite` | `SequenceStateNext` | None | `SequenceState{slot}` in and out | Forbidden | Reserved shape; pairs with sequence-state edge tokens. |

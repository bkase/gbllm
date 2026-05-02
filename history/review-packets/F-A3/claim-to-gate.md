# F-A3 Claim-To-Gate Matrix

| Claim | Gate |
| --- | --- |
| CURRENT_ABI is non-zero | version::current_constant_set |
| AbiVersion ordering is lexicographic | version::ord_total |
| AbiVersion converts to/from bounded SemVer | version::semver_round_trip |
| BuildIdentityBlock is 152 bytes with pinned offsets | version::build_identity_layout + version::build_identity_offsets |
| BuildIdentityBlock constructors stamp magic and zero reserved bytes | version::build_identity_constructor_sets_magic + version::build_identity_constructor_zeroes_reserved |
| BuildIdentityBlock rejects bad magic/nonzero reserved bytes | version::build_identity_validate_rejects_bad_magic + version::build_identity_validate_rejects_nonzero_reserved |
| BuildIdentityBlock byte parser round-trips | version::build_identity_from_bytes_round_trip |
| CompatibilityEnvelope rejects self-reference and invalid ranges | version::compatibility_envelope_no_self + version::compatibility_envelope_validate |
| Liveness counters saturate and threshold at >= unless disabled by zero | liveness::progress_advance + liveness::idle_frames_saturate + liveness::progress_epoch_saturates + liveness::livelock_threshold_zero_disables + liveness::livelock_threshold_fires_at_eq + property::record_progress_then_idle_frame_property |
| InferenceStateHeader is 32 bytes with pinned offsets and split tail sizing | continuation::header_layout + continuation::split_header_tail_validates_size + property::continuation_header_from_to_bytes_round_trip_property |
| Harness op/result discriminants are covered and reject unknowns | harness::op_kind_complete + harness::op_from_u16_rejects_unknown + harness::result_kind_from_u16_rejects_unknown |
| Harness blocks are 44-byte magic/reserved/signal/seq-checked control blocks | harness::layout + harness::constructor_sets_magic + harness::constructor_stages_signals_clear + harness::validate_rejects_bad_magic + harness::validate_rejects_invalid_signal_value + harness::seq_mismatch_rejected |
| FaultCode maps totally into FaultDomain ranges | fault::all_unique_discriminants + fault::code_to_domain_total + fault::range_partition |
| FaultSnapshot captures registers and liveness in a 36-byte layout | fault::register_snapshot_layout + fault::snapshot_layout + fault::snapshot_domain_matches_code + property::fault_snapshot_from_to_bytes_round_trip_property |
| FaultPolicy default action cannot be BootValidationOnly | fault::policy_default_action_validation + fault::policy_action_for_falls_back_to_default |
| ResourceLeaseKind yield safety table is pinned | interrupt::lease_yield_safety_table |
| ResourceLease active/balanced semantics use Option<SliceId> | interrupt::active_predicate + interrupt::balanced_predicate |
| SemanticCheckpointId parser rejects malformed ids | checkpoint::semantic_id_validation_basic + checkpoint::*rejects* |
| CompactCheckpointId(0) is reserved in schemas | checkpoint::compact_none_sentinel + checkpoint::schema_rejects_compact_zero |
| SemanticCheckpointSchema enforces nonzero schema version, unique compact/semantic ids, and resolves both ways | checkpoint::schema_rejects_zero_schema_version + checkpoint::schema_validates_unique_compact + checkpoint::schema_validates_unique_semantic + checkpoint::schema_resolve_round_trip + property::schema_resolve_round_trip_property |
| TraceEvent is exactly 32 bytes with pinned offsets | trace::event_layout |
| TraceBudget rejects inconsistent zero/nonzero settings | trace::trace_budget_constructor_rejects_inconsistent + trace::trace_budget_constructor_accepts_zero_zero |
| Trace probe/drop enums are exhaustive | trace::probe_level_exhaustive + trace::probe_budget_class_exhaustive + trace::drop_policy_exhaustive |
| gbf-abi forbids local unsafe | src/lib.rs contains #![forbid(unsafe_code)]; grep -R "unsafe" gbf-abi/src only finds that lint |

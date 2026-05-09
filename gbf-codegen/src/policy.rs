//! Stage 0.5 policy resolution.

use std::collections::BTreeMap;

use gbf_foundation::{FieldPath, Hash256};
use gbf_policy::{
    CompileKnobBounds, CompileKnobId, CompileKnobOverrides, CompileKnobPartialBounds,
    CompileKnobPartialValues, CompileKnobPath, CompileKnobProvenanceEntry, CompileKnobValues,
    CompileKnobs, ConstraintOperation, ConstraintProvenance, ConstraintValue, DiagnosticSeverity,
    EffectiveConstraints, EvidenceRef, KnobLockSet, MonotoneKnob, ObservationKnob, PlacementKnob,
    PolicyProvenance, PolicySource, ProbeCollectionLevel, ReductionPlanCeiling,
    ResolvedCompilePolicy, RomKernelDuplicationBias, RomKernelResidencyBias, RomWindowKnob,
    ScheduleKnob, ScheduleResourcePressure, ScheduleSliceCoarsening, ScheduleTileSearch,
    StorageMaterialization, ValidationCode, ValidationDetail, ValidationDiagnostic,
    ValidationOrigin, canonical_default_bounds_fixture,
};
use gbf_report::report_schemas::policy_resolution_v1::{
    ArtifactIdentitySection, CompileKnobsSection, CompileRequestSection, ConstraintEnforcement,
    HintConsumptionSection, IgnoredPreference, PolicyProvenanceSection, PolicyResolutionReportBody,
    PolicyResolutionSuccessSection, PreferenceUse, ResolvedSection,
};
use gbf_report::{
    ReportEnvelope, ReportOutcome, canonicalize as canonicalize_report, compute_self_hash,
};
use gbf_workload::{GoldenVectorId, WorkloadId};
use sha2::{Digest, Sha256};

use crate::validate::{ValidatedInputHashes, ValidationProduct};

pub type CompileKnobPreferences = CompileKnobPartialValues;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintFrame {
    pub source: PolicySource,
    pub evidence: Vec<EvidenceRef>,
    pub defaults: CompileKnobPartialValues,
    pub hard_bounds: CompileKnobPartialBounds,
    pub preferences: CompileKnobPreferences,
    pub locks: KnobLockSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPolicyProduct {
    pub policy: ResolvedCompilePolicy,
    pub input_hashes: ValidatedInputHashes,
    pub artifact_validation_self_hash: Hash256,
    pub report: ReportEnvelope<PolicyResolutionReportBody>,
    pub policy_resolution_self_hash: Hash256,
    pub policy_resolution_canonical_bytes_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyResolutionStageFailure {
    pub report: ReportEnvelope<PolicyResolutionReportBody>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

struct ResolutionState {
    values: CompileKnobValues,
    bounds: CompileKnobBounds,
    locks: KnobLockSet,
    overrides: CompileKnobOverrides,
    provenance: BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
    hint_consumption: HintConsumptionSection,
}

#[allow(clippy::result_large_err)]
pub fn resolve_policy(
    validation: &ValidationProduct<'_>,
) -> Result<ResolvedPolicyProduct, PolicyResolutionStageFailure> {
    let values = conservative_target_values();
    let mut state = ResolutionState {
        provenance: value_seed_provenance(&values, target_evidence(validation)),
        values,
        bounds: canonical_default_bounds_fixture(),
        locks: KnobLockSet::default(),
        overrides: CompileKnobOverrides::default(),
        hint_consumption: HintConsumptionSection::default(),
    };

    let mut frames = match initial_constraint_frames(validation) {
        Ok(frames) => frames,
        Err(diagnostic) => return Err(finalized_failure(validation, diagnostic)),
    };
    frames.push(compile_request_frame(validation));
    let profile_frame_index = profile_frame_index(&frames);

    for (index, frame) in frames.iter().enumerate() {
        if let Err(diagnostic) = apply_frame(
            &mut state,
            frame,
            index > profile_frame_index,
            matches!(frame.source, PolicySource::CompileRequestOverride),
        ) {
            return Err(finalized_failure(validation, diagnostic));
        }
    }

    let calibration_frame_index = frames.len();
    let calibration_frame = calibration_frame(validation, &state.values);
    if let Err(diagnostic) = apply_frame(
        &mut state,
        &calibration_frame,
        calibration_frame_index > profile_frame_index,
        false,
    ) {
        return Err(finalized_failure(validation, diagnostic));
    }

    let hint_consumption = state.hint_consumption.clone();
    let policy = build_policy(validation, state);
    let report = success_report(validation, &policy, hint_consumption);
    let (report, policy_resolution_self_hash, policy_resolution_canonical_bytes_hash) =
        finalize_report(report);

    Ok(ResolvedPolicyProduct {
        policy,
        input_hashes: validation.validated.input_hashes,
        artifact_validation_self_hash: validation.artifact_validation_self_hash,
        report,
        policy_resolution_self_hash,
        policy_resolution_canonical_bytes_hash,
    })
}

#[allow(clippy::result_large_err)]
fn initial_constraint_frames(
    validation: &ValidationProduct<'_>,
) -> Result<Vec<ConstraintFrame>, ValidationDiagnostic> {
    Ok(vec![
        ConstraintFrame {
            source: PolicySource::TargetDefault,
            evidence: vec![target_evidence(validation)],
            defaults: CompileKnobPartialValues::default(),
            hard_bounds: partial_bounds_from_full(canonical_default_bounds_fixture()),
            preferences: CompileKnobPartialValues::default(),
            locks: KnobLockSet::default(),
        },
        ConstraintFrame {
            source: PolicySource::ProfileDefault,
            evidence: vec![profile_evidence(validation)],
            defaults: validation.validated.compile_profile.knob_defaults.clone(),
            hard_bounds: validation.validated.compile_profile.knob_bounds.clone(),
            preferences: CompileKnobPartialValues::default(),
            locks: validation.validated.compile_profile.locks.clone(),
        },
        hint_preference_frame(validation),
        hint_constraint_frame(validation)?,
    ])
}

fn profile_frame_index(frames: &[ConstraintFrame]) -> usize {
    frames
        .iter()
        .position(|frame| matches!(frame.source, PolicySource::ProfileDefault))
        .expect("policy resolution frames include a profile-default frame")
}

fn hint_preference_frame(validation: &ValidationProduct<'_>) -> ConstraintFrame {
    let mut preferences = CompileKnobPartialValues::default();
    if !validation
        .validated
        .artifact
        .hint_bundle
        .preferences
        .expert_slot_affinity()
        .is_empty()
    {
        preferences.placement = Some(PlacementKnob {
            profile: gbf_policy::PlacementProfile::PackedExperts,
        });
    }

    ConstraintFrame {
        source: PolicySource::HintBundle,
        evidence: vec![hint_evidence(validation)],
        defaults: CompileKnobPartialValues::default(),
        hard_bounds: CompileKnobPartialBounds::default(),
        preferences,
        locks: KnobLockSet::default(),
    }
}

#[allow(clippy::result_large_err)]
fn hint_constraint_frame(
    validation: &ValidationProduct<'_>,
) -> Result<ConstraintFrame, ValidationDiagnostic> {
    let mut defaults = CompileKnobPartialValues::default();
    for entry in &validation
        .validated
        .artifact
        .hint_bundle
        .constraints
        .entries
    {
        set_partial_value_from_constraint(&mut defaults, entry.knob, &entry.value).map_err(
            |()| {
                let mut evidence = vec![hint_evidence(validation)];
                evidence.extend(entry.evidence.clone());
                unsupported_hint_constraint_diagnostic(entry.knob, entry.value.clone(), evidence)
            },
        )?;
    }

    Ok(ConstraintFrame {
        source: PolicySource::HintBundle,
        evidence: vec![hint_evidence(validation)],
        defaults,
        hard_bounds: CompileKnobPartialBounds::default(),
        preferences: CompileKnobPartialValues::default(),
        locks: KnobLockSet::default(),
    })
}

fn compile_request_frame(validation: &ValidationProduct<'_>) -> ConstraintFrame {
    let overrides = validation
        .validated
        .compile_request
        .constraint_overrides
        .clone()
        .unwrap_or_default();
    ConstraintFrame {
        source: PolicySource::CompileRequestOverride,
        evidence: vec![compile_request_evidence(validation)],
        defaults: overrides.values,
        hard_bounds: overrides.bounds,
        preferences: CompileKnobPartialValues::default(),
        locks: KnobLockSet::default(),
    }
}

fn calibration_frame(
    validation: &ValidationProduct<'_>,
    current_values: &CompileKnobValues,
) -> ConstraintFrame {
    let mut defaults = CompileKnobPartialValues::default();
    if validation
        .validated
        .calibration
        .bundles
        .values()
        .any(|bundle| bundle.measurements.is_some())
    {
        defaults.schedule = Some(ScheduleKnob {
            resource_pressure: ScheduleResourcePressure::FitFirst,
            ..current_values.schedule
        });
    }

    ConstraintFrame {
        source: PolicySource::Calibration,
        evidence: vec![calibration_evidence(validation)],
        defaults,
        hard_bounds: CompileKnobPartialBounds::default(),
        preferences: CompileKnobPartialValues::default(),
        locks: KnobLockSet::default(),
    }
}

#[allow(clippy::result_large_err)]
fn apply_frame(
    state: &mut ResolutionState,
    frame: &ConstraintFrame,
    locks_active: bool,
    compile_request_override: bool,
) -> Result<(), ValidationDiagnostic> {
    apply_bounds(state, frame, compile_request_override)?;
    apply_values(
        state,
        &frame.defaults,
        frame,
        locks_active,
        operation_for_value_source(&frame.source),
    )?;
    apply_preferences(state, &frame.preferences, frame, locks_active)?;

    for knob in &frame.locks.locked {
        state.locks.locked.insert(*knob);
        push_provenance_path(
            &mut state.provenance,
            lock_path(*knob),
            ConstraintProvenance {
                source: frame.source.clone(),
                operation: ConstraintOperation::SeedDefault,
                evidence: frame.evidence.clone(),
            },
        );
    }

    if matches!(frame.source, PolicySource::HintBundle) {
        for entry in state_hint_constraints(frame) {
            state.hint_consumption.constraints_enforced.push(entry);
        }
    }

    Ok(())
}

#[allow(clippy::result_large_err)]
fn apply_bounds(
    state: &mut ResolutionState,
    frame: &ConstraintFrame,
    compile_request_override: bool,
) -> Result<(), ValidationDiagnostic> {
    macro_rules! apply_one {
        ($field:ident, $knob:expr) => {
            if let Some(next) = frame.hard_bounds.$field {
                if compile_request_override && !next.is_monotone_successor_of(&state.bounds.$field)
                {
                    return Err(loosened_bound_diagnostic(
                        $knob,
                        state.bounds.clone(),
                        with_bound(state.bounds.clone(), $knob, next),
                        frame.evidence.clone(),
                    ));
                }
                let before = state.bounds.$field;
                state.bounds.$field = meet_bound(before, next);
                if compile_request_override {
                    state.overrides.bounds.$field = Some(next);
                    push_bound_override_provenance(
                        &mut state.provenance,
                        $knob,
                        next,
                        ConstraintProvenance {
                            source: frame.source.clone(),
                            operation: ConstraintOperation::ApplyOverride,
                            evidence: frame.evidence.clone(),
                        },
                    );
                }
                if !value_within_knob_bounds($knob, &state.values, &state.bounds) {
                    return Err(out_of_bounds_diagnostic(
                        $knob,
                        value_descriptor($knob, &state.values),
                        state.bounds.clone(),
                        frame.evidence.clone(),
                    ));
                }
                push_bound_provenance(
                    &mut state.provenance,
                    $knob,
                    before,
                    state.bounds.$field,
                    matches!(frame.source, PolicySource::TargetDefault),
                    ConstraintProvenance {
                        source: frame.source.clone(),
                        operation: operation_for_bound_source(&frame.source),
                        evidence: frame.evidence.clone(),
                    },
                );
            }
        };
    }

    apply_one!(placement, CompileKnobId::Placement);
    apply_one!(observation, CompileKnobId::Observation);
    apply_one!(range, CompileKnobId::Range);
    apply_one!(storage, CompileKnobId::Storage);
    apply_one!(sram, CompileKnobId::Sram);
    apply_one!(rom_window, CompileKnobId::RomWindow);
    apply_one!(overlay, CompileKnobId::Overlay);
    apply_one!(schedule, CompileKnobId::Schedule);
    Ok(())
}

#[allow(clippy::result_large_err)]
fn apply_values(
    state: &mut ResolutionState,
    values: &CompileKnobPartialValues,
    frame: &ConstraintFrame,
    locks_active: bool,
    operation: ConstraintOperation,
) -> Result<(), ValidationDiagnostic> {
    macro_rules! apply_one {
        ($field:ident, $knob:expr) => {
            if let Some(next) = values.$field {
                if locks_active
                    && state.locks.locked.contains(&$knob)
                    && state.values.$field != next
                {
                    return Err(locked_diagnostic($knob, frame.evidence.clone()));
                }
                let mut candidate = state.values.clone();
                candidate.$field = next;
                if !value_within_knob_bounds($knob, &candidate, &state.bounds) {
                    return Err(out_of_bounds_diagnostic(
                        $knob,
                        value_descriptor($knob, &candidate),
                        state.bounds.clone(),
                        frame.evidence.clone(),
                    ));
                }
                let before = state.values.$field;
                state.values.$field = next;
                if matches!(frame.source, PolicySource::CompileRequestOverride) {
                    state.overrides.values.$field = Some(next);
                    push_value_override_provenance(
                        &mut state.provenance,
                        $knob,
                        next,
                        ConstraintProvenance {
                            source: frame.source.clone(),
                            operation,
                            evidence: frame.evidence.clone(),
                        },
                    );
                }
                push_value_provenance(
                    &mut state.provenance,
                    $knob,
                    before,
                    next,
                    !matches!(frame.source, PolicySource::Calibration),
                    ConstraintProvenance {
                        source: frame.source.clone(),
                        operation,
                        evidence: frame.evidence.clone(),
                    },
                );
            }
        };
    }

    apply_one!(placement, CompileKnobId::Placement);
    apply_one!(observation, CompileKnobId::Observation);
    apply_one!(range, CompileKnobId::Range);
    apply_one!(storage, CompileKnobId::Storage);
    apply_one!(sram, CompileKnobId::Sram);
    apply_one!(rom_window, CompileKnobId::RomWindow);
    apply_one!(overlay, CompileKnobId::Overlay);
    apply_one!(schedule, CompileKnobId::Schedule);
    Ok(())
}

#[allow(clippy::result_large_err)]
fn apply_preferences(
    state: &mut ResolutionState,
    values: &CompileKnobPartialValues,
    frame: &ConstraintFrame,
    locks_active: bool,
) -> Result<(), ValidationDiagnostic> {
    macro_rules! apply_one {
        ($field:ident, $knob:expr) => {
            if let Some(next) = values.$field {
                let provenance = preference_provenance(frame);
                let mut candidate = state.values.clone();
                candidate.$field = next;
                if !value_within_knob_bounds($knob, &candidate, &state.bounds) {
                    state
                        .hint_consumption
                        .preferences_ignored
                        .push(IgnoredPreference {
                            knob: $knob,
                            reason: "outside_bounds".to_owned(),
                            provenance,
                        });
                } else {
                    if locks_active
                        && state.locks.locked.contains(&$knob)
                        && state.values.$field != next
                    {
                        return Err(locked_diagnostic($knob, frame.evidence.clone()));
                    }

                    let before = state.values.$field;
                    state.values.$field = next;
                    state
                        .hint_consumption
                        .preferences_honored
                        .push(PreferenceUse {
                            knob: $knob,
                            provenance: provenance.clone(),
                        });
                    push_value_provenance(
                        &mut state.provenance,
                        $knob,
                        before,
                        next,
                        true,
                        ConstraintProvenance {
                            source: frame.source.clone(),
                            operation: ConstraintOperation::ApplyPreference,
                            evidence: frame.evidence.clone(),
                        },
                    );
                }
            }
        };
    }

    apply_one!(placement, CompileKnobId::Placement);
    apply_one!(observation, CompileKnobId::Observation);
    apply_one!(range, CompileKnobId::Range);
    apply_one!(storage, CompileKnobId::Storage);
    apply_one!(sram, CompileKnobId::Sram);
    apply_one!(rom_window, CompileKnobId::RomWindow);
    apply_one!(overlay, CompileKnobId::Overlay);
    apply_one!(schedule, CompileKnobId::Schedule);

    Ok(())
}

fn build_policy(
    validation: &ValidationProduct<'_>,
    state: ResolutionState,
) -> ResolvedCompilePolicy {
    let provenance = provenance_entries(state.provenance);
    ResolvedCompilePolicy {
        target: validation.validated.compile_request.target.clone(),
        profile: validation.validated.compile_request.profile.clone(),
        objective: validation.validated.compile_request.objective.clone(),
        effective_constraints: EffectiveConstraints {
            target_caps: canonical_default_bounds_fixture(),
            required_features: validation
                .validated
                .compile_request
                .required_features
                .clone(),
            requested_runtime_modes: validation
                .validated
                .compile_request
                .requested_runtime_modes
                .clone(),
            runtime_chrome_budget: None,
        },
        observability: validation.validated.compile_profile.observability,
        trace_budget: validation.validated.compile_profile.trace_budget,
        requested_runtime_modes: validation
            .validated
            .compile_request
            .requested_runtime_modes
            .clone(),
        knobs: CompileKnobs {
            global: state.values,
            bounds: state.bounds,
            locks: state.locks,
            overrides: state.overrides,
            provenance,
        },
        repair: validation.validated.compile_profile.repair_policy,
        provenance: PolicyProvenance {
            target_defaults: validation.validated.input_hashes.target_profile_hash,
            profile_defaults: validation.validated.compile_profile.defaults_hash,
            hint_bundle_hash: Some(validation.validated.input_hashes.hint_bundle_hash),
            compile_request_hash: validation.validated.input_hashes.compile_request_hash,
            calibration_hash: Some(validation.validated.input_hashes.calibration_hash),
        },
    }
}

fn success_report(
    validation: &ValidationProduct<'_>,
    policy: &ResolvedCompilePolicy,
    hint_consumption: HintConsumptionSection,
) -> ReportEnvelope<PolicyResolutionReportBody> {
    policy_report(
        validation,
        Some(policy),
        hint_consumption,
        Vec::new(),
        ReportOutcome::Passed,
    )
}

fn failure_report(
    validation: &ValidationProduct<'_>,
    diagnostics: Vec<ValidationDiagnostic>,
) -> ReportEnvelope<PolicyResolutionReportBody> {
    policy_report(
        validation,
        None,
        HintConsumptionSection::default(),
        diagnostics,
        ReportOutcome::Failed,
    )
}

fn finalized_failure(
    validation: &ValidationProduct<'_>,
    diagnostic: ValidationDiagnostic,
) -> PolicyResolutionStageFailure {
    let diagnostics = vec![diagnostic];
    let report = failure_report(validation, diagnostics.clone());
    let (report, _, _) = finalize_report(report);
    PolicyResolutionStageFailure {
        report,
        diagnostics,
    }
}

fn policy_report(
    validation: &ValidationProduct<'_>,
    policy: Option<&ResolvedCompilePolicy>,
    hint_consumption: HintConsumptionSection,
    diagnostics: Vec<ValidationDiagnostic>,
    outcome: ReportOutcome,
) -> ReportEnvelope<PolicyResolutionReportBody> {
    let result = policy.map(|policy| PolicyResolutionSuccessSection {
        resolved: ResolvedSection::from(policy),
        compile_knobs: CompileKnobsSection::from(&policy.knobs),
        provenance: PolicyProvenanceSection::from_policy(
            &policy.provenance,
            validation.validated.input_hashes.hint_bundle_hash,
            validation.validated.input_hashes.calibration_hash,
        ),
    });

    ReportEnvelope::new(
        outcome,
        PolicyResolutionReportBody {
            artifact_identity: artifact_identity(validation),
            compile_request: compile_request_section(validation),
            result,
            hint_consumption,
            diagnostics,
        },
    )
    .expect("policy_resolution.v1 schema constants are valid")
}

fn artifact_identity(validation: &ValidationProduct<'_>) -> ArtifactIdentitySection {
    let mut workload_refs = validation
        .validated
        .workloads
        .iter()
        .map(|workload| workload.id.clone())
        .collect::<Vec<WorkloadId>>();
    workload_refs.sort();

    let mut golden_vector_refs = validation
        .validated
        .golden_vectors
        .iter()
        .map(|vector| vector.id.clone())
        .collect::<Vec<GoldenVectorId>>();
    golden_vector_refs.sort();

    ArtifactIdentitySection {
        artifact_core_hash: validation
            .validated
            .input_hashes
            .artifact_effective_core_hash,
        artifact_manifest_hash: validation.validated.input_hashes.artifact_manifest_hash,
        semantic_lineage: validation.validated.artifact.manifest.lineage.clone(),
        lowering_manifest_hash: validation.validated.input_hashes.lowering_manifest_hash,
        hint_bundle_hash: validation.validated.input_hashes.hint_bundle_hash,
        workload_refs,
        golden_vector_refs,
    }
}

fn compile_request_section(validation: &ValidationProduct<'_>) -> CompileRequestSection {
    let request = validation.validated.compile_request;
    CompileRequestSection {
        compile_request_hash: validation.validated.input_hashes.compile_request_hash,
        target: request.target.clone(),
        target_profile_hash: validation.validated.input_hashes.target_profile_hash,
        profile: request.profile.clone(),
        objective: request.objective.clone(),
        required_features: request.required_features.clone(),
        requested_runtime_modes: request.requested_runtime_modes.clone(),
        calibration_set_ref: request.calibration_set_ref.clone(),
        calibration_hash: validation.validated.input_hashes.calibration_hash,
    }
}

fn finalize_report(
    mut report: ReportEnvelope<PolicyResolutionReportBody>,
) -> (ReportEnvelope<PolicyResolutionReportBody>, Hash256, Hash256) {
    report.report_self_hash =
        compute_self_hash(&report).expect("policy resolution report self-hash is computable");
    let canonical_bytes =
        canonicalize_report(&report).expect("policy resolution report canonicalizes");
    let canonical_bytes_hash = Hash256::from_bytes(Sha256::digest(&canonical_bytes).into());
    (
        report.clone(),
        report.report_self_hash,
        canonical_bytes_hash,
    )
}

fn conservative_target_values() -> CompileKnobValues {
    CompileKnobValues {
        placement: PlacementKnob {
            profile: gbf_policy::PlacementProfile::StrictOnePerBank,
        },
        observation: ObservationKnob {
            observability: gbf_policy::ObservabilityMode::Invariant,
            probe_level: ProbeCollectionLevel::RequiredOnly,
        },
        range: gbf_policy::RangeKnob {
            reduction_ceiling: ReductionPlanCeiling::ExactOnly,
        },
        storage: gbf_policy::StorageKnob {
            materialization: StorageMaterialization::PreserveAll,
        },
        sram: gbf_policy::SramKnob {
            page_aggression: gbf_policy::SramPageAggression::Preserve,
        },
        rom_window: RomWindowKnob {
            kernel_residency_bias: RomKernelResidencyBias::PreferCommonBank,
            kernel_duplication_bias: RomKernelDuplicationBias::Share,
        },
        overlay: gbf_policy::OverlayKnob {
            promotion: gbf_policy::OverlayPromotion::Disabled,
        },
        schedule: ScheduleKnob {
            tile_search: ScheduleTileSearch::Fixed,
            slice_coarsening: ScheduleSliceCoarsening::Fine,
            resource_pressure: ScheduleResourcePressure::Conservative,
        },
    }
}

fn partial_bounds_from_full(bounds: CompileKnobBounds) -> CompileKnobPartialBounds {
    CompileKnobPartialBounds {
        placement: Some(bounds.placement),
        observation: Some(bounds.observation),
        range: Some(bounds.range),
        storage: Some(bounds.storage),
        sram: Some(bounds.sram),
        rom_window: Some(bounds.rom_window),
        overlay: Some(bounds.overlay),
        schedule: Some(bounds.schedule),
    }
}

fn set_partial_value_from_constraint(
    values: &mut CompileKnobPartialValues,
    knob: CompileKnobId,
    value: &ConstraintValue,
) -> Result<(), ()> {
    match (knob, value) {
        (CompileKnobId::Placement, ConstraintValue::PlacementProfile { value }) => {
            values.placement = Some(PlacementKnob { profile: *value });
        }
        (CompileKnobId::Observation, ConstraintValue::ObservabilityMode { value }) => {
            let mut observation = values
                .observation
                .unwrap_or_else(|| conservative_target_values().observation);
            observation.observability = *value;
            values.observation = Some(observation);
        }
        _ => return Err(()),
    }
    Ok(())
}

fn value_within_knob_bounds(
    knob: CompileKnobId,
    values: &CompileKnobValues,
    bounds: &CompileKnobBounds,
) -> bool {
    match knob {
        CompileKnobId::Placement => values.placement.profile <= bounds.placement.max_profile,
        CompileKnobId::Observation => {
            values.observation.probe_level <= bounds.observation.max_probe_level
        }
        CompileKnobId::Range => {
            values.range.reduction_ceiling <= bounds.range.max_reduction_ceiling
        }
        CompileKnobId::Storage => {
            values.storage.materialization <= bounds.storage.max_materialization
        }
        CompileKnobId::Sram => values.sram.page_aggression <= bounds.sram.max_page_aggression,
        CompileKnobId::RomWindow => {
            values.rom_window.kernel_residency_bias <= bounds.rom_window.max_kernel_residency_bias
                && values.rom_window.kernel_duplication_bias
                    <= bounds.rom_window.max_kernel_duplication_bias
        }
        CompileKnobId::Overlay => values.overlay.promotion <= bounds.overlay.max_promotion,
        CompileKnobId::Schedule => {
            values.schedule.tile_search <= bounds.schedule.max_tile_search
                && values.schedule.slice_coarsening <= bounds.schedule.max_slice_coarsening
                && values.schedule.resource_pressure <= bounds.schedule.max_resource_pressure
        }
    }
}

fn value_descriptor(
    knob: CompileKnobId,
    values: &CompileKnobValues,
) -> gbf_policy::KnobValueDescriptor {
    let value = match knob {
        CompileKnobId::Placement => ConstraintValue::PlacementProfile {
            value: values.placement.profile,
        },
        CompileKnobId::Observation => ConstraintValue::ObservabilityMode {
            value: values.observation.observability,
        },
        _ => ConstraintValue::Text {
            value: format!("{:?}", values),
        },
    };
    gbf_policy::KnobValueDescriptor { value }
}

fn out_of_bounds_diagnostic(
    knob: CompileKnobId,
    requested: gbf_policy::KnobValueDescriptor,
    bounds: CompileKnobBounds,
    provenance: Vec<EvidenceRef>,
) -> ValidationDiagnostic {
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::PolicyResolution,
        code: ValidationCode::PolicyKnobOutOfBounds {
            knob,
            requested,
            bounds,
        },
        detail: ValidationDetail::Field {
            field: FieldPath::from(format!("compile_knobs.{knob:?}")),
        },
        provenance,
    }
}

fn locked_diagnostic(knob: CompileKnobId, provenance: Vec<EvidenceRef>) -> ValidationDiagnostic {
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::PolicyResolution,
        code: ValidationCode::PolicyKnobLockedAndOverridden { knob },
        detail: ValidationDetail::Field {
            field: FieldPath::from(format!("compile_knobs.{knob:?}")),
        },
        provenance,
    }
}

fn loosened_bound_diagnostic(
    knob: CompileKnobId,
    previous: CompileKnobBounds,
    requested: CompileKnobBounds,
    provenance: Vec<EvidenceRef>,
) -> ValidationDiagnostic {
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::PolicyResolution,
        code: ValidationCode::PolicyConstraintLoosened {
            knob,
            previous,
            requested,
        },
        detail: ValidationDetail::Field {
            field: FieldPath::from(format!("compile_knobs.bounds.{knob:?}")),
        },
        provenance,
    }
}

fn unsupported_hint_constraint_diagnostic(
    knob: CompileKnobId,
    value: ConstraintValue,
    provenance: Vec<EvidenceRef>,
) -> ValidationDiagnostic {
    ValidationDiagnostic {
        severity: DiagnosticSeverity::Hard,
        origin: ValidationOrigin::PolicyResolution,
        code: ValidationCode::PolicyHintConstraintUnsupported { knob, value },
        detail: ValidationDetail::Field {
            field: FieldPath::from(format!("hint_bundle.constraints.{knob:?}")),
        },
        provenance,
    }
}

fn operation_for_value_source(source: &PolicySource) -> ConstraintOperation {
    match source {
        PolicySource::CompileRequestOverride => ConstraintOperation::ApplyOverride,
        PolicySource::HintBundle => ConstraintOperation::ApplyHardConstraint,
        PolicySource::Calibration => ConstraintOperation::ApplyCalibration,
        _ => ConstraintOperation::SeedDefault,
    }
}

fn operation_for_bound_source(source: &PolicySource) -> ConstraintOperation {
    match source {
        PolicySource::TargetDefault => ConstraintOperation::SeedDefault,
        PolicySource::CompileRequestOverride => ConstraintOperation::ApplyOverride,
        PolicySource::HintBundle => ConstraintOperation::ApplyHardConstraint,
        PolicySource::Calibration => ConstraintOperation::ApplyCalibration,
        _ => ConstraintOperation::TightenBound,
    }
}

fn push_provenance_path(
    provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
    path: CompileKnobPath,
    entry: ConstraintProvenance,
) {
    provenance.entry(path).or_default().push(entry);
}

fn value_seed_provenance(
    values: &CompileKnobValues,
    evidence: EvidenceRef,
) -> BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>> {
    let mut provenance = BTreeMap::new();
    let entry = ConstraintProvenance {
        source: PolicySource::TargetDefault,
        operation: ConstraintOperation::SeedDefault,
        evidence: vec![evidence],
    };
    push_value_provenance(
        &mut provenance,
        CompileKnobId::Placement,
        values.placement,
        values.placement,
        true,
        entry.clone(),
    );
    push_value_provenance(
        &mut provenance,
        CompileKnobId::Observation,
        values.observation,
        values.observation,
        true,
        entry.clone(),
    );
    push_value_provenance(
        &mut provenance,
        CompileKnobId::Range,
        values.range,
        values.range,
        true,
        entry.clone(),
    );
    push_value_provenance(
        &mut provenance,
        CompileKnobId::Storage,
        values.storage,
        values.storage,
        true,
        entry.clone(),
    );
    push_value_provenance(
        &mut provenance,
        CompileKnobId::Sram,
        values.sram,
        values.sram,
        true,
        entry.clone(),
    );
    push_value_provenance(
        &mut provenance,
        CompileKnobId::RomWindow,
        values.rom_window,
        values.rom_window,
        true,
        entry.clone(),
    );
    push_value_provenance(
        &mut provenance,
        CompileKnobId::Overlay,
        values.overlay,
        values.overlay,
        true,
        entry.clone(),
    );
    push_value_provenance(
        &mut provenance,
        CompileKnobId::Schedule,
        values.schedule,
        values.schedule,
        true,
        entry,
    );
    provenance
}

trait ValueProvenanceFields: Copy + PartialEq {
    fn push_value_fields(
        provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
        knob: CompileKnobId,
        before: Self,
        after: Self,
        force: bool,
        entry: ConstraintProvenance,
        prefix: &'static str,
    );
}

trait BoundProvenanceFields: Copy + PartialEq {
    fn push_bound_fields(
        provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
        knob: CompileKnobId,
        before: Self,
        after: Self,
        force: bool,
        entry: ConstraintProvenance,
        prefix: &'static str,
    );
}

fn push_value_provenance<T: ValueProvenanceFields>(
    provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
    knob: CompileKnobId,
    before: T,
    after: T,
    force: bool,
    entry: ConstraintProvenance,
) {
    T::push_value_fields(provenance, knob, before, after, force, entry, "global");
}

fn push_bound_provenance<T: BoundProvenanceFields>(
    provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
    knob: CompileKnobId,
    before: T,
    after: T,
    force: bool,
    entry: ConstraintProvenance,
) {
    T::push_bound_fields(provenance, knob, before, after, force, entry, "bounds");
}

fn push_value_override_provenance<T: ValueProvenanceFields>(
    provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
    knob: CompileKnobId,
    value: T,
    entry: ConstraintProvenance,
) {
    T::push_value_fields(
        provenance,
        knob,
        value,
        value,
        true,
        entry,
        "overrides.values",
    );
}

fn push_bound_override_provenance<T: BoundProvenanceFields>(
    provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
    knob: CompileKnobId,
    value: T,
    entry: ConstraintProvenance,
) {
    T::push_bound_fields(
        provenance,
        knob,
        value,
        value,
        true,
        entry,
        "overrides.bounds",
    );
}

fn push_leaf_if_changed<T: PartialEq>(
    provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
    knob: CompileKnobId,
    before: &T,
    after: &T,
    force: bool,
    field: &str,
    entry: &ConstraintProvenance,
) {
    if force || before != after {
        push_provenance_path(provenance, field_path(knob, field), entry.clone());
    }
}

macro_rules! impl_single_value_provenance {
    ($ty:ty, $field:ident) => {
        impl ValueProvenanceFields for $ty {
            fn push_value_fields(
                provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
                knob: CompileKnobId,
                before: Self,
                after: Self,
                force: bool,
                entry: ConstraintProvenance,
                prefix: &'static str,
            ) {
                push_leaf_if_changed(
                    provenance,
                    knob,
                    &before.$field,
                    &after.$field,
                    force,
                    &format!("{prefix}.{}", stringify!($field)),
                    &entry,
                );
            }
        }
    };
}

macro_rules! impl_single_bound_provenance {
    ($ty:ty, $field:ident) => {
        impl BoundProvenanceFields for $ty {
            fn push_bound_fields(
                provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
                knob: CompileKnobId,
                before: Self,
                after: Self,
                force: bool,
                entry: ConstraintProvenance,
                prefix: &'static str,
            ) {
                push_leaf_if_changed(
                    provenance,
                    knob,
                    &before.$field,
                    &after.$field,
                    force,
                    &format!("{prefix}.{}", stringify!($field)),
                    &entry,
                );
            }
        }
    };
}

impl_single_value_provenance!(PlacementKnob, profile);
impl_single_value_provenance!(gbf_policy::RangeKnob, reduction_ceiling);
impl_single_value_provenance!(gbf_policy::StorageKnob, materialization);
impl_single_value_provenance!(gbf_policy::SramKnob, page_aggression);
impl_single_value_provenance!(gbf_policy::OverlayKnob, promotion);

impl ValueProvenanceFields for ObservationKnob {
    fn push_value_fields(
        provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
        knob: CompileKnobId,
        before: Self,
        after: Self,
        force: bool,
        entry: ConstraintProvenance,
        prefix: &'static str,
    ) {
        push_leaf_if_changed(
            provenance,
            knob,
            &before.observability,
            &after.observability,
            force,
            &format!("{prefix}.observability"),
            &entry,
        );
        push_leaf_if_changed(
            provenance,
            knob,
            &before.probe_level,
            &after.probe_level,
            force,
            &format!("{prefix}.probe_level"),
            &entry,
        );
    }
}

impl ValueProvenanceFields for RomWindowKnob {
    fn push_value_fields(
        provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
        knob: CompileKnobId,
        before: Self,
        after: Self,
        force: bool,
        entry: ConstraintProvenance,
        prefix: &'static str,
    ) {
        push_leaf_if_changed(
            provenance,
            knob,
            &before.kernel_residency_bias,
            &after.kernel_residency_bias,
            force,
            &format!("{prefix}.kernel_residency_bias"),
            &entry,
        );
        push_leaf_if_changed(
            provenance,
            knob,
            &before.kernel_duplication_bias,
            &after.kernel_duplication_bias,
            force,
            &format!("{prefix}.kernel_duplication_bias"),
            &entry,
        );
    }
}

impl ValueProvenanceFields for ScheduleKnob {
    fn push_value_fields(
        provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
        knob: CompileKnobId,
        before: Self,
        after: Self,
        force: bool,
        entry: ConstraintProvenance,
        prefix: &'static str,
    ) {
        push_leaf_if_changed(
            provenance,
            knob,
            &before.tile_search,
            &after.tile_search,
            force,
            &format!("{prefix}.tile_search"),
            &entry,
        );
        push_leaf_if_changed(
            provenance,
            knob,
            &before.slice_coarsening,
            &after.slice_coarsening,
            force,
            &format!("{prefix}.slice_coarsening"),
            &entry,
        );
        push_leaf_if_changed(
            provenance,
            knob,
            &before.resource_pressure,
            &after.resource_pressure,
            force,
            &format!("{prefix}.resource_pressure"),
            &entry,
        );
    }
}

impl_single_bound_provenance!(gbf_policy::PlacementKnobBounds, max_profile);
impl_single_bound_provenance!(gbf_policy::ObservationKnobBounds, max_probe_level);
impl_single_bound_provenance!(gbf_policy::RangeKnobBounds, max_reduction_ceiling);
impl_single_bound_provenance!(gbf_policy::StorageKnobBounds, max_materialization);
impl_single_bound_provenance!(gbf_policy::SramKnobBounds, max_page_aggression);
impl_single_bound_provenance!(gbf_policy::OverlayKnobBounds, max_promotion);

impl BoundProvenanceFields for gbf_policy::RomWindowKnobBounds {
    fn push_bound_fields(
        provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
        knob: CompileKnobId,
        before: Self,
        after: Self,
        force: bool,
        entry: ConstraintProvenance,
        prefix: &'static str,
    ) {
        push_leaf_if_changed(
            provenance,
            knob,
            &before.max_kernel_residency_bias,
            &after.max_kernel_residency_bias,
            force,
            &format!("{prefix}.max_kernel_residency_bias"),
            &entry,
        );
        push_leaf_if_changed(
            provenance,
            knob,
            &before.max_kernel_duplication_bias,
            &after.max_kernel_duplication_bias,
            force,
            &format!("{prefix}.max_kernel_duplication_bias"),
            &entry,
        );
    }
}

impl BoundProvenanceFields for gbf_policy::ScheduleKnobBounds {
    fn push_bound_fields(
        provenance: &mut BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
        knob: CompileKnobId,
        before: Self,
        after: Self,
        force: bool,
        entry: ConstraintProvenance,
        prefix: &'static str,
    ) {
        push_leaf_if_changed(
            provenance,
            knob,
            &before.max_tile_search,
            &after.max_tile_search,
            force,
            &format!("{prefix}.max_tile_search"),
            &entry,
        );
        push_leaf_if_changed(
            provenance,
            knob,
            &before.max_slice_coarsening,
            &after.max_slice_coarsening,
            force,
            &format!("{prefix}.max_slice_coarsening"),
            &entry,
        );
        push_leaf_if_changed(
            provenance,
            knob,
            &before.max_resource_pressure,
            &after.max_resource_pressure,
            force,
            &format!("{prefix}.max_resource_pressure"),
            &entry,
        );
    }
}

fn provenance_entries(
    mut provenance: BTreeMap<CompileKnobPath, Vec<ConstraintProvenance>>,
) -> Vec<CompileKnobProvenanceEntry> {
    for path in all_required_leaf_paths() {
        provenance.entry(path).or_insert_with(|| {
            vec![ConstraintProvenance {
                source: PolicySource::TargetDefault,
                operation: ConstraintOperation::SeedDefault,
                evidence: Vec::new(),
            }]
        });
    }
    provenance
        .into_iter()
        .map(|(path, chain)| CompileKnobProvenanceEntry { path, chain })
        .collect()
}

fn field_path(knob: CompileKnobId, field: &str) -> CompileKnobPath {
    CompileKnobPath {
        knob,
        selector: None,
        field: Some(FieldPath::from(field)),
    }
}

fn lock_path(knob: CompileKnobId) -> CompileKnobPath {
    field_path(knob, "locks.locked")
}

fn all_knobs() -> [CompileKnobId; 8] {
    [
        CompileKnobId::Placement,
        CompileKnobId::Observation,
        CompileKnobId::Range,
        CompileKnobId::Storage,
        CompileKnobId::Sram,
        CompileKnobId::RomWindow,
        CompileKnobId::Overlay,
        CompileKnobId::Schedule,
    ]
}

fn all_required_leaf_paths() -> Vec<CompileKnobPath> {
    let mut paths = Vec::new();
    paths.extend([
        field_path(CompileKnobId::Placement, "global.profile"),
        field_path(CompileKnobId::Placement, "bounds.max_profile"),
        field_path(CompileKnobId::Observation, "global.observability"),
        field_path(CompileKnobId::Observation, "global.probe_level"),
        field_path(CompileKnobId::Observation, "bounds.max_probe_level"),
        field_path(CompileKnobId::Range, "global.reduction_ceiling"),
        field_path(CompileKnobId::Range, "bounds.max_reduction_ceiling"),
        field_path(CompileKnobId::Storage, "global.materialization"),
        field_path(CompileKnobId::Storage, "bounds.max_materialization"),
        field_path(CompileKnobId::Sram, "global.page_aggression"),
        field_path(CompileKnobId::Sram, "bounds.max_page_aggression"),
        field_path(CompileKnobId::RomWindow, "global.kernel_residency_bias"),
        field_path(CompileKnobId::RomWindow, "global.kernel_duplication_bias"),
        field_path(CompileKnobId::RomWindow, "bounds.max_kernel_residency_bias"),
        field_path(
            CompileKnobId::RomWindow,
            "bounds.max_kernel_duplication_bias",
        ),
        field_path(CompileKnobId::Overlay, "global.promotion"),
        field_path(CompileKnobId::Overlay, "bounds.max_promotion"),
        field_path(CompileKnobId::Schedule, "global.tile_search"),
        field_path(CompileKnobId::Schedule, "global.slice_coarsening"),
        field_path(CompileKnobId::Schedule, "global.resource_pressure"),
        field_path(CompileKnobId::Schedule, "bounds.max_tile_search"),
        field_path(CompileKnobId::Schedule, "bounds.max_slice_coarsening"),
        field_path(CompileKnobId::Schedule, "bounds.max_resource_pressure"),
    ]);
    paths
}

fn state_hint_constraints(frame: &ConstraintFrame) -> Vec<ConstraintEnforcement> {
    all_knobs()
        .into_iter()
        .filter(|knob| partial_has_value(&frame.defaults, *knob))
        .map(|knob| ConstraintEnforcement {
            knob,
            provenance: vec![ConstraintProvenance {
                source: frame.source.clone(),
                operation: ConstraintOperation::ApplyHardConstraint,
                evidence: frame.evidence.clone(),
            }],
        })
        .collect()
}

fn preference_provenance(frame: &ConstraintFrame) -> Vec<ConstraintProvenance> {
    vec![ConstraintProvenance {
        source: frame.source.clone(),
        operation: ConstraintOperation::ApplyPreference,
        evidence: frame.evidence.clone(),
    }]
}

fn partial_has_value(values: &CompileKnobPartialValues, knob: CompileKnobId) -> bool {
    match knob {
        CompileKnobId::Placement => values.placement.is_some(),
        CompileKnobId::Observation => values.observation.is_some(),
        CompileKnobId::Range => values.range.is_some(),
        CompileKnobId::Storage => values.storage.is_some(),
        CompileKnobId::Sram => values.sram.is_some(),
        CompileKnobId::RomWindow => values.rom_window.is_some(),
        CompileKnobId::Overlay => values.overlay.is_some(),
        CompileKnobId::Schedule => values.schedule.is_some(),
    }
}

fn meet_bound<T: BoundMeet>(left: T, right: T) -> T {
    left.meet(right)
}

fn with_bound<T>(mut bounds: CompileKnobBounds, knob: CompileKnobId, next: T) -> CompileKnobBounds
where
    T: Copy,
    CompileKnobBounds: SetBound<T>,
{
    bounds.set_bound(knob, next);
    bounds
}

trait BoundMeet {
    fn meet(self, other: Self) -> Self;
}

impl BoundMeet for gbf_policy::PlacementKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_profile: self.max_profile.min(other.max_profile),
        }
    }
}

impl BoundMeet for gbf_policy::ObservationKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_probe_level: self.max_probe_level.min(other.max_probe_level),
        }
    }
}

impl BoundMeet for gbf_policy::RangeKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_reduction_ceiling: self.max_reduction_ceiling.min(other.max_reduction_ceiling),
        }
    }
}

impl BoundMeet for gbf_policy::StorageKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_materialization: self.max_materialization.min(other.max_materialization),
        }
    }
}

impl BoundMeet for gbf_policy::SramKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_page_aggression: self.max_page_aggression.min(other.max_page_aggression),
        }
    }
}

impl BoundMeet for gbf_policy::RomWindowKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_kernel_residency_bias: self
                .max_kernel_residency_bias
                .min(other.max_kernel_residency_bias),
            max_kernel_duplication_bias: self
                .max_kernel_duplication_bias
                .min(other.max_kernel_duplication_bias),
        }
    }
}

impl BoundMeet for gbf_policy::OverlayKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_promotion: self.max_promotion.min(other.max_promotion),
        }
    }
}

impl BoundMeet for gbf_policy::ScheduleKnobBounds {
    fn meet(self, other: Self) -> Self {
        Self {
            max_tile_search: self.max_tile_search.min(other.max_tile_search),
            max_slice_coarsening: self.max_slice_coarsening.min(other.max_slice_coarsening),
            max_resource_pressure: self.max_resource_pressure.min(other.max_resource_pressure),
        }
    }
}

trait SetBound<T> {
    fn set_bound(&mut self, knob: CompileKnobId, value: T);
}

macro_rules! impl_set_bound {
    ($ty:ty, $variant:ident, $field:ident) => {
        impl SetBound<$ty> for CompileKnobBounds {
            fn set_bound(&mut self, knob: CompileKnobId, value: $ty) {
                if knob == CompileKnobId::$variant {
                    self.$field = value;
                }
            }
        }
    };
}

impl_set_bound!(gbf_policy::PlacementKnobBounds, Placement, placement);
impl_set_bound!(gbf_policy::ObservationKnobBounds, Observation, observation);
impl_set_bound!(gbf_policy::RangeKnobBounds, Range, range);
impl_set_bound!(gbf_policy::StorageKnobBounds, Storage, storage);
impl_set_bound!(gbf_policy::SramKnobBounds, Sram, sram);
impl_set_bound!(gbf_policy::RomWindowKnobBounds, RomWindow, rom_window);
impl_set_bound!(gbf_policy::OverlayKnobBounds, Overlay, overlay);
impl_set_bound!(gbf_policy::ScheduleKnobBounds, Schedule, schedule);

fn target_evidence(validation: &ValidationProduct<'_>) -> EvidenceRef {
    EvidenceRef {
        kind: "target_profile".to_owned(),
        reference: validation.validated.target_profile.id().as_str().to_owned(),
        hash: Some(validation.validated.input_hashes.target_profile_hash),
    }
}

fn profile_evidence(validation: &ValidationProduct<'_>) -> EvidenceRef {
    EvidenceRef {
        kind: "compile_profile".to_owned(),
        reference: validation.validated.compile_profile.id.as_str().to_owned(),
        hash: Some(validation.validated.compile_profile.defaults_hash),
    }
}

fn hint_evidence(validation: &ValidationProduct<'_>) -> EvidenceRef {
    EvidenceRef {
        kind: "hint_bundle".to_owned(),
        reference: "artifact.hint_bundle".to_owned(),
        hash: Some(validation.validated.input_hashes.hint_bundle_hash),
    }
}

fn compile_request_evidence(validation: &ValidationProduct<'_>) -> EvidenceRef {
    EvidenceRef {
        kind: "compile_request".to_owned(),
        reference: "constraint_overrides".to_owned(),
        hash: Some(validation.validated.input_hashes.compile_request_hash),
    }
}

fn calibration_evidence(validation: &ValidationProduct<'_>) -> EvidenceRef {
    EvidenceRef {
        kind: "calibration".to_owned(),
        reference: "calibration_bundle_set".to_owned(),
        hash: Some(validation.validated.input_hashes.calibration_hash),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeSet;

    use gbf_artifact::aux::{ArtifactAux, SemanticCheckpointSchemaId, SemanticCheckpointSchemaRef};
    use gbf_artifact::core::ArtifactCore;
    use gbf_artifact::export_facts::RateQ8_8;
    use gbf_artifact::lowerings::{
        DataLoweringProfileId, LoweringManifest, LoweringShard, LoweringShardId, LoweringShardKind,
        Pack,
    };
    use gbf_artifact::manifest::{ArtifactFeature, ArtifactManifest, ManifestTimestamp};
    use gbf_artifact::preferences::{AffinityPair, CompilePreferences, ExpertSlotAffinity};
    use gbf_artifact::quant::QuantSpec;
    use gbf_artifact::sequence::SequenceSemanticsSpec;
    use gbf_artifact::{
        BuildConstraintEntry, BuildConstraints, EvidenceScope, HintBundle,
        TargetDataLoweringArtifact,
    };
    use gbf_foundation::{
        BlobRef, CompileProfileId, ExpertId, GoldenVectorId, Hash256, LayerId, LineageId,
        PackerVersion, TargetProfileId, WorkloadId,
    };
    use gbf_hw::target::{TargetProfile, dmg_mbc5_8mib_128kib};
    use gbf_policy::{
        BRINGUP_COMPILE_PROFILE_ID, BootstrapCalibrationBundle, CalibrationBundleSet,
        CalibrationConfidenceClass, CalibrationLayer, CompileObjective, CompileProfileSpec,
        CompileRequest, DEFAULT_COMPILE_PROFILE_ID, MeasurementBlob, PlacementKnobBounds,
        PlacementProfile, RomKernelResidencyBias, RomWindowKnob, RuntimeMode,
        ServiceLevelObjective, ValidationCode, canonical_compile_profile_specs,
    };
    use gbf_report::{ReportOutcome, round_trip_self_hash};
    use gbf_workload::{GoldenVectorRef, WorkloadLocator, WorkloadManifest, WorkloadManifestRef};
    use serde::Serialize;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::validate::{
        ArtifactResolveError, ArtifactResolver, ArtifactTransportIdentity,
        CURRENT_ARTIFACT_SCHEMA_VERSION, ImportedArtifactView, ReferenceLink, ResolvedBlob,
        ResolvedEvidence, ResolvedGoldenVector, ResolvedSidecar, ResolvedWorkload, SidecarRef,
        ValidateInputs, compute_artifact_manifest_self_hash, validate_artifact_and_request,
    };

    #[test]
    fn f_b2_resolve_policy_target_defaults_seed_global() {
        let fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        let product = resolve_policy(&fixture.validation()).expect("policy resolves");

        assert_eq!(
            product.policy.knobs.global.placement.profile,
            PlacementProfile::StrictOnePerBank
        );
        assert_eq!(
            product.policy.provenance.target_defaults,
            product.input_hashes.target_profile_hash
        );
    }

    #[test]
    fn f_b2_resolve_policy_profile_defaults_tighten_bounds() {
        let fixture = Fixture::new("Trace");
        let product = resolve_policy(&fixture.validation()).expect("policy resolves");

        assert_eq!(
            product.policy.knobs.bounds.placement.max_profile,
            PlacementProfile::Budgeted
        );
    }

    #[test]
    fn f_b2_resolve_policy_hints_apply_within_bounds() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture
            .artifact
            .hint_bundle
            .constraints
            .entries
            .push(BuildConstraintEntry {
                knob: CompileKnobId::Placement,
                path: None,
                value: ConstraintValue::PlacementProfile {
                    value: PlacementProfile::PackedExperts,
                },
                evidence: Vec::new(),
                scope: EvidenceScope::WholeArtifact,
            });
        fixture.refresh_transport_hash();

        let product = resolve_policy(&fixture.validation()).expect("policy resolves");
        assert_eq!(
            product.policy.knobs.global.placement.profile,
            PlacementProfile::PackedExperts
        );
    }

    #[test]
    fn f_b2_resolve_policy_constraints_tighten_bounds() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            bounds: CompileKnobPartialBounds {
                placement: Some(PlacementKnobBounds {
                    max_profile: PlacementProfile::Budgeted,
                }),
                ..CompileKnobPartialBounds::default()
            },
            ..CompileKnobOverrides::default()
        });

        let product = resolve_policy(&fixture.validation()).expect("policy resolves");
        assert_eq!(
            product.policy.knobs.bounds.placement.max_profile,
            PlacementProfile::Budgeted
        );
        assert_eq!(
            product.policy.knobs.overrides.bounds.placement,
            Some(PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            })
        );
        assert_eq!(
            product
                .report
                .body
                .result
                .as_ref()
                .expect("success report has result")
                .compile_knobs
                .overrides
                .bounds
                .placement,
            Some(PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            })
        );
    }

    #[test]
    fn f_b2_resolve_policy_overrides_only_tighten() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            values: CompileKnobPartialValues {
                placement: Some(PlacementKnob {
                    profile: PlacementProfile::StrictOnePerBank,
                }),
                ..CompileKnobPartialValues::default()
            },
            ..CompileKnobOverrides::default()
        });

        let product = resolve_policy(&fixture.validation()).expect("policy resolves");
        assert_eq!(
            product.policy.knobs.global.placement.profile,
            PlacementProfile::StrictOnePerBank
        );
    }

    #[test]
    fn f_b2_resolve_policy_calibration_data_drives_pressure_thresholds() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture.add_runtime_measurement();

        let product = resolve_policy(&fixture.validation()).expect("policy resolves");
        assert_eq!(
            product.policy.knobs.global.schedule.resource_pressure,
            ScheduleResourcePressure::FitFirst
        );
    }

    #[test]
    fn f_b2_resolve_policy_rejects_locked_knob_override() {
        let mut fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            values: CompileKnobPartialValues {
                rom_window: Some(RomWindowKnob {
                    kernel_residency_bias: RomKernelResidencyBias::PreferWramOverlay,
                    kernel_duplication_bias: RomKernelDuplicationBias::Share,
                }),
                ..CompileKnobPartialValues::default()
            },
            ..CompileKnobOverrides::default()
        });

        let failure = resolve_policy(&fixture.validation()).expect_err("locked override rejects");
        assert_policy_failure(&failure, |code| {
            matches!(
                code,
                ValidationCode::PolicyKnobLockedAndOverridden {
                    knob: CompileKnobId::RomWindow
                }
            )
        });
    }

    #[test]
    fn f_b2_resolve_policy_rejects_out_of_bounds_value() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            values: CompileKnobPartialValues {
                placement: Some(PlacementKnob {
                    profile: PlacementProfile::PackedExperts,
                }),
                ..CompileKnobPartialValues::default()
            },
            bounds: CompileKnobPartialBounds {
                placement: Some(PlacementKnobBounds {
                    max_profile: PlacementProfile::Budgeted,
                }),
                ..CompileKnobPartialBounds::default()
            },
        });

        let failure = resolve_policy(&fixture.validation()).expect_err("out of bounds rejects");
        assert_policy_failure(&failure, |code| {
            matches!(code, ValidationCode::PolicyKnobOutOfBounds { .. })
        });
    }

    #[test]
    fn f_b2_resolve_policy_failure_emits_policy_resolution_failure_report() {
        let mut fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            values: CompileKnobPartialValues {
                placement: Some(PlacementKnob {
                    profile: PlacementProfile::PackedExperts,
                }),
                ..CompileKnobPartialValues::default()
            },
            ..CompileKnobOverrides::default()
        });

        let failure = resolve_policy(&fixture.validation()).expect_err("failure reports");
        assert_eq!(failure.report.schema.as_str(), "policy_resolution.v1");
        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert!(failure.report.body.result.is_none());
        round_trip_self_hash(&failure.report).expect("failure report self-hash round-trips");
    }

    #[test]
    fn f_b2_resolve_policy_failure_does_not_mutate_artifact_validation_report() {
        let mut fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            values: CompileKnobPartialValues {
                placement: Some(PlacementKnob {
                    profile: PlacementProfile::PackedExperts,
                }),
                ..CompileKnobPartialValues::default()
            },
            ..CompileKnobOverrides::default()
        });
        let validation = fixture.validation();
        let before = validation.report.clone();
        let before_hash = validation.artifact_validation_self_hash;

        let _ = resolve_policy(&validation).expect_err("failure reports");

        assert_eq!(validation.report, before);
        assert_eq!(validation.artifact_validation_self_hash, before_hash);
    }

    #[test]
    fn f_b2_resolve_policy_bound_meet_value_violation_reports_out_of_bounds() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            bounds: CompileKnobPartialBounds {
                placement: Some(PlacementKnobBounds {
                    max_profile: PlacementProfile::StrictOnePerBank,
                }),
                ..CompileKnobPartialBounds::default()
            },
            ..CompileKnobOverrides::default()
        });

        let failure =
            resolve_policy(&fixture.validation()).expect_err("value outside bound rejects");
        assert_policy_failure(&failure, |code| {
            matches!(
                code,
                ValidationCode::PolicyKnobOutOfBounds {
                    knob: CompileKnobId::Placement,
                    ..
                }
            )
        });
    }

    #[test]
    fn f_b2_resolve_policy_compile_request_bound_override_cannot_loosen() {
        let mut fixture = Fixture::new("Trace");
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            bounds: CompileKnobPartialBounds {
                placement: Some(PlacementKnobBounds {
                    max_profile: PlacementProfile::PackedExperts,
                }),
                ..CompileKnobPartialBounds::default()
            },
            ..CompileKnobOverrides::default()
        });

        let failure = resolve_policy(&fixture.validation()).expect_err("bound relaxation rejects");
        assert_policy_failure(&failure, |code| {
            matches!(
                code,
                ValidationCode::PolicyConstraintLoosened {
                    knob: CompileKnobId::Placement,
                    ..
                }
            )
        });
    }

    #[test]
    fn f_b2_resolve_policy_target_fixture_leaves_profile_specific_knobs_unset() {
        let fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let validation = fixture.validation();
        let frames = initial_constraint_frames(&validation).expect("frames collect");

        assert_eq!(frames[0].source, PolicySource::TargetDefault);
        assert_eq!(frames[0].defaults, CompileKnobPartialValues::default());
    }

    #[test]
    fn f_b2_resolve_policy_lock_blocks_calibration_change() {
        let mut fixture = Fixture::new("Trace");
        fixture.add_runtime_measurement();

        let failure =
            resolve_policy(&fixture.validation()).expect_err("locked calibration rejects");
        assert_policy_failure(&failure, |code| {
            matches!(
                code,
                ValidationCode::PolicyKnobLockedAndOverridden {
                    knob: CompileKnobId::Schedule
                }
            )
        });
    }

    #[test]
    fn f_b2_resolve_policy_lock_allows_identical_re_affirmation() {
        let mut fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        fixture.compile_request.constraint_overrides = Some(CompileKnobOverrides {
            values: CompileKnobPartialValues {
                placement: Some(PlacementKnob {
                    profile: PlacementProfile::StrictOnePerBank,
                }),
                ..CompileKnobPartialValues::default()
            },
            ..CompileKnobOverrides::default()
        });

        resolve_policy(&fixture.validation()).expect("identical locked value is allowed");
    }

    #[test]
    fn f_b2_resolve_policy_lock_blocks_hint_change() {
        let mut fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        fixture
            .artifact
            .hint_bundle
            .constraints
            .entries
            .push(BuildConstraintEntry {
                knob: CompileKnobId::Placement,
                path: None,
                value: ConstraintValue::PlacementProfile {
                    value: PlacementProfile::PackedExperts,
                },
                evidence: Vec::new(),
                scope: EvidenceScope::WholeArtifact,
            });
        fixture.refresh_transport_hash();

        let failure = resolve_policy(&fixture.validation()).expect_err("locked hint rejects");
        assert_policy_failure(&failure, |code| {
            matches!(
                code,
                ValidationCode::PolicyKnobLockedAndOverridden {
                    knob: CompileKnobId::Placement
                }
            )
        });
    }

    #[test]
    fn f_b2_resolve_policy_lock_allows_identical_hint_re_affirmation() {
        let mut fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        fixture
            .artifact
            .hint_bundle
            .constraints
            .entries
            .push(BuildConstraintEntry {
                knob: CompileKnobId::Placement,
                path: None,
                value: ConstraintValue::PlacementProfile {
                    value: PlacementProfile::StrictOnePerBank,
                },
                evidence: Vec::new(),
                scope: EvidenceScope::WholeArtifact,
            });
        fixture.refresh_transport_hash();

        let product = resolve_policy(&fixture.validation()).expect("identical hint is allowed");
        assert_eq!(
            product.policy.knobs.global.placement.profile,
            PlacementProfile::StrictOnePerBank
        );
        assert_eq!(
            product
                .report
                .body
                .hint_consumption
                .constraints_enforced
                .len(),
            1
        );
    }

    #[test]
    fn f_b2_resolve_policy_rejects_unsupported_hint_constraint_value() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture
            .artifact
            .hint_bundle
            .constraints
            .entries
            .push(BuildConstraintEntry {
                knob: CompileKnobId::Schedule,
                path: None,
                value: ConstraintValue::Bool { value: true },
                evidence: Vec::new(),
                scope: EvidenceScope::WholeArtifact,
            });
        fixture.refresh_transport_hash();

        let failure = resolve_policy(&fixture.validation()).expect_err("unsupported hint rejects");
        assert_policy_failure(&failure, |code| {
            matches!(
                code,
                ValidationCode::PolicyHintConstraintUnsupported {
                    knob: CompileKnobId::Schedule,
                    value: ConstraintValue::Bool { value: true },
                }
            )
        });
        round_trip_self_hash(&failure.report).expect("unsupported failure report self-hash");
    }

    #[test]
    fn f_b2_resolve_policy_records_honored_hint_preference() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture.artifact.hint_bundle.preferences = slot_affinity_preferences();
        fixture.refresh_transport_hash();

        let product = resolve_policy(&fixture.validation()).expect("preference resolves");
        assert_eq!(
            product.policy.knobs.global.placement.profile,
            PlacementProfile::PackedExperts
        );
        assert_eq!(
            product
                .report
                .body
                .hint_consumption
                .preferences_honored
                .len(),
            1
        );
        assert!(
            product
                .report
                .body
                .hint_consumption
                .preferences_ignored
                .is_empty()
        );
    }

    #[test]
    fn f_b2_resolve_policy_ignores_out_of_bounds_hint_preference() {
        let mut fixture = Fixture::new(BRINGUP_COMPILE_PROFILE_ID);
        fixture.artifact.hint_bundle.preferences = slot_affinity_preferences();
        fixture.refresh_transport_hash();

        let product = resolve_policy(&fixture.validation()).expect("oob preference is ignored");
        assert_eq!(
            product.policy.knobs.global.placement.profile,
            PlacementProfile::StrictOnePerBank
        );
        assert!(
            product
                .report
                .body
                .hint_consumption
                .preferences_honored
                .is_empty()
        );
        assert_eq!(
            product.report.body.hint_consumption.preferences_ignored[0].knob,
            CompileKnobId::Placement
        );
        assert_eq!(
            product.report.body.hint_consumption.preferences_ignored[0].reason,
            "outside_bounds"
        );
    }

    #[test]
    fn f_b2_resolve_policy_records_per_knob_provenance() {
        let fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let product = resolve_policy(&fixture.validation()).expect("policy resolves");
        let knobs = product
            .policy
            .knobs
            .provenance
            .iter()
            .map(|entry| entry.path.knob)
            .collect::<BTreeSet<_>>();

        assert_eq!(knobs, BTreeSet::from(all_knobs()));
        assert!(product.policy.knobs.provenance.iter().all(|entry| {
            !entry.chain.is_empty()
                && entry.chain.iter().all(|provenance| {
                    !matches!(provenance.source, PolicySource::RepairProposal { .. })
                })
        }));
    }

    #[test]
    fn f_b2_resolve_policy_records_path_level_provenance() {
        let mut fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        fixture.add_runtime_measurement();

        let product = resolve_policy(&fixture.validation()).expect("policy resolves");
        assert!(
            product
                .policy
                .knobs
                .provenance
                .iter()
                .all(|entry| { entry.path.selector.is_none() && entry.path.field.is_some() })
        );

        let pressure = provenance_entry(
            &product,
            CompileKnobId::Schedule,
            "global.resource_pressure",
        );
        assert!(pressure.chain.iter().any(|provenance| {
            provenance.source == PolicySource::Calibration
                && provenance.operation == ConstraintOperation::ApplyCalibration
        }));

        let max_pressure = provenance_entry(
            &product,
            CompileKnobId::Schedule,
            "bounds.max_resource_pressure",
        );
        assert!(max_pressure.chain.iter().any(|provenance| {
            provenance.source == PolicySource::TargetDefault
                && provenance.operation == ConstraintOperation::SeedDefault
        }));
    }

    #[test]
    fn f_b2_resolve_policy_no_repair_proposal_provenance_in_chunk() {
        let fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let product = resolve_policy(&fixture.validation()).expect("policy resolves");

        assert!(product.policy.knobs.provenance.iter().all(|entry| {
            entry
                .chain
                .iter()
                .all(|provenance| !matches!(provenance.source, PolicySource::RepairProposal { .. }))
        }));
    }

    #[test]
    fn f_b2_resolve_policy_no_profile_relaxation_field() {
        let fixture = Fixture::new(DEFAULT_COMPILE_PROFILE_ID);
        let product = resolve_policy(&fixture.validation()).expect("policy resolves");
        let encoded = serde_json::to_string(&product.report).expect("report serializes");

        assert!(!encoded.contains("relaxation"));
        assert!(!encoded.contains("AuthorizedRelaxation"));
    }

    fn provenance_entry<'a>(
        product: &'a ResolvedPolicyProduct,
        knob: CompileKnobId,
        field: &str,
    ) -> &'a CompileKnobProvenanceEntry {
        product
            .policy
            .knobs
            .provenance
            .iter()
            .find(|entry| {
                entry.path.knob == knob
                    && entry.path.field.as_ref() == Some(&FieldPath::from(field))
            })
            .expect("path-level provenance entry exists")
    }

    fn assert_policy_failure(
        failure: &PolicyResolutionStageFailure,
        matches_code: impl Fn(&ValidationCode) -> bool,
    ) {
        assert_eq!(failure.report.outcome, ReportOutcome::Failed);
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| matches_code(&diagnostic.code)),
            "diagnostics were {:#?}",
            failure.diagnostics
        );
    }

    struct Fixture {
        artifact: ImportedArtifactView,
        lowerings: Vec<TargetDataLoweringArtifact>,
        workloads: Vec<WorkloadManifestRef>,
        golden_vectors: Vec<GoldenVectorRef>,
        compile_request: CompileRequest,
        target_profile: TargetProfile,
        compile_profile: CompileProfileSpec,
        calibration: CalibrationBundleSet,
        resolver: Resolver,
    }

    impl Fixture {
        fn new(profile: &str) -> Self {
            let target_profile = dmg_mbc5_8mib_128kib();
            let target_profile_hash = input_hash(
                "gbf-hw",
                "TargetProfile",
                "target_profile",
                "1.0.0",
                &target_profile,
            );
            let mut artifact = ImportedArtifactView::new(
                artifact_core(),
                artifact_manifest(),
                artifact_aux(),
                Some(HintBundle {
                    constraints: BuildConstraints::empty(),
                    ..HintBundle::empty()
                }),
                None::<ReferenceLink>,
                transport_identity(),
            );
            artifact.transport.transport_hash = imported_artifact_source_hash(&artifact);

            let mut calibration = BootstrapCalibrationBundle::new(target_profile_hash);
            for bundle in calibration.bundles.values_mut() {
                bundle.confidence = CalibrationConfidenceClass::Strong;
            }

            Self {
                artifact,
                lowerings: vec![lowering()],
                workloads: vec![workload()],
                golden_vectors: vec![golden_vector()],
                compile_request: compile_request(profile),
                target_profile,
                compile_profile: compile_profile(profile),
                calibration,
                resolver: Resolver::default(),
            }
        }

        fn validation(&self) -> ValidationProduct<'_> {
            validate_artifact_and_request(self.inputs()).expect("fixture validates")
        }

        fn inputs(&self) -> ValidateInputs<'_> {
            ValidateInputs {
                artifact: &self.artifact,
                lowerings: &self.lowerings,
                workloads: &self.workloads,
                golden_vectors: &self.golden_vectors,
                compile_request: &self.compile_request,
                target_profile: &self.target_profile,
                compile_profile: &self.compile_profile,
                calibration: Some(&self.calibration),
                resolver: &self.resolver,
            }
        }

        fn refresh_transport_hash(&mut self) {
            self.artifact.transport.transport_hash = imported_artifact_source_hash(&self.artifact);
        }

        fn add_runtime_measurement(&mut self) {
            self.calibration
                .bundles
                .get_mut(&CalibrationLayer::Runtime)
                .expect("runtime calibration exists")
                .measurements = Some(MeasurementBlob {
                schema: "fixture.measurements.v1".to_owned(),
                payload_hash: hash(0xaa),
            });
        }
    }

    #[derive(Default)]
    struct Resolver {
        workload_resolve_calls: Cell<usize>,
        golden_vector_resolve_calls: Cell<usize>,
    }

    impl ArtifactResolver for Resolver {
        fn resolve_blob(&self, blob: &BlobRef) -> Result<ResolvedBlob, ArtifactResolveError> {
            Ok(ResolvedBlob {
                bytes: Vec::new(),
                content_hash: blob.hash,
            })
        }

        fn resolve_sidecar(
            &self,
            _sidecar: &SidecarRef,
        ) -> Result<ResolvedSidecar, ArtifactResolveError> {
            let bytes = Vec::new();
            Ok(ResolvedSidecar {
                content_hash: sha256_hash(&bytes),
                bytes,
            })
        }

        fn resolve_evidence(
            &self,
            evidence: &EvidenceRef,
        ) -> Result<ResolvedEvidence, ArtifactResolveError> {
            Ok(ResolvedEvidence {
                bytes: evidence.reference.as_bytes().to_vec(),
                content_hash: evidence.hash,
            })
        }

        fn resolve_workload(
            &self,
            workload: &WorkloadManifestRef,
        ) -> Result<ResolvedWorkload, ArtifactResolveError> {
            self.workload_resolve_calls
                .set(self.workload_resolve_calls.get() + 1);
            Ok(ResolvedWorkload {
                manifest: WorkloadManifest {
                    id: workload.id.clone(),
                    schema_version: gbf_workload::WorkloadSchemaVersion { epoch: 1, minor: 0 },
                    self_hash: workload.manifest_hash,
                    golden_vectors: vec![golden_vector()],
                    future_fields: gbf_workload::WorkloadFuturePlaceholder::default(),
                },
            })
        }

        fn resolve_golden_vector(
            &self,
            _vector: &GoldenVectorRef,
        ) -> Result<ResolvedGoldenVector, ArtifactResolveError> {
            self.golden_vector_resolve_calls
                .set(self.golden_vector_resolve_calls.get() + 1);
            Ok(ResolvedGoldenVector {
                bytes: golden_vector_bytes().to_vec(),
                manifest_hash: golden_vector_hash(),
            })
        }
    }

    fn artifact_core() -> ArtifactCore {
        ArtifactCore::new(
            Vec::new(),
            QuantSpec::default(),
            SequenceSemanticsSpec::linear_state(1).expect("fixture state width is nonzero"),
        )
        .expect("empty core is valid")
    }

    fn artifact_aux() -> ArtifactAux {
        ArtifactAux {
            checkpoint_schema: Some(SemanticCheckpointSchemaRef {
                id: SemanticCheckpointSchemaId("checkpoint.fixture".to_owned()),
                hash: sha256_hash(&[]),
            }),
            conformance_envelope: None,
            golden_vectors: Vec::new(),
            interaction_bundle: None,
            lexical_spec: None,
            reference_observation_cache: None,
        }
    }

    fn artifact_manifest() -> ArtifactManifest {
        let mut manifest = ArtifactManifest {
            components: Vec::new(),
            created_at: ManifestTimestamp(0),
            lineage: LineageId(hash(0x08)),
            manifest_self_hash: Hash256::ZERO,
            required_features: BTreeSet::from([
                ArtifactFeature::DenseI8,
                ArtifactFeature::LinearStateSequence,
            ]),
            schema_version: CURRENT_ARTIFACT_SCHEMA_VERSION,
            semantic_core_hash: artifact_core().semantic_hash(),
        };
        manifest.manifest_self_hash = compute_artifact_manifest_self_hash(&manifest);
        manifest
    }

    fn transport_identity() -> ArtifactTransportIdentity {
        ArtifactTransportIdentity {
            source_uri: Some("fixture://artifact".to_owned()),
            transport_hash: Hash256::ZERO,
            import_tool_hash: hash(0x02),
        }
    }

    fn lowering() -> TargetDataLoweringArtifact {
        let shards = vec![lowering_shard(
            "weight.layer0",
            LoweringShardKind::WeightShard,
            hash(0x04),
        )];
        TargetDataLoweringArtifact {
            profile: DataLoweringProfileId("dmg-default".to_owned()),
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            packer_version: PackerVersion::new(1, 0, 0),
            manifest_hash: lowering_manifest_hash(&shards),
            shards,
        }
    }

    fn lowering_shard(id: &str, kind: LoweringShardKind, payload_hash: Hash256) -> LoweringShard {
        let mut shard = LoweringShard {
            id: LoweringShardId(id.to_owned()),
            kind,
            payload_hash,
            packed_bytes_hash: Hash256::ZERO,
        };
        shard.packed_bytes_hash = sha256_hash(&shard.pack().expect("shard packs"));
        shard
    }

    fn lowering_manifest_hash(shards: &[LoweringShard]) -> Hash256 {
        let manifest = LoweringManifest {
            shard_refs: shards
                .iter()
                .map(|shard| gbf_foundation::LoweringShardRef {
                    id: shard.id.clone(),
                    manifest_hash: shard.packed_bytes_hash,
                })
                .collect(),
            aggregate_hash: Hash256::ZERO,
        };
        sha256_hash(&manifest.pack().expect("manifest packs"))
    }

    fn workload() -> WorkloadManifestRef {
        WorkloadManifestRef {
            id: WorkloadId::from("workload.fixture"),
            manifest_hash: hash(0x06),
            locator: WorkloadLocator::Path {
                path: "fixtures/workload.json".to_owned(),
            },
        }
    }

    fn golden_vector() -> GoldenVectorRef {
        GoldenVectorRef {
            id: GoldenVectorId("golden.fixture".to_owned()),
            manifest_hash: golden_vector_hash(),
        }
    }

    fn golden_vector_bytes() -> &'static [u8] {
        b"golden vector fixture"
    }

    fn golden_vector_hash() -> Hash256 {
        sha256_hash(golden_vector_bytes())
    }

    fn compile_request(profile: &str) -> CompileRequest {
        CompileRequest {
            target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
            profile: CompileProfileId::from(profile),
            objective: CompileObjective {
                service: Some(ServiceLevelObjective {
                    max_first_token_cycles_p95: Some(21_000),
                    max_checkpoint_gap_cycles_p95: Some(13_000),
                    max_resume_latency_cycles_p95: Some(8_000),
                    max_ui_jitter_frames_p99: Some(2),
                }),
                max_cycles_per_token: Some(24_000),
                max_bank_switches_per_token: Some(17),
                max_sram_page_switches_per_token: Some(3),
                min_ui_headroom_pct: 11,
                max_rom_bytes: Some(2 * 1024 * 1024),
                risk: gbf_policy::RiskPolicy {
                    cycle_quantile: 95,
                    switch_quantile: 99,
                    calibration_confidence_requirement:
                        gbf_policy::CalibrationConfidenceRequirement::NoMinimumConfidence,
                    fallback_profile: None,
                    fallback_runtime_mode: Some(RuntimeMode::Safe),
                },
            },
            calibration_set_ref: BootstrapCalibrationBundle::dmg_mbc5_ref(),
            required_features: BTreeSet::from([gbf_policy::CompilerFeature::ArtifactValidation]),
            constraint_overrides: None,
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Safe]),
        }
    }

    fn slot_affinity_preferences() -> CompilePreferences {
        CompilePreferences::new(vec![
            ExpertSlotAffinity::new(
                LayerId::new(0),
                vec![
                    AffinityPair::new(
                        ExpertId::new(0),
                        ExpertId::new(1),
                        RateQ8_8::new(128).expect("rate is valid"),
                    )
                    .expect("affinity pair is valid"),
                ],
            )
            .expect("slot affinity is valid"),
        ])
        .expect("preferences are valid")
    }

    fn compile_profile(id: &str) -> CompileProfileSpec {
        canonical_compile_profile_specs()
            .expect("canonical profiles parse")
            .into_iter()
            .find(|profile| profile.id.as_str() == id)
            .unwrap_or_else(|| panic!("{id} profile exists"))
    }

    fn imported_artifact_source_hash(artifact: &ImportedArtifactView) -> Hash256 {
        #[derive(Serialize)]
        struct SourceHashMaterial<'a> {
            core: &'a ArtifactCore,
            manifest: &'a ArtifactManifest,
            aux: &'a ArtifactAux,
            hint_bundle: &'a HintBundle,
            reference: &'a Option<ReferenceLink>,
        }

        input_hash(
            "gbf-codegen",
            "ImportedArtifactViewSource",
            "imported_artifact_source",
            "1.0.0",
            &SourceHashMaterial {
                core: &artifact.core,
                manifest: &artifact.manifest,
                aux: &artifact.aux,
                hint_bundle: &artifact.hint_bundle,
                reference: &artifact.reference,
            },
        )
    }

    fn input_hash<T: Serialize + ?Sized>(
        crate_name: &str,
        type_name: &str,
        schema_id: &str,
        schema_version: &str,
        value: &T,
    ) -> Hash256 {
        let value = serde_json::to_value(value).expect("input serializes");
        let canonical = gbf_report::canonicalize_value(&value).expect("input canonicalizes");
        let mut hasher = Sha256::new();
        hasher.update(format!(
            "gbf:{crate_name}:{type_name}:{schema_id}:{schema_version}\0"
        ));
        hasher.update(canonical);
        Hash256::from_bytes(hasher.finalize().into())
    }

    fn hash(byte: u8) -> Hash256 {
        Hash256::from_bytes([byte; 32])
    }

    fn sha256_hash(bytes: &[u8]) -> Hash256 {
        Hash256::from_bytes(Sha256::digest(bytes).into())
    }
}

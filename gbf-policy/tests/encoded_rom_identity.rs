use std::collections::BTreeSet;

use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
use gbf_policy::budget::{RomBudgetSlot, RuntimeMemoryCapSection};
use gbf_policy::compile::{
    BRINGUP_COMPILE_PROFILE_ID, S5_ENCODED_ROM_REQUIRED_SEEDS, S5AttentionOracleBindingHashes,
    S5AttentionOracleReport, S5AttentionOracleReportPayload, S5EncodedRomBuildIdentityBlock,
    S5EncodedRomCertKind, S5EncodedRomCertValidity, S5EncodedRomIdentityField,
    S5EncodedRomO11Error, verify_s5_encoded_rom_er3_runtime_nucleus_hash,
    verify_s5_encoded_rom_er7_attention_oracle_report, verify_s5_o11_all_seed_certs,
};
use gbf_policy::{BudgetSlotClass, PlacementProfile, RuntimeChromeBudget, RuntimeNucleusHash};

#[test]
fn er_3_runtime_nucleus_hash_equality() {
    let budget = runtime_budget(runtime_hash(0x51));
    let identity = build_identity(runtime_hash(0x51), hash(0x71));

    verify_s5_encoded_rom_er3_runtime_nucleus_hash(&identity, &budget)
        .expect("matching build identity and chrome budget runtime hashes pass ER-3");
}

#[test]
fn er_3_diagnoses_mismatched_chrome_budget() {
    let budget = runtime_budget(runtime_hash(0x51));
    let identity = build_identity(runtime_hash(0x52), hash(0x71));

    let error = verify_s5_encoded_rom_er3_runtime_nucleus_hash(&identity, &budget)
        .expect_err("mismatched runtime nucleus hash must fail ER-3");

    assert_eq!(error.field, S5EncodedRomIdentityField::RuntimeNucleusHash);
    assert_eq!(error.expected, runtime_hash(0x51));
    assert_eq!(error.observed, runtime_hash(0x52));
}

#[test]
fn er_7_artifact_core_hash_equality_with_oracle_report() {
    let report = oracle_report(hash(0x81));
    let identity = build_identity(runtime_hash(0x51), hash(0x81));

    verify_s5_encoded_rom_er7_attention_oracle_report(&identity, &report)
        .expect("build identity artifact_core_hash matches oracle report artifact binding");
}

#[test]
fn er_7_diagnoses_oracle_artifact_mismatch() {
    let report = oracle_report(hash(0x81));
    let identity = build_identity(runtime_hash(0x51), hash(0x82));

    let error = verify_s5_encoded_rom_er7_attention_oracle_report(&identity, &report)
        .expect_err("wrong ROM artifact_core_hash must fail ER-7");

    assert_eq!(error.field, S5EncodedRomIdentityField::ArtifactCoreHash);
    assert_eq!(error.expected, hash(0x81));
    assert_eq!(error.observed, hash(0x82));
}

#[test]
fn o11_all_certs_valid_seeds_0_through_4() {
    let certs = S5_ENCODED_ROM_REQUIRED_SEEDS
        .into_iter()
        .map(S5EncodedRomCertValidity::all_valid_for_seed)
        .collect::<Vec<_>>();

    verify_s5_o11_all_seed_certs(&certs)
        .expect("O11 passes only when every required seed has all certs valid");
}

#[test]
fn o11_one_seed_missing_cert_fails() {
    let certs = [0, 1, 2, 4]
        .into_iter()
        .map(S5EncodedRomCertValidity::all_valid_for_seed)
        .collect::<Vec<_>>();

    let error =
        verify_s5_o11_all_seed_certs(&certs).expect_err("O11 must fail when seed 3 is absent");

    assert_eq!(error, S5EncodedRomO11Error::MissingSeed { seed: 3 });
}

#[test]
fn o11_invalid_cert_surfaces_seed_and_cert_kind() {
    for (cert_kind, invalidate) in [
        (
            S5EncodedRomCertKind::Range,
            invalidate_range_cert as fn(&mut S5EncodedRomCertValidity),
        ),
        (S5EncodedRomCertKind::Arena, invalidate_arena_cert),
        (S5EncodedRomCertKind::Window, invalidate_window_cert),
        (
            S5EncodedRomCertKind::Reachability,
            invalidate_reachability_cert,
        ),
        (
            S5EncodedRomCertKind::ResourceState,
            invalidate_resource_state_cert,
        ),
    ] {
        let mut certs = S5_ENCODED_ROM_REQUIRED_SEEDS
            .into_iter()
            .map(S5EncodedRomCertValidity::all_valid_for_seed)
            .collect::<Vec<_>>();
        invalidate(&mut certs[2]);

        let error = verify_s5_o11_all_seed_certs(&certs)
            .expect_err("O11 must fail when any per-seed cert is invalid");

        assert_eq!(
            error,
            S5EncodedRomO11Error::InvalidCert { seed: 2, cert_kind }
        );
    }
}

fn build_identity(
    runtime_nucleus_hash: RuntimeNucleusHash,
    artifact_core_hash: Hash256,
) -> S5EncodedRomBuildIdentityBlock {
    S5EncodedRomBuildIdentityBlock {
        runtime_nucleus_hash,
        artifact_core_hash,
    }
}

fn oracle_report(artifact_core_hash: Hash256) -> S5AttentionOracleReport {
    S5AttentionOracleReport::new(
        0,
        S5AttentionOracleBindingHashes {
            phase_a_checkpoint_sha: artifact_core_hash,
            projection_tensors_sha: hash(0x91),
            quant_spec_sha: hash(0x92),
            activation_clip_sha: hash(0x93),
        },
        S5AttentionOracleReportPayload {
            fixture_suite_sha: hash(0xa1),
            spec_sha: hash(0xa2),
            per_fixture_results: Vec::new(),
            aggregate_max_abs_diff: 0.0,
            aggregate_p99_max_abs_diff: 0.0,
            aggregate_agreement: true,
        },
    )
}

fn runtime_budget(runtime_nucleus_hash: RuntimeNucleusHash) -> RuntimeChromeBudget {
    RuntimeChromeBudget {
        target: TargetProfileId::from("dmg-mbc5-8mib-128kib"),
        profile: CompileProfileId::from(BRINGUP_COMPILE_PROFILE_ID),
        runtime_nucleus_hash,
        rom_slots: vec![RomBudgetSlot {
            id: BudgetSlotId::new(0),
            class: BudgetSlotClass::Bank0Free,
            usable_bytes: 8 * 1024,
            reserved_slack: 256,
            placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
        }],
        memory_caps: RuntimeMemoryCapSection {
            wram_usable_bytes: 8 * 1024,
            sram_usable_bytes: 32 * 1024,
            hram_usable_bytes: 127,
            source_target_profile_hash: hash(0x09),
        },
        wram_reserved: 128,
        sram_reserved: 512,
    }
}

fn runtime_hash(byte: u8) -> RuntimeNucleusHash {
    RuntimeNucleusHash::real(hash(byte))
}

fn invalidate_range_cert(cert: &mut S5EncodedRomCertValidity) {
    cert.range_cert_valid = false;
}

fn invalidate_arena_cert(cert: &mut S5EncodedRomCertValidity) {
    cert.arena_cert_valid = false;
}

fn invalidate_window_cert(cert: &mut S5EncodedRomCertValidity) {
    cert.window_cert_valid = false;
}

fn invalidate_reachability_cert(cert: &mut S5EncodedRomCertValidity) {
    cert.reachability_cert_valid = false;
}

fn invalidate_resource_state_cert(cert: &mut S5EncodedRomCertValidity) {
    cert.resource_state_cert_valid = false;
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

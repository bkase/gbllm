use gbf_foundation::Hash256;
use gbf_policy::{O9SafetensorsHashObservation, S5VariantId, o9_cross_seed_difference_report};

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn observation(
    variant: S5VariantId,
    seed: u64,
    safetensors_hash: Hash256,
) -> O9SafetensorsHashObservation {
    O9SafetensorsHashObservation {
        variant,
        seed,
        safetensors_hash,
    }
}

fn varying_observations_for(variant: S5VariantId, base: u8) -> Vec<O9SafetensorsHashObservation> {
    (0..5)
        .map(|seed| observation(variant, seed, hash(base + u8::from(seed != 0))))
        .collect()
}

#[test]
fn o9_passes_when_every_variant_has_cross_seed_variation() {
    let mut observations = Vec::new();
    observations.extend(varying_observations_for(S5VariantId::BoundedKv, 0x10));
    observations.extend(varying_observations_for(S5VariantId::LFix1, 0x20));
    observations.extend(varying_observations_for(S5VariantId::LMt4, 0x30));

    let report = o9_cross_seed_difference_report(&observations);

    assert!(report.passes);
    assert_eq!(report.variant_results.len(), 3);
    assert!(
        report
            .variant_results
            .iter()
            .all(|result| result.distinct_safetensors_hash_count == 2)
    );
}

#[test]
fn o9_fails_when_one_variant_has_all_identical_hashes() {
    let mut observations = Vec::new();
    observations.extend(varying_observations_for(S5VariantId::BoundedKv, 0x10));
    observations.extend((0..5).map(|seed| observation(S5VariantId::LFix1, seed, hash(0x20))));
    observations.extend(varying_observations_for(S5VariantId::LMt4, 0x30));

    let report = o9_cross_seed_difference_report(&observations);
    let failing = report
        .variant_results
        .iter()
        .find(|result| result.variant == S5VariantId::LFix1)
        .expect("L_FIX1 result is present");

    assert!(!report.passes);
    assert!(!failing.passes);
    assert_eq!(failing.observed_seed_count, 5);
    assert_eq!(failing.distinct_safetensors_hash_count, 1);
}

use gbf_model::loss::temporal_smoothness::{
    SmoothnessWindow, TemporalSmoothnessPair, s7_temporal_smoothness_with_boundaries,
};

#[test]
fn l_switch_pair_generation_does_not_cross_sequence_boundary() {
    let sequence_mask = vec![true; 5];
    let boundary_before = vec![false, false, false, true, false];
    let pairs = s7_temporal_smoothness_with_boundaries(
        &sequence_mask,
        &boundary_before,
        SmoothnessWindow::new(3).unwrap(),
    )
    .unwrap();

    assert!(pairs.contains(&TemporalSmoothnessPair { t: 2, u: 0 }));
    assert!(!pairs.contains(&TemporalSmoothnessPair { t: 3, u: 2 }));
    assert!(!pairs.contains(&TemporalSmoothnessPair { t: 4, u: 2 }));
    assert!(pairs.contains(&TemporalSmoothnessPair { t: 4, u: 3 }));
}

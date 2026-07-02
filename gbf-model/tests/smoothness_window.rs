use gbf_model::loss::temporal_smoothness::{
    S7_DEFAULT_SMOOTHNESS_WINDOW, SmoothnessWindow, TemporalSmoothnessError,
    TemporalSmoothnessPair, s7_temporal_smoothness, s7_temporal_smoothness_with_boundaries,
};

const S7_SEQ_256_WINDOW_32_PAIR_COUNT: usize = 7_664;

#[test]
fn smoothness_window_32_generates_full_previous_window_pairs() {
    let sequence_mask = vec![true; 256];
    let window = SmoothnessWindow::s7_default();

    let pairs = s7_temporal_smoothness(&sequence_mask, window);
    let expected_pair_count = 256 * 32 - 32 * (usize::from(S7_DEFAULT_SMOOTHNESS_WINDOW) + 1) / 2;

    assert_eq!(expected_pair_count, S7_SEQ_256_WINDOW_32_PAIR_COUNT);
    assert_ne!(expected_pair_count, 256 * 32 - 32 * (32 - 1) / 2);
    assert_eq!(
        pairs.len(),
        full_window_pair_count(
            sequence_mask.len(),
            usize::from(S7_DEFAULT_SMOOTHNESS_WINDOW)
        )
    );
    assert_eq!(pairs.len(), S7_SEQ_256_WINDOW_32_PAIR_COUNT);
    assert_eq!(
        pairs.iter().filter(|pair| pair.t == 40).count(),
        usize::from(S7_DEFAULT_SMOOTHNESS_WINDOW)
    );
    for u in 8..40 {
        assert!(pairs.contains(&TemporalSmoothnessPair { t: 40, u }));
    }
}

#[test]
fn smoothness_window_one_is_rejected_at_construction() {
    assert_eq!(
        SmoothnessWindow::new(1).unwrap_err(),
        TemporalSmoothnessError::SmoothnessWindowTooSmall { value: 1 }
    );
}

#[test]
fn explicit_boundary_before_t_excludes_current_token_pairs() {
    let sequence_mask = vec![true; 4];
    let sequence_boundary_before = vec![false, false, true, false];
    let window = SmoothnessWindow::new(3).unwrap();

    let pairs =
        s7_temporal_smoothness_with_boundaries(&sequence_mask, &sequence_boundary_before, window)
            .unwrap();

    assert_eq!(
        pairs,
        vec![
            TemporalSmoothnessPair { t: 1, u: 0 },
            TemporalSmoothnessPair { t: 3, u: 2 },
        ]
    );
    assert!(pairs.contains(&TemporalSmoothnessPair { t: 1, u: 0 }));
    assert!(!pairs.contains(&TemporalSmoothnessPair { t: 2, u: 0 }));
    assert!(!pairs.contains(&TemporalSmoothnessPair { t: 2, u: 1 }));
    assert!(!pairs.contains(&TemporalSmoothnessPair { t: 3, u: 1 }));
    assert!(pairs.contains(&TemporalSmoothnessPair { t: 3, u: 2 }));
}

#[test]
fn invalid_mask_token_resets_window() {
    let sequence_mask = vec![true, false, true, true];
    let window = SmoothnessWindow::new(3).unwrap();

    let pairs = s7_temporal_smoothness(&sequence_mask, window);

    assert_eq!(pairs, vec![TemporalSmoothnessPair { t: 3, u: 2 }]);
    assert!(!pairs.contains(&TemporalSmoothnessPair { t: 2, u: 0 }));
    assert!(!pairs.contains(&TemporalSmoothnessPair { t: 3, u: 0 }));
    assert!(pairs.contains(&TemporalSmoothnessPair { t: 3, u: 2 }));
}

#[test]
fn boundary_mask_length_must_match_sequence_mask_length() {
    let err = s7_temporal_smoothness_with_boundaries(
        &[true, true],
        &[false],
        SmoothnessWindow::new(2).unwrap(),
    )
    .unwrap_err();

    assert_eq!(
        err,
        TemporalSmoothnessError::BoundaryMaskLenMismatch {
            sequence_mask_len: 2,
            boundary_mask_len: 1,
        }
    );
}

#[test]
fn first_token_has_no_pairs_and_short_sequence_uses_triangular_prefix() {
    let sequence_mask = vec![true; 10];
    let window = SmoothnessWindow::s7_default();

    let pairs = s7_temporal_smoothness(&sequence_mask, window);

    assert!(pairs.iter().all(|pair| pair.t > 0));
    assert_eq!(pairs.len(), 10 * 9 / 2);
}

fn full_window_pair_count(sequence_len: usize, smoothness_window: usize) -> usize {
    (1..sequence_len)
        .map(|t| t.min(smoothness_window))
        .sum::<usize>()
}

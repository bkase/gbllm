mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::run::threshold_init::{
    PerRowThresholds, ThresholdInitError, ThresholdInitMatrix, initialize_thresholds,
};
use proptest::prelude::*;
use serde_json::Value;

#[test]
fn threshold_init_matches_hand_computed_4x8_reference() {
    let weights = [
        1.0, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 3.5, -3.5,
        1.25, -1.75, 2.25, -2.75, 3.25, -3.75, 4.25, -4.75, 0.0, 0.0, 0.0, 0.0, 8.0, -8.0, 4.0,
        -4.0,
    ];
    let capture = TraceCapture::default();
    let result = with_trace_capture(&capture, || {
        initialize_thresholds(
            &[ThresholdInitMatrix {
                id: "toy0.ffn.weight",
                rows: 4,
                cols: 8,
                weights: &weights,
            }],
            0.7,
        )
        .unwrap()
    });

    assert_eq!(result.threshold_count, 4);
    assert_eq!(
        threshold_bits(&result.buffers[0].thresholds),
        [0x4049_9999, 0x3fb3_3333, 0x4006_6666, 0x4006_6666]
    );
    assert_eq!(result.mean_threshold.to_bits(), 0x400c_0000);
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "threshold_init_complete"
            && event.fields.get("threshold_count").and_then(Value::as_u64) == Some(4)
    }));
}

#[test]
fn threshold_init_is_byte_equal_across_replays() {
    let weights = [1.0, -2.0, 3.0, -4.0];
    let matrix = ThresholdInitMatrix {
        id: "toy0.replay.weight",
        rows: 2,
        cols: 2,
        weights: &weights,
    };

    assert_eq!(
        initialize_thresholds(&[matrix], 0.7).unwrap(),
        initialize_thresholds(&[matrix], 0.7).unwrap()
    );
}

#[test]
fn per_weight_threshold_shape_is_rejected() {
    assert_eq!(
        PerRowThresholds::new("toy0.ffn.weight", 4, vec![0.1; 32]).unwrap_err(),
        ThresholdInitError::ThresholdCountMismatch {
            rows: 4,
            observed: 32
        }
    );
}

#[test]
fn threshold_init_rejects_bad_matrix_shapes() {
    assert!(matches!(
        initialize_thresholds(
            &[ThresholdInitMatrix {
                id: "bad",
                rows: 2,
                cols: 2,
                weights: &[1.0, 2.0],
            }],
            0.7
        ),
        Err(ThresholdInitError::WeightCountMismatch {
            expected: 4,
            observed: 2
        })
    ));
    assert!(matches!(
        initialize_thresholds(
            &[ThresholdInitMatrix {
                id: "bad",
                rows: 1,
                cols: 1,
                weights: &[f32::NAN],
            }],
            0.7
        ),
        Err(ThresholdInitError::NonFiniteWeight)
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn arb_canonical_weight_matrix_thresholds_are_per_row_and_replay_stable(
        (rows, cols, weights) in arb_canonical_weight_matrix()
    ) {
        let matrix = ThresholdInitMatrix {
            id: "prop.weight",
            rows,
            cols,
            weights: &weights,
        };
        let result = initialize_thresholds(&[matrix], 0.7).unwrap();
        let replay = initialize_thresholds(&[matrix], 0.7).unwrap();

        prop_assert_eq!(&result, &replay);
        prop_assert_eq!(result.threshold_count, rows);
        prop_assert_eq!(result.buffers[0].thresholds.len(), rows);
        prop_assert_eq!(
            threshold_bits(&result.buffers[0].thresholds),
            independent_threshold_bits(&weights, rows, cols, 0.7)
        );
    }
}

fn arb_canonical_weight_matrix() -> impl Strategy<Value = (usize, usize, Vec<f32>)> {
    (1_usize..=8, 1_usize..=8).prop_flat_map(|(rows, cols)| {
        let weight_count = rows * cols;
        proptest::collection::vec(-1024.0_f32..=1024.0, weight_count)
            .prop_map(move |weights| (rows, cols, weights))
    })
}

fn independent_threshold_bits(
    weights: &[f32],
    rows: usize,
    cols: usize,
    multiplier: f32,
) -> Vec<u32> {
    (0..rows)
        .map(|row| {
            let start = row * cols;
            let sum_abs = weights[start..start + cols]
                .iter()
                .map(|value| f64::from(*value).abs())
                .sum::<f64>();
            ((f64::from(multiplier) * (sum_abs / cols as f64)) as f32).to_bits()
        })
        .collect()
}

fn threshold_bits(thresholds: &[f32]) -> Vec<u32> {
    thresholds
        .iter()
        .map(|threshold| threshold.to_bits())
        .collect()
}

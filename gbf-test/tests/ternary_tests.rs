use std::num::NonZeroU16;

use gbf_artifact::quant::TernaryQuantEntry;
use gbf_artifact::tensor::{CanonicalTensor, CanonicalTensorKind};
use gbf_artifact::weight_plan::{
    ScaleFormat, ScaleGranularity, TernaryWeightPlan, ThresholdPlan, WeightEncoding,
};
use gbf_test::fixtures::{assert_artifact_valid, assert_bytes_equal, make_tiny_artifact};
use gbf_verify::ternary::{
    TernaryReferenceError, dequantize_reference_scales, pack_reference_scale_values,
    pack_reference_ternary, pack_reference_ternary_values, project_reference_ternary_values,
    quantize_reference_scales, unpack_reference_scale_values, unpack_reference_ternary_values,
};

#[test]
fn ternary_tests_byte_cost_known_values_pin_matrix_orientation() {
    let per_tensor = plan(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerTensor,
        ScaleFormat::Q8_8,
    );
    let per_output_row = plan(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerOutputRow,
        ScaleFormat::Q8_8,
    );
    let per_group = plan(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerGroup(NonZeroU16::new(16).unwrap()),
        ScaleFormat::Q8_8,
    );

    assert_eq!(per_tensor.compute_byte_cost(128, 224).as_u64(), 7_170);
    assert_eq!(per_output_row.compute_byte_cost(128, 224).as_u64(), 7_424);
    assert_eq!(per_output_row.compute_byte_cost(224, 128).as_u64(), 7_616);
    assert_eq!(per_group.compute_byte_cost(128, 224).as_u64(), 10_752);

    let full_two_matrix_expert =
        per_output_row.compute_byte_cost(224, 128) + per_output_row.compute_byte_cost(128, 224);
    assert_eq!(full_two_matrix_expert.as_u64(), 15_040);
}

#[test]
fn ternary_tests_byte_cost_matches_independent_formula_for_plan_matrix() {
    let encodings = [
        WeightEncoding::Ternary2,
        WeightEncoding::SparseTernaryBitplanes,
        WeightEncoding::Binary1,
    ];
    let granularities = [
        ScaleGranularity::PerTensor,
        ScaleGranularity::PerOutputRow,
        ScaleGranularity::PerGroup(NonZeroU16::new(16).unwrap()),
    ];
    let formats = [ScaleFormat::Q8_8, ScaleFormat::Q4_4, ScaleFormat::Pow2];
    let shapes = [(1, 1), (7, 9), (128, 224), (224, 128)];

    for encoding in encodings {
        for granularity in granularities {
            for format in formats {
                let plan = plan(encoding, granularity, format);
                for (rows, cols) in shapes {
                    let expected = expected_byte_cost(encoding, granularity, format, rows, cols);
                    assert_eq!(
                        plan.compute_byte_cost(rows, cols).as_u64(),
                        expected,
                        "encoding={encoding:?} granularity={granularity:?} format={format:?} rows={rows} cols={cols}"
                    );
                    assert_eq!(
                        plan.compute_byte_cost(rows, cols),
                        plan.compute_byte_cost(rows, cols),
                        "byte cost must be deterministic for {plan:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn ternary_tests_known_float_projection_packs_and_unpacks_losslessly() {
    let plan = plan(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerOutputRow,
        ScaleFormat::Q8_8,
    );
    let weights = [
        -0.6, -0.25, 0.0, 0.7, 0.3, -0.8, 0.2, -0.2, 0.9, 0.0, -0.3, -0.9,
    ];
    let thresholds = [0.25, 0.5, 0.1];
    let scales = [64, 384, 512];

    let ternary = project_reference_ternary_values(3, 4, &weights, &thresholds).unwrap();
    let packet = pack_reference_ternary(plan, 3, 4, &ternary, &scales).unwrap();

    assert_eq!(ternary, vec![-1, 0, 0, 1, 0, -1, 0, 0, 1, 0, -1, -1]);
    assert_bytes_equal(packet.weight_bytes(), &[0x42, 0x08, 0xa1]);
    assert_bytes_equal(packet.scale_bytes(), &[0x40, 0x00, 0x80, 0x01, 0x00, 0x02]);
    assert_eq!(
        unpack_reference_ternary_values(plan, 3, 4, packet.weight_bytes()).unwrap(),
        ternary
    );
    assert_eq!(
        unpack_reference_scale_values(plan, 3, 4, packet.scale_bytes()).unwrap(),
        scales
    );
}

#[test]
fn ternary_tests_reference_round_trips_boundary_patterns_for_weight_encodings() {
    let cases = [
        (
            "ternary2_all_zero",
            plan(
                WeightEncoding::Ternary2,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            vec![0; 17],
        ),
        (
            "ternary2_alternating",
            plan(
                WeightEncoding::Ternary2,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            alternating_ternary_values(17),
        ),
        (
            "sparse_all_zero",
            plan(
                WeightEncoding::SparseTernaryBitplanes,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            vec![0; 17],
        ),
        (
            "sparse_alternating",
            plan(
                WeightEncoding::SparseTernaryBitplanes,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            alternating_ternary_values(17),
        ),
        (
            "binary_all_positive",
            plan(
                WeightEncoding::Binary1,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            vec![1; 17],
        ),
        (
            "binary_alternating_signs",
            plan(
                WeightEncoding::Binary1,
                ScaleGranularity::PerTensor,
                ScaleFormat::Q8_8,
            ),
            alternating_binary_values(17),
        ),
    ];

    for (name, plan, values) in cases {
        let bytes = pack_reference_ternary_values(plan, 1, 17, &values)
            .unwrap_or_else(|error| panic!("{name} pack failed: {error}"));
        let unpacked = unpack_reference_ternary_values(plan, 1, 17, &bytes)
            .unwrap_or_else(|error| panic!("{name} unpack failed: {error}"));

        assert_eq!(unpacked, values, "{name} failed to round-trip");
    }

    let binary = plan(
        WeightEncoding::Binary1,
        ScaleGranularity::PerTensor,
        ScaleFormat::Q8_8,
    );
    assert_eq!(
        pack_reference_ternary_values(binary, 1, 1, &[0]).unwrap_err(),
        TernaryReferenceError::BinaryCannotEncodeZero { index: 0 }
    );
}

#[test]
fn ternary_tests_reference_round_trips_randomized_tensors_with_diagnostics() {
    let plans = [
        plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        ),
        plan(
            WeightEncoding::SparseTernaryBitplanes,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        ),
        plan(
            WeightEncoding::Binary1,
            ScaleGranularity::PerTensor,
            ScaleFormat::Q8_8,
        ),
    ];
    let mut rng = Lcg::new(0x05ee_d19b);

    for case_index in 0..100 {
        let rows = 4 + rng.next_u32() % 253;
        let cols = 4 + rng.next_u32() % 253;
        let len = usize::try_from(rows * cols).unwrap();

        for plan in plans {
            let values = random_values_for_plan(&mut rng, plan.encoding, len);
            let bytes = pack_reference_ternary_values(plan, rows, cols, &values).unwrap_or_else(
                |error| {
                    panic!(
                        "pack failed for case={case_index} rows={rows} cols={cols} plan={plan:?}: {error}"
                    )
                },
            );
            let unpacked = unpack_reference_ternary_values(plan, rows, cols, &bytes)
                .unwrap_or_else(|error| {
                    panic!(
                        "unpack failed for case={case_index} rows={rows} cols={cols} plan={plan:?}: {error}"
                    )
                });

            assert_eq!(
                unpacked, values,
                "round-trip mismatch for case={case_index} rows={rows} cols={cols} plan={plan:?}"
            );
        }
    }
}

#[test]
fn ternary_tests_q8_8_scales_round_trip_and_future_scale_formats_are_rejected() {
    let q8_8 = plan(
        WeightEncoding::Ternary2,
        ScaleGranularity::PerOutputRow,
        ScaleFormat::Q8_8,
    );
    let scales = [0.0, 1.5, 255.996_1];
    let quantized = quantize_reference_scales(q8_8, 3, 4, &scales).unwrap();
    let bytes = pack_reference_scale_values(q8_8, 3, 4, &quantized).unwrap();

    assert_eq!(quantized, vec![0, 384, u16::MAX]);
    assert_bytes_equal(&bytes, &[0x00, 0x00, 0x80, 0x01, 0xff, 0xff]);
    assert_eq!(
        unpack_reference_scale_values(q8_8, 3, 4, &bytes).unwrap(),
        quantized
    );
    assert_eq!(
        dequantize_reference_scales(q8_8, 3, 4, &quantized).unwrap(),
        vec![0.0, 1.5, f32::from(u16::MAX) / 256.0]
    );

    for format in [ScaleFormat::Q4_4, ScaleFormat::Pow2] {
        let unsupported = plan(
            WeightEncoding::Ternary2,
            ScaleGranularity::PerTensor,
            format,
        );
        assert_eq!(
            quantize_reference_scales(unsupported, 1, 1, &[1.0]).unwrap_err(),
            TernaryReferenceError::UnsupportedScaleFormat { format }
        );
    }
}

#[test]
fn ternary_tests_tiny_export_canonical_tensors_are_reference_packable() {
    let artifact = make_tiny_artifact();
    assert_artifact_valid(&artifact);

    for entry in artifact.quant().ternary_weight_plans() {
        assert_export_entry_reference_packable(artifact.tensors(), entry);
    }
}

#[test]
fn ternary_tests_artifact_core_hash_is_stable_for_tiny_export() {
    let first = make_tiny_artifact();
    let second = make_tiny_artifact();
    let reconstructed = gbf_artifact::core::ArtifactCore::new(
        first.tensors().to_vec(),
        first.quant().clone(),
        first.sequence_semantics(),
    )
    .unwrap();

    assert_eq!(first.semantic_hash(), second.semantic_hash());
    assert_eq!(first.semantic_hash(), reconstructed.semantic_hash());
}

fn assert_export_entry_reference_packable(tensors: &[CanonicalTensor], entry: &TernaryQuantEntry) {
    let weight = tensor_by_id(tensors, entry.weight.as_ref());
    let scale = tensor_by_id(tensors, entry.scale.as_ref());
    assert_eq!(weight.kind, CanonicalTensorKind::TernaryWeight);
    assert_eq!(scale.kind, CanonicalTensorKind::TernaryScale);

    let dims = weight.layout.shape.dims();
    assert_eq!(
        dims.len(),
        2,
        "ternary weight {} must be a matrix, got shape {dims:?}",
        weight.id
    );
    let rows = dims[0];
    let cols = dims[1];
    let ternary_values = weight
        .payload
        .as_i8_slice()
        .expect("ternary weight payload must be i8");
    let scale_values = scale
        .payload
        .as_u16_slice()
        .expect("ternary scale payload must be u16");

    let packet = pack_reference_ternary(entry.plan, rows, cols, ternary_values, scale_values)
        .unwrap_or_else(|error| {
            panic!(
                "reference packing failed for projection={} rows={rows} cols={cols}: {error}",
                entry.projection
            )
        });
    assert_eq!(
        packet.total_byte_len(),
        entry.plan.compute_byte_cost(rows, cols).as_u64() as usize,
        "packed byte count mismatch for projection={}",
        entry.projection
    );
    assert_eq!(
        unpack_reference_ternary_values(entry.plan, rows, cols, packet.weight_bytes()).unwrap(),
        ternary_values
    );
    assert_eq!(
        unpack_reference_scale_values(entry.plan, rows, cols, packet.scale_bytes()).unwrap(),
        scale_values
    );
}

fn tensor_by_id<'a>(tensors: &'a [CanonicalTensor], id: &str) -> &'a CanonicalTensor {
    tensors
        .iter()
        .find(|tensor| tensor.id.to_string() == id)
        .unwrap_or_else(|| panic!("missing tensor {id}"))
}

fn plan(
    encoding: WeightEncoding,
    scale_granularity: ScaleGranularity,
    scale_format: ScaleFormat,
) -> TernaryWeightPlan {
    TernaryWeightPlan::new(
        encoding,
        scale_granularity,
        scale_format,
        ThresholdPlan::AnnealedGlobalThenPerOutputRow,
    )
}

fn expected_byte_cost(
    encoding: WeightEncoding,
    scale_granularity: ScaleGranularity,
    scale_format: ScaleFormat,
    rows: u32,
    cols: u32,
) -> u64 {
    let elements = u128::from(rows) * u128::from(cols);
    let weight_bytes = match encoding {
        WeightEncoding::Ternary2 => elements.saturating_mul(2).div_ceil(8),
        WeightEncoding::SparseTernaryBitplanes => elements.div_ceil(8).saturating_mul(2),
        WeightEncoding::Binary1 => elements.div_ceil(8),
    };
    let scale_count = match scale_granularity {
        ScaleGranularity::PerTensor => 1,
        ScaleGranularity::PerOutputRow => u128::from(rows),
        ScaleGranularity::PerGroup(group_size) => elements.div_ceil(u128::from(group_size.get())),
    };
    u64::try_from(weight_bytes + scale_count * u128::from(scale_format.byte_len())).unwrap()
}

fn alternating_ternary_values(len: usize) -> Vec<i8> {
    (0..len)
        .map(|index| match index % 3 {
            0 => -1,
            1 => 0,
            _ => 1,
        })
        .collect()
}

fn alternating_binary_values(len: usize) -> Vec<i8> {
    (0..len)
        .map(|index| if index % 2 == 0 { -1 } else { 1 })
        .collect()
}

fn random_values_for_plan(rng: &mut Lcg, encoding: WeightEncoding, len: usize) -> Vec<i8> {
    (0..len)
        .map(|_| match encoding {
            WeightEncoding::Binary1 => {
                if rng.next_u32() & 1 == 0 {
                    -1
                } else {
                    1
                }
            }
            WeightEncoding::Ternary2 | WeightEncoding::SparseTernaryBitplanes => {
                match rng.next_u32() % 3 {
                    0 => -1,
                    1 => 0,
                    _ => 1,
                }
            }
        })
        .collect()
}

struct Lcg {
    state: u64,
}

impl Lcg {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.state >> 32) as u32
    }
}

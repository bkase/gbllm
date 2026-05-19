use gbf_artifact::{Accumulator, CanonicalIntegerThenScale, Q8_8Scale, QuantSpec_S3, WeightQuant};
use proptest::prelude::*;

proptest! {
    #[test]
    fn quant_spec_ternary2_round_trips_with_pinned_reduction_order(row_scale in any::<u16>(), threshold in any::<u16>()) {
        let quant = WeightQuant::Ternary2 {
            row_scale: Q8_8Scale(row_scale),
            threshold: Q8_8Scale(threshold),
            accumulator: Accumulator::I32,
            reduction_order: CanonicalIntegerThenScale::HardenedReductionPolicyV1,
        };

        let encoded = serde_json::to_string(&quant).expect("weight quant serializes");
        let decoded: WeightQuant = serde_json::from_str(&encoded).expect("weight quant decodes");

        prop_assert_eq!(decoded, quant);
        let WeightQuant::Ternary2 { reduction_order, accumulator, .. } = decoded else {
            unreachable!("constructed ternary2");
        };
        prop_assert_eq!(reduction_order, CanonicalIntegerThenScale::HardenedReductionPolicyV1);
        prop_assert_eq!(accumulator, Accumulator::I32);
    }
}

#[test]
fn quant_spec_weight_quant_map_round_trips() {
    let spec = QuantSpec_S3::new(std::collections::BTreeMap::from([(
        gbf_artifact::CanonicalTensorId::new("tensor.linear.weight").unwrap(),
        WeightQuant::Fp32,
    )]));

    let encoded = serde_json::to_string(&spec).expect("quant spec serializes");
    let decoded: QuantSpec_S3 = serde_json::from_str(&encoded).expect("quant spec decodes");

    assert_eq!(decoded, spec);
}

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::{Hash256, PackerVersion};
use gbf_hw::calibration::CalibrationConfidenceClass;
use gbf_policy::calibration::{
    BootstrapCalibrationBundle, CalibrationBundle, CalibrationBundleSet, CalibrationLayer,
    CalibrationSetRef, MeasurementBlob, ValidityEnvelope,
};
use gbf_policy::risk::CalibrationConfidenceRequirement;

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn hash_json(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}

fn measurement_blob_fixture() -> MeasurementBlob {
    MeasurementBlob {
        schema: "calibration_measurement.v1".to_owned(),
        payload_hash: hash(0x40),
    }
}

fn calibration_bundle_fixture(layer: CalibrationLayer) -> CalibrationBundle {
    CalibrationBundle {
        layer,
        target_profile_hash: hash(0x10),
        kernel_set_hash: hash(0x20),
        packer_version: PackerVersion::new(1, 2, 3),
        calibration_schema_hash: hash(0x30),
        validity_envelope: ValidityEnvelope::default(),
        confidence: CalibrationConfidenceClass::Reasonable,
        measurements: Some(measurement_blob_fixture()),
    }
}

fn expected_bundle_json(layer: CalibrationLayer) -> serde_json::Value {
    serde_json::json!({
        "layer": {"kind": layer.as_str()},
        "target_profile_hash": hash_json(0x10),
        "kernel_set_hash": hash_json(0x20),
        "packer_version": "1.2.3",
        "calibration_schema_hash": hash_json(0x30),
        "validity_envelope": {"future_fields": {}},
        "confidence": {"kind": "Reasonable"},
        "measurements": {
            "schema": "calibration_measurement.v1",
            "payload_hash": hash_json(0x40)
        }
    })
}

fn expected_resolved_ref_json() -> serde_json::Value {
    serde_json::json!({
        "platform": "platform.bootstrap-dmg-mbc5",
        "kernel": "kernel.bootstrap-dmg-mbc5",
        "runtime": "runtime.bootstrap-dmg-mbc5"
    })
}

#[test]
fn calibration_layer_round_trip_all_variants() {
    for layer in CalibrationLayer::all() {
        let encoded = serde_json::to_string(&layer).expect("layer serializes");
        let decoded: CalibrationLayer = serde_json::from_str(&encoded).expect("layer deserializes");

        assert_eq!(decoded, layer);
        assert_eq!(
            serde_json::to_value(layer).expect("layer serializes"),
            serde_json::json!({"kind": layer.as_str()})
        );
    }
}

#[test]
fn calibration_bundle_round_trip() {
    let bundle = calibration_bundle_fixture(CalibrationLayer::Kernel);
    let encoded = serde_json::to_string(&bundle).expect("bundle serializes");
    let decoded: CalibrationBundle = serde_json::from_str(&encoded).expect("bundle deserializes");

    assert_eq!(decoded, bundle);
    assert_eq!(
        serde_json::to_value(&bundle).expect("bundle serializes"),
        expected_bundle_json(CalibrationLayer::Kernel)
    );
}

#[test]
fn calibration_bundle_set_round_trip() {
    let bundles = BTreeMap::from([
        (
            CalibrationLayer::Platform,
            calibration_bundle_fixture(CalibrationLayer::Platform),
        ),
        (
            CalibrationLayer::Kernel,
            calibration_bundle_fixture(CalibrationLayer::Kernel),
        ),
        (
            CalibrationLayer::Runtime,
            calibration_bundle_fixture(CalibrationLayer::Runtime),
        ),
    ]);
    let set = CalibrationBundleSet {
        resolved_ref: BootstrapCalibrationBundle::dmg_mbc5_ref(),
        bundles,
    };
    let encoded = serde_json::to_string(&set).expect("set serializes");
    let decoded: CalibrationBundleSet = serde_json::from_str(&encoded).expect("set deserializes");

    assert_eq!(decoded, set);
    assert_eq!(
        serde_json::to_value(&set).expect("set serializes"),
        serde_json::json!({
            "bundles": {
                "Kernel": expected_bundle_json(CalibrationLayer::Kernel),
                "Platform": expected_bundle_json(CalibrationLayer::Platform),
                "Runtime": expected_bundle_json(CalibrationLayer::Runtime)
            },
            "resolved_ref": expected_resolved_ref_json()
        })
    );
}

#[test]
fn calibration_set_ref_round_trip() {
    let reference = CalibrationSetRef {
        set_hash: hash(0x55),
        layers: BTreeSet::from([CalibrationLayer::Kernel, CalibrationLayer::Platform]),
    };
    let encoded = serde_json::to_string(&reference).expect("ref serializes");
    let decoded: CalibrationSetRef = serde_json::from_str(&encoded).expect("ref deserializes");

    assert_eq!(decoded, reference);
    assert_eq!(
        serde_json::to_value(reference).expect("ref serializes"),
        serde_json::json!({
            "set_hash": hash_json(0x55),
            "layers": [
                {"kind": "Platform"},
                {"kind": "Kernel"}
            ]
        })
    );
}

#[test]
fn validity_envelope_round_trip_empty() {
    let envelope = ValidityEnvelope::default();
    let encoded = serde_json::to_string(&envelope).expect("envelope serializes");
    let decoded: ValidityEnvelope = serde_json::from_str(&encoded).expect("envelope deserializes");

    assert_eq!(decoded, envelope);
    assert_eq!(
        serde_json::to_value(envelope).expect("envelope serializes"),
        serde_json::json!({"future_fields": {}})
    );
}

#[test]
fn measurement_blob_round_trip() {
    let blob = measurement_blob_fixture();
    let encoded = serde_json::to_string(&blob).expect("measurement blob serializes");
    let decoded: MeasurementBlob =
        serde_json::from_str(&encoded).expect("measurement blob deserializes");

    assert_eq!(decoded, blob);
    assert_eq!(
        serde_json::to_value(blob).expect("measurement blob serializes"),
        serde_json::json!({
            "schema": "calibration_measurement.v1",
            "payload_hash": hash_json(0x40)
        })
    );
}

#[test]
fn calibration_bundle_set_serializes_with_sorted_keys() {
    let set = BootstrapCalibrationBundle::new(hash(0xaa));
    let encoded = serde_json::to_string(&set).expect("set serializes");
    let kernel = encoded.find("\"Kernel\"").expect("kernel key exists");
    let platform = encoded.find("\"Platform\"").expect("platform key exists");
    let runtime = encoded.find("\"Runtime\"").expect("runtime key exists");

    assert!(kernel < platform);
    assert!(platform < runtime);
}

#[test]
fn calibration_bundle_rejects_unknown_field() {
    let mut value = expected_bundle_json(CalibrationLayer::Kernel);
    value["unexpected"] = serde_json::json!(true);

    assert!(serde_json::from_value::<CalibrationBundle>(value).is_err());
}

#[test]
fn calibration_bundle_set_rejects_unknown_field() {
    let value = serde_json::json!({
        "bundles": {},
        "unexpected": true
    });

    assert!(serde_json::from_value::<CalibrationBundleSet>(value).is_err());
}

#[test]
fn calibration_bundle_set_rejects_unknown_layer_key() {
    let value = serde_json::json!({
        "bundles": {
            "Firmware": expected_bundle_json(CalibrationLayer::Platform)
        }
    });

    assert!(serde_json::from_value::<CalibrationBundleSet>(value).is_err());
}

#[test]
fn calibration_bundle_set_rejects_mismatched_bundle_layer() {
    let value = serde_json::json!({
        "bundles": {
            "Kernel": expected_bundle_json(CalibrationLayer::Platform)
        }
    });

    assert!(serde_json::from_value::<CalibrationBundleSet>(value).is_err());
}

#[test]
fn calibration_layer_rejects_unknown_kind() {
    assert!(
        serde_json::from_value::<CalibrationLayer>(serde_json::json!({"kind": "Firmware"}))
            .is_err()
    );
}

#[test]
fn validity_envelope_rejects_unknown_field() {
    let value = serde_json::json!({
        "future_fields": {},
        "unexpected": true
    });

    assert!(serde_json::from_value::<ValidityEnvelope>(value).is_err());
}

#[test]
fn validity_envelope_rejects_unknown_nested_future_field() {
    let value = serde_json::json!({
        "future_fields": {
            "unexpected": true
        }
    });

    assert!(serde_json::from_value::<ValidityEnvelope>(value).is_err());
}

#[test]
fn measurement_blob_rejects_unknown_field() {
    let mut value =
        serde_json::to_value(measurement_blob_fixture()).expect("measurement blob serializes");
    value["unexpected"] = serde_json::json!(true);

    assert!(serde_json::from_value::<MeasurementBlob>(value).is_err());
}

#[test]
fn calibration_set_ref_rejects_unknown_field() {
    let mut value = serde_json::to_value(CalibrationSetRef {
        set_hash: hash(0x55),
        layers: BTreeSet::from([CalibrationLayer::Kernel]),
    })
    .expect("ref serializes");
    value["unexpected"] = serde_json::json!(true);

    assert!(serde_json::from_value::<CalibrationSetRef>(value).is_err());
}

#[test]
fn bootstrap_factory_emits_none_confidence_for_all_layers() {
    let set = BootstrapCalibrationBundle::new(hash(0xaa));

    assert_eq!(set.bundles.len(), CalibrationLayer::all().len());
    for layer in CalibrationLayer::all() {
        let bundle = set.bundles.get(&layer).expect("bootstrap layer exists");
        assert_eq!(bundle.layer, layer);
        assert_eq!(bundle.confidence, CalibrationConfidenceClass::None);
        assert_eq!(bundle.measurements, None);
    }
}

#[test]
fn bootstrap_factory_is_deterministic() {
    let left = serde_json::to_string(&BootstrapCalibrationBundle::new(hash(0xaa)))
        .expect("left serializes");
    let right = serde_json::to_string(&BootstrapCalibrationBundle::new(hash(0xaa)))
        .expect("right serializes");

    assert_eq!(left, right);
}

#[test]
fn bootstrap_factory_uses_packer_version_1_0_0() {
    let set = BootstrapCalibrationBundle::new(hash(0xaa));

    for bundle in set.bundles.values() {
        assert_eq!(bundle.packer_version, PackerVersion::new(1, 0, 0));
    }
}

#[test]
fn measurement_none_serializes_as_json_null() {
    let mut bundle = calibration_bundle_fixture(CalibrationLayer::Platform);
    bundle.measurements = None;

    assert_eq!(
        serde_json::to_value(bundle).expect("bundle serializes")["measurements"],
        serde_json::Value::Null
    );
}

#[test]
fn confidence_class_distinct_from_requirement() {
    assert_ne!(
        TypeId::of::<CalibrationConfidenceClass>(),
        TypeId::of::<CalibrationConfidenceRequirement>()
    );
    assert_ne!(
        serde_json::to_value(CalibrationConfidenceClass::None).expect("class serializes"),
        serde_json::to_value(CalibrationConfidenceRequirement::NoMinimumConfidence)
            .expect("requirement serializes")
    );
}

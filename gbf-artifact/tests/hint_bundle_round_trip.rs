use gbf_artifact::hint_bundle::{
    BuildConstraintEntry, BuildConstraints, EvidenceScope, HINT_BUNDLE_HASH_DOMAIN_SEPARATOR,
    HintBundle,
};
use gbf_artifact::lowerings::LoweringShardId;
use gbf_foundation::{Hash256, LayerId, TargetFamilyId, WorkloadId};
use gbf_policy::compile::{
    CompileKnobId, CompileKnobPath, ConstraintValue, EvidenceRef, FieldPath, ObservabilityMode,
    PlacementProfile, SelectorPath,
};
use sha2::{Digest, Sha256};

fn assert_round_trip<T>(value: &T)
where
    T: std::fmt::Debug + PartialEq + serde::Serialize + serde::de::DeserializeOwned,
{
    let encoded = serde_json::to_string(value).expect("value serializes");
    let decoded: T = serde_json::from_str(&encoded).expect("value deserializes");
    let reencoded = serde_json::to_string(&decoded).expect("decoded value serializes");

    assert_eq!(&decoded, value);
    assert_eq!(reencoded, encoded);
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

fn constraint_entry(knob: CompileKnobId, value: ConstraintValue) -> BuildConstraintEntry {
    BuildConstraintEntry {
        knob,
        path: None,
        value,
        evidence: Vec::new(),
        scope: EvidenceScope::WholeArtifact,
    }
}

fn empty_hint_bundle_fixture() -> HintBundle {
    HintBundle::empty()
}

fn build_constraint_entry_fixture() -> BuildConstraintEntry {
    constraint_entry(
        CompileKnobId::Placement,
        ConstraintValue::PlacementProfile {
            value: PlacementProfile::Budgeted,
        },
    )
}

#[test]
fn hint_bundle_round_trip_empty_fixture() {
    let bundle = empty_hint_bundle_fixture();

    assert_round_trip(&bundle);
    assert_eq!(bundle, HintBundle::empty());
}

#[test]
fn hint_bundle_round_trip_with_constraint() {
    let bundle = HintBundle {
        constraints: BuildConstraints {
            entries: vec![build_constraint_entry_fixture()],
        },
        ..HintBundle::empty()
    };

    assert_round_trip(&bundle);
    assert_eq!(bundle.constraints.entries.len(), 1);
}

#[test]
fn build_constraints_round_trip() {
    let constraints = BuildConstraints {
        entries: vec![build_constraint_entry_fixture()],
    };

    assert_round_trip(&constraints);
}

#[test]
fn build_constraint_entry_round_trip() {
    let entry = BuildConstraintEntry {
        path: Some(CompileKnobPath {
            knob: CompileKnobId::Placement,
            selector: Some(SelectorPath("bank[0]".to_owned())),
            field: Some(FieldPath("profile".to_owned())),
        }),
        evidence: vec![EvidenceRef {
            kind: "fixture".to_owned(),
            reference: "hint-fixture".to_owned(),
            hash: Some(hash(0x33)),
        }],
        scope: EvidenceScope::LayerScoped {
            layer: LayerId::new(2),
        },
        ..build_constraint_entry_fixture()
    };

    assert_round_trip(&entry);
}

#[test]
fn evidence_scope_round_trip_all_variants() {
    let variants = [
        EvidenceScope::WholeArtifact,
        EvidenceScope::LayerScoped {
            layer: LayerId::new(7),
        },
        EvidenceScope::TargetFamily {
            family: TargetFamilyId::from("DMG"),
        },
        EvidenceScope::WorkloadScoped {
            workload: WorkloadId::from("smoke"),
        },
        EvidenceScope::LoweringScoped {
            shard: LoweringShardId("weight.layer0".to_owned()),
        },
    ];

    for scope in variants {
        assert_round_trip(&scope);
    }
}

#[test]
fn constraint_value_round_trip_all_variants() {
    let variants = [
        ConstraintValue::PlacementProfile {
            value: PlacementProfile::Budgeted,
        },
        ConstraintValue::ObservabilityMode {
            value: ObservabilityMode::Flexible,
        },
        ConstraintValue::U16 { value: 17 },
        ConstraintValue::U32 { value: 65_537 },
        ConstraintValue::Bool { value: true },
        ConstraintValue::Text {
            value: "profile.default".to_owned(),
        },
    ];

    for value in variants {
        assert_round_trip(&value);
    }
}

#[test]
fn hint_bundle_rejects_unknown_top_level_field() {
    let mut value = serde_json::to_value(empty_hint_bundle_fixture()).expect("bundle json value");
    value["unexpected"] = serde_json::json!(true);

    let err = serde_json::from_value::<HintBundle>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn build_constraint_entry_rejects_unknown_field() {
    let mut value =
        serde_json::to_value(build_constraint_entry_fixture()).expect("entry json value");
    value["unexpected"] = serde_json::json!(true);

    let err =
        serde_json::from_value::<BuildConstraintEntry>(value).expect_err("unknown field rejects");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn evidence_scope_rejects_unknown_kind() {
    let err = serde_json::from_value::<EvidenceScope>(serde_json::json!({
        "kind": "ModelLineage"
    }))
    .expect_err("unknown kind rejects");

    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn constraint_value_rejects_unknown_kind() {
    let err = serde_json::from_value::<ConstraintValue>(serde_json::json!({
        "kind": "BoundedQ8_8",
        "value": 256
    }))
    .expect_err("unknown kind rejects");

    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn canonical_hash_uses_domain_separator() {
    let bundle = empty_hint_bundle_fixture();
    let mut hasher = Sha256::new();
    hasher.update(HINT_BUNDLE_HASH_DOMAIN_SEPARATOR);
    hasher.update(bundle.canonical_json_bytes());
    let expected = Hash256::from_bytes(hasher.finalize().into());

    assert_eq!(bundle.compute_canonical_hash(), expected);
    assert_eq!(
        HINT_BUNDLE_HASH_DOMAIN_SEPARATOR,
        b"gbf:gbf-artifact:HintBundle:hint_bundle:1.0.0\0"
    );
}

#[test]
fn canonical_hash_changes_with_constraint() {
    let empty = empty_hint_bundle_fixture();
    let constrained = HintBundle {
        constraints: BuildConstraints {
            entries: vec![build_constraint_entry_fixture()],
        },
        ..HintBundle::empty()
    };

    assert_ne!(
        empty.compute_canonical_hash(),
        constrained.compute_canonical_hash()
    );
}

#[test]
fn canonical_hash_byte_stable_for_empty_bundle() {
    let first = empty_hint_bundle_fixture().compute_canonical_hash();
    let second = empty_hint_bundle_fixture().compute_canonical_hash();

    assert_eq!(first, second);
}

#[test]
fn empty_hint_bundle_canonical_hash_is_pinned() {
    assert_eq!(
        empty_hint_bundle_fixture().compute_canonical_hash(),
        Hash256::from_bytes([
            0x12, 0xfb, 0xa3, 0x64, 0xfa, 0x05, 0x9d, 0x08, 0x1a, 0x33, 0x95, 0xd1, 0xf6, 0xa3,
            0x5c, 0xc5, 0xf1, 0x16, 0x4e, 0xd4, 0x19, 0x58, 0xfc, 0x31, 0x92, 0x11, 0xa9, 0x59,
            0x21, 0x2b, 0x59, 0xe8,
        ])
    );
}

#[test]
fn constraint_entries_preserve_declaration_order() {
    let constraints = BuildConstraints {
        entries: vec![
            constraint_entry(CompileKnobId::Schedule, ConstraintValue::U32 { value: 1 }),
            constraint_entry(CompileKnobId::Placement, ConstraintValue::U32 { value: 2 }),
            constraint_entry(CompileKnobId::Storage, ConstraintValue::U32 { value: 3 }),
        ],
    };

    let encoded = serde_json::to_value(&constraints).expect("constraints json value");
    let decoded: BuildConstraints =
        serde_json::from_value(encoded.clone()).expect("constraints deserialize");

    assert_eq!(
        decoded
            .entries
            .iter()
            .map(|entry| entry.knob)
            .collect::<Vec<_>>(),
        vec![
            CompileKnobId::Schedule,
            CompileKnobId::Placement,
            CompileKnobId::Storage,
        ]
    );
    assert_eq!(
        encoded["entries"]
            .as_array()
            .expect("entries is an array")
            .iter()
            .map(|entry| entry["knob"]["kind"].as_str().expect("knob kind"))
            .collect::<Vec<_>>(),
        vec!["Schedule", "Placement", "Storage"]
    );
}

#[test]
fn path_none_serializes_as_json_null() {
    let value = serde_json::to_value(build_constraint_entry_fixture()).expect("entry json value");

    assert_eq!(value["path"], serde_json::Value::Null);
}

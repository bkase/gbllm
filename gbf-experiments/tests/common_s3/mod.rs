#![allow(dead_code)]

pub mod fixtures {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gbf_experiments::s3::schema::S3BuildKind;
    use gbf_foundation::{Hash256, sha256};
    use gbf_train::student::{
        HardTernaryStudentModel, StudentStorageFingerprint, StudentWeightFingerprint,
    };

    static NEXT_STUDENT_STORAGE_ID: AtomicUsize = AtomicUsize::new(30_000);

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Toy0S3Model {
        pub seed: u64,
        pub vocab_size: usize,
        pub hidden_width: usize,
    }

    impl Toy0S3Model {
        pub fn logits(&self) -> Vec<f32> {
            vec![0.0; self.vocab_size]
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct FixedKnFixture {
        pub order: usize,
        pub discount: f64,
        pub validation_text: &'static str,
        pub expected_bpc_char: f64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct MockArtifact {
        pub build_kind: S3BuildKind,
        pub payload: Vec<u8>,
        pub self_hash: Hash256,
    }

    #[derive(Debug, PartialEq)]
    pub struct ToyHardTernaryStudent {
        pub weights: Vec<f32>,
        pub requires_grad: bool,
        storage_identity: usize,
    }

    impl ToyHardTernaryStudent {
        pub fn new(weights: Vec<f32>, requires_grad: bool) -> Self {
            Self {
                weights,
                requires_grad,
                storage_identity: next_student_storage_id(),
            }
        }
    }

    impl Clone for ToyHardTernaryStudent {
        fn clone(&self) -> Self {
            Self {
                weights: self.weights.clone(),
                requires_grad: self.requires_grad,
                storage_identity: next_student_storage_id(),
            }
        }
    }

    impl HardTernaryStudentModel for ToyHardTernaryStudent {
        fn detach_for_student(&mut self) {
            self.requires_grad = false;
            self.storage_identity = next_student_storage_id();
        }

        fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
            StudentWeightFingerprint::new(weight_bytes(&self.weights)).unwrap()
        }

        fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
            let mut bytes = Vec::from("s3-toy-hard-ternary-student:f32:rank1:");
            bytes.extend_from_slice(&self.weights.len().to_le_bytes());
            bytes.extend_from_slice(&weight_bytes(&self.weights));
            StudentStorageFingerprint::new(bytes).unwrap()
        }

        fn student_storage_identity(&self) -> usize {
            self.storage_identity
        }

        fn student_requires_grad(&self) -> bool {
            self.requires_grad
        }
    }

    pub fn toy0_model_factory(seed: u64) -> Toy0S3Model {
        Toy0S3Model {
            seed,
            vocab_size: 80,
            hidden_width: 16,
        }
    }

    pub fn fixed_kn_fixture() -> FixedKnFixture {
        FixedKnFixture {
            order: 5,
            discount: 0.75,
            validation_text: "Once upon a byte.\n",
            expected_bpc_char: 4.25,
        }
    }

    pub fn mock_artifact(build_kind: S3BuildKind, payload: impl AsRef<[u8]>) -> MockArtifact {
        let payload = payload.as_ref().to_vec();
        let mut preimage = Vec::new();
        preimage.extend_from_slice(format!("{build_kind:?}").as_bytes());
        preimage.push(0);
        preimage.extend_from_slice(&payload);
        MockArtifact {
            build_kind,
            payload,
            self_hash: sha256(preimage),
        }
    }

    pub fn build_kind_matrix() -> impl Iterator<Item = S3BuildKind> {
        S3BuildKind::ALL.into_iter()
    }

    fn next_student_storage_id() -> usize {
        NEXT_STUDENT_STORAGE_ID.fetch_add(1, Ordering::Relaxed)
    }

    fn weight_bytes(weights: &[f32]) -> Vec<u8> {
        weights
            .iter()
            .flat_map(|weight| weight.to_le_bytes())
            .collect()
    }
}

pub mod helpers {
    pub mod ndjson_capture {
        use gbf_experiments::s1::schema::S1CanonicalJson;
        use serde::Serialize;
        use serde_json::Value;

        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct NdjsonCaptureSink {
            entries: Vec<Value>,
        }

        impl NdjsonCaptureSink {
            pub fn new() -> Self {
                Self::default()
            }

            pub fn push<T: Serialize>(&mut self, value: &T) {
                let value = serde_json::to_value(value).expect("NDJSON fixture value encodes");
                self.entries.push(value);
            }

            pub fn entries(&self) -> &[Value] {
                &self.entries
            }

            pub fn to_bytes(&self) -> Vec<u8> {
                let mut bytes = Vec::new();
                for entry in &self.entries {
                    bytes.extend_from_slice(
                        &S1CanonicalJson::value_to_vec(entry)
                            .expect("NDJSON fixture value canonicalizes"),
                    );
                    bytes.push(b'\n');
                }
                bytes
            }

            pub fn parsed_lines(&self) -> Vec<Value> {
                self.to_bytes()
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| serde_json::from_slice(line).expect("NDJSON line parses"))
                    .collect()
            }
        }
    }

    pub mod tracing_capture_s3 {
        use crate::common::tracing_capture::{
            TraceCapture, TracingEvent, captured_events, with_trace_capture,
        };

        pub type S3TraceCapture = TraceCapture;

        pub fn capture_events<R>(f: impl FnOnce() -> R) -> (R, Vec<TracingEvent>) {
            let capture = S3TraceCapture::default();
            let result = with_trace_capture(&capture, f);
            (result, captured_events(&capture))
        }

        pub fn events_to_ndjson(events: &[TracingEvent]) -> Vec<u8> {
            let mut bytes = Vec::new();
            for event in events {
                serde_json::to_writer(&mut bytes, &event_to_json(event))
                    .expect("trace event encodes as JSON");
                bytes.push(b'\n');
            }
            bytes
        }

        pub fn assert_event_emitted(events: &[TracingEvent], name: &str) {
            assert!(
                events.iter().any(|event| event.name == name),
                "expected event {name:?} to be emitted; saw {:?}",
                events.iter().map(|event| &event.name).collect::<Vec<_>>()
            );
        }

        fn event_to_json(event: &TracingEvent) -> serde_json::Value {
            serde_json::json!({
                "name": event.name,
                "level": event.level,
                "fields": event.fields,
            })
        }
    }
}

pub mod proptest_strategies_s3;

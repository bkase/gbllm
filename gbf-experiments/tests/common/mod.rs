#![allow(dead_code)]

pub mod assertions {
    use pretty_assertions::assert_eq;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CanonicalTensor {
        pub name: String,
        pub dtype: String,
        pub shape: Vec<u64>,
        pub bytes: Vec<u8>,
    }

    pub fn assert_canonical_json_byte_eq(a: &[u8], b: &[u8]) {
        let left: Value = serde_json::from_slice(a).expect("left JSON must parse");
        let right: Value = serde_json::from_slice(b).expect("right JSON must parse");
        assert_eq!(left, right, "canonical JSON values differ");
        assert_eq!(a, b, "canonical JSON bytes differ");
    }

    pub fn canonical_json_byte_eq(a: &[u8], b: &[u8]) {
        assert_canonical_json_byte_eq(a, b);
    }

    pub fn assert_self_hash_excludes_field(
        value: &Value,
        excluded_field: &str,
        replacement: Value,
    ) {
        let mut changed = value.clone();
        let object = changed
            .as_object_mut()
            .expect("self-hash helper expects a JSON object");
        assert!(
            object.contains_key(excluded_field),
            "excluded field {excluded_field:?} is missing"
        );
        object.insert(excluded_field.to_owned(), replacement);

        assert_eq!(
            hash_without_field(value, excluded_field),
            hash_without_field(&changed, excluded_field),
            "self hash changed after mutating excluded field {excluded_field:?}"
        );
    }

    pub fn self_hash_excludes_field(value: &Value, excluded_field: &str, replacement: Value) {
        assert_self_hash_excludes_field(value, excluded_field, replacement);
    }

    pub fn phase_entry_invariants_assert(
        value: &Value,
        expected_from: Option<&str>,
        expected_to: &str,
    ) {
        let object = value
            .as_object()
            .expect("phase entry invariant helper expects a JSON object");
        assert_eq!(
            object.get("event").and_then(Value::as_str),
            Some("phase_transition")
        );
        assert_eq!(object.get("to").and_then(Value::as_str), Some(expected_to));
        assert_eq!(
            object.get("from").and_then(Value::as_str),
            expected_from,
            "phase transition source differed"
        );
        let step = object
            .get("step")
            .and_then(Value::as_u64)
            .expect("phase entry must carry a numeric step");
        assert!(
            step < u64::MAX,
            "phase step must leave room for later events"
        );
    }

    /// Assert the fixture-local tensor-set hash is stable under input reordering.
    ///
    /// This helper intentionally uses a test-scaffolding domain prefix and simple
    /// string dtype names. It is not the production
    /// `gbf_artifact::tensor::canonical_tensor_payload_hash` contract.
    pub fn assert_canonical_tensor_payload_hash_invariant(tensors: &[CanonicalTensor]) {
        let mut reversed = tensors.to_vec();
        reversed.reverse();
        assert_eq!(
            canonical_tensor_payload_hash(tensors),
            canonical_tensor_payload_hash(&reversed),
            "tensor payload hash must be independent of input order"
        );
    }

    pub fn assert_no_nondeterministic_field(json: &Value) {
        const FORBIDDEN: &[&str] = &[
            "created_at",
            "mtime",
            "nonce",
            "pid",
            "random",
            "timestamp",
            "uuid",
            "wall_clock",
        ];

        match json {
            Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        !FORBIDDEN.contains(&key.as_str()),
                        "nondeterministic field {key:?} is not allowed"
                    );
                    assert_no_nondeterministic_field(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    assert_no_nondeterministic_field(value);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    /// Compute the fixture-local tensor-set hash used by integration-test
    /// scaffolding. Production S1 checkpoint and ablation comparisons must use
    /// `gbf_artifact::tensor::canonical_tensor_payload_hash` instead.
    pub fn canonical_tensor_payload_hash(tensors: &[CanonicalTensor]) -> [u8; 32] {
        let mut sorted = tensors.to_vec();
        sorted.sort_by(|left, right| left.name.cmp(&right.name));

        let mut hasher = Sha256::new();
        hasher.update(b"gbf-experiments.s1.canonical-tensor-set.v0");
        for tensor in sorted {
            update_len_prefixed(&mut hasher, tensor.name.as_bytes());
            update_len_prefixed(&mut hasher, tensor.dtype.as_bytes());
            hasher.update((tensor.shape.len() as u64).to_le_bytes());
            for dim in tensor.shape {
                hasher.update(dim.to_le_bytes());
            }
            update_len_prefixed(&mut hasher, &tensor.bytes);
        }
        hasher.finalize().into()
    }

    fn hash_without_field(value: &Value, excluded_field: &str) -> [u8; 32] {
        let mut stripped = value.clone();
        stripped
            .as_object_mut()
            .expect("self-hash helper expects a JSON object")
            .remove(excluded_field);
        let encoded = serde_json::to_vec(&stripped).expect("JSON value must encode");
        Sha256::digest(encoded).into()
    }

    fn update_len_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
}

pub mod fixtures {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use gbf_model::qat::{RouterForwardOptions, RouterShape, Top1RouterQat};

    #[derive(Debug, Eq, PartialEq)]
    pub struct TinyCorpus {
        pub name: &'static str,
        pub bytes: &'static [u8],
        pub token_count: usize,
    }

    #[derive(Debug, Eq, PartialEq)]
    pub struct TinyCorpusS2 {
        pub inherited_s1: &'static TinyCorpus,
        pub train_stub: TinyCorpus,
        pub eval_stub: TinyCorpus,
        pub manifest_path: &'static str,
    }

    #[derive(Debug, Eq, PartialEq)]
    pub struct HandCountedNgram {
        pub order: usize,
        pub counts: BTreeMap<&'static [u8], u64>,
    }

    pub trait ProbabilityProvider {
        fn logits(&self, vocab_size: usize) -> Vec<f32>;
        fn state_width(&self) -> usize;
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct UniformLogitsModel;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ZeroStateModel;

    pub fn tiny_corpus() -> &'static TinyCorpus {
        static CORPUS: OnceLock<TinyCorpus> = OnceLock::new();
        CORPUS.get_or_init(|| TinyCorpus {
            name: "s1-tiny-corpus-v0",
            bytes: b"abracadabra\n",
            token_count: 12,
        })
    }

    pub fn hand_counted_ngram() -> &'static HandCountedNgram {
        static NGRAM: OnceLock<HandCountedNgram> = OnceLock::new();
        NGRAM.get_or_init(|| {
            let mut counts = BTreeMap::new();
            counts.insert(&b"ab"[..], 2);
            counts.insert(&b"br"[..], 2);
            counts.insert(&b"ra"[..], 2);
            counts.insert(&b"ac"[..], 1);
            counts.insert(&b"ca"[..], 1);
            counts.insert(&b"ad"[..], 1);
            counts.insert(&b"da"[..], 1);
            counts.insert(&b"a\n"[..], 1);
            HandCountedNgram { order: 2, counts }
        })
    }

    pub fn fixture_uniform_logits_model() -> UniformLogitsModel {
        UniformLogitsModel
    }

    pub fn fixture_zero_state_model() -> ZeroStateModel {
        ZeroStateModel
    }

    impl ProbabilityProvider for UniformLogitsModel {
        fn logits(&self, vocab_size: usize) -> Vec<f32> {
            vec![0.0; vocab_size]
        }

        fn state_width(&self) -> usize {
            1
        }
    }

    impl ProbabilityProvider for ZeroStateModel {
        fn logits(&self, vocab_size: usize) -> Vec<f32> {
            vec![0.0; vocab_size]
        }

        fn state_width(&self) -> usize {
            0
        }
    }

    pub fn tiny_corpus_s2() -> &'static TinyCorpus {
        &tiny_corpus_s2_fixture().train_stub
    }

    pub fn tiny_corpus_s2_fixture() -> &'static TinyCorpusS2 {
        static CORPUS: OnceLock<TinyCorpus> = OnceLock::new();
        static FIXTURE: OnceLock<TinyCorpusS2> = OnceLock::new();
        let train_stub = CORPUS.get_or_init(|| {
            tracing::debug!(
                event_name = "fixture_loaded",
                name = "s2-tinystories-stub-with-eval-split",
                path = "gbf-experiments/tests/fixtures/tiny_corpus_s2/manifest.toml",
                expected_bytes_sha = "fixture-local"
            );
            TinyCorpus {
                name: "s2-tinystories-stub-with-eval-split",
                bytes: b"Once upon a byte.\nEval bytes follow.\n",
                token_count: 37,
            }
        });
        FIXTURE.get_or_init(|| TinyCorpusS2 {
            inherited_s1: tiny_corpus(),
            train_stub: TinyCorpus {
                name: train_stub.name,
                bytes: b"Once upon a byte.\n",
                token_count: 18,
            },
            eval_stub: TinyCorpus {
                name: "s2-tinystories-eval-stub",
                bytes: b"Eval bytes follow.\n",
                token_count: 19,
            },
            manifest_path: "gbf-experiments/tests/fixtures/tiny_corpus_s2/manifest.toml",
        })
    }

    pub mod synthetic_router {
        use super::*;

        #[derive(Clone, Debug)]
        pub struct SyntheticRouterFixture {
            pub router: Top1RouterQat,
            pub input: Vec<f32>,
            pub previous_distribution: Option<Vec<f32>>,
            pub options: RouterForwardOptions,
        }

        pub fn four_experts() -> SyntheticRouterFixture {
            let shape = RouterShape::new(2, 4, 1).expect("four-expert router shape is valid");
            let router = Top1RouterQat::new(
                shape,
                vec![1.0, -1.0],
                None,
                vec![0.4, 0.2, -0.1, -0.3],
                None,
            )
            .expect("four-expert router fixture is valid");
            SyntheticRouterFixture {
                router,
                input: vec![0.5, -0.25],
                previous_distribution: Some(vec![0.25, 0.25, 0.25, 0.25]),
                options: RouterForwardOptions::hard_top1(4),
            }
        }

        pub fn soft_top1_dispatch() -> SyntheticRouterFixture {
            let mut fixture = four_experts();
            fixture.options = RouterForwardOptions::soft_top1(4);
            fixture
        }
    }
}

pub mod helpers {
    pub mod phase_log_capture {
        use serde::{Deserialize, Serialize};
        use serde_json::Value;

        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub struct PhaseLogEntry {
            pub event: String,
            pub from: Option<String>,
            pub to: String,
            pub step: u64,
        }

        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct PhaseLogCapture {
            entries: Vec<PhaseLogEntry>,
        }

        impl PhaseLogCapture {
            pub fn new() -> Self {
                Self::default()
            }

            pub fn push_transition(&mut self, from: Option<&str>, to: &str, step: u64) {
                self.entries.push(PhaseLogEntry {
                    event: "phase_transition".to_owned(),
                    from: from.map(str::to_owned),
                    to: to.to_owned(),
                    step,
                });
            }

            pub fn entries(&self) -> &[PhaseLogEntry] {
                &self.entries
            }

            pub fn to_values(&self) -> Vec<Value> {
                self.entries
                    .iter()
                    .map(|entry| serde_json::to_value(entry).expect("phase entry encodes"))
                    .collect()
            }

            pub fn to_ndjson(&self) -> Vec<u8> {
                let mut bytes = Vec::new();
                for entry in &self.entries {
                    serde_json::to_writer(&mut bytes, entry).expect("phase entry encodes");
                    bytes.push(b'\n');
                }
                bytes
            }
        }
    }

    pub mod gradient_capture {
        #[derive(Clone, Debug, PartialEq)]
        pub struct TensorGradNorm {
            pub name: String,
            pub l2_norm: f32,
        }

        pub fn capture_explicit_grad_norm(
            name: impl Into<String>,
            gradient: &[f32],
        ) -> TensorGradNorm {
            let squared_sum = gradient.iter().map(|value| value * value).sum::<f32>();
            TensorGradNorm {
                name: name.into(),
                l2_norm: squared_sum.sqrt(),
            }
        }

        #[cfg(any(
            feature = "phase-a",
            feature = "ablation",
            feature = "s2-full",
            feature = "s2-ablation"
        ))]
        pub fn capture_trivial_burn_grad_norms() -> Vec<TensorGradNorm> {
            use gbf_train::adapter::burn::{
                BurnDevice, BurnNdArrayAutodiffBackend, float_tensor_from_vec,
                float_tensor_into_vec,
            };

            type B = BurnNdArrayAutodiffBackend;
            let device = BurnDevice::<B>::default();
            let input = float_tensor_from_vec::<B, 1>(vec![2.0, -3.0], [2], &device)
                .expect("trivial autodiff tensor builds")
                .require_grad();
            let loss = (input.clone() * input.clone()).sum();
            let gradients = loss.backward();
            let grad = input.grad(&gradients).expect("input receives gradients");
            let grad_values = float_tensor_into_vec(grad).expect("gradient reads back");

            vec![capture_explicit_grad_norm("input", &grad_values)]
        }
    }

    pub mod scripted_falsify_runner {
        use std::cell::RefCell;
        use std::panic::{RefUnwindSafe, UnwindSafe};

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum BrokenS2Kind {
            F1PhaseBSkipsTernary,
            F2PhaseDUnfreezesTeacher,
            F3DistillTempInverted,
            F4ThresholdPerWeight,
            F5ZeroLossShortCircuit,
            F6LinearStateGradDead,
        }

        thread_local! {
            static ACTIVE_BROKEN_KIND: RefCell<Option<BrokenS2Kind>> = const { RefCell::new(None) };
        }

        #[derive(Debug)]
        pub struct ScriptedFalsifyGuard {
            previous: Option<BrokenS2Kind>,
        }

        impl ScriptedFalsifyGuard {
            pub fn activate(kind: BrokenS2Kind) -> Self {
                tracing::debug!(
                    event_name = "scripted_falsify_active",
                    broken_kind = ?kind
                );
                let previous = ACTIVE_BROKEN_KIND.with(|active| active.replace(Some(kind)));
                Self { previous }
            }
        }

        impl Drop for ScriptedFalsifyGuard {
            fn drop(&mut self) {
                ACTIVE_BROKEN_KIND.with(|active| {
                    active.replace(self.previous);
                });
            }
        }

        pub fn active_broken_kind() -> Option<BrokenS2Kind> {
            ACTIVE_BROKEN_KIND.with(|active| *active.borrow())
        }

        pub fn run_with_broken_kind<R>(
            kind: BrokenS2Kind,
            f: impl FnOnce() -> R + UnwindSafe + RefUnwindSafe,
        ) -> R {
            let _guard = ScriptedFalsifyGuard::activate(kind);
            f()
        }
    }

    pub mod tiny_model_s2 {
        use gbf_train::phase::{TrainPhaseKind, TrainingPhaseSchedule};
        use gbf_train::scheduler::{PhaseControlledModel, PhaseControls};

        pub type PhaseKindFixture = TrainPhaseKind;

        #[derive(Clone, Debug, Default)]
        pub struct TinyModelS2 {
            applied: Vec<PhaseControls>,
        }

        impl TinyModelS2 {
            pub fn applied_controls(&self) -> &[PhaseControls] {
                &self.applied
            }

            pub fn applied_kinds(&self) -> Vec<PhaseKindFixture> {
                self.applied
                    .iter()
                    .map(|controls| controls.phase().kind())
                    .collect()
            }
        }

        impl PhaseControlledModel for TinyModelS2 {
            fn apply_phase_controls(&mut self, controls: PhaseControls) {
                self.applied.push(controls);
            }
        }

        pub fn five_phase_schedule_fixture(steps_per_phase: u64) -> TrainingPhaseSchedule {
            TrainingPhaseSchedule::default_five_phase(steps_per_phase)
                .expect("fixture schedule is canonical five-phase")
        }

        pub fn five_phase_fixture() -> (TinyModelS2, TrainingPhaseSchedule) {
            (TinyModelS2::default(), five_phase_schedule_fixture(2))
        }
    }

    pub mod tracing_capture_s2 {
        use crate::common::tracing_capture::{
            TraceCapture, TracingEvent, captured_events, with_trace_capture,
        };

        pub type S2TraceCapture = TraceCapture;

        pub fn capture_events<R>(f: impl FnOnce() -> R) -> (R, Vec<TracingEvent>) {
            let capture = S2TraceCapture::default();
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

        pub fn assert_no_event(events: &[TracingEvent], name: &str) {
            assert!(
                events.iter().all(|event| event.name != name),
                "event {name:?} was unexpectedly emitted"
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

    pub mod structured_log_assert {
        use crate::common::tracing_capture::TracingEvent;
        use serde_json::Value;

        pub fn assert_log_sequence(events: &[TracingEvent], sequence: &[(&str, &str)]) {
            let mut cursor = 0;
            for (field, expected) in sequence {
                let Some((offset, _)) = events[cursor..]
                    .iter()
                    .enumerate()
                    .find(|(_, event)| field_matches(event, field, expected))
                else {
                    panic!("missing ordered log field {field:?}={expected:?}");
                };
                cursor += offset + 1;
            }
        }

        fn field_matches(event: &TracingEvent, field: &str, expected: &str) -> bool {
            if field == "event" || field == "name" {
                return event.name == expected;
            }
            match event.fields.get(field) {
                Some(Value::String(value)) => value == expected,
                Some(Value::Bool(value)) => value.to_string() == expected,
                Some(Value::Number(value)) => value.to_string() == expected,
                Some(Value::Null) | Some(Value::Array(_)) | Some(Value::Object(_)) | None => false,
            }
        }
    }
}

pub mod proptest_strategies;

#[macro_export]
macro_rules! assert_event_emitted {
    ($events:expr, name = $name:expr $(,)?) => {
        $crate::common::helpers::tracing_capture_s2::assert_event_emitted($events, $name)
    };
}

#[macro_export]
macro_rules! assert_no_event {
    ($events:expr, name = $name:expr $(,)?) => {
        $crate::common::helpers::tracing_capture_s2::assert_no_event($events, $name)
    };
}

#[macro_export]
macro_rules! assert_log_sequence {
    ($events:expr, [$($entry:expr),* $(,)?] $(,)?) => {
        $crate::common::helpers::structured_log_assert::assert_log_sequence(
            $events,
            &[$($entry),*],
        )
    };
}

pub mod injectable_rng {
    use std::collections::VecDeque;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ScriptedRng {
        draws: VecDeque<u64>,
    }

    impl ScriptedRng {
        pub fn new(draws: impl IntoIterator<Item = u64>) -> Self {
            Self {
                draws: draws.into_iter().collect(),
            }
        }

        pub fn remaining(&self) -> usize {
            self.draws.len()
        }

        pub fn is_empty(&self) -> bool {
            self.draws.is_empty()
        }

        pub fn next_u64(&mut self) -> u64 {
            self.draws
                .pop_front()
                .expect("ScriptedRng exhausted its scripted draws")
        }

        pub fn fill_bytes(&mut self, out: &mut [u8]) {
            for chunk in out.chunks_mut(8) {
                let draw = self.next_u64().to_le_bytes();
                chunk.copy_from_slice(&draw[..chunk.len()]);
            }
        }
    }
}

pub mod strategies {
    use crate::common::assertions::CanonicalTensor;
    use proptest::prelude::*;
    use serde_json::{Map, Number, Value};
    use std::ops::RangeInclusive;

    pub fn arb_seed_in_range(range: RangeInclusive<u64>) -> impl Strategy<Value = u64> {
        range
    }

    pub fn arb_byte_seq(min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(any::<u8>(), min_len..=max_len)
    }

    pub fn arb_canonical_json_value() -> BoxedStrategy<Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(|value| Value::Number(Number::from(value))),
            "[a-zA-Z0-9_ ./:-]{0,24}".prop_map(Value::String),
        ];

        leaf.prop_recursive(4, 64, 8, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
                prop::collection::btree_map(arb_deterministic_json_key(), inner, 0..8)
                    .prop_map(|entries| Value::Object(Map::from_iter(entries))),
            ]
        })
        .boxed()
    }

    pub fn arb_canonical_tensor_set() -> BoxedStrategy<Vec<CanonicalTensor>> {
        let tensor_body = (
            "[a-z][a-z0-9_]{0,15}",
            prop::collection::vec(1_u64..=16, 0..4),
            prop::collection::vec(any::<u8>(), 0..64),
        );

        prop::collection::btree_map("[a-z][a-z0-9_]{0,15}", tensor_body, 0..8)
            .prop_map(|entries| {
                entries
                    .into_iter()
                    .map(|(name, (dtype, shape, bytes))| CanonicalTensor {
                        name,
                        dtype,
                        shape,
                        bytes,
                    })
                    .collect()
            })
            .boxed()
    }

    fn arb_deterministic_json_key() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,15}".prop_filter("key must not be nondeterministic", |key| {
            !is_nondeterministic_field_name(key)
        })
    }

    fn is_nondeterministic_field_name(key: &str) -> bool {
        matches!(
            key,
            "created_at"
                | "mtime"
                | "nonce"
                | "pid"
                | "random"
                | "timestamp"
                | "uuid"
                | "wall_clock"
        )
    }
}

pub mod tempdir {
    use std::env;
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::TempDir;

    // This lock serializes environment mutation only among tests that use EnvGuard.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub struct EnvGuard {
        original: Vec<(OsString, OsString)>,
        _lock: MutexGuard<'static, ()>,
    }

    pub fn fresh_run_dir() -> TempDir {
        tempfile::Builder::new()
            .prefix("gbf-s1-run-")
            .tempdir()
            .expect("fresh S1 run directory must be creatable")
    }

    pub fn fresh_isolated_env(extra: &[(&str, &str)]) -> EnvGuard {
        let lock = ENV_LOCK.lock().expect("env test lock poisoned");
        let original = env::vars_os().collect::<Vec<_>>();
        for (key, _) in &original {
            // SAFETY: EnvGuard serializes process environment mutation through
            // ENV_LOCK and restores the full snapshot when dropped.
            unsafe { env::remove_var(key) };
        }
        for (key, value) in extra {
            // SAFETY: See the remove_var safety note above.
            unsafe { env::set_var(key, value) };
        }
        EnvGuard {
            original,
            _lock: lock,
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, _) in env::vars_os() {
                // SAFETY: EnvGuard holds ENV_LOCK for its lifetime.
                unsafe { env::remove_var(key) };
            }
            for (key, value) in &self.original {
                // SAFETY: EnvGuard holds ENV_LOCK for its lifetime.
                unsafe { env::set_var(key, value) };
            }
        }
    }
}

pub mod tracing_capture {
    use pretty_assertions::assert_eq;
    use serde_json::Number;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::fmt;
    use std::sync::{Arc, Mutex, Once};
    use std::thread::ThreadId;
    use tracing_subscriber::prelude::*;

    type EventSink = Arc<Mutex<Vec<TracingEvent>>>;

    static ACTIVE_TRACE_CAPTURES: Mutex<Vec<ActiveTraceSink>> = Mutex::new(Vec::new());
    static GLOBAL_TRACE_CAPTURE_INIT: Once = Once::new();
    static TRACE_CAPTURE_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct TracingEvent {
        pub name: String,
        pub level: String,
        pub fields: BTreeMap<String, Value>,
    }

    #[derive(Clone, Debug)]
    struct ActiveTraceSink {
        thread_id: ThreadId,
        events: EventSink,
    }

    #[derive(Clone, Debug, Default)]
    pub struct TraceCapture {
        events: Arc<Mutex<Vec<TracingEvent>>>,
    }

    impl TraceCapture {
        pub fn captured_events(&self) -> Vec<TracingEvent> {
            self.events
                .lock()
                .expect("trace capture mutex is not poisoned")
                .clone()
        }
    }

    impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            self.events
                .lock()
                .expect("trace capture mutex is not poisoned")
                .push(tracing_event_from(event));
        }
    }

    pub fn captured_events(capture: &TraceCapture) -> Vec<TracingEvent> {
        capture.captured_events()
    }

    pub fn with_trace_capture<R>(capture: &TraceCapture, f: impl FnOnce() -> R) -> R {
        ensure_global_trace_capture();
        let _guard = TRACE_CAPTURE_LOCK
            .lock()
            .expect("trace capture mutex is not poisoned");
        let _active_capture = ActiveTraceCapture::new(capture.events.clone());

        tracing::callsite::rebuild_interest_cache();
        f()
    }

    pub fn assert_event_at(
        events: &[TracingEvent],
        idx: usize,
        name: &str,
        fields: &[(&str, Value)],
    ) {
        let event = events
            .get(idx)
            .unwrap_or_else(|| panic!("missing tracing event at index {idx}"));
        assert_eq!(event.name, name);
        for (key, expected) in fields {
            assert_eq!(
                event.fields.get(*key),
                Some(expected),
                "field {key:?} differed for event {name:?}"
            );
        }
    }

    #[derive(Debug, Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, Value>,
    }

    #[derive(Clone, Copy, Debug)]
    struct GlobalTraceCaptureLayer;

    impl<S> tracing_subscriber::layer::Layer<S> for GlobalTraceCaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let sinks = ACTIVE_TRACE_CAPTURES
                .lock()
                .expect("active trace capture mutex is not poisoned")
                .clone();
            if sinks.is_empty() {
                return;
            }

            let thread_id = std::thread::current().id();
            let captured = tracing_event_from(event);
            for sink in sinks {
                if sink.thread_id == thread_id {
                    sink.events
                        .lock()
                        .expect("trace capture mutex is not poisoned")
                        .push(captured.clone());
                }
            }
        }
    }

    struct ActiveTraceCapture {
        events: EventSink,
    }

    impl ActiveTraceCapture {
        fn new(events: EventSink) -> Self {
            ACTIVE_TRACE_CAPTURES
                .lock()
                .expect("active trace capture mutex is not poisoned")
                .push(ActiveTraceSink {
                    thread_id: std::thread::current().id(),
                    events: events.clone(),
                });
            Self { events }
        }
    }

    impl Drop for ActiveTraceCapture {
        fn drop(&mut self) {
            let mut active = ACTIVE_TRACE_CAPTURES
                .lock()
                .expect("active trace capture mutex is not poisoned");
            if let Some(index) = active
                .iter()
                .position(|sink| Arc::ptr_eq(&sink.events, &self.events))
            {
                active.remove(index);
            }
            tracing::callsite::rebuild_interest_cache();
        }
    }

    fn ensure_global_trace_capture() {
        GLOBAL_TRACE_CAPTURE_INIT.call_once(|| {
            let subscriber = tracing_subscriber::registry()
                .with(tracing_subscriber::filter::LevelFilter::TRACE)
                .with(GlobalTraceCaptureLayer);
            tracing::subscriber::set_global_default(subscriber)
                .expect("global test trace capture subscriber must install once");
        });
    }

    fn tracing_event_from(event: &tracing::Event<'_>) -> TracingEvent {
        let mut visitor = TraceFieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .get("event_name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| event.metadata().name().to_owned());
        TracingEvent {
            name,
            level: event.metadata().level().to_string(),
            fields: visitor.fields,
        }
    }

    impl TraceFieldVisitor {
        fn insert(&mut self, field: &tracing::field::Field, value: Value) {
            self.fields.insert(field.name().to_owned(), value);
        }
    }

    impl tracing::field::Visit for TraceFieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.insert(field, Value::String(format!("{value:?}")));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.insert(field, Value::String(value.to_owned()));
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.insert(field, Value::Bool(value));
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.insert(field, Value::Number(Number::from(value)));
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.insert(field, Value::Number(Number::from(value)));
        }

        fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
            let value = Number::from_f64(value)
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(value.to_string()));
            self.insert(field, value);
        }
    }
}

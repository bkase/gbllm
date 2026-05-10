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

    #[derive(Debug, Eq, PartialEq)]
    pub struct TinyCorpus {
        pub name: &'static str,
        pub bytes: &'static [u8],
        pub token_count: usize,
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
        static TRACE_CAPTURE_LOCK: Mutex<()> = Mutex::new(());

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

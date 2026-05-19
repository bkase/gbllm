use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

pub const STAGE4_TRACE_TARGET: &str = "gbf_codegen::s4";
pub const STAGE5_TRACE_TARGET: &str = "gbf_codegen::s5";
pub const RANGE_CERT_TRACE_TARGET: &str = "gbf_verify::range_cert";

pub const STAGE4_EVENT_NAMES: &[&str] = &[
    "stage4.observation_plan.identity_bind",
    "stage4.observation_plan.schema_ingest",
    "stage4.observation_plan.build_feasibility_filter",
    "stage4.observation_plan.semantic_selection",
    "stage4.observation_plan.semantic_anchor_binding",
    "stage4.observation_plan.observation_encoding_binding",
    "stage4.observation_plan.probe_registry_instantiation",
    "stage4.observation_plan.probe_budget_governance",
    "stage4.observation_plan.probe_ordering",
    "stage4.observation_plan.metric_registry_filter",
    "stage4.observation_plan.metric_selection",
    "stage4.observation_plan.metric_ordering",
    "stage4.observation_plan.anchor_table_bind",
    "stage4.observation_plan.provenance_bind",
    "stage4.observation_plan.schema_re_emit",
    "stage4.observation_plan.operational_probe_schema_emit",
    "stage4.observation_plan.invariant_budget_check",
    "stage4.observation_plan.self_consistency",
    "stage4.observation_plan.canonical_sort",
    "stage4.driver.cache_lookup",
    "stage4.driver.cache_hit",
    "stage4.driver.cache_miss",
    "stage4.driver.report_emit",
    "stage4.driver.failure_memo",
    "stage4.driver.audit_parent_rewrap",
    "stage4.driver.run",
];

pub const STAGE5_EVENT_NAMES: &[&str] = &[
    "stage5.range_plan.identity_bind",
    "stage5.range_plan.reduction_site_enumeration",
    "stage5.range_plan.site_facts_binding",
    "stage5.range_plan.effective_ceiling_binding",
    "stage5.range_plan.plan_candidate_generation",
    "stage5.range_plan.plan_length_selection",
    "stage5.range_plan.certificate_construction",
    "stage5.range_plan.plan_choice",
    "stage5.range_plan.provenance_bind",
    "stage5.range_plan.canonical_sort",
    "stage5.range_plan.self_consistency",
    "stage5.driver.cache_lookup",
    "stage5.driver.cache_hit",
    "stage5.driver.cache_miss",
    "stage5.driver.report_emit",
    "stage5.driver.failure_memo",
    "stage5.driver.audit_parent_rewrap",
    "stage5.driver.run",
    "range_cert.verifies.single_i16",
    "range_cert.verifies.chunked_i16",
    "range_cert.verifies.renorm_loop",
    "range_cert.verifies.failed",
    "range_cert.renorm_recurrence_verifies",
];

pub const RANGE_CERT_VERIFY_EVENT_NAMES: &[&str] = &[
    "range_cert.independent_verify.parse",
    "range_cert.independent_verify.report_self_hash_check",
    "range_cert.independent_verify.certified_reduction.single_i16",
    "range_cert.independent_verify.certified_reduction.chunked_i16",
    "range_cert.independent_verify.certified_reduction.renorm_loop",
    "range_cert.independent_verify.failed",
    "range_cert.independent_verify.failed.malformed",
    "range_cert.independent_verify.failed.report_self_hash_mismatch",
    "range_cert.independent_verify.failed.unsupported_plan_family",
    "range_cert.independent_verify.failed.witness_mismatch",
];

pub const F_B6_F_B7_COMMON_EVENT_FIELDS: &[&str] = &[
    "site_id",
    "checkpoint_id",
    "compact_checkpoint_id",
    "stratum",
    "probe_instance_id",
    "runtime_probe_id",
    "importance_class",
    "build_id",
    "k4_hash",
    "k5_hash",
    "outcome",
    "diag_code",
    "elapsed_ns",
    "event_seq",
];

pub fn is_closed_event_name(name: &str) -> bool {
    STAGE4_EVENT_NAMES.contains(&name)
        || STAGE5_EVENT_NAMES.contains(&name)
        || RANGE_CERT_VERIFY_EVENT_NAMES.contains(&name)
}

pub fn closed_event_names() -> impl Iterator<Item = &'static str> {
    STAGE4_EVENT_NAMES
        .iter()
        .copied()
        .chain(STAGE5_EVENT_NAMES.iter().copied())
        .chain(RANGE_CERT_VERIFY_EVENT_NAMES.iter().copied())
}

#[macro_export]
macro_rules! f_b6_f_b7_trace_event {
    (target: $target:expr, $event_name:expr $(, $field:ident = $value:expr)* $(,)?) => {
        tracing::info!(target: $target, event = $event_name $(, $field = $value)*);
    };
    ($event_name:expr $(, $field:ident = $value:expr)* $(,)?) => {
        $crate::f_b6_f_b7_trace_event!(target: module_path!(), $event_name $(, $field = $value)*);
    };
}

#[macro_export]
macro_rules! f_b6_f_b7_stage4_trace_event {
    ($event_name:expr $(, $field:ident = $value:expr)* $(,)?) => {
        $crate::f_b6_f_b7_trace_event!(
            target: $crate::support::f_b6_f_b7::STAGE4_TRACE_TARGET,
            $event_name
            $(, $field = $value)*
        );
    };
}

#[macro_export]
macro_rules! f_b6_f_b7_stage5_trace_event {
    ($event_name:expr $(, $field:ident = $value:expr)* $(,)?) => {
        $crate::f_b6_f_b7_trace_event!(
            target: $crate::support::f_b6_f_b7::STAGE5_TRACE_TARGET,
            $event_name
            $(, $field = $value)*
        );
    };
}

#[macro_export]
macro_rules! f_b6_f_b7_range_cert_trace_event {
    ($event_name:expr $(, $field:ident = $value:expr)* $(,)?) => {
        $crate::f_b6_f_b7_trace_event!(
            target: $crate::support::f_b6_f_b7::RANGE_CERT_TRACE_TARGET,
            $event_name
            $(, $field = $value)*
        );
    };
}

#[derive(Clone)]
pub struct Fb6Fb7NdjsonSink {
    writer: Arc<Mutex<File>>,
}

impl Fb6Fb7NdjsonSink {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            writer: Arc::new(Mutex::new(File::create(path)?)),
        })
    }
}

impl<S> Layer<S> for Fb6Fb7NdjsonSink
where
    S: Subscriber,
    for<'a> S: LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let event_name = visitor
            .fields
            .remove("event")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| event.metadata().name().to_owned());
        let span = ctx.lookup_current().map(|span| {
            serde_json::json!({
                "name": span.name(),
                "target": span.metadata().target(),
            })
        });
        let line = serde_json::json!({
            "ts": timestamp_string(),
            "event": event_name,
            "level": event.metadata().level().as_str(),
            "target": event.metadata().target(),
            "fields": Value::Object(visitor.fields),
            "span": span,
        });

        let result = (|| -> io::Result<()> {
            let mut writer = self
                .writer
                .lock()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "ndjson sink mutex poisoned"))?;
            serde_json::to_writer(&mut *writer, &line)?;
            writer.write_all(b"\n")?;
            writer.flush()
        })();
        if let Err(error) = result {
            panic!("failed to write F-B6/F-B7 telemetry event {event_name}: {error}");
        }
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl Visit for JsonFieldVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }
}

/// Timestamp shape pinned by the packet verifier: `unix:<seconds>.<9 nanos>`.
pub fn timestamp_string() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("unix:{}.{:09}", duration.as_secs(), duration.subsec_nanos())
}

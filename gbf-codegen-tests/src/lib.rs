//! Shared test helpers for gbf-codegen integration and downstream F-B8 beads.

pub use gbf_codegen::assert_storage_plan_traced;
pub use gbf_codegen::storage_plan_test_infra::{
    NdjsonTraceSink, TraceCapture, cache_key_prefix, debug_harness, sc_violations, synth,
    timestamp_string, trace_catalog, with_trace_capture,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_synth_surface_builds_canonical_storage_plan_inputs() {
        let inputs = synth::tiny_routed_ffn_inputs();

        gbf_codegen::storage_plan::canonicalize_inputs(&inputs)
            .expect("public synth surface emits canonical inputs");
    }

    #[test]
    fn public_trace_assertion_macro_is_reexported() {
        let (_, events) = with_trace_capture(|| {
            tracing_event_for_reexport_test();
        });

        assert_storage_plan_traced!(
            &events,
            [
                {
                    event: "f_b8.rule.fired",
                    rule_name: "DR-6"
                }
            ]
        );
    }

    fn tracing_event_for_reexport_test() {
        tracing::info!(event = "f_b8.rule.fired", rule_name = "DR-6");
    }
}

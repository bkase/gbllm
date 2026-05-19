use gbf_policy::{
    metric_registry_canonical_json_bytes, metric_registry_v1, probe_registry_canonical_json_bytes,
    probe_registry_v1, trace_event_layout_registry_canonical_json_bytes,
    trace_event_layout_registry_v1,
};

fn main() {
    dump(
        "probe_registry.v1.json",
        probe_registry_canonical_json_bytes(&probe_registry_v1()).expect("probe registry dumps"),
    );
    dump(
        "metric_registry.v1.json",
        metric_registry_canonical_json_bytes(&metric_registry_v1()).expect("metric registry dumps"),
    );
    dump(
        "trace_event_layout_registry.v1.json",
        trace_event_layout_registry_canonical_json_bytes(&trace_event_layout_registry_v1())
            .expect("trace event layout registry dumps"),
    );
}

fn dump(name: &str, bytes: Vec<u8>) {
    println!("--- {name} ---");
    println!(
        "{}",
        String::from_utf8(bytes).expect("canonical registry JSON is UTF-8")
    );
}

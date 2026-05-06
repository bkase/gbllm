//! Workspace meta-checks that guard architectural contracts.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const FORBIDDEN_VERIFY_TARGET: &str = "gbf-verify";
pub const PRODUCTION_CRATES: &[&str] = &[
    "gbf-codegen",
    "gbf-ir",
    "gbf-bench",
    "gbf-report",
    "gbf-runtime",
    "gbf-asm",
    "gbf-emu",
    "gbf-abi",
    "gbf-hw",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepDirectionViolation {
    pub offender: String,
    pub forbidden: String,
    pub chain: Vec<String>,
}

impl fmt::Display for DepDirectionViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} has forbidden production dependency on {} via {}",
            self.offender,
            self.forbidden,
            self.chain.join(" -> ")
        )
    }
}

pub fn enforce_no_verify_in_production(
    workspace_root: &Path,
) -> Result<(), Vec<DepDirectionViolation>> {
    let graph = read_workspace_graph(workspace_root).map_err(|error| {
        vec![DepDirectionViolation {
            offender: "workspace".to_owned(),
            forbidden: FORBIDDEN_VERIFY_TARGET.to_owned(),
            chain: vec![error],
        }]
    })?;
    let mut violations = Vec::new();
    for offender in PRODUCTION_CRATES {
        if let Some(chain) = find_path(&graph, offender, FORBIDDEN_VERIFY_TARGET) {
            let violation = DepDirectionViolation {
                offender: (*offender).to_owned(),
                forbidden: FORBIDDEN_VERIFY_TARGET.to_owned(),
                chain,
            };
            tracing::error!(
                target: "gbf_meta_checks::dep_direction",
                offender = %violation.offender,
                forbidden = %violation.forbidden,
                chain = %violation.chain.join(" -> "),
                "production crate has forbidden dependency on verifier"
            );
            violations.push(violation);
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

pub fn crate_depends_on(workspace_root: &Path, crate_name: &str, target: &str) -> bool {
    read_workspace_graph(workspace_root)
        .ok()
        .and_then(|graph| find_path(&graph, crate_name, target))
        .is_some()
}

pub fn ignored_f_b1_heavy_tests(workspace_root: &Path) -> Result<(), String> {
    let source_path = workspace_root.join("gbf-emu/src/lib.rs");
    let source = fs::read_to_string(&source_path)
        .map_err(|error| format!("{}: {error}", source_path.display()))?;
    let heavy_tests = [
        "f_b1_l3_streamed_output_matches_reference_n128",
        "f_b1_l4_n128_no_frame_service_misses",
        "f_b1_l4_liveness_no_progress_frames_bounded",
        "f_b1_l4_output_matches_reference",
    ];

    for test_name in heavy_tests {
        let needle = format!("fn {test_name}()");
        let fn_start = source
            .find(&needle)
            .ok_or_else(|| format!("{test_name} missing from gbf-emu tests"))?;
        let before_fn = &source[..fn_start];
        let attrs_start = before_fn
            .rfind("\n    #[test]")
            .ok_or_else(|| format!("{test_name} has no #[test] attribute"))?;
        let attrs = &source[attrs_start..fn_start];
        if !attrs.contains("#[ignore") {
            return Err(format!(
                "{test_name} must remain #[ignore] until real L4 emulated evidence replaces the synthetic fixture"
            ));
        }
    }
    Ok(())
}

fn read_workspace_graph(workspace_root: &Path) -> Result<BTreeMap<String, Vec<String>>, String> {
    let root_toml = read_toml(&workspace_root.join("Cargo.toml"))?;
    let members = root_toml["workspace"]["members"]
        .as_array()
        .ok_or("workspace.members missing")?;
    let mut graph = BTreeMap::new();
    for member in members {
        let member = member.as_str().ok_or("workspace member is not a string")?;
        let manifest = workspace_root.join(member).join("Cargo.toml");
        let value = read_toml(&manifest)?;
        let name = value["package"]["name"]
            .as_str()
            .ok_or_else(|| format!("{} package.name missing", manifest.display()))?
            .to_owned();
        graph.insert(name, dependency_names(&value, "dependencies"));
    }
    Ok(graph)
}

fn dependency_names(value: &toml::Value, table: &str) -> Vec<String> {
    value
        .get(table)
        .and_then(toml::Value::as_table)
        .map(|deps| deps.keys().cloned().collect())
        .unwrap_or_default()
}

fn read_toml(path: &Path) -> Result<toml::Value, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    toml::from_str::<toml::Value>(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn find_path(
    graph: &BTreeMap<String, Vec<String>>,
    start: &str,
    target: &str,
) -> Option<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(vec![start.to_owned()]);
    while let Some(path) = queue.pop_front() {
        let node = path.last().expect("path has node");
        if !seen.insert(node.clone()) {
            continue;
        }
        for dep in graph.get(node).into_iter().flatten() {
            let mut next = path.clone();
            next.push(dep.clone());
            if dep == target {
                return Some(next);
            }
            queue.push_back(next);
        }
    }
    None
}

#[must_use]
pub fn workspace_root_from_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate is in workspace root")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::subscriber::Interest;
    use tracing::{Event, Id, Metadata, Subscriber};

    #[test]
    fn production_crates_do_not_depend_on_gbf_verify() {
        enforce_no_verify_in_production(&workspace_root_from_manifest()).expect("dep direction");
    }

    #[test]
    fn gbf_verify_can_depend_on_gbf_abi() {
        assert!(crate_depends_on(
            &workspace_root_from_manifest(),
            "gbf-verify",
            "gbf-abi"
        ));
    }

    #[test]
    fn dev_only_use_of_gbf_verify_is_allowed() {
        let graph = read_workspace_graph(&workspace_root_from_manifest()).expect("graph");
        assert!(
            !graph
                .get("gbf-codegen")
                .expect("codegen node")
                .contains(&"gbf-verify".to_owned()),
            "gbf-codegen may use gbf-verify only as a dev-dependency"
        );
    }

    #[test]
    fn ignored_discipline_heavy_f_b1_tests_are_ignored() {
        ignored_f_b1_heavy_tests(&workspace_root_from_manifest()).expect("ignored discipline");
    }

    #[test]
    fn violation_logging_shape_is_captured() {
        let subscriber = ErrorCapture::default();
        let events = Arc::clone(&subscriber.events);
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(
                target: "gbf_meta_checks::dep_direction",
                offender = "gbf-codegen",
                forbidden = "gbf-verify",
                chain = "gbf-codegen -> gbf-verify",
                "production crate has forbidden dependency on verifier"
            );
        });
        assert_eq!(
            events.lock().expect("events").as_slice(),
            &[(
                "gbf-codegen".to_owned(),
                "gbf-verify".to_owned(),
                "gbf-codegen -> gbf-verify".to_owned()
            )]
        );
    }

    #[derive(Clone, Default)]
    struct ErrorCapture {
        events: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    impl Subscriber for ErrorCapture {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.target() == "gbf_meta_checks::dep_direction"
        }

        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = ErrorVisitor::default();
            event.record(&mut visitor);
            self.events.lock().expect("events").push((
                visitor.offender.unwrap_or_default(),
                visitor.forbidden.unwrap_or_default(),
                visitor.chain.unwrap_or_default(),
            ));
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}

        fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
            if self.enabled(metadata) {
                Interest::always()
            } else {
                Interest::never()
            }
        }
    }

    #[derive(Default)]
    struct ErrorVisitor {
        offender: Option<String>,
        forbidden: Option<String>,
        chain: Option<String>,
    }

    impl Visit for ErrorVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.set(field.name(), format!("{value:?}").trim_matches('"'));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.set(field.name(), value);
        }
    }

    impl ErrorVisitor {
        fn set(&mut self, name: &str, value: &str) {
            match name {
                "offender" => self.offender = Some(value.to_owned()),
                "forbidden" => self.forbidden = Some(value.to_owned()),
                "chain" => self.chain = Some(value.to_owned()),
                _ => {}
            }
        }
    }
}

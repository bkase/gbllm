use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn burn_imports_are_confined_to_train_adapter() {
    let root = workspace_root();
    let allowed_adapter_dir = root.join("gbf-train/src/adapter");
    let scan_roots = [
        "gbf-artifact/src",
        "gbf-foundation/src",
        "gbf-model/src",
        "gbf-oracle/src",
        "gbf-train/src",
        "gbf-verify/src",
    ];

    let mut violations = Vec::new();
    for scan_root in scan_roots {
        for file in rust_files(&root.join(scan_root)) {
            if file.starts_with(&allowed_adapter_dir) {
                continue;
            }

            for (line_number, line) in read_lines(&file).iter().enumerate() {
                if has_direct_burn_reference(line) {
                    violations.push(format!(
                        "{}:{}: {}",
                        display_path(&root, &file),
                        line_number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "direct Burn imports must stay behind gbf-train::adapter:\n{}",
        violations.join("\n")
    );
}

#[test]
fn model_does_not_export_final_artifact_tensor_types_before_artifact_owns_them() {
    let root = workspace_root();
    if artifact_owns_canonical_tensor_contract(&root) {
        return;
    }

    let mut violations = Vec::new();
    for file in rust_files(&root.join("gbf-model/src")) {
        for (line_number, line) in read_lines(&file).iter().enumerate() {
            if exports_final_artifact_tensor_type(line) {
                violations.push(format!(
                    "{}:{}: {}",
                    display_path(&root, &file),
                    line_number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "gbf-model must not export final artifact canonical tensor types before gbf-artifact owns the contract:\n{}",
        violations.join("\n")
    );
}

#[test]
fn qat_modules_do_not_hide_test_only_backend_seams() {
    let root = workspace_root();
    let scan_roots = ["gbf-model/src/qat", "gbf-train/src/qat"];

    let mut violations = Vec::new();
    for scan_root in scan_roots {
        for file in rust_files(&root.join(scan_root)) {
            let lines = read_lines(&file);
            for (line_index, line) in lines.iter().enumerate() {
                if !strip_line_comment(line).contains("#[cfg(test)]") {
                    continue;
                }

                let window_end = lines.len().min(line_index + 8);
                for (offset, candidate) in lines[line_index + 1..window_end].iter().enumerate() {
                    let code = strip_line_comment(candidate).trim();
                    if is_test_only_backend_seam(code) {
                        violations.push(format!(
                            "{}:{}: {}",
                            display_path(&root, &file),
                            line_index + offset + 2,
                            code
                        ));
                    }
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "QAT modules must not keep test-only backend seams; expose a real adapter seam or keep tests scalar-only:\n{}",
        violations.join("\n")
    );
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-test crate should live directly under the workspace root")
        .to_path_buf()
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", root.display());
    });

    for entry in entries {
        let path = entry.expect("directory entries should be readable").path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        .lines()
        .map(ToOwned::to_owned)
        .collect()
}

fn has_direct_burn_reference(line: &str) -> bool {
    let code = strip_line_comment(line);
    let trimmed = code.trim_start();
    trimmed.starts_with("use burn::")
        || trimmed.starts_with("pub use burn::")
        || trimmed.starts_with("extern crate burn")
        || code.contains(" burn::")
        || code.contains("= burn::")
        || code.contains("<burn::")
        || code.contains("(burn::")
        || code.contains("[burn::")
        || code.contains(", burn::")
        || code.contains(": burn::")
}

fn exports_final_artifact_tensor_type(line: &str) -> bool {
    let code = strip_line_comment(line).trim();
    let exports_type = code.starts_with("pub struct ")
        || code.starts_with("pub enum ")
        || code.starts_with("pub type ")
        || code.starts_with("pub use ");

    exports_type
        && [
            "CanonicalTensor",
            "ArtifactTensor",
            "CanonicalArtifact",
            "ArtifactCanonical",
            "FinalArtifact",
        ]
        .iter()
        .any(|name| code.contains(name))
}

fn is_test_only_backend_seam(code: &str) -> bool {
    let declares_type = code.starts_with("trait ")
        || code.starts_with("pub trait ")
        || code.starts_with("struct ")
        || code.starts_with("pub struct ");

    declares_type
        && [
            "Backend",
            "SteBackend",
            "FakeQuantBackend",
            "TensorBackend",
            "AdapterBackend",
        ]
        .iter()
        .any(|name| code.contains(name))
}

fn artifact_owns_canonical_tensor_contract(root: &Path) -> bool {
    rust_files(&root.join("gbf-artifact/src"))
        .iter()
        .flat_map(|file| read_lines(file))
        .any(|line| {
            let code = strip_line_comment(&line).trim();
            code.starts_with("pub struct CanonicalTensor")
                || code.starts_with("pub enum CanonicalTensor")
                || code.starts_with("pub type CanonicalTensor")
        })
}

fn strip_line_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

use std::fs;
use std::path::{Path, PathBuf};

#[test]
#[ignore = "promoted to a workspace gate once gbf-test owns literal enforcement; see bd-3sk"]
fn grep_no_redundant_constants() {
    let root = workspace_root();
    let allowlist = load_allowlist(&root);
    let needles = [
        "0xC000", "0xFF80", "0xA000", "0xFFFF", "0x4000", "0x0A", "0x0040", "0x0048", "0x0050",
        "0x0058", "0x0060", "0xFF00", "0xFF0F", "0xFF04", "0xFF05", "0xFF06", "0xFF07", "0xFF40",
        "0xFF41", "0xFF42", "0xFF43", "0xFF44", "0xFF45", "0xFF46", "0xFF47", "0xFF48", "0xFF49",
        "0xFF4A", "0xFF4B", "0x0148", "0x0149", "17556", "1140", "0xCE",
    ];

    let mut violations = Vec::new();
    for file in source_files(&root) {
        let rel = display_path(&root, &file);
        if rel.starts_with("gbf-hw/") || allowlist.iter().any(|allowed| allowed == &rel) {
            continue;
        }

        let source =
            fs::read_to_string(&file).unwrap_or_else(|err| panic!("failed to read {rel}: {err}"));
        for (line_index, line) in production_lines(&source).enumerate() {
            if needles.iter().any(|needle| line.contains(needle)) {
                violations.push(format!("{rel}:{}: {}", line_index + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "hardware literals must be sourced from gbf-hw:\n{}",
        violations.join("\n")
    );
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-hw should be directly under workspace root")
        .to_path_buf()
}

fn production_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
}

fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).expect("workspace root should be readable") {
        let path = entry.expect("directory entry should be readable").path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("gbf-") {
            continue;
        }
        collect_files(&path, &mut files);
    }
    files.sort();
    files
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).unwrap_or_else(|err| panic!("failed to read {path:?}: {err}")) {
        let child = entry.expect("directory entry should be readable").path();
        if child.is_dir() {
            collect_files(&child, files);
        } else if child
            .extension()
            .is_some_and(|extension| extension == "rs" || extension == "json")
        {
            files.push(child);
        }
    }
}

fn load_allowlist(root: &Path) -> Vec<String> {
    let path = root.join("gbf-hw/tests/single_source_smoke.allowlist.yaml");
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(ToOwned::to_owned)
        .collect()
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

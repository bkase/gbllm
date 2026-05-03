//! Stable adapter boundary around training-framework dependencies.
//!
//! `gbf-model` owns deployable numeric semantics and must not import Burn
//! directly. Training code that needs Burn APIs should go through this module so
//! version/API drift is contained inside `gbf-train`.

#[cfg(feature = "burn-adapter")]
pub mod burn;

/// Exact Burn version pinned by the workspace.
pub const BURN_VERSION: &str = "0.21.0-pre.3";

/// Exact Cargo requirement expected for the Burn workspace dependency.
pub const BURN_VERSION_REQUIREMENT: &str = "=0.21.0-pre.3";

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn burn_static_boundary_from_env_root_passes() {
        let root = std::env::var_os("GBF_BURN_PIN_REPO_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(workspace_root);

        check_burn_static_boundaries(&root)
            .unwrap_or_else(|error| panic!("Burn static boundary failed: {error}"));
    }

    #[test]
    fn burn_static_boundary_rejects_invalid_fixtures() {
        let valid = FixtureRoot::new("valid");
        assert!(check_burn_static_boundaries(valid.path()).is_ok());

        let loosened = FixtureRoot::new("loosened");
        loosened.write(
            "Cargo.toml",
            r#"
[workspace.dependencies]
burn = { version = "0.21.0-pre.3", default-features = false }
"#,
        );
        expect_fixture_error(
            loosened.path(),
            "workspace Cargo.toml must pin burn with an exact version",
        );

        let missing = FixtureRoot::new("missing");
        missing.write(
            "Cargo.toml",
            r#"
[workspace.dependencies]
serde = "1"
"#,
        );
        expect_fixture_error(
            missing.path(),
            "workspace Cargo.toml must declare the burn dependency",
        );

        let train_direct = FixtureRoot::new("train-direct");
        train_direct.write(
            "gbf-train/Cargo.toml",
            r#"
[dependencies]
burn = { version = "=0.21.0-pre.3", optional = true }
"#,
        );
        expect_fixture_error(
            train_direct.path(),
            "gbf-train/Cargo.toml must consume workspace burn",
        );

        let model_dep = FixtureRoot::new("model-dep");
        model_dep.write(
            "gbf-model/Cargo.toml",
            r#"
[dependencies]
burn = { version = "=0.21.0-pre.3" }
"#,
        );
        expect_fixture_error(
            model_dep.path(),
            "gbf-model/Cargo.toml must not depend on Burn directly",
        );

        let alias_dep = FixtureRoot::new("alias-dep");
        alias_dep.write(
            "gbf-model/Cargo.toml",
            r#"
[dependencies]
burn_alias = { package = "burn", version = "=0.21.0-pre.3" }
"#,
        );
        expect_fixture_error(
            alias_dep.path(),
            "gbf-model/Cargo.toml must not depend on Burn directly",
        );

        let workspace_alias = FixtureRoot::new("workspace-alias");
        workspace_alias.write(
            "Cargo.toml",
            r#"
[workspace.dependencies]
burn = { version = "=0.21.0-pre.3", default-features = false }
burn_alias = { package = "burn", version = "=0.21.0-pre.3" }
"#,
        );
        workspace_alias.write(
            "gbf-model/Cargo.toml",
            r#"
[dependencies]
burn_alias = { workspace = true }
"#,
        );
        expect_fixture_error(
            workspace_alias.path(),
            "gbf-model/Cargo.toml must not depend on Burn directly",
        );

        let direct_ref = FixtureRoot::new("direct-ref");
        direct_ref.write(
            "gbf-model/src/lib.rs",
            r#"
use burn::tensor::Tensor;

pub fn scalar_only() -> u32 {
    1
}
"#,
        );
        expect_fixture_error(
            direct_ref.path(),
            "gbf-model/src must not reference burn directly",
        );
    }

    #[test]
    fn adapter_constants_match_pinned_requirement() {
        assert_eq!(BURN_VERSION_REQUIREMENT, format!("={BURN_VERSION}"));
    }

    fn check_burn_static_boundaries(root: &Path) -> Result<(), String> {
        let root_doc = load_manifest(root, "Cargo.toml")?;
        let train_doc = load_manifest(root, "gbf-train/Cargo.toml")?;
        let model_doc = load_manifest(root, "gbf-model/Cargo.toml")?;
        let workspace_deps = child_table(root_doc.as_table(), "workspace")
            .and_then(|workspace| child_table(Some(workspace), "dependencies"))
            .ok_or_else(|| {
                "workspace Cargo.toml must declare [workspace.dependencies]".to_owned()
            })?;

        let burn_spec = workspace_deps
            .get("burn")
            .ok_or_else(|| "workspace Cargo.toml must declare the burn dependency".to_owned())?;
        if dependency_package("burn", burn_spec, workspace_deps) != "burn" {
            return Err(r#"workspace burn dependency must refer to package "burn""#.to_owned());
        }

        let burn_version = dependency_version(burn_spec).ok_or_else(|| {
            "workspace Cargo.toml must declare a version for the burn dependency".to_owned()
        })?;
        if !is_exact_requirement(burn_version) {
            return Err(
                "workspace Cargo.toml must pin burn with an exact version, e.g. burn = { version = \"=0.21.0-pre.3\", ... }"
                    .to_owned(),
            );
        }

        let train_deps = child_table(train_doc.as_table(), "dependencies")
            .ok_or_else(|| "gbf-train/Cargo.toml must have a [dependencies] table".to_owned())?;
        let train_burn = train_deps.get("burn").ok_or_else(|| {
            "gbf-train/Cargo.toml must declare its optional workspace burn dependency".to_owned()
        })?;
        let train_burn_table = train_burn.as_table().ok_or_else(|| {
            "gbf-train/Cargo.toml must declare burn as a dependency table".to_owned()
        })?;
        if train_burn_table.contains_key("version") {
            return Err(
                "gbf-train/Cargo.toml must consume workspace burn, not declare its own Burn version"
                    .to_owned(),
            );
        }
        if !table_bool(train_burn_table, "workspace") || !table_bool(train_burn_table, "optional") {
            return Err(
                "gbf-train/Cargo.toml must depend on burn via workspace = true and optional = true"
                    .to_owned(),
            );
        }
        if dependency_package("burn", train_burn, workspace_deps) != "burn" {
            return Err(r#"gbf-train burn dependency must refer to package "burn""#.to_owned());
        }

        let mut model_burn_dependencies = Vec::new();
        for (table_name, table) in dependency_tables(&model_doc) {
            for (dep_name, spec) in table {
                let package = dependency_package(dep_name, spec, workspace_deps);
                if dep_name == "burn" || package == "burn" {
                    model_burn_dependencies.push(format!("{table_name}.{dep_name}"));
                }
            }
        }
        if !model_burn_dependencies.is_empty() {
            return Err(format!(
                "gbf-model/Cargo.toml must not depend on Burn directly, including aliases: {}",
                model_burn_dependencies.join(", ")
            ));
        }

        let model_src = root.join("gbf-model/src");
        if !model_src.is_dir() {
            return Err("missing required directory: gbf-model/src".to_owned());
        }
        let mut direct_refs = Vec::new();
        for file in rust_files(&model_src)? {
            let contents = fs::read_to_string(&file)
                .map_err(|error| format!("failed to read {}: {error}", file.display()))?;
            for (line_index, line) in contents.lines().enumerate() {
                if has_direct_burn_reference(line) {
                    direct_refs.push(format!(
                        "{}:{}: {}",
                        display_path(root, &file),
                        line_index + 1,
                        line.trim()
                    ));
                }
            }
        }
        if !direct_refs.is_empty() {
            return Err(format!(
                "gbf-model/src must not reference burn directly:\n{}",
                direct_refs.join("\n")
            ));
        }

        Ok(())
    }

    fn load_manifest(root: &Path, relative: &str) -> Result<toml::Value, String> {
        let path = root.join(relative);
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", display_path(root, &path)))?;
        toml::from_str::<toml::Value>(&contents)
            .map_err(|error| format!("failed to parse {}: {error}", display_path(root, &path)))
    }

    fn child_table<'a>(
        table: Option<&'a toml::map::Map<String, toml::Value>>,
        name: &str,
    ) -> Option<&'a toml::map::Map<String, toml::Value>> {
        table?.get(name)?.as_table()
    }

    fn dependency_version(spec: &toml::Value) -> Option<&str> {
        spec.as_str()
            .or_else(|| spec.as_table()?.get("version")?.as_str())
    }

    fn dependency_package(
        dep_name: &str,
        spec: &toml::Value,
        workspace_deps: &toml::map::Map<String, toml::Value>,
    ) -> String {
        dependency_package_inner(dep_name, spec, workspace_deps, false)
    }

    fn dependency_package_inner(
        dep_name: &str,
        spec: &toml::Value,
        workspace_deps: &toml::map::Map<String, toml::Value>,
        resolving_workspace: bool,
    ) -> String {
        if let Some(table) = spec.as_table() {
            if let Some(package) = table.get("package").and_then(toml::Value::as_str) {
                return package.to_owned();
            }
            if !resolving_workspace
                && table_bool(table, "workspace")
                && let Some(workspace_spec) = workspace_deps.get(dep_name)
            {
                return dependency_package_inner(dep_name, workspace_spec, workspace_deps, true);
            }
        }

        dep_name.to_owned()
    }

    fn dependency_tables(doc: &toml::Value) -> Vec<(String, &toml::map::Map<String, toml::Value>)> {
        let mut tables = Vec::new();
        let Some(root) = doc.as_table() else {
            return tables;
        };

        for name in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(table) = child_table(Some(root), name) {
                tables.push((name.to_owned(), table));
            }
        }

        if let Some(targets) = child_table(Some(root), "target") {
            for (target_name, target_doc) in targets {
                let Some(target_table) = target_doc.as_table() else {
                    continue;
                };
                for name in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(table) = child_table(Some(target_table), name) {
                        tables.push((format!("target.{target_name}.{name}"), table));
                    }
                }
            }
        }

        tables
    }

    fn table_bool(table: &toml::map::Map<String, toml::Value>, name: &str) -> bool {
        table
            .get(name)
            .and_then(toml::Value::as_bool)
            .unwrap_or(false)
    }

    fn is_exact_requirement(requirement: &str) -> bool {
        let Some(rest) = requirement.strip_prefix('=') else {
            return false;
        };
        !rest.is_empty()
            && rest
                .chars()
                .all(|ch| !matches!(ch, '=' | ',' | ' ' | '\t' | '\n'))
    }

    fn rust_files(root: &Path) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        collect_rust_files(root, &mut files)?;
        files.sort();
        Ok(files)
    }

    fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        let entries = fs::read_dir(root)
            .map_err(|error| format!("failed to read {}: {error}", root.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|error| format!("failed to read entry in {}: {error}", root.display()))?
                .path();
            if path.is_dir() {
                collect_rust_files(&path, files)?;
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
        Ok(())
    }

    fn has_direct_burn_reference(line: &str) -> bool {
        let code = line.split("//").next().unwrap_or(line);
        let trimmed = code.trim_start();
        trimmed.starts_with("use burn::")
            || trimmed.starts_with("pub use burn::")
            || trimmed.starts_with("extern crate burn")
            || trimmed.starts_with("burn::")
            || code.contains(" burn::")
            || code.contains("= burn::")
            || code.contains("<burn::")
            || code.contains("(burn::")
            || code.contains("[burn::")
            || code.contains(", burn::")
            || code.contains(": burn::")
    }

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("gbf-train crate should live directly under the workspace root")
            .to_path_buf()
    }

    fn display_path(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn expect_fixture_error(root: &Path, expected: &str) {
        let error = check_burn_static_boundaries(root)
            .expect_err("fixture should violate the Burn static boundary");
        assert!(
            error.contains(expected),
            "expected error containing {expected:?}, got {error:?}"
        );
    }

    struct FixtureRoot {
        root: PathBuf,
    }

    impl FixtureRoot {
        fn new(name: &str) -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "gbllm-burn-pin-{}-{suffix}-{name}",
                std::process::id()
            ));
            let fixture = Self { root };
            fixture.write_valid();
            fixture
        }

        fn path(&self) -> &Path {
            &self.root
        }

        fn write_valid(&self) {
            self.write(
                "Cargo.toml",
                r#"
[workspace.dependencies]
burn = { version = "=0.21.0-pre.3", default-features = false }
"#,
            );
            self.write(
                "gbf-train/Cargo.toml",
                r#"
[dependencies]
burn = { workspace = true, optional = true }
"#,
            );
            self.write(
                "gbf-model/Cargo.toml",
                r#"
[dependencies]
serde = "1"
"#,
            );
            self.write(
                "gbf-model/src/lib.rs",
                r#"
pub fn scalar_only() -> u32 {
    1
}
"#,
            );
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|error| {
                    panic!("failed to create {}: {error}", parent.display())
                });
            }
            fs::write(&path, contents)
                .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

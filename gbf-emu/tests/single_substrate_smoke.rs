mod common;

#[test]
#[ignore = "gbf-test will promote this to a workspace gate"]
fn only_gbf_emu_imports_gameroy_core_directly() {
    let workspace = common::workspace_file("Cargo.toml");
    assert!(workspace.contains("\"gbf-emu\""));

    for manifest in std::fs::read_dir(".")
        .expect("workspace dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("Cargo.toml"))
        .filter(|path| path.exists())
    {
        let text = std::fs::read_to_string(&manifest).expect("manifest readable");
        if !manifest.ends_with("gbf-emu/Cargo.toml") {
            assert!(
                !text.contains("gameroy-core"),
                "{} imports gameroy-core directly",
                manifest.display()
            );
        }
    }
}

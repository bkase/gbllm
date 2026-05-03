use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let lock_path = manifest_dir
        .parent()
        .expect("workspace root")
        .join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());

    let lock = fs::read_to_string(&lock_path).expect("workspace Cargo.lock is readable");
    let package = find_package_section(&lock, "gameroy-core")
        .expect("Cargo.lock contains the gameroy-core package");
    let version = find_field(package, "version").expect("gameroy-core version in Cargo.lock");
    let source = find_field(package, "source").expect("gameroy-core git source in Cargo.lock");
    let rev = source
        .rsplit('#')
        .next()
        .filter(|rev| rev.len() == 40)
        .expect("gameroy-core source includes a resolved git revision");

    println!("cargo:rustc-env=GBF_EMU_GAMEROY_GIT_REV={rev}");
    println!("cargo:rustc-env=GBF_EMU_GAMEROY_VERSION={version}");
}

fn find_package_section<'a>(lock: &'a str, package_name: &str) -> Option<&'a str> {
    lock.split("[[package]]")
        .find(|section| find_field(section, "name") == Some(package_name))
}

fn find_field<'a>(section: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field} = \"");
    section
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix)?.strip_suffix('"'))
}

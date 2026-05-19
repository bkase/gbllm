use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::process::ExitCode;

use gbf_artifact::LexicalSpec_v1;
use gbf_data::{
    SplitRole, TinyStoriesV2Manifest, read_tinystories_manifest, read_tinystories_v2_manifest,
    verify_tinystories_v2_manifest,
};
use gbf_foundation::{Hash256, sha256};
use serde_json::json;

fn main() -> ExitCode {
    let args = env::args().collect::<Vec<_>>();
    let manifest_path = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("fixtures/corpora/tinystories.v2.toml");
    let ndjson_path = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("/tmp/s3-manifest-verify.json");

    match run(manifest_path, ndjson_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = std::fs::write(
                ndjson_path,
                format!(
                    "{}\n",
                    json!({
                        "event_name": "s3::manifest_verify",
                        "manifest_path": manifest_path,
                        "status": "error",
                        "error": error.to_string(),
                    })
                ),
            );
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(manifest_path: &str, ndjson_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = read_tinystories_v2_manifest(manifest_path)?;
    let verification = verify_tinystories_v2_manifest(&manifest)?;

    let source_manifest =
        read_tinystories_manifest(manifest.resolve_path(&manifest.source_manifest_path))?;
    let fixture_raw_train = std::fs::read(manifest.resolve_path(&manifest.fixture_raw_train_path))?;
    let fixture_raw_val = std::fs::read(manifest.resolve_path(&manifest.fixture_raw_val_path))?;
    let chapter = std::fs::read(manifest.resolve_path(&manifest.held_out_chapter_path))?;
    let records = [
        record(
            manifest_path,
            &manifest,
            "raw_train_sha256",
            manifest.raw_train_sha256,
            source_manifest.file(SplitRole::Train).sha256,
        ),
        record(
            manifest_path,
            &manifest,
            "raw_val_sha256",
            manifest.raw_val_sha256,
            source_manifest.file(SplitRole::Validation).sha256,
        ),
        record(
            manifest_path,
            &manifest,
            "fixture_raw_train_sha256",
            manifest.fixture_raw_train_sha256,
            sha256(&fixture_raw_train),
        ),
        record(
            manifest_path,
            &manifest,
            "fixture_raw_val_sha256",
            manifest.fixture_raw_val_sha256,
            sha256(&fixture_raw_val),
        ),
        record(
            manifest_path,
            &manifest,
            "train_post_sha256",
            manifest.train_post_sha256,
            sha256(verification.train_post.as_slice()),
        ),
        record(
            manifest_path,
            &manifest,
            "val_post_sha256",
            manifest.val_post_sha256,
            sha256(verification.val_post.as_slice()),
        ),
        record(
            manifest_path,
            &manifest,
            "charset_v1_sha256",
            manifest.charset_v1_sha256,
            LexicalSpec_v1::pinned().lexical_self_hash,
        ),
        record(
            manifest_path,
            &manifest,
            "chapter_sha256",
            manifest.chapter_sha256,
            sha256(&chapter),
        ),
    ];

    let mut writer = BufWriter::new(File::create(ndjson_path)?);
    for record in records {
        writeln!(writer, "{record}")?;
    }
    Ok(())
}

fn record(
    manifest_path: &str,
    manifest: &TinyStoriesV2Manifest,
    field: &str,
    expected: Hash256,
    observed: Hash256,
) -> serde_json::Value {
    json!({
        "event_name": "s3::manifest_verify",
        "manifest_path": manifest_path,
        "status": if expected == observed { "ok" } else { "mismatch" },
        "fixture_mode": manifest.fixture_mode,
        "field": field,
        "expected": expected.to_string(),
        "observed": observed.to_string(),
    })
}

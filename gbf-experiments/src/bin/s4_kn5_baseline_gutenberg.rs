//! F-S4 §D7 (bd-2nca): Kneser-Ney 5-gram baseline on the committed Gutenberg
//! corpus, emitting `s4_baseline_gutenberg.v1`.
//!
//! What this program does, end to end:
//!
//!   1. Reads the committed concatenated Gutenberg train stream
//!      (`corpus/gutenberg/gutenberg_train_concatenated.bin`) and, for each
//!      configured `--train-cap-bytes`, takes the front prefix (trimmed back
//!      to a UTF-8 character boundary when the cap splits a code point).
//!   2. Assembles the held-out validation byte stream from the
//!      `corpus/gutenberg/splits.json` val-split book bodies using the exact
//!      same logic (same book order, same 1 MiB default cap) as
//!      `s2_gap_and_export.rs`, so the KN baseline is scored on the identical
//!      book-level val stream the dense trainer used for
//!      `experiments/S2/gap/gap.json`.
//!   3. Normalizes both streams through the pinned `charset_v1` pipeline
//!      (`gbf_data::charset_v1::normalize_raw`) into 80-id token streams.
//!   4. Fits the F-S3-owned interpolated modified Kneser-Ney 5-gram baseline
//!      (Chen-Goodman D-rule discounts d_1/d_2/d_3+ per order 2..=5;
//!      left-continuation effective counts for orders 2..=4, raw counts at
//!      order 5, left-continuation unigram) via
//!      `gbf_experiments::s4::baseline::s4_fit_kn5_gutenberg` and scores val
//!      with the S1/S3 reset-context windowed-bpc primitive
//!      (chunk_size = 128).
//!   5. Writes one canonical `s4_baseline_gutenberg.v1` report per train cap,
//!      copies the largest-cap report to the promotion-gate paths, and writes
//!      a run-metadata JSON with git sha, corpus SHA-256s, discounts, wall
//!      clock, and bits-per-raw-byte conversions.
//!
//! Unit note: `bpc_kn5` is bits per *normalized charset_v1 token*. The run
//! metadata also records `kn5_bits_per_raw_val_byte`
//! (= bpc_kn5 * val_chars / val_raw_bytes) so the number can be laid next to
//! the neural byte-level bpc figures (e.g. `experiments/S2/gap/gap.json`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s4::baseline::{
    S4BaselineGutenbergReport, S4BaselineInputs, s4_fit_kn5_gutenberg,
};
use gbf_foundation::{Hash256, sha256};

/// The `val_bytes_sha256` recorded by the dense trainer in
/// `experiments/S2/gap/gap.json`. Used only to *report* whether this run's
/// assembled val stream is byte-identical to the one the neural numbers were
/// measured on; a mismatch is recorded, not fatal.
const S2_GAP_VAL_SHA256_HEX: &str =
    "e31abc36189a319d452ba2516e1944bc0e82ea31fb6cb4c4e5a4ce997b8d8e70";

#[derive(Debug, Parser)]
#[command(about = "F-S4 D7 KN-5 Gutenberg baseline + s4_baseline_gutenberg.v1 emitter (bd-2nca)")]
struct Args {
    /// Repository root containing corpus/ and experiments/.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Committed concatenated Gutenberg train stream, relative to repo root.
    #[arg(
        long,
        default_value = "corpus/gutenberg/gutenberg_train_concatenated.bin"
    )]
    train_bin: PathBuf,
    /// Front-prefix caps (bytes) on the train stream; one fit per cap.
    /// Defaults to 64 MiB (the dense trainer's train_cap_bytes) and 256 MiB.
    #[arg(long = "train-cap-bytes")]
    train_cap_bytes: Vec<u64>,
    /// Cap on held-out validation bytes assembled from val-split book bodies.
    #[arg(long, default_value_t = 1024 * 1024)]
    val_cap_bytes: usize,
    /// Output directory for baseline artifacts, relative to repo root.
    #[arg(long, default_value = "experiments/S4/baseline")]
    out_dir: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let total_start = Instant::now();
    let mut args = Args::parse();
    if args.train_cap_bytes.is_empty() {
        args.train_cap_bytes = vec![64 * 1024 * 1024, 256 * 1024 * 1024];
    }
    args.train_cap_bytes.sort_unstable();
    args.train_cap_bytes.dedup();

    let repo_root = args.repo_root.clone();
    let git_sha = git_head_sha(&repo_root);

    // Lineage hashes required by the s4_baseline_gutenberg.v1 schema.
    let gutenberg_manifest_self_hash = read_hash_field(
        &repo_root.join("experiments/S4/corpus/gutenberg-manifest.json"),
        "manifest_self_hash",
    )?;
    let tinystories_manifest_self_hash = read_hash_field(
        &repo_root.join("experiments/S4/corpus_quality/corpus_quality.json"),
        "tinystories_manifest_self_hash",
    )?;

    // Train stream.
    let train_bin_path = repo_root.join(&args.train_bin);
    let train_all = fs::read(&train_bin_path)
        .map_err(|e| format!("read train bin {}: {e}", train_bin_path.display()))?;
    let train_full_sha = sha256(&train_all);
    eprintln!(
        "[data] train bin {} bytes, sha256 {}",
        train_all.len(),
        train_full_sha.to_hex()
    );

    // Validation stream: byte-identical assembly to s2_gap_and_export.rs.
    let (val_bytes, val_book_ids) = build_val_bytes(&repo_root, args.val_cap_bytes)?;
    let val_raw_sha = sha256(&val_bytes);
    let val_matches_s2_gap_stream = val_raw_sha.to_hex() == S2_GAP_VAL_SHA256_HEX;
    eprintln!(
        "[data] val {} bytes from books {:?}, sha256 {} (matches gap.json val stream: {})",
        val_bytes.len(),
        val_book_ids,
        val_raw_sha.to_hex(),
        val_matches_s2_gap_stream
    );

    let (val_utf8, val_bytes_trimmed) = utf8_prefix(&val_bytes, val_bytes.len())?;
    let val_norm = normalize_raw(val_utf8)?;
    let val_chars = val_norm.tokens.len();
    let corpus_val_sha = sha256(val_norm.tokens.as_slice());
    eprintln!(
        "[data] val normalized: {} charset_v1 tokens ({} unk, {} raw bytes trimmed at utf8 tail)",
        val_chars, val_norm.unk_count_in_example, val_bytes_trimmed
    );

    let out_dir = repo_root.join(&args.out_dir);
    fs::create_dir_all(&out_dir)?;

    let mut runs = Vec::new();
    let mut primary: Option<(u64, S4BaselineGutenbergReport, PathBuf)> = None;
    for &cap in &args.train_cap_bytes {
        let cap_usize = usize::try_from(cap)?;
        let fit_start = Instant::now();
        let (train_prefix, train_bytes_trimmed) = utf8_prefix(&train_all, cap_usize)?;
        let train_bytes_used = train_prefix.len();
        let train_prefix_raw_sha = sha256(train_prefix);
        let train_norm = normalize_raw(train_prefix)?;
        let train_chars = train_norm.tokens.len();
        let corpus_train_sha = sha256(train_norm.tokens.as_slice());
        eprintln!(
            "[fit] cap {} bytes: using {} raw bytes -> {} charset_v1 tokens ({} unk); fitting KN-5",
            cap, train_bytes_used, train_chars, train_norm.unk_count_in_example
        );

        let report = s4_fit_kn5_gutenberg(S4BaselineInputs {
            tinystories_manifest_self_hash,
            gutenberg_manifest_self_hash,
            corpus_train_sha,
            corpus_val_sha,
            corpus_train: train_norm.tokens,
            corpus_val: val_norm.tokens.clone(),
        })?;
        let fit_and_score_seconds = fit_start.elapsed().as_secs_f64();

        let report_path = out_dir.join(format!(
            "s4_baseline_gutenberg.v1.train-cap-{}MiB.json",
            cap / (1024 * 1024)
        ));
        fs::write(&report_path, report.canonical_bytes()?)?;
        eprintln!(
            "[fit] cap {}: bpc_kn5={:.6} bpc_kn3={:.6} bpc_unigram={:.6} ({:.1}s) -> {}",
            cap,
            report.bpc_kn5,
            report.bpc_kn3,
            report.bpc_unigram,
            fit_and_score_seconds,
            report_path.display()
        );

        let val_raw_bytes = val_utf8.len() as f64;
        runs.push(serde_json::json!({
            "train_cap_bytes": cap,
            "train_bytes_used": train_bytes_used,
            "train_bytes_trimmed_at_utf8_boundary": train_bytes_trimmed,
            "train_prefix_raw_sha256": train_prefix_raw_sha.to_hex(),
            "train_chars_normalized": train_chars,
            "train_unk_count": train_norm.unk_count_in_example,
            "corpus_train_sha_normalized": report.corpus_train_sha.to_string(),
            "bpc_kn5_val_per_normalized_char": report.bpc_kn5,
            "bpc_kn3_val_per_normalized_char": report.bpc_kn3,
            "bpc_unigram_val_per_normalized_char": report.bpc_unigram,
            "kn5_bits_per_raw_val_byte": report.bpc_kn5 * val_chars as f64 / val_raw_bytes,
            "kn3_bits_per_raw_val_byte": report.bpc_kn3 * val_chars as f64 / val_raw_bytes,
            "kn_discounts_by_order": report.kn_params.discounts,
            "counts_summary": report.counts_summary,
            "counts_blob_sha256": report.counts_blob_sha256.to_string(),
            "baseline_gutenberg_self_hash": report.baseline_gutenberg_self_hash.to_string(),
            "report_path": rel_display(&report_path, &repo_root),
            "fit_and_score_wall_clock_seconds": fit_and_score_seconds,
        }));

        if primary.as_ref().is_none_or(|(best, _, _)| cap > *best) {
            primary = Some((cap, report, report_path));
        }
    }

    let (primary_cap, primary_report, primary_path) =
        primary.ok_or("at least one --train-cap-bytes run is required")?;

    // Primary artifact under the bead-specified name, plus the path the
    // corpus_quality kn_baseline_pointer / `gbf s4 promote` examples expect.
    let canonical = primary_report.canonical_bytes()?;
    let primary_out = out_dir.join("s4_baseline_gutenberg.v1.json");
    let pointer_out = out_dir.join("baseline_gutenberg.json");
    fs::write(&primary_out, &canonical)?;
    fs::write(&pointer_out, &canonical)?;

    let neural_reference = read_neural_reference(&repo_root);
    let run_meta = serde_json::json!({
        "schema": "s4_baseline_gutenberg_run_meta.v1",
        "bead": "bd-2nca",
        "git_sha": git_sha,
        "kn_variant": {
            "family": "interpolated modified Kneser-Ney (Chen-Goodman), inherited unchanged from F-S3",
            "max_order": 5,
            "vocab": "charset_v1 (80 ids; case-preserving, accent-stripped, quote/dash-folded, whitespace-collapsed, unmappable -> <unk>)",
            "effective_counts": "orders 2..4 left-continuation counts, order 5 raw counts, unigram = left-continuation distribution",
            "discounts": "per-order D-rule d_1/d_2/d_3+ fit from count-of-counts (values recorded per run)",
            "scoring": "reset-context windowed bpc, chunk_size = 128, no <bos>/<eos>, contexts do not cross chunk resets",
        },
        "corpus": {
            "train_bin_path": rel_display(&train_bin_path, &repo_root),
            "train_bin_total_bytes": train_all.len(),
            "train_bin_sha256": train_full_sha.to_hex(),
            "gutenberg_manifest_self_hash": gutenberg_manifest_self_hash.to_string(),
            "tinystories_manifest_self_hash": tinystories_manifest_self_hash.to_string(),
        },
        "val_stream": {
            "source": "corpus/gutenberg/splits.json val-split book bodies (book-level held out), assembled with the same logic and 1 MiB cap as s2_gap_and_export.rs",
            "val_book_ids_used": val_book_ids,
            "val_cap_bytes": args.val_cap_bytes,
            "val_raw_bytes_used": val_utf8.len(),
            "val_raw_bytes_sha256": val_raw_sha.to_hex(),
            "matches_s2_gap_val_stream": val_matches_s2_gap_stream,
            "s2_gap_val_bytes_sha256": S2_GAP_VAL_SHA256_HEX,
            "val_bytes_trimmed_at_utf8_boundary": val_bytes_trimmed,
            "val_chars_normalized": val_chars,
            "val_unk_count": val_norm.unk_count_in_example,
            "corpus_val_sha_normalized": corpus_val_sha.to_string(),
        },
        "runs": runs,
        "primary": {
            "train_cap_bytes": primary_cap,
            "artifact_path": rel_display(&primary_out, &repo_root),
            "pointer_copy_path": rel_display(&pointer_out, &repo_root),
            "source_report_path": rel_display(&primary_path, &repo_root),
            "baseline_gutenberg_self_hash": primary_report.baseline_gutenberg_self_hash.to_string(),
        },
        "neural_reference": neural_reference,
        "caveats": [
            "bpc_kn5 is bits per normalized charset_v1 token, not per raw byte; charset_v1 normalization (whitespace collapse, accent strip, quote/dash folding, unmappable -> <unk>) removes some information, which mildly favors the KN side in per-char terms.",
            "kn5_bits_per_raw_val_byte re-expresses the same total val bits over the raw byte count of the normalization input and is the number to lay next to byte-level neural bpc (gap.json, S7 runs).",
            "Each run fits on a front prefix of the committed train stream (train_cap_bytes / train_bytes_used recorded per run); a run whose train_bytes_used equals train_bin_total_bytes covers the full committed stream. The 64 MiB run's train prefix is byte-identical (same sha256) to the dense trainer's capped train stream in gap.json.",
            "Wall-clock and git_sha make this file non-byte-deterministic across reruns; the per-cap s4_baseline_gutenberg.v1 reports themselves are deterministic and self-hashed.",
        ],
        "total_wall_clock_seconds": total_start.elapsed().as_secs_f64(),
    });
    let run_meta_path = out_dir.join("s4_baseline_gutenberg_run_meta.json");
    fs::write(&run_meta_path, serde_json::to_vec_pretty(&run_meta)?)?;

    eprintln!(
        "[done] primary (train cap {} bytes) bpc_kn5={:.6} -> {} (+ pointer copy {}), run meta {} ({:.1}s total)",
        primary_cap,
        primary_report.bpc_kn5,
        primary_out.display(),
        pointer_out.display(),
        run_meta_path.display(),
        total_start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Render a path relative to the repo root when possible, for stable
/// artifact-internal path strings.
fn rel_display(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Resolve `git rev-parse HEAD`, or `"unknown"` when git is unavailable.
fn git_head_sha(repo_root: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Read a `sha256:...` string field from the top level of a JSON artifact.
fn read_hash_field(path: &Path, field: &str) -> Result<Hash256, Box<dyn std::error::Error>> {
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?,
    )?;
    let raw = value[field]
        .as_str()
        .ok_or_else(|| format!("{} missing string field {field}", path.display()))?;
    Ok(raw
        .parse::<Hash256>()
        .map_err(|e| format!("{} field {field}: {e:?}", path.display()))?)
}

/// Longest prefix of `bytes[..cap]` that is valid UTF-8, plus how many bytes
/// were trimmed from a code point split at the cap. Invalid UTF-8 anywhere
/// other than the truncated tail is a hard error.
fn utf8_prefix(bytes: &[u8], cap: usize) -> Result<(&[u8], usize), Box<dyn std::error::Error>> {
    let take = bytes.len().min(cap);
    let slice = &bytes[..take];
    match std::str::from_utf8(slice) {
        Ok(_) => Ok((slice, 0)),
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            Ok((&slice[..valid], take - valid))
        }
        Err(error) => Err(format!("train/val stream is not valid UTF-8: {error}").into()),
    }
}

/// Build the held-out validation byte stream from the val-split book bodies.
/// Byte-for-byte the same assembly as `s2_gap_and_export.rs::build_val_bytes`.
fn build_val_bytes(
    repo_root: &Path,
    cap: usize,
) -> Result<(Vec<u8>, Vec<u64>), Box<dyn std::error::Error>> {
    let splits_path = repo_root.join("corpus/gutenberg/splits.json");
    let splits: serde_json::Value = serde_json::from_slice(
        &fs::read(&splits_path).map_err(|e| format!("read {}: {e}", splits_path.display()))?,
    )?;
    let val_ids = splits["val"]
        .as_array()
        .ok_or("splits.json missing val array")?
        .iter()
        .filter_map(|v| v.as_u64())
        .collect::<Vec<_>>();

    let mut bytes = Vec::with_capacity(cap.min(4 * 1024 * 1024));
    let mut used_ids = Vec::new();
    for id in &val_ids {
        if bytes.len() >= cap {
            break;
        }
        let body_path = repo_root
            .join("corpus/gutenberg/bodies")
            .join(id.to_string())
            .join("body.txt");
        let Ok(body) = fs::read(&body_path) else {
            continue;
        };
        if body.is_empty() {
            continue;
        }
        used_ids.push(*id);
        let remaining = cap - bytes.len();
        bytes.extend_from_slice(&body[..body.len().min(remaining)]);
    }
    if bytes.len() < 2 {
        return Err("assembled validation stream is too small".into());
    }
    Ok((bytes, used_ids))
}

/// Copy the already-committed neural bpc reference numbers out of
/// `experiments/S2/gap/gap.json` (when present) so the baseline artifact
/// carries its own comparison context without fabricating anything.
fn read_neural_reference(repo_root: &Path) -> serde_json::Value {
    let gap_path = repo_root.join("experiments/S2/gap/gap.json");
    let Ok(raw) = fs::read(&gap_path) else {
        return serde_json::json!({ "status": "experiments/S2/gap/gap.json not found" });
    };
    let Ok(gap) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return serde_json::json!({ "status": "experiments/S2/gap/gap.json unparseable" });
    };
    serde_json::json!({
        "source": "experiments/S2/gap/gap.json (copied verbatim, bits per raw byte on the same val stream)",
        "dense_ternary_val_bpc": gap["measurement"]["ternary_val_bpc"],
        "dense_fp_val_bpc": gap["measurement"]["fp_val_bpc"],
        "train_cap_bytes": gap["corpus"]["train_cap_bytes"],
        "val_bytes_sha256": gap["corpus"]["val_bytes_sha256"],
    })
}

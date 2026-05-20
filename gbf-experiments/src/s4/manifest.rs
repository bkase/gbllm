//! S4 Gutenberg corpus manifest assembly.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use flate2::read::GzDecoder;
use gbf_artifact::{
    BOS_ID, EOS_ID, GutenbergCompressionKind, GutenbergDedupPolicy, GutenbergDropReason,
    GutenbergFetchNamespaceKind, GutenbergManifest, GutenbergManifestError, GutenbergSourceRecord,
    GutenbergSplit, LexicalSpec_v1, canonical_gutenberg_manifest_bytes,
};
use gbf_data::{
    GUTENBERG_D3_FOOTER_MARKER_REGEX_PATTERN, GUTENBERG_D3_HEADER_REGEX_PATTERN,
    GutenbergD3DropReason, empty_after_strip_reason, marker_missing_drop_cap_breached,
    normalize_raw, read_tinystories_manifest, strip_gutenberg_d3,
};
use gbf_foundation::{CanonicalJson, DomainHash, Hash256, Hash256ParseError, sha256};
use serde::Deserialize;
use zip::ZipArchive;

use crate::s4::corpus_quality::{
    GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX, S4CorpusQuality, S4CorpusQualityError,
    S4PerCorpusQuality, S4UnmappableGateError, verify_gutenberg_unmappable_gate_with_max,
};
use crate::s4::schema::S4_SEQUENCE_LENGTH;

/// Default fixture pin consumed by the network-disabled build.
pub const DEFAULT_GUTENBERG_FIXTURE_PATH: &str = "fixtures/corpora/gutenberg.toml";
/// Default emitted manifest path from the F-S4 RFC.
pub const DEFAULT_GUTENBERG_MANIFEST_PATH: &str = "experiments/S4/corpus/gutenberg-manifest.json";
/// Default train token stream path from the F-S4 RFC.
pub const DEFAULT_GUTENBERG_TRAIN_PATH: &str = "experiments/S4/corpus/gutenberg-train.bin";
/// Default validation token stream path from the F-S4 RFC.
pub const DEFAULT_GUTENBERG_VAL_PATH: &str = "experiments/S4/corpus/gutenberg-val.bin";
/// Default corpus-quality artifact path from the F-S4 RFC.
pub const DEFAULT_S4_CORPUS_QUALITY_PATH: &str =
    "experiments/S4/corpus_quality/corpus_quality.json";
/// Default TinyStories manifest used only for the corpus-quality pointer hash.
pub const DEFAULT_TINYSTORIES_MANIFEST_PATH: &str = "fixtures/corpora/tinystories.toml";

const S4_SPLIT_HASH_PREFIX: &[u8] = b"gbf:s4:book-split:v1";
const S4_BUILD_CORPUS_LOG_TARGET: &str = "gbf_experiments::s4::manifest";
const TINYSTORIES_MANIFEST_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-data",
    "TinyStoriesManifest",
    "tinystories_manifest.v1",
    "1.0.0",
);

/// Options for `gbf s4 build-corpus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutenbergBuildOptions {
    /// Network-disabled fixture TOML path.
    pub fixture_path: PathBuf,
    /// Destination canonical `gutenberg_manifest.v1` JSON path.
    pub manifest_path: PathBuf,
    /// Destination Gutenberg train token-id stream.
    pub train_path: PathBuf,
    /// Destination Gutenberg validation token-id stream.
    pub val_path: PathBuf,
    /// Optional destination `s4_corpus_quality.v1` JSON path.
    pub corpus_quality_path: Option<PathBuf>,
    /// Optional TinyStories manifest path used only for corpus-quality hashing.
    pub tinystories_manifest_path: Option<PathBuf>,
}

impl Default for GutenbergBuildOptions {
    fn default() -> Self {
        Self {
            fixture_path: DEFAULT_GUTENBERG_FIXTURE_PATH.into(),
            manifest_path: DEFAULT_GUTENBERG_MANIFEST_PATH.into(),
            train_path: DEFAULT_GUTENBERG_TRAIN_PATH.into(),
            val_path: DEFAULT_GUTENBERG_VAL_PATH.into(),
            corpus_quality_path: Some(DEFAULT_S4_CORPUS_QUALITY_PATH.into()),
            tinystories_manifest_path: Some(DEFAULT_TINYSTORIES_MANIFEST_PATH.into()),
        }
    }
}

/// In-memory artifacts produced by the network-disabled assembler.
#[derive(Debug, Clone, PartialEq)]
pub struct GutenbergBuildArtifacts {
    /// Canonical Gutenberg manifest object with computed self-hash.
    pub manifest: GutenbergManifest,
    /// Optional corpus-quality artifact with computed self-hash.
    pub corpus_quality: Option<S4CorpusQuality>,
    /// Train split token-id bytes, including BOS/EOS book boundaries.
    pub train_bytes: Vec<u8>,
    /// Validation split token-id bytes, including BOS/EOS book boundaries.
    pub val_bytes: Vec<u8>,
}

/// Write-summary returned by `build_gutenberg_corpus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutenbergBuildSummary {
    /// Path that received `gutenberg_manifest.v1`.
    pub manifest_path: PathBuf,
    /// Path that received the train token-id stream.
    pub train_path: PathBuf,
    /// Path that received the validation token-id stream.
    pub val_path: PathBuf,
    /// Path that received `s4_corpus_quality.v1`, when enabled.
    pub corpus_quality_path: Option<PathBuf>,
    /// Manifest self-hash written to stdout by the CLI.
    pub manifest_self_hash: Hash256,
    /// Corpus-quality self-hash, when emitted.
    pub corpus_quality_self_hash: Option<Hash256>,
    /// SHA-256 of the train stream.
    pub train_sha256: Hash256,
    /// SHA-256 of the validation stream.
    pub val_sha256: Hash256,
    /// Train stream byte length.
    pub train_byte_length: u64,
    /// Validation stream byte length.
    pub val_byte_length: u64,
    /// Number of retained train books.
    pub train_book_count: u32,
    /// Number of retained validation books.
    pub val_book_count: u32,
    /// Number of dropped selected source records.
    pub drop_count_total: u32,
}

/// Assemble and write the S4 Gutenberg corpus artifacts.
pub fn build_gutenberg_corpus(
    options: &GutenbergBuildOptions,
) -> Result<GutenbergBuildSummary, GutenbergBuildError> {
    let artifacts = assemble_gutenberg_corpus(options)?;

    write_bytes_if_changed(&options.train_path, &artifacts.train_bytes)?;
    write_bytes_if_changed(&options.val_path, &artifacts.val_bytes)?;
    write_bytes_if_changed(
        &options.manifest_path,
        &canonical_gutenberg_manifest_bytes(&artifacts.manifest)?,
    )?;
    if let (Some(path), Some(quality)) = (&options.corpus_quality_path, &artifacts.corpus_quality) {
        write_bytes_if_changed(path, &quality.canonical_bytes()?)?;
    }

    Ok(GutenbergBuildSummary {
        manifest_path: options.manifest_path.clone(),
        train_path: options.train_path.clone(),
        val_path: options.val_path.clone(),
        corpus_quality_path: options.corpus_quality_path.clone(),
        manifest_self_hash: artifacts.manifest.manifest_self_hash,
        corpus_quality_self_hash: artifacts
            .corpus_quality
            .as_ref()
            .map(|quality| quality.corpus_quality_self_hash),
        train_sha256: artifacts.manifest.train_sha256,
        val_sha256: artifacts.manifest.val_sha256,
        train_byte_length: artifacts.manifest.train_byte_length,
        val_byte_length: artifacts.manifest.val_byte_length,
        train_book_count: artifacts.manifest.train_book_count,
        val_book_count: artifacts.manifest.val_book_count,
        drop_count_total: artifacts.manifest.drop_count_total,
    })
}

/// Assemble the S4 Gutenberg corpus artifacts without writing them.
pub fn assemble_gutenberg_corpus(
    options: &GutenbergBuildOptions,
) -> Result<GutenbergBuildArtifacts, GutenbergBuildError> {
    let fixture_text = std::fs::read_to_string(&options.fixture_path).map_err(|source| {
        GutenbergBuildError::Io {
            path: options.fixture_path.display().to_string(),
            source,
        }
    })?;
    let fixture: GutenbergFixture =
        toml::from_str(&fixture_text).map_err(GutenbergBuildError::Toml)?;
    validate_fixture_header(&fixture)?;

    let fixture_root = fixture_root(&options.fixture_path)?;
    let fixture_sources = sorted_fixture_sources(fixture.sources.clone())?;
    let book_ids = fixture_book_ids(&fixture, &fixture_sources)?;
    tracing::info!(
        target: S4_BUILD_CORPUS_LOG_TARGET,
        event_name = "s4_build_corpus_started",
        fixture_path = %options.fixture_path.display(),
        book_count = book_ids.len() as u64,
        network_policy = "disabled",
        "S4 Gutenberg build-corpus started"
    );
    let fixture_by_book_id = fixture_sources
        .into_iter()
        .map(|source| (source.book_id, source))
        .collect::<BTreeMap<_, _>>();

    let mut processed = Vec::with_capacity(book_ids.len());
    for book_id in &book_ids {
        let source = fixture_by_book_id
            .get(book_id)
            .ok_or(GutenbergBuildError::FixtureSourceMissingBookId { book_id: *book_id })?;
        processed.push(process_source(source, &fixture_root)?);
    }

    apply_splits(&mut processed, &fixture)?;
    apply_exact_body_dedup(&mut processed);
    let (train_bytes, val_bytes) = build_split_streams(&processed);
    let manifest = build_manifest(
        &fixture,
        book_ids,
        processed
            .iter()
            .map(|source| source.record.clone())
            .collect(),
        &train_bytes,
        &val_bytes,
        options,
    )?;
    let corpus_quality = build_corpus_quality(&manifest, &processed, options)?;

    Ok(GutenbergBuildArtifacts {
        manifest,
        corpus_quality,
        train_bytes,
        val_bytes,
    })
}

#[derive(Debug, Deserialize)]
struct GutenbergFixture {
    schema: String,
    source_name: String,
    catalog_snapshot: Option<FixtureCatalogSnapshot>,
    selection_filter: Option<FixtureSelectionFilter>,
    split: Option<FixtureSplit>,
    markers: Option<FixtureMarkers>,
    guards: Option<FixtureGuards>,
    book_ids: Option<FixtureBookIds>,
    sources: Vec<FixtureSource>,
}

#[derive(Debug, Deserialize)]
struct FixtureCatalogSnapshot {
    url: String,
    sha256: String,
    observed_at_utc: String,
    last_modified_utc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FixtureSelectionFilter {
    canonical_json: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct FixtureSplit {
    split_seed_u128_hex: Option<String>,
    train_fraction: Option<f64>,
    val_fraction: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct FixtureMarkers {
    header_regex_pattern: String,
    footer_regex_pattern: String,
}

#[derive(Debug, Deserialize)]
struct FixtureGuards {
    retained_book_count_min: Option<u32>,
    unmappable_rate_corpus_max: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct FixtureBookIds {
    values: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureSource {
    book_id: u32,
    title: Option<String>,
    author: Option<String>,
    source_landing_url: String,
    mirror_fetch_url: Option<String>,
    source_blob_sha256: Option<String>,
    source_blob_size_bytes: Option<u64>,
    pre_strip_utf8_sha256: Option<String>,
    pre_strip_utf8_size_bytes: Option<u64>,
    media_type: Option<String>,
    charset: Option<String>,
    selected_format: Option<String>,
    compression_kind: Option<String>,
    archive_member_path: Option<String>,
    fetch_namespace_kind: Option<String>,
    fetch_namespace_id: Option<String>,
    local_blob_path: Option<String>,
    drop_reason: Option<String>,
    duplicate_of_book_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
struct ProcessedSource {
    record: GutenbergSourceRecord,
    body_tokens: Vec<u8>,
}

fn validate_fixture_header(fixture: &GutenbergFixture) -> Result<(), GutenbergBuildError> {
    if fixture.schema != "gutenberg_fixture.v1" && fixture.schema != "gutenberg_smoke_fixture.v1" {
        return Err(GutenbergBuildError::InvalidFixtureSchema {
            observed: fixture.schema.clone(),
        });
    }
    if fixture.source_name != "Project Gutenberg" {
        return Err(GutenbergBuildError::InvalidFixtureSourceName {
            observed: fixture.source_name.clone(),
        });
    }
    Ok(())
}

fn sorted_fixture_sources(
    mut sources: Vec<FixtureSource>,
) -> Result<Vec<FixtureSource>, GutenbergBuildError> {
    sources.sort_by_key(|source| source.book_id);
    for pair in sources.windows(2) {
        if pair[0].book_id == pair[1].book_id {
            return Err(GutenbergBuildError::DuplicateFixtureBookId {
                book_id: pair[0].book_id,
            });
        }
    }
    Ok(sources)
}

fn fixture_book_ids(
    fixture: &GutenbergFixture,
    sources: &[FixtureSource],
) -> Result<Vec<u32>, GutenbergBuildError> {
    let book_ids = fixture
        .book_ids
        .as_ref()
        .map(|book_ids| book_ids.values.clone())
        .unwrap_or_else(|| sources.iter().map(|source| source.book_id).collect());
    if !book_ids.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(GutenbergBuildError::BookIdsNotStrictlySorted);
    }
    let source_ids = sources
        .iter()
        .map(|source| source.book_id)
        .collect::<Vec<_>>();
    if source_ids != book_ids {
        return Err(GutenbergBuildError::FixtureBookIdsMismatch);
    }
    Ok(book_ids)
}

fn process_source(
    source: &FixtureSource,
    fixture_root: &Path,
) -> Result<ProcessedSource, GutenbergBuildError> {
    if let Some(drop_reason) = &source.drop_reason {
        let drop_reason = parse_drop_reason(drop_reason)?;
        return Ok(ProcessedSource {
            record: base_record(source, None, None, None, None, Some(drop_reason)),
            body_tokens: Vec::new(),
        });
    }

    let source_blob_sha256 = source
        .source_blob_sha256
        .as_deref()
        .ok_or(GutenbergBuildError::MissingFixtureField {
            book_id: source.book_id,
            field: "source_blob_sha256",
        })
        .and_then(parse_unprefixed_or_prefixed_hash)?;
    let local_blob_path =
        source
            .local_blob_path
            .as_deref()
            .ok_or(GutenbergBuildError::MissingFixtureField {
                book_id: source.book_id,
                field: "local_blob_path",
            })?;
    let blob_path = resolve_fixture_path(fixture_root, local_blob_path);
    let blob = std::fs::read(&blob_path).map_err(|source| GutenbergBuildError::Io {
        path: blob_path.display().to_string(),
        source,
    })?;
    verify_blob_pin(source, &blob, source_blob_sha256)?;

    let decoded = match decode_source_blob(source, &blob) {
        Ok(decoded) => decoded,
        Err(DecodeDrop::NoPlaintextArchiveMember) => {
            return Ok(dropped_with_blob(
                source,
                source_blob_sha256,
                GutenbergDropReason::NoPlaintextArchiveMember,
            ));
        }
        Err(DecodeDrop::AmbiguousPlaintextArchive) => {
            return Ok(dropped_with_blob(
                source,
                source_blob_sha256,
                GutenbergDropReason::AmbiguousPlaintextArchive,
            ));
        }
        Err(DecodeDrop::SourceDecodeFailed) => {
            return Ok(dropped_with_blob(
                source,
                source_blob_sha256,
                GutenbergDropReason::SourceDecodeFailed,
            ));
        }
        Err(DecodeDrop::InvalidUtf8) => {
            return Ok(dropped_with_blob(
                source,
                source_blob_sha256,
                GutenbergDropReason::InvalidUtf8,
            ));
        }
    };
    let pre_strip_sha256 = sha256(&decoded);
    let pre_strip_byte_length =
        u64::try_from(decoded.len()).map_err(|_| GutenbergBuildError::CountOverflow {
            field: "pre_strip_byte_length",
        })?;
    verify_pre_strip_pin(source, &decoded, pre_strip_sha256, pre_strip_byte_length)?;

    let stripped = match strip_gutenberg_d3(&decoded) {
        Ok(stripped) => stripped,
        Err(GutenbergD3DropReason::SourceDecodeFailed) => {
            emit_strip_drop(source.book_id, GutenbergDropReason::SourceDecodeFailed);
            return Ok(dropped_after_decode(
                source,
                source_blob_sha256,
                pre_strip_sha256,
                pre_strip_byte_length,
                GutenbergDropReason::SourceDecodeFailed,
            ));
        }
        Err(GutenbergD3DropReason::InvalidUtf8) => {
            emit_strip_drop(source.book_id, GutenbergDropReason::InvalidUtf8);
            return Ok(dropped_after_decode(
                source,
                source_blob_sha256,
                pre_strip_sha256,
                pre_strip_byte_length,
                GutenbergDropReason::InvalidUtf8,
            ));
        }
        Err(GutenbergD3DropReason::GutenbergMarkerMissing) => {
            emit_strip_drop(source.book_id, GutenbergDropReason::GutenbergMarkerMissing);
            return Ok(dropped_after_decode(
                source,
                source_blob_sha256,
                pre_strip_sha256,
                pre_strip_byte_length,
                GutenbergDropReason::GutenbergMarkerMissing,
            ));
        }
        Err(GutenbergD3DropReason::EmptyAfterStrip) => {
            emit_strip_drop(source.book_id, GutenbergDropReason::EmptyAfterStrip);
            return Ok(dropped_after_decode(
                source,
                source_blob_sha256,
                pre_strip_sha256,
                pre_strip_byte_length,
                GutenbergDropReason::EmptyAfterStrip,
            ));
        }
    };

    let stats = normalize_raw(stripped.body.as_bytes()).map_err(GutenbergBuildError::Charset)?;
    let body_tokens = stats.tokens.as_slice().to_vec();
    let post_charset_token_length =
        u64::try_from(body_tokens.len()).map_err(|_| GutenbergBuildError::CountOverflow {
            field: "post_charset_token_length",
        })?;
    let unmappable_count = u64::from(stats.unk_count_in_example);
    let unmappable_density = if post_charset_token_length == 0 {
        0.0
    } else {
        unmappable_count as f64 / post_charset_token_length as f64
    };
    let post_strip_byte_length =
        u64::try_from(stripped.body.len()).map_err(|_| GutenbergBuildError::CountOverflow {
            field: "post_strip_byte_length",
        })?;

    let mut record = base_record(
        source,
        Some(source_blob_sha256),
        Some(pre_strip_sha256),
        Some(pre_strip_byte_length),
        Some(stripped.post_strip_sha256),
        None,
    );
    record.post_strip_byte_length = Some(post_strip_byte_length);
    record.post_charset_body_sha256 = Some(sha256(&body_tokens));
    record.post_charset_token_length = Some(post_charset_token_length);
    record.unmappable_count = Some(unmappable_count);
    record.unmappable_density = Some(unmappable_density);

    if let Some(reason) = empty_after_strip_reason(body_tokens.len()) {
        record.drop_reason = Some(match reason {
            GutenbergD3DropReason::EmptyAfterStrip => GutenbergDropReason::EmptyAfterStrip,
            _ => unreachable!("empty_after_strip_reason returns only EmptyAfterStrip"),
        });
        emit_charset_drop(
            record.book_id,
            GutenbergDropReason::EmptyAfterStrip,
            unmappable_density,
            post_charset_token_length,
            unmappable_count,
        );
        return Ok(ProcessedSource {
            record,
            body_tokens: Vec::new(),
        });
    }
    if stats.dropped {
        record.drop_reason = Some(GutenbergDropReason::UnmappableDensityHigh);
        emit_charset_drop(
            record.book_id,
            GutenbergDropReason::UnmappableDensityHigh,
            unmappable_density,
            post_charset_token_length,
            unmappable_count,
        );
        return Ok(ProcessedSource {
            record,
            body_tokens: Vec::new(),
        });
    }

    Ok(ProcessedSource {
        record,
        body_tokens,
    })
}

fn base_record(
    source: &FixtureSource,
    source_blob_sha256: Option<Hash256>,
    pre_strip_utf8_sha256: Option<Hash256>,
    pre_strip_byte_length: Option<u64>,
    post_strip_sha256: Option<Hash256>,
    drop_reason: Option<GutenbergDropReason>,
) -> GutenbergSourceRecord {
    GutenbergSourceRecord {
        book_id: source.book_id,
        title: source
            .title
            .clone()
            .unwrap_or_else(|| format!("Project Gutenberg Ebook {}", source.book_id)),
        author: source
            .author
            .clone()
            .unwrap_or_else(|| "Unknown".to_owned()),
        source_landing_url: source.source_landing_url.clone(),
        mirror_fetch_url: source.mirror_fetch_url.clone(),
        mirror_snapshot_id: None,
        selected_format: source.selected_format.clone().or_else(|| {
            source_blob_sha256
                .is_some()
                .then(|| default_selected_format(source))
        }),
        source_blob_sha256,
        pre_strip_utf8_sha256,
        license: GutenbergSourceRecord::public_domain_in_usa_license(),
        fetch_namespace_kind: source
            .fetch_namespace_kind
            .as_deref()
            .and_then(parse_fetch_namespace_kind),
        fetch_namespace_id: source.fetch_namespace_id.clone(),
        compression_kind: source
            .compression_kind
            .as_deref()
            .and_then(parse_compression_kind)
            .or_else(|| source_blob_sha256.map(|_| GutenbergCompressionKind::None)),
        archive_member_path: source.archive_member_path.clone(),
        pre_strip_byte_length,
        drop_reason,
        duplicate_of_book_id: source.duplicate_of_book_id,
        post_strip_byte_length: None,
        post_strip_sha256,
        post_charset_body_sha256: None,
        post_charset_token_length: None,
        unmappable_count: None,
        unmappable_density: None,
        split: None,
    }
}

fn dropped_with_blob(
    source: &FixtureSource,
    source_blob_sha256: Hash256,
    drop_reason: GutenbergDropReason,
) -> ProcessedSource {
    ProcessedSource {
        record: base_record(
            source,
            Some(source_blob_sha256),
            None,
            None,
            None,
            Some(drop_reason),
        ),
        body_tokens: Vec::new(),
    }
}

fn dropped_after_decode(
    source: &FixtureSource,
    source_blob_sha256: Hash256,
    pre_strip_utf8_sha256: Hash256,
    pre_strip_byte_length: u64,
    drop_reason: GutenbergDropReason,
) -> ProcessedSource {
    ProcessedSource {
        record: base_record(
            source,
            Some(source_blob_sha256),
            Some(pre_strip_utf8_sha256),
            Some(pre_strip_byte_length),
            None,
            Some(drop_reason),
        ),
        body_tokens: Vec::new(),
    }
}

fn emit_strip_drop(book_id: u32, reason: GutenbergDropReason) {
    tracing::info!(
        target: S4_BUILD_CORPUS_LOG_TARGET,
        event_name = "s4_strip_drop",
        book_id = book_id as u64,
        reason = ?reason,
        "S4 Gutenberg D3 strip dropped source"
    );
}

fn emit_charset_drop(
    book_id: u32,
    reason: GutenbergDropReason,
    unmappable_density: f64,
    post_charset_token_length: u64,
    unmappable_count: u64,
) {
    tracing::info!(
        target: S4_BUILD_CORPUS_LOG_TARGET,
        event_name = "s4_charset_drop",
        book_id = book_id as u64,
        reason = ?reason,
        unmappable_density,
        post_charset_token_length,
        unmappable_count,
        "S4 Gutenberg charset gate dropped source"
    );
}

fn verify_blob_pin(
    source: &FixtureSource,
    blob: &[u8],
    expected_sha256: Hash256,
) -> Result<(), GutenbergBuildError> {
    if let Some(expected_size) = source.source_blob_size_bytes
        && expected_size != blob.len() as u64
    {
        return Err(GutenbergBuildError::SourceBlobSizeMismatch {
            book_id: source.book_id,
            expected: expected_size,
            observed: blob.len() as u64,
        });
    }
    let observed = sha256(blob);
    if observed != expected_sha256 {
        return Err(GutenbergBuildError::SourceBlobShaMismatch {
            book_id: source.book_id,
            expected: expected_sha256,
            observed,
        });
    }
    tracing::info!(
        target: S4_BUILD_CORPUS_LOG_TARGET,
        event_name = "s4_blob_sha256_verified",
        book_id = source.book_id as u64,
        source_blob_sha256 = %observed,
        source_blob_size_bytes = blob.len() as u64,
        "S4 Gutenberg source blob hash verified"
    );
    Ok(())
}

fn verify_pre_strip_pin(
    source: &FixtureSource,
    decoded: &[u8],
    observed_sha256: Hash256,
    observed_byte_length: u64,
) -> Result<(), GutenbergBuildError> {
    if let Some(expected_size) = source.pre_strip_utf8_size_bytes
        && expected_size != observed_byte_length
    {
        return Err(GutenbergBuildError::PreStripSizeMismatch {
            book_id: source.book_id,
            expected: expected_size,
            observed: observed_byte_length,
        });
    }
    if let Some(expected_sha256) = source
        .pre_strip_utf8_sha256
        .as_deref()
        .map(parse_unprefixed_or_prefixed_hash)
        .transpose()?
        && expected_sha256 != observed_sha256
    {
        return Err(GutenbergBuildError::PreStripShaMismatch {
            book_id: source.book_id,
            expected: expected_sha256,
            observed: sha256(decoded),
        });
    }
    Ok(())
}

fn default_selected_format(source: &FixtureSource) -> String {
    let media_type = source.media_type.as_deref().unwrap_or("text/plain");
    let charset = source.charset.as_deref().unwrap_or("utf-8");
    let compression = source.compression_kind.as_deref().unwrap_or("none");
    format!(
        "{media_type}\n{charset}\n{compression}\n{}\n{}",
        source.archive_member_path.as_deref().unwrap_or(""),
        source
            .mirror_fetch_url
            .as_deref()
            .unwrap_or(&source.source_landing_url)
    )
}

fn decode_source_blob(source: &FixtureSource, blob: &[u8]) -> Result<Vec<u8>, DecodeDrop> {
    let compression = source
        .compression_kind
        .as_deref()
        .and_then(parse_compression_kind)
        .unwrap_or(GutenbergCompressionKind::None);
    let raw_plaintext = match compression {
        GutenbergCompressionKind::None => blob.to_vec(),
        GutenbergCompressionKind::Gzip => {
            let mut decoder = GzDecoder::new(Cursor::new(blob));
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|_| DecodeDrop::SourceDecodeFailed)?;
            decompressed
        }
        GutenbergCompressionKind::Zip => select_zip_plaintext(source, blob)?,
    };
    decode_text_charset(source.charset.as_deref(), &raw_plaintext)
}

fn select_zip_plaintext(source: &FixtureSource, blob: &[u8]) -> Result<Vec<u8>, DecodeDrop> {
    let reader = Cursor::new(blob);
    let mut archive = ZipArchive::new(reader).map_err(|_| DecodeDrop::SourceDecodeFailed)?;
    if let Some(member) = &source.archive_member_path {
        let mut file = archive
            .by_name(member)
            .map_err(|_| DecodeDrop::NoPlaintextArchiveMember)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| DecodeDrop::SourceDecodeFailed)?;
        return Ok(bytes);
    }

    let mut member_indices = Vec::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|_| DecodeDrop::SourceDecodeFailed)?;
        if file.is_file() && looks_like_plaintext_member(file.name()) {
            member_indices.push(index);
        }
    }
    match member_indices.as_slice() {
        [] => Err(DecodeDrop::NoPlaintextArchiveMember),
        [index] => {
            let mut file = archive
                .by_index(*index)
                .map_err(|_| DecodeDrop::SourceDecodeFailed)?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|_| DecodeDrop::SourceDecodeFailed)?;
            Ok(bytes)
        }
        _ => Err(DecodeDrop::AmbiguousPlaintextArchive),
    }
}

fn looks_like_plaintext_member(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".txt") || lower.ends_with(".utf8")
}

fn decode_text_charset(charset: Option<&str>, bytes: &[u8]) -> Result<Vec<u8>, DecodeDrop> {
    let canonical = charset.unwrap_or("utf-8").to_ascii_lowercase();
    match canonical.as_str() {
        "utf-8" | "utf8" => std::str::from_utf8(bytes)
            .map(|text| text.as_bytes().to_vec())
            .map_err(|_| DecodeDrop::InvalidUtf8),
        "us-ascii" | "ascii" => {
            if bytes.iter().any(|byte| *byte > 0x7f) {
                return Err(DecodeDrop::InvalidUtf8);
            }
            Ok(bytes.to_vec())
        }
        "iso-8859-1" | "latin-1" => Ok(bytes
            .iter()
            .map(|byte| char::from(*byte))
            .collect::<String>()
            .into_bytes()),
        "windows-1252" | "cp1252" => Ok(decode_windows_1252(bytes).into_bytes()),
        _ => Err(DecodeDrop::SourceDecodeFailed),
    }
}

fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| match *byte {
            0x80 => '\u{20ac}',
            0x82 => '\u{201a}',
            0x83 => '\u{0192}',
            0x84 => '\u{201e}',
            0x85 => '\u{2026}',
            0x86 => '\u{2020}',
            0x87 => '\u{2021}',
            0x88 => '\u{02c6}',
            0x89 => '\u{2030}',
            0x8a => '\u{0160}',
            0x8b => '\u{2039}',
            0x8c => '\u{0152}',
            0x8e => '\u{017d}',
            0x91 => '\u{2018}',
            0x92 => '\u{2019}',
            0x93 => '\u{201c}',
            0x94 => '\u{201d}',
            0x95 => '\u{2022}',
            0x96 => '\u{2013}',
            0x97 => '\u{2014}',
            0x98 => '\u{02dc}',
            0x99 => '\u{2122}',
            0x9a => '\u{0161}',
            0x9b => '\u{203a}',
            0x9c => '\u{0153}',
            0x9e => '\u{017e}',
            0x9f => '\u{0178}',
            other => char::from(other),
        })
        .collect()
}

fn apply_exact_body_dedup(processed: &mut [ProcessedSource]) {
    let mut retained_by_body = BTreeMap::<Hash256, u32>::new();
    for source in processed {
        if source.record.drop_reason.is_some() {
            continue;
        }
        let body_hash = source
            .record
            .post_charset_body_sha256
            .expect("retained source has post charset hash before dedup");
        if let Some(retained_book_id) = retained_by_body.get(&body_hash).copied() {
            source.record.drop_reason = Some(GutenbergDropReason::DedupCollision);
            source.record.duplicate_of_book_id = Some(retained_book_id);
            source.record.split = None;
            source.body_tokens.clear();
            tracing::info!(
                target: S4_BUILD_CORPUS_LOG_TARGET,
                event_name = "s4_dedup_drop",
                book_id = source.record.book_id as u64,
                duplicate_of_book_id = retained_book_id as u64,
                reason = "dedup_collision",
                "S4 Gutenberg deduplication dropped duplicate body"
            );
        } else {
            retained_by_body.insert(body_hash, source.record.book_id);
        }
    }
}

fn apply_splits(
    processed: &mut [ProcessedSource],
    fixture: &GutenbergFixture,
) -> Result<(), GutenbergBuildError> {
    let split = fixture.split.as_ref();
    let seed_hex = split
        .and_then(|split| split.split_seed_u128_hex.as_deref())
        .unwrap_or("f05b018451ce5602a13dc0dcd90c4696");
    let train_fraction = split.and_then(|split| split.train_fraction).unwrap_or(0.90);

    for source in processed {
        if source.record.drop_reason.is_none() {
            let split = split_for_book(source.record.book_id, seed_hex, train_fraction)?;
            source.record.split = Some(split);
            tracing::info!(
                target: S4_BUILD_CORPUS_LOG_TARGET,
                event_name = "s4_split_assigned",
                book_id = source.record.book_id as u64,
                split = ?split,
                "S4 Gutenberg split assigned"
            );
        }
    }
    Ok(())
}

fn split_for_book(
    book_id: u32,
    split_seed_hex: &str,
    train_fraction: f64,
) -> Result<GutenbergSplit, GutenbergBuildError> {
    let split_seed = parse_split_seed_bytes(split_seed_hex)?;
    let mut input = Vec::with_capacity(S4_SPLIT_HASH_PREFIX.len() + 16 + 4);
    input.extend_from_slice(S4_SPLIT_HASH_PREFIX);
    input.extend_from_slice(&split_seed);
    input.extend_from_slice(&book_id.to_le_bytes());
    let split_hash = sha256(&input);
    let high = u64::from_be_bytes(
        split_hash.as_bytes()[..8]
            .try_into()
            .expect("slice is exactly eight bytes"),
    ) >> 11;
    let u = high as f64 / ((1_u64 << 53) as f64);
    Ok(if u < train_fraction {
        GutenbergSplit::Train
    } else {
        GutenbergSplit::Val
    })
}

fn build_split_streams(processed: &[ProcessedSource]) -> (Vec<u8>, Vec<u8>) {
    let mut train = Vec::new();
    let mut val = Vec::new();
    for source in processed {
        let target = match source.record.split {
            Some(GutenbergSplit::Train) => &mut train,
            Some(GutenbergSplit::Val) => &mut val,
            None => continue,
        };
        target.push(BOS_ID);
        target.extend_from_slice(&source.body_tokens);
        target.push(EOS_ID);
    }
    (train, val)
}

fn build_manifest(
    fixture: &GutenbergFixture,
    book_ids: Vec<u32>,
    sources: Vec<GutenbergSourceRecord>,
    train_bytes: &[u8],
    val_bytes: &[u8],
    options: &GutenbergBuildOptions,
) -> Result<GutenbergManifest, GutenbergBuildError> {
    validate_build_guards(&book_ids, &sources)?;
    let train_book_count = count_split(&sources, GutenbergSplit::Train)?;
    let val_book_count = count_split(&sources, GutenbergSplit::Val)?;
    if train_book_count == 0 || val_book_count == 0 {
        return Err(GutenbergBuildError::EmptySplit {
            train_book_count,
            val_book_count,
        });
    }
    validate_split_byte_length("train", train_bytes.len())?;
    validate_split_byte_length("val", val_bytes.len())?;

    let catalog = fixture.catalog_snapshot.as_ref();
    let selection_filter = fixture.selection_filter.as_ref();
    let split = fixture.split.as_ref();
    let guards = fixture.guards.as_ref();
    let drop_count_total = count_drops(&sources)?;
    let drop_count_no_supported_plaintext_format =
        count_drop_reason(&sources, GutenbergDropReason::NoSupportedPlaintextFormat)?;
    let drop_count_no_plaintext_archive_member =
        count_drop_reason(&sources, GutenbergDropReason::NoPlaintextArchiveMember)?;
    let drop_count_source_decode_failed =
        count_drop_reason(&sources, GutenbergDropReason::SourceDecodeFailed)?;
    let drop_count_ambiguous_plaintext_archive =
        count_drop_reason(&sources, GutenbergDropReason::AmbiguousPlaintextArchive)?;
    let drop_count_invalid_utf8 = count_drop_reason(&sources, GutenbergDropReason::InvalidUtf8)?;
    let drop_count_empty_after_strip =
        count_drop_reason(&sources, GutenbergDropReason::EmptyAfterStrip)?;
    let drop_count_marker_missing =
        count_drop_reason(&sources, GutenbergDropReason::GutenbergMarkerMissing)?;
    let drop_count_unmappable_density =
        count_drop_reason(&sources, GutenbergDropReason::UnmappableDensityHigh)?;
    let drop_count_dedup_collision =
        count_drop_reason(&sources, GutenbergDropReason::DedupCollision)?;
    let unmappable_rate_corpus = corpus_unmappable_rate(&sources);

    let manifest = GutenbergManifest {
        schema: GutenbergManifest::schema_id(),
        source_name: GutenbergManifest::source_name_literal(),
        catalog_snapshot_url: catalog
            .map(|catalog| catalog.url.clone())
            .unwrap_or_else(|| "fixture:gutenberg_smoke".to_owned()),
        catalog_snapshot_sha256: catalog
            .map(|catalog| parse_unprefixed_or_prefixed_hash(&catalog.sha256))
            .transpose()?
            .unwrap_or_else(|| sha256("gutenberg_smoke_fixture.v1")),
        catalog_snapshot_observed_at_utc: catalog
            .map(|catalog| catalog.observed_at_utc.clone())
            .unwrap_or_else(|| "2026-05-19T00:00:00Z".to_owned()),
        catalog_snapshot_last_modified_utc: catalog
            .and_then(|catalog| catalog.last_modified_utc.clone()),
        selection_filter_canonical_json: selection_filter
            .map(|filter| filter.canonical_json.clone())
            .unwrap_or_else(|| "{}".to_owned()),
        selection_filter_sha256: selection_filter
            .map(|filter| parse_unprefixed_or_prefixed_hash(&filter.sha256))
            .transpose()?
            .unwrap_or_else(|| sha256("{}")),
        book_ids,
        sources,
        header_regex_pattern: fixture
            .markers
            .as_ref()
            .map(|markers| markers.header_regex_pattern.clone())
            .unwrap_or_else(|| GUTENBERG_D3_HEADER_REGEX_PATTERN.to_owned()),
        footer_regex_pattern: fixture
            .markers
            .as_ref()
            .map(|markers| markers.footer_regex_pattern.clone())
            .unwrap_or_else(|| GUTENBERG_D3_FOOTER_MARKER_REGEX_PATTERN.to_owned()),
        normalization_spec_self_hash: LexicalSpec_v1::pinned().lexical_self_hash,
        dedup_policy: GutenbergDedupPolicy::exact_post_strip_charset_body_sha(),
        split_seed_u128: split
            .and_then(|split| split.split_seed_u128_hex.clone())
            .unwrap_or_else(|| "f05b018451ce5602a13dc0dcd90c4696".to_owned()),
        split_train_fraction: split.and_then(|split| split.train_fraction).unwrap_or(0.90),
        split_val_fraction: split.and_then(|split| split.val_fraction).unwrap_or(0.10),
        train_path: options.train_path.display().to_string(),
        val_path: options.val_path.display().to_string(),
        train_sha256: sha256(train_bytes),
        val_sha256: sha256(val_bytes),
        train_byte_length: u64::try_from(train_bytes.len()).map_err(|_| {
            GutenbergBuildError::CountOverflow {
                field: "train_byte_length",
            }
        })?,
        val_byte_length: u64::try_from(val_bytes.len()).map_err(|_| {
            GutenbergBuildError::CountOverflow {
                field: "val_byte_length",
            }
        })?,
        train_book_count,
        val_book_count,
        drop_count_total,
        drop_count_no_supported_plaintext_format,
        drop_count_no_plaintext_archive_member,
        drop_count_source_decode_failed,
        drop_count_ambiguous_plaintext_archive,
        drop_count_invalid_utf8,
        drop_count_empty_after_strip,
        drop_count_marker_missing,
        drop_count_unmappable_density,
        drop_count_dedup_collision,
        unmappable_rate_corpus,
        raw_byte_policy: GutenbergManifest::raw_byte_policy_literal(),
        retained_book_count_min: retained_book_count_min(fixture),
        manifest_self_hash: Hash256::ZERO,
    };

    let retained_book_count = train_book_count.checked_add(val_book_count).ok_or(
        GutenbergBuildError::RetainedBookCountOverflow {
            train_book_count,
            val_book_count,
        },
    )?;
    if retained_book_count < manifest.retained_book_count_min {
        return Err(GutenbergBuildError::RetainedBookCountBelowFloor {
            retained_book_count,
            retained_book_count_min: manifest.retained_book_count_min,
        });
    }

    let max_unmappable_rate_corpus = guards
        .and_then(|guards| guards.unmappable_rate_corpus_max)
        .map(|fixture_cap| fixture_cap.min(GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX))
        .unwrap_or(GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX);
    verify_gutenberg_unmappable_gate_with_max(&manifest, max_unmappable_rate_corpus)?;

    let manifest = normalize_manifest_self_hash(manifest)?;
    tracing::info!(
        target: S4_BUILD_CORPUS_LOG_TARGET,
        event_name = "s4_manifest_finalized",
        manifest_self_hash = %manifest.manifest_self_hash,
        train_sha256 = %manifest.train_sha256,
        val_sha256 = %manifest.val_sha256,
        train_book_count = manifest.train_book_count as u64,
        val_book_count = manifest.val_book_count as u64,
        drop_count_total = manifest.drop_count_total as u64,
        unmappable_rate_corpus = manifest.unmappable_rate_corpus,
        "S4 Gutenberg manifest finalized"
    );
    Ok(manifest)
}

fn normalize_manifest_self_hash(
    manifest: GutenbergManifest,
) -> Result<GutenbergManifest, GutenbergBuildError> {
    let manifest = manifest.with_computed_self_hash()?;
    let canonical = manifest.canonical_bytes_unchecked()?;
    let mut normalized: GutenbergManifest =
        serde_json::from_slice(&canonical).map_err(GutenbergManifestError::Json)?;
    normalized.manifest_self_hash = Hash256::ZERO;
    Ok(normalized.with_computed_self_hash()?)
}

fn validate_build_guards(
    book_ids: &[u32],
    sources: &[GutenbergSourceRecord],
) -> Result<(), GutenbergBuildError> {
    let marker_missing = count_drop_reason(sources, GutenbergDropReason::GutenbergMarkerMissing)?;
    let book_count =
        u32::try_from(book_ids.len()).map_err(|_| GutenbergBuildError::CountOverflow {
            field: "book_count",
        })?;
    if marker_missing_drop_cap_breached(marker_missing, book_count) {
        return Err(GutenbergBuildError::MarkerMissingDropCapBreached {
            marker_missing,
            book_count,
        });
    }
    Ok(())
}

fn retained_book_count_min(fixture: &GutenbergFixture) -> u32 {
    fixture
        .guards
        .as_ref()
        .and_then(|guards| guards.retained_book_count_min)
        .unwrap_or_else(|| {
            if fixture.schema == "gutenberg_smoke_fixture.v1" {
                1
            } else {
                1350
            }
        })
}

fn validate_split_byte_length(
    split: &'static str,
    bytes: usize,
) -> Result<(), GutenbergBuildError> {
    let byte_length = u64::try_from(bytes).map_err(|_| GutenbergBuildError::CountOverflow {
        field: "split_byte_length",
    })?;
    let min_byte_length =
        u64::try_from(S4_SEQUENCE_LENGTH).map_err(|_| GutenbergBuildError::CountOverflow {
            field: "sequence_length",
        })?;
    if byte_length >= min_byte_length {
        Ok(())
    } else {
        Err(GutenbergBuildError::SplitByteLengthBelowMinimum {
            split,
            byte_length,
            min_byte_length,
        })
    }
}

fn tinystories_manifest_self_hash(path: &Path) -> Result<Hash256, GutenbergBuildError> {
    let manifest =
        read_tinystories_manifest(path).map_err(GutenbergBuildError::TinyStoriesManifest)?;
    let canonical = CanonicalJson::to_vec(&manifest).map_err(GutenbergBuildError::CanonicalJson)?;
    TINYSTORIES_MANIFEST_DOMAIN
        .hash_canonical_bytes(&canonical)
        .map_err(GutenbergBuildError::CanonicalJson)
}

fn build_corpus_quality(
    manifest: &GutenbergManifest,
    processed: &[ProcessedSource],
    options: &GutenbergBuildOptions,
) -> Result<Option<S4CorpusQuality>, GutenbergBuildError> {
    let Some(tinystories_manifest_path) = &options.tinystories_manifest_path else {
        return Ok(None);
    };
    let tinystories_manifest_self_hash = tinystories_manifest_self_hash(tinystories_manifest_path)?;

    let mut doc_lengths = Vec::new();
    let mut charset_coverage = BTreeSet::new();
    let mut longest_doc_id = None;
    let mut longest_doc_len = 0_u64;
    for source in processed {
        if source.record.drop_reason.is_some() {
            continue;
        }
        let len = u64::try_from(source.body_tokens.len()).map_err(|_| {
            GutenbergBuildError::CountOverflow {
                field: "tokens_per_doc",
            }
        })?;
        doc_lengths.push(len);
        if len > longest_doc_len {
            longest_doc_len = len;
            longest_doc_id = Some(source.record.book_id.to_string());
        }
        charset_coverage.extend(source.body_tokens.iter().copied());
    }
    let row = S4PerCorpusQuality::gutenberg_from_docs(
        &doc_lengths,
        longest_doc_id,
        u64::try_from(charset_coverage.len()).map_err(|_| GutenbergBuildError::CountOverflow {
            field: "charset_coverage_count",
        })?,
        manifest.unmappable_rate_corpus,
    )?;
    Ok(Some(S4CorpusQuality::new(
        manifest,
        tinystories_manifest_self_hash,
        vec![row],
    )?))
}

fn count_split(
    sources: &[GutenbergSourceRecord],
    split: GutenbergSplit,
) -> Result<u32, GutenbergBuildError> {
    count_usize(
        sources
            .iter()
            .filter(|source| source.split == Some(split))
            .count(),
        "split_count",
    )
}

fn count_drops(sources: &[GutenbergSourceRecord]) -> Result<u32, GutenbergBuildError> {
    count_usize(
        sources
            .iter()
            .filter(|source| source.drop_reason.is_some())
            .count(),
        "drop_count_total",
    )
}

fn count_drop_reason(
    sources: &[GutenbergSourceRecord],
    reason: GutenbergDropReason,
) -> Result<u32, GutenbergBuildError> {
    count_usize(
        sources
            .iter()
            .filter(|source| source.drop_reason == Some(reason))
            .count(),
        "drop_count",
    )
}

fn count_usize(actual: usize, field: &'static str) -> Result<u32, GutenbergBuildError> {
    u32::try_from(actual).map_err(|_| GutenbergBuildError::CountOverflow { field })
}

fn corpus_unmappable_rate(sources: &[GutenbergSourceRecord]) -> f64 {
    let (unmappable, total) = sources
        .iter()
        .filter(|source| source.drop_reason.is_none())
        .fold((0_u128, 0_u128), |(unmappable, total), source| {
            (
                unmappable + u128::from(source.unmappable_count.unwrap_or(0)),
                total + u128::from(source.post_charset_token_length.unwrap_or(0)),
            )
        });
    if total == 0 {
        0.0
    } else {
        unmappable as f64 / total as f64
    }
}

fn parse_drop_reason(value: &str) -> Result<GutenbergDropReason, GutenbergBuildError> {
    match value {
        "no_supported_plaintext_format" => Ok(GutenbergDropReason::NoSupportedPlaintextFormat),
        "no_plaintext_archive_member" => Ok(GutenbergDropReason::NoPlaintextArchiveMember),
        "gutenberg_marker_missing" => Ok(GutenbergDropReason::GutenbergMarkerMissing),
        "source_decode_failed" => Ok(GutenbergDropReason::SourceDecodeFailed),
        "invalid_utf8" => Ok(GutenbergDropReason::InvalidUtf8),
        "ambiguous_plaintext_archive" => Ok(GutenbergDropReason::AmbiguousPlaintextArchive),
        "empty_after_strip" => Ok(GutenbergDropReason::EmptyAfterStrip),
        "unmappable_density_high" => Ok(GutenbergDropReason::UnmappableDensityHigh),
        "dedup_collision" => Ok(GutenbergDropReason::DedupCollision),
        _ => Err(GutenbergBuildError::InvalidDropReason {
            value: value.to_owned(),
        }),
    }
}

fn parse_fetch_namespace_kind(value: &str) -> Option<GutenbergFetchNamespaceKind> {
    match value {
        "local_private_mirror" => Some(GutenbergFetchNamespaceKind::LocalPrivateMirror),
        "official_robot_harvest" => Some(GutenbergFetchNamespaceKind::OfficialRobotHarvest),
        "content_addressed_cache" => Some(GutenbergFetchNamespaceKind::ContentAddressedCache),
        _ => None,
    }
}

fn parse_compression_kind(value: &str) -> Option<GutenbergCompressionKind> {
    match value {
        "none" => Some(GutenbergCompressionKind::None),
        "gzip" => Some(GutenbergCompressionKind::Gzip),
        "zip" => Some(GutenbergCompressionKind::Zip),
        _ => None,
    }
}

fn parse_unprefixed_or_prefixed_hash(value: &str) -> Result<Hash256, GutenbergBuildError> {
    let prefixed;
    let value = if value.starts_with("sha256:") {
        value
    } else {
        prefixed = format!("sha256:{value}");
        &prefixed
    };
    Hash256::from_str(value).map_err(GutenbergBuildError::Hash)
}

fn parse_split_seed_bytes(value: &str) -> Result<[u8; 16], GutenbergBuildError> {
    if value.len() != 32 {
        return Err(GutenbergBuildError::InvalidSplitSeed {
            value: value.to_owned(),
        });
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_value(pair[0]).ok_or_else(|| GutenbergBuildError::InvalidSplitSeed {
            value: value.to_owned(),
        })?;
        let low = hex_value(pair[1]).ok_or_else(|| GutenbergBuildError::InvalidSplitSeed {
            value: value.to_owned(),
        })?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn fixture_root(fixture_path: &Path) -> Result<PathBuf, GutenbergBuildError> {
    let absolute = if fixture_path.is_absolute() {
        fixture_path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| GutenbergBuildError::Io {
                path: ".".to_owned(),
                source,
            })?
            .join(fixture_path)
    };
    let components = absolute
        .components()
        .map(|component| component.as_os_str().to_owned())
        .collect::<Vec<_>>();
    if let Some(index) = components
        .iter()
        .position(|component| component == "fixtures")
    {
        let mut root = PathBuf::new();
        for component in &components[..index] {
            root.push(component);
        }
        return Ok(root);
    }
    Ok(absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".")))
}

fn resolve_fixture_path(fixture_root: &Path, relative_or_absolute: &str) -> PathBuf {
    let path = Path::new(relative_or_absolute);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        fixture_root.join(path)
    }
}

fn write_bytes_if_changed(path: &Path, bytes: &[u8]) -> Result<(), GutenbergBuildError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| GutenbergBuildError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    std::fs::write(path, bytes).map_err(|source| GutenbergBuildError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeDrop {
    NoPlaintextArchiveMember,
    AmbiguousPlaintextArchive,
    SourceDecodeFailed,
    InvalidUtf8,
}

/// Errors from the network-disabled S4 Gutenberg corpus build.
#[derive(Debug)]
pub enum GutenbergBuildError {
    /// File IO failed.
    Io {
        /// Path being read or written.
        path: String,
        /// Source IO error.
        source: std::io::Error,
    },
    /// Fixture TOML parsing failed.
    Toml(toml::de::Error),
    /// A SHA-256 string failed to parse.
    Hash(Hash256ParseError),
    /// Canonical JSON serialization failed.
    CanonicalJson(gbf_foundation::CanonicalJsonError),
    /// TinyStories manifest parsing failed while building corpus-quality evidence.
    TinyStoriesManifest(gbf_data::CorpusManifestError),
    /// Charset-v1 normalization failed.
    Charset(gbf_data::CharsetError),
    /// `gutenberg_manifest.v1` validation or canonicalization failed.
    Manifest(GutenbergManifestError),
    /// `s4_corpus_quality.v1` validation or canonicalization failed.
    CorpusQuality(S4CorpusQualityError),
    /// Fixture schema was not one of the supported Gutenberg fixture schemas.
    InvalidFixtureSchema {
        /// Rejected schema id.
        observed: String,
    },
    /// Fixture source name was not Project Gutenberg.
    InvalidFixtureSourceName {
        /// Rejected source name.
        observed: String,
    },
    /// Duplicate book id appeared in fixture source records.
    DuplicateFixtureBookId {
        /// Duplicated book id.
        book_id: u32,
    },
    /// Fixture `book_ids.values` was not strictly ascending.
    BookIdsNotStrictlySorted,
    /// Fixture `book_ids.values` did not match the source-record order.
    FixtureBookIdsMismatch,
    /// A selected book id was missing its source record.
    FixtureSourceMissingBookId {
        /// Missing book id.
        book_id: u32,
    },
    /// A required source field was absent.
    MissingFixtureField {
        /// Affected book id.
        book_id: u32,
        /// Missing fixture field.
        field: &'static str,
    },
    /// A fixture drop reason was not part of `gutenberg_manifest.v1`.
    InvalidDropReason {
        /// Rejected drop-reason literal.
        value: String,
    },
    /// The D2 split seed was not 16 bytes of lowercase hex.
    InvalidSplitSeed {
        /// Rejected split seed.
        value: String,
    },
    /// Source blob length did not match the fixture pin.
    SourceBlobSizeMismatch {
        /// Affected book id.
        book_id: u32,
        /// Fixture-pinned byte length.
        expected: u64,
        /// Observed byte length.
        observed: u64,
    },
    /// Source blob SHA-256 did not match the fixture pin.
    SourceBlobShaMismatch {
        /// Affected book id.
        book_id: u32,
        /// Fixture-pinned SHA-256.
        expected: Hash256,
        /// Observed SHA-256.
        observed: Hash256,
    },
    /// Decoded pre-strip byte length did not match an optional fixture pin.
    PreStripSizeMismatch {
        /// Affected book id.
        book_id: u32,
        /// Fixture-pinned pre-strip byte length.
        expected: u64,
        /// Observed pre-strip byte length.
        observed: u64,
    },
    /// Decoded pre-strip SHA-256 did not match an optional fixture pin.
    PreStripShaMismatch {
        /// Affected book id.
        book_id: u32,
        /// Fixture-pinned pre-strip SHA-256.
        expected: Hash256,
        /// Observed pre-strip SHA-256.
        observed: Hash256,
    },
    /// D3 marker-missing hard cap was breached.
    MarkerMissingDropCapBreached {
        /// Marker-missing drops.
        marker_missing: u32,
        /// Total selected book ids.
        book_count: u32,
    },
    /// S4 D5 unmappable corpus gate failed.
    CorpusGate(S4UnmappableGateError),
    /// A retained split stream was shorter than the S4 sequence length.
    SplitByteLengthBelowMinimum {
        /// Split name.
        split: &'static str,
        /// Observed byte length.
        byte_length: u64,
        /// S4 sequence-length floor.
        min_byte_length: u64,
    },
    /// The D2 split produced an empty retained train or validation split.
    EmptySplit {
        /// Retained train book count.
        train_book_count: u32,
        /// Retained validation book count.
        val_book_count: u32,
    },
    /// Retained train + validation count overflowed u32.
    RetainedBookCountOverflow {
        /// Retained train book count.
        train_book_count: u32,
        /// Retained validation book count.
        val_book_count: u32,
    },
    /// Retained train + validation count did not meet the manifest floor.
    RetainedBookCountBelowFloor {
        /// Retained train + validation book count.
        retained_book_count: u32,
        /// Required retained floor.
        retained_book_count_min: u32,
    },
    /// A usize count did not fit the public schema field.
    CountOverflow {
        /// Field whose count overflowed.
        field: &'static str,
    },
}

impl fmt::Display for GutenbergBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Toml(error) => write!(f, "{error}"),
            Self::Hash(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::TinyStoriesManifest(error) => write!(f, "{error}"),
            Self::Charset(error) => write!(f, "{error}"),
            Self::Manifest(error) => write!(f, "{error}"),
            Self::CorpusQuality(error) => write!(f, "{error}"),
            Self::InvalidFixtureSchema { observed } => {
                write!(f, "invalid Gutenberg fixture schema {observed:?}")
            }
            Self::InvalidFixtureSourceName { observed } => {
                write!(f, "invalid Gutenberg fixture source_name {observed:?}")
            }
            Self::DuplicateFixtureBookId { book_id } => {
                write!(f, "duplicate Gutenberg fixture book id {book_id}")
            }
            Self::BookIdsNotStrictlySorted => {
                f.write_str("Gutenberg fixture book_ids must be strictly sorted")
            }
            Self::FixtureBookIdsMismatch => {
                f.write_str("Gutenberg fixture book_ids do not match [[sources]] book ids")
            }
            Self::FixtureSourceMissingBookId { book_id } => {
                write!(
                    f,
                    "Gutenberg fixture missing source record for book {book_id}"
                )
            }
            Self::MissingFixtureField { book_id, field } => {
                write!(f, "Gutenberg fixture book {book_id} missing {field}")
            }
            Self::InvalidDropReason { value } => {
                write!(f, "invalid Gutenberg fixture drop_reason {value:?}")
            }
            Self::InvalidSplitSeed { value } => {
                write!(
                    f,
                    "split seed must be 32 lowercase hex characters, got {value:?}"
                )
            }
            Self::SourceBlobSizeMismatch {
                book_id,
                expected,
                observed,
            } => write!(
                f,
                "source blob size mismatch for book {book_id}: expected {expected}, observed {observed}"
            ),
            Self::SourceBlobShaMismatch {
                book_id,
                expected,
                observed,
            } => write!(
                f,
                "source blob sha256 mismatch for book {book_id}: expected {expected}, observed {observed}"
            ),
            Self::PreStripSizeMismatch {
                book_id,
                expected,
                observed,
            } => write!(
                f,
                "pre-strip byte length mismatch for book {book_id}: expected {expected}, observed {observed}"
            ),
            Self::PreStripShaMismatch {
                book_id,
                expected,
                observed,
            } => write!(
                f,
                "pre-strip sha256 mismatch for book {book_id}: expected {expected}, observed {observed}"
            ),
            Self::MarkerMissingDropCapBreached {
                marker_missing,
                book_count,
            } => write!(
                f,
                "marker-missing drops breached D3 cap: {marker_missing} of {book_count}"
            ),
            Self::CorpusGate(error) => write!(f, "{error}"),
            Self::SplitByteLengthBelowMinimum {
                split,
                byte_length,
                min_byte_length,
            } => write!(
                f,
                "{split} byte length {byte_length} is below S4 sequence length {min_byte_length}"
            ),
            Self::EmptySplit {
                train_book_count,
                val_book_count,
            } => write!(
                f,
                "Gutenberg build requires non-empty train and val splits, got train={train_book_count} val={val_book_count}"
            ),
            Self::RetainedBookCountOverflow {
                train_book_count,
                val_book_count,
            } => write!(
                f,
                "retained book count overflow: train={train_book_count} val={val_book_count}"
            ),
            Self::RetainedBookCountBelowFloor {
                retained_book_count,
                retained_book_count_min,
            } => write!(
                f,
                "retained Gutenberg book count {retained_book_count} is below retained_book_count_min {retained_book_count_min}"
            ),
            Self::CountOverflow { field } => write!(f, "{field} does not fit in the schema"),
        }
    }
}

impl Error for GutenbergBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Toml(error) => Some(error),
            Self::Hash(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::TinyStoriesManifest(error) => Some(error),
            Self::Charset(error) => Some(error),
            Self::Manifest(error) => Some(error),
            Self::CorpusQuality(error) => Some(error),
            Self::CorpusGate(error) => Some(error),
            _ => None,
        }
    }
}

impl From<GutenbergManifestError> for GutenbergBuildError {
    fn from(error: GutenbergManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<S4CorpusQualityError> for GutenbergBuildError {
    fn from(error: S4CorpusQualityError) -> Self {
        Self::CorpusQuality(error)
    }
}

impl From<S4UnmappableGateError> for GutenbergBuildError {
    fn from(error: S4UnmappableGateError) -> Self {
        Self::CorpusGate(error)
    }
}

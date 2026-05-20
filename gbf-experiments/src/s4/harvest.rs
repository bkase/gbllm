//! Network-permitted Project Gutenberg fixture harvest for F-S4.04.
#![allow(missing_docs)]
//!
//! This module owns only the input-side harvest pin set: catalog filtering,
//! deterministic ID selection, source-resource fetch, compression/member
//! inspection, charset re-encoding to UTF-8, and fixture TOML emission.
//! Corpus assembly, D3 stripping, charset-v1 tokenization, deduplication,
//! contamination checks, and baseline fitting are intentionally left to their
//! owning S4 beads.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use gbf_data::unicode_nfc;
use gbf_foundation::{Hash256, Hash256ParseError, sha256};
use serde::{Deserialize, Serialize};
use tar::Archive;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use url::Url;
use xml::attribute::OwnedAttribute;
use xml::reader::{EventReader, XmlEvent};
use zip::ZipArchive;

/// Official Project Gutenberg RDF catalog snapshot URL from F-S4 §D1.
pub const DEFAULT_CATALOG_SNAPSHOT_URL: &str =
    "https://www.gutenberg.org/cache/epub/feeds/rdf-files.tar.bz2";

/// Default harvest cache root used by the Python prototype.
pub const DEFAULT_CACHE_DIR: &str = "corpus/gutenberg";

/// Default fixture TOML path consumed by later S4 replay/build operations.
pub const DEFAULT_FIXTURE_OUTPUT: &str = "fixtures/corpora/gutenberg.toml";

/// S4 target slice from §D1.
pub const DEFAULT_TARGET_SLICE: usize = 1500;

/// S4 selection rank prefix from §D1.
pub const RANK_PREFIX_ASCII: &str = "gbf:s4:gutenberg-select:v1";

/// Canonical §D1 filter JSON, sorted and compact.
pub const SELECTION_FILTER_CANONICAL_JSON: &str = "{\"has_plain_text\":true,\"languages_canonical\":[\"en\"],\"pg_rights\":\"Public domain in the USA.\"}";

/// S4 public-domain catalog-side rights literal.
pub const PUBLIC_DOMAIN_RIGHTS: &str = "Public domain in the USA.";

/// Canonical harvest start event name captured through the S4 CLI subscriber.
pub const S4_HARVEST_STARTED_EVENT: &str = "s4_harvest_started";

/// Canonical catalog snapshot verification event name.
pub const S4_HARVEST_CATALOG_SNAPSHOT_VERIFIED_EVENT: &str = "s4_harvest_catalog_snapshot_verified";

/// Canonical deterministic book-selection event name.
pub const S4_HARVEST_BOOK_SELECTED_EVENT: &str = "s4_harvest_book_selected";

/// Canonical source blob fetch/cache event name.
pub const S4_HARVEST_SOURCE_BLOB_FETCHED_EVENT: &str = "s4_harvest_source_blob_fetched";

/// Canonical source drop event name.
pub const S4_HARVEST_SOURCE_DROPPED_EVENT: &str = "s4_harvest_source_dropped";

/// Canonical harvest finalization event name.
pub const S4_HARVEST_FINALIZED_EVENT: &str = "s4_harvest_finalized";

/// Polite harvest User-Agent used for official robot harvests.
pub const DEFAULT_USER_AGENT: &str =
    "gbllm-fixture-builder/0.1 (research; https://github.com/bkase/gbllm)";

const HARVEST_LOG_TARGET: &str = "gbf_experiments::s4::harvest";

/// Input options for `gbf s4 harvest-gutenberg-fixture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutenbergHarvestOptions {
    /// Explicit network permission acknowledgement required by the CLI.
    pub network_permitted: bool,
    /// RDF catalog snapshot URL recorded into fixture provenance.
    pub catalog_url: String,
    /// Optional local catalog snapshot path used instead of fetching the URL.
    pub catalog_path: Option<PathBuf>,
    /// Cache root for the catalog snapshot and fetched source blobs.
    pub cache_dir: PathBuf,
    /// Fixture TOML path to write.
    pub fixture_output: PathBuf,
    /// Number of deterministically-ranked IDs to select.
    pub target_slice: usize,
    /// RFC3339 UTC observation timestamp for the catalog snapshot.
    pub catalog_observed_at_utc: String,
    /// Optional RFC3339 UTC Last-Modified timestamp for the catalog snapshot.
    pub catalog_last_modified_utc: Option<String>,
    /// Source namespace kind recorded into each fixture source row.
    pub fetch_namespace_kind: String,
    /// Source namespace identifier recorded into each fixture source row.
    pub fetch_namespace_id: String,
    /// User-Agent used for HTTP(S) harvest requests.
    pub user_agent: String,
    /// Per-request timeout for catalog and source fetches.
    pub fetch_timeout_seconds: u64,
}

/// Deterministic summary returned by the harvest operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GutenbergHarvestSummary {
    /// Summary schema id.
    pub schema: &'static str,
    /// Path of the written fixture TOML.
    pub fixture_path: String,
    /// SHA-256 of the written fixture TOML bytes.
    pub fixture_sha256: Hash256,
    /// Catalog snapshot URL recorded in fixture provenance.
    pub catalog_snapshot_url: String,
    /// SHA-256 of the catalog snapshot bytes.
    pub catalog_snapshot_sha256: Hash256,
    /// Catalog snapshot byte length.
    pub catalog_snapshot_size_bytes: u64,
    /// Local cache path where the catalog snapshot was written.
    pub catalog_snapshot_local_path: String,
    /// Count of catalog candidates matching the §D1 filter.
    pub candidates_total: usize,
    /// Requested deterministic target slice size.
    pub target_slice: usize,
    /// Count of selected book IDs.
    pub book_count: usize,
    /// SHA-256 pin over the comma-joined selected book IDs.
    pub book_ids_self_hash_sha256: Hash256,
    /// Count of source blobs written.
    pub source_count: usize,
    /// Total byte length of source blobs written.
    pub source_blob_total_bytes: u64,
    /// Network policy for this operation.
    pub network_policy: &'static str,
    /// Source namespace kind recorded into source rows.
    pub fetch_namespace_kind: String,
    /// Source namespace identifier recorded into source rows.
    pub fetch_namespace_id: String,
}

#[derive(Debug, Clone)]
struct CatalogCandidate {
    id: u32,
    title: String,
    author: String,
    languages_canonical: Vec<String>,
    rights: Option<String>,
    resources: Vec<PlaintextResource>,
}

#[derive(Debug, Clone)]
struct PlaintextResource {
    url: String,
    media_type: String,
    charset: Option<String>,
    extent: Option<u64>,
}

#[derive(Debug, Clone)]
struct SourceCandidate {
    resource: PlaintextResource,
    canonical_url: String,
    mime_type: String,
    charset_or_empty: String,
    effective_charset: CharsetKind,
    compression_kind: CompressionKind,
    preference_class: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompressionKind {
    None,
    Gzip,
    Zip,
}

impl CompressionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Zip => "zip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharsetKind {
    Utf8,
    UsAscii,
    Iso8859_1,
    Windows1252,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HarvestDropReason {
    NoSupportedPlaintextFormat,
    NoPlaintextArchiveMember,
    SourceDecodeFailed,
    InvalidUtf8,
    AmbiguousPlaintextArchive,
}

impl HarvestDropReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::NoSupportedPlaintextFormat => "no_supported_plaintext_format",
            Self::NoPlaintextArchiveMember => "no_plaintext_archive_member",
            Self::SourceDecodeFailed => "source_decode_failed",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::AmbiguousPlaintextArchive => "ambiguous_plaintext_archive",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedSourceRecord {
    book_id: u32,
    rdf_resource_url: String,
    selected_format: String,
    source_blob_sha256: Hash256,
    blob_filename: String,
}

#[derive(Debug, Clone)]
struct HarvestedSource {
    book_id: u32,
    title: String,
    author: String,
    source_landing_url: String,
    rdf_resource_url: Option<String>,
    mirror_fetch_url: Option<String>,
    source_blob_sha256: Option<Hash256>,
    source_blob_size_bytes: Option<u64>,
    pre_strip_utf8_sha256: Option<Hash256>,
    pre_strip_utf8_size_bytes: Option<u64>,
    media_type: Option<String>,
    charset: Option<String>,
    extent_declared: Option<u64>,
    preference_class: Option<u8>,
    selected_format: Option<String>,
    compression_kind: Option<CompressionKind>,
    archive_member_path: Option<String>,
    fetch_namespace_kind: Option<String>,
    fetch_namespace_id: Option<String>,
    local_blob_path: Option<String>,
    drop_reason: Option<HarvestDropReason>,
}

#[derive(Debug, Clone)]
struct DecodedPlaintext {
    utf8_bytes: Vec<u8>,
    archive_member_path: Option<String>,
}

#[derive(Debug, Default)]
struct RdfParseState {
    in_ebook: bool,
    stack: Vec<String>,
    candidate: Option<CatalogCandidate>,
    current_file: Option<PlaintextResource>,
    text: String,
}

/// Return the current UTC timestamp in RFC3339 form for real harvests.
pub fn current_rfc3339_utc() -> Result<String, GutenbergHarvestError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(GutenbergHarvestError::TimeFormat)
}

fn validate_catalog_timestamps(
    observed_at_utc: &str,
    last_modified_utc: Option<&str>,
) -> Result<(), GutenbergHarvestError> {
    let observed_at = OffsetDateTime::parse(observed_at_utc, &Rfc3339).map_err(|source| {
        GutenbergHarvestError::InvalidCatalogTimestamp {
            field: "catalog_observed_at_utc",
            value: observed_at_utc.to_owned(),
            source,
        }
    })?;
    let Some(last_modified_utc) = last_modified_utc else {
        return Ok(());
    };
    let last_modified = OffsetDateTime::parse(last_modified_utc, &Rfc3339).map_err(|source| {
        GutenbergHarvestError::InvalidCatalogTimestamp {
            field: "catalog_last_modified_utc",
            value: last_modified_utc.to_owned(),
            source,
        }
    })?;
    if observed_at < last_modified {
        return Err(GutenbergHarvestError::CatalogTimestampOrder {
            observed_at_utc: observed_at_utc.to_owned(),
            last_modified_utc: last_modified_utc.to_owned(),
        });
    }
    Ok(())
}

/// Run the F-S4.04 Gutenberg harvest operation and write the fixture TOML.
pub fn harvest_gutenberg_fixture(
    options: &GutenbergHarvestOptions,
) -> Result<GutenbergHarvestSummary, GutenbergHarvestError> {
    if !options.network_permitted {
        return Err(GutenbergHarvestError::NetworkPermissionRequired);
    }
    if options.target_slice == 0 {
        return Err(GutenbergHarvestError::InvalidTargetSlice);
    }
    validate_catalog_timestamps(
        &options.catalog_observed_at_utc,
        options.catalog_last_modified_utc.as_deref(),
    )?;
    emit_harvest_started(options);

    let catalog = read_catalog_snapshot(options)?;
    let catalog_sha256 = sha256(&catalog.bytes);
    let catalog_pin_verification =
        verify_existing_fixture_catalog_pin(&options.fixture_output, catalog_sha256)?;
    emit_catalog_snapshot_verified(options, catalog_sha256, catalog_pin_verification);
    let catalog_local_path = options.cache_dir.join("rdf-files.tar.bz2");
    write_bytes_if_changed(&catalog_local_path, &catalog.bytes)?;

    let catalog_selection = parse_catalog_snapshot(&catalog.bytes, options.target_slice)?;
    let mut harvested_sources = Vec::with_capacity(catalog_selection.book_ids.len());
    for (selection_index, book_id) in catalog_selection.book_ids.iter().enumerate() {
        emit_book_selected(
            selection_index,
            *book_id,
            catalog_selection.book_ids.len(),
            catalog_selection.book_ids_self_hash_sha256,
        );
        let candidate = catalog_selection
            .candidates_by_id
            .get(book_id)
            .ok_or(GutenbergHarvestError::SelectedBookMissingCandidate { book_id: *book_id })?;
        let harvested_source = harvest_source(candidate, options)?;
        emit_source_drop(&harvested_source);
        harvested_sources.push(harvested_source);
    }

    let fixture = render_fixture_toml(
        options,
        &catalog,
        catalog_sha256,
        &catalog_local_path,
        &catalog_selection,
        &harvested_sources,
    )?;
    write_string_if_changed(&options.fixture_output, &fixture)?;
    let fixture_bytes = fixture.into_bytes();
    let fixture_sha256 = sha256(&fixture_bytes);
    let source_count = harvested_sources
        .iter()
        .filter(|source| source.source_blob_sha256.is_some())
        .count();
    let drop_count = harvested_sources
        .iter()
        .filter(|source| source.drop_reason.is_some())
        .count();
    let source_blob_total_bytes = harvested_sources
        .iter()
        .filter_map(|source| source.source_blob_size_bytes)
        .try_fold(0_u64, |acc, bytes| {
            acc.checked_add(bytes)
                .ok_or(GutenbergHarvestError::CountOverflow(
                    "source_blob_total_bytes",
                ))
        })?;

    let summary = GutenbergHarvestSummary {
        schema: "s4_gutenberg_harvest.v1",
        fixture_path: options.fixture_output.display().to_string(),
        fixture_sha256,
        catalog_snapshot_url: options.catalog_url.clone(),
        catalog_snapshot_sha256: catalog_sha256,
        catalog_snapshot_size_bytes: catalog
            .bytes
            .len()
            .try_into()
            .map_err(|_| GutenbergHarvestError::CountOverflow("catalog_snapshot_size_bytes"))?,
        catalog_snapshot_local_path: catalog_local_path.display().to_string(),
        candidates_total: catalog_selection.candidates_total,
        target_slice: options.target_slice,
        book_count: catalog_selection.book_ids.len(),
        book_ids_self_hash_sha256: catalog_selection.book_ids_self_hash_sha256,
        source_count,
        source_blob_total_bytes,
        network_policy: "permitted",
        fetch_namespace_kind: options.fetch_namespace_kind.clone(),
        fetch_namespace_id: options.fetch_namespace_id.clone(),
    };
    emit_harvest_finalized(&summary, drop_count);
    Ok(summary)
}

struct CatalogSnapshot {
    bytes: Vec<u8>,
    last_modified_utc: Option<String>,
}

struct CatalogSelection {
    candidates_total: usize,
    book_ids: Vec<u32>,
    book_ids_self_hash_sha256: Hash256,
    candidates_by_id: BTreeMap<u32, CatalogCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CatalogPinVerification {
    fixture_pin_present: bool,
}

fn verify_existing_fixture_catalog_pin(
    fixture_path: &Path,
    observed_catalog_sha256: Hash256,
) -> Result<CatalogPinVerification, GutenbergHarvestError> {
    let fixture_text =
        match std::fs::read_to_string(fixture_path).map_err(|source| GutenbergHarvestError::Io {
            path: fixture_path.display().to_string(),
            source,
        }) {
            Ok(fixture_text) => fixture_text,
            Err(GutenbergHarvestError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(CatalogPinVerification {
                    fixture_pin_present: false,
                });
            }
            Err(error) => return Err(error),
        };

    let fixture: toml::Value = toml::from_str(&fixture_text).map_err(|source| {
        GutenbergHarvestError::ExistingFixtureToml {
            path: fixture_path.display().to_string(),
            source,
        }
    })?;
    let Some(catalog_sha256_value) = fixture
        .get("catalog_snapshot")
        .and_then(|catalog| catalog.get("sha256"))
        .and_then(toml::Value::as_str)
    else {
        return Ok(CatalogPinVerification {
            fixture_pin_present: false,
        });
    };

    let pinned_catalog_sha256 = parse_fixture_hash(catalog_sha256_value).map_err(|source| {
        GutenbergHarvestError::InvalidExistingFixtureCatalogSha {
            fixture_path: fixture_path.display().to_string(),
            value: catalog_sha256_value.to_owned(),
            source,
        }
    })?;
    if pinned_catalog_sha256 != observed_catalog_sha256 {
        return Err(GutenbergHarvestError::CatalogSnapshotShaMismatch {
            fixture_path: fixture_path.display().to_string(),
            expected: pinned_catalog_sha256,
            observed: observed_catalog_sha256,
        });
    }

    Ok(CatalogPinVerification {
        fixture_pin_present: true,
    })
}

fn parse_fixture_hash(value: &str) -> Result<Hash256, Hash256ParseError> {
    if value.starts_with("sha256:") {
        Hash256::from_str(value)
    } else {
        Hash256::from_str(&format!("sha256:{value}"))
    }
}

fn emit_harvest_started(options: &GutenbergHarvestOptions) {
    tracing::info!(
        target: HARVEST_LOG_TARGET,
        event_name = S4_HARVEST_STARTED_EVENT,
        catalog_url = options.catalog_url.as_str(),
        fixture_output = %options.fixture_output.display(),
        target_slice = options.target_slice as u64,
        network_permitted = options.network_permitted,
        "s4 harvest started"
    );
}

fn emit_catalog_snapshot_verified(
    options: &GutenbergHarvestOptions,
    catalog_sha256: Hash256,
    pin_verification: CatalogPinVerification,
) {
    tracing::info!(
        target: HARVEST_LOG_TARGET,
        event_name = S4_HARVEST_CATALOG_SNAPSHOT_VERIFIED_EVENT,
        catalog_url = options.catalog_url.as_str(),
        fixture_output = %options.fixture_output.display(),
        catalog_snapshot_sha256 = %catalog_sha256,
        fixture_pin_present = pin_verification.fixture_pin_present,
        "s4 harvest catalog snapshot verified"
    );
}

fn emit_book_selected(
    selection_index: usize,
    book_id: u32,
    book_count: usize,
    book_ids_self_hash_sha256: Hash256,
) {
    tracing::info!(
        target: HARVEST_LOG_TARGET,
        event_name = S4_HARVEST_BOOK_SELECTED_EVENT,
        selection_index = selection_index as u64,
        book_id = book_id as u64,
        book_count = book_count as u64,
        book_ids_self_hash_sha256 = %book_ids_self_hash_sha256,
        "s4 harvest selected book"
    );
}

fn emit_source_blob_fetched(
    book_id: u32,
    source_blob_sha256: Hash256,
    source_blob_size_bytes: u64,
    compression_kind: CompressionKind,
    cache_hit: bool,
) {
    tracing::info!(
        target: HARVEST_LOG_TARGET,
        event_name = S4_HARVEST_SOURCE_BLOB_FETCHED_EVENT,
        book_id = book_id as u64,
        source_blob_sha256 = %source_blob_sha256,
        source_blob_size_bytes,
        compression_kind = compression_kind.as_str(),
        cache_hit,
        "s4 harvest source blob fetched"
    );
}

fn emit_source_drop(source: &HarvestedSource) {
    let Some(drop_reason) = source.drop_reason else {
        return;
    };
    let source_blob_sha256 = source
        .source_blob_sha256
        .map(|hash| hash.to_string())
        .unwrap_or_default();
    let compression_kind = source
        .compression_kind
        .map(CompressionKind::as_str)
        .unwrap_or("");
    tracing::info!(
        target: HARVEST_LOG_TARGET,
        event_name = S4_HARVEST_SOURCE_DROPPED_EVENT,
        book_id = source.book_id as u64,
        drop_reason = drop_reason.as_str(),
        source_blob_sha256 = source_blob_sha256.as_str(),
        source_blob_size_bytes = source.source_blob_size_bytes.unwrap_or(0),
        compression_kind,
        "s4 harvest source dropped"
    );
}

fn emit_harvest_finalized(summary: &GutenbergHarvestSummary, drop_count: usize) {
    tracing::info!(
        target: HARVEST_LOG_TARGET,
        event_name = S4_HARVEST_FINALIZED_EVENT,
        fixture_path = summary.fixture_path.as_str(),
        fixture_sha256 = %summary.fixture_sha256,
        catalog_snapshot_sha256 = %summary.catalog_snapshot_sha256,
        book_count = summary.book_count as u64,
        source_count = summary.source_count as u64,
        drop_count = drop_count as u64,
        source_blob_total_bytes = summary.source_blob_total_bytes,
        "s4 harvest finalized"
    );
}

fn read_catalog_snapshot(
    options: &GutenbergHarvestOptions,
) -> Result<CatalogSnapshot, GutenbergHarvestError> {
    if let Some(path) = &options.catalog_path {
        let bytes = std::fs::read(path).map_err(|source| GutenbergHarvestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        return Ok(CatalogSnapshot {
            bytes,
            last_modified_utc: options.catalog_last_modified_utc.clone(),
        });
    }

    let fetched = fetch_url_bytes(
        &options.catalog_url,
        &options.user_agent,
        options.fetch_timeout_seconds,
    )?;
    Ok(CatalogSnapshot {
        bytes: fetched.bytes,
        last_modified_utc: options.catalog_last_modified_utc.clone(),
    })
}

fn parse_catalog_snapshot(
    catalog_bytes: &[u8],
    target_slice: usize,
) -> Result<CatalogSelection, GutenbergHarvestError> {
    let decoder = BzDecoder::new(Cursor::new(catalog_bytes));
    let mut archive = Archive::new(decoder);
    let mut candidates = BTreeMap::new();

    for entry in archive
        .entries()
        .map_err(|source| GutenbergHarvestError::Io {
            path: "catalog tar.bz2".to_owned(),
            source,
        })?
    {
        let mut entry = entry.map_err(|source| GutenbergHarvestError::Io {
            path: "catalog tar entry".to_owned(),
            source,
        })?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|source| GutenbergHarvestError::Io {
                path: "catalog tar entry path".to_owned(),
                source,
            })?
            .to_string_lossy()
            .into_owned();
        if !path.ends_with(".rdf") {
            continue;
        }

        let mut blob = Vec::new();
        entry
            .read_to_end(&mut blob)
            .map_err(|source| GutenbergHarvestError::Io {
                path: path.clone(),
                source,
            })?;
        if let Some(mut candidate) = parse_rdf_blob(&blob, &path)? {
            candidate.resources.retain(is_catalog_plaintext_resource);
            if candidate.languages_canonical == ["en"]
                && candidate.rights.as_deref() == Some(PUBLIC_DOMAIN_RIGHTS)
                && !candidate.resources.is_empty()
            {
                candidates.insert(candidate.id, candidate);
            }
        }
    }

    if candidates.len() < target_slice {
        return Err(GutenbergHarvestError::TooFewCandidates {
            observed: candidates.len(),
            required: target_slice,
        });
    }

    let mut ranked = candidates
        .keys()
        .copied()
        .map(|book_id| (rank_key(book_id), book_id))
        .collect::<Vec<_>>();
    ranked.sort();
    let mut book_ids = ranked
        .into_iter()
        .take(target_slice)
        .map(|(_, book_id)| book_id)
        .collect::<Vec<_>>();
    book_ids.sort_unstable();
    let book_ids_self_hash_sha256 = book_ids_self_hash(&book_ids);
    Ok(CatalogSelection {
        candidates_total: candidates.len(),
        book_ids,
        book_ids_self_hash_sha256,
        candidates_by_id: candidates,
    })
}

fn parse_rdf_blob(
    blob: &[u8],
    path: &str,
) -> Result<Option<CatalogCandidate>, GutenbergHarvestError> {
    let parser = EventReader::new(Cursor::new(blob));
    let mut state = RdfParseState::default();

    for event in parser {
        match event.map_err(|source| GutenbergHarvestError::Xml {
            path: path.to_owned(),
            source,
        })? {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                let local = name.local_name;
                state.stack.push(local.clone());
                state.text.clear();

                if local == "ebook" {
                    if let Some(book_id) = ebook_id_from_attributes(&attributes) {
                        state.in_ebook = true;
                        state.candidate = Some(CatalogCandidate {
                            id: book_id,
                            title: String::new(),
                            author: String::new(),
                            languages_canonical: Vec::new(),
                            rights: None,
                            resources: Vec::new(),
                        });
                    }
                } else if state.in_ebook && local == "file" {
                    let url = rdf_about_from_attributes(&attributes).unwrap_or_default();
                    state.current_file = Some(PlaintextResource {
                        url,
                        media_type: String::new(),
                        charset: None,
                        extent: None,
                    });
                }
            }
            XmlEvent::Characters(text) | XmlEvent::CData(text) if state.in_ebook => {
                state.text.push_str(&text);
            }
            XmlEvent::EndElement { name } => {
                if state.in_ebook {
                    apply_rdf_text(&mut state, &name.local_name);
                }
                if name.local_name == "file"
                    && let (Some(candidate), Some(file)) =
                        (state.candidate.as_mut(), state.current_file.take())
                    && !file.url.is_empty()
                    && !file.media_type.is_empty()
                {
                    candidate.resources.push(file);
                }
                if name.local_name == "ebook" {
                    state.in_ebook = false;
                    if let Some(mut candidate) = state.candidate.take() {
                        candidate.languages_canonical.sort();
                        candidate.languages_canonical.dedup();
                        if candidate.title.is_empty() {
                            candidate.title = format!("Project Gutenberg Ebook {}", candidate.id);
                        }
                        if candidate.author.is_empty() {
                            candidate.author = "Unknown".to_owned();
                        }
                        return Ok(Some(candidate));
                    }
                }
                state.stack.pop();
                state.text.clear();
            }
            XmlEvent::EndDocument => break,
            _ => {}
        }
    }

    Ok(None)
}

fn apply_rdf_text(state: &mut RdfParseState, end_local: &str) {
    let text = state.text.trim();
    if text.is_empty() {
        return;
    }

    if state.current_file.is_some() {
        if end_local == "extent" {
            if let Ok(extent) = text.parse::<u64>()
                && let Some(file) = state.current_file.as_mut()
            {
                file.extent = Some(extent);
            }
        } else if end_local == "value"
            && stack_contains(&state.stack, "format")
            && let Some(file) = state.current_file.as_mut()
            && file.media_type.is_empty()
        {
            file.media_type = text.to_owned();
            file.charset = charset_from_media_type(text).map(str::to_owned);
        }
        return;
    }

    let Some(candidate) = state.candidate.as_mut() else {
        return;
    };
    if end_local == "title" && candidate.title.is_empty() {
        candidate.title = text.to_owned();
    } else if end_local == "rights" {
        candidate.rights = Some(text.to_owned());
    } else if end_local == "value" && stack_contains(&state.stack, "language") {
        candidate
            .languages_canonical
            .push(text.to_ascii_lowercase());
    } else if end_local == "name"
        && candidate.author.is_empty()
        && stack_contains(&state.stack, "creator")
    {
        candidate.author = text.to_owned();
    }
}

fn stack_contains(stack: &[String], local: &str) -> bool {
    stack.iter().any(|entry| entry == local)
}

fn ebook_id_from_attributes(attributes: &[OwnedAttribute]) -> Option<u32> {
    rdf_about_from_attributes(attributes).and_then(|about| {
        about
            .strip_prefix("ebooks/")
            .and_then(|suffix| suffix.parse::<u32>().ok())
    })
}

fn rdf_about_from_attributes(attributes: &[OwnedAttribute]) -> Option<String> {
    attributes
        .iter()
        .find(|attribute| attribute.name.local_name == "about")
        .map(|attribute| attribute.value.clone())
}

fn is_catalog_plaintext_resource(resource: &PlaintextResource) -> bool {
    let mime_type = media_type_base(&resource.media_type).unwrap_or("text/plain");
    if mime_type == "text/plain" || mime_type == "application/zip" || is_gzip_mime(mime_type) {
        return true;
    }
    let url_path = Url::parse(&resource.url)
        .ok()
        .map(|url| url.path().to_ascii_lowercase())
        .unwrap_or_else(|| resource.url.to_ascii_lowercase());
    url_path.ends_with(".txt")
        || url_path.ends_with(".utf8")
        || url_path.ends_with(".zip")
        || url_path.ends_with(".gz")
}

fn harvest_source(
    candidate: &CatalogCandidate,
    options: &GutenbergHarvestOptions,
) -> Result<HarvestedSource, GutenbergHarvestError> {
    let mut source_candidates = candidate
        .resources
        .iter()
        .filter_map(|resource| SourceCandidate::from_resource(resource).ok())
        .collect::<Vec<_>>();
    source_candidates.sort_by(|left, right| {
        (left.preference_class, left.canonical_format_id(""))
            .cmp(&(right.preference_class, right.canonical_format_id("")))
    });
    let Some(source_candidate) = source_candidates.into_iter().next() else {
        return Ok(dropped_source_without_blob(
            candidate,
            HarvestDropReason::NoSupportedPlaintextFormat,
        ));
    };

    let selected_format_without_member = source_candidate.canonical_format_id("");
    let book_dir = options
        .cache_dir
        .join("sources")
        .join(candidate.id.to_string());
    let cached = read_cached_source_record(&book_dir).and_then(|record| {
        if record.rdf_resource_url == source_candidate.resource.url
            && record.selected_format == selected_format_without_member
        {
            let blob_path = book_dir.join(&record.blob_filename);
            let blob = std::fs::read(&blob_path).ok()?;
            (sha256(&blob) == record.source_blob_sha256).then_some((record, blob))
        } else {
            None
        }
    });

    let (raw_blob, source_cache_hit) = if let Some((_, blob)) = cached {
        (blob, true)
    } else {
        (
            fetch_url_bytes(
                &source_candidate.resource.url,
                &options.user_agent,
                options.fetch_timeout_seconds,
            )?
            .bytes,
            false,
        )
    };
    let source_blob_sha256 = sha256(&raw_blob);
    let source_blob_size_bytes = raw_blob
        .len()
        .try_into()
        .map_err(|_| GutenbergHarvestError::CountOverflow("source_blob_size_bytes"))?;
    let blob_filename = format!("{}.bin", source_blob_sha256.to_hex());
    let blob_path = book_dir.join(&blob_filename);
    write_bytes_if_changed(&blob_path, &raw_blob)?;
    let cache_record = CachedSourceRecord {
        book_id: candidate.id,
        rdf_resource_url: source_candidate.resource.url.clone(),
        selected_format: selected_format_without_member.clone(),
        source_blob_sha256,
        blob_filename,
    };
    write_string_if_changed(
        &book_dir.join("source_record.json"),
        &serde_json::to_string_pretty(&cache_record).map_err(GutenbergHarvestError::Json)?,
    )?;
    emit_source_blob_fetched(
        candidate.id,
        source_blob_sha256,
        source_blob_size_bytes,
        source_candidate.compression_kind,
        source_cache_hit,
    );

    let decoded = match decode_source_blob(candidate.id, &source_candidate, &raw_blob) {
        Ok(decoded) => decoded,
        Err(error) => {
            let Some(drop_reason) = drop_reason_for_decode_error(&error) else {
                return Err(error);
            };
            return Ok(dropped_source_with_blob(
                candidate,
                options,
                &source_candidate,
                DroppedBlobEvidence {
                    selected_format: selected_format_without_member,
                    source_blob_sha256,
                    source_blob_size_bytes,
                    blob_path,
                },
                drop_reason,
            ));
        }
    };
    let archive_member_path = decoded.archive_member_path;
    let selected_format =
        source_candidate.canonical_format_id(archive_member_path.as_deref().unwrap_or(""));
    let pre_strip_utf8_size_bytes = decoded
        .utf8_bytes
        .len()
        .try_into()
        .map_err(|_| GutenbergHarvestError::CountOverflow("pre_strip_utf8_size_bytes"))?;
    let pre_strip_utf8_sha256 = sha256(&decoded.utf8_bytes);

    Ok(HarvestedSource {
        book_id: candidate.id,
        title: candidate.title.clone(),
        author: candidate.author.clone(),
        source_landing_url: format!("https://www.gutenberg.org/ebooks/{}", candidate.id),
        rdf_resource_url: Some(source_candidate.resource.url.clone()),
        mirror_fetch_url: Some(source_candidate.resource.url.clone()),
        source_blob_sha256: Some(source_blob_sha256),
        source_blob_size_bytes: Some(source_blob_size_bytes),
        pre_strip_utf8_sha256: Some(pre_strip_utf8_sha256),
        pre_strip_utf8_size_bytes: Some(pre_strip_utf8_size_bytes),
        media_type: Some(source_candidate.resource.media_type.clone()),
        charset: source_candidate.resource.charset.clone(),
        extent_declared: source_candidate.resource.extent,
        preference_class: Some(source_candidate.preference_class),
        selected_format: Some(selected_format),
        compression_kind: Some(source_candidate.compression_kind),
        archive_member_path,
        fetch_namespace_kind: Some(options.fetch_namespace_kind.clone()),
        fetch_namespace_id: Some(options.fetch_namespace_id.clone()),
        local_blob_path: Some(blob_path.display().to_string()),
        drop_reason: None,
    })
}

fn dropped_source_without_blob(
    candidate: &CatalogCandidate,
    drop_reason: HarvestDropReason,
) -> HarvestedSource {
    HarvestedSource {
        book_id: candidate.id,
        title: candidate.title.clone(),
        author: candidate.author.clone(),
        source_landing_url: format!("https://www.gutenberg.org/ebooks/{}", candidate.id),
        rdf_resource_url: None,
        mirror_fetch_url: None,
        source_blob_sha256: None,
        source_blob_size_bytes: None,
        pre_strip_utf8_sha256: None,
        pre_strip_utf8_size_bytes: None,
        media_type: None,
        charset: None,
        extent_declared: None,
        preference_class: None,
        selected_format: None,
        compression_kind: None,
        archive_member_path: None,
        fetch_namespace_kind: None,
        fetch_namespace_id: None,
        local_blob_path: None,
        drop_reason: Some(drop_reason),
    }
}

struct DroppedBlobEvidence {
    selected_format: String,
    source_blob_sha256: Hash256,
    source_blob_size_bytes: u64,
    blob_path: PathBuf,
}

fn dropped_source_with_blob(
    candidate: &CatalogCandidate,
    options: &GutenbergHarvestOptions,
    source_candidate: &SourceCandidate,
    blob: DroppedBlobEvidence,
    drop_reason: HarvestDropReason,
) -> HarvestedSource {
    HarvestedSource {
        book_id: candidate.id,
        title: candidate.title.clone(),
        author: candidate.author.clone(),
        source_landing_url: format!("https://www.gutenberg.org/ebooks/{}", candidate.id),
        rdf_resource_url: Some(source_candidate.resource.url.clone()),
        mirror_fetch_url: Some(source_candidate.resource.url.clone()),
        source_blob_sha256: Some(blob.source_blob_sha256),
        source_blob_size_bytes: Some(blob.source_blob_size_bytes),
        pre_strip_utf8_sha256: None,
        pre_strip_utf8_size_bytes: None,
        media_type: Some(source_candidate.resource.media_type.clone()),
        charset: source_candidate.resource.charset.clone(),
        extent_declared: source_candidate.resource.extent,
        preference_class: Some(source_candidate.preference_class),
        selected_format: Some(blob.selected_format),
        compression_kind: Some(source_candidate.compression_kind),
        archive_member_path: None,
        fetch_namespace_kind: Some(options.fetch_namespace_kind.clone()),
        fetch_namespace_id: Some(options.fetch_namespace_id.clone()),
        local_blob_path: Some(blob.blob_path.display().to_string()),
        drop_reason: Some(drop_reason),
    }
}

fn drop_reason_for_decode_error(error: &GutenbergHarvestError) -> Option<HarvestDropReason> {
    match error {
        GutenbergHarvestError::NoPlaintextArchiveMember { .. } => {
            Some(HarvestDropReason::NoPlaintextArchiveMember)
        }
        GutenbergHarvestError::AmbiguousPlaintextArchive { .. } => {
            Some(HarvestDropReason::AmbiguousPlaintextArchive)
        }
        GutenbergHarvestError::InvalidUtf8 { .. } => Some(HarvestDropReason::InvalidUtf8),
        GutenbergHarvestError::SourceDecodeFailed { .. } => {
            Some(HarvestDropReason::SourceDecodeFailed)
        }
        _ => None,
    }
}

fn read_cached_source_record(book_dir: &Path) -> Option<CachedSourceRecord> {
    let bytes = std::fs::read(book_dir.join("source_record.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

impl SourceCandidate {
    fn from_resource(resource: &PlaintextResource) -> Result<Self, GutenbergHarvestError> {
        let canonical_url = canonicalize_resource_url(&resource.url)?;
        let mime_type = media_type_base(&resource.media_type).unwrap_or("text/plain");
        let url_path = Url::parse(&resource.url)
            .ok()
            .map(|url| url.path().to_ascii_lowercase())
            .unwrap_or_else(|| resource.url.to_ascii_lowercase());
        let compression_kind = if is_gzip_mime(mime_type) || url_path.ends_with(".gz") {
            CompressionKind::Gzip
        } else if mime_type == "application/zip" || url_path.ends_with(".zip") {
            CompressionKind::Zip
        } else if mime_type == "text/plain"
            || url_path.ends_with(".txt")
            || url_path.ends_with(".utf8")
        {
            CompressionKind::None
        } else {
            return Err(GutenbergHarvestError::UnsupportedResourceFormat {
                url: resource.url.clone(),
                media_type: resource.media_type.clone(),
            });
        };

        let charset_raw = resource
            .charset
            .as_deref()
            .or_else(|| charset_from_media_type(&resource.media_type));
        let (charset_or_empty, effective_charset) = canonical_charset(charset_raw)?;
        let preference_class = match (compression_kind, effective_charset) {
            (CompressionKind::None, CharsetKind::Utf8) => 1,
            (CompressionKind::Gzip | CompressionKind::Zip, CharsetKind::Utf8) => 2,
            (CompressionKind::None, _) => 3,
            (CompressionKind::Gzip | CompressionKind::Zip, _) => 4,
        };

        Ok(Self {
            resource: resource.clone(),
            canonical_url,
            mime_type: mime_type.to_owned(),
            charset_or_empty,
            effective_charset,
            compression_kind,
            preference_class,
        })
    }

    fn canonical_format_id(&self, archive_member_path: &str) -> String {
        format!(
            "{}\n{}\n{}\n{}\n{}",
            self.mime_type,
            self.charset_or_empty,
            self.compression_kind.as_str(),
            archive_member_path,
            self.canonical_url
        )
    }
}

fn decode_source_blob(
    book_id: u32,
    candidate: &SourceCandidate,
    raw_blob: &[u8],
) -> Result<DecodedPlaintext, GutenbergHarvestError> {
    match candidate.compression_kind {
        CompressionKind::None => Ok(DecodedPlaintext {
            utf8_bytes: decode_to_utf8(book_id, candidate.effective_charset, raw_blob)?,
            archive_member_path: None,
        }),
        CompressionKind::Gzip => {
            let mut decoder = GzDecoder::new(Cursor::new(raw_blob));
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).map_err(|source| {
                GutenbergHarvestError::SourceDecodeFailed {
                    book_id,
                    message: format!("gzip decode failed: {source}"),
                }
            })?;
            Ok(DecodedPlaintext {
                utf8_bytes: decode_to_utf8(book_id, candidate.effective_charset, &decompressed)?,
                archive_member_path: None,
            })
        }
        CompressionKind::Zip => select_zip_plaintext_member(book_id, candidate, raw_blob),
    }
}

fn select_zip_plaintext_member(
    book_id: u32,
    candidate: &SourceCandidate,
    raw_blob: &[u8],
) -> Result<DecodedPlaintext, GutenbergHarvestError> {
    let reader = Cursor::new(raw_blob);
    let mut archive =
        ZipArchive::new(reader).map_err(|source| GutenbergHarvestError::SourceDecodeFailed {
            book_id,
            message: format!("zip open failed: {source}"),
        })?;
    let mut members: BTreeMap<(u8, String, u64), Vec<u8>> = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|source| {
            GutenbergHarvestError::SourceDecodeFailed {
                book_id,
                message: format!("zip member read failed: {source}"),
            }
        })?;
        let Some(normalized_path) = normalize_zip_member_path(file.name_raw()) else {
            continue;
        };
        if !is_plaintext_member_path(&normalized_path) && candidate.mime_type != "text/plain" {
            continue;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|source| {
            GutenbergHarvestError::SourceDecodeFailed {
                book_id,
                message: format!("zip member bytes failed: {source}"),
            }
        })?;
        let key = (candidate.preference_class, normalized_path, file.size());
        if let Some(existing) = members.get(&key)
            && existing != &bytes
        {
            return Err(GutenbergHarvestError::AmbiguousPlaintextArchive {
                book_id,
                member_path: key.1,
            });
        }
        members.insert(key, bytes);
    }

    let Some(((_, member_path, _), bytes)) = members.into_iter().next() else {
        return Err(GutenbergHarvestError::NoPlaintextArchiveMember { book_id });
    };

    Ok(DecodedPlaintext {
        utf8_bytes: decode_to_utf8(book_id, candidate.effective_charset, &bytes)?,
        archive_member_path: Some(member_path),
    })
}

fn normalize_zip_member_path(raw: &[u8]) -> Option<String> {
    let raw = std::str::from_utf8(raw).ok()?;
    let replaced = raw.replace('\\', "/");
    if replaced.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        parts.push(part);
    }
    let first = parts.first()?;
    if first.starts_with('.') || *first == "__MACOSX" {
        return None;
    }
    Some(unicode_nfc(&parts.join("/")))
}

fn is_plaintext_member_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".txt") || lower.ends_with(".utf8")
}

fn decode_to_utf8(
    book_id: u32,
    charset: CharsetKind,
    bytes: &[u8],
) -> Result<Vec<u8>, GutenbergHarvestError> {
    match charset {
        CharsetKind::Utf8 => String::from_utf8(bytes.to_vec())
            .map(|text| text.into_bytes())
            .map_err(|source| GutenbergHarvestError::InvalidUtf8 { book_id, source }),
        CharsetKind::UsAscii => {
            if let Some((index, byte)) = bytes
                .iter()
                .copied()
                .enumerate()
                .find(|(_, byte)| *byte > 0x7f)
            {
                return Err(GutenbergHarvestError::SourceDecodeFailed {
                    book_id,
                    message: format!("US-ASCII byte 0x{byte:02x} at offset {index}"),
                });
            }
            Ok(bytes.to_vec())
        }
        CharsetKind::Iso8859_1 => {
            let text = bytes
                .iter()
                .map(|byte| char::from(*byte))
                .collect::<String>();
            Ok(text.into_bytes())
        }
        CharsetKind::Windows1252 => {
            let mut text = String::with_capacity(bytes.len());
            for (index, byte) in bytes.iter().copied().enumerate() {
                let Some(ch) = windows_1252_char(byte) else {
                    return Err(GutenbergHarvestError::SourceDecodeFailed {
                        book_id,
                        message: format!(
                            "undefined windows-1252 byte 0x{byte:02x} at offset {index}"
                        ),
                    });
                };
                text.push(ch);
            }
            Ok(text.into_bytes())
        }
    }
}

fn windows_1252_char(byte: u8) -> Option<char> {
    match byte {
        0x00..=0x7f | 0xa0..=0xff => Some(char::from(byte)),
        0x80 => Some('\u{20ac}'),
        0x82 => Some('\u{201a}'),
        0x83 => Some('\u{0192}'),
        0x84 => Some('\u{201e}'),
        0x85 => Some('\u{2026}'),
        0x86 => Some('\u{2020}'),
        0x87 => Some('\u{2021}'),
        0x88 => Some('\u{02c6}'),
        0x89 => Some('\u{2030}'),
        0x8a => Some('\u{0160}'),
        0x8b => Some('\u{2039}'),
        0x8c => Some('\u{0152}'),
        0x8e => Some('\u{017d}'),
        0x91 => Some('\u{2018}'),
        0x92 => Some('\u{2019}'),
        0x93 => Some('\u{201c}'),
        0x94 => Some('\u{201d}'),
        0x95 => Some('\u{2022}'),
        0x96 => Some('\u{2013}'),
        0x97 => Some('\u{2014}'),
        0x98 => Some('\u{02dc}'),
        0x99 => Some('\u{2122}'),
        0x9a => Some('\u{0161}'),
        0x9b => Some('\u{203a}'),
        0x9c => Some('\u{0153}'),
        0x9e => Some('\u{017e}'),
        0x9f => Some('\u{0178}'),
        _ => None,
    }
}

fn render_fixture_toml(
    options: &GutenbergHarvestOptions,
    catalog: &CatalogSnapshot,
    catalog_sha256: Hash256,
    catalog_local_path: &Path,
    selection: &CatalogSelection,
    sources: &[HarvestedSource],
) -> Result<String, GutenbergHarvestError> {
    let mut lines: Vec<String> = Vec::new();
    macro_rules! add {
        ($line:expr) => {
            lines.push(($line).into());
        };
    }

    add!("# fixtures/corpora/gutenberg.toml");
    add!("# F-S4 fixture pin file. Records the network-derived inputs to");
    add!("# `gbf s4 build-corpus`. See history/rfcs/F-S4-gutenberg-promotion.md.");
    add!("#");
    add!("# Emitted by `gbf s4 harvest-gutenberg-fixture`.");
    add!("");
    add!("schema = \"gutenberg_fixture.v1\"");
    add!("source_name = \"Project Gutenberg\"");
    add!("");

    add!("[catalog_snapshot]");
    add!(format!("url = {}", toml_str(&options.catalog_url)));
    add!(format!("sha256 = {}", toml_str(&catalog_sha256.to_hex())));
    add!(format!("size_bytes = {}", catalog.bytes.len()));
    add!(format!(
        "observed_at_utc = {}",
        toml_str(&options.catalog_observed_at_utc)
    ));
    if let Some(last_modified) = catalog
        .last_modified_utc
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        add!(format!("last_modified_utc = {}", toml_str(last_modified)));
    }
    add!(format!(
        "local_path = {}",
        toml_str(&catalog_local_path.display().to_string())
    ));
    add!("");

    add!("[selection_filter]");
    add!("# Verbatim §D1 filter, applied to the RDF catalog snapshot.");
    add!(format!(
        "canonical_json = {}",
        toml_str(SELECTION_FILTER_CANONICAL_JSON)
    ));
    add!(format!(
        "sha256 = {}",
        toml_str(&sha256(SELECTION_FILTER_CANONICAL_JSON.as_bytes()).to_hex())
    ));
    add!("");

    add!("[rank_selection]");
    add!("# §D1 deterministic top-1500 selection by rank_key.");
    add!(format!(
        "rank_prefix_ascii = {}",
        toml_str(RANK_PREFIX_ASCII)
    ));
    add!(format!("target_slice = {}", options.target_slice));
    add!(format!("candidates_total = {}", selection.candidates_total));
    add!(format!(
        "book_ids_self_hash_sha256 = {}",
        toml_str(&selection.book_ids_self_hash_sha256.to_hex())
    ));
    add!(format!("book_count = {}", selection.book_ids.len()));
    add!("");

    add!("[book_ids]");
    add!("# §D1 selected IDs, sorted ascending.");
    add!(format!(
        "values = {}",
        toml_array_of_u32(&selection.book_ids)
    ));
    add!("");

    add!("# §D1 source pins: one [[sources]] table per selected book id.");
    add!("");
    for source in sources {
        add!("[[sources]]");
        add!(format!("book_id = {}", source.book_id));
        add!(format!("title = {}", toml_str(&source.title)));
        add!(format!("author = {}", toml_str(&source.author)));
        add!(format!(
            "source_landing_url = {}",
            toml_str(&source.source_landing_url)
        ));
        if let Some(rdf_resource_url) = &source.rdf_resource_url {
            add!(format!("rdf_resource_url = {}", toml_str(rdf_resource_url)));
        }
        if let Some(mirror_fetch_url) = &source.mirror_fetch_url {
            add!(format!("mirror_fetch_url = {}", toml_str(mirror_fetch_url)));
        }
        if let Some(source_blob_sha256) = source.source_blob_sha256 {
            add!(format!(
                "source_blob_sha256 = {}",
                toml_str(&source_blob_sha256.to_hex())
            ));
        }
        if let Some(source_blob_size_bytes) = source.source_blob_size_bytes {
            add!(format!("source_blob_size_bytes = {source_blob_size_bytes}"));
        }
        if let Some(pre_strip_utf8_sha256) = source.pre_strip_utf8_sha256 {
            add!(format!(
                "pre_strip_utf8_sha256 = {}",
                toml_str(&pre_strip_utf8_sha256.to_hex())
            ));
        }
        if let Some(pre_strip_utf8_size_bytes) = source.pre_strip_utf8_size_bytes {
            add!(format!(
                "pre_strip_utf8_size_bytes = {pre_strip_utf8_size_bytes}"
            ));
        }
        if let Some(media_type) = &source.media_type {
            add!(format!("media_type = {}", toml_str(media_type)));
        }
        if let Some(charset) = &source.charset {
            add!(format!("charset = {}", toml_str(charset)));
        }
        if let Some(extent) = source.extent_declared {
            add!(format!("extent_declared = {extent}"));
        }
        if let Some(preference_class) = source.preference_class {
            add!(format!("preference_class = {preference_class}"));
        }
        if let Some(selected_format) = &source.selected_format {
            add!(format!("selected_format = {}", toml_str(selected_format)));
        }
        if let Some(compression_kind) = source.compression_kind {
            add!(format!(
                "compression_kind = {}",
                toml_str(compression_kind.as_str())
            ));
        }
        if let Some(member) = &source.archive_member_path {
            add!(format!("archive_member_path = {}", toml_str(member)));
        }
        if let Some(fetch_namespace_kind) = &source.fetch_namespace_kind {
            add!(format!(
                "fetch_namespace_kind = {}",
                toml_str(fetch_namespace_kind)
            ));
        }
        if let Some(fetch_namespace_id) = &source.fetch_namespace_id {
            add!(format!(
                "fetch_namespace_id = {}",
                toml_str(fetch_namespace_id)
            ));
        }
        if let Some(local_blob_path) = &source.local_blob_path {
            add!(format!("local_blob_path = {}", toml_str(local_blob_path)));
        }
        if let Some(drop_reason) = source.drop_reason {
            add!(format!("drop_reason = {}", toml_str(drop_reason.as_str())));
        }
        add!("");
    }

    Ok(lines.join("\n"))
}

fn rank_key(book_id: u32) -> Hash256 {
    let mut bytes = Vec::with_capacity(RANK_PREFIX_ASCII.len() + 4);
    bytes.extend_from_slice(RANK_PREFIX_ASCII.as_bytes());
    bytes.extend_from_slice(&book_id.to_le_bytes());
    sha256(bytes)
}

fn book_ids_self_hash(book_ids: &[u32]) -> Hash256 {
    let joined = book_ids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    sha256(joined.as_bytes())
}

struct FetchedBytes {
    bytes: Vec<u8>,
}

fn fetch_url_bytes(
    raw_url: &str,
    user_agent: &str,
    timeout_seconds: u64,
) -> Result<FetchedBytes, GutenbergHarvestError> {
    let url = Url::parse(raw_url).map_err(|source| GutenbergHarvestError::Url {
        value: raw_url.to_owned(),
        source,
    })?;
    match url.scheme() {
        "file" => {
            let path = url
                .to_file_path()
                .map_err(|()| GutenbergHarvestError::FileUrlToPath {
                    url: raw_url.to_owned(),
                })?;
            let bytes = std::fs::read(&path).map_err(|source| GutenbergHarvestError::Io {
                path: path.display().to_string(),
                source,
            })?;
            Ok(FetchedBytes { bytes })
        }
        "http" | "https" => {
            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(timeout_seconds))
                .build();
            let response = agent
                .get(raw_url)
                .set("User-Agent", user_agent)
                .call()
                .map_err(|source| GutenbergHarvestError::Fetch {
                    url: raw_url.to_owned(),
                    message: source.to_string(),
                })?;
            let mut reader = response.into_reader();
            let mut bytes = Vec::new();
            reader
                .read_to_end(&mut bytes)
                .map_err(|source| GutenbergHarvestError::FetchRead {
                    url: raw_url.to_owned(),
                    source,
                })?;
            Ok(FetchedBytes { bytes })
        }
        scheme => Err(GutenbergHarvestError::UnsupportedUrlScheme {
            url: raw_url.to_owned(),
            scheme: scheme.to_owned(),
        }),
    }
}

fn canonicalize_resource_url(raw_url: &str) -> Result<String, GutenbergHarvestError> {
    if !raw_url.is_ascii() {
        return Err(GutenbergHarvestError::LossyUrlCanonicalization {
            url: raw_url.to_owned(),
            message: "URL contains non-ASCII characters".to_owned(),
        });
    }
    if raw_url.bytes().any(|byte| byte <= 0x20 || byte == 0x7f) {
        return Err(GutenbergHarvestError::LossyUrlCanonicalization {
            url: raw_url.to_owned(),
            message: "URL contains ASCII control or space characters".to_owned(),
        });
    }
    if raw_url.contains('\\') {
        return Err(GutenbergHarvestError::LossyUrlCanonicalization {
            url: raw_url.to_owned(),
            message: "URL contains backslash path separators".to_owned(),
        });
    }
    validate_percent_encoding(raw_url)?;

    let url = Url::parse(raw_url).map_err(|source| GutenbergHarvestError::Url {
        value: raw_url.to_owned(),
        source,
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(GutenbergHarvestError::LossyUrlCanonicalization {
            url: raw_url.to_owned(),
            message: "URL userinfo is outside S4 canonicalization".to_owned(),
        });
    }
    let scheme_end =
        raw_url
            .find(':')
            .ok_or_else(|| GutenbergHarvestError::LossyUrlCanonicalization {
                url: raw_url.to_owned(),
                message: "URL has no scheme separator".to_owned(),
            })?;
    let scheme = raw_url[..scheme_end].to_ascii_lowercase();
    let mut rest = &raw_url[scheme_end + 1..];
    let mut canonical = String::with_capacity(raw_url.len());
    canonical.push_str(&scheme);
    canonical.push(':');
    let mut had_authority = false;

    if let Some(after_authority_prefix) = rest.strip_prefix("//") {
        had_authority = true;
        rest = after_authority_prefix;
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        canonical.push_str("//");
        canonical.push_str(&canonical_authority(authority, raw_url)?);
        rest = &rest[authority_end..];
    }

    let query_start = rest.find('?');
    let fragment_start = rest.find('#');
    let path_end = match (query_start, fragment_start) {
        (Some(query), Some(fragment)) => query.min(fragment),
        (Some(query), None) => query,
        (None, Some(fragment)) => fragment,
        (None, None) => rest.len(),
    };
    let path = &rest[..path_end];
    if had_authority && path.is_empty() {
        canonical.push('/');
    } else {
        canonical.push_str(&percent_decode_unreserved(path));
    }

    if let Some(query_start) = query_start
        && fragment_start.is_none_or(|fragment| query_start < fragment)
    {
        let query_end = fragment_start.unwrap_or(rest.len());
        canonical.push('?');
        canonical.push_str(&canonical_query(&rest[query_start + 1..query_end]));
    }

    Ok(canonical)
}

fn validate_percent_encoding(raw: &str) -> Result<(), GutenbergHarvestError> {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || hex_value(bytes[index + 1]).is_none()
                || hex_value(bytes[index + 2]).is_none()
            {
                return Err(GutenbergHarvestError::LossyUrlCanonicalization {
                    url: raw.to_owned(),
                    message: "URL contains an invalid percent escape".to_owned(),
                });
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn canonical_authority(authority: &str, raw_url: &str) -> Result<String, GutenbergHarvestError> {
    if authority.contains('@') {
        return Err(GutenbergHarvestError::LossyUrlCanonicalization {
            url: raw_url.to_owned(),
            message: "URL userinfo is outside S4 canonicalization".to_owned(),
        });
    }
    if authority.is_empty() {
        return Ok(String::new());
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let Some(host_end) = rest.find(']') else {
            return Err(GutenbergHarvestError::LossyUrlCanonicalization {
                url: raw_url.to_owned(),
                message: "URL has an unterminated IPv6 host".to_owned(),
            });
        };
        let host = &rest[..host_end];
        let after_host = &rest[host_end + 1..];
        if !after_host.is_empty()
            && !after_host
                .strip_prefix(':')
                .is_some_and(|port| port.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(GutenbergHarvestError::LossyUrlCanonicalization {
                url: raw_url.to_owned(),
                message: "URL authority has invalid IPv6 port syntax".to_owned(),
            });
        }
        return Ok(format!("[{}]{}", host.to_ascii_lowercase(), after_host));
    }

    let (host, port) = authority
        .rsplit_once(':')
        .map_or((authority, ""), |(host, port)| {
            if port.bytes().all(|byte| byte.is_ascii_digit()) {
                (host, port)
            } else {
                (authority, "")
            }
        });
    if host.contains(':') || host.contains('%') {
        return Err(GutenbergHarvestError::LossyUrlCanonicalization {
            url: raw_url.to_owned(),
            message: "URL host cannot be canonicalized losslessly".to_owned(),
        });
    }
    let mut canonical = host.to_ascii_lowercase();
    if !port.is_empty() {
        canonical.push(':');
        canonical.push_str(port);
    }
    Ok(canonical)
}

fn canonical_query(raw_query: &str) -> String {
    let mut pairs = raw_query
        .split('&')
        .map(|pair| {
            let (key, value, has_equals) = pair
                .split_once('=')
                .map_or((pair, "", false), |(key, value)| (key, value, true));
            (
                percent_decode_unreserved(key),
                percent_decode_unreserved(value),
                has_equals,
            )
        })
        .collect::<Vec<_>>();
    pairs.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    pairs
        .into_iter()
        .map(|(key, value, has_equals)| {
            if has_equals {
                format!("{key}={value}")
            } else {
                key
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_decode_unreserved(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut canonical = String::with_capacity(raw.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let decoded = (hex_value(bytes[index + 1]).expect("validated percent escape") << 4)
                | hex_value(bytes[index + 2]).expect("validated percent escape");
            if is_unreserved(decoded) {
                canonical.push(char::from(decoded));
            } else {
                canonical.push('%');
                canonical.push(hex_upper(bytes[index + 1]));
                canonical.push(hex_upper(bytes[index + 2]));
            }
            index += 3;
        } else {
            canonical.push(char::from(bytes[index]));
            index += 1;
        }
    }
    canonical
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_upper(byte: u8) -> char {
    match byte {
        b'a'..=b'f' => char::from(byte - b'a' + b'A'),
        _ => char::from(byte),
    }
}

fn media_type_base(raw: &str) -> Option<&str> {
    let base = raw.split(';').next()?.trim();
    if base.is_empty() {
        None
    } else if base.eq_ignore_ascii_case("text/plain") {
        Some("text/plain")
    } else if base.eq_ignore_ascii_case("application/gzip") {
        Some("application/gzip")
    } else if base.eq_ignore_ascii_case("application/x-gzip") {
        Some("application/x-gzip")
    } else if base.eq_ignore_ascii_case("application/zip") {
        Some("application/zip")
    } else {
        Some(base)
    }
}

fn is_gzip_mime(mime_type: &str) -> bool {
    mime_type == "application/gzip" || mime_type == "application/x-gzip"
}

fn charset_from_media_type(raw: &str) -> Option<&str> {
    raw.split(';').skip(1).find_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
    })
}

fn canonical_charset(raw: Option<&str>) -> Result<(String, CharsetKind), GutenbergHarvestError> {
    let Some(raw) = raw else {
        return Ok((String::new(), CharsetKind::UsAscii));
    };
    let normalized = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
        .replace('_', "-");
    let (label, kind) = match normalized.as_str() {
        "utf-8" | "utf8" => ("utf-8", CharsetKind::Utf8),
        "us-ascii" | "ascii" => ("us-ascii", CharsetKind::UsAscii),
        "iso-8859-1" | "iso8859-1" | "latin1" | "latin-1" => ("iso-8859-1", CharsetKind::Iso8859_1),
        "windows-1252" | "cp1252" => ("windows-1252", CharsetKind::Windows1252),
        _ => {
            return Err(GutenbergHarvestError::UnsupportedCharset {
                charset: raw.to_owned(),
            });
        }
    };
    Ok((label.to_owned(), kind))
}

fn write_bytes_if_changed(path: &Path, bytes: &[u8]) -> Result<(), GutenbergHarvestError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| GutenbergHarvestError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    std::fs::write(path, bytes).map_err(|source| GutenbergHarvestError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn write_string_if_changed(path: &Path, text: &str) -> Result<(), GutenbergHarvestError> {
    write_bytes_if_changed(path, text.as_bytes())
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", toml_escape(s))
}

fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn toml_array_of_u32(values: &[u32]) -> String {
    if values.is_empty() {
        return "[]".to_owned();
    }
    let mut lines = Vec::new();
    let mut line = String::from("[");
    for (index, value) in values.iter().enumerate() {
        let mut token = value.to_string();
        if index + 1 < values.len() {
            token.push_str(", ");
        }
        if line.len() + token.len() > 100 {
            lines.push(line);
            line = format!("  {token}");
        } else {
            line.push_str(&token);
        }
    }
    line.push(']');
    lines.push(line);
    lines.join("\n")
}

/// Errors from the F-S4.04 harvest operation.
#[derive(Debug)]
pub enum GutenbergHarvestError {
    /// The harvest command was invoked without explicit network permission.
    NetworkPermissionRequired,
    /// The requested deterministic target slice was zero.
    InvalidTargetSlice,
    /// The catalog had fewer matching candidates than the requested slice.
    TooFewCandidates {
        /// Matching candidate count observed in the catalog.
        observed: usize,
        /// Minimum count required by the requested target slice.
        required: usize,
    },
    /// A selected ID was not found in the parsed candidate map.
    SelectedBookMissingCandidate {
        /// Gutenberg ebook id.
        book_id: u32,
    },
    /// A book has no supported plaintext or compressed plaintext resource.
    NoSupportedPlaintextFormat {
        /// Gutenberg ebook id.
        book_id: u32,
    },
    /// A zip source contained no normalized plaintext-compatible member.
    NoPlaintextArchiveMember {
        /// Gutenberg ebook id.
        book_id: u32,
    },
    /// A zip source had tied byte-distinct plaintext members.
    AmbiguousPlaintextArchive {
        /// Gutenberg ebook id.
        book_id: u32,
        /// Tied normalized member path.
        member_path: String,
    },
    /// An RDF resource did not match S4 plaintext/compressed media rules.
    UnsupportedResourceFormat {
        /// RDF resource URL.
        url: String,
        /// Declared media type.
        media_type: String,
    },
    /// A plaintext resource declared an unsupported charset.
    UnsupportedCharset {
        /// Raw charset label.
        charset: String,
    },
    /// A harvest URL used a scheme outside `file`, `http`, or `https`.
    UnsupportedUrlScheme {
        /// Original URL.
        url: String,
        /// Parsed URL scheme.
        scheme: String,
    },
    /// A `file://` URL could not be converted into a local path.
    FileUrlToPath {
        /// Original file URL.
        url: String,
    },
    /// URL parsing failed.
    Url {
        /// Original URL.
        value: String,
        /// URL parse error.
        source: url::ParseError,
    },
    /// URL canonicalization would require a lossy rewrite.
    LossyUrlCanonicalization {
        /// Original URL.
        url: String,
        /// Rejection explanation.
        message: String,
    },
    /// RDF XML parsing failed.
    Xml {
        /// Catalog member path being parsed.
        path: String,
        /// XML parser error.
        source: xml::reader::Error,
    },
    /// Filesystem IO failed.
    Io {
        /// Path being read or written.
        path: String,
        /// IO error.
        source: std::io::Error,
    },
    /// HTTP(S) fetch failed before a response body was available.
    Fetch {
        /// URL being fetched.
        url: String,
        /// Fetch error message.
        message: String,
    },
    /// HTTP(S) response body read failed.
    FetchRead {
        /// URL being fetched.
        url: String,
        /// IO error while reading the response body.
        source: std::io::Error,
    },
    /// A UTF-8 resource failed strict UTF-8 decoding.
    InvalidUtf8 {
        /// Gutenberg ebook id.
        book_id: u32,
        /// UTF-8 conversion error.
        source: std::string::FromUtf8Error,
    },
    /// Source decoding, decompression, archive inspection, or charset conversion failed.
    SourceDecodeFailed {
        /// Gutenberg ebook id.
        book_id: u32,
        /// Decode failure message.
        message: String,
    },
    /// A computed count did not fit its target integer type.
    CountOverflow(&'static str),
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// Existing fixture TOML could not be parsed for catalog pin validation.
    ExistingFixtureToml {
        /// Existing fixture path.
        path: String,
        /// TOML parse error.
        source: toml::de::Error,
    },
    /// Existing fixture catalog snapshot hash was malformed.
    InvalidExistingFixtureCatalogSha {
        /// Existing fixture path.
        fixture_path: String,
        /// Rejected hash value.
        value: String,
        /// Hash parse error.
        source: Hash256ParseError,
    },
    /// Existing fixture catalog snapshot hash does not match the observed snapshot bytes.
    CatalogSnapshotShaMismatch {
        /// Existing fixture path.
        fixture_path: String,
        /// Hash pinned in the existing fixture.
        expected: Hash256,
        /// Hash observed for the current catalog bytes.
        observed: Hash256,
    },
    /// A catalog provenance timestamp was not valid RFC3339.
    InvalidCatalogTimestamp {
        /// Timestamp field name.
        field: &'static str,
        /// Rejected timestamp value.
        value: String,
        /// RFC3339 parse error.
        source: time::error::Parse,
    },
    /// Catalog observation time predates the Last-Modified provenance.
    CatalogTimestampOrder {
        /// Observed-at timestamp.
        observed_at_utc: String,
        /// Last-modified timestamp.
        last_modified_utc: String,
    },
    /// RFC3339 timestamp formatting failed.
    TimeFormat(time::error::Format),
}

impl fmt::Display for GutenbergHarvestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetworkPermissionRequired => f.write_str(
                "gbf s4 harvest-gutenberg-fixture requires explicit --network-permitted",
            ),
            Self::InvalidTargetSlice => f.write_str("target slice must be greater than zero"),
            Self::TooFewCandidates { observed, required } => write!(
                f,
                "Gutenberg catalog yielded {observed} candidates, need at least {required}"
            ),
            Self::SelectedBookMissingCandidate { book_id } => {
                write!(f, "selected book {book_id} is missing from candidate map")
            }
            Self::NoSupportedPlaintextFormat { book_id } => {
                write!(f, "book {book_id} has no supported plaintext source format")
            }
            Self::NoPlaintextArchiveMember { book_id } => {
                write!(
                    f,
                    "book {book_id} zip source has no plaintext archive member"
                )
            }
            Self::AmbiguousPlaintextArchive {
                book_id,
                member_path,
            } => write!(
                f,
                "book {book_id} zip source has ambiguous plaintext archive member {member_path:?}"
            ),
            Self::UnsupportedResourceFormat { url, media_type } => {
                write!(
                    f,
                    "unsupported Gutenberg resource format {media_type:?} at {url}"
                )
            }
            Self::UnsupportedCharset { charset } => {
                write!(f, "unsupported Gutenberg plaintext charset {charset:?}")
            }
            Self::UnsupportedUrlScheme { url, scheme } => {
                write!(f, "unsupported URL scheme {scheme:?} for {url}")
            }
            Self::FileUrlToPath { url } => write!(f, "could not convert file URL to path: {url}"),
            Self::Url { value, source } => write!(f, "{value}: {source}"),
            Self::LossyUrlCanonicalization { url, message } => {
                write!(
                    f,
                    "could not canonicalize URL losslessly {url:?}: {message}"
                )
            }
            Self::Xml { path, source } => write!(f, "{path}: {source}"),
            Self::Io { path, source } => write!(f, "{path}: {source}"),
            Self::Fetch { url, message } => write!(f, "{url}: {message}"),
            Self::FetchRead { url, source } => write!(f, "{url}: {source}"),
            Self::InvalidUtf8 { book_id, source } => {
                write!(f, "book {book_id} plaintext is not valid UTF-8: {source}")
            }
            Self::SourceDecodeFailed { book_id, message } => {
                write!(f, "book {book_id} source decode failed: {message}")
            }
            Self::CountOverflow(field) => write!(f, "{field} does not fit in u64"),
            Self::Json(error) => write!(f, "{error}"),
            Self::ExistingFixtureToml { path, source } => {
                write!(
                    f,
                    "existing fixture TOML {path} could not be parsed for catalog snapshot pin verification: {source}"
                )
            }
            Self::InvalidExistingFixtureCatalogSha {
                fixture_path,
                value,
                source,
            } => write!(
                f,
                "existing fixture {fixture_path} has invalid catalog snapshot SHA {value:?}: {source}"
            ),
            Self::CatalogSnapshotShaMismatch {
                fixture_path,
                expected,
                observed,
            } => write!(
                f,
                "catalog snapshot SHA mismatch for existing fixture {fixture_path}: expected {expected}, observed {observed}"
            ),
            Self::InvalidCatalogTimestamp {
                field,
                value,
                source,
            } => {
                write!(f, "{field} must be RFC3339, got {value:?}: {source}")
            }
            Self::CatalogTimestampOrder {
                observed_at_utc,
                last_modified_utc,
            } => write!(
                f,
                "catalog_observed_at_utc {observed_at_utc:?} must be >= catalog_last_modified_utc {last_modified_utc:?}"
            ),
            Self::TimeFormat(error) => write!(f, "{error}"),
        }
    }
}

impl Error for GutenbergHarvestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Url { source, .. } => Some(source),
            Self::Xml { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::FetchRead { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::Json(error) => Some(error),
            Self::ExistingFixtureToml { source, .. } => Some(source),
            Self::InvalidExistingFixtureCatalogSha { source, .. } => Some(source),
            Self::InvalidCatalogTimestamp { source, .. } => Some(source),
            Self::TimeFormat(error) => Some(error),
            _ => None,
        }
    }
}

//! S4 model-free corpus-oracle surface.

use std::collections::{BTreeMap, BTreeSet};

use gbf_artifact::{
    BOS_ID, CHARSET_V1, CanonicalGutenbergManifestWrite, Char, EOS_ID, GutenbergManifest,
    GutenbergSplit, UNK_ID,
};
use gbf_data::{
    GutenbergD3DropReason, UNMAPPABLE_EXAMPLE_DROP_THRESHOLD, encode_charset_v1, strip_gutenberg_d3,
};
use gbf_foundation::Hash256;

use crate::s4::contamination::{
    S4_CONTAMINATION_FINGERPRINT_KIND, S4_CONTAMINATION_NGRAM_N, sha256_high_u64,
};
use crate::s4::schema::{HypothesisStatus, S4Hypothesis, S4Outcome};

/// Structured event emitted only when the fixture-local COr fallback is selected.
///
/// Production/non-fallback COr paths intentionally do not emit this event;
/// subscriber tests pin that negative assertion so fallback substitution cannot
/// become silent.
pub const S4_CORPUS_ORACLE_FALLBACK_USED_EVENT: &str = "s4_corpus_oracle_fallback_used";

/// Structured event emitted when the production COr evaluator is selected.
///
/// This event is deliberately distinct from `s4_corpus_oracle_fallback_used`:
/// downstream evidence can assert a production path ran while also asserting
/// the fallback-used event count stayed zero.
pub const S4_CORPUS_ORACLE_PRODUCTION_STARTED_EVENT: &str = "s4_corpus_oracle_production_started";

/// Structured event emitted for each COr check.
pub const S4_CORPUS_ORACLE_CHECK_EVENT: &str = "s4_corpus_oracle_check";

/// Structured event emitted after all COr checks complete.
pub const S4_CORPUS_ORACLE_OUTCOME_EVENT: &str = "s4_corpus_oracle_outcome";

/// Tracing target for S4 corpus-oracle events.
pub const S4_CORPUS_ORACLE_LOG_TARGET: &str = "gbf_experiments::s4::corpus_oracle";

/// Name of the explicitly selected fallback evaluator.
///
/// This is the documented D17 pivot for the current S4 corpus-side oracle:
/// the suite uses one named fixture-local slow reference instead of separate
/// `S4ArtifactFallback`/`S4DenotationalFallback` types until the real
/// artifact-oracle lane (`bd-c4wg`, F-C2 ArtifactOracle) owns those evaluators.
pub const S4_CORPUS_ORACLE_FIXTURE_FALLBACK: &str = "fixture_local_slow_reference";

/// Name of the non-fallback production COr evaluator.
pub const S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR: &str = "production_corpus_oracle";

type TokenWindow = [u8; S4_CONTAMINATION_NGRAM_N];

/// One model-free COr check from F-S4 §1/D17.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum S4CorpusOracleCheckId {
    /// COr-1 manifest canonical JSON and self-hash round-trip.
    ManifestRoundTrip,
    /// COr-2 Gutenberg D3 stripper idempotence and marker-missing mode.
    StripperIdempotence,
    /// COr-3 charset_v1 token decode/re-encode round-trip.
    CharsetRoundTrip,
    /// COr-4 split determinism and train/val byte replay.
    SplitDeterminism,
    /// COr-5 unmappable aggregate accounting and retained per-doc bound.
    UnmappableAccounting,
    /// COr-6 contamination overlap math and fingerprint/collision contract.
    ContaminationOverlapMath,
}

impl S4CorpusOracleCheckId {
    /// All COr checks in RFC order.
    pub const ALL: [Self; 6] = [
        Self::ManifestRoundTrip,
        Self::StripperIdempotence,
        Self::CharsetRoundTrip,
        Self::SplitDeterminism,
        Self::UnmappableAccounting,
        Self::ContaminationOverlapMath,
    ];

    /// Stable RFC identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManifestRoundTrip => "COr-1",
            Self::StripperIdempotence => "COr-2",
            Self::CharsetRoundTrip => "COr-3",
            Self::SplitDeterminism => "COr-4",
            Self::UnmappableAccounting => "COr-5",
            Self::ContaminationOverlapMath => "COr-6",
        }
    }

    /// Human-readable check label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ManifestRoundTrip => "manifest_round_trip",
            Self::StripperIdempotence => "stripper_idempotence",
            Self::CharsetRoundTrip => "charset_v1_round_trip",
            Self::SplitDeterminism => "split_determinism",
            Self::UnmappableAccounting => "unmappable_accounting",
            Self::ContaminationOverlapMath => "contamination_overlap_math",
        }
    }

    /// Hypothesis refuted when this corpus-side measurement oracle fails.
    #[must_use]
    pub const fn refuted_hypothesis(self) -> S4Hypothesis {
        match self {
            Self::ContaminationOverlapMath => S4Hypothesis::H2,
            _ => S4Hypothesis::H1,
        }
    }

    /// Formal map from COr failures onto existing S4 outcome tags.
    ///
    /// The corpus oracle does not mint COr-specific report outcomes: COr-1
    /// through COr-5 are H1 corpus-integrity failures, while COr-6 is the H2
    /// contamination failure path.
    #[must_use]
    pub const fn refuted_outcome(self) -> S4Outcome {
        match self {
            Self::ContaminationOverlapMath => S4Outcome::FailContamination,
            Self::ManifestRoundTrip
            | Self::StripperIdempotence
            | Self::CharsetRoundTrip
            | Self::SplitDeterminism
            | Self::UnmappableAccounting => S4Outcome::FailCorpusIntegrity,
        }
    }
}

/// One Gutenberg stripper oracle fixture row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4StripperOracleCase {
    /// Raw Gutenberg-shaped UTF-8 bytes before D3 normalization/stripping.
    pub raw_utf8: Vec<u8>,
    /// Pinned post-strip body SHA-256.
    pub expected_post_strip_sha256: Hash256,
}

/// COr-4 split replay fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4SplitDeterminismFixture {
    /// First split map, usually from the manifest build.
    pub expected_split_map: BTreeMap<u32, GutenbergSplit>,
    /// Replayed split map from the same seed/book ids.
    pub replayed_split_map: BTreeMap<u32, GutenbergSplit>,
    /// First train byte stream.
    pub expected_train_bytes: Vec<u8>,
    /// Replayed train byte stream.
    pub replayed_train_bytes: Vec<u8>,
    /// First validation byte stream.
    pub expected_val_bytes: Vec<u8>,
    /// Replayed validation byte stream.
    pub replayed_val_bytes: Vec<u8>,
}

/// Forced-index collision used by COr-6 to prove exact-window disambiguation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4ForcedIndexCollision {
    /// Window appearing in the containing set.
    pub left_window: TokenWindow,
    /// Distinct window appearing in the denominator set.
    pub right_window: TokenWindow,
    /// Shared test-double fingerprint index for both windows.
    pub forced_index: u64,
}

/// COr-6 hand-counted overlap math fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4ContaminationMathFixture {
    /// N-gram width asserted by the fixture.
    pub n: usize,
    /// Fingerprint kind asserted by the fixture.
    pub fingerprint_kind: String,
    /// Unique windows in set A.
    pub windows_a: Vec<TokenWindow>,
    /// Unique windows in set B.
    pub windows_b: Vec<TokenWindow>,
    /// Hand-counted exact intersection size.
    pub expected_intersection_count: u64,
    /// Optional forced-index collisions that must not count without exact equality.
    pub forced_index_collisions: Vec<S4ForcedIndexCollision>,
}

/// Inputs consumed by COr evaluators.
#[derive(Debug, Clone, PartialEq)]
pub struct S4CorpusOracleInputs {
    /// Canonical manifest object used by COr-1.
    pub manifest: GutenbergManifest,
    /// Canonical manifest bytes used by COr-1.
    pub manifest_canonical_json: Vec<u8>,
    /// Stripper fixtures used by COr-2.
    pub stripper_cases: Vec<S4StripperOracleCase>,
    /// 64 KiB-or-smaller charset token prefix used by COr-3.
    pub charset_roundtrip_prefix: Vec<u8>,
    /// Split replay fixture used by COr-4.
    pub split_replay: S4SplitDeterminismFixture,
    /// Manifest accounting fixture used by COr-5.
    pub unmappable_manifest: GutenbergManifest,
    /// Contamination overlap fixture used by COr-6.
    pub contamination_math: S4ContaminationMathFixture,
}

/// Result of one COr check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4CorpusOracleCheckResult {
    /// Check identifier.
    pub check: S4CorpusOracleCheckId,
    /// Whether the check passed.
    pub passed: bool,
    /// Stable detail string for failure diagnostics.
    pub detail: String,
}

/// Result of a COr evaluator suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4CorpusOracleSuiteReport {
    /// Explicit evaluator name.
    pub evaluator_name: &'static str,
    /// Fallback evaluator name when a fallback path was selected.
    pub fallback_name: Option<&'static str>,
    /// Ordered COr check results.
    pub checks: Vec<S4CorpusOracleCheckResult>,
}

impl S4CorpusOracleSuiteReport {
    /// True when every COr check passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed)
    }

    /// True when this report came from an explicit fallback evaluator.
    #[must_use]
    pub const fn used_fallback(&self) -> bool {
        self.fallback_name.is_some()
    }

    /// Failed check ids in deterministic order.
    #[must_use]
    pub fn failed_checks(&self) -> Vec<S4CorpusOracleCheckId> {
        self.checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.check)
            .collect()
    }

    /// Corpus-side hypothesis status derived from COr failures.
    #[must_use]
    pub fn hypothesis_status(&self, hypothesis: S4Hypothesis) -> HypothesisStatus {
        if self
            .checks
            .iter()
            .any(|check| !check.passed && check.check.refuted_hypothesis() == hypothesis)
        {
            HypothesisStatus::Refuted
        } else {
            HypothesisStatus::Confirmed
        }
    }
}

/// Run COr-1..COr-6 through the named fixture-local slow-reference fallback.
#[must_use]
pub fn run_fixture_local_corpus_oracle(inputs: &S4CorpusOracleInputs) -> S4CorpusOracleSuiteReport {
    tracing::info!(
        target: S4_CORPUS_ORACLE_LOG_TARGET,
        event_name = S4_CORPUS_ORACLE_FALLBACK_USED_EVENT,
        fallback_name = S4_CORPUS_ORACLE_FIXTURE_FALLBACK,
        fallback_kind = "named_fixture_local_fallback",
        check_count = 6_u64,
        "s4 corpus oracle fallback selected"
    );

    run_corpus_oracle_evaluator(
        inputs,
        S4_CORPUS_ORACLE_FIXTURE_FALLBACK,
        Some(S4_CORPUS_ORACLE_FIXTURE_FALLBACK),
    )
}

/// Run COr-1..COr-6 through the production, non-fallback corpus-oracle path.
#[must_use]
pub fn run_production_corpus_oracle(inputs: &S4CorpusOracleInputs) -> S4CorpusOracleSuiteReport {
    tracing::info!(
        target: S4_CORPUS_ORACLE_LOG_TARGET,
        event_name = S4_CORPUS_ORACLE_PRODUCTION_STARTED_EVENT,
        evaluator_name = S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR,
        fallback_used = false,
        check_count = 6_u64,
        "s4 corpus oracle production evaluator selected"
    );

    run_corpus_oracle_evaluator(inputs, S4_CORPUS_ORACLE_PRODUCTION_EVALUATOR, None)
}

fn run_corpus_oracle_evaluator(
    inputs: &S4CorpusOracleInputs,
    evaluator_name: &'static str,
    fallback_name: Option<&'static str>,
) -> S4CorpusOracleSuiteReport {
    let checks = vec![
        run_check(
            S4CorpusOracleCheckId::ManifestRoundTrip,
            evaluator_name,
            fallback_name,
            || check_manifest_round_trip(inputs),
        ),
        run_check(
            S4CorpusOracleCheckId::StripperIdempotence,
            evaluator_name,
            fallback_name,
            || check_stripper_idempotence(inputs),
        ),
        run_check(
            S4CorpusOracleCheckId::CharsetRoundTrip,
            evaluator_name,
            fallback_name,
            || check_charset_roundtrip(inputs),
        ),
        run_check(
            S4CorpusOracleCheckId::SplitDeterminism,
            evaluator_name,
            fallback_name,
            || check_split_determinism(inputs),
        ),
        run_check(
            S4CorpusOracleCheckId::UnmappableAccounting,
            evaluator_name,
            fallback_name,
            || check_unmappable_accounting(inputs),
        ),
        run_check(
            S4CorpusOracleCheckId::ContaminationOverlapMath,
            evaluator_name,
            fallback_name,
            || check_contamination_overlap_math(inputs),
        ),
    ];

    let report = S4CorpusOracleSuiteReport {
        evaluator_name,
        fallback_name,
        checks,
    };
    tracing::info!(
        target: S4_CORPUS_ORACLE_LOG_TARGET,
        event_name = S4_CORPUS_ORACLE_OUTCOME_EVENT,
        evaluator_name = report.evaluator_name,
        fallback_name = report.fallback_name.unwrap_or(""),
        fallback_used = report.used_fallback(),
        passed = report.passed(),
        failed_check_count = report.failed_checks().len() as u64,
        "s4 corpus oracle outcome"
    );
    report
}

fn run_check(
    check: S4CorpusOracleCheckId,
    evaluator_name: &'static str,
    fallback_name: Option<&'static str>,
    f: impl FnOnce() -> Result<(), String>,
) -> S4CorpusOracleCheckResult {
    let result = match f() {
        Ok(()) => S4CorpusOracleCheckResult {
            check,
            passed: true,
            detail: "pass".to_owned(),
        },
        Err(detail) => S4CorpusOracleCheckResult {
            check,
            passed: false,
            detail,
        },
    };
    tracing::info!(
        target: S4_CORPUS_ORACLE_LOG_TARGET,
        event_name = S4_CORPUS_ORACLE_CHECK_EVENT,
        evaluator_name = evaluator_name,
        fallback_name = fallback_name.unwrap_or(""),
        fallback_used = fallback_name.is_some(),
        check_id = check.as_str(),
        check_label = check.label(),
        refuted_hypothesis = ?check.refuted_hypothesis(),
        refuted_outcome = ?check.refuted_outcome(),
        passed = result.passed,
        detail = result.detail.as_str(),
        "s4 corpus oracle check"
    );
    result
}

fn check_manifest_round_trip(inputs: &S4CorpusOracleInputs) -> Result<(), String> {
    let encoded = CanonicalGutenbergManifestWrite::to_vec(&inputs.manifest)
        .map_err(|error| format!("manifest canonical write failed: {error}"))?;
    if encoded != inputs.manifest_canonical_json {
        return Err("canonical manifest bytes changed across encode/decode/encode".to_owned());
    }
    let decoded: GutenbergManifest = serde_json::from_slice(&encoded)
        .map_err(|error| format!("manifest decode failed: {error}"))?;
    if decoded.manifest_self_hash != inputs.manifest.manifest_self_hash {
        return Err("manifest_self_hash changed across round-trip".to_owned());
    }
    Ok(())
}

fn check_stripper_idempotence(inputs: &S4CorpusOracleInputs) -> Result<(), String> {
    if inputs.stripper_cases.is_empty() {
        return Err("stripper fixture list is empty".to_owned());
    }
    for case in &inputs.stripper_cases {
        let stripped = strip_gutenberg_d3(&case.raw_utf8)
            .map_err(|reason| format!("stripper rejected fixture: {reason}"))?;
        if stripped.post_strip_sha256 != case.expected_post_strip_sha256 {
            return Err("post-strip body sha256 did not match fixture pin".to_owned());
        }
        let already_stripped_ok = already_stripped_identity(stripped.body.as_bytes());
        if already_stripped_ok != stripped.body.as_bytes() {
            return Err("already-stripped-ok identity fallback changed body bytes".to_owned());
        }
        match strip_gutenberg_d3(stripped.body.as_bytes()) {
            Err(GutenbergD3DropReason::GutenbergMarkerMissing) => {}
            Ok(_) => {
                return Err("manifest-build second strip unexpectedly found markers".to_owned());
            }
            Err(reason) => {
                return Err(format!(
                    "manifest-build second strip returned {reason}, expected gutenberg_marker_missing"
                ));
            }
        }
    }
    Ok(())
}

fn check_charset_roundtrip(inputs: &S4CorpusOracleInputs) -> Result<(), String> {
    let mut reencoded = Vec::with_capacity(inputs.charset_roundtrip_prefix.len());
    for (position, &id) in inputs.charset_roundtrip_prefix.iter().enumerate() {
        match id {
            BOS_ID | EOS_ID => reencoded.push(id),
            id if id == UNK_ID => {
                let (encoded, _) = encode_charset_v1("\u{fffd}");
                reencoded.extend(encoded);
            }
            id => {
                let ch = decode_printable_charset_id(id).ok_or_else(|| {
                    format!("invalid charset_v1 id {id} at prefix position {position}")
                })?;
                let (encoded, _) = encode_charset_v1(&ch.to_string());
                reencoded.extend(encoded);
            }
        }
    }
    if reencoded != inputs.charset_roundtrip_prefix {
        return Err("charset_v1 decode/re-encode changed token ids".to_owned());
    }
    Ok(())
}

fn check_split_determinism(inputs: &S4CorpusOracleInputs) -> Result<(), String> {
    let split = &inputs.split_replay;
    if split.expected_split_map != split.replayed_split_map {
        return Err("book_id -> split map replay differed".to_owned());
    }
    if split.expected_train_bytes != split.replayed_train_bytes {
        return Err("train byte stream replay differed".to_owned());
    }
    if split.expected_val_bytes != split.replayed_val_bytes {
        return Err("val byte stream replay differed".to_owned());
    }
    Ok(())
}

fn check_unmappable_accounting(inputs: &S4CorpusOracleInputs) -> Result<(), String> {
    let mut unmappable = 0_u128;
    let mut total = 0_u128;
    for source in &inputs.unmappable_manifest.sources {
        if source.drop_reason.is_some() {
            continue;
        }
        let count = source
            .unmappable_count
            .ok_or_else(|| format!("retained book {} missing unmappable_count", source.book_id))?;
        let body = source.post_charset_token_length.ok_or_else(|| {
            format!(
                "retained book {} missing post_charset_token_length",
                source.book_id
            )
        })?;
        let density = source.unmappable_density.ok_or_else(|| {
            format!(
                "retained book {} missing unmappable_density",
                source.book_id
            )
        })?;
        if body == 0 {
            return Err(format!("retained book {} has empty body", source.book_id));
        }
        let expected_density = count as f64 / body as f64;
        if !rates_match_within_one_ulp(expected_density, density) {
            return Err(format!(
                "retained book {} unmappable_density mismatch",
                source.book_id
            ));
        }
        if density > UNMAPPABLE_EXAMPLE_DROP_THRESHOLD {
            return Err(format!(
                "retained book {} exceeded inherited per-doc unmappable threshold",
                source.book_id
            ));
        }
        unmappable += u128::from(count);
        total += u128::from(body);
    }
    let expected_rate = if total == 0 {
        0.0
    } else {
        unmappable as f64 / total as f64
    };
    if !rates_match_within_one_ulp(
        expected_rate,
        inputs.unmappable_manifest.unmappable_rate_corpus,
    ) {
        return Err("unmappable_rate_corpus does not match slow retained-source sum".to_owned());
    }
    Ok(())
}

fn check_contamination_overlap_math(inputs: &S4CorpusOracleInputs) -> Result<(), String> {
    let fixture = &inputs.contamination_math;
    if fixture.n != S4_CONTAMINATION_NGRAM_N {
        return Err(format!(
            "contamination n must be {}, got {}",
            S4_CONTAMINATION_NGRAM_N, fixture.n
        ));
    }
    if fixture.fingerprint_kind != S4_CONTAMINATION_FINGERPRINT_KIND {
        return Err(format!(
            "fingerprint_kind must be {}, got {}",
            S4_CONTAMINATION_FINGERPRINT_KIND, fixture.fingerprint_kind
        ));
    }
    let set_a = OracleWindowSet::from_fixture(fixture, true);
    let set_b = OracleWindowSet::from_fixture(fixture, false);
    let a_contains_b = set_a.overlap_against_denominator(&set_b)?;
    let b_contains_a = set_b.overlap_against_denominator(&set_a)?;
    let expected = fixture.expected_intersection_count;
    if a_contains_b.overlap_count != expected || b_contains_a.overlap_count != expected {
        return Err("exact intersection count differed from hand-counted fixture".to_owned());
    }
    if a_contains_b.denominator_count != fixture.windows_b.len() as u64 {
        return Err("A contains B denominator did not equal |B|".to_owned());
    }
    if b_contains_a.denominator_count != fixture.windows_a.len() as u64 {
        return Err("B contains A denominator did not equal |A|".to_owned());
    }
    let expected_a_contains_b = expected as f64 / fixture.windows_b.len() as f64;
    let expected_b_contains_a = expected as f64 / fixture.windows_a.len() as f64;
    if a_contains_b.fraction != expected_a_contains_b
        || b_contains_a.fraction != expected_b_contains_a
    {
        return Err("overlap fraction did not match exact IEEE-754 fixture division".to_owned());
    }
    Ok(())
}

fn decode_printable_charset_id(id: u8) -> Option<char> {
    let entry = CHARSET_V1.get(id as usize)?;
    match entry {
        Char::Printable { codepoint, .. } => Some(*codepoint),
        Char::Reserved { .. } | Char::Control { .. } => None,
    }
}

fn already_stripped_identity(input: &[u8]) -> Vec<u8> {
    input.to_vec()
}

fn rates_match_within_one_ulp(expected: f64, observed: f64) -> bool {
    if expected == observed {
        return true;
    }
    if !(expected.is_finite() && observed.is_finite() && expected >= 0.0 && observed >= 0.0) {
        return false;
    }
    expected.to_bits().abs_diff(observed.to_bits()) <= 1
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OracleOverlap {
    overlap_count: u64,
    denominator_count: u64,
    fraction: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OracleWindowSet {
    buckets: BTreeMap<u64, BTreeSet<TokenWindow>>,
    len: usize,
}

impl OracleWindowSet {
    fn from_fixture(fixture: &S4ContaminationMathFixture, left: bool) -> Self {
        let windows = if left {
            &fixture.windows_a
        } else {
            &fixture.windows_b
        };
        let mut buckets = BTreeMap::<u64, BTreeSet<TokenWindow>>::new();
        let mut len = 0_usize;
        for window in windows {
            let fingerprint = forced_or_sha256_high_u64(fixture, *window);
            if buckets.entry(fingerprint).or_default().insert(*window) {
                len += 1;
            }
        }
        Self { buckets, len }
    }

    fn overlap_against_denominator(&self, denominator: &Self) -> Result<OracleOverlap, String> {
        if denominator.len == 0 {
            return Err("contamination denominator set is empty".to_owned());
        }
        let mut overlap_count = 0_u64;
        for (fingerprint, denominator_windows) in &denominator.buckets {
            let Some(containing_windows) = self.buckets.get(fingerprint) else {
                continue;
            };
            for window in denominator_windows {
                if containing_windows.contains(window) {
                    overlap_count = overlap_count
                        .checked_add(1)
                        .ok_or_else(|| "overlap count overflowed".to_owned())?;
                }
            }
        }
        let denominator_count = denominator.len as u64;
        Ok(OracleOverlap {
            overlap_count,
            denominator_count,
            fraction: overlap_count as f64 / denominator_count as f64,
        })
    }
}

fn forced_or_sha256_high_u64(fixture: &S4ContaminationMathFixture, window: TokenWindow) -> u64 {
    fixture
        .forced_index_collisions
        .iter()
        .find_map(|collision| {
            if collision.left_window == window || collision.right_window == window {
                Some(collision.forced_index)
            } else {
                None
            }
        })
        .unwrap_or_else(|| sha256_high_u64(&window))
}

/// Build a COr-6 fixture from windows and hand-counted exact intersection.
#[must_use]
pub fn contamination_math_fixture(
    windows_a: Vec<TokenWindow>,
    windows_b: Vec<TokenWindow>,
    expected_intersection_count: u64,
    forced_index_collisions: Vec<S4ForcedIndexCollision>,
) -> S4ContaminationMathFixture {
    S4ContaminationMathFixture {
        n: S4_CONTAMINATION_NGRAM_N,
        fingerprint_kind: S4_CONTAMINATION_FINGERPRINT_KIND.to_owned(),
        windows_a,
        windows_b,
        expected_intersection_count,
        forced_index_collisions,
    }
}

/// Compute a fixture post-strip SHA-256 for tests that pin raw Gutenberg text.
pub fn fixture_post_strip_sha256(raw_utf8: &[u8]) -> Result<Hash256, GutenbergD3DropReason> {
    strip_gutenberg_d3(raw_utf8).map(|stripped| stripped.post_strip_sha256)
}

# Formal spec pack: F-S4 Promote to Gutenberg

> **Status: DRAFT** — first authoring pass, 2026-05-09. Pre-registered numeric
> thresholds tagged `[ESTIMATE]` are committed by this draft for the purpose
> of falsification but should be reviewed by a corpus-quality human before the
> closure PR for bd-2hmm is merged. Numbers without that tag are normative.
>
> **Amendment 2026-05-17 — enwiki8 dropped.** The Project Gutenberg slice
> turned out larger than originally planned, retiring the need for a third
> corpus. enwiki8 (bd-59af / T16.3) is removed from the slice graph. Two
> consequences propagate through this RFC: (a) S4 no longer hands off a
> third-corpus risk to S8; the "production-scale + held-out test split"
> work at S8 now runs against an amended Gutenberg manifest
> (`gutenberg_manifest.v1` → `v2`, owned by S8), with the test partition
> carved from the v1 train side. (b) F16 (bd-1lin, "Multi-Corpus Training
> Data Preparation") closes at S4, not S8. The Section 19 ambiguity-ledger
> entry A25 is flipped accordingly. No other contract in this RFC changes:
> S4 still owns `gutenberg_manifest.v1` at 90/10 train/val, the per-book
> split rule, contamination, promotion gate, and Gutenberg-side v0_success.

This is the fourth scientific/experimental RFC in the training-contract epic.
Its deliverable is **verified knowledge** that the corpus-progression machinery
fires correctly: a Toy0 dense ternary model trained on TinyStories under S3's
v0_success contract clears a deterministic *promotion gate*, then trains on a
hash-pinned Project Gutenberg slice, then passes v0_success on the Gutenberg
val. S4 is conceptually narrower than S3: it does not invent a new oracle
stratum, a new charset, a new baseline family, or a new workload contract. It
inherits all of those from S3 and proves they generalize across a real,
larger English-prose corpus.

Important interpretation:
  A `Fail-promotion-gate-readiness` result is a successful scientific
  falsification, not an implementation failure. It means the promotion-gate
  implementation was sound, but the canonical S3 checkpoint did not satisfy
  one or more true D8 readiness preconditions.

  A `Fail-promotion-gate` result is different: it means H3 was Refuted and
  the gate implementation is unsound. That is an implementation failure and
  blocks every later corpus transition until fixed.

  S4 retires corpus-progression risk only when H1, H2, H3, H4, H5, and H6 are
  Confirmed. Even then, it does not retire production-scale risk at
  UpperBankCandidate on Gutenberg (S8). Closure of bd-2hmm remains blocked
  unless Gutenberg-side v0_success passes as a mandatory gate.

```text
Spec:
  F-S4 Promote to Gutenberg
  Slice S4 of the training-contract epic (bd-1rb)
  Closure bead: bd-2hmm

Hypothesis-under-test:
  Given the S3 ternary checkpoint that passed v0_success on TinyStories, the
  promotion gate G_TS->Gutenberg accepts that checkpoint, deterministically
  initializes a continuation training run on the hash-pinned Gutenberg
  corpus, produces a final ternary checkpoint that passes v0_success on the
  Gutenberg val, and the three-way oracle agreement (live training,
  ReferenceModelBundle / DenotationalOracle, ArtifactOracle) holds across
  the corpus switch — all under the same fixed five-seed protocol used in
  S1..S3.

Owns:
  hypothesis statements H1..H6 (H7 optional)
  pre-registered prediction tables for S4
  gutenberg_manifest.v1 schema and the S4 canonical instance
  Gutenberg loader, header/footer stripper, and per-document split rule
  s4_corpus_quality.v1, s4_contamination_report.v1
  promotion gate G_TS->Gutenberg semantics + s4_promotion_gate.v1
  CorpusProgressionSchedule (S4 instance: TinyStories then Gutenberg)
  s4_corpus_progression.v1
  s4_baseline_gutenberg.v1
  s4_fp_reference.v1
  Gutenberg-side v0_success workload manifest binding
  s4_gutenberg_run_log.v1, s4_gutenberg_checkpoint.v1, s4_gutenberg_score.v1
  s4_oracle_agreement.v1 (Gutenberg side; reuses S3 contract)
  s4_report.v1
  S4 reproducibility laws
  S4 falsification suite (six broken substitutes)
  S4 measurement-oracle suite (COr-* — corpus integrity oracles)

Does not own:
  ternary QAT contract                 (S2; bd-1xqf)
  Phase A->D scheduler internals       (closed under F4; consumed unchanged)
  charset_v1 normalization spec        (S3; bd-3k8o, F-G2)
  ReferenceModelBundle export          (S3; bd-7lu)
  DenotationalOracle / ArtifactOracle  (S3; F-C1, F-C2)
  v0_success workload definition       (S3; bd-3rsw / F-C4)
  Gutenberg test-partition introduction (S8; amends gutenberg_manifest.v1
    (gutenberg_manifest.v2)               -> v2 by carving a test split from
                                          the v1 train side)
  UpperBankCandidate production-scale  (S8)
    run on Gutenberg
  StructuredWidthGates / M6            (S8)
  BoundedKv vs LinearState A/B         (S5; bd-36y1)
  Multi-timescale LinearState          (S5)
  RuntimeChromeBudget preflight        (S6)
  Shadow compile / EncodedRom path     (S6)
  MoE / router                         (S7)
```

## Decisions

```text
D1 Gutenberg slice provenance
   The Gutenberg corpus consumed by S4 is a deterministically-pinned
   subset of Project Gutenberg's English public-domain catalog drawn by
   the following filter, evaluated against a single dated catalog snapshot:

     filter:
       languages_canonical == ["en"]
       pg_rights           == "Public domain in the USA."
       has_plain_text      == true

   `languages_canonical == ["en"]` means the RDF language set for the
   ebook, after lowercasing and sorting, is exactly the singleton list
   `["en"]`. Multilingual records that merely contain English are excluded
   from S4 unless a later RFC explicitly admits them.

   `has_plain_text` is a derived predicate over RDF file/resource records:
   it is true iff the ebook has at least one RDF resource whose media type
   is plain text or a supported compressed archive containing plain text.
   It is not assumed to be a literal boolean field in the RDF.
     catalog_snapshot_url:
       https://www.gutenberg.org/cache/epub/feeds/rdf-files.tar.bz2
     catalog_snapshot_sha256:
       sha256:<PINNED_AT_FIXTURE_CREATION>     ; draft placeholder only
     catalog_snapshot_observed_at_utc:
       <PINNED_AT_FIXTURE_CREATION>             ; draft placeholder only
     catalog_snapshot_last_modified_utc:
       <PINNED_AT_FIXTURE_CREATION>             ; nullable if unavailable

   The SHA-256 of the RDF tarball, not the URL, observed-at timestamp, or
   Last-Modified timestamp, is the semantic identity of the catalog
   snapshot. The timestamps are provenance fields only.

   Execution RFC constraint:

     parse_rfc3339_utc(catalog_snapshot_observed_at_utc)
       >= parse_rfc3339_utc(catalog_snapshot_last_modified_utc)

   whenever `catalog_snapshot_last_modified_utc` is non-null. A nominal
   date such as midnight UTC is not valid execution provenance unless that
   was the actual observation instant.

   The applied filter is reduced to an explicit candidate list. Because
   Project Gutenberg catalog metadata does not contain original print
   publication dates, S4 does not filter on original publication year unless
   a separate, hash-pinned external publication-year table is added. The
   "pre-1928" framing some prior drafts used is not part of S4 unless a
   separate, hash-pinned publication-year table is added. The S4 legal
   predicate is Project Gutenberg's catalog-side `pg_rights` value
   "Public domain in the USA."; S4 makes no claim about public-domain status
   outside the United States.

   S4 uses a 1500-book target slice rather than every matching English
   public-domain plaintext ebook. Selection is:

     candidate_ids = all ids satisfying the catalog filter above
     rank_key(id)  = (sha256(ascii("gbf:s4:gutenberg-select:v1")
                              || le_u32(id)),
                      id)
     book_ids      = first 1500 ids by rank_key, then sorted ascending

   The `id` tie-breaker is normative even though SHA-256 rank collisions are
   expected to be practically nonexistent. Determinism must not rely on
   collision improbability.

   There is no backfill. If some of these 1500 selected ids are later
   dropped by source-format selection, marker stripping, charset_v1
   normalization, empty-body detection, or deduplication, S4 does not
   replace them with lower-ranked candidate ids. The retained-count guard
   below is the only acceptance criterion.

   Replay never re-evaluates the filter against a live catalog. Replay reads
   `book_ids`, validates the pinned raw-file sha256s, and loads bytes from
   the content-addressed mirror/cache only.

   If multiple plaintext encodings/formats are present for an id in the RDF
   snapshot, candidate plaintext resources are first canonicalized as:

     canonical_format_id =
       mime_type || "\n" ||
       charset_or_empty || "\n" ||
       compression_kind || "\n" ||
       archive_member_path_or_empty || "\n" ||
       canonical_rdf_resource_url

   mime_type is lowercased ASCII after removing parameters and trimming
   surrounding ASCII whitespace. S4 v1 recognizes:

     plaintext media types:
       text/plain

     supported compressed/archive media types:
       application/gzip
       application/x-gzip
       application/zip

   A filename extension is plaintext-compatible iff its final lowercased
   extension is one of:

     .txt
     .utf8

   charset_or_empty is the canonical charset label after ASCII-lowercasing
   and alias normalization. S4 v1 recognizes:

     utf-8, us-ascii, iso-8859-1, windows-1252

   Any other explicit charset is rejected for S4 v1 unless a later RFC
   extends this list.

   URL canonicalization for `canonical_rdf_resource_url` is:

     - lowercase scheme and host,
     - preserve path bytes after percent-decoding only unreserved
       RFC3986 characters,
     - remove URL fragments,
     - sort query parameters by bytewise key/value order,
     - reject URLs that cannot be canonicalized losslessly.

   The selected source blob is chosen by the following deterministic
   preference order:

     1. uncompressed UTF-8 plaintext,
     2. compressed UTF-8 plaintext,
     3. uncompressed plaintext with explicit charset that decodes losslessly
        and re-encodes to UTF-8,
     4. compressed plaintext with explicit charset that decodes losslessly
        and re-encodes to UTF-8.

   Ties within the same preference class are resolved by ascending
   canonical_format_id.

   Supported compression/archive forms for S4 v1 are:

     compression_kind = "none"
     compression_kind = "gzip"
     compression_kind = "zip"

   For gzip, the decompressed stream is the candidate plaintext bytes.

   For zip, candidate archive members are non-directory members whose
   normalized path does not begin with "." or "__MACOSX/" and whose media
   type or extension is plaintext-compatible.

   normalized_member_path is computed as follows:

     - decode the zip member name as UTF-8; otherwise reject that member;
     - replace "\" with "/";
     - split on "/";
     - reject empty components, "." components, and ".." components;
     - reject absolute paths and paths whose first component begins with ".";
     - reject paths whose first component equals "__MACOSX";
     - join remaining components with "/";
     - apply Unicode NFC.

   Archive extraction is logical only: member bytes are read from the zip
   central-directory entry selected by the deterministic rule. The
   implementation must not write archive members to the filesystem.

   If multiple candidate members exist, the selected member is the one
   with the ascending tuple:

     (member_preference_class, normalized_member_path, uncompressed_size)

   If no RDF resource can be canonicalized into a supported plaintext or
   supported compressed-plaintext candidate, the book is dropped with reason
   `no_supported_plaintext_format`.

   If a zip archive is a supported candidate but contains no
   plaintext-compatible member after archive-member normalization, the book
   is dropped with reason `no_plaintext_archive_member`. If a zip archive
   contains multiple byte-distinct plaintext-compatible members tied under
   that tuple, the book is dropped with reason
   `ambiguous_plaintext_archive`.

   The selected format identifier, source-blob sha256, and decoded UTF-8
   pre-strip sha256 are recorded per book.
   A book with no candidate satisfying this order is dropped with reason
   `no_supported_plaintext_format`.

   Retained-count guard:

     retained_book_count_after_all_drops >= 1350

   This guard is a corpus-integrity precondition. S4 targets a 1500-book
   slice; a much smaller retained corpus is a different experiment even if
   train_book_count and val_book_count remain non-zero.

   Fixture construction must not crawl per-book files from the Project
   Gutenberg main website and must not use main-site deep links as replay
   dependencies. Any allowed harvest mechanism must identify the mirror
   snapshot, robot-harvest namespace, or content-addressed cache namespace
   recorded in each GutenbergSourceRecord.
   The fixture-build op uses either:

     - a local/private Project Gutenberg mirror,
     - the official robot/harvest mechanism,
     - or a pre-existing content-addressed cache whose raw-file sha256s
       are pinned in fixtures/corpora/gutenberg.toml.

   Replay is network-disabled and never fetches from gutenberg.org.

   Fixture construction records, for every fetched source blob:

     fetch_namespace_kind:
       "local_private_mirror"
     | "official_robot_harvest"
     | "content_addressed_cache"

     fetch_namespace_id: String

   This field identifies the harvest/mirror/cache namespace. It is
   distinct from the RDF resource URL and from the replay cache path.

D2 Document-level split rule
   The split is computed at the BOOK level. No book straddles splits.

     split_seed_bytes = the first 16 digest bytes of
                        sha256(ascii("gbf:s4:book-split:2026-05-09"))
     split_seed_u128  = split_seed_bytes interpreted as little-endian u128
     train_fraction   = 0.90
     val_fraction     = 0.10
     test_fraction    = 0.00     ; S4 does not allocate a test split.
                                 ; S8 amends gutenberg_manifest.v1 -> v2 by
                                 ; carving a held-out test partition from the
                                 ; v1 train side under a separate pinned
                                 ; test_split_seed_u128 (owned by S8). The
                                 ; v1 val partition is byte-identical in v2.

   For each retained, non-duplicate book id b:

     split_hash = sha256(ascii("gbf:s4:book-split:v1")
                          || split_seed_bytes
                          || le_u32(b))
     u          = high_53_bits_as_f64(split_hash)

   high_53_bits_as_f64(h):
     x = the high-order 53 bits of h, interpreting the first 8 digest bytes
         as a big-endian u64 and shifting right by 11
     return x / 2^53

   Therefore u is always in [0, 1).

   If u < 0.90 -> train; else val.

   The split assignment is independent of dropped books, duplicate-removal
   order, and future fixture amendments that add or remove other ids. It is
   stateless: dropping or amending one book never changes any other retained
   book's split assignment.

   Document-level split rationale: corpora that split mid-book leak
   topical and stylistic n-grams across train and val, contaminating
   the gate. Per-book splits are the standard discipline for Project
   Gutenberg evaluations.

D3 Header / footer stripping
   Project Gutenberg ebooks ship a banner before the body text and a
   trailer after it. The format is heuristic-stable but not strictly
   fixed: leading whitespace, casing, and CR/LF combinations vary across
   the catalog. Both are stripped by a deterministic marker recognizer
   operating on a Unicode string after:

     1. validating UTF-8,
     2. removing a leading UTF-8 BOM if present,
     3. normalizing line endings CRLF/CR -> LF,
     4. applying Unicode NFC.

   Format decoding happens before this step. The selected source blob is
   decoded according to D1's selected format into a UTF-8 byte string.
   A source blob that cannot be decoded losslessly is dropped with reason
   `source_decode_failed`.

   D3 then validates that decoded byte string as UTF-8, removes a leading
   UTF-8 BOM if present, normalizes line endings, and applies NFC. A byte
   string that is not valid UTF-8 at this point is dropped with reason
   `invalid_utf8`.

   Hashes over stripped text are computed over the UTF-8 encoding of the
   normalized string. The recognizer pair is:

     header_regex  = (?im-s)\A(?s:.*?)^[ \t]*\*{3}[ \t]*START OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*\n
     footer_marker_regex =
       (?im-s)\n[ \t]*\*{3}[ \t]*END OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*

   Both regexes are applied after line-ending normalization. Horizontal
   marker whitespace is `[ \t]`, not `\s`; marker matching must not consume
   arbitrary newlines as whitespace.

   Manifest-build stripping algorithm:

     1. find the first header_regex match; fail if no header marker exists;
     2. find all footer_marker_regex matches whose start offset is at or
        after header_match.end;
     3. select the footer marker with maximal start offset;
     4. require header_match.end <= footer_marker.start;
     5. body = normalized_text[header_match.end .. footer_marker.start].

   No leading or trailing body whitespace is trimmed by the stripper. Empty
   bodies are detected after charset_v1 normalization by checking whether
   the body token-id stream excluding <bos>/<eos> has length zero.

   A book that fails either marker check or has marker order reversed is
   dropped with reason `gutenberg_marker_missing` and recorded in the
   manifest's per-book provenance. The dropped count is hard-capped at 5%
   of `book_ids` length; exceeding the cap is a manifest-construction
   failure and the manifest does not finalize.

   A book that produces zero body token ids after stripping and charset_v1
   normalization is dropped with reason `empty_after_strip`.

D4 charset_v1 normalization is inherited from S3
   The deterministic normalization order, the unmappable replacement
   policy, and the per-document drop policy come from F-G2 / bd-3k8o
   verbatim. S4 does not amend any of those rules. It adds a stricter
   aggregate Gutenberg-corpus bound after those inherited per-document
   drops (see D5).

D5 Per-document unmappable bound for Gutenberg
   For each retained book, after charset_v1 normalization:

     unmappable_density(book) =
       count(<unk>) / count(body token ids after charset_v1 normalization)

   <bos> and <eos> are excluded from both numerator and denominator.
   A retained book with zero body token ids is dropped with reason
   `empty_after_strip`.

     drop if unmappable_density(book) > 0.02     ; 2%, inherited from F-G2
     drop reason: `unmappable_density_high`

   Aggregate Gutenberg-corpus unmappable rate, computed on retained body
   token ids from train+val after source-format drops, marker drops,
   charset_v1 per-document drops, empty-body drops, and exact duplicate
   removal, excluding <bos>/<eos>, must satisfy:

     unmappable_rate_corpus_gutenberg <= 0.005    ; 0.5%, hard fail

   This per-corpus aggregate bound is an S4 promotion-gate precondition
   (see D8). Note: planv0/bd-pso7 names <5% as a per-corpus *target*; S4
   tightens that to <=0.5% as the post-normalization actual, because the
   S4 slice is English public-domain-in-USA plaintext and is expected to be
   mostly representable by charset_v1. A higher rate is treated as evidence
   of a charset bug, wrong format selection, or wrong language filter before
   it is treated as a real text property.

D6 Cross-corpus contamination fingerprint
   Contamination is measured by token-id n-gram overlap on
   charset_v1-normalized text. Each token id is serialized as one octet
   before hashing. Both directions are measured:

     n                              = 13     ; character 13-grams
     fingerprint_index_hash         = sha256_high_u64
     collision_disambiguation       = exact_13_token_bytes_on_hit
     comparison_unit                = canonicalized unique 13-token window,
                                       where each token id is serialized as
                                       one octet before hashing
     gated_overlap_policy           = full validation split against full
                                       opposite training split
     diagnostic_sample_cap_token_ids_per_split
                                    = 1_048_576
     overlap_threshold_hard_fail    = 0.0010   ; [ESTIMATE for review]
                                                ; in gated directions,
                                                ; more than 0.10% of the
                                                ; unique validation 13-grams
                                                ; may not also appear in the
                                                ; full opposite training
                                                ; split
     overlap_threshold_warn         = 0.0005   ; [ESTIMATE for review]
                                                ; 0.05% triggers a warning

   The two closure-gated directions are exact, not sampled:

     TS_train_contains_GB_val:
       index every 13-token window in GB_val and test membership against
       the index set of every 13-token window in TS_train. On each
       sha256_high_u64 index hit, compare the full 13-token byte window.
       Count overlap only when the full 13-token window is byte-identical.

     GB_train_contains_TS_val:
       index every 13-token window in TS_val and test membership against
       the index set of every 13-token window in GB_train. On each
       sha256_high_u64 index hit, compare the full 13-token byte window.
       Count overlap only when the full 13-token window is byte-identical.

   Diagnostic-only directions may use the diagnostic sample cap. Diagnostic
   sampling is per-document stratified, not a global leading prefix. For each
   sampled split, every retained document contributes up to:

     per_doc_cap =
       ceil(diagnostic_sample_cap_token_ids_per_split
            / retained_doc_count(split))

   If document_body_token_length <= per_doc_cap, include the whole document.
   Otherwise include deterministic head/middle/tail fragments whose total
   length is per_doc_cap, with fragment boundaries adjusted so 13-grams never
   cross fragment or document boundaries.

   13-grams do not cross document boundaries and do not include <bos> or
   <eos>. <unk> is treated as an ordinary charset_v1 token.

   Rationale for collision disambiguation:
     S4 may use sha256_high_u64 as an index for memory efficiency, but the
     closure-gated contamination result must not rely on hash-collision
     improbability. Exact byte-window confirmation is mandatory before an
     overlap is counted.

   The 13-token-id window is chosen so common short English idioms
   ("the the the", "of the king") do not dominate, while a real
   inadvertent inclusion of an entire Gutenberg passage in TinyStories
   train (or vice versa) lights up the metric. This is independent of
   the n-gram baseline order in §6.2; the 5-gram KN baseline scores
   probability, while the 13-gram contamination check measures literal
   text reuse.

D7 Cross-corpus n-gram baseline
   For Gutenberg, S4 reuses the F-S3-defined 5-gram modified Kneser-Ney
   baseline (S3 owns `kn5` math). The baseline is fit on
   `gutenberg_train` and scored on `gutenberg_val` under the *same*
   reset-context windowed-bpc primitive (chunk_size = 128) defined in S1
   and reused by S3.

   Baseline self-hash, smoothing constants, vocabulary, and chunking
   semantics are inherited unchanged. Only the corpus identity changes.

D8 Promotion gate criterion (G_TS->Gutenberg)
   A TinyStories-trained ternary checkpoint c_TS may begin Gutenberg
   training only if all of the following hold:

     P-1  c_TS is a valid s3_v0_success.v1-passing artifact and a
          Phase-D-resumable S3 checkpoint:
            - every v0_success acceptance bit is set,
            - the workload manifest run is signed by ReferenceModelBundle
              agreement and ArtifactOracle agreement per S3,
            - c_TS contains the deployed ternary weights,
            - c_TS contains the S2 QAT shadow-weight payload needed to
              resume Phase::D without synthesizing FP weights after the fact.
     P-2  Three-way oracle agreement on TinyStories val is recorded in
          the supplied hash-bound s3_oracle_agreement.v1 artifact, whose
          checkpoint_self_hash equals c_TS.checkpoint_self_hash, with all
          per-token bpc gaps within S3-pinned tolerance.

          The word "latest" is not a legal selector for promotion. The
          gate consumes an explicit artifact path plus self-hash.
     P-3  ternary_gap_TS is read from the self-hash-valid
          s3_v0_success.v1 artifact bound to c_TS and satisfies:

            ternary_gap_TS <= 0.5                              ; from S3

          S4 does not recompute or select an unbound "teacher" here.
     P-4  Gutenberg corpus integrity:
            gutenberg_manifest.v1 present, all sha256 fields validated
            against on-disk archive, and §5 G-Ok-1..G-Ok-12 hold.
     P-5  Cross-corpus contamination not hard-failed:
            s4_contamination_report.v1.outcome in {Clean, Warn(_)}
            (D6 thresholds; HardFail is the only rejection condition)
     P-6  Gutenberg unmappable bound:
            unmappable_rate_corpus_gutenberg <= 0.005          (D5)
     P-7  Gutenberg KN-5 baseline finite and reproducible:
            s4_baseline_gutenberg.v1 emitted, baseline_self_hash
            round-trips
     P-8  No repetition collapse signature on the hash-bound TinyStories
          val generation-sample artifact consumed by S3 v0_success and
          bound to c_TS.checkpoint_self_hash.

          The word "latest" is not a legal selector here either. The gate
          consumes an explicit repetition_collapse_check artifact path plus
          self-hash, and verifies that:

            repetition_collapse_check.self_hash round-trips,
            repetition_collapse_check.checkpoint_self_hash =
              c_TS.checkpoint_self_hash,
            repetition_collapse_check.tinystories_manifest_self_hash =
              tinystories_manifest_self_hash,
            repetition_collapse_check.outcome = Pass.

   The promotion gate is a deterministic CLI op (`gbf s4 promote`). Its
   single boolean output Promoted | Rejected(reasons) is recorded in
   s4_promotion_gate.v1 and its self-hash is bound into every downstream
   artifact's metadata.

D9 Gutenberg continuation training initialization
   Continuation training initializes from one promoted S3 ternary
   checkpoint, c_TS_ref. In S4 v1 this is the S3 seed-0 checkpoint:

     c_TS_ref = experiments/S3/checkpoints/seed-0/checkpoint.safetensors

   The S4 seed list {0,1,2,3,4} controls only Gutenberg continuation RNG
   streams. It does not select different S3 starting checkpoints.

   c_TS_ref is the S3 *ternary* (final-phase) checkpoint that passed the
   promotion gate. NOT the S3 fp teacher, NOT a fresh init, NOT the dense
   baseline.

   Rationale: the v0_success contract on Gutenberg val measures the
   deployed (ternary) artifact's quality after corpus progression. A
   fresh-init or fp-init would not test what the bead claims to test.

   Concretely:

     model_weights_initial(s)          = c_TS_ref.weights
     adamw_state_initial(s)            = ZeroInitAdamW
                                          (warm-restart; no momentum reuse)
     phase_state_initial(s)            = Phase::D    ; QAT fully hardened
                                                       (continuation)
     rng_streams_initial(s)            = re-seeded:
                                           InitRng(s),
                                           BatchRng(s) over Gutenberg corpus,
                                           ShuffleRng(s),
                                          all per the exact seed128 domains
                                          defined in §9.1:
                                            "s4-init-init"
                                            "s4-init-batch"
                                            "s4-init-shuffle"

   InitRng exists only to keep the stream registry total. Under D9
   warm-start semantics it must consume zero draws before the first
   optimizer step. Any draw from InitRng to initialize model weights is a
   lineage violation and Refutes H6 via S4-Run-Ok-4.

   ShuffleRng is reserved for future epoch-style samplers. S4 v1's random
   offset sampler does not consume ShuffleRng. Therefore
   shuffle_rng_draw_count_before_first_step = 0 and
   shuffle_rng_draw_count_total = 0 for every S4 v1 run.

   This is a *warm-weight, cold-optimizer* restart. The optimizer state
   is reset because AdamW momentum on TinyStories step distributions is
   ill-matched to Gutenberg gradients and would silently bias early
   updates. Weight reuse is the load-bearing part of "continuation"; it
   is what carries learning forward. Optimizer reuse is not.

D10 Gutenberg training step budget
   For Gutenberg continuation training under Toy0 + Phase D:

     optimizer_steps_gutenberg   = 20000        ; [ESTIMATE for review]
     batch_size                  = 32           ; inherited from S1
     sequence_length             = 128          ; inherited from S1
     eval_every_steps            = 2000
     eval_subset_size            = 4096 sequences
     optimizer                   = AdamW { lr = 5e-4,
                                           beta1 = 0.9,
                                           beta2 = 0.999,
                                           eps = 1e-8,
                                           weight_decay = 0.0 }

   2x the S1 step budget reflects the larger Gutenberg byte stream and
   the broader English vocabulary; lr is halved relative to S1's 1e-3
   because AdamW state was reset and a hot LR on cold momentum is the
   classical recipe for early divergence. These constants are part of
   this RFC. Changing any invalidates prior comparisons and constitutes
   a new experiment.

D11 Fixed seed list
   seeds = [0, 1, 2, 3, 4]
   Five seeds are mandatory. No more, no fewer. Same list as S1, S2, S3.

D12 Same device profile
   All S4 semantic runs (manifest build from pinned fixture, KN baseline
   fitting, training, scoring, oracle, contamination report, promotion gate
   evaluation) execute under the S1CpuDeterministic
   device profile defined in F-S1 §5. env_exact and
   env_forbidden_unless_listed apply unchanged. Network is permitted only
   during the explicit fixture-build CLI op (D1 catalog fetch +
   per-id fetch); the trainer, scorer, oracle, and report stages run with
   network disabled by S1CpuDeterministic.

D13 Fail-closed on NaN / divergence
   Any seed producing non-finite loss or non-finite gradient norm at any
   step of Gutenberg continuation training fails the entire S4. No
   partial pass.

D14 Strict v0_success on Gutenberg
   The S3 v0_success workload manifest is re-instantiated against
   gutenberg_val. Every per-seed acceptance bit must pass. Per-seed
   strictness — not aggregate.

D15 Three-way oracle agreement under corpus switch
   For seed 0 (mandatory), the live-training output, the
   ReferenceModelBundle re-export over the Gutenberg checkpoint, and the
   ArtifactOracle re-execution must agree on the canonical S3-pinned
   conformance fixture set evaluated against gutenberg_val. Tolerance is
   inherited from S3 unchanged.

   Seeds 1..4 are reported as observational.

D16 Corpus-progression replay determinism
   Same per-seed Gutenberg checkpoint bytes are reproduced under:
     same seed +
     same gutenberg_manifest_self_hash +
     same tinystories_manifest_self_hash +
     same c_TS_checkpoint_self_hash +
     same train_config_hash +
     same model_config_hash +
     same gbf-train pass_version +
     same dependency lockfile +
     same rust_toolchain_hash +
     same build_config_hash +
     same device_profile.

D17 Mandatory measurement-oracle falsification (COr corpus-side)
   H1 corpus-integrity is tested by deterministic fixtures independent
   of any trained model. See COr-* in §1, mirroring S1 §D7.

D18 Closure rule (no result placeholders)
   No `[ESTIMATE]`, `<PINNED_AT_FIXTURE_CREATION>`, approximate byte
   length, or placeholder hash may appear in:

     - the prediction-bearing RFC revision used for S4 execution,
     - any fixture consumed by S4 execution,
     - any s4_report.v1 front matter,
     - or any closure-candidate RFC revision.

   All `[ESTIMATE]` thresholds that participate in a falsification rule or
   closure gate must be resolved to concrete values before
   `first_result_commit`. Resolving them after any S4 result artifact exists
   is not pre-registration.

   The current draft legitimately contains placeholders; the S4 execution
   revision and the bd-2hmm closure PR must remove every one of them and pin
   concrete values.
```

---

# 1. Core notation

```text
Hash256        := /^sha256:[0-9a-f]{64}$/
Seed           := u64
TrainStep      := u32
EvalStep       := u32
Step           := u32
LossNatsPerToken := f32     ; finite natural-log cross entropy per target
                            ; charset_v1 token id. Historical artifacts may
                            ; still spell this LossNatsPerByte.

LossNatsPerByte := LossNatsPerToken
                            ; deprecated spelling accepted only when reading
                            ; historical S1..S3 artifacts. New S4 artifacts
                            ; MUST serialize LossNatsPerToken fields.
BpcValue       := f64       ; bits per charset_v1 token id, required finite,
                            ; >= 0; all gates compare in f64
GradNorm       := f32       ; required finite, >= 0; global L2 norm

Verdict          := Confirmed | Refuted
HypothesisStatus :=
    Confirmed
  | Refuted
  | NotEvaluatedDueToPriorGate(reason: String)

FailureKind :=
    CorpusIntegrity
  | Contamination
  | PromotionGate
  | QualityOnGutenberg
  | OracleDisagreement
  | Substrate

; S4 does not emit a top-level generic Outcome. Use S4Outcome in §11.
;
; "Inconclusive" is not a legal S4 value. Missing evidence is represented
; only by NotEvaluatedDueToPriorGate(reason), and only for hypotheses made
; unreachable by an earlier mandatory gate.
;
; Closure-candidate reports (Decision = ProceedToS5) must give binary
; Verdict values for all of H1..H6. Early-failure reports — emitted before
; downstream evidence exists, e.g. promotion-gate rejection short-circuits
; before Gutenberg training even starts — must mark unreachable downstream
; hypotheses as NotEvaluatedDueToPriorGate(reason) rather than asserting
; Confirmed or Refuted on evidence that does not exist.

Hypothesis  := H1 | H2 | H3 | H4 | H5 | H6 | H7

PredictedRange     := { low: BpcValue, high: BpcValue }   ; low <= high
ObservedStatistic  := { median: BpcValue, min: BpcValue, max: BpcValue, stddev: f64 }

Median rule:
  For the fixed five-seed list, median(xs) is the third value after sorting
  the five finite BpcValue values ascending. Median is undefined unless all
  five values exist and are finite.

Stddev rule:
  stddev is the population standard deviation over the five finite values:

    sqrt((1/5) * Σ_i (x_i - mean(xs))^2)

  accumulated in f64.

CharVocab83        := token_id ∈ [0, 79] | <bos>=80 | <eos>=81 | <unk>=82
                                            ; charset_v1, vocab=83
                                            ; (inherited from S3 / F-G2)

NGramOrder         := 1 | 2 | 3 | 5
KneserNeyParams    := { discount: f64, max_order: 5 }   ; values pinned by S3

CorpusManifestRef  := { sha256: Hash256, path: String, schema_version: SemVer }

TinyStoriesManifest := <as defined in F-S1 §1, pinned by S3>

GutenbergSourceRecord :=
  {
    book_id:                u32                   ; Gutenberg ebook id
    title:                  String
    author:                 String
    source_landing_url:     String                ; e.g. /ebooks/{id}; never
                                                  ; fetched at replay time
    mirror_fetch_url:       Null | String         ; private mirror/cache URL;
                                                  ; never a main-site deep link
    mirror_snapshot_id:     Null | String         ; identifier for the mirror
                                                  ; snapshot whose raw bytes
                                                  ; are pinned by
                                                  ; source_blob_sha256
    selected_format:        Null | String
                                                  ; canonical selected
                                                  ; plaintext format id; must
                                                  ; resolve to UTF-8 bytes
                                                  ; before D3
    source_blob_sha256:     Null | Hash256        ; sha256 of fetched source
                                                  ; blob; compressed iff the
                                                  ; selected source is
                                                  ; compressed
    pre_strip_utf8_sha256:  Null | Hash256        ; sha256 of decoded,
                                                  ; decompressed UTF-8 bytes
                                                  ; before BOM removal,
                                                  ; line-ending normalization,
                                                  ; marker stripping, and NFC
    license:                "public_domain_in_usa"; constant per D1 filter;
                                                  ; no worldwide claim
    fetch_namespace_kind:   Null
                          | "local_private_mirror"
                          | "official_robot_harvest"
                          | "content_addressed_cache"
    fetch_namespace_id:     Null | String
    compression_kind:       Null | "none" | "gzip" | "zip"
    archive_member_path:    Null | String
    pre_strip_byte_length:  Null | u64            ; byte length of decoded
                                                  ; UTF-8 pre-strip bytes
    drop_reason:            Null
                          | "no_supported_plaintext_format"
                          | "no_plaintext_archive_member"
                          | "gutenberg_marker_missing"
                          | "source_decode_failed"
                          | "invalid_utf8"
                          | "ambiguous_plaintext_archive"
                          | "empty_after_strip"
                          | "unmappable_density_high"
                          | "dedup_collision"
    duplicate_of_book_id:   Null | u32            ; non-null iff
                                                  ; drop_reason =
                                                  ; "dedup_collision"
    post_strip_byte_length: Null | u64            ; null only if not computed
    post_strip_sha256:      Null | Hash256        ; null only if not computed;
                                                  ; sha256 over UTF-8 bytes of
                                                  ; the NFC-normalized stripped
                                                  ; body, before charset_v1
    post_charset_body_sha256:
                            Null | Hash256        ; null only if not computed;
                                                  ; sha256 over the post-strip,
                                                  ; post-charset_v1 body
                                                  ; token-id stream, excluding
                                                  ; <bos>/<eos>; the dedup key
    post_charset_token_length:
                            Null | u64            ; null only if not computed
    unmappable_count:       Null | u64            ; null only if not computed
    unmappable_density:     Null | f64            ; null only if not computed
    split:                  Null | "train" | "val"
                                                  ; null iff drop_reason set
  }

GutenbergManifest :=
  {
    schema:                          "gutenberg_manifest.v1"
    source_name:                     "Project Gutenberg"
    catalog_snapshot_url:            String
    catalog_snapshot_sha256:         Hash256
    catalog_snapshot_observed_at_utc:String                  ; informational
    catalog_snapshot_last_modified_utc:
                                      Null | String           ; informational
    selection_filter_canonical_json: String                  ; UTF-8 string
                                                              ; containing the
                                                              ; S1CanonicalJson
                                                              ; encoding of the
                                                              ; pinned D1 filter
    selection_filter_sha256:          Hash256
    book_ids:                        [u32]                   ; sorted ascending
    sources:                         [GutenbergSourceRecord]
                                                              ; one per book_id;
                                                              ; same order
    header_regex_pattern:            String
    footer_regex_pattern:            String
    normalization_spec_self_hash:    Hash256                  ; charset_v1
                                                              ; (S3 / F-G2)
    dedup_policy:
      {
        kind:                "exact_post_strip_charset_body_sha";
        notes:               "Two retained books with identical
                              post_charset_body_sha256 (i.e. identical body
                              token-id streams excluding <bos>/<eos>) are
                              treated as duplicates; only the lowest book_id
                              is retained. Raw source_blob_sha256 is reported but
                              is not the dedup key, because Gutenberg
                              boilerplate divergence (release notes, edition
                              metadata) can mask body-identical duplicates."
      }
    split_seed_u128:                 String                   ; hex 32 chars
    split_train_fraction:            0.90
    split_val_fraction:              0.10
    train_path:                      String                   ; on-disk path
    val_path:                        String
    train_sha256:                    Hash256                  ; post-strip,
                                                              ; post-charset_v1,
                                                              ; concatenated
                                                              ; train byte stream
    val_sha256:                      Hash256                  ; same, val
    train_byte_length:               u64
    val_byte_length:                 u64
    train_book_count:                u32
    val_book_count:                  u32
    drop_count_total:                u32
    drop_count_no_supported_plaintext_format: u32
    drop_count_no_plaintext_archive_member: u32
    drop_count_source_decode_failed: u32
    drop_count_ambiguous_plaintext_archive: u32
    drop_count_invalid_utf8:         u32
    drop_count_empty_after_strip:    u32
    drop_count_marker_missing:       u32
    drop_count_unmappable_density:   u32
    drop_count_dedup_collision:      u32
    unmappable_rate_corpus:          f64                      ; D5 aggregate
    raw_byte_policy:                 "post-strip, post-charset_v1 token-id
                                      stream, one octet per token id;
                                      <bos>/<eos> inserted at book boundaries
                                      (id 80 / 81); <unk> id 82."
    retained_book_count_min:         1350
    manifest_self_hash:              Hash256
  }

S4 canonical Gutenberg instance:
  catalog_snapshot_url:    "https://www.gutenberg.org/cache/epub/feeds/rdf-files.tar.bz2"
  catalog_snapshot_sha256: sha256:<PINNED_AT_FIXTURE_CREATION>     ; draft placeholder only
  catalog_snapshot_observed_at_utc:
                           <PINNED_AT_FIXTURE_CREATION_RFC3339_UTC>
                                                              ; draft placeholder only
  catalog_snapshot_last_modified_utc:
                           <PINNED_AT_FIXTURE_CREATION>      ; draft placeholder only
  selection_filter:        D1 verbatim
  book_count_target:       1500
  train_byte_length:       <PINNED_AT_FIXTURE_CREATION>       ; no estimate in execution RFC
  val_byte_length:         <PINNED_AT_FIXTURE_CREATION>       ; no estimate in execution RFC
  unmappable_rate_corpus:  <= 0.005     ; constraint, not a measurement
  fixture_pin:             fixtures/corpora/gutenberg.toml
                           records catalog, selected raw-file, mirror/cache,
                           and sha256 pins used to construct the manifest.

  manifest_path:           experiments/S4/corpus/gutenberg-manifest.json
                           is the emitted gutenberg_manifest.v1 canonical JSON
                           artifact consumed by S4 replay, promotion, scoring,
                           and report generation.

ContaminationDirection :=
    TS_train_contains_GB_val
  | GB_train_contains_TS_val
  | TS_train_contains_GB_train
  | GB_train_contains_TS_train
  | TS_val_overlaps_GB_val
  | GB_val_overlaps_TS_val

ContaminationFinding :=
  {
    direction:        ContaminationDirection
    overlap_fraction: f64
  }

ContaminationOutcome :=
    Clean
  | Warn(findings: [ContaminationFinding])
  | HardFail(failures: [ContaminationFinding],
             warnings: [ContaminationFinding])

PromotionGateOutcome :=
    Promoted(c_TS_checkpoint_sha: Hash256,
             gutenberg_manifest_sha: Hash256)
  | Rejected(reasons: [PromotionGateRejectionReason])

PromotionGateRejectionReason :=
    P1_v0success_missing
  | P1_checkpoint_self_hash_invalid
  | P1_v0success_self_hash_invalid
  | P1_v0success_checkpoint_mismatch
  | P1_v0success_manifest_mismatch
  | P1_checkpoint_not_phase_d_resumeable
  | P2_oracle_missing
  | P2_oracle_self_hash_invalid
  | P2_oracle_checkpoint_mismatch
  | P2_oracle_manifest_mismatch
  | P2_oracle_disagreement
  | P3_ternary_gap_too_large
  | P4_gutenberg_manifest_invalid
  | P5_contamination_dirty
  | P5_contamination_self_hash_invalid
  | P5_contamination_manifest_mismatch
  | P6_unmappable_rate_too_high
  | P7_baseline_missing
  | P7_baseline_self_hash_invalid
  | P7_baseline_manifest_mismatch
  | P7_baseline_nonfinite
  | P8_repetition_collapse
  | P8_repetition_self_hash_invalid
  | P8_repetition_checkpoint_mismatch

CorpusProgressionScheduleSnapshot :=
  {
    schedule_version: "s4.v1"
    edges:
      [
        { from: "TinyStories", to: "Gutenberg", gate: "G_TS->Gutenberg" }
      ]
    active_corpus_at_start:  "TinyStories"
    active_corpus_at_finish: "Gutenberg"
    progression_self_hash:   Hash256
  }

DomainHash, Self-hash rule, CanonicalTensorPayloadHash,
CanonicalCheckpointWrite, S1CanonicalJson, Prediction status rule:
  All inherited verbatim from F-S1 §1. The S4 schema_id strings differ
  ("s4_checkpoint", "gutenberg_manifest", etc.) but the domain prefix
  and hashing rules are identical.
```

bpc on Gutenberg val (S4 instance reusing the S1 chunked-reset primitive):

```text
For a model M and Gutenberg validation token-id sequence V containing N
charset_v1 token ids:

  Let chunk(i) = floor(i / 128) and start(i) = 128 * chunk(i).
  Let ctx(i)  = V[start(i) .. i], the prefix within the current chunk only.

  bpc(M, V) = (1 / N) * sum_{i=0}^{N-1} -log2(P_M(V[i] | ctx(i)))

P_M is computed by numerically stable log_softmax over the 83-id charset_v1
vocabulary.
Logits are produced from the model state before consuming V[i].
For ctx(i) = epsilon, logits are produced from the deterministic zero
initial state.

Required:
  - log2_sum is accumulated in f64; final division by N happens once.
  - N equals token_length(V_gutenberg_val) exactly. No padding.
  - V_gutenberg_val is the manifest token-id stream, including <bos> and
    <eos> boundary ids inserted by G-Ok-5.
  - V is consumed in non-overlapping chunks of length 128; the final chunk
    may be shorter. State resets to zero at each chunk boundary.
  - The first id of each chunk is scored from empty context.
  - This is the chunked-reset bpc primitive, identical in shape to S1's
    raw-byte bpc and to S3's charset_v1 bpc; only the vocabulary and the
    token-id stream differ.

Cross-corpus baseline parity:
  Both bpc_ternary(c, V_gutenberg_val) and bpc_kn5(V_gutenberg_train,
  V_gutenberg_val) use this exact reset-context primitive, so the
  promotion-gate margin comparison is consistent.
```

COr Corpus integrity oracles (mandatory, model-free):

```text
COr-1 manifest round-trip:
  Round-trip the canonical gutenberg_manifest.v1 JSON through
  S1CanonicalJson encode/decode/encode and assert byte equality plus
  manifest_self_hash equality.

COr-2 header/footer stripper idempotence:
  For 10 hand-picked Gutenberg ebook ids fixed in
  fixtures/corpora/gutenberg-stripper-fixture.toml, applying the D3 regex
  pair to a known-good plaintext input must produce a known-good output
  whose sha256 matches the fixture pin. Applying the stripper a second
  time under `StripMode::AlreadyStrippedOk` must return the input
  byte-for-byte. Applying the stripper a second time under manifest-build
  mode must return `gutenberg_marker_missing`.

COr-3 charset_v1 round-trip on a Gutenberg sample:
  For a 64 KiB prefix of `gutenberg_train`, decoding token ids back to
  charset_v1 characters and re-encoding must produce a byte-identical id
  stream. <bos>/<eos> insertion points must round-trip.

COr-4 split determinism:
  Re-running the D2 book-level split with the pinned split_seed_bytes
  against the same retained, non-duplicate book ids must produce a
  byte-identical split map:

    book_id -> train | val

  Re-running the full manifest build with the same fixture pins must then
  produce byte-identical train_path and val_path streams.

COr-5 unmappable accounting:
  unmappable_rate_corpus reported in the manifest must equal the value
  recomputed by an independent slow reference walker over the post-strip
  corpus to within 1 ULP in f64.

COr-6 contamination overlap math:
  For a fixture pair (synthetic 13-gram set A, synthetic 13-gram set B)
  with hand-counted intersection size k and sample sizes n_A, n_B, the
  contamination overlap math must report the exact IEEE-754 f64 result of
  (k as f64) / (n_A as f64) and (k as f64) / (n_B as f64), with no early
  rounding.
  The fixture must additionally assert that
  `fingerprint_kind = "sha256_high_u64"` is used exactly as the index:
  interpret the first eight bytes of the SHA-256 digest as a big-endian
  u64. An implementation using low_u64, little-endian high_u64, modulo
  reduction, or a non-SHA fingerprint must fail this fixture.

  The fixture must also include a synthetic forced-index-collision case:
  two distinct 13-token windows are assigned the same test double index.
  The implementation must not count them as overlapping unless the exact
  13-token byte windows are equal. This proves that the high_u64 value is
  an index, not the semantic equality relation.
```

---

# 2. Authority rules

```text
Scope(F-S4) =
  {
    H1, H2, H3, H4, H5, H6, H7,
    gutenberg_manifest.v1 schema and S4 canonical instance,
    Gutenberg loader, header/footer stripper, document-level split rule,
    s4_corpus_quality.v1, s4_contamination_report.v1,
    promotion gate G_TS->Gutenberg semantics,
    s4_promotion_gate.v1,
    CorpusProgressionSchedule (S4 snapshot),
    s4_corpus_progression.v1,
    s4_gutenberg_run_log.v1, s4_gutenberg_checkpoint.v1,
    s4_gutenberg_score.v1, s4_baseline_gutenberg.v1,
    s4_fp_reference.v1,
    s4_oracle_agreement.v1 (Gutenberg side),
    s4_report.v1
  }

Rule Authority:
  for all behavior b in Scope(F-S4), if this RFC specifies b, then
  SourceOfTruth(b) = this RFC.

Rule InheritedFromS1:
  Bpc primitive (chunked-reset, chunk_size = 128, f64 log2_sum, finite
  division at the end). Pcg64Mcg, seed128, uniform_u64_inclusive
  rejection sampling, Fisher-Yates over indices. S1CpuDeterministic
  device profile (env_exact, env_forbidden_unless_listed). DomainHash,
  CanonicalTensorPayloadHash, CanonicalCheckpointWrite, S1CanonicalJson.
  S4 may not amend any of these. If it needs to, the amendment must be
  explicit per Rule Amendment.

Rule InheritedFromS2:
  Phase scheduler A->B->C->D, QuantHardness, ternary projection
  semantics, Burn LinearState gradient flow contract, structured
  logging adoption. S4 consumes Phase::D unchanged for the
  Gutenberg continuation run.

Rule InheritedFromS3:
  charset_v1 normalization (F-G2), 5-gram modified Kneser-Ney baseline
  math, ReferenceModelBundle export, DenotationalOracle contract,
  ArtifactOracle contract, three-way oracle agreement tolerances,
  v0_success workload manifest definition, ConformanceEnvelope, S3
  ternary checkpoint format. S4 instantiates these against gutenberg_val
  rather than tinystories_val; it does not redefine them.

Rule PlanContext:
  Behavior outside Scope informed by planv0 amendments and bd-2hmm
  comments. The 2026-05-06 sizing-realism amendment (Toy0 / Toy1 /
  MoeTiny / UpperBankCandidate registry, dense-baseline-first rule,
  matched-deployed-bytes parity) governs S4's model choice (Toy0,
  same as S1..S3) and forbids any silent jump to a larger profile
  during corpus progression.

Rule CrateOwnership:
  Every behavior in Scope(F-S4) is implemented in exactly one of:
    - gbf-experiments       (S4 module: s4_* operations, COr-* oracle
                              suite, schema encoders, replay CLI
                              entrypoints, falsification suite)
    - gbf-policy            (Toy0 ModelSizeProfile reference instance)
    - gbf-model             (LinearStateBlock with Fixed(0.5);
                              ternary projection consumed unchanged)
    - gbf-train             (Phase scheduler at Phase::D, AdamW config,
                              CorpusProgressionSchedule, `qat`
                              and `qat-ablation` features inherited from
                              S1/S2)
    - gbf-data              (GutenbergManifest reader, Gutenberg loader,
                              header/footer stripper, document split,
                              charset_v1 plumbing inherited from S3,
                              cross-corpus contamination check)
    - gbf-foundation        (Hash256, sha256 helper)
    - gbf-artifact          (CanonicalTensor, CanonicalTensorPayloadHash;
                              ReferenceModelBundle re-export consumed
                              unchanged)
    - gbf-oracle            (DenotationalOracle, ArtifactOracle consumed
                              unchanged from S3)
    - gbf-cli               (`gbf s4` subcommand for replay)
  No S4-specific code lives outside this set.

Rule Amendment:
  Later slice changing any of:
    Toy0 dim caps
    bpc primitive
    KN-5 baseline math
    seed list
    Gutenberg train budget (D10)
    promotion gate criterion (D8)
    contamination thresholds (D6)
    unmappable bound (D5)
  must explicitly amend this RFC. S5..S8 may not silently change S4's
  numbers.

Rule Falsification:
  This RFC is correct only if a deliberately-broken implementation
  produces the expected Refuted verdict on the appropriate hypothesis.
  Falsification sensitivity is a first-class proof obligation (§15 O5).
```

---

# 3. Hypothesis algebra

Every hypothesis carries a statement, predicted observables, falsification
rule, verdict mapping, and downstream consequence. H1, H2, H3, H4, H5, H6
are **mandatory closure gates**. H7 is **optional / observational**: it
exists to bound the "did we actually transfer learning vs memorize TS" risk
but does not by itself block bd-2hmm closure.

## H1 Corpus integrity

```text
Statement:
  gutenberg_manifest.v1 + source-format selection + archive extraction +
  header/footer stripping + dedup + charset_v1 normalization + document-
  level split round-trip deterministically: replaying the manifest build
  on the same catalog_snapshot_sha256 + same book_ids + same source-blob
  sha256 pins + same regex pair + same normalization_spec_self_hash +
  same split_seed_u128 produces byte-identical train_path / val_path /
  train_sha256 / val_sha256 / unmappable_rate_corpus.

Predicted:
  unmappable_rate_corpus_gutenberg     in [0.0001, 0.005]   ; sanity range
  drop_count_marker_missing            <= 0.05 * len(book_ids)
  drop_count_unmappable_density        <= 0.02 * len(book_ids)
  train_book_count + val_book_count + drop_count_total
                                       == len(book_ids), where
                                       drop_count_total is the sum of every
                                       enumerated drop_count_* field in
                                       gutenberg_manifest.v1
  train_book_count + val_book_count    >= 1350
  manifest_self_hash round-trips through canonical JSON

Falsification:
  any of COr-1..COr-5 fails                                    => Refuted
  unmappable_rate_corpus_gutenberg > 0.005                     => Refuted
  drop_count_marker_missing > 0.05 * len(book_ids)             => Refuted
  drop_count_unmappable_density > 0.02 * len(book_ids)         => Refuted
  train_book_count + val_book_count < 1350                     => Refuted
  manifest_self_hash recomputation differs from recorded value => Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Gutenberg corpus is structurally broken. Halt. Every later S4 hypothesis
  is unreliable until corpus integrity is restored.
```

## H2 Cross-corpus contamination clean

```text
Statement:
  Exact cross-corpus 13-token overlap between TinyStories train and
  Gutenberg val, and between Gutenberg train and TinyStories val, is below
  the pinned hard-fail threshold in both gated directions.

Predicted:
  overlap(TS_train, GB_val)          in [0.0, 0.0005]
  overlap(GB_train, TS_val)          in [0.0, 0.0005]
  overlap(TS_train, GB_train)        in [0.0, 0.0010]   ; reported only
  overlap(TS_val,   GB_val)          in [0.0, 0.0010]   ; reported only

Falsification:
  overlap(TS_train, GB_val)   > 0.0010                            => Refuted
  overlap(GB_train, TS_val)   > 0.0010                            => Refuted
  COr-6 fails on synthetic fixture                                => Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted:
  Either at least one gated split contains text from the other corpus, or
  the contamination measurement oracle is invalid. Halt. v0_success on
  Gutenberg is not interpretable while contamination is dirty or
  unmeasurable.

  If overlap is in [0.0005, 0.0010] in either gated direction:
  ContaminationOutcome = Warn; H2 is still Confirmed but s4_report.v1
  records the warning explicitly and Decision becomes
  ProceedToS5-with-contamination-warning.
```

## H3 Promotion gate implementation soundness

```text
Statement:
  The promotion gate G_TS->Gutenberg implements D8 exactly over parseable
  promotion bundles: it accepts an input bundle iff D8 P-1..P-8 all hold,
  and rejects otherwise with a complete enumeration of failed evaluable
  reasons. Predicates depending on an artifact with an invalid self-hash
  are not evaluable from that artifact's semantic fields.
  "Implementation soundness" means the implementation faithfully encodes
  D8 over the tested input space; it is distinct from the question of
  whether the canonical c_TS_ref happens to satisfy P-1..P-8 (that is a
  readiness question, not a soundness question).

Predicted:
  promotion_gate(c_TS_canonical)   = Promoted(...) if P-1..P-8 hold.
  If c_TS_canonical is rejected for a true failed precondition, H3 remains
  Confirmed and S4Outcome becomes Fail-promotion-gate-readiness.
  for every k in {P1, P2, P3, P4, P5, P6, P7, P8}:
    promotion_gate(c_TS_with_Pk_broken) = Rejected(reasons including Pk)

  promotion_gate_self_hash round-trips.
  promotion_gate is referentially transparent: same inputs => same
  PromotionGateOutcome bytes (under S1CanonicalJson encoding).

Falsification:
  promotion_gate rejects a reference-positive bundle for which the
    independent evaluator proves P-1..P-8 all hold                 => Refuted
  exists k. promotion_gate(c_TS_with_Pk_broken) = Promoted(_)   => Refuted
  promotion_gate accepts a bundle for which D8 P-1..P-8 are not
    all simultaneously satisfied                                => Refuted
  promotion_gate evaluation depends on host clock or env state  => Refuted
  promotion_gate_self_hash recomputation differs                => Refuted

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise. A genuine rejection of c_TS_canonical for a true
  failed precondition keeps H3 Confirmed; the S4 outcome is then
  Fail-promotion-gate-readiness rather than Fail-promotion-gate.

Consequence of Refuted:
  Corpus progression has no honest gate. Every later production-scale
  run on Gutenberg (S8 UpperBankCandidate, including the
  gutenberg_manifest.v2 test-partition amendment) inherits this risk.
  Halt.
```

## H4 Cross-corpus generalization

```text
Statement:
  For every seed s in {0, 1, 2, 3, 4}, the Gutenberg-trained ternary
  checkpoint c_GB(s) passes v0_success on gutenberg_val and beats the
  Gutenberg KN-5 baseline by a strictly positive pinned margin.

Predicted:
  bpc_kn5_gutenberg_val          in [1.4, 2.2]                ; sanity range
                                                              ; [ESTIMATE]
  median(bpc_ternary_gutenberg)  in [1.1, 1.9]                ; sanity range
                                                              ; [ESTIMATE]
  for all s. bpc_ternary(c_GB(s), gutenberg_val)
              < bpc_kn5_gutenberg_val - 0.05                  ; the actual gate
  for all s. v0_success_workload(c_GB(s), gutenberg_val) = Pass

Falsification:
  exists s. bpc_ternary(c_GB(s), gutenberg_val)
              >= bpc_kn5_gutenberg_val - 0.05                 => Refuted
  exists s. v0_success_workload(c_GB(s), gutenberg_val) = Fail
                                                                => Refuted
  median(bpc_ternary_gutenberg) < 0.5                          => Refuted
                                                              ; suspicious-low

Verdict:
  Refuted if any falsification hits.
  Confirmed otherwise.

Consequence of Refuted (non-suspicious):
  Toy0 + the S3 ternary continuation contract does not transfer to
  Gutenberg-quality English under the pinned step budget. Open follow-up
  beads: (a) propose increased optimizer_steps_gutenberg, (b) re-evaluate
  whether D9's warm-weight cold-optimizer rule is the right choice, (c)
  consider whether Toy1 is required for non-toy English. Do not silently
  bump Toy0 caps.

Consequence when median(bpc_ternary_gutenberg) < 0.5:
  Halt. Audit gutenberg train/val split for leakage, audit bpc
  accumulator, audit corpus loader. Do not proceed to any later slice.
```

## H5 Three-way oracle agreement preserved across corpora

```text
Statement:
  For seed 0, the live training output, the ReferenceModelBundle re-export
  from c_GB(0), and the ArtifactOracle re-execution agree on the canonical
  S3-pinned conformance fixture set evaluated against gutenberg_val,
  within S3-pinned tolerance.

Non-claim:
  This is not a new oracle contract. S4 inherits the three-way agreement
  contract from S3 verbatim. H5 asserts only that switching the corpus
  does not break that contract.

Predicted:
  oracle_agreement(c_GB(0), gutenberg_val_fixture).status = Agree
  per-token bpc gap |bpc_live - bpc_denot|       <= S3_tol_denotational
  per-token bpc gap |bpc_live - bpc_artifact|    <= S3_tol_artifact
  per-token bpc gap |bpc_denot - bpc_artifact|   <= S3_tol_inter_oracle

Falsification:
  oracle_agreement(c_GB(0), gutenberg_val_fixture).status != Agree
                                                                => Refuted
  any per-token gap exceeds the S3 tolerance                    => Refuted

Verdict:
  Confirmed if all three pairwise agreements hold within tolerance.
  Refuted otherwise.

Consequence of Refuted:
  Corpus switching breaks oracle equivalence. Either the Gutenberg loader
  silently changed token semantics, or the ternary projection lost some
  invariant under continuation training, or one of the oracles has a
  data-distribution-dependent bug. Halt all later slices until the gap
  is localized.
```

## H6 Determinism across corpus switching

```text
Statement:
  Replaying the full S4 pipeline (manifest build, fit baselines,
  contamination report, promotion gate, Gutenberg continuation training,
  scoring, oracle agreement) under the same hashes from D16 produces
  bit-identical Gutenberg checkpoints for every seed and bit-identical
  s4_*.v1 self-hashes for every artifact.

Predicted:
  for all s. canonical_tensor_payload_sha(c_GB(s), replay)
              == canonical_tensor_payload_sha(c_GB(s), original)
  for all artifacts a in {gutenberg_manifest, corpus_quality,
                          contamination_report,
                          corpus_progression,
                          promotion_gate, baseline_gutenberg,
                          fp_reference[*],
                          gutenberg_run_log[*], gutenberg_checkpoint[*],
                          gutenberg_score[*], oracle_agreement,
                          report}.
    a.self_hash(replay) == a.self_hash(original)

Falsification:
  exists s. canonical_tensor_payload_sha(c_GB(s), replay)
             != canonical_tensor_payload_sha(c_GB(s), original)
                                                                => Refuted
  exists a. a.self_hash(replay) != a.self_hash(original)        => Refuted

Verdict:
  Confirmed iff every seed in {0,1,2,3,4} replays to bit-identical
  canonical-tensor payloads and every required artifact's self-hash
  round-trips.
  Refuted otherwise.

Consequence of Refuted:
  Some hidden input (host clock, env var, network, RNG leakage between
  TS and GB streams, non-deterministic reduction) entered the pipeline.
  Halt. Reproducibility is foundational; without it, every later quality
  number is meaningless.
```

## H7 Distribution-shift sanity (optional / observational)

```text
Statement:
  The Gutenberg-trained checkpoint c_GB(s) scores meaningfully better
  than the TinyStories-only checkpoint c_TS on gutenberg_val:

    bpc_ternary(c_TS, gutenberg_val)
      - bpc_ternary(c_GB(s), gutenberg_val)
        > 0.10                                  ; [ESTIMATE for review]

  for at least 4 of the 5 seeds.

Non-claim:
  This is not a closure gate. A Refuted H7 means continuation training
  did not transfer learning enough to be visibly distinct from the
  TS-only baseline; it does NOT mean v0_success failed (H4 owns that).

Predicted:
  for at least 4 seeds.
    bpc_ternary(c_TS, gutenberg_val) - bpc_ternary(c_GB(s), gutenberg_val)
      > 0.10

Falsification:
  for at most 3 seeds.
    bpc_ternary(c_TS, gutenberg_val) - bpc_ternary(c_GB(s), gutenberg_val)
      > 0.10                                                    => Refuted

Verdict:
  Refuted if the prediction fails on more than one seed.
  Confirmed otherwise.

Consequence of Refuted:
  Note in the surprises section. Open a follow-up bead asking whether
  c_TS already generalizes to Gutenberg (in which case continuation
  training provides little marginal signal and S5+ should reconsider
  schedule) or whether c_GB barely moved away from c_TS (in which case
  D10 step budget is too small for visible transfer). Closure of bd-2hmm
  is unaffected.
```

Hypothesis composition rules are formalized in §11 (Outcome algebra).

---

# 4. Experiment state machine

```text
State :=
    Configured(ts_manifest, gb_manifest_pinned, model_config, train_config,
               c_TS_ref)
  | CorpusBuilt(state, gb_manifest_built)
  | CorpusQualityReady(state, gb_manifest_built, corpus_quality)
  | CorpusReady(state, gb_manifest_built, corpus_quality, gb_baseline)
  | ContaminationChecked(state, contamination_report)
  | Promoted(state, promotion_gate_outcome)
  | TrainAttempted(state, gb_run_products[5])
  | TrainedOnGutenberg(state, gb_completed_run_products[5])
  | ScoredOnGutenberg(state, val_bpc_gb[5], v0_success_results[5])
  | OracleAgreement(state, agreement_seed_0)
  | Reported(state, report)
  | Decided(state, decision: ProceedToS5
                          | ProceedToS5-with-contamination-warning
                          | Investigate(reason)
                          | Halt(reason))
```

Transitions:

```text
T0 configure:
  empty -> Configured(c)

T1 build_corpus:
  Configured(c) -> CorpusBuilt(c, build_gutenberg_manifest(c))

  Post:
    COr-1..COr-5 all pass; otherwise transition to
    Reported(state, build_corpus_integrity_failure_report(state)).

T1q corpus_quality:
  CorpusBuilt(c, gb) -> CorpusQualityReady(c, gb,
                         emit_s4_corpus_quality(ts_manifest, gb))

T1b fit_baseline:
  CorpusQualityReady(c, gb, q) -> CorpusReady(c, gb, q,
                                  fit_kn5_gutenberg(c, gb))

  Failure:
    baseline missing, non-finite, counts blob invalid, or baseline self-hash
    mismatch => Reported(state, build_fail_substrate_report(state)).

T1c contamination_math_oracle:
  CorpusReady(c, gb, q, baseline) validates COr-6 before T2.
  COr-6 failure Refutes H2 and transitions to
  Reported(state, build_contamination_math_failure_report(state)).

T2 contamination:
  CorpusReady(c, gb, _) -> ContaminationChecked(c, gb,
                            cross_corpus_contamination_report(ts_val,
                                                              gb_train,
                                                              gb_val,
                                                              ts_train))

T3 promote:
  ContaminationChecked(...) ->
    Promoted(state, promote(c_TS_ref,
                            c_TS_v0success,
                            c_TS_oracle_agreement,
                            gb_manifest,
                            contamination_report,
                            baseline_gutenberg,
                            repetition_collapse_check))

  Note:
    T3 still runs when contamination_report.outcome = HardFail(_). In that
    case the promotion gate must reject with P5_contamination_dirty. The
    final S4Outcome remains Fail-contamination by §11 priority ordering.
    This preserves a self-hashed promotion-gate artifact while ensuring
    contamination failure remains the controlling scientific result.

T3a rejection short-circuit:
  Promoted(state, Rejected(reasons))
    -> Reported(state, build_promotion_rejection_report(state, reasons))

T4 train_gutenberg:
  Promoted(state, Promoted(...))
    -> TrainAttempted(state, [s4_gutenberg_train_run(c, s, c_TS_ref)
                              for s in seeds])

T4a all completed:
  TrainAttempted(state, runs) and for all r in runs. r.completion = Completed
    -> TrainedOnGutenberg(state, runs)

T4b divergence short-circuit:
  TrainAttempted(state, runs) and exists r in runs.
    r.completion = DivergedAt(_)
      -> Reported(state, build_fail_substrate_report(state))

T5 score:
  TrainedOnGutenberg(state, runs) ->
    ScoredOnGutenberg(state,
                      [s4_score_bpc(runs[s], gutenberg_val) for s in seeds],
                      [v0_success_workload(runs[s], gutenberg_val)
                                                          for s in seeds])

T6 oracle (seed 0 mandatory):
  ScoredOnGutenberg(...)
    -> OracleAgreement(state, three_way_oracle_agreement(runs[0],
                              gutenberg_val_fixture))

T7 report:
  OracleAgreement(...) -> Reported(state, build_report(state))

T8 decide:
  Reported(state, r) -> Decided(state, decide(r))
```

Invariants:

```text
I-S4-1
  T1 must not run without a verified TinyStories manifest already pinned
  by S3 and the c_TS_ref checkpoint already reachable on disk.

I-S4-2
  T2 must consume both ts_train, ts_val, gb_train, gb_val byte streams
  pinned by their manifest sha256s. No on-the-fly resampling.

I-S4-3
  T3 must record promotion_gate_self_hash before T4 begins. Every
  s4_gutenberg_*.v1 artifact records this hash in its metadata; replay
  validates it before consuming the artifact.

I-S4-4
  T4 must not run with PromotionGateOutcome != Promoted.

I-S4-5
  T6 must run on the *first* completed seed in the configured seed list,
  which is seed 0 by D11. Seed 0 ablation cannot be skipped.

I-S4-6
  T7 emits exactly one s4_report.v1 per S4 PR. Re-runs after RFC
  amendment produce a new report with bumped rfc_revision.

I-S4-7
  Decided is final: closure of bd-2hmm is gated on
  Decision in {ProceedToS5, ProceedToS5-with-contamination-warning}.

I-S4-8
  Every artifact under experiments/S4/ depends only on inputs listed in
  §13 Rep-3. No host-clock value other than s4_report.v1.generated_at
  appears in any S4 artifact.
```

---

# 5. Corpus contract

```text
operation s4_build_gutenberg_manifest
  input:  GutenbergBuildInputs
  output: GutenbergManifest

GutenbergBuildInputs :=
  {
    catalog_snapshot_path:       String     ; on-disk path to fetched RDF
    catalog_snapshot_sha256:     Hash256
    selection_filter:            CanonicalJson  ; D1 verbatim
    fixtures_path:               String     ; fixtures/corpora/gutenberg.toml
    charset_v1_spec_self_hash:   Hash256    ; from S3 / F-G2
    split_seed_u128:             u128       ; D2 verbatim
  }

Preconditions:
  G-Pre-1  catalog_snapshot file sha256 matches fixture pin (D1).
  G-Pre-2  selection_filter equals D1 byte-for-byte under S1CanonicalJson.
  G-Pre-3  charset_v1_spec_self_hash equals the value pinned by S3.
  G-Pre-4  every book id in `book_ids` resolves on the content-addressed
           fixture either:
             - has a selected source blob whose sha256 is present in the
               content-addressed mirror/cache, or
             - is recorded with drop_reason =
               `no_supported_plaintext_format`.

           A selected source blob missing from the mirror/cache is a
           manifest-construction failure.

Postconditions:
  G-Ok-1   `book_ids` is sorted ascending, deduplicated, and equals the
           deterministic selected subset produced by D1:

             first 1500 ids by rank among ids satisfying the catalog filter,
             then sorted ascending.
  G-Ok-2   for every book id, header_regex and footer_marker_regex are
           applied per D3; books with marker_missing are dropped, recorded.
  G-Ok-3   for every retained book, charset_v1 normalization is applied
           per inherited F-G2 / S3 spec; per-book unmappable_density is
           computed and books exceeding 2% are dropped, recorded.
  G-Ok-3a  exact duplicate removal is applied after charset_v1
           normalization and before split assignment. For each
           post_charset_body_sha256 collision, retain the lowest book_id
           and drop all other books in the collision class with
           drop_reason = "dedup_collision" and duplicate_of_book_id set
           to the retained id.
  G-Ok-4   the document-level split (D2) assigns each retained book to
           exactly one of train | val.
  G-Ok-5   <bos> (id 80) and <eos> (id 81) are inserted at book
           boundaries in both train_path and val_path streams in the
           sorted-ascending order of retained book ids per split.
           The exact stream for each retained book is:

             [<bos>] || body_token_ids || [<eos>]

           Book streams are concatenated with no additional separator.
  G-Ok-6   train_sha256 and val_sha256 are sha256 over the
           post-strip post-charset_v1 token-id stream serialized as one
           octet per token id. Endianness is not applicable to u8.
  G-Ok-7   unmappable_rate_corpus aggregates over the retained corpus
           (train + val) and is reported in the manifest. Pre/post-drop
           accounting is consistent:

             drop_count_total ==
               drop_count_no_supported_plaintext_format
             + drop_count_no_plaintext_archive_member
             + drop_count_source_decode_failed
             + drop_count_ambiguous_plaintext_archive
             + drop_count_invalid_utf8
             + drop_count_empty_after_strip
             + drop_count_marker_missing
             + drop_count_unmappable_density
             + drop_count_dedup_collision.
  G-Ok-8   manifest_self_hash is the DomainHash of the canonical-JSON
           encoding of the manifest with manifest_self_hash omitted.
  G-Ok-9   train_byte_length >= sequence_length.
  G-Ok-10  val_byte_length >= sequence_length.
  G-Ok-11  train_book_count > 0 and val_book_count > 0.
  G-Ok-11a train_book_count + val_book_count >=
           retained_book_count_min.

  G-Ok-12  For a record with
           drop_reason = "no_supported_plaintext_format":

             mirror_fetch_url = null
             mirror_snapshot_id = null
             selected_format = null
             source_blob_sha256 = null
             pre_strip_utf8_sha256 = null
             pre_strip_byte_length = null

           For drop_reason = "no_plaintext_archive_member":

             source_blob_sha256 is non-null,
             compression_kind = "zip",
             archive_member_path = null,
             pre_strip_utf8_sha256 = null,
             pre_strip_byte_length = null.

           For drop_reason = "ambiguous_plaintext_archive":

             source_blob_sha256 is non-null,
             compression_kind = "zip",
             archive_member_path = null,
             pre_strip_utf8_sha256 = null,
             pre_strip_byte_length = null.

           For drop_reason = "source_decode_failed":

             source_blob_sha256 is non-null,
             selected_format is non-null,
             pre_strip_utf8_sha256 = null,
             pre_strip_byte_length = null.

           For drop_reason in {
             "gutenberg_marker_missing",
             "invalid_utf8",
             "empty_after_strip",
             "unmappable_density_high",
             "dedup_collision"
           }:

             source_blob_sha256 is non-null,
             selected_format is non-null,
             pre_strip_utf8_sha256 is non-null,
             pre_strip_byte_length is non-null.

  G-Fail-1
    unmappable_rate_corpus > 0.005
    => the manifest is *not* written; the build CLI exits non-zero.
       Higher rates indicate a charset bug, not a real text property.

  G-Fail-2
    drop_count_marker_missing * 20 > len(book_ids)
    => the manifest is *not* written. The catalog snapshot or fetch
       mirror is corrupted.

  G-Fail-3
    train_book_count + val_book_count < retained_book_count_min
    => the manifest is *not* written. The retained corpus is too small
       for the S4 canonical instance.

Failure mode:
  KN-5 baseline math is intended to be total over valid non-empty
  token streams. A non-finite bpc_kn5, missing counts blob, or
  self-hash mismatch is a substrate failure and causes
  S4Outcome = Fail-substrate unless H1 has already been Refuted.
```

---

# 6. Baseline contract (Gutenberg side)

## 6.1 Inheritance

The KN-5 baseline math, smoothing constants, and counts-extraction
semantics are inherited verbatim from S3 (bd-3k8o). S4 does not
re-derive them. The S4 contribution is the *Gutenberg-side instance*
of the baseline.

## 6.2 Operation

```text
operation s4_fit_kn5_gutenberg
  input:  KnBaselineInputs
  output: KnBaselineProduct

KnBaselineInputs :=
  {
    corpus_train: ByteSeq        ; sha256-pinned, gutenberg_train
    corpus_val:   ByteSeq        ; sha256-pinned, gutenberg_val
    kn_params:    KneserNeyParams ; from S3
  }

KnBaselineProduct :=
  {
    schema:                        "s4_baseline_gutenberg.v1"
    tinystories_manifest_self_hash: Hash256
    gutenberg_manifest_self_hash:   Hash256
    corpus_train_sha:              Hash256
    corpus_val_sha:                Hash256
    kn_params:                     KneserNeyParams
    bpc_kn5:                     BpcValue
    bpc_kn3:                     BpcValue              ; reported
    bpc_unigram:                 BpcValue              ; reported
    counts_summary:              CountsSummary
    counts_blob_sha256:          Hash256
    baseline_gutenberg_self_hash: Hash256
  }

Preconditions:
  KB-Pre-1  corpus_train and corpus_val sha256s match
            gutenberg_manifest.v1 fields.
  KB-Pre-2  kn_params equals the S3-pinned values byte-for-byte.

Postconditions:
  KB-Ok-1   bpc_kn5 is finite, >= 0, computed under chunked-reset bpc.
  KB-Ok-2   bpc_kn5, bpc_kn3, and bpc_unigram are finite and >= 0.

            Reported sanity check, not a postcondition:
              bpc_kn5 <= bpc_kn3 <= bpc_unigram.
  KB-Ok-3   counts_summary is reproducible: same train sha256 +
            same kn_params -> same counts.
  KB-Ok-4   baseline_gutenberg_self_hash is canonical hash of bpc_*,
            counts_summary, counts_blob_sha256, kn_params.
```

---

# 7. Contamination contract

```text
operation s4_cross_corpus_contamination
  input:  CrossCorpusInputs
  output: CrossCorpusReport

CrossCorpusInputs :=
  {
    ts_manifest:  TinyStoriesManifest    ; from S3
    gb_manifest:  GutenbergManifest      ; from §5
    n:            13                     ; D6
    fingerprint_kind: "sha256_high_u64"  ; D6
    gated_overlap_policy:
      "full_val_against_full_opposite_train"
    diagnostic_sample_cap_token_ids_per_split:
      1_048_576                          ; D6
  }

CrossCorpusReport :=
  {
    schema:                       "s4_contamination_report.v1"
    tinystories_manifest_self_hash: Hash256
    gutenberg_manifest_self_hash: Hash256
    ts_train_sha:                 Hash256
    ts_val_sha:                   Hash256
    gb_train_sha:                 Hash256
    gb_val_sha:                   Hash256
    n:                            13
    fingerprint_kind:             "sha256_high_u64"
    collision_disambiguation:     "exact_13_token_bytes_on_hit"
    fingerprint_count_ts_val_ngrams: u64 ; cardinality of unique full or
                                          ; sampled 13-gram window set
    fingerprint_count_gb_val_ngrams:    u64
    fingerprint_count_ts_train_ngrams:  u64
    fingerprint_count_gb_train_ngrams:  u64
    overlap_ts_train_to_gb_val:   f64    ; exact full-window overlap:
                                          ; |TS_train ∩ GB_val|
                                          ;   / fingerprint_count_gb_val_ngrams
                                          ; (TS_train_contains_GB_val)
    overlap_gb_train_to_ts_val:   f64    ; exact full-window overlap:
                                          ; |GB_train ∩ TS_val|
                                          ;   / fingerprint_count_ts_val_ngrams
                                          ; (GB_train_contains_TS_val)
    overlap_ts_train_contains_gb_train: f64
                                         ; |TS_train ∩ GB_train|
                                         ;   / fingerprint_count_gb_train_ngrams
                                         ; reported, not gated
    overlap_gb_train_contains_ts_train: f64
                                         ; |GB_train ∩ TS_train|
                                         ;   / fingerprint_count_ts_train_ngrams
                                         ; reported, not gated
    overlap_ts_val_to_gb_val:     f64    ; |TS_val ∩ GB_val|
                                          ;   / fingerprint_count_gb_val_ngrams
                                          ; reported, not gated
    overlap_gb_val_to_ts_val:     f64    ; |GB_val ∩ TS_val|
                                          ;   / fingerprint_count_ts_val_ngrams
                                          ; reported, not gated
    denominator_policy:
      {
        gated_directions:     "full validation split against full opposite train split";
        diagnostic_directions:"deterministic per-document stratified sample, if cap applies";
      }
    warnings:                     [ContaminationFinding]
    hard_failures:                [ContaminationFinding]
    outcome:                      ContaminationOutcome
    contamination_self_hash:      Hash256
  }

Preconditions:
  X-Pre-1  ts_manifest and gb_manifest sha256 fields validate against
           on-disk archives.
  X-Pre-2  charset_v1 normalization has already been applied to both
           sides (D4).
  X-Pre-3  n = 13 exactly. Overriding n is a new experiment.
  X-Pre-4  every gated split has at least n token ids after
           document-boundary handling; otherwise contamination is
           undefined and H2 is Refuted as corpus-integrity-adjacent.
           Diagnostic-only sampled directions with zero sampled 13-grams
           are reported as diagnostic_not_available and do not affect H2.

Postconditions:
  X-Ok-1   overlap_ts_train_to_gb_val =
              count(grams in ts_train_set INTERSECT gb_val_set) /
              fingerprint_count_gb_val_ngrams
           computed as f64 with no early rounding.
  X-Ok-2   overlap_gb_train_to_ts_val computed symmetrically.
  X-Ok-3   Gated directions are:

              TS_train_contains_GB_val
              GB_train_contains_TS_val

           Diagnostic-only directions are:

              TS_train_contains_GB_train
              GB_train_contains_TS_train
              TS_val_overlaps_GB_val
              GB_val_overlaps_TS_val

           For each gated direction:

             overlap_fraction > 0.0010  => hard_failures includes finding
             overlap_fraction >= 0.0005 => warnings includes finding

           outcome =
             HardFail(hard_failures, warnings) if hard_failures is non-empty
             Warn(warnings)                    if hard_failures is empty
                                                and warnings is non-empty
             Clean                             otherwise.
  X-Ok-4   contamination_self_hash round-trips per Self-hash rule.

  X-Fail-1
    COr-6 fails => contamination math is broken; the report
    is *not* written; the CLI exits non-zero. (The fingerprint hash
    must be the high-order u64 of the sha256, not modulo or low-order
    bits, which can produce false-collision counts.)

Diagnostic sampling rule:
  Diagnostic-only directions are sampled by deterministic per-document
  stratification so contamination signal is not concentrated in the earliest
  book ids:

    per_doc_cap =
      ceil(diagnostic_sample_cap_token_ids_per_split
           / retained_doc_count(split))

    include deterministic head/middle/tail fragments totaling at most
    per_doc_cap token ids from every retained document in that split, in
    ascending book-id order.

  The 13-gram set is the unique set of length-13 contiguous windows
  within each sampled document fragment. 13-grams do not cross fragment
  or document boundaries. The cap exists only for diagnostic directions.
  Closure-gated directions use the full validation split and the full
  opposite training split.
```

---

# 8. Promotion gate contract

```text
operation s4_promote
  input:  PromotionGateInputs
  output: PromotionGateProduct

PromotionGateInputs :=
  {
    c_TS:                  S3CheckpointArtifact      ; ternary, from S3
    c_TS_v0success:        S3V0SuccessArtifact       ; from S3
    c_TS_oracle_agreement: S3OracleAgreementArtifact ; from S3
    gb_manifest:           GutenbergManifest         ; from §5
    contamination_report:  CrossCorpusReport         ; from §7
    baseline_gutenberg:    KnBaselineProduct         ; from §6.2
    repetition_collapse_check: RepetitionCheckArtifact ; from S3
  }

PromotionGateProduct :=
  {
    schema:                      "s4_promotion_gate.v1"
    tinystories_manifest_self_hash: Hash256
    gutenberg_manifest_self_hash: Hash256
    c_TS_checkpoint_self_hash:   Hash256
    contamination_self_hash:     Hash256
    baseline_gutenberg_self_hash: Hash256
    outcome:                     PromotionGateOutcome
    promotion_gate_self_hash:    Hash256
  }

Preconditions:
  Pr-Pre-1  Input bundle bytes are readable and parseable enough to identify
            the artifact kind. Semantic invalidity, missing required fields,
            self-hash mismatch, dirty contamination, baseline absence, and
            lineage mismatch are gate rejection reasons, not CLI aborts.

            Rejection reasons are complete over evaluable predicates. If an
            artifact's self-hash is invalid, the gate records that artifact's
            self-hash rejection reason and must not rely on that artifact's
            semantic fields for downstream predicates. Predicates whose only
            evidence would come from that invalid artifact are marked
            unevaluable and do not produce additional semantic rejection
            reasons.
  Pr-Pre-2  c_TS.metadata.build_kind = "phase_d" (or whatever S3 names
            the final ternary checkpoint phase). A mismatch is a gate
            rejection reason, not an abort.
  Pr-Pre-3  none. (Contamination HardFail is reported as
            P5_contamination_dirty, not as an abort.)

Postconditions:
  Pr-Ok-1   outcome = Promoted iff:
              P-1: c_TS_v0success.self_hash round-trips,
                   c_TS_v0success.checkpoint_self_hash =
                     c_TS.checkpoint_self_hash,
                   c_TS_v0success.tinystories_manifest_self_hash =
                     tinystories_manifest_self_hash,
                   c_TS_v0success.outcome = Pass for every acceptance bit,
                   and c_TS is Phase-D-resumable with QAT shadow weights

              P-2: c_TS_oracle_agreement.self_hash round-trips,
                   c_TS_oracle_agreement.checkpoint_self_hash =
                     c_TS.checkpoint_self_hash,
                   c_TS_oracle_agreement.tinystories_manifest_self_hash =
                     tinystories_manifest_self_hash,
                   and c_TS_oracle_agreement.outcome = Agree

              P-3: c_TS_v0success.ternary_gap_bpc <= 0.5
              P-4: gb_manifest validates per §5 G-Ok-1..G-Ok-12
              P-5: contamination_report.self_hash round-trips,
                   contamination_report.tinystories_manifest_self_hash =
                     tinystories_manifest_self_hash,
                   contamination_report.gutenberg_manifest_self_hash =
                     gb_manifest.manifest_self_hash,
                   and contamination_report.outcome in {Clean, Warn(_)}
                   (Warn promotes but forces
                    PassWithContaminationWarning in §11)
              P-6: gb_manifest.unmappable_rate_corpus <= 0.005
              P-7: baseline_gutenberg.self_hash round-trips,
                   baseline_gutenberg.gutenberg_manifest_self_hash =
                     gb_manifest.manifest_self_hash,
                   baseline_gutenberg.corpus_train_sha =
                     gb_manifest.train_sha256,
                   baseline_gutenberg.corpus_val_sha =
                     gb_manifest.val_sha256,
                   and baseline_gutenberg.bpc_kn5 is finite
              P-8: repetition_collapse_check.self_hash round-trips,
                   repetition_collapse_check.checkpoint_self_hash =
                     c_TS.checkpoint_self_hash,
                   and repetition_collapse_check.outcome = Pass
            else outcome = Rejected with reasons enumerated.
  Pr-Ok-2   Same inputs => same PromotionGateOutcome bytes under
            S1CanonicalJson encoding (referential transparency).
            Rejection reasons are emitted in the canonical order of
            PromotionGateRejectionReason enum declaration, not filesystem
            discovery order, parser map order, or check execution order.
  Pr-Ok-3   promotion_gate_self_hash round-trips per Self-hash rule.

  Pr-Fail-1
    Network access during evaluation, host-clock read other than for
    informational logging, or stdin read => abort with non-zero exit
    before producing the artifact.
```

---

# 9. Continuation training contract

## 9.1 Initialization

```text
operation s4_gutenberg_train_run
  input:  S4GutenbergRunInputs
  output: S4GutenbergRunProduct

S4GutenbergRunInputs :=
  {
    corpus_train:             ByteSeq    ; gutenberg_train, sha256-pinned
    corpus_val:               ByteSeq    ; gutenberg_val,   sha256-pinned
    model_config:             Toy0Config ; ModelSizeProfile::Toy0 (T14.1)
    train_config:             S4TrainConfig
    seed:                     Seed
    c_TS_ref:                 S3CheckpointArtifact   ; promoted by gate
    promotion_gate_self_hash: Hash256                ; from §8
  }

S4TrainConfig :=
  {
    optimizer_steps:   20000
    batch_size:        32
    sequence_length:   128
    eval_every_steps:  2000
    eval_subset_size:  4096
    optimizer:         AdamW { lr: 5e-4, beta1: 0.9, beta2: 0.999,
                               eps: 1e-8, weight_decay: 0.0 }
    phase:             Phase::D       ; QAT fully hardened (continuation)
    rng_kind:          Pcg64Mcg
    device_profile:    S1CpuDeterministic
  }

Initial state:
  model_weights_initial(s) = c_TS_ref.weights
                              ; warm weights from S3 ternary checkpoint
  qat_shadow_weights_initial(s) = c_TS_ref.qat_shadow_weights
                              ; warm FP shadow weights from the S3
                              ; Phase-D-resumable checkpoint. Absence of
                              ; this payload is a promotion-gate rejection,
                              ; not a training-time inference opportunity.
  adamw_state_initial(s)   = ZeroInitAdamW
                              ; cold optimizer, per D9
  phase_state_initial(s)   = Phase::D
                              ; continuation, no warmup phases re-run
  rng_streams_initial(s):
    InitRng(s)    = Pcg64Mcg(seed128("s4-init-init",      s))
    BatchRng(s)   = Pcg64Mcg(seed128("s4-init-batch",     s))
    ShuffleRng(s) = Pcg64Mcg(seed128("s4-init-shuffle",   s))

  S4 RNG streams are disjoint from S1 / S2 / S3 streams by domain string.
  No cross-slice rng leakage.
```

## 9.2 Training step contract

```text
Training batch sampler:
  start_offset(step, batch_index) is drawn from BatchRng(seed) uniformly
  over the inclusive integer interval
  0..=(token_length(corpus_train) - sequence_length), via rejection
  sampling. The sampled token-id sequence is
  corpus_train[start_offset .. start_offset + sequence_length].

  BatchRng draw order:

    for step in 1..=optimizer_steps:
      for batch_index in 0 <= batch_index < batch_size:
        draw one start_offset

  This produces exactly batch_size sequences per step (not batch_size + 1).

  Evaluation, scoring, baseline fitting, contamination, promotion-gate
  evaluation, initialization, and oracle-agreement runs must not consume
  from BatchRng.

Training objective:
  For each sampled sequence x[0..sequence_length), the model scores all
  sequence_length target ids under the same chunked-reset semantics used
  by the bpc primitive:

    - x[0] is predicted from the deterministic zero initial state.
    - for j > 0, x[j] is predicted from the state after consuming x[0..j).
    - state is reset between batch elements.
    - state is reset between optimizer steps.

  Per-step train loss is the mean natural-log cross entropy over
  batch_size * sequence_length target ids. No padding token, no synthetic
  boundary token. <bos>/<eos> are ordinary input ids inserted at
  manifest-build time (G-Ok-5).

Progress eval subset:
  progress_eval_ids = gutenberg_val[0 .. min(token_length(gb_val),
                                             4096 * 128)]
  Scored with the same non-overlapping 128-id chunking and final-short-
  chunk rule as final gate scoring.

Gate scoring:
  Full gutenberg_val ids, including a final short chunk if present.
```

## 9.3 Run product

```text
S4GutenbergRunProduct :=
    CompletedS4RunProduct | DivergedS4RunProduct

CompletedS4RunProduct :=
  {
    seed:                       Seed
    final_checkpoint:           SafeTensors blob       ; canonical write;
                                                       ; contains both the
                                                       ; deployed ternary
                                                       ; tensors and the QAT
                                                       ; FP shadow tensors
                                                       ; under schema-pinned
                                                       ; tensor names
    final_checkpoint_payload_sha: Hash256              ; deployed ternary
    final_fp_shadow_payload_sha:  Hash256              ; QAT shadow weights
    metadata:                   S4CheckpointMetadata
    run_log:                    S4RunLog
    weight_stats:               WeightStats[per eval_every_steps]
    grad_log:                   GradLog[per optimizer step]
    completion:                 Completed
  }

DivergedS4RunProduct :=
  {
    seed:                       Seed
    run_log:                    S4RunLog
    weight_stats:               WeightStats[recorded until divergence]
    grad_log:                   GradLog[recorded until divergence]
    completion:                 DivergedAt(TrainStep)
    divergence_event:           {
                                  step: TrainStep,
                                  observed: NonFiniteLoss | NonFiniteGradNorm,
                                  last_finite_loss: Null | LossNatsPerToken
                                }
  }

Postconditions:
  S4-Run-Ok-1
    completion = Completed
    => for all step in 1..=optimizer_steps. run_log.loss(step) is finite.
  S4-Run-Ok-2
    completion = Completed
    => run_log records 20000 train losses for steps 1..20000,
       plus 11 eval points at steps 0, 2000, 4000, ..., 20000.
  S4-Run-Ok-3
    completion = Completed
    => final_checkpoint deserializes back to a Toy0 ternary model whose
       weight tensors match the in-memory model at termination
       byte-for-byte under canonical write.
       The same checkpoint also deserializes the QAT FP shadow tensors,
       whose canonical payload hash equals final_fp_shadow_payload_sha.
  S4-Run-Ok-4
    initial_checkpoint_payload_sha == c_TS_ref.canonical_tensor_payload_sha
    AND initial_fp_shadow_payload_sha ==
        c_TS_ref.qat_shadow_tensor_payload_sha
    AND initial_weight_source == "c_TS_ref".
    AND init_rng_draw_count_before_first_step == 0.
    Replay validates this invariant before consuming run_log; a recorded
    run that deviates (e.g. silently re-initialized from InitRng) is
    refuted under H6 even if H4 quality happened to pass by accident.
  S4-Run-Fail-1
    completion = DivergedAt(k)
    => divergence_event.step = k and divergence_event.observed records
       the first non-finite loss or gradient norm without serializing
       NaN or Inf.
  S4-Run-Fail-2
    exists s. completion(s) = DivergedAt(_)
    => S4Outcome = Fail-substrate (per D13).
  S4-Run-Warn-1
    A 10-step mean train-loss increase greater than 2.0 is recorded as
    a surprise, not DivergedAt, unless it also produces non-finite loss.
```

---

# 10. v0_success on Gutenberg

```text
operation s4_v0_success_gutenberg
  input:  S4V0SuccessInputs
  output: S4V0SuccessProduct

S4V0SuccessInputs :=
  {
    checkpoint:           CompletedS4RunProduct.final_checkpoint
    val_ids:              gutenberg_val             ; sha256-pinned
    workload_manifest_template:
                          S3 v0_success workload manifest template
                            ; inherited predicate definitions
    workload_manifest_instance:
                          S4-bound v0_success workload instance
                            ; binds the inherited predicates to
                            ; gutenberg_train/gutenberg_val and records
                            ; corpus sha256s
    baseline_gutenberg:   KnBaselineProduct
    fp_reference:         S4FpReferenceArtifact
                            ; FP reference for ternary_qat_gap_ok on
                            ; Gutenberg val (see S4FpReferenceArtifact)
    seed:                 Seed
  }

S4FpReferenceArtifact :=
  {
    schema:                          "s4_fp_reference.v1"
    seed:                            Seed
    source_checkpoint_self_hash:     Hash256
    fp_reference_kind:
      "qat_shadow_weights_after_gutenberg_continuation"
      ; The FP reference is the QAT shadow-weight side of the c_GB(s)
      ; checkpoint after Gutenberg continuation training.
      ;
      ; The checkpoint or paired run artifact must carry the shadow weights
      ; needed to evaluate this reference. If shadow weights are unavailable,
      ; ternary_qat_gap_ok is not evaluable and v0_success fails for that
      ; seed. S4 must not synthesize or infer missing FP shadow weights after
      ; the fact.
    fp_shadow_payload_sha:           Hash256
    tinystories_manifest_self_hash:  Hash256
    gutenberg_manifest_self_hash:    Hash256
    corpus_val_sha:                  Hash256
    fp_reference_self_hash:          Hash256
  }

S4V0SuccessProduct :=
  {
    schema:                "s4_gutenberg_score.v1"
    tinystories_manifest_self_hash: Hash256
    gutenberg_manifest_self_hash: Hash256
    seed:                  Seed
    checkpoint_self_hash:  Hash256
    checkpoint_payload_sha: Hash256
    corpus_val_sha:        Hash256
    workload_manifest_template_self_hash: Hash256
    workload_manifest_instance_self_hash: Hash256
    fp_reference_self_hash: Hash256
    bpc_ternary:           BpcValue
    bpc_kn5:               BpcValue           ; from baseline_gutenberg
    bpc_margin:            f64                ; bpc_kn5 - bpc_ternary
    v0_success_acceptance:
      {
        prompt_length_ok:           Bool
        generation_length_ok:       Bool
        no_repetition_collapse:     Bool
        only_charset_v1_ids:        Bool
        beats_kn5_baseline:         Bool   ; bpc_margin > 0.05
        ternary_qat_gap_ok:         Bool   ; bpc_ternary(c_GB_deployed,
                                           ;             GB_val)
                                           ; - bpc_fp_reference(
                                           ;     c_GB_fp_shadow, GB_val)
                                           ; <= 0.5
        runtime_chrome_budget_ok:   Bool   ; conservative S6-stub estimate
        emulator_smoke_ok:          Bool   ; one token through emu, S3 stub
      }
    pass:                  Bool                ; AND over acceptance bits
    score_self_hash:       Hash256
  }

Inheritance:
  The acceptance bits and their exact predicate forms are inherited from
  S3's v0_success workload manifest template verbatim. S4 creates a
  corpus-bound workload instance that records gutenberg_train and
  gutenberg_val hashes. No new bit, no new threshold; only the corpus
  identity changes.

Strict per-seed pass:
  for all s in {0,1,2,3,4}. s4_v0_success_gutenberg(c_GB(s), ...).pass
                              = true
  is the H4 closure gate. Aggregate (median) is reported but does not
  participate in the gate.
```

---

# 11. Outcome algebra

```text
S4Outcome tag strings :=
    "PassClean"
  | "PassWithContaminationWarning"
  | "Fail-corpus-integrity"
  | "Fail-contamination"
  | "Fail-promotion-gate"
  | "Fail-promotion-gate-readiness"
  | "Fail-quality-on-gutenberg"
  | "Fail-oracle-disagreement"
  | "Fail-substrate"
  | "Fail-suspicious"

Rust/API enum names, if needed, use identifier-safe forms:

    PassClean
    PassWithContaminationWarning
    FailCorpusIntegrity
    FailContamination
    FailPromotionGate
    FailPromotionGateReadiness
    FailQualityOnGutenberg
    FailOracleDisagreement
    FailSubstrate
    FailSuspicious

Where:
  PassClean
    H1, H2, H3, H4, H5, H6 all Confirmed.
    H7 verdict reported but does not affect S4Outcome.
    contamination_report.outcome = Clean.

  PassWithContaminationWarning
    H1, H2, H3, H4, H5, H6 all Confirmed.
    contamination_report.outcome = Warn(_).

  Fail-corpus-integrity         H1 Refuted
  Fail-contamination            H2 Refuted because either contamination
                                HardFail occurred in a gated direction or
                                COr-6 proved the contamination measurement
                                oracle invalid.
  Fail-promotion-gate           H3 Refuted (gate implementation unsound)
  Fail-promotion-gate-readiness H3 Confirmed, but canonical c_TS rejected
                                for one or more true D8 reasons
  Fail-quality-on-gutenberg     H4 Refuted (non-suspicious)
  Fail-oracle-disagreement      H5 Refuted
  Fail-substrate                H6 Refuted, or any seed diverged
  Fail-suspicious               median(bpc_ternary_gutenberg) < 0.5
```

Combination (mandatory checks first; corpus failures take priority over
substrate/quality so early-exit reports do not need phantom downstream
evidence):

```text
if H1 status = Refuted                                     => Fail-corpus-integrity
elif H1 status = NotEvaluatedDueToPriorGate(_)             => Fail-substrate
elif H2 status = Refuted                                   => Fail-contamination
elif H3 status = Refuted                                   => Fail-promotion-gate
elif promotion_gate.outcome = Rejected(_)                  => Fail-promotion-gate-readiness
elif exists seed s. completion(s) = DivergedAt(_)          => Fail-substrate
elif scores_exist and median(bpc_ternary_gutenberg) < 0.5  => Fail-suspicious
elif H4 status = Refuted                                   => Fail-quality-on-gutenberg
elif H5 status = Refuted                                   => Fail-oracle-disagreement
elif H6 status = Refuted                                   => Fail-substrate
elif contamination_report.outcome = Warn(_)                => PassWithContaminationWarning
else                                                       => PassClean

Validity constraint:
  No dispatcher may reach the final `else` unless H1..H6 are all Confirmed.
  Any NotEvaluatedDueToPriorGate status must be explained by an earlier
  branch in this ordering; otherwise the report is invalid and maps to
  Fail-substrate.
  `scores_exist` is true iff all five s4_gutenberg_score.v1 artifacts
  exist and self-hash-validate. Dispatchers must not evaluate median-based
  branches before that condition holds.
```

Decision dispatch:

```text
PassClean                            -> Decision::ProceedToS5
PassWithContaminationWarning         -> Decision::ProceedToS5-with-contamination-warning
Fail-corpus-integrity                -> Decision::Halt(corpus-integrity-broken)
Fail-contamination                   -> Decision::Halt(contamination-dirty)
Fail-promotion-gate                  -> Decision::Halt(promotion-gate-unsound)
Fail-promotion-gate-readiness        -> Decision::Halt(promotion-gate-rejected-canonical)
Fail-quality-on-gutenberg            -> Decision::Investigate(propose-step-budget-or-Toy1)
Fail-oracle-disagreement             -> Decision::Halt(oracle-disagrees-on-gutenberg)
Fail-substrate                       -> Decision::Investigate(burn-or-corpus-loader)
Fail-suspicious                      -> Decision::Halt(audit-split-and-bpc)
```

`Halt` blocks bd-2hmm closure unconditionally. `Investigate` creates a
follow-up bead. Any extension of this RFC's scope or seed list requires
an explicit RFC amendment under Rule Amendment.

---

# 12. Artifact schemas

## 12.1 s4_gutenberg_checkpoint.v1

```text
Path:
  experiments/S4/checkpoints/seed-{seed}/checkpoint.safetensors
  experiments/S4/checkpoints/seed-{seed}/checkpoint.metadata.json

S4CheckpointMetadata (JSON) :=
  {
    schema:                       "s4_gutenberg_checkpoint.v1"
    seed:                         Seed
    c_TS_checkpoint_self_hash:    Hash256
    promotion_gate_self_hash:     Hash256
    deployed_tensor_payload_sha:  Hash256
    fp_shadow_tensor_payload_sha: Hash256
    corpus_train_sha:             Hash256          ; gutenberg_train
    corpus_val_sha:               Hash256          ; gutenberg_val
    gutenberg_manifest_self_hash: Hash256
    tinystories_manifest_self_hash: Hash256
    model_config_hash:            Hash256
    train_config_hash:            Hash256
    build_kind:                   "phase_d_continuation"
    build_config_hash:            Hash256
    dependency_lockfile_sha:      Hash256
    rust_toolchain_hash:          Hash256
    device_profile_hash:          Hash256
    pass_version:                 SemVer
    final_step:                   TrainStep
    final_train_loss:             LossNatsPerToken
    completion:                   Completed
    checkpoint_self_hash:         Hash256
  }

Invariants:
  S4-C-Self-Hash      DomainHash(...) round-trips.
  S4-C-Determinism    Replay with same seed + same hashes => identical
                      safetensors bytes.
  S4-C-NoLeakage      Replay must not depend on host clock, network, or
                      stdin. The runner enforces S1CpuDeterministic
                      .env_exact before any tensor allocation.
  S4-C-LineageTS      c_TS_checkpoint_self_hash is non-null and matches
                      the checkpoint promoted by §8.
```

## 12.2 s4_gutenberg_run_log.v1

```text
Path:
  experiments/S4/runs/seed-{seed}/run-log.json
  experiments/S4/runs/seed-{seed}/grad-log.jsonl
  experiments/S4/runs/seed-{seed}/weight-stats.jsonl

S4RunLog (JSON) :=
  {
    schema:                  "s4_gutenberg_run_log.v1"
    tinystories_manifest_self_hash: Hash256
    gutenberg_manifest_self_hash: Hash256
    seed:                    Seed
    train_config_hash:       Hash256
    promotion_gate_self_hash: Hash256
    c_TS_checkpoint_self_hash: Hash256
    initial_checkpoint_payload_sha: Hash256
                              ; canonical-tensor payload sha of weights
                              ; loaded as model_weights_initial(s); used
                              ; by the F4 lineage invariant (S4-Run-Ok-4)
    initial_weight_source:   "c_TS_ref"
                              ; constant; deviating from this in the
                              ; recorded run violates the lineage check
    initial_fp_shadow_payload_sha: Hash256
                              ; canonical-tensor payload sha of QAT shadow
                              ; weights loaded from c_TS_ref
    init_rng_draw_count_before_first_step: 0
                              ; D9 warm-start requires InitRng to consume
                              ; zero draws before training
    shuffle_rng_draw_count_total: 0
                              ; S4 v1 random-offset sampler does not consume
                              ; ShuffleRng
    losses:                  List[(TrainStep, LossNatsPerToken)]
                                                ; one per optimizer step
    eval_points:             List[(EvalStep, BpcValue)]
                                                ; includes step 0
    final_grad_norms:        GradNormSummary
    run_log_self_hash:       Hash256
  }

Invariants:
  S4-RL-Length     losses.length = train_config.optimizer_steps = 20000
  S4-RL-Eval       eval_points.length = optimizer_steps / eval_every_steps
                                       + 1 = 11
  S4-RL-Finite     every recorded value is finite (else completion =
                   DivergedAt)
```

## 12.3 s4_gutenberg_score.v1

Defined inline in §10 above (S4V0SuccessProduct is the same JSON object
written under
`experiments/S4/scores/seed-{seed}/score.json`).

## 12.4 s4_baseline_gutenberg.v1

```text
Path:
  experiments/S4/baseline/kn5.bin
  experiments/S4/baseline/kn5-report.json
  experiments/S4/baseline/unigram-report.json

Note:
  kn5-report.json is the canonical s4_baseline_gutenberg.v1 artifact.
  kn5.bin is the counts blob bound by counts_blob_sha256.

S4BaselineReport (JSON) :=
  {
    schema:                          "s4_baseline_gutenberg.v1"
    tinystories_manifest_self_hash:  Hash256
    gutenberg_manifest_self_hash:    Hash256
    corpus_train_sha:                Hash256
    corpus_val_sha:                  Hash256
    kn_params:                       KneserNeyParams
    bpc_kn5:                         BpcValue
    bpc_kn3:                         BpcValue
    bpc_unigram:                     BpcValue
    counts_summary:                  CountsSummary
    counts_blob_sha256:              Hash256
    baseline_gutenberg_self_hash:    Hash256
  }

Invariants:
  S4-B-Self-Hash   round-trips.
Reported (not invariant):
  bpc_kn5 <= bpc_kn3 <= bpc_unigram
```

## 12.5 s4_corpus_quality.v1

```text
Path:
  experiments/S4/corpus_quality/corpus_quality.json

S4CorpusQuality (JSON) :=
  {
    schema:                          "s4_corpus_quality.v1"
    gutenberg_manifest_self_hash:    Hash256
    tinystories_manifest_self_hash:  Hash256
    per_corpus:
      [
        {
          corpus_id:                "TinyStories" | "Gutenberg"
          unmappable_rate:          f64
          tokens_per_doc_mean:      f64
          tokens_per_doc_median:    f64
          tokens_per_doc_max:       u64
          longest_doc_id:           Null | String
          charset_coverage_count:   u64    ; distinct charset_v1 ids seen
        }
      ]
    corpus_quality_self_hash:        Hash256
  }
```

## 12.6 s4_contamination_report.v1

Defined in §7 (CrossCorpusReport). Path:

```text
experiments/S4/contamination/cross_corpus.json
```

## 12.7 s4_promotion_gate.v1

Defined in §8 (PromotionGateProduct). Path:

```text
experiments/S4/promotion_gate/promotion_gate.json
```

## 12.8 s4_oracle_agreement.v1

```text
Path:
  experiments/S4/oracle_agreement/seed-0.json

S4OracleAgreementReport (JSON) :=
  {
    schema:                       "s4_oracle_agreement.v1"
    tinystories_manifest_self_hash: Hash256
    gutenberg_manifest_self_hash: Hash256
    seed:                         0
    checkpoint_sha:               Hash256
    corpus_val_sha:               Hash256
    fixture_set_self_hash:        Hash256       ; from S3
    bpc_live:                     BpcValue
    bpc_denotational:             BpcValue
    bpc_artifact:                 BpcValue
    gap_live_vs_denotational:     f64
    gap_live_vs_artifact:         f64
    gap_denotational_vs_artifact: f64
    s3_tolerance_self_hash:       Hash256       ; pinned tolerances
    outcome:                      Agree | Disagree
    oracle_agreement_self_hash:   Hash256
  }

Invariants:
  S4-O-Self-Hash        round-trips.
  S4-O-S3InheritedTol   tolerance values used to compute outcome are the
                        S3-pinned values; recomputing the tolerance hash
                        from S3 must match s3_tolerance_self_hash.
```

## 12.9 s4_fp_reference.v1

```text
Path:
  experiments/S4/fp_reference/seed-{seed}/fp-reference.json

Schema:
  S4FpReferenceArtifact from §10.

Invariants:
  S4-FP-Self-Hash       fp_reference_self_hash round-trips.
  S4-FP-Lineage         source_checkpoint_self_hash matches the
                        corresponding s4_gutenberg_checkpoint.v1
                        checkpoint_self_hash.
  S4-FP-Corpus          corpus_val_sha matches gutenberg_manifest.val_sha256.
```

## 12.10 s4_corpus_progression.v1

```text
Path:
  experiments/S4/corpus_progression/schedule.json

S4CorpusProgressionReport :=
  {
    schema:                         "s4_corpus_progression.v1"
    tinystories_manifest_self_hash: Hash256
    gutenberg_manifest_self_hash:   Hash256
    schedule:                       CorpusProgressionScheduleSnapshot
    corpus_progression_self_hash:   Hash256
  }

Invariants:
  S4-CP-Self-Hash       corpus_progression_self_hash round-trips.
  S4-CP-Edge            exactly one edge exists:
                        {from: TinyStories, to: Gutenberg,
                         gate: G_TS->Gutenberg}.
```

## 12.11 s4_report.v1

```text
Path:
  docs/experiments/S4-report.md

Front-matter:
  ---
  schema:                          "s4_report.v1"
  s4_outcome:                      S4Outcome
  decision:                        Decision
  ts_manifest_self_hash:           Null | Hash256
  gutenberg_manifest_self_hash:    Null | Hash256
  baseline_gutenberg_self_hash:    Null | Hash256
  corpus_quality_self_hash:        Null | Hash256
  contamination_self_hash:         Null | Hash256
  promotion_gate_self_hash:        Null | Hash256
  corpus_progression_self_hash:    Null | Hash256
  c_TS_checkpoint_self_hash:       Null | Hash256
  per_seed_artifacts:
    List[{
      seed:                         Seed,
      completion:                   Completed | DivergedAt(TrainStep)
                                              | NotReached,
      checkpoint_self_hash:         Null | Hash256,
      run_log_self_hash:            Null | Hash256,
      score_self_hash:              Null | Hash256,
      oracle_agreement_self_hash:   Null | Hash256
    }]
  generated_at:           RFC3339 UTC, informational only, excluded from
                          report hash.
                          Report generation may read the host clock only
                          for this field. Training, scoring, baseline,
                          contamination, promotion-gate, oracle, and
                          corpus-quality artifacts must not depend on
                          host clock.
  rfc_revision:                    GitCommitId | Hash256
  predictions_section_hash:        Hash256
  predictions_commit:              GitCommitId
  first_result_commit:             GitCommitId
  report_self_hash:                Hash256
  ---

Canonicalization:
  The front-matter is parsed as a restricted YAML subset:

    - mappings with string keys,
    - arrays,
    - strings,
    - integers,
    - finite floats,
    - booleans,
    - null,
    - no anchors,
    - no aliases,
    - no custom tags.

  The parsed value is converted to the S1CanonicalJson data model and
  canonicalized using S1CanonicalJson before hashing. Any YAML construct
  outside this restricted subset is invalid.

Required sections (markdown body):
  ## Pre-registered predictions
    Predicted ranges and pass criteria as committed before any S4
    result artifact.

    Raw fixture pins in fixtures/corpora/gutenberg.toml may precede the
    prediction commit. The emitted gutenberg_manifest.v1 is not excluded:
    it contains measured corpus-integrity evidence such as drop counts,
    byte lengths, split counts, and unmappable_rate_corpus.

  ## Observed
    Per-seed table: bpc_ternary_gutenberg, v0_success_pass, completion.
    Plus baseline numbers, contamination overlap fractions, and
    aggregate statistics.

  ## Hypothesis verdicts
    H1, H2, H3, H4, H5, H6, H7 each as HypothesisStatus, with the
    concrete observation that drove each verdict.
    Closure-candidate reports must use only Confirmed | Refuted for
    H1..H6. H7 may be Confirmed | Refuted when its required scores exist;
    otherwise early-failure reports may mark it
    NotEvaluatedDueToPriorGate(reason). It is not closure-gating.
    Early-failure reports may use NotEvaluatedDueToPriorGate(reason)
    for hypotheses whose required observations do not exist because an
    earlier mandatory gate failed (e.g. T3a promotion-gate rejection
    bypasses Gutenberg training, scoring, and oracle agreement).

  ## Falsification analysis
    Direct citation of which prediction or falsification rule fired for
    each Refuted hypothesis.

  ## Surprises
    Anything outside predicted ranges, even if not a verdict change.

  ## Decision
    Exactly one Decision tag, justified in <=3 sentences.

  ## Reproducibility statement
    Exact command + manifest hashes + pass_version to replay.

Invariants:
  S4-R-Decision        Exactly one Decision tag in front-matter.
  S4-R-AllSeeds        per_seed_artifacts and the observed per-seed
                       table cover all 5 seeds in {0,1,2,3,4}.
  S4-R-ClosureArtifacts
                       For Decision in {ProceedToS5,
                       ProceedToS5-with-contamination-warning},
                       all top-level artifact hashes are non-null;
                       checkpoint_self_hash, run_log_self_hash, and
                       score_self_hash are non-null for all five seeds;
                       oracle_agreement_self_hash is non-null for seed 0;
                       and corpus_progression_self_hash is non-null.
  S4-R-Self-Hash       report_self_hash is computed over:
                         - front-matter with generated_at and
                           report_self_hash omitted
                         - markdown body bytes exactly as committed
                       using S1CanonicalJson for front-matter
                       normalization.
                       The committed report file MUST use LF line endings.
                       CRLF or CR line endings make the report invalid.
  S4-R-Predictions     The commit introducing the exact "Pre-registered
                       predictions" section, identified by
                       predictions_section_hash, is a strict ancestor of
                       first_result_commit. first_result_commit is the earliest
                       commit introducing any gutenberg_manifest_self_hash,
                       checkpoint_self_hash,
                       run_log_self_hash, score_self_hash,
                       fp_reference_self_hash,
                       oracle_agreement_self_hash,
                       corpus_quality_self_hash,
                       corpus_progression_self_hash,
                       contamination_self_hash,
                       promotion_gate_self_hash, or
                       baseline_gutenberg_self_hash derived from S4
                       execution.
                       If any `[ESTIMATE]` threshold is resolved after
                       predictions_commit, the resolved revision becomes the
                       new predictions_commit and must still be a strict
                       ancestor of first_result_commit.
  S4-R-AllHypotheses   All seven hypotheses have an explicit
                       HypothesisStatus. For Decision in {ProceedToS5,
                       ProceedToS5-with-contamination-warning}, every
                       H1..H6 status must be a binary Verdict, not
                       NotEvaluatedDueToPriorGate.
```

The pre-registration timestamp is itself a load-bearing artifact:
predictions written after-the-fact are not pre-registered, even if
textually identical.

---

# 13. Reproducibility laws

```text
Rep-1 Seed determinism
  for all s. replay(s, ts_manifest, gb_manifest, c_TS_ref) is byte-
  identical to original(s, ...).

Rep-2 Cross-machine determinism is NOT required for v1.
  Bit-identicality is asserted within a single machine + OS + pinned
  Burn version + pinned dependency lockfile + S1CpuDeterministic device
  profile. Cross-platform reproducibility is a future concern.

Rep-3 Corpus pinning
  Every s4_*.v1 artifact except gutenberg_manifest.v1 records both
  tinystories_manifest_self_hash and gutenberg_manifest_self_hash.
  This includes s4_fp_reference.v1.
  Replay validates these against on-disk manifests before proceeding.
  Replay also re-validates corpus_train_sha and corpus_val_sha for both
  corpora.

Rep-4 Train-config pinning
  train_config_hash binds D10 values exactly. Changing any pinned value
  invalidates prior s4 artifacts.

Rep-5 Pass-version pinning
  pass_version is bumped by any change to: optimizer step semantics,
  Phase D continuation behavior, sequence-state forward, AdamW state
  reset rule, or rng stream domain strings. Bump invalidates checkpoints.

Rep-6 RFC revision pinning
  s4_report.v1 records the git sha of this RFC at report generation. A
  re-run after this RFC is amended produces a new report with a new
  rfc_revision; old reports remain valid for their revision.

Rep-7 Per-seed isolation
  No global mutable state is shared across seeds. Seed s and seed s' are
  independent runs; no rng leakage, no shared tensor cache, no static
  mutable model registry.

Rep-8 Cross-slice isolation
  S4 RNG streams are domain-string-disjoint from S1, S2, S3 streams.
  S4 must not consume or mutate any S1 / S2 / S3 RNG state.

Rep-9 No hidden semantic inputs
  Informational report fields such as generated_at are excluded from
  semantic hashes and closure predicates.

Rep-10 Lineage edge to S3
  Every s4_*.v1 checkpoint metadata records c_TS_checkpoint_self_hash.
  Replay validates this against the on-disk S3 ternary checkpoint
  before consuming it.

Rep-11 Promotion-gate self-hash binds downstream
  Every Gutenberg training, scoring, and oracle artifact records
  promotion_gate_self_hash and refuses to load if the on-disk
  s4_promotion_gate.v1 self-hash does not match.
```

---

# 14. Decision protocol

```text
S4 closure (bd-2hmm) requires:
  1. Gutenberg manifest built and validated; COr-1..COr-6
     all pass.
  2. Cross-corpus contamination report emitted with outcome in
     {Clean, Warn(_)} (HardFail blocks closure).
  3. Promotion gate evaluated with outcome = Promoted.
  4. All 5 Gutenberg seed runs Completed (D13).
  5. s4_report.v1 emitted with R-Predictions verified by git history.
  6. Decision in {ProceedToS5, ProceedToS5-with-contamination-warning}.
  7. baseline_gutenberg_self_hash, contamination_self_hash,
     promotion_gate_self_hash, gutenberg_manifest_self_hash, and
     per_seed_artifacts recorded in front-matter.
  8. Three-way oracle agreement recorded for seed 0 with outcome = Agree.
  9. v0_success on Gutenberg passes for every seed (H4).
  10. Falsification suite (§15 O5) green.

S4 closure is forbidden when:
  Any of:
    Decision::Halt(_), Decision::Investigate(_),
    missing pre-registration,
    any seed completion = DivergedAt(_),
    contamination_report.outcome = HardFail(_),
    promotion_gate.outcome = Rejected(_),
    oracle_agreement.outcome = Disagree,
    any required artifact missing or self-hash invalid.

If Decision = ProceedToS5-with-contamination-warning:
  S5 inherits a known-warning corpus-progression edge. The S5 RFC must
  reference the warning explicitly. This warning does not by itself
  block S5; S5's own closure may downgrade or upgrade it.

  No structural slice-graph amendment is made (unlike S1's optional
  T12.5 prereq edge).
```

---

# 15. Proof obligations

```text
O1  Pre-registration provability
    "Pre-registered predictions" section content of S4-report.md must
    appear in git history strictly before any S4 result artifact commit
    (including the emitted gutenberg_manifest.v1; raw fixture pins may
    exist earlier).
    CI script asserts:
      1. predictions_section_hash matches the exact normalized markdown
         section in predictions_commit;
      2. predictions_commit is a strict ancestor of first_result_commit;
      3. first_result_commit is the earliest commit introducing any
         gutenberg_manifest_self_hash,
         checkpoint_self_hash, run_log_self_hash, score_self_hash,
         fp_reference_self_hash, oracle_agreement_self_hash,
         corpus_quality_self_hash, corpus_progression_self_hash,
         contamination_self_hash, promotion_gate_self_hash, or
         baseline_gutenberg_self_hash derived from S4 execution.

    This proves repository pre-registration order.

O2  Determinism
    Same seed + same ts_manifest_self_hash + same
    gutenberg_manifest_self_hash + same c_TS_checkpoint_self_hash +
    same train_config_hash + same pass_version + same device_profile +
    same dependency lockfile -> bit-identical safetensors.

    v1 CI smoke test:
      run seed 0 twice and assert byte equality of canonical-tensor
      payload.

    v1 closure evidence:
      replay all five seeds under the report-pinned hashes and assert
      byte equality of canonical-tensor payloads and s4_*.v1 self-hashes.

    v1 law:
      all five seeds satisfy the same replay property; H6 is Confirmed
      iff every seed in {0,1,2,3,4} replays bit-identically.

O3  Measurement-oracle correctness (corpus side)
    COr-1..COr-6 all pass. (Required for closure.)

O4  Three-way oracle agreement on Gutenberg
    For seed 0: s4_oracle_agreement.outcome = Agree under S3-pinned
    tolerances. (Required for closure.)

O5  Falsification suite
    Six deliberately-broken implementations must each produce the
    expected Refuted verdict on the corresponding hypothesis:

      F1-broken: gutenberg_manifest_lossy_decompression
                  (a unicode-stripping decompressor that silently drops
                   non-ASCII bytes during NFC normalization)
                                                       => H1 Refuted
                  Required fixture:
                    includes at least one retained Gutenberg-shaped document
                    with non-ASCII text whose charset_v1-normalized token
                    stream is pinned. The lossy implementation must change
                    post_strip_sha256 and/or post_charset_body_sha256.
      F2-broken: contamination_check_window_too_small
                  (n=3 instead of n=13)
                                                       => contract rejection
                                                          before report write,
                                                          or H2 Refuted via
                                                          X-Pre-3 violation
      F3-broken: promotion_gate_skips_oracle_agreement
                  (P-2 silently wired to true regardless of input)
                                                       => H3 Refuted
      F4-broken: gutenberg_train_resets_to_random_weights_silently
                  (ignores c_TS, calls InitRng instead of loading c_TS)
                                                       => H6 Refuted via
                                                          lineage/replay
                                                          invariant
                                                          (S4-Run-Ok-4),
                                                          not H4. Random
                                                          init might pass
                                                          quality by
                                                          accident; the
                                                          lineage check
                                                          catches it
                                                          deterministically.
                  Required assertion:
                    initial_checkpoint_payload_sha is computed from the
                    actual in-memory model weights after initialization,
                    before the first optimizer step, not copied from config.
      F5-broken: oracle_drift_under_corpus_switch
                  (ArtifactOracle silently uses tinystories_val
                   normalization on gutenberg_val tokens)
                                                       => H5 Refuted
      F6-broken: unmappable_rate_silently_dropped
                  (fixture contains one document with unmappable_density
                   strictly greater than 0.02; implementation computes but
                   does not enforce the drop)
                                                       => H1 Refuted
                                                          and (transitively)
                                                          can elevate H4
                                                          quality risk; the
                                                          test asserts H1.

    These are unit tests against the S4 framework, not actual S4 runs.
    Required test files:
      gbf-experiments/tests/s4_falsification.rs
      gbf-experiments/tests/s4_falsification/s4_f1_lossy_decompression.rs
      gbf-experiments/tests/s4_falsification/s4_f2_window_too_small.rs
      gbf-experiments/tests/s4_falsification/s4_f3_gate_skips_oracle.rs
      gbf-experiments/tests/s4_falsification/s4_f4_train_random_init.rs
      gbf-experiments/tests/s4_falsification/s4_f5_oracle_drift.rs
      gbf-experiments/tests/s4_falsification/s4_f6_unmappable_dropped.rs
    These tests are gated by the test-only `falsify` feature on
    gbf-experiments so the broken substitutes cannot leak into a
    release build.

O6  Hash round-trip
    Every emitted s4_*.v1 artifact round-trips through canonical JSON
    with self-hash equality.

O7  Outcome algebra totality
    Every observable combination of binary H1..H6 verdicts, per-seed
    completion states, suspicion thresholds, and PromotionGateOutcome
    (Promoted | Rejected) maps to exactly one S4Outcome variant under
    §11. The Fail-promotion-gate-readiness branch in particular must be
    reachable when H3 is Confirmed and promotion_gate.outcome =
    Rejected(_).

O8  No hidden inputs
    s4 artifacts depend only on:
      ts_manifest, gb_manifest (sha256-pinned)
      c_TS_ref (sha256-pinned)
      model_config (Toy0 pinned by T14.1 reference instance)
      train_config (D9, D10 pinned)
      seed
      pass_version
      gbf-train pinned dependency set
      gbf-data pinned dependency set
      S3-pinned KN_params and v0_success workload manifest
    No env-var, no host-clock, no network, no stdin during the
    train/score/oracle/promotion-gate stages. The corpus-build stage
    may read network only during the explicit fixture-build CLI op.

O9  Per-seed isolation
    Seed s and seed s' produce independent run products. No shared
    mutable state.

    CI smoke checks, not a complete proof:
      1. seed 0 and seed 1 produce different first-1024 BatchRng offset
         traces under the same corpus length and train_config;
      2. running seeds [0, 1] and [1, 0] produces the same per-seed
         hashes.

O10 Cross-slice rng isolation
    S4 RNG streams are domain-string-disjoint from S1, S2, S3 streams.
    A CI lint enforces the domain-string convention "s4-*" for seed128
    training RNG domains in gbf-experiments::s4 training modules. Hash-domain
    strings used for fixture selection and split assignment may use the
    longer "gbf:s4:*" convention.

    A unit test additionally asserts that:

      seed128("s4-init-batch", 0) != seed128("s1-batch", 0)
      seed128("s4-init-batch", 0) != seed128("s2-batch", 0)
      seed128("s4-init-batch", 0) != seed128("s3-batch", 0)

      ; If S2 defines no batch RNG domain, the S2 assertion is replaced by
      ; the complete list of S2 seed128 training domains.

O11 Promotion gate referential transparency
    s4_promote (operation in §8) is a pure function of its inputs under
    S1CanonicalJson. A unit test feeds two byte-identical input bundles
    and asserts byte-identical PromotionGateProduct output.

O12 Promotion gate predicate matrix
    H3 is not established by the implementation under test evaluating
    itself. gbf-experiments must provide a small, independent
    PromotionGateReferenceEvaluator that computes P-1..P-8 from parsed
    artifacts without calling the production s4_promote implementation.

    Required tests:
      1. canonical positive bundle:
           reference = Promoted and production = Promoted.
      2. for every k in {P1, P2, P3, P4, P5, P6, P7, P8}:
           construct exactly one minimally-broken bundle;
           reference rejects with reason including Pk;
           production rejects with reason including Pk;
           production must not emit Promoted.
      3. multi-failure bundle:
           reference rejection reason set equals production rejection
           reason set for every evaluable predicate.

    A failure of this matrix Refutes H3 even if F3-broken also fails as
    expected.

O13 Corpus-progression schedule snapshot
    The single edge {from: TinyStories, to: Gutenberg, gate:
    G_TS->Gutenberg} is recorded in s4_report.v1 front-matter and is
    consistent with the CorpusProgressionSchedule enum value used by
    gbf-train at run time. Mismatch is a halt-class failure.

O14 Closure gate
    bd-2hmm close is reachable iff Decision in
    {ProceedToS5, ProceedToS5-with-contamination-warning}.
```

---

# 16. Minimal end-to-end theorem

```text
Theorem S4Soundness:

Given:
  TinyStories manifest with valid sha256 pinned by S3
  Gutenberg manifest pinned in fixtures/corpora/gutenberg.toml
  c_TS ternary checkpoint passed S3 v0_success and three-way oracle
    agreement
  Toy0 reference instance (T14.1 closed, bd-1r6k)
  S4TrainConfig pinned per D9 + D10
  KN_params and v0_success workload manifest pinned by S3
  pass_version V_S4 fixed by gbf-train HEAD at S4 PR merge

If:
  s4_build_gutenberg_manifest(...)       returns a manifest that
                                            satisfies G-Ok-1..G-Ok-12
  s4_fit_kn5_gutenberg(...)              returns finite bpc_kn5
  s4_cross_corpus_contamination(...)     returns Clean | Warn(_)
  s4_promote(...)                        returns Promoted(...)
  COr-1..COr-6                           all pass
  And for every seed s in {0, 1, 2, 3, 4}:
    s4_gutenberg_train_run(...)           returns Completed RunProduct
    s4_score_bpc(...)                     returns finite val_bpc on
                                           gutenberg_val
    s4_v0_success_gutenberg(...)          returns S4V0SuccessProduct
                                           with pass = true
  And for seed 0 specifically:
    three_way_oracle_agreement(c_GB(0))   returns outcome = Agree under
                                           S3-pinned tolerances
  And:
    replay under D16 for every seed in {0,1,2,3,4}
                                          reproduces bit-identical
                                          canonical-tensor payloads and
                                          bit-identical s4_*.v1 self-hashes
  And:
    s4_report.v1                          contains pre-registered
                                           predictions in pre-run git
                                           history.

Then:
  Each of H1, H2, H3, H4, H5, H6 has a defined status. In pass outcomes,
  each is Confirmed. In early-failure outcomes, downstream hypotheses may be
  NotEvaluatedDueToPriorGate(reason) only when the §11 dispatcher has already
  selected the earlier failure branch.
  H7 has a defined HypothesisStatus. In pass outcomes, and in any failure
  outcome where c_TS and all five c_GB(s) Gutenberg scores exist, H7 is
  Confirmed or Refuted. In early-failure outcomes that do not produce the
  required Gutenberg scores, H7 is
  NotEvaluatedDueToPriorGate(reason).

  S4Outcome is exactly one of:
    PassClean
    PassWithContaminationWarning
    Fail-corpus-integrity
    Fail-contamination
    Fail-promotion-gate
    Fail-promotion-gate-readiness
    Fail-quality-on-gutenberg
    Fail-oracle-disagreement
    Fail-substrate
    Fail-suspicious

  Decision is unique under the dispatch rule of §11.

  If S4Outcome in {PassClean, PassWithContaminationWarning}, S4 has
  produced these verified knowledge claims:
    - The Gutenberg manifest, header/footer stripper, charset_v1
      normalization on Gutenberg, document-level book-split rule, and
      cross-corpus contamination check are deterministic and round-trip
      under S1CanonicalJson + canonical replay.
    - The promotion gate G_TS->Gutenberg implements D8 exactly over the
      tested input space: it accepted the canonical c_TS_ref only because
      P-1..P-8 held, and it rejected the deliberately-broken variants
      under §15 O5 with the expected reasons. (Implementation soundness
      over the tested inputs, not a formal proof over all malformed
      bundles.)
    - Continuation training from c_TS under D9 (warm-weight, cold-
      optimizer) on gutenberg_train under the S4TrainConfig step budget
      produces a ternary checkpoint that beats the gutenberg KN-5
      baseline by > 0.05 bpc and passes v0_success on gutenberg_val,
      for every one of the five seeds.
    - The three-way oracle agreement contract from S3 holds across the
      corpus switch on seed 0.
    - The pipeline replays bit-identically under D16.

  If S4Outcome = PassWithContaminationWarning, S4 additionally
  verifies that 13-gram cross-corpus overlap is in (0.0005, 0.0010] in
  one or more directions and that S5 inherits this warning explicitly.

  If S4Outcome = Fail-corpus-integrity, the Gutenberg corpus is
  structurally broken and no quality, oracle, or promotion claim is
  licensed.

  If S4Outcome = Fail-contamination, either train/val splits leak across
  corpora or the contamination measurement oracle is invalid. In both cases,
  v0_success-on-Gutenberg numbers are not interpretable.

  If S4Outcome = Fail-promotion-gate, the promotion gate implementation
  is unsound; no later corpus transition can be trusted until it is fixed.

  If S4Outcome = Fail-promotion-gate-readiness, the gate implementation
  is sound (H3 Confirmed) but the canonical c_TS_ref does not currently
  satisfy D8; the experiment was successfully falsified at the readiness
  layer, not at the gate-soundness layer.

  If S4Outcome = Fail-quality-on-gutenberg, Toy0 + the S3 ternary
  continuation contract under D9 + D10 does not transfer to Gutenberg
  under the pinned step budget; it does not verify Toy1 sufficiency
  or that a different optimizer-state rule would succeed.

  If S4Outcome = Fail-oracle-disagreement, switching the corpus broke
  the three-way oracle agreement contract; the gap must be localized
  before any later slice proceeds.

  If S4Outcome = Fail-substrate, the Gutenberg training substrate or
  the corpus loader failed; no downstream quality, gate, or oracle
  claim is licensed.

  If S4Outcome = Fail-suspicious, the suspicious-low-bpc sentinel
  fired and split/leakage/metric audit is required.

Not proven:
  UpperBankCandidate production-scale quality on Gutenberg (S8)
  gutenberg_manifest.v2 test-partition correctness (S8)
  BoundedKv vs LinearState A/B (S5)
  RuntimeChromeBudget end-to-end (S6)
  shadow_compile / EncodedRom path (S6)
  emulator harness end-to-end (S6)
  MoE / router (S7)
  Toy1 sufficiency on Gutenberg (no Toy1 was trained)
  Cross-machine determinism (Rep-2)
```

---

# 17. Implementation crate layout

Scope(F-S4) is hosted in the existing `gbf-experiments` workspace crate
created at S1, plus extensions to `gbf-data` for the Gutenberg loader and
the cross-corpus contamination check. This section pins the public surface
that the hypotheses and proof obligations rely on. Module names within
each crate are illustrative; only items tagged **Required** are normative.

## 17.1 Crate map

```text
gbf-policy
  Required  ModelSizeProfile::Toy0 reference instance (T14.1, bd-1r6k).
            Inherited unchanged from S1.

gbf-model
  Required  LinearStateBlock with Fixed(0.5) decay (bd-tnb closed).
            S4 does not amend this contract.
  Required  Ternary projection (S2) consumed unchanged.

gbf-train
  Required  Phase scheduler with Phase::D semantics for continuation
            training. F4 closed.
  Required  AdamW config helper exposing the D10 hyperparameters
            { lr=5e-4, beta1=0.9, beta2=0.999, eps=1e-8,
              weight_decay=0.0 } as constants. (Distinct from S1's
              AdamW1e-3 instance; both must coexist as named constants.)
  Required  CorpusProgressionSchedule type with the S4 instance
            pinned: edges = [{TinyStories -> Gutenberg, gate:
            "G_TS->Gutenberg"}]. Per-edge gate functions are provided
            by gbf-experiments.
  Required  ZeroInitAdamW helper (D9 cold-optimizer rule).
  Required  Burn backend aliases inherited from S1 (`burn-adapter`).
  Required  Cargo features `qat` (default-on) and `qat-ablation`
            inherited from S1.

gbf-data
  Required  GutenbergManifest canonical JSON reader and Gutenberg loader
            (D1-D5).
  Required  Header/footer stripper with the D3 regex pair pinned.
  Required  Document-level split with the D2 split_seed pinned.
  Required  Cross-corpus contamination check (§7) with the D6 n=13
            and fingerprint_kind = "sha256_high_u64" pinned.
  Required  Canonical Gutenberg manifest path:
              fixtures/corpora/gutenberg.toml
            at repository root. Shared across S4..S8 experiments.
  Required  charset_v1 normalization plumbing inherited from S3 / F-G2.
            Re-exported under stable identifiers; S4 does not amend.

gbf-foundation
  Required  Hash256, sha256 helper inherited from S1.

gbf-artifact
  Required  CanonicalTensor, CanonicalTensorPayloadHash inherited from
            S1; consumed by H6 replay determinism.
  Required  ReferenceModelBundle re-export over c_GB(0) consumed
            unchanged from S3.

gbf-oracle
  Required  DenotationalOracle, ArtifactOracle inherited unchanged from
            S3. The three-way agreement runner accepts a corpus-id
            parameter and binds the workload to gutenberg_val for S4.

gbf-experiments  (existing workspace crate; new s4 module)
  Owns Scope(F-S4) end-to-end. Required modules under
  gbf_experiments::s4:

    gbf_experiments::s4::manifest
      S4 manifest validation wrapper; delegates canonical JSON reading to
      gbf-data and asserts manifest sha256 verification before bytes flow.

    gbf_experiments::s4::rng
      Pcg64Mcg, seed128 with domain prefix "s4-...", InitRng/BatchRng/
      ShuffleRng for Gutenberg continuation training.

      D2 book splitting is not RNG-based; it is a stateless SHA-256
      assignment rule and must not use BookSplitRng.

    gbf_experiments::s4::device_profile
      Re-exports S1CpuDeterministic. S4 does not amend it.

    gbf_experiments::s4::corpus_quality
      Emits s4_corpus_quality.v1 over both manifests.

    gbf_experiments::s4::contamination
      s4_cross_corpus_contamination operation per §7.

    gbf_experiments::s4::baseline
      s4_fit_kn5_gutenberg operation per §6.

    gbf_experiments::s4::promote
      s4_promote operation per §8.

    gbf_experiments::s4::run
      s4_gutenberg_train_run operation per §9.

    gbf_experiments::s4::score
      s4_score_bpc bound to gutenberg_val and s4_v0_success_gutenberg
      per §10.

    gbf_experiments::s4::oracle
      Three-way agreement runner bound to gutenberg_val for seed 0;
      consumes gbf-oracle types unchanged.

    gbf_experiments::s4::corpus_oracle
      COr-1..COr-6 fixtures. Deterministic, model-free.

    gbf_experiments::s4::schema
      Type definitions, S1CanonicalJson encoder reuse, DomainHash
      function with S4 schema_id strings, and self-hash round-trip
      helpers for:
        s4_gutenberg_checkpoint.v1, s4_gutenberg_run_log.v1,
        s4_gutenberg_score.v1, s4_baseline_gutenberg.v1,
        s4_corpus_quality.v1, s4_contamination_report.v1,
        s4_promotion_gate.v1, s4_oracle_agreement.v1,
        gutenberg_manifest.v1, s4_report.v1.

    gbf_experiments::s4::report
      s4_report.v1 emitter and outcome-algebra dispatcher implementing
      §11. Authors front-matter, validates S4-R-Decision, S4-R-AllSeeds,
      S4-R-Self-Hash, S4-R-Predictions, S4-R-AllHypotheses, S4-R-
      ClosureArtifacts, and binds the pre-registration commit history
      per O1.

    gbf_experiments::s4::cli
      Public entrypoint(s) for replay. The CLI surface is the canonical
      invocation point referenced by §13 Rep-1 and §14 closure.

gbf-cli
  Required  Subcommand `gbf s4 ...` dispatching into
            gbf_experiments::s4::cli. The pre-registration check, the
            determinism check, the corpus-build check, the promotion-
            gate check, and the closure script all shell into this
            surface.
```

## 17.2 Test layout

```text
gbf-experiments/tests/s4_falsification.rs
gbf-experiments/tests/s4_falsification/*.rs
  Root harness plus six module files required by §15 O5; gated by the
  test-only `falsify` feature so broken substitutes cannot leak into
  release builds.

gbf-experiments/tests/s4_corpus_oracle.rs
gbf-experiments/tests/s4_corpus_oracle/*.rs
  COr-1..COr-6 executed deterministically without a
  trained model.

gbf-experiments/tests/s4_canonical_json.rs
gbf-experiments/tests/s4_canonical_json/*.rs
  Round-trip tests for every s4_*.v1 schema (O6) and gutenberg_manifest.v1.
  Each artifact must serialize, hash, deserialize, re-serialize, re-hash,
  and produce byte-identical output and self-hash equality.

gbf-experiments/tests/s4_integration.rs
gbf-experiments/tests/s4_integration/*.rs
  End-to-end smoke run against a tiny in-repo Gutenberg-shaped fixture
  corpus (NOT the real 1500-book Gutenberg slice) and a tiny in-repo
  TinyStories-shaped fixture. Used in CI to gate determinism (O2),
  per-seed isolation (O9), cross-slice rng isolation (O10), and promotion
  gate referential transparency (O11). Sized so a 5-seed run completes
  within the project's standard test timeout.

  The full Gutenberg run is gated behind a separate CI job, but bd-2hmm
  closure requires that job's artifacts and s4_report.v1, not merely the
  tiny-fixture smoke run.

gbf-data/tests/gutenberg_loader.rs
gbf-data/tests/gutenberg_split.rs
gbf-data/tests/gutenberg_stripper.rs
  Loader, splitter, and stripper unit tests under the gbf-data crate
  (the tests live in the crate that owns the contract).
```

## 17.3 Artifact paths

Unchanged from §12. All run artifacts are written under the repository-
root `experiments/S4/` tree. The report is written to
`docs/experiments/S4-report.md`. The Gutenberg manifest fixture lives
at `fixtures/corpora/gutenberg.toml`.

## 17.4 Canonical replay command

```text
cargo run --release -p gbf-cli -- s4 replay \
  --ts-manifest fixtures/corpora/tinystories.toml \
  --gb-fixture fixtures/corpora/gutenberg.toml \
  --gb-manifest experiments/S4/corpus/gutenberg-manifest.json \
  --c-ts-checkpoint experiments/S3/checkpoints/seed-0/checkpoint.safetensors \
  --pass-version <pass_version_pinned_in_report> \
  --seed-list 0,1,2,3,4 \
  --device-profile S1CpuDeterministic
```

Under the same machine + OS + pinned Burn version + pinned dependency
lockfile + S1CpuDeterministic, this command reproduces
`experiments/S4/**` byte-for-byte per Rep-1 and Rep-2.

Optional non-normative subcommands:

```text
gbf s4 harvest-gutenberg-fixture
                               network-permitted fixture harvest only
gbf s4 build-corpus           network-disabled s4_build_gutenberg_manifest only
gbf s4 fit-baseline-gutenberg runs s4_fit_kn5_gutenberg only
gbf s4 contamination          runs s4_cross_corpus_contamination only
gbf s4 promote                runs s4_promote only
gbf s4 oracle                 runs the COr corpus-oracle suite
gbf s4 verify-determinism     replays seed 0 and asserts byte equality
```

## 17.5 Workspace registration

Cargo.toml workspace `members` already includes `gbf-experiments` (S1).
The `gbf-experiments` crate's Cargo.toml gains a workspace dependency on
`gbf-oracle` (consumed at S3) if not already present. `gbf-data` may gain
pinned workspace dependencies required to implement the Gutenberg loader
and fixture builder, including archive decompression, zip
central-directory inspection, URL canonicalization, RDF/XML parsing, and
explicit charset decoding. All such dependencies are pinned by the
workspace lockfile and included in dependency_lockfile_sha. The Gutenberg
loader uses the same charset_v1 plumbing as S3 after source decoding.

---

# 18. Build configurations and feature flags

Two build configurations participate in the S4 contract; both reuse
S1's feature topology and add no new features.

## 18.1 S4-build-A — "Phase D continuation"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments
Active features (workspace-resolved):
  gbf-experiments/default
  gbf-experiments/phase-a            ; S1 feature, still default-on for
                                       ; the binary; selects the QAT
                                       ; codepath set, not the runtime
                                       ; phase. Phase::D is selected at
                                       ; runtime by S4TrainConfig.

gbf-experiments/default expands to:
  gbf-experiments/phase-a

gbf-experiments/phase-a expands to:
  gbf-train/qat
  gbf-train/burn-adapter

Behavior:
  QAT codepaths are present in the binary and configured at Phase::D.
  This build produces the five seeded Gutenberg checkpoints used for
  H1..H6 verdicts.
Build identity tag (recorded in s4_gutenberg_checkpoint.v1.metadata):
  build_kind = "phase_d_continuation"
```

## 18.2 S4-build-B — "Ablation"

```text
Cargo invocation:
  cargo build --release -p gbf-experiments \
    --no-default-features \
    --features ablation
Active features:
  gbf-experiments/ablation
  gbf-train/qat-ablation
  gbf-train/burn-adapter
Behavior:
  S4 does not require an ablation checkpoint comparable to S1's H4
  (Phase A vs ablation). S4 inherits S2's Phase A->D bit-identicality
  contract and does not re-test it. S4-build-B exists only to verify
  that the Gutenberg corpus loader, manifest reader, header/footer
  stripper, charset_v1 normalizer, contamination check, and promotion
  gate are all reachable from a build without QAT codepaths — i.e.
  that gbf-data and gbf-experiments::s4 do not silently leak QAT
  symbols into the corpus stack.
Build identity:
  No S4 scientific artifact emitted under S4-build-B participates in closure.
  The ablation build is a compile/reachability check only.
```

## 18.3 Feature flag contract

```text
gbf-train/qat              default-on; gates all QAT codepaths.
                           Inherited from S1. S4 does not amend.
gbf-train/qat-ablation     mutually exclusive with `qat`; replaces QAT
                           codepaths with stubs. Inherited from S1.
gbf-experiments/phase-a    forwards to gbf-train/qat and
                           gbf-train/burn-adapter. Inherited from S1.
gbf-experiments/ablation   forwards to gbf-train/qat-ablation and
                           gbf-train/burn-adapter. Inherited from S1.
gbf-experiments/falsify    test-only; gates the F1..F6 broken
                           substitutes used by the S4 falsification
                           suite (in addition to S1..S3's broken
                           substitutes; the feature is a single flag,
                           the test files are slice-namespaced).

Mutual exclusion enforcement:
  gbf-train must compile_error! at the crate root when both `qat` and
  `qat-ablation` are enabled. Inherited from S1.

S4 introduces no new features.
```

## 18.4 Determinism budgets

```text
Both builds run under S1CpuDeterministic (§5). The runner sets each
variable in env_exact to its pinned value, and unsets every variable
not present in env_exact (env_forbidden_unless_listed = true), before
any tensor allocation:

  BURN_NDARRAY_NUM_THREADS=1
  BURN_DETERMINISTIC=1
  OMP_NUM_THREADS=1
  RAYON_NUM_THREADS=1

Violation — any unset env_exact entry or any value mismatch — aborts
the run with a non-zero exit before training begins.

The runner must spawn the semantic S4 subprocess with a sanitized
environment. Variables outside env_exact may be present only if they are
listed in S1's nonsemantic process-launch allowlist. S4 code must not
read, branch on, hash, or serialize those nonsemantic variables. This
avoids making normal process-launch requirements such as dynamic loader
paths or temporary-directory plumbing part of the scientific contract.

The Gutenberg fixture-harvest CLI op (`gbf s4 harvest-gutenberg-fixture`)
is the only S4 op that may bypass network=false. It runs under a relaxed
network profile and writes:

  - fixtures/corpora/gutenberg.toml,
  - the content-addressed source-blob cache,
  - catalog_snapshot_sha256 and per-source source_blob_sha256 pins.

The manifest-build op (`gbf s4 build-corpus`) is network-disabled. It
consumes only the pinned catalog snapshot, fixtures/corpora/gutenberg.toml,
and the content-addressed source-blob cache. Replay is also
network-disabled.
```

## 18.5 Pre-registration CI

```text
scripts/s4_preregistration_check.sh implements §15 O1:
  1. predictions_section_hash matches the exact markdown bytes of the
     "Pre-registered predictions" section in predictions_commit, after
     normalizing line endings to LF and excluding surrounding markdown
     sections. S1CanonicalJson is not applied to markdown body text.
  2. predictions_commit is a strict ancestor of first_result_commit;
  3. first_result_commit is the earliest commit introducing any
     gutenberg_manifest_self_hash, checkpoint_self_hash,
     run_log_self_hash, score_self_hash, fp_reference_self_hash,
     oracle_agreement_self_hash, corpus_quality_self_hash,
     corpus_progression_self_hash, contamination_self_hash,
     promotion_gate_self_hash, or baseline_gutenberg_self_hash derived
     from S4 execution.
Exit non-zero on any violation. Closure of bd-2hmm is forbidden while
this script exits non-zero.
```

## 18.6 CI gates that block bd-2hmm closure

```text
cargo test -p gbf-experiments
cargo test -p gbf-experiments --features falsify --test s4_falsification
cargo test -p gbf-experiments --test s4_corpus_oracle
cargo test -p gbf-experiments --test s4_canonical_json
cargo test -p gbf-experiments --test s4_integration
cargo test -p gbf-data --test gutenberg_loader
cargo test -p gbf-data --test gutenberg_split
cargo test -p gbf-data --test gutenberg_stripper
cargo build -p gbf-experiments --no-default-features --features ablation
scripts/s4_preregistration_check.sh
scripts/s4_determinism_check.sh
  (replays seed 0 and asserts byte equality of safetensors and
   run_log_self_hash; satisfies the O2 CI smoke test only)
scripts/s4_full_replay_check.sh
  (replays all five seeds and asserts byte equality of canonical-tensor
   payloads plus all report-pinned s4_*.v1 self-hashes; required for
   bd-2hmm closure and H6 Confirmed)
scripts/s4_isolation_check.sh
  (asserts seed 0 and seed 1 produce different first-1024 BatchRng offset
   traces under the same corpus length and train_config, that seed-pairs
   [0,1] and [1,0] produce identical per-seed hashes, and that
   seed128("s4-init-batch", 0) is not equal to seed128("s1-batch", 0)
   or seed128("s3-batch", 0); satisfies O9 and O10)
scripts/s4_promotion_gate_check.sh
  (runs s4_promote against the S3 ternary checkpoint twice with
   byte-identical input bundles and asserts byte-identical
   PromotionGateProduct; also runs the independent P1..P8 predicate
   matrix from O12; satisfies O11 and O12)
```

---

# 19. Ambiguity ledger

|  ID | Ambiguity                                                                           | Chosen path                                                                                                                | Clarifying question                                                                                                         | Suggested final decision                                                                                                                                                                                                                                                |
| --: | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
|  A1 | Initial weights for Gutenberg training: c_TS, fp teacher, or fresh init             | c_TS ternary checkpoint (D9)                                                                                               | Why not fp teacher?                                                                                                         | The bead claims a *promotion* gate; promoting an fp teacher into Gutenberg training would re-test S2's ternarization, not corpus progression. c_TS is the only choice that isolates the corpus axis.                                                                    |
|  A2 | Optimizer state across corpus switch: warm or cold                                  | Cold (ZeroInitAdamW) (D9)                                                                                                  | Should AdamW momentum carry from TS to Gutenberg?                                                                           | No. AdamW momentum is gradient-distribution-specific. Carrying TS momentum biases early Gutenberg updates and silently couples the slices, defeating cross-slice replay isolation (Rep-8).                                                                              |
|  A3 | Gutenberg slice provenance                                                          | 1500-book deterministic English public-domain plaintext slice from the pinned Project Gutenberg catalog snapshot (D1)      | Why not the full Gutenberg catalog?                                                                                         | Full catalog is too broad for this slice. S4 pins a deterministic English public-domain plaintext subset by catalog snapshot, filter, rank hash, and sorted book id list. S4 intentionally does not filter on original publication year unless a separate hash-pinned publication-year table is added. |
|  A4 | Document split granularity: per-book vs per-chapter vs random byte                  | Per-book (D2)                                                                                                              | Why not per-chapter?                                                                                                        | Books are the natural authorship unit. Per-chapter splits leak topical n-grams across train/val. Per-byte splits leak in-sentence context. Per-book is the standard discipline for Gutenberg evaluations.                                                               |
|  A5 | Header/footer stripping: regex vs heuristic                                         | Pinned regex pair (D3)                                                                                                     | What about books that diverge from the standard banner?                                                                     | They drop with reason `gutenberg_marker_missing`, capped at 5% of total. Higher drop rates indicate a corrupted catalog snapshot, not an under-tuned regex.                                                                                                             |
|  A6 | Per-corpus unmappable bound for Gutenberg                                           | <=0.5% (D5), tighter than F-G2 default                                                                                     | F-G2 defaults to <2%; why is Gutenberg stricter?                                                                            | Pre-1928 English ebooks are ASCII-dominant; >0.5% post-charset_v1 unmappable rate implies a charset-normalization bug or a wrong filter, not a real text property.                                                                                                      |
|  A7 | Contamination n-gram window size                                                    | n=13 (D6)                                                                                                                  | Why not n=8 or n=20?                                                                                                        | n<8 is dominated by common short idioms; n>20 misses real passage reuse if the inadvertent inclusion is short. n=13 is a defensible mid-range; sensitivity to n is an explicit ambiguity for review.                                                                    |
|  A8 | Contamination overlap thresholds                                                    | Hard fail >0.10% of sampled val n-grams; warn >0.05%                                                                       | Why these specific numbers?                                                                                                 | They are scaled to the 1 MiB sample cap (D6) and the n=13 window: 0.10% of ~1M unique 13-grams is about 1000 overlapping unique fingerprints, which is enough to indicate passage-level reuse. These thresholds are normative once the prediction-bearing RFC revision is committed; if they remain estimates, D6 must mark them `[ESTIMATE]` and they must be resolved before first_result_commit. |
|  A9 | Step budget for Gutenberg training                                                  | 20000 (D10)                                                                                                                | Why double the S1 budget?                                                                                                   | Gutenberg train byte stream is ~40x larger than TinyStories train. 2x steps with the same batch size is a conservative balance between coverage and CI time. `[ESTIMATE for review]`.                                                                                   |
| A10 | Learning rate for Gutenberg training                                                | 5e-4, half of S1 (D10)                                                                                                     | Why halve LR?                                                                                                               | AdamW state was reset (D9). Hot LR on cold momentum is the classical recipe for early divergence. Halving the LR is the standard workaround. `[ESTIMATE for review]`.                                                                                                   |
| A11 | Phase for Gutenberg continuation                                                    | Phase::D (fully hardened QAT) (D9)                                                                                         | Why not re-run Phase A->D from scratch?                                                                                     | The point of "continuation" is that ternarization is preserved across the corpus switch; re-running phase warmup would re-test S2's contract, not S4's. Phase::D is a strict consumer of S2's closed contract.                                                          |
| A12 | Ablation comparison required for S4                                                 | Not required at quality level; only smoke-build under `--features ablation`                                                | Should we replicate S1's H4 phase-A vs ablation byte-equality test for S4?                                                  | No. S2's closure already proved the Phase A->D bit-equality contract in the relevant sense. S4-build-B exists only to prove the corpus stack does not silently leak QAT symbols.                                                                                        |
| A13 | Promotion gate as CLI op vs implicit guard                                          | Explicit CLI op (`gbf s4 promote`)                                                                                         | Why not just guard at training entry?                                                                                       | An explicit CLI op produces a self-hashed artifact (s4_promotion_gate.v1) that downstream artifacts bind to, and that the falsification suite can target. An implicit guard provides no record.                                                                         |
| A14 | Three-way oracle agreement on multiple seeds vs seed 0 only                         | Seed 0 only (mandatory); seeds 1..4 observational                                                                          | Should all five seeds be required?                                                                                          | No for v1. Per-seed oracle re-export is expensive and S3 already established the contract. Future tightening is a follow-up bead.                                                                                                                                       |
| A15 | H7 (distribution-shift sanity) as closure gate vs observational                     | Observational                                                                                                              | Should we require Gutenberg-trained checkpoint to beat TS-only checkpoint on Gutenberg val?                                 | Not for closure. H4 already requires Gutenberg-trained to beat KN-5; H7 is an additional sanity check that would over-constrain the gate if mandatory. A Refuted H7 with Confirmed H4 is a useful surprise that informs S5 schedule.                                    |
| A16 | s4_promotion_gate.v1 lineage edge to s3_v0_success.v1                               | Bind via c_TS_checkpoint_self_hash + reading the s3_v0_success acceptance bits                                             | Should the promotion gate copy the v0_success report inline?                                                                | No. Bind by self-hash; copying inline duplicates state and creates drift risk. The promotion-gate evaluator reads the live s3_v0_success.v1 artifact and asserts its self-hash matches.                                                                                 |
| A17 | Cross-corpus rng leakage between S3 and S4                                          | Disjoint by domain string ("s4-..." vs "s3-..."); enforced by O10 lint                                                     | Should S4 inherit any S3 rng state?                                                                                         | No. Rep-8 is explicit. Even if seeds match, the seed128 domain prefix differs.                                                                                                                                                                                          |
| A18 | Network access for Gutenberg fetch                                                  | Permitted only during the explicit fixture-build CLI op                                                                    | Should the trainer fetch books on demand?                                                                                   | No. On-demand fetch breaks reproducibility and S1CpuDeterministic. Fetching is a one-time fixture-build step; replay reads from the content-addressed mirror.                                                                                                           |
| A19 | Catalog snapshot freshness                                                          | One-shot snapshot dated 2026-05-09 (D1)                                                                                    | What if Gutenberg catalog changes after snapshot?                                                                           | The snapshot is hash-pinned. Catalog drift produces a different snapshot sha256 and constitutes a different experiment requiring an RFC amendment per Rule Amendment.                                                                                                   |
| A20 | Whether S4 closure depends on S5 / S6 readiness                                     | No                                                                                                                         | Should we hold S4 closure until shadow_compile / RuntimeChromeBudget land?                                                  | No. v0_success on Gutenberg uses S3-stub forms of the runtime_chrome_budget and emulator-smoke acceptance bits. S4 inherits S3's stub semantics; tightening those bits is S6's job.                                                                                     |
| A21 | What if the Gutenberg KN-5 baseline bpc is *worse* than the TinyStories KN-5 bpc    | Report it; do not auto-fail                                                                                                | Should H4's margin be normalized?                                                                                           | No. H4's gate is `bpc_ternary < bpc_kn5_gutenberg - 0.05`, computed on Gutenberg numbers only. Cross-corpus bpc comparisons are not meaningful (different vocab usage distributions); ranges in §3 are sanity only.                                                      |
| A22 | What if a single book straddles 5% of corpus mass                                   | Allow it; record in s4_corpus_quality.v1                                                                                   | Should one giant book skew the eval?                                                                                        | Allow. Per-book fairness is not an S4 commitment; the per-book split rule is the only invariant. Future tightening (per-book downweighting) is out of scope.                                                                                                            |
| A23 | What if F-G2 charset_v1 spec is amended after S4 RFC commit                         | Amendment is a new experiment per Rule Amendment                                                                           | Could a charset_v1 patch invalidate S4 results?                                                                             | Yes by design. normalization_spec_self_hash is part of gutenberg_manifest.v1; a spec amendment changes the hash and invalidates the manifest.                                                                                                                           |
| A24 | Whether dedup is global or per-corpus                                               | Per-book within Gutenberg only; no cross-corpus dedup                                                                      | Should we dedup TinyStories against Gutenberg?                                                                              | No. Cross-corpus duplicates are caught by the contamination check (§7), which is a stronger contract than dedup. Dedup is per-corpus identity; contamination is cross-corpus leak.                                                                                      |
| A25 | Whether to extend bd-1lin (F16) to close at S4                                      | Yes (amended 2026-05-17 when enwiki8 was dropped)                                                                          | Could S4 close F16?                                                                                                         | Yes. F16's original scope was the three-corpus progression TinyStories -> Gutenberg -> enwiki8. With enwiki8 (T16.3 / bd-59af) dropped, F16 reduces to TinyStories -> Gutenberg, both delivered by end-of-S4; F16 closes at S4. S8's production-scale run is a new-profile experiment on the existing Gutenberg manifest (with a v2 test-partition amendment), not a new corpus. |
| A26 | Whether s4_report.v1 should embed the full predictions section                      | Yes (front-matter `predictions_section_hash` + body section)                                                                | Why pin both a hash and the literal text?                                                                                   | Pre-registration provability requires both: the literal text proves what was predicted; the hash proves the text has not changed since predictions_commit.                                                                                                              |

---

# 20. Final concise contract

```text
F-S4 Promote to Gutenberg is correct when:

1.  A hash-pinned Project Gutenberg English-public-domain-in-USA slice is built
    deterministically: header/footer stripped per the pinned regex pair,
    charset_v1-normalized, document-split at the book level under a
    pinned SHA-256 split seed, with manifest_self_hash, train_sha256, val_sha256,
    and unmappable_rate_corpus all round-tripping under canonical JSON
    + canonical replay.

2.  The cross-corpus 13-token contamination report between TinyStories
    and Gutenberg is exact for the two gated directions
    TS_train_contains_GB_val and GB_train_contains_TS_val. Diagnostic
    directions may be sampled, but Clean/Warn/HardFail for promotion is
    based on full validation splits against full opposite training splits.
    HardFail blocks closure.

3.  The promotion gate G_TS->Gutenberg accepts the S3 ternary checkpoint
    c_TS exactly when D8 P-1..P-8 all hold; the gate is sound under the
    six deliberately-broken substitutes of §15 O5.

4.  Continuation training from c_TS under D9 (warm weights, cold AdamW
    state, warm QAT shadow weights, Phase::D) on gutenberg_train under
    D10 (20000 steps, lr=5e-4) completes for every one of seeds
    {0, 1, 2, 3, 4} without divergence.

5.  Every seed's Gutenberg-trained ternary checkpoint c_GB(s) beats the
    Gutenberg KN-5 baseline by more than 0.05 bpc and passes v0_success
    on gutenberg_val.

6.  Three-way oracle agreement (live training, ReferenceModelBundle /
    DenotationalOracle, ArtifactOracle) holds for seed 0 on the S3-pinned
    conformance fixture set evaluated against gutenberg_val, within S3-
    pinned tolerance.

7.  s4_report.v1 emits pre-registered predictions in git history strictly
    before the first checkpoint commit, and concludes with exactly one
    Decision value chosen by §11 dispatch.

8.  Decision is one of {ProceedToS5, ProceedToS5-with-contamination-
    warning}; any other Decision blocks bd-2hmm closure.

9.  Every JSON artifact (gutenberg_manifest, s4_corpus_quality,
    s4_contamination_report, s4_promotion_gate, s4_baseline_gutenberg,
    s4_gutenberg_checkpoint metadata, s4_gutenberg_run_log,
    s4_gutenberg_score, s4_oracle_agreement, s4_report) is canonical,
    deterministic, and self-hash-valid. Binary blobs (Gutenberg
    train/val ID streams, KN-5 counts blob, checkpoint.safetensors)
    are bound by recorded Hash256 fields.

10. All six closure-gating hypotheses (H1..H6) have explicit verdicts
    with concrete observations cited; H7 (distribution-shift sanity) is
    reported observationally.

11. The six-test S4 falsification suite passes: deliberately-broken
    implementations produce the expected Refuted verdicts on the
    appropriate hypotheses.

12. Replay under D16 produces bit-identical Gutenberg checkpoints for
    every seed and bit-identical s4_*.v1 self-hashes for every artifact.

13. S4 retires corpus-progression risk only. It does not claim
    UpperBankCandidate production-scale readiness on Gutenberg (S8),
    gutenberg_manifest.v2 test-partition correctness (S8), BoundedKv
    vs LinearState A/B (S5), shadow_compile or EncodedRom soundness
    (S6), MoE benefit (S7), or any quality claim about a Toy1 /
    MoeTiny / UpperBankCandidate model — those are later slices'
    proof obligations.
```

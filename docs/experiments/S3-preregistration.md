# S3 Preregistration

This document records the F-S3 preregistration convention for
`history/rfcs/F-S3-v0-success-tinystories.md`.

The standalone pin lives at `experiments/S3/preregistration.toml`. It is
intentionally outside any future `gbf-experiments/src/s3` module tree so the
pre-result RFC pin can close before the larger S3 crate scaffolding lands.

## Pin Fields

- `schema`: fixed to `s3_preregistration.v1`.
- `predictions_commit`: the git commit containing the pinned F-S3 RFC text.
- `predictions_section_hash`: SHA-256 over the S1CanonicalJson-normalized
  markdown body of the RFC's `Pre-registered predictions` section.
- `pass_version_S3`: an S3-specific pass-version string. It is independent of
  S1 and S2 pass versions and must be bumped if the pinned predictions change.
- `rfc_revision`: the RFC revision commit used by this pin.
- `first_result_commit`: empty string until the first S3 result artifact lands.

The current pin uses RFC revision
`b4d9f0abcbec9140506288fd8344bcd16dab7479` and predictions hash
`sha256:77872aef2b0cb83523077015773a999ac713472e89673d22339d6441f520e95a`.

## Hash Convention

The hash mirrors `gbf_experiments::s1::report::predictions_section_hash`:

```text
sha256(S1CanonicalJson::to_vec(markdown.trim()))
```

For this RFC revision, the section body is the indented report-body contract
under `Required sections (markdown body):`, between `## Pre-registered
predictions` and `## Observed`. The heading itself is not part of the hash;
the body text and its internal line indentation are.

## Dry-Run Check

Before any S3 result artifact exists, run:

```bash
scripts/s3_preregistration_check.sh --dry-run
```

The dry-run checker validates that the TOML hash matches the RFC section, that
the RFC revision commit is still the latest commit for the RFC path, that
`first_result_commit` remains the empty sentinel, and that no S3 result
self-hash fields exist under `experiments/S3`.

The full live ancestry and earliest-result scan belongs to the later
F-S3.24/B24 preregistration checker. This B1 dry-run surface is deliberately
limited to the empty-result state so future S3 artifacts cannot be produced
before the predictions are pinned.

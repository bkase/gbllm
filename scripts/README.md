# Scripts

## S5 Feature Matrix

`scripts/s5_feature_matrix_check.sh` is the F-S5 closure feature gate. It
replaces `--all-features` for S5 closure builds because `s5-default`,
`s5-no-log`, and `s5-falsify-N` flags are mutually constrained.

Legal rows:

- `s5-default,qat,burn-adapter`
- `s5-no-log,qat,burn-adapter`
- `s5-default,qat,burn-adapter,s5-falsify-N` for exactly one `N` in `1..15`

Forbidden rows intentionally fail:

- `s5-default,s5-no-log,qat,burn-adapter`
- any pair of `s5-falsify-N` features in one build

Use `--dry-run` to inspect the full matrix without invoking Cargo, and
`--sample` for a local smoke subset of falsifiers `1,14,15`.

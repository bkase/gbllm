# Dependency Report

New workspace dependencies:

- `sha2 = 0.10.9` for `runtime_nucleus_hash`
- `static_assertions = 1.1.0` for ABI-size and address-contract assertions

Bundled review/runtime data sources:

- `tools/font/build_font.py` adapts the MIT-licensed `petabyt/font` 7x5 glyph table into the deterministic Game Boy 2bpp M0 font asset; the upstream license notice is carried in `tools/font/PETABYT_FONT_LICENSE.txt`.

F-A5 consumes F-A4 banking helpers and constants but does not alter `gbf-runtime/src/banking.rs` behavior.

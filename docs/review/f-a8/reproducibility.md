# Reproducibility Report

The session writer uses fixed `GBSE` magic, zero flags, deterministic serde JSON, and zstd level 3. Runtime host-duration metrics are opt-in and omitted from default envelopes.

Host triple: `aarch64-macos`

Generated tiny-ROM fixture evidence:

```
init stdout: {"command":"init","rom_sha256":"04dbcff2c0ce8f91f50ce1adf2166fbe163d7719f9400b11afe7f0a2885c4a79","session_path":"$TMP/gbf-debug-f-a8-review/s0.gbsess","session_sha256":"0a8a18aba62ed456db2a7723d623e412a79a6ae6d48ce0de9f038cb462e8fbff","symbol_count":2,"warnings":[]}
exec stdout: {"command":"exec","logs":[],"metrics":null,"parent_sha256":"0a8a18aba62ed456db2a7723d623e412a79a6ae6d48ce0de9f038cb462e8fbff","result":{"pc":257},"session_path":"$TMP/gbf-debug-f-a8-review/s1.gbsess","session_sha256":"10edfa263c8e5df2892d8e914dec0cef77cf4fb79fbdd56ce420b15f015780ae","warnings":[]}
s0.gbsess sha256: 0a8a18aba62ed456db2a7723d623e412a79a6ae6d48ce0de9f038cb462e8fbff
s1.gbsess sha256: 10edfa263c8e5df2892d8e914dec0cef77cf4fb79fbdd56ce420b15f015780ae
```

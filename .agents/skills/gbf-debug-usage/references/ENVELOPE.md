# gbf-debug Envelopes

`init` success:

```json
{"command":"init","session_path":"s0.gbsess","session_sha256":"...","rom_sha256":"...","symbol_count":2,"warnings":[]}
```

`exec` success:

```json
{"command":"exec","result":{},"logs":[],"session_path":"s1.gbsess","session_sha256":"...","parent_sha256":"...","warnings":[],"metrics":null}
```

`inspect` success includes schema, parent hash, ROM hash, registers, persisted breakpoints/watchpoints, trace summary, symbol summary, and metadata.

Failures emit `ErrorEnvelope` to stderr with `kind`, `message`, optional script position, and optional partial-session fields.


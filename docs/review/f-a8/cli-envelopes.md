# CLI Envelope Shapes

`init`, `exec`, and `inspect` write exactly one JSON object to stdout on success. Argument parse failures and runtime failures write exactly one `ErrorEnvelope` to stderr. `--help` writes one JSON help envelope to stdout with exit code 0.

Error envelope command names are `init`, `exec`, `inspect`, or `args` for parse/help failures. Deterministic runtime script failures include the parsed command name and normal `session_path`; timeout partial fields are only set when the user explicitly requested partial timeout writes.

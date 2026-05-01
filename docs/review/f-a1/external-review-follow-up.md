# External Review Follow-Up

PR3 was reviewed with Codex and Gemini. Accepted fixes applied after review:

- ROM assembly tracks occupied ROM bytes and rejects overlapping sections.
- ROM assembly rejects encoded byte lengths that disagree with placement size.
- ROM assembly rejects packages with no encoded section at the cartridge entry point `$0150`.
- Listing generation returns typed errors for missing, duplicate, extra, or out-of-bounds encoded spans.
- Program listings are emitted in placed ROM order.
- `verify-packet.sh` runs `git diff --check`.

Gemini reported no blocking correctness findings. Claude was launched twice through tmux with `claude --dangerously-skip-permissions`; both attempts produced no review output, and the second was bounded by `timeout 240`.

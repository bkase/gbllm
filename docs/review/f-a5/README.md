# F-A5 Bank0 Runtime Review Packet

Source of truth: `history/rfcs/F-A5-bank0-runtime.md`.

This packet covers the single F-A5 PR that closes the parent feature and tasks T-A5.1 through T-A5.7. It also records the upstream ABI/ASM additions required by the RFC:

- `gbf_abi::FaultCode::UiCommitQueueFull`
- `gbf_abi::RuntimeShellModule` and `RuntimeShellAnnotated`
- `gbf_asm::section::{ExecutionContext, InterruptDiscipline}` plus `panic_bypass`

Generated artifacts:

- demo ROM: `target/review/f-a5/demo_bank0_rom.gb`
- runtime nucleus hash: `2abeff40a74b7dc1e4a6572b44a145a7e8046d116e9886d0dd718521d14b0ec4`
- section sizes: `docs/review/f-a5/bank0-section-sizes.json`

Verify this packet with `scripts/review/f-a5/verify-packet.sh`.

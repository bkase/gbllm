# F-A5 Bank0 Runtime Review Packet

Source of truth: `history/rfcs/F-A5-bank0-runtime.md`.

This packet covers the single F-A5 PR that closes the parent feature and tasks T-A5.1 through T-A5.7. It also records the upstream ABI/ASM additions required by the RFC:

- `gbf_abi::FaultCode::UiCommitQueueFull`
- `gbf_abi::RuntimeShellModule` and `RuntimeShellAnnotated`
- `gbf_asm::section::{ExecutionContext, InterruptDiscipline}` plus `panic_bypass`

Generated artifacts:

- demo ROM: `target/review/f-a5/demo_bank0_rom.gb`
- demo screen PNG: `docs/review/f-a5/demo-screen.png`
- runtime nucleus hash: `ecc453b4abfe182a2463d35433df95953db32a47952da69be1fcf8b101b3b465`
- section sizes: `docs/review/f-a5/bank0-section-sizes.json`

![F-A5 demo screen](demo-screen.png)

Verify this packet with `scripts/review/f-a5/verify-packet.sh`.

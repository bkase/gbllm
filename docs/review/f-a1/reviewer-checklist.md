# Reviewer Checklist

- Run `./scripts/review/f-a1/verify-packet.sh`.
- Confirm `encode_section` has no unresolved-op path.
- Confirm ROM builder maps ROMX offsets with `cpu - $4000`.
- Confirm cartridge header bytes/checksums come from `rom.rs` and tests.
- Confirm `.sym` dot-safe escaping cannot collide.
- Confirm `tiny_rom` does not claim emulator boot validation.

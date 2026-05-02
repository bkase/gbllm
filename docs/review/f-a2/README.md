# F-A2 Review Packet

RFC: `history/rfcs/F-A2-gbf-hw.md`
Feature bead: `bd-3sk`
Child beads: `bd-7osa`, `bd-17x`, `bd-1yu`, `bd-121`, `bd-304`, `bd-e33`, `bd-21r`, `bd-xkp`

This packet covers the completed `gbf-hw` hardware contract:

```text
cartridge_header -> target -> memory
                         |-> mbc5
                         |-> lcd + timing
                         |-> interrupts
                         |-> joypad
                         |-> calibration
```

Highest-risk invariants: cartridge-header compatibility with F-A1, total memory classification, ISR residency versus I/O permission split, MBC5 canonical RAM-enable value, PPU accessibility table, interrupt priority order, joypad active-high post-decode state, and calibration serde validation.

Reviewer commands:

```bash
cargo test -p gbf-hw
cargo test -p gbf-asm
cargo clippy -p gbf-hw -- -D warnings
scripts/lints/no-hw-literal-redeclarations.py
scripts/review/f-a2/verify-packet.sh
```

The ignored `single_source_smoke::grep_no_redundant_constants` test is present for promotion to a future `gbf-test` workspace gate.

# Pan Docs Citations

| Surface | Pan Docs section |
|---|---|
| `NINTENDO_LOGO`, cartridge type, ROM size, RAM size, destination code | `The_Cartridge_Header.html#0104-0133--nintendo-logo`, `#0147--cartridge-type`, `#0148--rom-size`, `#0149--ram-size`, `#014a--destination-code` |
| Memory map constants | `Memory_Map.html#memory-map`, `#echo-ram`, `#fea0feff-range` |
| MBC5 register bands and RAM enable | `MBC5.html#registers`, `#0000-1fff---ram-enable-write-only` |
| PPU modes and accessibility | `Rendering.html#ppu-modes` |
| Interrupt vectors and IE/IF bits | `Interrupts.html#ffff--ie-interrupt-enable`, `#ff0f--if-interrupt-flag`, `#interrupt-priorities` |
| DIV/TIMA/TMA/TAC | `Timer_and_Divider_Registers.html#ff04--div-divider-register`, `#ff07--tac-timer-control` |
| JOYP select and button bits | `Joypad_Input.html#ff00--p1joyp-joypad` |

Fragment-level verification is executable through `scripts/review/f-a2/verify-pan-docs-fragments.py`.

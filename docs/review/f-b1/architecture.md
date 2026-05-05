# Architecture

The current streaming bringup path is:

`ComputeBringupRequest -> gbf-ir MatmulI8 -> one BankLease-lowered Bank0 streaming ROM -> gbf-emu tile-safe PC -> host DumpArena-shaped tile read -> one-instruction resume -> tile reassembly -> gbf-verify reference comparison -> realism report`.

Shared shape types live in `gbf-abi::compute_shape`. Production fixture and quarter-square helpers live in `gbf-codegen`; independent reference generation lives in `gbf-verify`. The emitted streaming ROM uses the checked `fixed_8_step_shift_add_i8_i32` multiply kernel, so report fields describe that kernel and record zero Bank0 table reads. The output path carries tile identity through the schedule and reassembles tiles by their recorded coordinates.

The existing F-A3 `DumpArena` opcode remains unchanged. The measured path builds
and decodes a `HarnessCommandBlock { op: DumpArena }` at each explicit
tile-safe PC, uses that command's arena address and length for the host payload
read, then resumes the same loaded ROM after each tile. Cooperative L4 evidence
is collected from PC traps at the ROM VBlank handler plus copy-yield and
compute-yield safe points, using the RFC fallback `KLaneRow` quantum. The
streaming ROM generates the handler section through
`gbf_runtime::interrupts::emit_vblank_handler` and installs the `$0040` jump
with typed instruction encoding after assembly; first-class vector placement is
follow-up `gbf-asm` work (`bd-18bu` and children) rather than an F-B1 closure
claim. The ROM records frame service at yield subroutines and does not add a new
harness opcode.

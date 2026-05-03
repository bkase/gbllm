# F-A4 Benchmark Notes

F-A4 pins byte and M-cycle counts rather than throughput benchmarks:

- Banking shadow zero init: 10 bytes / 14 M-cycles.
- ROM acquire under `ShortCriticalSection`: 20 bytes / 24 M-cycles.
- ROM acquire under `Disabled`: 18 bytes / 22 M-cycles.
- `AssertBank` default: label-only, zero instructions.
- `AssertBank` compare-and-trap opt-in: ROM check is 13 bytes ending in
  `RST $38`.

These claims are unit-gated by the focused banking tests and reproduced by the
packet verify script.


# API Guide

Primary entry points:

- `gbf_runtime::build_bank0_nucleus()`
- `gbf_runtime::build_bank0_nucleus_sections()`
- `gbf_runtime::runtime_nucleus_section_sizes()`
- `gbf_runtime::normalized_bank0_image_for_test()`
- `gbf_runtime::compute_runtime_nucleus_hash()`
- `gbf_runtime::scheduler::emit_yield_check(builder, kind, continue_label)`

Runtime sections carry both their `gbf_asm::section::Section` and `gbf_abi::RuntimeShellModule` annotation through `RuntimeNucleusSection`. Callers that need the interrupt-safety table should prefer `build_bank0_nucleus()` over taking bare sections.

Reject fixture: an explicit SRAM epoch must materialize a non-empty working
set. A range with no active SRAM page bindings is rejected as a policy
projection mismatch instead of producing an empty `SramWorkingSet`.

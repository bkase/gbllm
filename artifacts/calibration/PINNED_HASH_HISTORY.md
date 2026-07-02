# Pinned Runtime Hash History

Newest entries must be prepended; `scripts/check-nucleus-drift.sh` reads the first
backticked 64-hex hash as the active runtime nucleus pin.

| Date | Feature | runtime_nucleus_hash | Notes |
| --- | --- | --- | --- |
| 2026-05-03 | F-A5 Bank0 runtime | `ecc453b4abfe182a2463d35433df95953db32a47952da69be1fcf8b101b3b465` | Normalized linked Bank0 image, ABI version included, BuildIdentityBlock lineage hashes zeroed, CompileProfile excluded. The paired RuntimeChromeBudget contract is pinned in `artifacts/calibration/pinned_runtime_chrome_budget.json`. |

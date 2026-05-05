# F-B1 Review Scripts

| Script | When to run | Scope |
| --- | --- | --- |
| `smoke.sh` | During F-B1 iteration | Non-ignored F-B1 fast gates |
| `regen.sh` | Before PR review | Heavy N=128 gates and artifact regeneration |
| `verify-packet.sh` | Before merge | Regeneration reproducibility and clean diff |

`regen.sh` writes per-phase timings to
`target/f-b1/regen-profile.latest.tsv` by default.
`verify-packet.sh` writes wrapper timings to
`target/f-b1/verify-packet-profile.latest.tsv`.
Use `F_B1_PROFILE_DIR`, `F_B1_PROFILE_LOG`, or
`F_B1_VERIFY_PROFILE_LOG` to redirect those untracked logs.

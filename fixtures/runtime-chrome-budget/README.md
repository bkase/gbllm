# Runtime Chrome Budget Fixtures

`bringup-dmg-mbc5.chrome_budget.json` is the explicit Bringup runtime-chrome
budget input for the current canonical target profile:
`gbf_hw::target::dmg_mbc5_8mib_128kib()`.

Its reduced `reserved_slack` values are fixture inputs, not profile-time
relaxations. F-B4 consumers must hash and consume the selected budget artifact
as supplied.

The `runtime_nucleus_hash` is a fixture-only synthetic identifier, not derived
from F-A5's pinned runtime nucleus.

CGB/MBC5 bringup budget fixtures are deferred until `gbf-hw` exposes a
canonical CGB/MBC5 `TargetProfile`.

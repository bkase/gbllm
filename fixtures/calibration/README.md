# Bootstrap Calibration Fixtures

`bootstrap-dmg-mbc5.calibration.json` is the F-B2/F-B4 bringup calibration
fixture for the current canonical target profile:
`gbf_hw::target::dmg_mbc5_8mib_128kib()`.

The pinned bootstrap target profile hash is a synthetic fixture identifier for
`gbf_hw::target::dmg_mbc5_8mib_128kib()` until a canonical `TargetProfile`
content-hash helper exists.

The bundle intentionally declares `CalibrationConfidenceClass::None` and
contains no measurements. Bringup accepts this only because its
`RiskPolicy::calibration_confidence_requirement` is `NoMinimumConfidence`;
Default, Trace, and Recovery remain expected to reject it via the normal
calibration-confidence gate.

`bootstrap-cgb-mbc5.calibration.json` is deferred because `gbf-hw` does not
currently expose a canonical CGB/MBC5 `TargetProfile` constructor or fixture.
When that target exists, add a separate content-addressed bundle instead of
editing the DMG fixture in place.

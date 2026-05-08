//! Calibration schema fixtures.

use std::collections::BTreeMap;

use gbf_foundation::{Hash256, PackerVersion};
use gbf_policy::calibration::{
    BootstrapCalibrationBundle, CalibrationBundle, CalibrationBundleSet, CalibrationLayer,
    MeasurementBlob,
};

pub struct CalibrationBundleBuilder {
    bundle: CalibrationBundle,
}

impl CalibrationBundleBuilder {
    #[must_use]
    pub fn canonical(layer: CalibrationLayer) -> Self {
        let mut bundle = BootstrapCalibrationBundle::new(hash(0x10))
            .bundles
            .remove(&layer)
            .expect("bootstrap factory emits every layer");
        bundle.kernel_set_hash = hash(0x11);
        bundle.calibration_schema_hash = hash(0x12);

        Self { bundle }
    }

    #[must_use]
    pub fn with_target_profile_hash(mut self, target_profile_hash: Hash256) -> Self {
        self.bundle.target_profile_hash = target_profile_hash;
        self
    }

    #[must_use]
    pub fn with_packer_version(mut self, packer_version: PackerVersion) -> Self {
        self.bundle.packer_version = packer_version;
        self
    }

    #[must_use]
    pub fn with_measurements(mut self, measurements: MeasurementBlob) -> Self {
        self.bundle.measurements = Some(measurements);
        self
    }

    #[must_use]
    pub fn build(self) -> CalibrationBundle {
        self.bundle
    }
}

pub struct CalibrationBundleSetBuilder {
    target_profile_hash: Hash256,
    bundles: BTreeMap<CalibrationLayer, CalibrationBundle>,
}

impl CalibrationBundleSetBuilder {
    #[must_use]
    pub fn empty(target_profile_hash: Hash256) -> Self {
        Self {
            target_profile_hash,
            bundles: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn canonical() -> Self {
        CalibrationLayer::all()
            .into_iter()
            .fold(Self::empty(hash(0x10)), Self::with_layer)
    }

    #[must_use]
    pub fn with_layer(mut self, layer: CalibrationLayer) -> Self {
        let bundle = CalibrationBundleBuilder::canonical(layer)
            .with_target_profile_hash(self.target_profile_hash)
            .build();
        self.bundles.insert(layer, bundle);
        self
    }

    #[must_use]
    pub fn with_bundle(mut self, bundle: CalibrationBundle) -> Self {
        self.bundles.insert(bundle.layer, bundle);
        self
    }

    #[must_use]
    pub fn build(self) -> CalibrationBundleSet {
        CalibrationBundleSet {
            bundles: self.bundles,
        }
    }
}

#[must_use]
pub fn canonical_calibration_bundle_set_fixture() -> CalibrationBundleSet {
    CalibrationBundleSetBuilder::canonical().build()
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_supports_layered_construction() {
        let target = hash(0x44);
        let set = CalibrationBundleSetBuilder::empty(target)
            .with_layer(CalibrationLayer::Runtime)
            .with_layer(CalibrationLayer::Platform)
            .build();

        assert_eq!(set.bundles.len(), 2);
        assert_eq!(
            set.bundles[&CalibrationLayer::Runtime].target_profile_hash,
            target
        );
        assert_eq!(
            set.bundles[&CalibrationLayer::Platform].target_profile_hash,
            target
        );
        assert!(!set.bundles.contains_key(&CalibrationLayer::Kernel));
    }
}

//! Calibration bundle schema consumed by policy validation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gbf_foundation::{Hash256, PackerVersion};
use gbf_hw::calibration::{
    CalibrationConfidenceClass, CalibrationSetRef as LayeredCalibrationSetRef,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CalibrationLayer {
    Platform,
    Kernel,
    Runtime,
}

impl CalibrationLayer {
    const ALL: [Self; 3] = [Self::Platform, Self::Kernel, Self::Runtime];

    #[must_use]
    pub const fn all() -> [Self; 3] {
        Self::ALL
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "Platform",
            Self::Kernel => "Kernel",
            Self::Runtime => "Runtime",
        }
    }

    fn parse(value: &str) -> Result<Self, CalibrationLayerParseError> {
        match value {
            "Platform" => Ok(Self::Platform),
            "Kernel" => Ok(Self::Kernel),
            "Runtime" => Ok(Self::Runtime),
            _ => Err(CalibrationLayerParseError),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CalibrationLayerParseError;

impl fmt::Display for CalibrationLayerParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown calibration layer")
    }
}

impl std::error::Error for CalibrationLayerParseError {}

impl Serialize for CalibrationLayer {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct TaggedCalibrationLayer {
            kind: &'static str,
        }

        TaggedCalibrationLayer {
            kind: self.as_str(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CalibrationLayer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TaggedCalibrationLayer {
            kind: String,
        }

        let tagged = TaggedCalibrationLayer::deserialize(deserializer)?;
        Self::parse(&tagged.kind).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationBundle {
    pub layer: CalibrationLayer,
    pub target_profile_hash: Hash256,
    pub kernel_set_hash: Hash256,
    pub packer_version: PackerVersion,
    pub calibration_schema_hash: Hash256,
    pub validity_envelope: ValidityEnvelope,
    pub confidence: CalibrationConfidenceClass,
    pub measurements: Option<MeasurementBlob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrationBundleSet {
    pub resolved_ref: LayeredCalibrationSetRef,
    pub bundles: BTreeMap<CalibrationLayer, CalibrationBundle>,
}

impl Serialize for CalibrationBundleSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct CalibrationBundleSetRepr<'a> {
            resolved_ref: &'a LayeredCalibrationSetRef,
            bundles: BTreeMap<&'static str, &'a CalibrationBundle>,
        }

        let bundles = self
            .bundles
            .iter()
            .map(|(layer, bundle)| (layer.as_str(), bundle))
            .collect();
        CalibrationBundleSetRepr {
            resolved_ref: &self.resolved_ref,
            bundles,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CalibrationBundleSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CalibrationBundleSetRepr {
            #[serde(default)]
            resolved_ref: LayeredCalibrationSetRef,
            bundles: BTreeMap<String, CalibrationBundle>,
        }

        let repr = CalibrationBundleSetRepr::deserialize(deserializer)?;
        let bundles = repr
            .bundles
            .into_iter()
            .map(|(layer, bundle)| {
                let parsed = CalibrationLayer::parse(&layer).map_err(serde::de::Error::custom)?;
                if parsed != bundle.layer {
                    return Err(serde::de::Error::custom(format_args!(
                        "calibration bundle key {layer} does not match bundle layer {}",
                        bundle.layer.as_str()
                    )));
                }
                Ok((parsed, bundle))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            resolved_ref: repr.resolved_ref,
            bundles,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationSetRef {
    pub set_hash: Hash256,
    pub layers: BTreeSet<CalibrationLayer>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidityEnvelope {
    #[serde(default)]
    pub future_fields: ValidityEnvelopeFuturePlaceholder,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidityEnvelopeFuturePlaceholder {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementBlob {
    pub schema: String,
    pub payload_hash: Hash256,
}

pub struct BootstrapCalibrationBundle;

impl BootstrapCalibrationBundle {
    #[must_use]
    pub fn dmg_mbc5_ref() -> LayeredCalibrationSetRef {
        LayeredCalibrationSetRef {
            platform: Some(gbf_foundation::PlatformCalibrationId::from(
                "platform.bootstrap-dmg-mbc5",
            )),
            kernel: Some(gbf_foundation::KernelCalibrationId::from(
                "kernel.bootstrap-dmg-mbc5",
            )),
            runtime: Some(gbf_foundation::RuntimeCalibrationId::from(
                "runtime.bootstrap-dmg-mbc5",
            )),
        }
    }

    #[allow(clippy::new_ret_no_self)]
    #[must_use]
    pub fn new(synthetic_target_profile_hash: Hash256) -> CalibrationBundleSet {
        let bundles = CalibrationLayer::all()
            .into_iter()
            .map(|layer| {
                (
                    layer,
                    CalibrationBundle {
                        layer,
                        target_profile_hash: synthetic_target_profile_hash,
                        kernel_set_hash: Hash256::ZERO,
                        packer_version: PackerVersion::new(1, 0, 0),
                        calibration_schema_hash: Hash256::ZERO,
                        validity_envelope: ValidityEnvelope::default(),
                        confidence: CalibrationConfidenceClass::None,
                        measurements: None,
                    },
                )
            })
            .collect();

        CalibrationBundleSet {
            resolved_ref: Self::dmg_mbc5_ref(),
            bundles,
        }
    }
}

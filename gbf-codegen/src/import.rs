//! Artifact import helpers.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_artifact::core::ArtifactCore;
use gbf_artifact::{ArtifactAux, ArtifactManifest, HintBundle, TargetDataLoweringArtifact};
use gbf_foundation::FieldPath;

use crate::validate::{
    ArtifactTransportIdentity, ImportedArtifactView, ReferenceLink,
    is_forbidden_build_identity_key, raw_forbidden_build_identity_fields,
};

#[derive(Debug, Clone)]
pub struct RawArtifactJsonImport {
    pub core: ArtifactCore,
    pub manifest_json: serde_json::Value,
    pub aux_json: serde_json::Value,
    pub lowerings_json: serde_json::Value,
    pub hint_bundle: Option<HintBundle>,
    pub reference: Option<ReferenceLink>,
    pub transport: ArtifactTransportIdentity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedArtifactJson {
    pub artifact: ImportedArtifactView,
    pub lowerings: Vec<TargetDataLoweringArtifact>,
}

#[derive(Debug)]
pub enum ArtifactImportError {
    Manifest(serde_json::Error),
    Aux(serde_json::Error),
    Lowerings(serde_json::Error),
}

impl fmt::Display for ArtifactImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => write!(f, "artifact manifest JSON import failed: {error}"),
            Self::Aux(error) => write!(f, "artifact aux JSON import failed: {error}"),
            Self::Lowerings(error) => write!(f, "target-data lowering JSON import failed: {error}"),
        }
    }
}

impl Error for ArtifactImportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Manifest(error) | Self::Aux(error) | Self::Lowerings(error) => Some(error),
        }
    }
}

pub fn import_artifact_view_from_raw_json(
    input: RawArtifactJsonImport,
) -> Result<ImportedArtifactJson, ArtifactImportError> {
    let mut forbidden_fields = BTreeSet::new();
    let manifest_json = scrub_forbidden_build_identity_fields(
        input.manifest_json,
        "manifest",
        &mut forbidden_fields,
    );
    let aux_json =
        scrub_forbidden_build_identity_fields(input.aux_json, "aux", &mut forbidden_fields);
    let lowerings_json = scrub_forbidden_build_identity_fields(
        input.lowerings_json,
        "lowerings",
        &mut forbidden_fields,
    );

    let manifest: ArtifactManifest =
        serde_json::from_value(manifest_json).map_err(ArtifactImportError::Manifest)?;
    let aux: ArtifactAux = serde_json::from_value(aux_json).map_err(ArtifactImportError::Aux)?;
    let lowerings: Vec<TargetDataLoweringArtifact> =
        serde_json::from_value(lowerings_json).map_err(ArtifactImportError::Lowerings)?;

    let mut artifact = ImportedArtifactView::new(
        input.core,
        manifest,
        aux,
        input.hint_bundle,
        input.reference,
        input.transport,
    );
    artifact.forbidden_build_identity_fields = forbidden_fields;

    Ok(ImportedArtifactJson {
        artifact,
        lowerings,
    })
}

fn scrub_forbidden_build_identity_fields(
    mut value: serde_json::Value,
    root: &str,
    forbidden_fields: &mut BTreeSet<FieldPath>,
) -> serde_json::Value {
    forbidden_fields.extend(raw_forbidden_build_identity_fields(root, &value));
    strip_forbidden_build_identity_fields(&mut value);
    value
}

fn strip_forbidden_build_identity_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let forbidden_keys = map
                .keys()
                .filter(|key| is_forbidden_build_identity_key(key))
                .cloned()
                .collect::<Vec<_>>();
            for key in forbidden_keys {
                map.remove(&key);
            }
            for child in map.values_mut() {
                strip_forbidden_build_identity_fields(child);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                strip_forbidden_build_identity_fields(child);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

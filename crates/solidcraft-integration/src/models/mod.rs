//! Package 3D models in a board design: part files, imported library
//! models and envelopes. Every board body carries a [`BodyRole`] and a
//! [`MaterialId`], stored in namespaced metadata chunks under
//! [`MODEL_BODIES_KEY`]. A board model holds one definition per package
//! revision, shared by every instance's occurrence.
//!
//! The model frame has z = 0 on the seating plane and x along the package
//! length.
//!
//! Standard packages are made by their shipped generators, whose part
//! files these packages use ([`part_file`]).
pub mod gltf_ext;
pub mod imported;
pub mod inspect;
pub mod look;
pub mod named;
pub mod part_file;
pub mod step_assembly;

use crate::material::BodyKind;
use crate::native::Native;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sparksmith_core::library::BodyRole;
use sparksmith_core::materials::MaterialId;
use std::collections::BTreeMap;

/// Body role/material chunks of model definitions:
/// `org.sparksmith.electronics.models.0`, `.1`, … each ≤ 48 KiB.
pub const MODEL_BODIES_KEY: &str = "org.sparksmith.electronics.models.";

/// Prefix of the named native parameters of a package definition:
/// `pkg<revision-key>_<name>`, e.g. `pkg3r1_pitch`, so several package
/// definitions coexist in one board document.
pub const PARAMETER_PREFIX: &str = "pkg";

const CHUNK_BYTES: usize = 48 * 1024;
/// Most model chunks; with the body-kind chunks they stay below
/// Solidcraft's 256 metadata keys.
pub const MAX_MODEL_CHUNKS: usize = 200;

/// The definition name (and parameter stem) of one package revision.
pub fn definition_name(package: sparksmith_core::DefinitionId, revision: u32) -> String {
    format!("{PARAMETER_PREFIX}{}r{revision}", package.0)
}

/// One body of a package model, in the model frame (z = 0 seating plane).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedBody {
    /// The native body name, unique within the definition.
    pub name: String,
    pub role: BodyRole,
    pub material: MaterialId,
    /// The package terminal a lead or termination body belongs to.
    pub terminal: Option<u32>,
}

impl GeneratedBody {
    /// How the body is drawn and checked: a lead-like metal part or the
    /// body proper.
    pub fn kind(&self) -> BodyKind {
        match self.role {
            BodyRole::Lead | BodyRole::Termination | BodyRole::ExposedPad => BodyKind::ModelLead,
            _ => BodyKind::ModelBody,
        }
    }
}

/// A built native component definition of one package revision.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDefinition {
    /// The native component the definition lives in.
    pub component: String,
    pub bodies: Vec<GeneratedBody>,
}

/// Store the role, material and terminal of every generated body in
/// namespaced metadata chunks (each ≤ 48 KiB as serialized). Too many
/// bodies is a refusal, never a silent drop.
pub fn record(native: &mut Native, definitions: &[ModelDefinition]) -> Result<()> {
    for (index, chunk) in chunks(definitions)?.iter().enumerate() {
        native.set_metadata(
            &format!("{MODEL_BODIES_KEY}{index}"),
            json!({ "bodies": chunk }),
        )?;
    }
    Ok(())
}

fn chunks(definitions: &[ModelDefinition]) -> Result<Vec<Vec<&GeneratedBody>>> {
    // `{"bodies":[` … `]}`.
    const FRAME: usize = 13;
    let mut out: Vec<Vec<&GeneratedBody>> = vec![Vec::new()];
    let mut size = FRAME;
    for body in definitions.iter().flat_map(|d| &d.bodies) {
        let entry = serde_json::to_string(body)?.len() + 1;
        if entry + FRAME > CHUNK_BYTES {
            return Err(Error::Invalid(format!(
                "model.metadata: body name too long: {}",
                body.name
            )));
        }
        if size + entry > CHUNK_BYTES {
            out.push(Vec::new());
            size = FRAME;
        }
        if let Some(chunk) = out.last_mut() {
            chunk.push(body);
            size += entry;
        }
    }
    if out.len() > MAX_MODEL_CHUNKS {
        return Err(Error::Invalid(
            "model.metadata: the board has too many generated model bodies to record".into(),
        ));
    }
    Ok(out)
}

/// One metadata chunk as stored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    bodies: Vec<GeneratedBody>,
}

/// The generated bodies a native design records, by body name.
pub fn recorded(native: &Native) -> Result<BTreeMap<String, GeneratedBody>> {
    let metadata = serde_json::to_value(&native.session().doc.metadata)?;
    let mut out = BTreeMap::new();
    for (key, value) in metadata.as_object().into_iter().flatten() {
        if !key.starts_with(MODEL_BODIES_KEY) {
            continue;
        }
        let chunk: Chunk = serde_json::from_value(value.clone())?;
        for body in chunk.bodies {
            out.insert(body.name.clone(), body);
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests;

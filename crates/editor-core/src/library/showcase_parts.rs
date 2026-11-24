//! The parts of the PCB realism showcase (`Engine::showcase_with`), as
//! shipped library data (`library/showcase/parts.json`): each part's draft
//! definitions, naming placeholder lineages ([`PLACEHOLDERS`]) that
//! [`remap`] replaces with the lineages a project reserves, and its 3D
//! model, a generator's part file at values with what the host evaluated
//! them to. The part file's bytes come from the shipped generator bundles
//! by hash. The host reads the file
//! into its context.
use super::model3d::ModelDefinition;
use super::package_model::{PartDerived, PartValue};
use super::part_refs::SampleRefs;
use super::{Definition, DraftEdit};
use crate::diagnostics::{Error, Result};
use crate::ids::{BindingRef, DefinitionId, FootprintRef, PackageRef, Revision, SymbolRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The file's format.
pub const FORMAT: u32 = 1;
/// Parts the file may hold.
pub const MAX_PARTS: usize = 64;

/// The lineages the stored edits name.
pub const PLACEHOLDERS: SampleRefs = SampleRefs {
    component: DefinitionId(1),
    symbol: SymbolRef {
        id: DefinitionId(2),
        revision: Revision::FIRST,
    },
    package: PackageRef {
        id: DefinitionId(3),
        revision: Revision::FIRST,
    },
    footprint: FootprintRef {
        id: DefinitionId(4),
        revision: Revision::FIRST,
    },
    binding: BindingRef {
        id: DefinitionId(5),
        revision: Revision::FIRST,
    },
};

/// `library/showcase/parts.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShowcaseParts {
    pub format: u32,
    pub parts: Vec<ShowcasePart>,
}

/// One showcase part.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShowcasePart {
    pub name: String,
    /// Component, symbol, package, footprint and binding, naming
    /// [`PLACEHOLDERS`].
    pub edits: Vec<DraftEdit>,
    /// The package's 3D model; `None` keeps the package's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ShowcaseModel>,
}

/// A part file and its use by the part's package.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShowcaseModel {
    pub name: String,
    /// The Model the part file becomes; its asset's bytes come from the
    /// shipped generator bundles.
    pub definition: ModelDefinition,
    pub values: BTreeMap<String, PartValue>,
    pub derived: PartDerived,
}

impl ShowcaseParts {
    /// Read and check the file's text.
    pub fn parse(text: &str) -> Result<Self> {
        let parts: Self = serde_json::from_str(text)
            .map_err(|e| Error::Invalid(format!("showcase parts: {e}")))?;
        if parts.format != FORMAT {
            return Err(Error::Invalid(format!(
                "showcase parts: format {} is not {FORMAT}",
                parts.format
            )));
        }
        if parts.parts.len() > MAX_PARTS {
            return Err(Error::Limit(format!(
                "showcase parts: at most {MAX_PARTS} parts"
            )));
        }
        Ok(parts)
    }
}

/// `edits` with every reference to the [`PLACEHOLDERS`] lineages naming
/// `to` instead. An edit of another lineage is refused.
pub fn remap(edits: &[DraftEdit], to: SampleRefs) -> Result<Vec<DraftEdit>> {
    let from = PLACEHOLDERS;
    let id = |old: DefinitionId| -> Result<DefinitionId> {
        Ok(match old {
            x if x == from.component => to.component,
            x if x == from.symbol.id => to.symbol.id,
            x if x == from.package.id => to.package.id,
            x if x == from.footprint.id => to.footprint.id,
            x if x == from.binding.id => to.binding.id,
            other => {
                return Err(Error::Invalid(format!(
                    "showcase parts: an edit names lineage {} outside its part",
                    other.0
                )));
            }
        })
    };
    edits
        .iter()
        .map(|edit| {
            let mut edit = edit.clone();
            edit.id = id(edit.id)?;
            match &mut edit.definition {
                Definition::Component(c) => {
                    for r in &mut c.representations {
                        r.id = id(r.id)?;
                    }
                    for r in &mut c.implementations {
                        r.id = id(r.id)?;
                    }
                }
                Definition::Symbol(s) => s.component = id(s.component)?,
                Definition::Footprint(f) => f.package.id = id(f.package.id)?,
                Definition::Binding(b) => {
                    b.component = id(b.component)?;
                    b.package.id = id(b.package.id)?;
                    for f in &mut b.footprints {
                        f.id = id(f.id)?;
                    }
                }
                Definition::Package(_) => {}
                _ => {
                    return Err(Error::Invalid(
                        "showcase parts: a part holds a component, symbol, package, footprint and binding only".into(),
                    ));
                }
            }
            Ok(edit)
        })
        .collect()
}

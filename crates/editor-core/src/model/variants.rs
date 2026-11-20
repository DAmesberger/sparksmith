//! Assembly variants saved with the design (docs/assembly-variants.md):
//! data only. The rules (which part a variant fits, whether a substitute
//! fits the land pattern) live in `instances::variants`.
use crate::ids::{Id, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Variants one design keeps.
pub const MAX_VARIANTS: usize = 64;
/// Bytes of a variant name.
pub const MAX_VARIANT_NAME: usize = 64;

/// Whether a placed part is fitted at assembly. A typed state, never a
/// value text such as "DNP".
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Population {
    /// Fitted at assembly.
    #[default]
    Populated,
    /// Do not populate: the footprint stays on the board, the part is left
    /// off.
    NotPopulated,
}

impl Population {
    pub fn is_populated(&self) -> bool {
        *self == Self::Populated
    }
}

/// What one variant changes for one part. A field left `None` follows the
/// default (the board without a variant).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VariantOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub population: Option<Population>,
    /// The manufacturer part fitted instead of the default one: a Part on
    /// the same land pattern with the same pad grouping
    /// (`instances::variants::substitute_fit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl VariantOverride {
    pub fn is_empty(&self) -> bool {
        self.population.is_none() && self.part.is_none()
    }
}

/// One named assembly variant: per-part overrides of the default.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AssemblyVariant {
    /// From the project's identity allocator.
    pub id: Id,
    pub name: String,
    /// By instance; an instance not listed is as the default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<Id, VariantOverride>,
}

/// The design's assembly variants and the one outputs follow.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AssemblyVariants {
    /// In the order the person keeps them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<AssemblyVariant>,
    /// The variant the 3D view, BOM, pick-and-place and fabrication output
    /// follow; `None` is the default (no variant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<Id>,
}

impl AssemblyVariants {
    pub fn is_default(&self) -> bool {
        self.list.is_empty() && self.active.is_none()
    }

    pub fn get(&self, id: Id) -> Option<&AssemblyVariant> {
        self.list.iter().find(|v| v.id == id)
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut AssemblyVariant> {
        self.list.iter_mut().find(|v| v.id == id)
    }

    /// The active variant, when one is chosen.
    pub fn active(&self) -> Option<&AssemblyVariant> {
        self.active.and_then(|id| self.get(id))
    }

    /// The variant with this name, compared case-insensitively.
    pub fn by_name(&self, name: &str) -> Option<&AssemblyVariant> {
        let name = name.trim().to_lowercase();
        self.list
            .iter()
            .find(|v| v.name.trim().to_lowercase() == name)
    }
}

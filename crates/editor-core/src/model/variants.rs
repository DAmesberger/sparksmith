//! Assembly variants saved with the design (docs/assembly-variants.md):
//! data only. The rules (which part a variant fits, whether a substitute
//! fits the land pattern) live in `instances::variants`.
use crate::ids::{DefinitionId, Id, PartRef};
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

/// Requirement exceptions one substitute review records.
pub const MAX_EXCEPTIONS: usize = 64;
/// Bytes of an exception's reason.
pub const MAX_EXCEPTION_REASON: usize = 512;

/// A person's review of a substitute (`variant.review_substitute`), bound
/// to the substitute Part it reviewed. Fitting the lands with the same pad
/// grouping is physical fit; it says nothing about what each pin does or
/// what the part is rated for. A substitute counts as resolved only once a
/// person confirmed its electrical equivalence and every requirement of
/// the part it replaces is met or deliberately excepted here.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubstituteReview {
    /// The substitute reviewed; choosing another Part voids the review.
    pub part: PartRef,
    /// A person confirmed that on every pad the substitute does the job of
    /// the part it replaces (its pinout: VCC where VCC was, not only a pad
    /// grouping that matches).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub equivalent: bool,
    /// Requirements of the instance the substitute is accepted not to meet,
    /// each a deliberate decision with its reason (a 0 Ω link where a
    /// resistance was required).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<RequirementException>,
}

/// One requirement a substitute is deliberately accepted not to meet.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequirementException {
    /// The field lineage of the requirement.
    pub field: DefinitionId,
    /// Why it is accepted: printable, 1–512 bytes.
    pub reason: String,
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
    /// By instance: the review of the substitute this variant fits on it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reviews: BTreeMap<Id, SubstituteReview>,
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

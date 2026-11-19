//! The standard fields: SI quantity kinds and the fields a passive or a
//! rating usually selects, as an ordinary bundle `library.import` takes
//! (`library.standard_fields`, `library.import_standard_fields`). The bundle
//! and what its fields are for ([`FieldRoles`]) ship as data in the shipped
//! library's `standard-fields/` folder; [`super::standards`] holds them.
//!
//! The lineages and the library identity are fixed in that data, so
//! importing the bundle twice reuses the first copies. A published record
//! never changes: a change to one is a new revision of it, with the old
//! revision kept, or an import into a project that holds the old copy is
//! `library.source_modified`.
//!
//! No temperature field: a unit here is a scale of the SI unit, and a
//! Celsius reading needs an offset.
use super::bundle::{self, Bundle};
use super::{Definition, Library, standards};
use crate::diagnostics::{Error, Result};
use crate::fields::{FieldSelection, ValueType};
use crate::ids::{DefinitionKind, FieldRef, LibraryId, SourceLineageId};
use crate::quantity::UnitSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the standard fields are for (`standard-fields/roles.json`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldRoles {
    pub format: u32,
    #[serde(default)]
    pub about: String,
    /// The fields that are a part's value, as a BOM's Value column states
    /// it: a passive's resistance, capacitance or inductance, a crystal's
    /// frequency and an LED's colour. Ratings and tolerances qualify a
    /// value.
    pub value_fields: Vec<SourceLineageId>,
    /// The text field a part selects when its family selects no fields (a
    /// generated IC, connector or module, an imported project part), so
    /// every part can hold its value as a design states it. Not a value
    /// field: such a part's BOM Value falls back to its part number.
    pub part_value: SourceLineageId,
}

/// Fields one role lists.
const MAX_ROLE_FIELDS: usize = 16;

/// The standard fields bundle and its roles, checked, with each field's
/// units and choices by name.
#[derive(Clone, Debug, PartialEq)]
pub struct StandardFields {
    bundle: Bundle,
    roles: FieldRoles,
    /// Quantity fields by name, with their kind's units.
    units: Vec<(String, UnitSet)>,
    /// Enumeration fields by name, with their choice labels in order.
    choices: Vec<(String, Vec<String>)>,
}

impl StandardFields {
    /// The bundle must pass `library.import`'s structural checks, and every
    /// role must name one of its fields.
    pub fn new(bundle: Bundle, roles: FieldRoles) -> Result<Self> {
        standards::check_format(roles.format, "the standard field roles")?;
        bundle::validate_bundle(&bundle)?;
        let is_field = |lineage: &SourceLineageId| {
            bundle
                .records
                .iter()
                .any(|r| r.lineage == *lineage && r.kind == DefinitionKind::Field)
        };
        if roles.value_fields.len() > MAX_ROLE_FIELDS {
            return Err(Error::Limit(format!(
                "a standard field role lists at most {MAX_ROLE_FIELDS} fields"
            )));
        }
        if let Some(missing) = roles.value_fields.iter().find(|l| !is_field(l)) {
            return Err(Error::Invalid(format!(
                "the standard field roles name {missing}, which is not a field of the standard fields bundle"
            )));
        }
        let part_value_is_text = bundle.records.iter().any(|r| {
            r.lineage == roles.part_value
                && matches!(&r.definition, Definition::Field(f) if matches!(f.value_type, ValueType::Text { .. }))
        });
        if !part_value_is_text {
            return Err(Error::Invalid(format!(
                "the standard field roles name {} as the part value field, which is not a text field of the standard fields bundle",
                roles.part_value
            )));
        }
        let mut units = Vec::new();
        let mut choices = Vec::new();
        for record in &bundle.records {
            let Definition::Field(field) = &record.definition else {
                continue;
            };
            match &field.value_type {
                ValueType::Quantity { kind, .. } => {
                    let set = record
                        .links
                        .iter()
                        .find(|l| l.local == kind.id)
                        .and_then(|link| {
                            bundle.records.iter().find_map(|r| match &r.definition {
                                Definition::QuantityKind(k) if r.lineage == link.lineage => {
                                    Some(k.units.clone())
                                }
                                _ => None,
                            })
                        })
                        .ok_or_else(|| {
                            Error::Invalid(format!(
                                "the standard field {} names no quantity kind of the bundle",
                                record.meta.name
                            ))
                        })?;
                    units.push((record.meta.name.clone(), set));
                }
                ValueType::Enum { choices: list } => choices.push((
                    record.meta.name.clone(),
                    list.iter().map(|c| c.label.clone()).collect(),
                )),
                _ => {}
            }
        }
        Ok(Self {
            bundle,
            roles,
            units,
            choices,
        })
    }

    /// The bundle `library.import` takes.
    pub fn bundle(&self) -> &Bundle {
        &self.bundle
    }

    /// The source's identity, as origins of imported copies record it.
    pub fn library(&self) -> LibraryId {
        self.bundle.library
    }

    /// The lineage of a standard field, by its name.
    pub fn lineage(&self, field: &str) -> Option<SourceLineageId> {
        self.bundle
            .records
            .iter()
            .find(|r| r.kind == DefinitionKind::Field && r.meta.name == field)
            .map(|r| r.lineage)
    }

    /// The project's copy of a standard field, by its name: the field
    /// whose recorded origin is that standard field.
    pub fn local(&self, library: &Library, field: &str) -> Option<FieldRef> {
        self.local_of(library, self.lineage(field)?)
    }

    /// The project's copy of the standard field of `lineage`.
    fn local_of(&self, library: &Library, lineage: SourceLineageId) -> Option<FieldRef> {
        let source = self.library();
        library
            .origins
            .iter()
            .find(|o| {
                o.local.kind == DefinitionKind::Field
                    && o.origin.library == source
                    && o.origin.lineage == lineage
            })
            .map(|o| FieldRef::new(o.local.id, o.local.revision))
    }

    /// The project's copy of the part value field
    /// ([`FieldRoles::part_value`]); `None` until the bundle is imported.
    pub fn part_value(&self, library: &Library) -> Option<FieldRef> {
        self.local_of(library, self.roles.part_value)
    }

    /// The selection of the part value field a component without fields
    /// of its family takes, shown on its symbol.
    pub fn part_value_selection(&self, library: &Library) -> Option<FieldSelection> {
        self.part_value(library).map(|field| FieldSelection {
            field,
            default: None,
            show_on_symbol: true,
        })
    }

    /// The units of a standard quantity field, by its name.
    pub fn units(&self, field: &str) -> Option<&UnitSet> {
        self.units
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, units)| units)
    }

    /// The choice labels of a standard enumeration field, by its name.
    pub fn choices(&self, field: &str) -> Option<&[String]> {
        self.choices
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, labels)| labels.as_slice())
    }

    /// Whether `field` is the project's copy of a standard value field, by
    /// its recorded origin, never by its name.
    pub fn is_value_field(&self, library: &Library, field: FieldRef) -> bool {
        let source = self.library();
        library.origins.iter().any(|o| {
            o.local.kind == DefinitionKind::Field
                && o.local.id == field.id
                && o.origin.library == source
                && self.roles.value_fields.contains(&o.origin.lineage)
        })
    }
}

/// The bundle of every standard kind and field, from the installed
/// standards.
pub fn bundle() -> Result<Bundle> {
    Ok(standards::get()?.fields.bundle().clone())
}

/// The standard fields' source identity, when the standards are installed.
pub fn library_id() -> Option<LibraryId> {
    standards::get().ok().map(|s| s.fields.library())
}

/// Whether `field` is the project's copy of a standard value field
/// ([`StandardFields::is_value_field`]). False when the standards are not
/// installed, which the program refuses to start without.
pub fn is_value_field(library: &Library, field: FieldRef) -> bool {
    standards::get().is_ok_and(|s| s.fields.is_value_field(library, field))
}

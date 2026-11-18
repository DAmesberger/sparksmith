//! Package definitions: physical terminals, the envelope, the lead
//! contacts, the 3D model source and its placement.
use super::datum::{PackageDatum, PackageDatumOrigin};
use super::package_model::{ModelPlacement, PackageModel};
use super::publish::{issue, object};
use super::{MAX_LABEL, MAX_PACKAGE_TERMINALS, printable};
use crate::diagnostics::{Finding, ObjectRef, Result};
use crate::geometry::{Point3, finite_within, size_valid};
use crate::ids::{DefinitionId, PackageTerminalId};
use crate::units::Nm;
use crate::units::PointNm;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageDefinition {
    pub next_local: u32,
    pub terminals: Vec<PackageTerminal>,
    /// Envelope length, width and height, in the model frame from the
    /// seating plane up, centred in x and y.
    pub envelope_nm: [Nm; 3],
    /// Where the 3D model comes from and its body materials.
    pub model: PackageModel,
    /// The model in the datum frame (`model_frame`).
    pub placement: ModelPlacement,
    /// Set when the core generated this package's set from a part file
    /// (`layout.place_part`, docs/mechanical-parts.md): the variant record.
    /// Display, dedupe and assembly flags only; electrical rules never
    /// read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical: Option<Box<super::mechanical::MechanicalPart>>,
    /// Datums the package declares in its model frame, such as a
    /// connector's opening (slice B2, `library::datum`): authored, copied
    /// from the part file, or emitted by the typed form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datums: Vec<PackageDatum>,
    /// Where each lead touches the seating plane (z = 0), in the model
    /// frame, with the terminal it belongs to: what "Fit leads to pads"
    /// fits (`package_fit`). Plain data: kept from the typed form an older
    /// file stored (format 5, part-generation slice D2), never inferred.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leads: Vec<PackageLead>,
    /// How the realistic view draws the leads' solder joints and a chip's
    /// terminations: plain data kept from the typed form an older file
    /// stored (format 5); `None` draws none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joints: Option<PackageJoints>,
}

/// The solder joints and terminations of a package's leads, for display.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackageJoints {
    /// A two-terminal chip with terminations wrapped over both ends: a
    /// resistor's alumina body with an overcoat, else a ceramic one.
    Chip { resistor: bool },
    /// Flat feet on their lands (gull-wing, SOT, DPAK): toe and heel
    /// fillets.
    Feet {
        length_nm: Nm,
        width_nm: Nm,
        thickness_nm: Nm,
    },
    /// Square pins through their holes (a pin header): a cone round each
    /// on the far side.
    Pins { side_nm: Nm },
}

/// Lead contacts one package may list.
pub const MAX_PACKAGE_LEADS: usize = super::MAX_PACKAGE_TERMINALS;

/// One lead's contact with the seating plane and its package terminal.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageLead {
    pub terminal: PackageTerminalId,
    /// In the model frame, on the seating plane.
    pub contact: PointNm,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageTerminal {
    pub id: PackageTerminalId,
    /// Printed or datasheet designation, e.g. `1` or `EP`. Never identity.
    pub designation: String,
}

impl PackageDefinition {
    pub fn terminal(&self, id: PackageTerminalId) -> Option<&PackageTerminal> {
        self.terminals.iter().find(|t| t.id == id)
    }

    /// The datum named `name`.
    pub fn datum(&self, name: &str) -> Option<&PackageDatum> {
        self.datums.iter().find(|d| d.name.0 == name)
    }

    /// Replace the datums that came from `origin` (a part file or the
    /// typed form) by `datums`, each marked with that origin. A datum of
    /// another origin keeps its name: the source's datum of that name is
    /// left out, so what a person authored or edited is never overwritten.
    pub fn replace_datums(&mut self, origin: PackageDatumOrigin, datums: &[PackageDatum]) {
        self.datums.retain(|d| d.origin != origin);
        for datum in datums {
            if self.datum(&datum.name.0).is_none() {
                self.datums.push(PackageDatum {
                    origin,
                    ..datum.clone()
                });
            }
        }
    }

    /// The envelope's length, width and height in millimetres.
    pub fn envelope(&self) -> [f64; 3] {
        self.envelope_nm.map(Nm::to_mm_f64)
    }

    /// A positive finite envelope (the placement is in range by type): the
    /// condition for a native envelope model.
    pub fn envelope_valid(&self) -> bool {
        size_valid(&self.envelope())
    }

    /// A two-terminal chip's construction, when the package says it is
    /// one: whether it is a resistor.
    pub fn chip(&self) -> Option<bool> {
        match self.joints {
            Some(PackageJoints::Chip { resistor }) => Some(resistor),
            _ => None,
        }
    }

    /// Set the envelope to `size`.
    pub fn resize_envelope(&mut self, size: [f64; 3]) -> Result<()> {
        self.envelope_nm = [
            Nm::from_mm_f64_rounded(size[0])?,
            Nm::from_mm_f64_rounded(size[1])?,
            Nm::from_mm_f64_rounded(size[2])?,
        ];
        Ok(())
    }

    /// The envelope box's corners in the model frame (z = 0 up).
    pub fn envelope_corners(&self) -> [Point3; 8] {
        let [l, w, h] = self.envelope();
        std::array::from_fn(|i| {
            [
                if i & 1 == 0 { -l / 2.0 } else { l / 2.0 },
                if i & 2 == 0 { -w / 2.0 } else { w / 2.0 },
                if i & 4 == 0 { 0.0 } else { h },
            ]
        })
    }

    pub(super) fn within_bounds(&self) -> bool {
        self.terminals.len() <= MAX_PACKAGE_TERMINALS
            && self
                .terminals
                .iter()
                .all(|t| t.designation.len() <= MAX_LABEL)
            && self.datums.len() <= super::datum::MAX_PACKAGE_DATUMS
            && self.leads.len() <= MAX_PACKAGE_LEADS
            && match &self.model {
                PackageModel::PartFile(part) => {
                    part.values.len() <= super::package_model::MAX_PART_VALUES
                        && part.derived.surfaces.len() <= super::package_model::MAX_PART_SURFACES
                }
                _ => true,
            }
    }

    pub(super) fn geometry_finite(&self) -> bool {
        finite_within(&self.envelope())
    }

    pub(super) fn local_ids(&self) -> Vec<u32> {
        self.terminals.iter().map(|t| t.id.0).collect()
    }

    pub(super) fn findings(&self, id: DefinitionId, out: &mut Vec<Finding>) {
        let this = ObjectRef::Definition { id };
        // A package may have no physical terminal: a fiducial, a mounting
        // hole, a logo or a mechanical part (docs/mechanical-parts.md).
        if self.terminals.len() > MAX_PACKAGE_TERMINALS {
            out.push(issue(
                "package.terminal_count",
                format!("A package has at most {MAX_PACKAGE_TERMINALS} physical terminals."),
                vec![this],
            ));
        }
        let mut designations = BTreeSet::new();
        for terminal in &self.terminals {
            if !printable(&terminal.designation) || !designations.insert(&terminal.designation) {
                out.push(issue(
                    "package.designation",
                    "A package terminal needs a unique printable designation.".into(),
                    vec![object(id, terminal.id.0)],
                ));
            }
        }
        if self
            .leads
            .iter()
            .any(|l| self.terminal(l.terminal).is_none())
        {
            out.push(issue(
                "package.leads",
                "Every lead contact names a terminal of the package.".into(),
                vec![this],
            ));
        }
        // Chip joints draw a termination at each end: two terminals.
        if matches!(self.joints, Some(PackageJoints::Chip { .. })) && self.terminals.len() != 2 {
            out.push(issue(
                "package.chip_terminals",
                format!(
                    "A chip package has exactly two terminals; this one has {}.",
                    self.terminals.len()
                ),
                vec![this],
            ));
        }
        match &self.model {
            PackageModel::Library(model) => {
                super::model3d::package_findings(self, model, id, out);
            }
            PackageModel::PartFile(part) => {
                for problem in part.problems() {
                    out.push(issue(
                        "part_file.values",
                        format!("The part file: {problem}."),
                        vec![this],
                    ));
                }
            }
            PackageModel::Envelope { .. } | PackageModel::NoBody => {}
        }
        for problem in super::datum::datum_problems(&self.datums) {
            out.push(issue(
                "package.datum",
                format!("Package datum: {problem}."),
                vec![this],
            ));
        }
        if !self.envelope_valid() {
            out.push(issue(
                "package.envelope",
                "Package envelope dimensions must be finite and positive.".into(),
                vec![this],
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::support::{nm, pt};

    fn codes(package: &PackageDefinition) -> Vec<String> {
        let mut out = Vec::new();
        package.findings(DefinitionId(1), &mut out);
        out.into_iter().map(|f| f.code).collect()
    }

    fn chip() -> PackageDefinition {
        PackageDefinition {
            next_local: 3,
            terminals: (1..=2)
                .map(|i| PackageTerminal {
                    id: PackageTerminalId(i),
                    designation: i.to_string(),
                })
                .collect(),
            envelope_nm: [1.6, 0.8, 0.45].map(nm),
            leads: vec![
                PackageLead {
                    terminal: PackageTerminalId(1),
                    contact: pt([-0.65, 0.0]),
                },
                PackageLead {
                    terminal: PackageTerminalId(2),
                    contact: pt([0.65, 0.0]),
                },
            ],
            ..PackageDefinition::default()
        }
    }

    #[test]
    fn a_consistent_package_has_no_findings() {
        assert_eq!(codes(&chip()), Vec::<String>::new());
    }

    #[test]
    fn a_lead_names_a_terminal_of_the_package() {
        let mut p = chip();
        p.leads[1].terminal = PackageTerminalId(9);
        assert_eq!(codes(&p), ["package.leads"]);
    }

    #[test]
    fn model_findings_and_bounds() {
        let mut p = chip();
        p.model = PackageModel::Library(Default::default());
        assert_eq!(codes(&p), Vec::<String>::new(), "an empty lead map");
        let entry = |lead, terminal| crate::library::model3d::LeadMapEntry {
            lead,
            terminal: PackageTerminalId(terminal),
        };
        p.model = PackageModel::Library(crate::library::model3d::LibraryModel {
            model: Default::default(),
            lead_map: vec![entry(0, 1), entry(0, 2), entry(1, 9), entry(2, 1)],
        });
        assert_eq!(
            codes(&p),
            ["model.leads", "model.leads", "model.leads"],
            "a repeated lead, a foreign terminal and a repeated terminal"
        );
        // A placement is whole nanometres and millidegrees: an infinite
        // turn never becomes one.
        assert!(
            crate::library::ModelPlacement::from_mm([0.0; 3], [f64::INFINITY, 0.0, 0.0]).is_err()
        );
    }

    #[test]
    fn resizing_sets_the_envelope() -> Result<()> {
        let mut generic = PackageDefinition::default();
        generic.resize_envelope([5.0, 4.0, 1.0])?;
        assert_eq!(generic.envelope(), [5.0, 4.0, 1.0]);
        assert!(generic.resize_envelope([f64::NAN, 1.0, 1.0]).is_err());
        Ok(())
    }
}

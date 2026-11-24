//! A fabricator's own stackup as a file (`library/stackups/<fabricator>/
//! <id>.json`, loaded by `sparksmith-io`): the layers as published, each
//! with the row it was read from, the thicknesses the fabricator states
//! beside their sum, and where its own sources disagree. [`FabStackup`]
//! holds what was read; [`FabStackup::stackup`] is what a board takes from
//! it. Nothing is filled in: a value the source does not give stays unset.
//! [`pick`] is what taking one does to a board, before it is taken.
use crate::diagnostics::{Error, Result};
use crate::model::{
    COPPER_UM_RANGE, CopperFoil, DIELECTRIC_MM_RANGE, DK_RANGE, Dielectric, DielectricKind,
    GlassStyle, MASK_UM_RANGE, MAX_COPPER_LAYERS, MIN_COPPER_LAYERS, OUNCE_UM, Stackup,
};
use crate::rules::ProfileSource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod pick;
pub use pick::{
    PickOptions, StackupBuildRow, StackupFilter, StackupList, StackupPick, StackupReport,
    StackupRow, list, pick, report,
};

/// The stackup file's format; any other version is refused.
pub const FAB_STACKUP_FORMAT: u32 = 1;
/// Bytes of one text in a file.
const MAX_TEXT: usize = 4096;
/// Layers, notes, disagreements and profiles one file lists, at most.
const MAX_LIST: usize = 256;
/// The stored sums are rounded to 5 decimals, twice for a difference.
const SUM_MM: f64 = 2e-5;
/// Millimetres a mil.
const MIL_MM: f64 = 0.0254;

/// One stackup file.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FabStackupFile {
    pub format: u32,
    pub fab_stackup: FabStackup,
}

/// What a listed layer is.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FabLayerRole {
    Copper,
    Prepreg,
    Core,
}

impl FabLayerRole {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Copper => "Copper",
            Self::Prepreg => "Prepreg",
            Self::Core => "Core",
        }
    }
}

/// One layer as the fabricator lists it, top to bottom.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FabLayer {
    pub role: FabLayerRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A prepreg's glass style ("2116").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<String>,
    /// The laminate's brand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// The material as the source names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_english: Option<String>,
    /// As listed; before pressing where the source lists both.
    pub thickness_mm: f64,
    /// After pressing, where the source lists it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_lamination_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dk: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foil_text: Option<String>,
    /// The finished weight a listed base foil is plated to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plated_to_oz: Option<f64>,
    /// Its plated thickness, where the source lists one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plated_thickness_mm: Option<f64>,
    /// A core's thickness with its own foils, where that is listed too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed_with_copper_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed_with_copper_text: Option<String>,
    /// A core etched bare: it carries no copper layer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bare_board: bool,
    /// The row it was read from.
    pub source: String,
}

impl FabLayer {
    /// Its thickness in the pressed board.
    pub fn pressed_mm(&self) -> f64 {
        self.after_lamination_mm.unwrap_or(self.thickness_mm)
    }
}

/// The thickness option a stackup is listed under.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FabNominal {
    pub board_mm: f64,
    pub basis: String,
}

/// A thickness the source states, in its own words.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FabStated {
    pub value_mm: f64,
    #[serde(default)]
    pub tolerance: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reading: String,
}

/// The solder mask figures the source states with the stackup.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FabMask {
    pub source: String,
    pub above_substrate_mil: f64,
    pub above_trace_mil: f64,
    pub between_traces_mil: f64,
    pub dk: f64,
}

/// Flags the source data carries, as it gives them.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FabFlags {
    pub reception_default: i64,
    pub use_purpose: i64,
    pub lamination_type: i64,
    pub urgent_state: i64,
    pub default_flag: i64,
    pub display_name: String,
    pub note: String,
}

/// One stackup as a fabricator publishes it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FabStackup {
    /// Names the file.
    pub id: String,
    /// The fabricator's own name for it.
    pub name: String,
    /// The fabricator's record id; names the folder.
    pub fabricator: String,
    /// The fab profiles it goes with.
    pub fab_profiles: Vec<String>,
    pub copper_layers: usize,
    /// Finished outer copper.
    pub outer_copper_oz: f64,
    #[serde(default)]
    pub inner_copper_oz: Option<f64>,
    pub source: ProfileSource,
    pub nominal: FabNominal,
    #[serde(default)]
    pub stated_finished: Option<FabStated>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated_after_lamination: Option<FabStated>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_layer_residual_copper_ratio_pct: Option<f64>,
    pub layers: Vec<FabLayer>,
    pub layers_sum_mm: f64,
    pub layers_sum_basis: String,
    pub layers_sum_minus_nominal_mm: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers_sum_minus_stated_finished_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers_sum_after_lamination_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers_sum_after_lamination_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers_sum_after_lamination_minus_stated_mm: Option<f64>,
    pub symmetric: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<FabMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<FabFlags>,
    /// Where the source's own texts give other values, both named.
    pub disagreements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_impedance_page: Option<bool>,
}

/// The dielectric between two neighbouring foils: one or more listed plies.
#[derive(Clone, Debug, PartialEq)]
pub struct FabDielectric<'a> {
    pub plies: Vec<&'a FabLayer>,
}

impl FabDielectric<'_> {
    /// A core alone is a core; anything with a prepreg in it is bonded.
    pub fn kind(&self) -> DielectricKind {
        if self.plies.iter().any(|p| p.role == FabLayerRole::Prepreg) {
            DielectricKind::Prepreg
        } else {
            DielectricKind::Core
        }
    }

    /// The pressed plies together, to 1 nm.
    pub fn thickness_mm(&self) -> f64 {
        round(self.plies.iter().map(|p| p.pressed_mm()).sum())
    }

    /// The one Dk every ply states; `None` when they state several or none.
    pub fn dk(&self) -> Option<f64> {
        let first = self.plies.first()?.dk?;
        self.plies
            .iter()
            .all(|p| p.dk == Some(first))
            .then_some(first)
    }

    /// The plies' Dk averaged by thickness, as the impedance model averages
    /// the dielectrics between a trace and its plane. A computed value, not
    /// the fabricator's.
    pub fn average_dk(&self) -> Option<f64> {
        let thick: f64 = self.plies.iter().map(|p| p.pressed_mm()).sum();
        let mut weighted = 0.0;
        for ply in &self.plies {
            weighted += ply.pressed_mm() * ply.dk?;
        }
        (thick > 0.0).then(|| (weighted / thick * 1e4).round() / 1e4)
    }

    /// The one glass style every ply has.
    pub fn glass(&self) -> GlassStyle {
        let style = |p: &FabLayer| p.glass.as_deref().and_then(GlassStyle::from_designation);
        match self.plies.first().and_then(|p| style(p)) {
            Some(glass) if self.plies.iter().all(|p| style(p) == Some(glass)) => glass,
            _ => GlassStyle::Unspecified,
        }
    }

    /// "2116 RC54% 4.9mil + 7628 RC49% 8.6mil", as the source names them.
    pub fn materials(&self) -> String {
        self.plies
            .iter()
            .map(|p| {
                p.material_english
                    .as_deref()
                    .or(p.material.as_deref())
                    .unwrap_or(p.role.label())
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// The Dk of each ply: "3.91 and 4.4" for a mixed layer.
    pub fn dk_text(&self) -> String {
        let mut values: Vec<String> = Vec::new();
        for ply in &self.plies {
            let text = ply.dk.map_or("not stated".into(), |v| v.to_string());
            if !values.contains(&text) {
                values.push(text);
            }
        }
        values.join(" and ")
    }
}

/// A sum without binary noise.
fn round(mm: f64) -> f64 {
    (mm * 1e6).round() / 1e6
}

impl FabStackup {
    /// The file's own rules; the error names the field.
    pub fn check(&self) -> Result<()> {
        self.check_texts()?;
        self.check_layers()?;
        self.check_sums()
    }

    fn check_texts(&self) -> Result<()> {
        let text = |field: &str, text: &str| {
            if text.trim().is_empty() || text.len() > MAX_TEXT || text.chars().any(char::is_control)
            {
                return Err(bad(field, "needs a printable text"));
            }
            Ok(())
        };
        for (field, id) in [("id", &self.id), ("fabricator", &self.fabricator)] {
            if !crate::rules::fabricator::valid_id(id) {
                return Err(bad(
                    field,
                    "is lower-case letters, digits and single dashes",
                ));
            }
        }
        text("name", &self.name)?;
        for (field, value) in [
            ("source.publisher", &self.source.publisher),
            ("source.document", &self.source.document),
            ("source.url", &self.source.url),
            ("source.date", &self.source.date),
            ("nominal.basis", &self.nominal.basis),
            ("layers_sum_basis", &self.layers_sum_basis),
        ] {
            text(field, value)?;
        }
        if self.fab_profiles.is_empty()
            || self.fab_profiles.len() > MAX_LIST
            || self.layers.len() > MAX_LIST
            || self.disagreements.len() > MAX_LIST
            || self.notes.len() > MAX_LIST
        {
            return Err(bad(
                "fab_profiles",
                &format!("names at least one profile; a list holds at most {MAX_LIST}"),
            ));
        }
        for (field, list) in [
            ("disagreements", &self.disagreements),
            ("notes", &self.notes),
        ] {
            for (i, entry) in list.iter().enumerate() {
                text(&format!("{field}[{i}]"), entry)?;
            }
        }
        Ok(())
    }

    fn check_layers(&self) -> Result<()> {
        let count = self.copper_layers;
        if !(MIN_COPPER_LAYERS..=MAX_COPPER_LAYERS).contains(&count) || !count.is_multiple_of(2) {
            return Err(bad(
                "copper_layers",
                &format!("is an even number from {MIN_COPPER_LAYERS} to {MAX_COPPER_LAYERS}"),
            ));
        }
        if !(self.outer_copper_oz.is_finite() && self.outer_copper_oz > 0.0) {
            return Err(bad("outer_copper_oz", "is above 0"));
        }
        for (i, layer) in self.layers.iter().enumerate() {
            let field = |name: &str| format!("layers[{i}].{name}");
            if layer.source.trim().is_empty() || layer.source.len() > MAX_TEXT {
                return Err(bad(&field("source"), "names the row it was read from"));
            }
            let range = match layer.role {
                FabLayerRole::Copper => {
                    COPPER_UM_RANGE.start() / 1000.0..=COPPER_UM_RANGE.end() / 1000.0
                }
                FabLayerRole::Prepreg | FabLayerRole::Core => DIELECTRIC_MM_RANGE,
            };
            for (name, value) in [
                ("thickness_mm", Some(layer.thickness_mm)),
                ("after_lamination_mm", layer.after_lamination_mm),
            ] {
                if value.is_some_and(|v| !(v.is_finite() && range.contains(&v))) {
                    return Err(bad(
                        &field(name),
                        &format!("is {} to {} mm", range.start(), range.end()),
                    ));
                }
            }
            if let Some(glass) = &layer.glass
                && GlassStyle::from_designation(glass).is_none()
            {
                return Err(bad(
                    &field("glass"),
                    &format!("‘{glass}’ is not a glass style"),
                ));
            }
            match (layer.role, layer.dk) {
                (FabLayerRole::Copper, Some(_)) => {
                    return Err(bad(&field("dk"), "is a dielectric's"));
                }
                (_, Some(dk)) if !(dk.is_finite() && DK_RANGE.contains(&dk)) => {
                    return Err(bad(
                        &field("dk"),
                        &format!("is {} to {}", DK_RANGE.start(), DK_RANGE.end()),
                    ));
                }
                _ => {}
            }
        }
        let copper = |l: &FabLayer| l.role == FabLayerRole::Copper;
        let listed = self.layers.iter().filter(|l| copper(l)).count();
        if listed != count {
            return Err(bad(
                "layers",
                &format!("lists {listed} copper layers, copper_layers says {count}"),
            ));
        }
        let ends =
            self.layers.first().is_some_and(copper) && self.layers.last().is_some_and(copper);
        let touching = self.layers.windows(2).any(|w| w.iter().all(copper));
        if !ends || touching {
            return Err(bad(
                "layers",
                "starts and ends with copper and has a dielectric between copper layers",
            ));
        }
        for (i, d) in self.dielectrics().iter().enumerate() {
            if !DIELECTRIC_MM_RANGE.contains(&d.thickness_mm()) {
                return Err(bad(
                    "layers",
                    &format!(
                        "has dielectric {} at {} mm, not {} to {} mm",
                        i + 1,
                        d.thickness_mm(),
                        DIELECTRIC_MM_RANGE.start(),
                        DIELECTRIC_MM_RANGE.end()
                    ),
                ));
            }
        }
        if let Some(mask) = &self.mask {
            for (name, mil) in [
                ("mask.above_substrate_mil", mask.above_substrate_mil),
                ("mask.above_trace_mil", mask.above_trace_mil),
                ("mask.between_traces_mil", mask.between_traces_mil),
            ] {
                if !(mil.is_finite() && MASK_UM_RANGE.contains(&(mil * MIL_MM * 1000.0))) {
                    return Err(bad(
                        name,
                        &format!("is {} to {} µm", MASK_UM_RANGE.start(), MASK_UM_RANGE.end()),
                    ));
                }
            }
            if !(mask.dk.is_finite() && DK_RANGE.contains(&mask.dk)) {
                return Err(bad("mask.dk", "is a relative permittivity"));
            }
        }
        Ok(())
    }

    /// The stored sums and differences are the layers' own.
    fn check_sums(&self) -> Result<()> {
        let near = |field: &str, stored: f64, computed: f64| {
            if stored.is_finite() && (stored - computed).abs() <= SUM_MM {
                Ok(())
            } else {
                Err(bad(
                    field,
                    &format!("is {stored}, the layers give {}", round(computed)),
                ))
            }
        };
        let positive = |field: &str, mm: f64| {
            if mm.is_finite() && mm > 0.0 {
                Ok(())
            } else {
                Err(bad(field, "is above 0 mm"))
            }
        };
        let sum: f64 = self.layers.iter().map(|l| l.thickness_mm).sum();
        positive("nominal.board_mm", self.nominal.board_mm)?;
        near("layers_sum_mm", self.layers_sum_mm, sum)?;
        near(
            "layers_sum_minus_nominal_mm",
            self.layers_sum_minus_nominal_mm,
            sum - self.nominal.board_mm,
        )?;
        if let Some(stated) = &self.stated_finished {
            positive("stated_finished.value_mm", stated.value_mm)?;
            if let Some(difference) = self.layers_sum_minus_stated_finished_mm {
                near(
                    "layers_sum_minus_stated_finished_mm",
                    difference,
                    sum - stated.value_mm,
                )?;
            }
        }
        if let Some(stored) = self.layers_sum_after_lamination_mm {
            let after = self.pressed_sum_mm();
            near("layers_sum_after_lamination_mm", stored, after)?;
            if let (Some(stated), Some(difference)) = (
                &self.stated_after_lamination,
                self.layers_sum_after_lamination_minus_stated_mm,
            ) {
                positive("stated_after_lamination.value_mm", stated.value_mm)?;
                near(
                    "layers_sum_after_lamination_minus_stated_mm",
                    difference,
                    after - stated.value_mm,
                )?;
            }
        }
        let mirror = |l: &FabLayer| {
            (
                l.role,
                l.thickness_mm.to_bits(),
                l.after_lamination_mm.map(f64::to_bits),
                l.dk.map(f64::to_bits),
                l.glass.clone(),
            )
        };
        let keys: Vec<_> = self.layers.iter().map(mirror).collect();
        if keys.iter().eq(keys.iter().rev()) != self.symmetric {
            return Err(bad("symmetric", "does not say what the layers show"));
        }
        Ok(())
    }

    /// Every layer as pressed, outer foils as listed.
    pub fn pressed_sum_mm(&self) -> f64 {
        round(self.layers.iter().map(FabLayer::pressed_mm).sum())
    }

    /// The copper layers, top to bottom.
    pub fn copper(&self) -> impl Iterator<Item = &FabLayer> {
        self.layers
            .iter()
            .filter(|l| l.role == FabLayerRole::Copper)
    }

    /// The dielectrics between them, top to bottom.
    pub fn dielectrics(&self) -> Vec<FabDielectric<'_>> {
        let mut out = Vec::new();
        let mut plies = Vec::new();
        for layer in &self.layers {
            if layer.role != FabLayerRole::Copper {
                plies.push(layer);
            } else if !plies.is_empty() {
                out.push(FabDielectric {
                    plies: std::mem::take(&mut plies),
                });
            }
        }
        out
    }

    /// Its finished outer foils, µm: the stated weight.
    pub fn outer_um(&self) -> f64 {
        self.outer_copper_oz * OUNCE_UM
    }

    /// The stackup a board takes from this: pressed dielectrics, the listed
    /// inner foils, the outer foils at their finished weight, and the mask
    /// where the source states it. What the source does not give (mask and
    /// legend sides, hole wall, Dk estimates) stays `board`'s. A layer whose
    /// plies state several Dk has none, or with `average_dk` their average.
    pub fn stackup(&self, board: &Stackup, average_dk: bool) -> Result<Stackup> {
        let count = self.copper_layers;
        let copper = self
            .copper()
            .enumerate()
            .map(|(i, layer)| {
                CopperFoil::of_um(if i == 0 || i + 1 == count {
                    self.outer_um()
                } else {
                    round(layer.pressed_mm() * 1000.0)
                })
            })
            .collect();
        let dielectrics = self
            .dielectrics()
            .iter()
            .map(|d| Dielectric {
                kind: d.kind(),
                thickness_mm: d.thickness_mm(),
                glass: d.glass(),
                dk: d.dk().or(if average_dk { d.average_dk() } else { None }),
            })
            .collect();
        let mut stackup = Stackup {
            copper,
            dielectrics,
            ..board.clone()
        };
        if let Some(mask) = &self.mask {
            let um = |mil: f64| round(mil * MIL_MM * 1000.0);
            stackup = stackup.with_mask(um(mask.above_trace_mil), um(mask.above_substrate_mil))?;
        }
        stackup.check()?;
        Ok(stackup)
    }

    /// Whether `board` has exactly this build (its mask and legend aside),
    /// with or without averaged Dk.
    pub fn is_build_of(&self, board: &Stackup) -> bool {
        [false, true].into_iter().any(|average| {
            self.stackup(board, average).is_ok_and(|s| {
                s.dielectrics == board.dielectrics
                    && s.copper.len() == board.copper.len()
                    && s.copper
                        .iter()
                        .zip(&board.copper)
                        .all(|(a, b)| a.thickness_um == b.thickness_um)
            })
        })
    }
}

/// No colon after the field: "source.url: " would read as a rule code, which
/// the shown text leaves out (`diagnostics::split_rule`).
fn bad(field: &str, why: &str) -> Error {
    Error::Invalid(format!("{field} {why}"))
}

#[cfg(test)]
mod tests;

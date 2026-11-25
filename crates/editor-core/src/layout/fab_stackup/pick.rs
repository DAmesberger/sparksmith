//! Taking a fabricator's stackup on a board: the one `layout.edit_stackup`
//! it is, what that changes in words, what refuses it and what the board's
//! fab profile and net classes say of it. Nothing is applied here.
use super::FabStackup;
use crate::diagnostics::{Error, Result};
use crate::layout::limits::{mm, mm_against};
use crate::layout::restack::{self, LayerMove, StackupDraft, StackupEdit};
use crate::layout::{profile_fit, via_class};
use crate::model::{CopperLayer, Dielectric, DielectricKind, PickedStackup, Project, Stackup};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a stackup is taken.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickOptions {
    /// Where the copper of a layer the stackup lacks goes.
    pub moves: Vec<LayerMove>,
}

/// What taking a stackup does.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StackupPick {
    /// The edit that takes it.
    pub edit: StackupEdit,
    /// Every change, in words. Empty when the board has this build.
    pub changes: Vec<String>,
    /// Why the edit would be refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// What the board's fab profile and net classes say of the board then,
    /// and do not say now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// What the source leaves open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<String>,
    /// Old layers the stackup lacks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lost: Vec<CopperLayer>,
}

/// One dielectric of the build a board takes.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StackupBuildRow {
    /// "Top to Inner 1".
    pub between: String,
    pub kind: DielectricKind,
    /// Pressed, all plies.
    pub thickness_mm: f64,
    /// The sheets as the source names them.
    pub materials: String,
    /// The Dk of each sheet.
    pub dk: String,
    /// The Dk the layer takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_dk: Option<f64>,
    /// How it is calculated, when the sheets state several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calculated: Option<String>,
}

/// One listed stackup.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StackupRow {
    pub id: String,
    pub name: String,
    pub fabricator: String,
    pub copper_layers: usize,
    /// The thickness option it is listed under.
    pub nominal_mm: f64,
    /// Its layers added as listed.
    pub layers_sum_mm: f64,
    /// Its layers added as pressed.
    pub pressed_sum_mm: f64,
    /// The finished thickness the source states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated_finished_mm: Option<f64>,
    /// Dielectrics whose sheets state several Dk: theirs is calculated.
    pub calculated_dk: usize,
    /// Where the source's own texts give other values.
    pub disagreements: usize,
    /// The board has these layers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub board: bool,
    /// The board's layers were picked from it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub picked: bool,
}

/// `layout.stackups`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StackupList {
    pub stackups: Vec<StackupRow>,
    /// The one the board's layers were picked from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<PickedStackup>,
    /// Stackups of the library this list leaves out.
    pub left_out: usize,
}

/// `layout.stackups {id}`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StackupReport {
    /// As published, with every source.
    pub stackup: FabStackup,
    /// The build a board takes from it, top to bottom.
    pub build: Vec<StackupBuildRow>,
    pub pick: StackupPick,
}

/// Which stackups a list shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackupFilter {
    pub copper_layers: Option<usize>,
    pub board_mm: Option<f64>,
    pub fabricator: Option<String>,
    pub all: bool,
}

impl StackupFilter {
    /// The maker shown: the one named, else the fab profile's, else every.
    fn maker<'a>(&'a self, project: &'a Project) -> Option<&'a str> {
        if self.all {
            return self.fabricator.as_deref();
        }
        self.fabricator.as_deref().or_else(|| {
            project
                .rules
                .profile
                .as_ref()
                .and_then(|p| p.fabricator.as_deref())
        })
    }

    fn shows(&self, project: &Project, stackup: &FabStackup) -> bool {
        let count = match (self.copper_layers, self.all) {
            (None, true) => None,
            (None, false) => Some(project.stackup.copper.len()),
            (given, _) => given,
        };
        count.is_none_or(|n| n == stackup.copper_layers)
            && self
                .board_mm
                .is_none_or(|t| (t - stackup.nominal.board_mm).abs() < 1e-6)
            && self
                .maker(project)
                .is_none_or(|maker| maker == stackup.fabricator)
    }
}

/// The stackups of `library` that `filter` shows for `project`, by maker,
/// layers, thickness and name.
pub fn list(project: &Project, library: &[FabStackup], filter: &StackupFilter) -> StackupList {
    let mut shown: Vec<&FabStackup> = library
        .iter()
        .filter(|s| filter.shows(project, s))
        .collect();
    shown.sort_by(|a, b| {
        a.fabricator
            .cmp(&b.fabricator)
            .then(a.copper_layers.cmp(&b.copper_layers))
            .then(a.nominal.board_mm.total_cmp(&b.nominal.board_mm))
            .then(a.name.cmp(&b.name))
    });
    StackupList {
        picked: project.stackup.picked().cloned(),
        left_out: library.len() - shown.len(),
        stackups: shown
            .into_iter()
            .map(|s| StackupRow {
                id: s.id.clone(),
                name: s.name.clone(),
                fabricator: s.fabricator.clone(),
                copper_layers: s.copper_layers,
                nominal_mm: s.nominal.board_mm,
                layers_sum_mm: s.layers_sum_mm,
                pressed_sum_mm: s.pressed_sum_mm(),
                stated_finished_mm: s.stated_finished.as_ref().map(|t| t.value_mm),
                calculated_dk: s
                    .dielectrics()
                    .iter()
                    .filter(|d| !d.sheets().is_empty())
                    .count(),
                disagreements: s.disagreements.len(),
                board: s.is_build_of(&project.stackup),
                picked: project.stackup.picked().is_some_and(|p| p.id == s.id),
            })
            .collect(),
    }
}

/// One stackup as published, its build and what taking it does.
pub fn report(
    project: &Project,
    stackup: &FabStackup,
    options: &PickOptions,
) -> Result<StackupReport> {
    let pick = pick(project, stackup, options)?;
    let build = stackup
        .dielectrics()
        .iter()
        .zip(&pick.edit.stackup.dielectrics)
        .enumerate()
        .map(|(i, (listed, taken))| StackupBuildRow {
            between: between(&pick.edit.stackup, i),
            kind: taken.kind,
            thickness_mm: taken.thickness_mm,
            materials: listed.materials(),
            dk: listed.dk_text(),
            board_dk: taken.dk,
            calculated: taken.calculated_text(),
        })
        .collect();
    Ok(StackupReport {
        stackup: stackup.clone(),
        build,
        pick,
    })
}

/// What taking `stackup` does to `project`.
pub fn pick(project: &Project, stackup: &FabStackup, options: &PickOptions) -> Result<StackupPick> {
    let mut draft = StackupDraft::of(&project.stackup);
    draft.use_stackup(stackup.stackup(&project.stackup)?);
    let lost = draft.removed();
    let mut dropped = BTreeMap::new();
    for m in &options.moves {
        if !lost.contains(&m.from) {
            return Err(Error::Invalid(format!(
                "layout.use_stackup: {} stays, so its copper does not move; moves are for layers \
                 the stackup lacks",
                m.from.name()
            )));
        }
        dropped.insert(m.from, m.to);
    }
    let edit = draft.edit(&dropped);
    let mut then = project.clone();
    let refused = restack::apply(&mut then, edit.clone())
        .err()
        .map(|e| e.to_string());
    if refused.is_some() {
        then = project.clone();
        then.stackup = edit.stackup.clone();
    }
    let now = said(project);
    let mut conflicts = made_for(project, stackup);
    conflicts.extend(said(&then).into_iter().filter(|m| !now.contains(m)));
    Ok(StackupPick {
        changes: changes(project, &draft, &edit),
        refused,
        conflicts,
        open: open(stackup, &edit.stackup),
        lost,
        edit,
    })
}

/// What the fab profile and the net classes say of `project`.
fn said(project: &Project) -> Vec<String> {
    profile_fit::findings(project)
        .into_iter()
        .chain(via_class::findings(project))
        .map(|f| f.message)
        .collect()
}

/// Whether the stackup is for the board's fab profile.
fn made_for(project: &Project, stackup: &FabStackup) -> Vec<String> {
    let Some(profile) = &project.rules.profile else {
        return Vec::new();
    };
    let Some(maker) = &profile.fabricator else {
        return Vec::new();
    };
    if *maker != stackup.fabricator {
        vec![format!(
            "{} is a stackup of {}; the board's fab profile is {}",
            stackup.name, stackup.source.publisher, profile.name
        )]
    } else if !stackup.fab_profiles.contains(&profile.id) {
        vec![format!(
            "{} is not listed for the fab profile {}",
            stackup.name, profile.name
        )]
    } else {
        Vec::new()
    }
}

/// "Top to Inner 1": the foils either side of dielectric `index`.
fn between(stackup: &Stackup, index: usize) -> String {
    let layers = stackup.layers();
    let name = |i: usize| layers.get(i).map_or_else(String::new, |l| l.name());
    format!("{} to {}", name(index), name(index + 1))
}

fn dielectric(d: &Dielectric) -> String {
    let kind = match d.kind {
        DielectricKind::Core => "core",
        DielectricKind::Prepreg => "prepreg",
    };
    let glass = d
        .glass
        .designation()
        .map_or_else(String::new, |g| format!(" {g}"));
    let dk =
        d.dk.map_or_else(|| "no Dk entered".into(), |v| format!("Dk {v}"));
    let calculated = d
        .calculated_text()
        .map_or_else(String::new, |_| " (calculated)".into());
    format!("{kind}{glass}, {} mm, {dk}{calculated}", mm(d.thickness_mm))
}

/// Every change of `edit`, in words.
fn changes(project: &Project, draft: &StackupDraft, edit: &StackupEdit) -> Vec<String> {
    let (old, new) = (&project.stackup, &edit.stackup);
    let mut out = Vec::new();
    if old.copper.len() != new.copper.len() {
        out.push(format!(
            "{} copper layers become {}",
            old.copper.len(),
            new.copper.len()
        ));
    }
    for layer in &edit.removed {
        match edit.moves.iter().find(|m| m.from == *layer) {
            Some(m) => out.push(format!(
                "{} goes; its copper moves to {}",
                layer.name(),
                m.to.name()
            )),
            None => out.push(format!("{} goes", layer.name())),
        }
    }
    for m in edit
        .moves
        .iter()
        .filter(|m| !edit.removed.contains(&m.from))
    {
        out.push(format!("{} becomes {}", m.from.name(), m.to.name()));
    }
    for (i, (layer, foil)) in new.layers().iter().zip(&new.copper).enumerate() {
        let was = draft
            .origin(i)
            .and_then(|o| old.index(o))
            .and_then(|k| old.copper.get(k));
        match was {
            None => out.push(format!("{} is new, {} µm", layer.name(), foil.thickness_um)),
            Some(was) if was.thickness_um != foil.thickness_um => out.push(format!(
                "{} copper {} µm becomes {} µm",
                layer.name(),
                was.thickness_um,
                foil.thickness_um
            )),
            Some(_) => {}
        }
    }
    let same_count = old.dielectrics.len() == new.dielectrics.len();
    for (i, d) in new.dielectrics.iter().enumerate() {
        match old.dielectrics.get(i).filter(|_| same_count) {
            Some(was) if was == d => {}
            Some(was) => out.push(format!(
                "{}: {} becomes {}",
                between(new, i),
                dielectric(was),
                dielectric(d)
            )),
            None => out.push(format!("{}: {}", between(new, i), dielectric(d))),
        }
    }
    match (old.picked(), new.picked()) {
        (was, Some(now)) if was.is_none_or(|w| w.id != now.id || w.name != now.name) => {
            out.push(format!("The stackup is named {}", now.name));
        }
        (Some(was), None) => out.push(format!("The stackup is no longer named {}", was.name)),
        _ => {}
    }
    let (was, is) = (old.thickness_mm(), new.thickness_mm());
    if (was - is).abs() > 1e-6 {
        out.push(format!(
            "Board {} mm becomes {} mm",
            mm(was),
            mm_against(is, was)
        ));
    }
    if old.mask_um != new.mask_um || old.mask_laminate_nm != new.mask_laminate_nm {
        out.push(format!(
            "Solder mask {} µm over copper and {} µm over laminate becomes {} and {} µm",
            mm(old.mask_um),
            mm(old.mask_laminate_nm.to_mm_f64() * 1000.0),
            mm(new.mask_um),
            mm(new.mask_laminate_nm.to_mm_f64() * 1000.0)
        ));
    }
    out
}

/// What the source leaves open for a board.
fn open(stackup: &FabStackup, taken: &Stackup) -> Vec<String> {
    let mut out = Vec::new();
    for (i, d) in taken.dielectrics.iter().enumerate() {
        if let (Some(how), Some(dk)) = (d.calculated_text(), d.dk) {
            out.push(format!("{}: Dk {dk}. {how}", between(taken, i)));
        }
    }
    for (end, layer) in [
        ("Top", stackup.copper().next()),
        ("Bottom", stackup.copper().last()),
    ] {
        if let Some(layer) = layer
            && (layer.thickness_mm * 1000.0 - stackup.outer_um()).abs() > 1e-6
        {
            out.push(format!(
                "{end} copper is listed {} mm thick ({}) and finishes at {} oz; the board takes \
                 {} µm, as fabricators count an ounce",
                layer.thickness_mm,
                layer.foil_text.as_deref().unwrap_or("no foil named"),
                stackup.outer_copper_oz,
                stackup.outer_um()
            ));
        }
    }
    if let Some(after) = stackup.layers_sum_after_lamination_mm {
        out.push(format!(
            "The layers add to {} mm before pressing and {after} mm after; the board takes the \
             pressed ones",
            stackup.layers_sum_mm
        ));
    }
    out
}

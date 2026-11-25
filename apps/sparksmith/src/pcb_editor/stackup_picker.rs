//! The Stackup dialog's fabricators' stackups: those for the board's layer
//! count, thickness and fab profile, each with its stack to scale, its
//! values and sources, and what taking it changes, said before it is taken.
//! Taking one stages it in the dialog's draft; OK sends the dialog's one
//! `layout.edit_stackup`, one undo step.
use super::stack_section;
use crate::kit;
use crate::refusal;
use egui::Ui;
use sparksmith_core::layout::fab_stackup::{
    self, FabStackup, PickOptions, StackupFilter, StackupReport, StackupRow,
};
use sparksmith_core::{DielectricKind, Engine, Project, Stackup};
use sparksmith_ui::ui_kit::{self, Button};

/// What the picker shows.
pub(super) struct Picker {
    library: Result<Vec<FabStackup>, String>,
    copper_layers: usize,
    /// A thickness option in µm; `None` for every one.
    thickness: Option<i64>,
    /// Every maker, not only the fab profile's.
    every_maker: bool,
    all_rows: bool,
    selected: Option<String>,
    /// The selected one's report, and what it was made for.
    shown: Option<(Key, Result<StackupReport, String>)>,
}

/// The revision and stackup a report is for.
type Key = (u64, String);

/// What the picker asks of the dialog.
pub(super) enum Picked {
    Back,
    /// Stage this build.
    Take(Box<Stackup>),
}

fn um(mm: f64) -> i64 {
    (mm * 1000.0).round() as i64
}

impl Picker {
    pub(super) fn new(project: &Project) -> Self {
        let library = sparksmith_io::fab_stackups::shipped().map_err(|e| e.to_string());
        let mut picker = Self {
            library,
            copper_layers: project.stackup.copper.len(),
            thickness: None,
            every_maker: false,
            all_rows: false,
            selected: None,
            shown: None,
        };
        // The option nearest the board's thickness; the row shows it.
        let board = um(project.stackup.thickness_mm());
        picker.thickness = picker
            .thicknesses(project)
            .into_iter()
            .min_by_key(|t| (t - board).abs());
        picker
    }

    fn rows(&self, project: &Project, thickness: Option<i64>) -> Vec<StackupRow> {
        let Ok(library) = &self.library else {
            return Vec::new();
        };
        let filter = StackupFilter {
            copper_layers: Some(self.copper_layers),
            all: self.every_maker,
            ..StackupFilter::default()
        };
        fab_stackup::list(project, library, &filter)
            .stackups
            .into_iter()
            .filter(|row| thickness.is_none_or(|t| um(row.nominal_mm) == t))
            .collect()
    }

    /// The thickness options of the listed layer count and makers.
    fn thicknesses(&self, project: &Project) -> Vec<i64> {
        let mut out: Vec<i64> = self
            .rows(project, None)
            .iter()
            .map(|r| um(r.nominal_mm))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The selected stackup's report, kept until the board or the choice
    /// changes.
    fn report(&mut self, engine: &Engine) -> Option<&Result<StackupReport, String>> {
        let id = self.selected.clone()?;
        let key = (engine.revision(), id);
        if self.shown.as_ref().is_none_or(|(k, _)| *k != key) {
            let options = PickOptions::default();
            let made = self.library.as_ref().map_err(Clone::clone).and_then(|l| {
                let stackup =
                    sparksmith_io::fab_stackups::find(l, &key.1).map_err(|e| e.to_string())?;
                fab_stackup::report(engine.project(), stackup, &options)
                    .map_err(|e| refusal::shown_error(&e))
            });
            self.shown = Some((key, made));
        }
        self.shown.as_ref().map(|(_, report)| report)
    }

    /// The picker in place of the dialog's table.
    pub(super) fn show(&mut self, ui: &mut Ui, engine: &Engine) -> Option<Picked> {
        let project = engine.project();
        let mut out = None;
        self.filters(ui, project);
        let rows = self.rows(project, self.thickness);
        if rows.is_empty() {
            match &self.library {
                Err(error) => kit::hint(ui, "No fabricators' stackups", error),
                Ok(_) => kit::hint(
                    ui,
                    "No stackup is published for these layers and thickness",
                    "",
                ),
            };
        }
        let publisher = |row: &StackupRow| {
            self.library
                .as_ref()
                .ok()
                .and_then(|l| l.iter().find(|s| s.id == row.id))
                .map_or_else(|| row.fabricator.clone(), |s| s.source.publisher.clone())
        };
        let names: Vec<(String, String)> = rows
            .iter()
            .map(|row| {
                let board = match (row.picked, row.board) {
                    (true, _) => " · this board's",
                    (false, true) => " · this board's layers",
                    (false, false) => "",
                };
                let secondary = format!(
                    "{} · {} mm{board}",
                    publisher(row),
                    kit::number_text(row.nominal_mm)
                );
                (row.name.clone(), secondary)
            })
            .collect();
        let max = if self.all_rows {
            rows.len()
        } else {
            kit::LIST_MAX_ROWS
        };
        let mut clicked = None;
        let listed = kit::list_frame(ui, rows.len(), max, |ui, i| {
            let (Some(row), Some((name, secondary))) = (rows.get(i), names.get(i)) else {
                return;
            };
            let tree = kit::Row {
                icon: "plane",
                label: name,
                secondary,
                selected: self.selected.as_deref() == Some(row.id.as_str()),
                tip: "Show its layers, sources and what it changes",
                ..Default::default()
            };
            if kit::tree_row(ui, ui.scope_id().with(("stackup", &row.id)), &tree).clicked {
                clicked = Some(row.id.clone());
            }
        });
        if listed.show_all {
            self.all_rows = true;
        }
        if clicked.is_some() {
            self.selected = clicked;
        }
        let shown = self.report(engine).cloned();
        kit::actions(ui, |ui| {
            let ready = matches!(&shown, Some(Ok(_)));
            let take = ui_kit::button(ui, "Use this stackup", Button::Enabled(ready))
                .on_hover_text("Shows it in the dialog; OK applies it as one step");
            if take.clicked()
                && let Some(Ok(report)) = &shown
            {
                out = Some(Picked::Take(Box::new(report.pick.edit.stackup.clone())));
            }
            if ui_kit::button(ui, "Back", Button::Enabled(true)).clicked() {
                out = Some(Picked::Back);
            }
        });
        match &shown {
            Some(Ok(report)) => details(ui, report),
            Some(Err(error)) => {
                kit::warning_row(ui, error);
            }
            None if !rows.is_empty() => {
                kit::hint(ui, "Select a stackup to see it", "");
            }
            None => {}
        }
        out
    }

    fn filters(&mut self, ui: &mut Ui, project: &Project) {
        let mut counts: Vec<usize> = self
            .library
            .iter()
            .flatten()
            .map(|s| s.copper_layers)
            .chain([self.copper_layers])
            .collect();
        counts.sort_unstable();
        counts.dedup();
        let counts: Vec<(usize, String)> = counts
            .into_iter()
            .map(|n| (n, format!("{n} copper layers")))
            .collect();
        let mut changed = kit::choice_row(ui, "Layers", &mut self.copper_layers, &counts);
        let maker = project
            .rules
            .profile
            .as_ref()
            .filter(|p| p.fabricator.is_some());
        if let Some(profile) = maker {
            changed |= kit::check_row(ui, "Every maker", &mut self.every_maker);
            if !self.every_maker {
                kit::hint(ui, &format!("For the fab profile {}", profile.name), "");
            }
        }
        let options: Vec<(Option<i64>, String)> = std::iter::once((None, "Any".to_owned()))
            .chain(self.thicknesses(project).into_iter().map(|t| {
                (
                    Some(t),
                    format!("{} mm", kit::number_text(t as f64 / 1000.0)),
                )
            }))
            .collect();
        if !options.iter().any(|(t, _)| *t == self.thickness) {
            self.thickness = None;
        }
        changed |= kit::choice_row(ui, "Thickness", &mut self.thickness, &options);
        if changed {
            self.selected = None;
            self.all_rows = false;
        }
    }
}

/// One stackup: its stack, what taking it does and what it is read from.
fn details(ui: &mut Ui, report: &StackupReport) {
    let fab = &report.stackup;
    let pick = &report.pick;
    let bands = stack_section::bands(&pick.edit.stackup);
    stack_section::show(ui, &bands, &[]);
    for text in pick.refused.iter().chain(&pick.conflicts) {
        kit::warning_row(ui, text);
    }
    let mm = |v: f64| format!("{} mm", kit::number_text(v));
    kit::value_row(ui, "Listed as", &mm(fab.nominal.board_mm));
    kit::value_row(ui, "Layers add to", &mm(fab.layers_sum_mm));
    if let Some(after) = fab.layers_sum_after_lamination_mm {
        kit::value_row(ui, "After pressing", &mm(after));
    }
    if let Some(stated) = &fab.stated_after_lamination {
        kit::value_row(
            ui,
            "Stated pressed",
            &format!("{} · {}", mm(stated.value_mm), stated.text),
        );
    }
    if let Some(stated) = &fab.stated_finished {
        kit::value_row(
            ui,
            "Stated finished",
            &format!("{} · {}", mm(stated.value_mm), stated.text),
        );
    }
    kit::section(
        ui,
        "stackup-changes",
        "Changes",
        true,
        Some(pick.changes.len()),
        |ui| {
            if pick.changes.is_empty() {
                kit::note(ui, "The board has this stackup");
            }
            for change in &pick.changes {
                kit::note(ui, change);
            }
        },
    );
    kit::section(
        ui,
        "stackup-values",
        "Dielectrics",
        false,
        Some(report.build.len()),
        |ui| {
            for row in &report.build {
                let kind = match row.kind {
                    DielectricKind::Core => "Core",
                    DielectricKind::Prepreg => "Prepreg",
                };
                kit::value_row(
                    ui,
                    &row.between,
                    &format!(
                        "{kind} {} · Dk {} · {}",
                        mm(row.thickness_mm),
                        row.dk,
                        row.materials
                    ),
                );
                if let (Some(how), Some(dk)) = (&row.calculated, row.board_dk) {
                    kit::note(ui, &format!("Dk {}. {how}", kit::number_text(dk)));
                }
            }
            if let Some(mask) = &fab.mask {
                kit::value_row(
                    ui,
                    "Solder mask",
                    &format!(
                        "{} mil over copper, {} mil over laminate, Dk {}",
                        kit::number_text(mask.above_trace_mil),
                        kit::number_text(mask.above_substrate_mil),
                        kit::number_text(mask.dk)
                    ),
                );
            }
        },
    );
    if !pick.open.is_empty() {
        kit::section(
            ui,
            "stackup-open",
            "Not settled by the source",
            true,
            Some(pick.open.len()),
            |ui| {
                for text in &pick.open {
                    kit::note(ui, text);
                }
            },
        );
    }
    if !fab.disagreements.is_empty() {
        kit::section(
            ui,
            "stackup-disagreements",
            "Sources disagree",
            false,
            Some(fab.disagreements.len()),
            |ui| {
                for text in &fab.disagreements {
                    kit::note(ui, text);
                }
            },
        );
    }
    kit::section(ui, "stackup-source", "Source", false, None, |ui| {
        kit::value_row(ui, "Publisher", &fab.source.publisher);
        kit::value_row(ui, "Document", &fab.source.document);
        kit::value_row(ui, "Address", &fab.source.url);
        kit::value_row(ui, "Read", &fab.source.date);
        for text in std::iter::once(&fab.source.notes)
            .chain(&fab.notes)
            .filter(|t| !t.is_empty())
        {
            kit::note(ui, text);
        }
    });
}

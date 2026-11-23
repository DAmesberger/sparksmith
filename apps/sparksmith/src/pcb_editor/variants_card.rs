//! The Assembly variants card (docs/assembly-variants.md): the variants
//! with Add, Duplicate, Rename and Delete, the active one, and a bounded
//! table of the parts with their population and fitted Part in the
//! variant chosen in the list (or the default).
//!
//! Every change is one `variant.*` command and one undo step, as the card
//! commits live (docs/sidebar-ia.md, principle 13). What each variant
//! fits comes from the core's `variant.list` and `variant.substitutes`
//! queries; nothing here decides it.
use super::PcbEditor;
use crate::kit::{self, card::CardResponse};
use crate::refusal::{self, Origin};
use egui::{FontId, Ui};
use sparksmith_core::instances::variants::{
    Substitute, VariantCommand, VariantRow, VariantsReport,
};
use sparksmith_core::model::Population;
use sparksmith_core::{Engine, Id, Operation, PartRef, Query, QueryResult};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{self, Button};
use std::collections::HashMap;
use std::sync::Arc;

/// The open card's own state.
#[derive(Default)]
pub struct VariantsCard {
    /// The variant the Variant section and the parts table edit; `None`
    /// is the default.
    pub(super) selected: Option<Id>,
    /// A name just added or duplicated: selected once the list has it.
    adding: Option<String>,
    /// The Name field's text while it is edited, for the selected variant.
    name: Option<(Id, String)>,
    variants_all: bool,
    parts_all: bool,
    /// `variant.list` of one revision.
    report: Option<(u64, Arc<VariantsReport>)>,
    /// `variant.substitutes` per part, of one revision.
    substitutes: (u64, HashMap<Id, Arc<[Substitute]>>),
}

impl VariantsCard {
    fn report(&mut self, engine: &Engine) -> Option<Arc<VariantsReport>> {
        let revision = engine.revision();
        if let Some((at, report)) = &self.report
            && *at == revision
        {
            return Some(Arc::clone(report));
        }
        let QueryResult::Variants(report) = engine.query(Query::Variants {}).ok()? else {
            return None;
        };
        let report = Arc::new(report);
        self.report = Some((revision, Arc::clone(&report)));
        Some(report)
    }

    fn substitutes(&mut self, engine: &Engine, instance: Id) -> Arc<[Substitute]> {
        if self.substitutes.0 != engine.revision() {
            self.substitutes = (engine.revision(), HashMap::new());
        }
        Arc::clone(self.substitutes.1.entry(instance).or_insert_with(|| {
            match engine.query(Query::Substitutes { instance }) {
                Ok(QueryResult::Substitutes(list)) => list.into(),
                _ => Arc::from(Vec::new()),
            }
        }))
    }
}

/// A variant name no other variant has: "Variant 1", "Variant 2", ….
fn fresh_name(report: &VariantsReport, stem: &str) -> String {
    let taken = |name: &str| {
        report
            .variants
            .iter()
            .any(|v| v.name.eq_ignore_ascii_case(name))
    };
    if !taken(stem) && stem != "Variant" {
        return stem.to_owned();
    }
    (1..=report.variants.len() + 1)
        .map(|n| format!("{stem} {n}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| stem.to_owned())
}

/// "2 not populated · 1 substitute", or what the variant leaves as is.
fn counts(not_populated: usize, substituted: usize) -> String {
    match (not_populated, substituted) {
        (0, 0) => "As the default".to_owned(),
        (n, 0) => format!("{n} not populated"),
        (0, s) => format!("{s} substitute{}", if s == 1 { "" } else { "s" }),
        (n, s) => format!(
            "{n} not populated · {s} substitute{}",
            if s == 1 { "" } else { "s" }
        ),
    }
}

/// The command a population check in `variant` (or the default) sends:
/// in a variant, the default's own value clears the override.
fn population_edit(row: &VariantRow, variant: Option<Id>, populated: bool) -> Operation {
    let population = if populated {
        Population::Populated
    } else {
        Population::NotPopulated
    };
    let population = match variant {
        Some(_) if row.population == population => None,
        _ => Some(population),
    };
    Operation::Variant(VariantCommand::SetPopulation {
        instances: vec![row.instance],
        variant,
        population,
    })
}

impl PcbEditor {
    /// Open the Assembly variants card.
    pub fn open_variants(&mut self) {
        self.setup = None;
        self.stackup = None;
        self.rules = None;
        self.signals = None;
        self.net_classes = None;
        self.production = None;
        self.end_route();
        if self.variants.is_none() {
            self.variants = Some(VariantsCard::default());
        }
    }

    /// The Assembly variants card in the subject slot.
    pub(super) fn variants_card(
        &mut self,
        ui: &mut Ui,
        engine: &Engine,
    ) -> CardResponse<Operation> {
        let Some(mut card) = self.variants.take() else {
            return CardResponse::output(None);
        };
        let Some(report) = card.report(engine) else {
            return CardResponse::output(None);
        };
        // A variant deleted (or undone) leaves the default chosen; one just
        // added is chosen once it is listed.
        if card
            .selected
            .is_some_and(|id| !report.variants.iter().any(|v| v.id == id))
        {
            card.selected = None;
        }
        if let Some(name) = &card.adding
            && let Some(added) = report.variants.iter().find(|v| &v.name == name)
        {
            card.selected = Some(added.id);
            card.adding = None;
        }
        let inspector = kit::card::Inspector {
            header: crate::shell::Header::new("list", "Assembly variants", "Assembly"),
            subject: egui::Id::unique("pcb-variants"),
            footer: true,
            issues: None,
        };
        let mut close = false;
        let response = kit::card::card(ui, &inspector, |ui, region| match region {
            kit::card::Region::Body => {
                let listed = variants_section(ui, &report, &mut card);
                let named = name_section(ui, &report, &mut card);
                let parts = parts_section(ui, engine, &report, &mut card);
                listed.or(named).or(parts)
            }
            kit::card::Region::Footer => {
                let (operation, done) = footer(ui, &card);
                close |= done;
                operation
            }
        });
        if !close {
            self.variants = Some(card);
        }
        response
    }
}

/// VARIANTS: the default and each variant (a click chooses the one the
/// table edits), Add variant and Duplicate, and the active variant.
fn variants_section(
    ui: &mut Ui,
    report: &VariantsReport,
    card: &mut VariantsCard,
) -> Option<Operation> {
    let mut operation = None;
    kit::section(ui, "variants", "Variants", true, None, |ui| {
        let off_by_default = report
            .parts
            .iter()
            .filter(|p| !p.population.is_populated())
            .count();
        let len = report.variants.len() + 1;
        let max = if card.variants_all {
            len
        } else {
            kit::LIST_MAX_ROWS
        };
        let shown = kit::list_frame(ui, len, max, |ui, i| {
            let (id, name, secondary) = match i.checked_sub(1) {
                None => (
                    None,
                    "Default",
                    match off_by_default {
                        0 => "All populated".to_owned(),
                        n => format!("{n} not populated"),
                    },
                ),
                Some(k) => {
                    let Some(v) = report.variants.get(k) else {
                        return;
                    };
                    (
                        Some(v.id),
                        v.name.as_str(),
                        counts(v.not_populated, v.substituted),
                    )
                }
            };
            let secondary = if report.active == id {
                format!("Active · {secondary}")
            } else {
                secondary
            };
            let row = kit::Row {
                icon: "list",
                label: name,
                secondary: &secondary,
                selected: card.selected == id,
                tip: "Edit this variant's parts",
                ..Default::default()
            };
            if kit::tree_row(ui, ui.scope_id().with(("variant", id)), &row).clicked {
                card.selected = id;
                card.name = None;
            }
        });
        card.variants_all |= shown.show_all;
        kit::actions(ui, |ui| {
            let selected = card
                .selected
                .and_then(|id| report.variants.iter().find(|v| v.id == id));
            if ui_kit::button(
                ui,
                "Duplicate",
                if selected.is_some() {
                    Button::Small
                } else {
                    Button::Enabled(false)
                },
            )
            .on_disabled_hover_text("Choose a variant to duplicate")
            .clicked()
                && let Some(v) = selected
            {
                let name = fresh_name(report, &format!("{} copy", v.name));
                refusal::mark(ui.ctx(), Origin::Command);
                card.adding = Some(name.clone());
                operation = Some(Operation::Variant(VariantCommand::Add {
                    name,
                    copy_of: Some(v.id),
                }));
            }
            if ui_kit::button(ui, "Add variant", Button::Small).clicked() {
                let name = fresh_name(report, "Variant");
                refusal::mark(ui.ctx(), Origin::Command);
                card.adding = Some(name.clone());
                operation = Some(Operation::Variant(VariantCommand::Add {
                    name,
                    copy_of: None,
                }));
            }
        });
        let mut options = vec![(None, "Default".to_owned())];
        options.extend(report.variants.iter().map(|v| (Some(v.id), v.name.clone())));
        let mut active = report.active;
        if kit::choice_row(ui, "Active", &mut active, &options) && active != report.active {
            refusal::mark(ui.ctx(), Origin::Command);
            operation = Some(Operation::Variant(VariantCommand::SetActive {
                variant: active,
            }));
        }
        kit::hint(
            ui,
            "The 3D view, BOM and assembly files follow the active variant",
            "",
        );
    });
    operation
}

/// VARIANT: the chosen variant's name, renamed when the field is left.
fn name_section(
    ui: &mut Ui,
    report: &VariantsReport,
    card: &mut VariantsCard,
) -> Option<Operation> {
    let variant = card
        .selected
        .and_then(|id| report.variants.iter().find(|v| v.id == id))?;
    let mut operation = None;
    kit::section(ui, "variant", "Variant", true, None, |ui| {
        let mut text = match &card.name {
            Some((id, text)) if *id == variant.id => text.clone(),
            _ => variant.name.clone(),
        };
        let blank = text.trim().is_empty();
        let response = kit::row(ui, "Name", |ui| {
            kit::text_field(ui, "Name", &mut text, "", blank)
        });
        if blank {
            kit::field_error(ui, &response, "A variant needs a name", true);
        }
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if response.has_focus() {
            card.name = Some((variant.id, text));
            return;
        }
        card.name = None;
        let name = text.trim();
        if response.lost_focus() && !escape && !blank && name != variant.name {
            refusal::mark(ui.ctx(), Origin::Command);
            operation = Some(Operation::Variant(VariantCommand::Rename {
                variant: variant.id,
                name: name.to_owned(),
            }));
        }
    });
    operation
}

fn jumper(row: &VariantRow) -> bool {
    matches!(
        row.tie_form,
        Some(sparksmith_core::library::tie::TieForm::SolderJumper { .. })
    )
}

fn population_cell(
    ui: &mut Ui,
    row: &VariantRow,
    variant: Option<Id>,
    population: Population,
    name: &str,
) -> Option<Operation> {
    let mut on = population.is_populated();
    let changed = if jumper(row) {
        kit::dropdown(
            ui,
            name,
            &mut on,
            &[(true, "Closed".into()), (false, "Open".into())],
        )
    } else {
        kit::cell_check(ui, name, &mut on)
    };
    changed.then(|| {
        refusal::mark(ui.ctx(), Origin::Command);
        population_edit(row, variant, on)
    })
}

/// PARTS: each bought part with its value; for the default, whether it is
/// populated; for a variant, the default's state, whether the variant
/// populates it and the Part it fits.
fn parts_section(
    ui: &mut Ui,
    engine: &Engine,
    report: &VariantsReport,
    card: &mut VariantsCard,
) -> Option<Operation> {
    let t = Tokens::get();
    let variant = card
        .selected
        .and_then(|id| report.variants.iter().find(|v| v.id == id));
    let mut operation = None;
    let title = match variant {
        Some(v) => format!("Parts in {}", v.name),
        None => "Parts by default".to_owned(),
    };
    kit::section(
        ui,
        "variant-parts",
        &title,
        true,
        Some(report.parts.len()),
        |ui| {
            if report.parts.is_empty() {
                kit::hint(ui, "No parts to populate yet", "");
                return;
            }
            let max = if card.parts_all {
                report.parts.len()
            } else {
                kit::LIST_MAX_ROWS
            };
            let text = |ui: &mut Ui, key: (Id, usize), text: &str, ink: egui::Color32| {
                let rect = ui.max_rect();
                kit::text::line(
                    ui,
                    ui.scope_id().with(key),
                    rect,
                    text,
                    "",
                    FontId::proportional(t.font_label),
                    ink,
                );
            };
            let shown = match variant {
                None => {
                    let columns = [
                        kit::TableColumn {
                            title: "Ref",
                            weight: 1.0,
                        },
                        kit::TableColumn {
                            title: "Value",
                            weight: 2.0,
                        },
                        kit::TableColumn {
                            title: if report.parts.iter().any(jumper) {
                                "State"
                            } else {
                                "Populated"
                            },
                            weight: 1.2,
                        },
                    ];
                    kit::table_bounded(
                        ui,
                        &columns,
                        report.parts.len(),
                        max,
                        |ui, r, column| {
                            let Some(row) = report.parts.get(r) else {
                                return;
                            };
                            match column {
                                0 => text(ui, (row.instance, 0), &row.designator, t.text),
                                1 => {
                                    let value = row.value.as_deref().unwrap_or("");
                                    text(ui, (row.instance, 1), value, t.text_dim);
                                }
                                _ => {
                                    let name = format!("{} populated", row.designator);
                                    operation =
                                        population_cell(ui, row, None, row.population, &name)
                                            .or(operation.take());
                                }
                            }
                        },
                        None,
                    )
                }
                Some(v) => {
                    let caption = format!("In {}", v.name);
                    let columns = [
                        kit::TableColumn {
                            title: "Ref",
                            weight: 0.9,
                        },
                        kit::TableColumn {
                            title: "Default",
                            weight: 0.9,
                        },
                        kit::TableColumn {
                            title: &caption,
                            weight: 0.9,
                        },
                        kit::TableColumn {
                            title: "Part",
                            weight: 2.4,
                        },
                    ];
                    let rows: Vec<_> = report
                        .parts
                        .iter()
                        .take(max)
                        .filter(|p| !jumper(p))
                        .map(|p| p.instance)
                        .collect();
                    let substitutes: HashMap<Id, Arc<[Substitute]>> = rows
                        .iter()
                        .map(|id| (*id, card.substitutes(engine, *id)))
                        .collect();
                    kit::table_bounded(
                        ui,
                        &columns,
                        report.parts.len(),
                        max,
                        |ui, r, column| {
                            let Some(row) = report.parts.get(r) else {
                                return;
                            };
                            let Some(cell) = row.variants.iter().find(|c| c.variant == v.id) else {
                                return;
                            };
                            match column {
                                0 => text(ui, (row.instance, 0), &row.designator, t.text),
                                1 => {
                                    let state = match (jumper(row), row.population.is_populated()) {
                                        (true, true) => "Closed",
                                        (true, false) => "Open",
                                        (false, true) => "Yes",
                                        (false, false) => "No",
                                    };
                                    text(ui, (row.instance, 1), state, t.text_dim);
                                }
                                2 => {
                                    let name =
                                        format!("{} populated in {}", row.designator, v.name);
                                    operation = population_cell(
                                        ui,
                                        row,
                                        Some(v.id),
                                        cell.population,
                                        &name,
                                    )
                                    .or(operation.take());
                                }
                                _ if jumper(row) => {
                                    text(ui, (row.instance, 3), "Solder jumper", t.text_dim)
                                }
                                _ => {
                                    let default = match row.mpn.as_deref() {
                                        Some(mpn) => format!("Default · {mpn}"),
                                        None => "Default part".to_owned(),
                                    };
                                    let mut options: Vec<(Option<PartRef>, String)> =
                                        vec![(None, default)];
                                    if let Some(list) = substitutes.get(&row.instance) {
                                        options.extend(list.iter().map(|s| {
                                            (Some(s.part), format!("{} {}", s.manufacturer, s.mpn))
                                        }));
                                    }
                                    // A substitute that left the list (an unfit
                                    // one Issues reports) still shows by name.
                                    if let Some(part) = cell.part
                                        && !options.iter().any(|(o, _)| *o == Some(part))
                                    {
                                        let mpn = cell.mpn.clone().unwrap_or_default();
                                        options.push((Some(part), mpn));
                                    }
                                    let mut chosen = cell.part;
                                    let changed = ui
                                        .push_id(("variant-part", row.instance), |ui| {
                                            kit::dropdown(ui, "part", &mut chosen, &options)
                                        })
                                        .inner;
                                    if changed && chosen != cell.part {
                                        refusal::mark(ui.ctx(), Origin::Command);
                                        operation =
                                            Some(Operation::Variant(VariantCommand::SetPart {
                                                instance: row.instance,
                                                variant: v.id,
                                                part: chosen,
                                            }));
                                    }
                                }
                            }
                        },
                        None,
                    )
                }
            };
            card.parts_all |= shown.show_all;
        },
    );
    operation
}

/// Delete variant (the chosen one) and Done.
fn footer(ui: &mut Ui, card: &VariantsCard) -> (Option<Operation>, bool) {
    let mut operation = None;
    let mut done = false;
    kit::actions(ui, |ui| {
        done = kit::primary(ui, "Done", true).clicked();
        if let Some(variant) = card.selected
            && ui_kit::button(ui, "Delete variant", Button::Small)
                .on_hover_text("Delete this variant and what it changes")
                .clicked()
        {
            refusal::mark(ui.ctx(), Origin::Command);
            operation = Some(Operation::Variant(VariantCommand::Remove { variant }));
        }
    });
    (operation, done)
}

#[cfg(test)]
#[path = "variants_tests.rs"]
pub(crate) mod tests;

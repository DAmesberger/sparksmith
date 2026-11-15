//! The Component 3D tab's Body and Model sections: the package's envelope
//! (authored) and Change package… (the generator gallery in package scope, slice D1), and
//! the model card (the current source with its outline thumbnail, name,
//! revision, format and licence) with Choose model… (the 3D library sheet,
//! `crate::model_library`) and Import model file… (`model_import.rs`). A
//! model change is one `library.edit_package`; an authored envelope is a
//! working-set edit, sent with `library.edit`.
use super::PartEditor;
use crate::kit;
use egui::Ui;
use sparksmith_core::library::{PackageDefinition, PackageModel};
use sparksmith_core::units::Nm;
use sparksmith_core::{Engine, Operation};
use sparksmith_ui::ui_kit::{self, Button};
use std::ops::RangeInclusive;

const LENGTH: RangeInclusive<f64> = 0.01..=1000.0;

fn length(ui: &mut Ui, label: &str, value: &mut Nm) -> bool {
    super::footprint_props::nm_row(ui, label, value, LENGTH)
}

/// The Body section's one line on what the 3D model is, with its hover.
fn model_hint(model: &PackageModel) -> (&'static str, &'static str) {
    match model {
        PackageModel::Envelope { .. } => (
            "Approximate envelope only",
            "An approximate parametric envelope, not a verified supplier model",
        ),
        PackageModel::Library(_) => (
            "Library model; check it against the datasheet",
            "A model from the 3D library, placed by the offset below; its leads map by the lead map",
        ),
        PackageModel::PartFile(_) => (
            "Part file; check it against the datasheet",
            "A parametric SolidCraft part file built at this package's values; its marking surfaces are the file's marking planes",
        ),
    }
}

impl PartEditor {
    /// The Body section of the 3D tab.
    pub(super) fn body_inspector(&mut self, ui: &mut Ui, engine: &Engine) -> Option<Operation> {
        let source = self.model_source(ui, engine);
        let set = self.draft.as_mut()?;
        let package = &mut set.package;
        let mut change = false;
        kit::section(ui, "model-body", "Body", true, None, |ui| {
            // The envelope: a working-set edit.
            for (index, label) in ["Length", "Width", "Height"].into_iter().enumerate() {
                length(ui, label, &mut package.envelope_nm[index]);
            }
            kit::actions(ui, |ui| {
                change = ui_kit::button(ui, "Change package…", Button::Small)
                    .on_hover_text(
                        "Take the package, footprint and model from a generator, terminals matched by number",
                    )
                    .clicked();
            });
        });
        if change {
            self.open_template_picker();
        }
        source
    }

    /// The Model section: the model card and its actions. A package using
    /// a part file also shows the file and its parameters (`model_part.rs`).
    fn model_source(&mut self, ui: &mut Ui, engine: &Engine) -> Option<Operation> {
        let set = self.draft.as_ref()?;
        let draft = set.ids.package;
        let package = set.package.clone();
        let mut operation = None;
        let mut open_part = false;
        let mut choose = false;
        let mut import = false;
        let busy = self.model_import.busy();
        kit::section(ui, "model-source", "Model", true, None, |ui| {
            let card = ModelCard::of(engine, &package);
            model_card_rows(ui, &card);
            if let PackageModel::PartFile(part) = &package.model {
                operation = self.part_file_rows(ui, engine, draft, part);
            }
            let (shown, tip) = model_hint(&package.model);
            kit::hint(ui, shown, tip);
            kit::actions(ui, |ui| {
                let response = ui_kit::button(ui, "Import model file…", Button::Enabled(!busy));
                let response = if busy {
                    response.on_disabled_hover_text("An import is already running")
                } else {
                    response
                        .on_hover_text("Import a STEP, GLB or .solidcraft file into the 3D library")
                };
                import = response.clicked();
                choose = ui_kit::button(ui, "Choose model…", Button::Small)
                    .on_hover_text(
                        "Search the 3D library, ranked by how well each model fits this package",
                    )
                    .clicked();
            });
            kit::actions(ui, |ui| {
                open_part = ui_kit::button(ui, "Part file…", Button::Small)
                    .on_hover_text(
                        "Use a parametric SolidCraft part file: built-in, from this project or from disk",
                    )
                    .clicked();
                if matches!(package.model, PackageModel::PartFile(_))
                    && ui_kit::button(ui, "Use envelope box", Button::Small)
                        .on_hover_text(
                            "Stop using the part file: the package shows its envelope box",
                        )
                        .clicked()
                {
                    operation = Some(super::model_part::use_own_model(draft, &package));
                }
            });
        });
        if import {
            self.model_import.start(ui.ctx());
        }
        if choose {
            self.model_chooser.open_for(draft);
        }
        if open_part {
            self.open_part_chooser();
        }
        operation
    }
}

/// What the model card shows of the package's current model.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCard {
    pub name: String,
    /// "r2", or "draft" for a model not yet published.
    pub revision: Option<String>,
    pub detail: String,
    pub licence: Option<String>,
    pub leads: Option<String>,
    pub body_mm: [f64; 3],
    pub pins: Option<u32>,
}

impl ModelCard {
    pub fn of(engine: &Engine, package: &PackageDefinition) -> Self {
        let pins = u32::try_from(package.terminals.len())
            .ok()
            .filter(|n| *n > 0);
        match &package.model {
            PackageModel::Envelope { .. } => Self {
                name: "Envelope box".into(),
                revision: None,
                detail: "Approximate".into(),
                licence: None,
                leads: None,
                body_mm: package.envelope(),
                pins: None,
            },
            PackageModel::Library(model) => {
                let published = engine.project().library.model(model.model).is_some();
                match crate::model_library::model_definition(engine, model.model) {
                    Some((name, definition)) => {
                        let b = definition.bounds();
                        Self {
                            name: if name.trim().is_empty() {
                                definition.asset.file_name.clone()
                            } else {
                                name.to_owned()
                            },
                            revision: Some(if published {
                                model.model.revision.to_string()
                            } else {
                                "draft".into()
                            }),
                            detail: format!(
                                "{} · {}",
                                crate::model_library::format_label(definition.format),
                                crate::model_library::redistribution_label(
                                    definition.provenance.redistribution
                                )
                            ),
                            licence: definition.provenance.record.licence.clone(),
                            leads: Some(crate::model_library::lead_map_line(
                                &model.lead_map,
                                definition.leads.len(),
                                package.terminals.len(),
                            )),
                            body_mm: [b[1][0] - b[0][0], b[1][1] - b[0][1], b[1][2] - b[0][2]],
                            pins: definition.hints.pins,
                        }
                    }
                    None => Self {
                        name: "Missing library model".into(),
                        revision: Some(model.model.revision.to_string()),
                        detail: "This project holds no such model revision".into(),
                        licence: None,
                        leads: None,
                        body_mm: package.envelope(),
                        pins: None,
                    },
                }
            }
            PackageModel::PartFile(part) => {
                let published = engine.project().library.model(part.model).is_some();
                let [lo, hi] = part.derived.bounds();
                let name = crate::model_library::model_definition(engine, part.model).map_or_else(
                    || "Missing part file".to_owned(),
                    |(name, definition)| {
                        if name.trim().is_empty() {
                            definition.asset.file_name.clone()
                        } else {
                            name.to_owned()
                        }
                    },
                );
                Self {
                    name,
                    revision: Some(if published {
                        part.model.revision.to_string()
                    } else {
                        "draft".into()
                    }),
                    detail: format!("SolidCraft part file · {} value(s)", part.values.len()),
                    licence: crate::model_library::model_definition(engine, part.model)
                        .and_then(|(_, d)| d.provenance.record.licence.clone()),
                    leads: None,
                    body_mm: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
                    pins,
                }
            }
        }
    }
}

/// The model card: the outline thumbnail, then name, revision and the
/// source line, licence and lead map.
fn model_card_rows(ui: &mut Ui, card: &ModelCard) {
    let t = sparksmith_ui::theme::Tokens::get();
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), t.space_24 * 3.0),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, t.control_radius, t.dialog_footer);
    crate::model_library::thumbnail(
        ui.painter(),
        rect.shrink(t.space_8),
        card.body_mm,
        card.pins,
    );
    let name = match &card.revision {
        Some(r) => format!("{} · {r}", card.name),
        None => card.name.clone(),
    };
    kit::value_row(ui, "Model", &name);
    kit::value_row(ui, "Kind", &card.detail);
    if let Some(licence) = &card.licence {
        kit::value_row(ui, "Licence", licence);
    }
    if let Some(leads) = &card.leads {
        kit::value_row(ui, "Leads", leads);
    }
}

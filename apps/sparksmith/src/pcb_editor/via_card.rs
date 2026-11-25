//! The Via card of the selected via (net, position, drill, pad, span,
//! tenting and lock, with Delete via in the footer) and the Via tool card
//! (docs/sidebar-ia.md, "Properties card by subject" and "Tool card").
use super::placement_card::COORDINATE_LIMIT;
use super::{Field, PcbEditor, focus_within};
use crate::kit::{self, ToolCard};
use crate::refusal::{self, Origin};
use egui::Ui;
use sparksmith_core::model::ViaFill;
use sparksmith_core::rules::drills::{Verdict, through_fill};
use sparksmith_core::{Operation, Project, Via, ViaSpan, layout::LayoutCommand};
use sparksmith_ui::ui_kit::{self, Button};

/// Why a locked via's toggles and Delete are disabled.
const LOCKED: &str = "Unlock the via to change it";

impl PcbEditor {
    /// VIA: net (a link that highlights it), position, drill and pad,
    /// the layers it joins, tented and locked.
    pub(super) fn via_section(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        via: &Via,
    ) -> Option<Operation> {
        let mut action = None;
        let mut moved = None;
        let mut class_action = None;
        let mut open_classes = false;
        kit::section(ui, "pcb-via", "Via", true, None, |ui| {
            kit::row(ui, "Net", |ui| {
                if ui_kit::button(ui, project.net_name(via.net), Button::Small)
                    .on_hover_text("Highlight this net")
                    .clicked()
                {
                    self.net = Some(via.net);
                    self.highlighted = Some(via.net);
                }
            });
            let class = super::net_classes::class_row(ui, project, via.net, true);
            class_action = class.operation;
            open_classes = class.edit;
            let mm = |v: f64| format!("{} mm", kit::number_text((v * 1000.0).round() / 1000.0));
            // Typed, as a part's: sent when the field is left or Enter is
            // pressed; a refusal stands under it.
            if self.sent.get(&Field::ViaPosition) != Some(&self.via_position) {
                self.errors.0.remove(&Field::ViaPosition);
            }
            let position = ui
                .add_enabled_ui(!via.locked, |ui| {
                    kit::pair_row(
                        ui,
                        "Position",
                        &mut self.via_position,
                        "mm",
                        -COORDINATE_LIMIT..=COORDINATE_LIMIT,
                    );
                })
                .response
                .on_disabled_hover_text(LOCKED);
            if let Some(message) = self.errors.0.get(&Field::ViaPosition) {
                kit::field_error(ui, &position, message, true);
            }
            let editing = focus_within(ui, position.rect);
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) && editing {
                self.via_position = via.position;
            } else if !editing
                && !via.locked
                && self.via_position != via.position
                && self.sent.get(&Field::ViaPosition) != Some(&self.via_position)
            {
                self.sent.insert(Field::ViaPosition, self.via_position);
                moved = Some(LayoutCommand::MoveVia {
                    id: via.id,
                    position: self.via_position,
                });
            }
            kit::value_row(ui, "Drill", &mm(via.drill));
            kit::value_row(ui, "Pad", &mm(via.diameter));
            kit::value_row(ui, "Ring", &mm(via.annular_ring()));
            let span = match via.span {
                ViaSpan::Through => {
                    let layers = project.stackup.layers();
                    format!("Through, {} layers", layers.len())
                }
                ViaSpan::Between { from, to } => format!("{} to {}", from.name(), to.name()),
            };
            kit::value_row(ui, "Span", &span);
            let mut fill = via.fill;
            let filled =
                ui.add_enabled_ui(!via.locked, |ui| fill_row(ui, project, "Fill", &mut fill));
            crate::app::help_sheet::region(ui, filled.response.rect, "entry/layout.via_set", 3);
            filled.response.on_disabled_hover_text(LOCKED);
            if filled.inner && fill != via.fill {
                action = Some(LayoutCommand::SetVias {
                    ids: vec![via.id],
                    fill,
                });
            }
            let mut tented = via.tented;
            let tent =
                ui.add_enabled_ui(!via.locked, |ui| kit::check_row(ui, "Tented", &mut tented));
            crate::app::help_sheet::region(ui, tent.response.rect, "entry/layout.via_tent", 3);
            tent.response.on_disabled_hover_text(LOCKED);
            if tent.inner && tented != via.tented {
                action = Some(LayoutCommand::SetViaTented { id: via.id, tented });
            }
            let mut locked = via.locked;
            let lock = ui.scope(|ui| kit::check_row(ui, "Locked", &mut locked));
            crate::app::help_sheet::region(ui, lock.response.rect, "entry/layout.via_lock", 3);
            if lock.inner {
                action = Some(LayoutCommand::SetViaLock { id: via.id, locked });
            }
        });
        if open_classes {
            self.open_net_classes(sparksmith_core::layout::net_class(project, via.net));
        }
        if class_action.is_some() {
            return class_action;
        }
        if let Some(command) = moved {
            refusal::mark(ui.ctx(), Origin::Field(Field::ViaPosition.id()));
            return Some(Operation::Layout(command));
        }
        let command = action?;
        refusal::mark(ui.ctx(), Origin::Command);
        Some(Operation::Layout(command))
    }

    /// The Via tool card: the net and tenting of the vias it places.
    /// Tool settings are editor state, so it issues no operation.
    pub(super) fn via_tool_card(&mut self, ui: &mut Ui, project: &Project) -> Option<Operation> {
        let card = ToolCard {
            icon: "circle",
            name: "Via",
            key: "",
            hint: "Click a track end, pad or point to place a via; Esc ends",
            tip: "Places a through via (its net class's drill and pad, else the design \
                  rules') on the net of the pad or track under the pointer, else on the \
                  chosen net. It snaps to a track end within reach, else to the 0.5 mm grid. \
                  A via the clearance rules refuse is not added.",
            finish: None,
        };
        let t = sparksmith_ui::theme::Tokens::get();
        let margin = egui::Margin::symmetric(t.space_8 as i8, 0);
        let response = egui::Frame::NONE
            .inner_margin(margin)
            .show(ui, |ui| {
                kit::tool_card(ui, &card, |ui| {
                    let nets = super::net_options(project, "From the pad or track");
                    if kit::choice_row(ui, "Net", &mut self.net, &nets) {
                        self.route_highlight(self.net);
                    }
                    let mut tented = self.via_tented(project);
                    if kit::check_row(ui, "Tented", &mut tented) {
                        self.via_tented = Some(tented);
                    }
                    fill_row(ui, project, "Fill", &mut self.via_fill);
                })
            })
            .inner;
        if response.done {
            self.end_route();
            self.discard_staged();
        }
        None
    }
}

/// The fills a via may be given: those the fab profile makes or offers
/// without stating what it makes, and the via's own.
pub(super) fn fill_options(project: &Project, current: ViaFill) -> Vec<(ViaFill, String)> {
    let profile = project.rules.profile.as_ref();
    ViaFill::ALL
        .into_iter()
        .filter_map(|fill| {
            let verdict = profile.map(|p| through_fill(&p.drill_rules, fill));
            match verdict {
                Some(Verdict::NotMade(_)) if fill != current => None,
                Some(Verdict::NotMade(_)) => Some((fill, format!("{} (not made)", fill.label()))),
                Some(Verdict::NotStated(_)) => {
                    Some((fill, format!("{} (not stated)", fill.label())))
                }
                _ => Some((fill, fill.label().to_owned())),
            }
        })
        .collect()
}

/// What closes a via's hole: a choice of what the fab profile makes, else
/// the one value and why there is no other. Returns whether it changed.
pub(super) fn fill_row(ui: &mut Ui, project: &Project, label: &str, fill: &mut ViaFill) -> bool {
    let options = fill_options(project, *fill);
    if options.len() > 1 {
        return kit::choice_row(ui, label, fill, &options);
    }
    kit::value_row(ui, label, fill.label());
    if let Some(profile) = &project.rules.profile {
        kit::note(ui, &format!("{} makes open vias only", profile.name));
    }
    false
}

/// The via footer: Delete via, disabled while the via is locked.
pub(super) fn via_footer(ui: &mut Ui, via: &Via) -> Option<Operation> {
    let enabled = !via.locked;
    let clicked = ui_kit::button(ui, "Delete via", Button::Enabled(enabled))
        .on_disabled_hover_text(LOCKED)
        .clicked();
    (clicked && enabled).then_some(Operation::Layout(LayoutCommand::DeleteVia { id: via.id }))
}

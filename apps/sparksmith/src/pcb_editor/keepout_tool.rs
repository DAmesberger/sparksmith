//! Board keepouts in Layout: the Keepout tool draws the outline as the
//! Fill tool does (click corners, A for arcs, Enter or the first point
//! closes) and adds it as one `keepout.add`; a click on a keepout selects
//! it, and its card changes its name, layers and what it forbids
//! (`keepout.edit`) or deletes it (`keepout.delete`). Each is one undo step.
use super::{PcbEditor, Tool};
use crate::kit::{self, ToolCard};
use egui::Ui;
use sparksmith_core::CopperLayer;
use sparksmith_core::layout::fill::FillCommand;
use sparksmith_core::library::Outline;
use sparksmith_core::model::Keepout;
use sparksmith_core::model::fill::{KeepoutLayers, KeepoutRules, KeepoutSpec};
use sparksmith_core::{Id, Operation, Point, Project};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{self, Button};

/// The Keepout tool's settings: the layers and rules of the keepouts it
/// adds.
#[derive(Clone, Debug, Default)]
pub(super) struct KeepoutDraw {
    pub layers: KeepoutLayers,
    pub forbid: KeepoutRules,
}

/// The layers choice: every copper layer, or one.
fn layer_options(stack: &[CopperLayer]) -> Vec<(KeepoutLayers, String)> {
    std::iter::once((KeepoutLayers::All, "All copper layers".to_owned()))
        .chain(
            stack
                .iter()
                .map(|l| (KeepoutLayers::Only(vec![*l]), format!("{} only", l.name()))),
        )
        .collect()
}

/// The rows of what a keepout forbids; `true` when one changed.
fn rule_rows(ui: &mut Ui, forbid: &mut KeepoutRules) -> bool {
    let mut changed = false;
    for (label, value) in [
        ("No fills", &mut forbid.fills),
        ("No tracks", &mut forbid.tracks),
        ("No vias", &mut forbid.vias),
        ("No pads", &mut forbid.pads),
    ] {
        changed |= kit::check_row(ui, label, value);
    }
    changed
}

impl PcbEditor {
    /// Select board keepout `id`.
    pub fn select_keepout(&mut self, id: Id) {
        self.set_selection(None, None, None);
        self.keepout = Some(id);
    }

    /// The board keepout under `at`: its edge within `tolerance` when
    /// `edge`, else its inside. The last drawn first.
    pub(super) fn pick_keepout(
        &self,
        project: &Project,
        at: Point,
        tolerance: f64,
        edge: bool,
    ) -> Option<Id> {
        project
            .keepouts
            .iter()
            .rev()
            .find(|k| {
                let ring = super::fill_paint::outline_ring(&k.outline);
                if edge {
                    let next = ring.iter().skip(1).chain(ring.first());
                    ring.iter().zip(next).any(|(a, b)| {
                        sparksmith_core::geometry::segment_distance(at, *a, *b) <= tolerance
                    })
                } else {
                    sparksmith_core::geometry::polygon_contains(&ring, at)
                }
            })
            .map(|k| k.id)
    }

    /// The `keepout.add` of a closed outline with the tool's settings.
    pub(super) fn add_keepout(&self, outline: Outline) -> Operation {
        Operation::Fill(FillCommand::AddKeepout {
            keepout: Box::new(KeepoutSpec {
                name: String::new(),
                outline,
                layers: self.keepout_draw.layers.clone(),
                forbid: self.keepout_draw.forbid,
            }),
        })
    }

    /// What a closed outline of the drawing tool adds: a fill or a keepout.
    pub(super) fn add_outline(&mut self, outline: Outline) -> Operation {
        if self.tool == Tool::Keepout {
            self.add_keepout(outline)
        } else {
            self.add_fill(outline)
        }
    }

    /// The Keepout tool card: the layers and rules of the keepout it adds.
    /// Tool settings are editor state, so it issues no operation, except
    /// Finish (closing the outline).
    pub(super) fn keepout_tool_card(&mut self, ui: &mut Ui) -> Option<Operation> {
        let drawing = self.fill_draw.start.is_some();
        let card = ToolCard {
            icon: "suppress",
            name: "Keepout",
            key: "",
            hint: if self.fill_draw.arc_end.is_some() {
                "Click a point the arc passes through; Esc drops the outline"
            } else if drawing {
                "Click corners; click the first point or Enter closes; A arcs"
            } else {
                "Click the first corner of the keepout; A draws arcs"
            },
            tip: "Draws an area where copper may not go, on all copper layers or one. Each \
                  click adds a corner on the 0.5 mm grid; with arcs on (A), a click sets an \
                  arc's end and the next a point it passes through. Clicking the first point, \
                  Enter or Finish closes the outline and adds the keepout; Backspace takes \
                  back the last click and Esc drops the outline. The checks report copper \
                  inside it, and fills leave it out.",
            finish: Some("Finish"),
        };
        let t = Tokens::get();
        let margin = egui::Margin::symmetric(t.space_8 as i8, 0);
        let stack = self.stack.clone();
        let response = egui::Frame::NONE
            .inner_margin(margin)
            .show(ui, |ui| {
                kit::tool_card(ui, &card, |ui| {
                    let options = layer_options(&stack);
                    kit::choice_row(ui, "Layers", &mut self.keepout_draw.layers, &options);
                    rule_rows(ui, &mut self.keepout_draw.forbid);
                    let arcs = crate::keys::labelled(ui.ctx(), "Arcs", "fill.arc");
                    kit::check_row(ui, &arcs, &mut self.fill_draw.arc);
                })
            })
            .inner;
        if response.cancel {
            self.fill_draw.clear();
        }
        if response.done {
            self.end_route();
            self.discard_staged();
        }
        if response.finish {
            return self.fill_draw.close().map(|o| self.add_keepout(o));
        }
        None
    }

    /// KEEPOUT: its name, layers and what it forbids, each change one
    /// `keepout.edit`.
    pub(super) fn keepout_section(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        keepout: &Keepout,
    ) -> Option<Operation> {
        if (self.keepout_name.0, self.keepout_name.1.as_str())
            != (Some(keepout.id), keepout.name.as_str())
        {
            self.keepout_name = (Some(keepout.id), keepout.name.clone(), keepout.name.clone());
        }
        let mut next = keepout.clone();
        let mut changed = false;
        kit::section(ui, "pcb-keepout", "Keepout", true, None, |ui| {
            let name = kit::text_row(ui, "Name", &mut self.keepout_name.2);
            if name.lost_focus() && self.keepout_name.2.trim() != keepout.name {
                next.name = self.keepout_name.2.trim().to_owned();
                changed = true;
            }
            let options = layer_options(&project.stackup.layers());
            changed |= kit::choice_row(ui, "Layers", &mut next.layers, &options);
            changed |= rule_rows(ui, &mut next.forbid);
        });
        (changed && next != *keepout).then(|| {
            crate::refusal::mark(ui.ctx(), crate::refusal::Origin::Command);
            Operation::Fill(FillCommand::EditKeepout {
                keepout: Box::new(next),
            })
        })
    }

    /// The keepout card's footer: Delete.
    pub(super) fn keepout_footer(ui: &mut Ui, keepout: &Keepout) -> Option<Operation> {
        ui_kit::button(ui, "Delete", Button::Normal)
            .on_hover_text("Delete the keepout")
            .clicked()
            .then_some(Operation::Fill(FillCommand::DeleteKeepout {
                id: keepout.id,
            }))
    }
}

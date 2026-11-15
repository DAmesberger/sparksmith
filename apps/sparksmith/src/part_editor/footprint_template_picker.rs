//! The Footprint tab's From generator and its command dialog, docked in
//! the subject slot (docs/sidebar-ia.md, "Dialogs and reviews"):
//!
//! - **From generator** (the ribbon tool) opens the generator gallery
//!   (`generator_gallery`) in package scope: one
//!   `library.package_from_generator` for package, footprint and model.
//! - **Assign pads by number**: the changes `library.assign_terminals`
//!   would make, then one confirmed command.
//!
//! Each dialog stays open until the engine answers: a refusal comes back
//! to its refusal row (`refusal::Origin::Dialog`), a success closes it with
//! a toast that offers Undo. Findings are never suppressed while a dialog
//! is open; the Issues bar keeps counting.
use super::PartEditor;
use crate::kit::severity::Level;
use crate::kit::toast::{ToastAction, post};
use crate::kit::{self, DialogPanel, dialog::DialogAction, dialog::PrimaryKind};
use crate::refusal::{self, Origin};
use egui::Ui;
use sparksmith_core::library::footprint_edit::{self, AssignRule, TerminalChange};
use sparksmith_core::library::{FootprintDefinition, PackageDefinition, PadKind};
use sparksmith_core::{ComponentRef, DefinitionId, Engine, LibraryCommand, Operation};
/// The Assign dialog's panel id (also its refusal origin).
pub const ASSIGN_DIALOG: &str = "footprint-assign-dialog";
/// Frames a submitted dialog waits for the engine's answer before it is
/// usable again (the app may drop an operation while a file is busy).
const ANSWER_FRAMES: u64 = 3;
/// Why a primary waits for the working copy.
const WAIT_STORED: &str = "Wait until the last edit is stored";

/// The ribbon tool that opens the picker.
pub const TOOL: &str = "footprint-template";

/// The open Assign dialog and its submit.
#[derive(Clone, Debug, Default)]
pub struct TemplatePicker {
    /// The open Assign dialog: the terminal changes "Assign by number"
    /// would make, shown before they are confirmed.
    pub assign: Option<Vec<TerminalChange>>,
    /// The open dialog was drawn before (its body starts at the top once).
    shown: bool,
    /// A submitted operation waiting for the engine's answer.
    submitted: Option<Submitted>,
    /// The engine's refusal of the last submit, in shown wording.
    pub refusal: Option<String>,
}

/// A dialog operation sent to the engine: the footprint it was made
/// against and the frame it was sent in.
#[derive(Clone, Debug)]
struct Submitted {
    before: FootprintDefinition,
    frame: u64,
}

/// What became of a submitted dialog operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Nothing yet.
    Waiting,
    /// The footprint changed: the operation was applied.
    Applied,
    /// The engine refused it, in shown wording.
    Refused(String),
    /// No answer came (the operation was dropped); the dialog is usable
    /// again.
    Lost,
}

/// The answer to a dialog operation sent against `before` in `sent`: a
/// refusal handed back to `id` wins, then a changed footprint.
pub fn answer(
    ctx: &egui::Context,
    id: egui::Id,
    before: &FootprintDefinition,
    sent: u64,
    now: Option<&FootprintDefinition>,
) -> Answer {
    if let Some(why) = refusal::claim(ctx, Origin::Dialog(id)) {
        return Answer::Refused(why);
    }
    if now != Some(before) {
        return Answer::Applied;
    }
    if ctx.cumulative_frame_nr() > sent.saturating_add(ANSWER_FRAMES) {
        return Answer::Lost;
    }
    Answer::Waiting
}

impl TemplatePicker {
    pub fn close(&mut self) {
        self.reset();
    }

    /// Open the Assign dialog on `changes`.
    pub fn open_assign(&mut self, changes: Vec<TerminalChange>) {
        self.close();
        self.assign = Some(changes);
    }

    pub fn close_assign(&mut self) {
        self.assign = None;
        self.reset();
    }

    pub fn is_assigning(&self) -> bool {
        self.assign.is_some()
    }

    /// Whether an operation is waiting for the engine's answer.
    pub fn is_waiting(&self) -> bool {
        self.submitted.is_some()
    }

    fn reset(&mut self) {
        self.shown = false;
        self.submitted = None;
        self.refusal = None;
    }

    /// Record a submit against `before`, sent in frame `frame`.
    fn submit(&mut self, before: FootprintDefinition, frame: u64) {
        self.submitted = Some(Submitted { before, frame });
        self.refusal = None;
    }

    /// Settle a submitted operation; `Some(true)` once it was applied.
    fn settle(
        &mut self,
        ctx: &egui::Context,
        id: egui::Id,
        now: Option<&FootprintDefinition>,
    ) -> bool {
        let Some(sent) = &self.submitted else {
            return false;
        };
        match answer(ctx, id, &sent.before, sent.frame, now) {
            Answer::Waiting => false,
            Answer::Applied => true,
            Answer::Refused(why) => {
                self.submitted = None;
                self.refusal = Some(why);
                false
            }
            Answer::Lost => {
                self.submitted = None;
                false
            }
        }
    }
}

pub fn assign_request(draft: DefinitionId) -> Operation {
    Operation::Library(LibraryCommand::AssignTerminals {
        draft,
        rule: AssignRule::ByPackageDesignation,
        confirm: true,
    })
}

/// The changes Assign by number would make to `footprint`, previewed by
/// the core's pure rule (nothing is inferred without confirming them).
pub fn assign_preview(
    footprint: &FootprintDefinition,
    package: &PackageDefinition,
) -> Vec<TerminalChange> {
    footprint_edit::assign_terminals(footprint, package, &AssignRule::ByPackageDesignation)
        .map(|(_, changes)| changes)
        .unwrap_or_default()
}

/// Copper pads of a sketched footprint that land no package terminal yet.
pub fn unassigned(footprint: &FootprintDefinition) -> usize {
    if footprint.source.is_none() {
        return 0;
    }
    footprint
        .pads
        .iter()
        .filter(|p| p.terminal.is_none() && matches!(p.kind, PadKind::Smd | PadKind::PlatedThrough))
        .count()
}

impl PartEditor {
    /// The ribbon's "From generator" tool (and the 3D tab's Change
    /// package…): the generator gallery in package scope (slice D1), on the
    /// working set's package draft. One `library.package_from_generator`
    /// gives the package, footprint and model the generator's; a footprint
    /// with pads is replaced in the same atomic step.
    pub(super) fn open_template_picker(&mut self) {
        if let Some(set) = &self.draft {
            self.template_picker.close_assign();
            self.gallery
                .open_package(set.ids.package, set.footprint.pads.len(), String::new());
        }
    }

    /// The gallery in new-component scope: the ribbon's New and the start
    /// page's New from generator… open it.
    pub(super) fn open_template_gallery(&mut self) {
        self.gallery.open_new();
    }

    /// The gallery on `request` for the Schematic's Place palette: its
    /// primary continues to placement ([`Self::take_template_to_place`]).
    pub fn open_template_to_place(
        &mut self,
        request: &crate::generators::MakeRequest,
        query: String,
    ) {
        self.gallery.open_on(request, query, true);
    }

    /// The gallery sheet where the Component workspace is not drawing it.
    pub fn template_sheet(&mut self, ctx: &egui::Context, engine: &Engine) -> Option<Operation> {
        self.gallery.show(ctx, engine)
    }

    /// The component the gallery created for placement, once.
    pub fn take_template_to_place(&mut self) -> Option<ComponentRef> {
        self.gallery.take_to_place()
    }

    /// Open Assign pads by number with its preview (the Terminals list's
    /// action, and the Issues Fix for pads without a package terminal).
    pub(super) fn open_assign_dialog(&mut self) {
        if let Some(set) = &self.draft {
            let changes = assign_preview(&set.footprint, &set.package);
            self.template_picker.open_assign(changes);
        }
    }

    /// The docked "Assign pads by number" dialog: each pad and the package
    /// terminal it takes, then one confirmed `library.assign_terminals`.
    pub(super) fn assign_dialog(&mut self, ui: &mut Ui) -> Option<Operation> {
        let id = egui::Id::unique(ASSIGN_DIALOG);
        let ctx = ui.ctx().clone();
        let now = self.draft.as_ref().map(|s| &s.footprint);
        if self.template_picker.settle(&ctx, id, now) {
            let count = self.template_picker.assign.as_ref().map_or(0, Vec::len);
            let noun = if count == 1 { "pad" } else { "pads" };
            post(
                &ctx,
                Level::Ok,
                format!("Assigned {count} {noun}"),
                vec![ToastAction::Undo],
            );
            self.template_picker.close_assign();
            return None;
        }
        let set = self.draft.as_ref()?;
        let draft = set.ids.footprint;
        let synced = self.baseline.as_ref() == Some(set);
        let picker = &mut self.template_picker;
        let changes = picker.assign.clone()?;
        let count = changes.len();
        let reset_scroll = !std::mem::replace(&mut picker.shown, true);
        let primary = match count {
            1 => "Assign 1 pad".to_owned(),
            n => format!("Assign {n} pads"),
        };
        let disabled = if picker.is_waiting() {
            Some("Assigning…")
        } else if !synced {
            Some(WAIT_STORED)
        } else if count == 0 {
            Some("No pad number matches exactly one free package terminal")
        } else {
            None
        };
        let refusal = picker.refusal.clone();
        let rows: Vec<(String, String)> = changes
            .iter()
            .map(|change| {
                let to = change
                    .to
                    .and_then(|t| set.package.terminal(t))
                    .map_or_else(|| "none".to_owned(), |t| t.designation.clone());
                (format!("Pad {}", change.number), format!("→ terminal {to}"))
            })
            .collect();
        let show_all_id = id.with("show-all");
        let show_all = ctx
            .data(|d| d.get_temp::<bool>(show_all_id))
            .unwrap_or(false);
        let dialog = DialogPanel {
            id,
            icon: "point",
            title: "Assign pads by number",
            kind: "",
            body_rows: 12.0,
            primary: &primary,
            primary_kind: PrimaryKind::Normal,
            primary_disabled: disabled,
            refusal: refusal.as_deref(),
            reset_scroll,
            status: None,
        };
        if reset_scroll {
            // A new dialog starts with its header in view.
            ui.scroll_to_cursor(Some(egui::Align::TOP));
        }
        let action = dialog.show(ui, |ui| {
            kit::hint(
                ui,
                "Pads take the matching terminal",
                "A pad takes the package terminal whose designation equals its number, when exactly one free terminal matches. Explicit terminals are kept.",
            );
            if rows.is_empty() {
                kit::hint(ui, "No pad number matches a free package terminal", "");
            }
            let max = if show_all { rows.len() } else { kit::LIST_MAX_ROWS };
            let list = kit::list_frame(ui, rows.len(), max, |ui, index| {
                if let Some((pad, to)) = rows.get(index) {
                    kit::value_row(ui, pad, to);
                }
            });
            if list.show_all {
                ctx.data_mut(|d| d.insert_temp(show_all_id, true));
            }
        });
        match action {
            Some(DialogAction::Primary) if disabled.is_none() => {
                refusal::mark(&ctx, Origin::Dialog(id));
                picker.submit(set.footprint.clone(), ctx.cumulative_frame_nr());
                self.awaiting = true;
                Some(assign_request(draft))
            }
            Some(DialogAction::Cancel) => {
                picker.close_assign();
                ctx.data_mut(|d| d.remove::<bool>(show_all_id));
                None
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator_gallery::Scope;
    use sparksmith_core::Engine;

    /// The Footprint tab's From generator opens the gallery in package
    /// scope on the working set's package draft.
    #[test]
    fn from_generator_opens_the_gallery_on_the_package() -> Result<(), Box<dyn std::error::Error>> {
        let ctx = egui::Context::default();
        sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
        let engine = Engine::demo()?;
        let mut editor = PartEditor::default();
        editor.open_first(&engine);
        editor.commit(&ctx, &engine);
        editor.on_tab("footprint");
        let set = editor.draft.as_ref().ok_or("a working set")?;
        let (package, pads) = (set.ids.package, set.footprint.pads.len());
        editor.on_tool(TOOL, &engine);
        assert_eq!(
            editor.gallery.scope(),
            Some(&Scope::Package {
                draft: package,
                pads
            })
        );
        Ok(())
    }
}

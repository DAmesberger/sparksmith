//! Where the window offers workflows without a model (assistant slice C2,
//! `docs/assistant-panel.md` §3.2, §3.3): a palette entry per workflow,
//! generated from its data, a dropped file whose type a workflow's
//! `offer_when` names, and the Component start page's Create from
//! datasheet…, which offers the workflows that take a datasheet. Each puts
//! the workflow card in the chat ([`crate::workflow_cards`]); nothing starts
//! until the person chooses Accept and continue there (`app/assistant_run.rs`).
use super::App;
use super::command_palette::PaletteRow;
use crate::kit::severity::Level;
use crate::workflow_cards::{self, CardAction, PALETTE_PREFIX};
use sparksmith_assist::workflow::{Listed, Signal, offer};
use sparksmith_automation::WindowHost as _;
use sparksmith_automation::view::OpenParams;

/// The palette's workflow rows for `query`.
pub(super) fn workflow_rows(listed: &[Listed], query: &str) -> Vec<PaletteRow> {
    workflow_cards::palette_rows(listed, query)
        .into_iter()
        .map(|row| PaletteRow {
            id: row.id,
            title: row.title,
            icon: "assistant",
            detail: row.detail,
            shortcut: None,
            enabled: row.enabled,
            recent: false,
        })
        .collect()
}

impl App {
    /// A palette row chosen: when it is a workflow's, its card is shown
    /// and the answer is true.
    pub(super) fn run_workflow_row(&mut self, id: &str) -> bool {
        let Some(workflow) = id.strip_prefix(PALETTE_PREFIX) else {
            return false;
        };
        if let Err(why) = self
            .workflow_cards
            .offer(workflow, serde_json::Map::new(), "")
        {
            self.messages.push(Level::Error, &why, self.now);
            self.toast(
                Level::Error,
                crate::refusal::clip(&why, crate::refusal::MAX_SHOWN),
                Vec::new(),
            );
        }
        true
    }

    /// Files dropped on the window this frame: a waiting datasheet field
    /// takes them, else a workflow that names their type is offered. A file
    /// nothing takes is said in a toast.
    /// Once a frame, before the chat draws: the cards added since the last
    /// frame join its transcript, and the chat opens for a new one.
    pub(super) fn workflow_frame(&mut self, ctx: &egui::Context) {
        let folder = self.project_folder();
        self.workflow_cards.set_project(folder);
        self.dropped_files(ctx);
        if std::mem::take(&mut self.part.datasheet_requested) {
            self.offer_datasheet_workflows();
        }
        self.workflow_cards.frame();
        if let Some((card, receiver)) = &self.workflow_pick
            && let Ok(paths) = receiver.try_recv()
        {
            let card = card.clone();
            self.workflow_pick = None;
            self.workflow_cards.picked(&card, paths);
        }
        for key in self.workflow_cards.take_new() {
            self.assistant
                .conversation
                .push(crate::assistant::Item::Workflow(key));
        }
        if std::mem::take(&mut self.workflow_cards.open_requested) {
            self.assistant.open = true;
            self.assistant.folded = false;
        }
    }

    /// Create from datasheet…: the chat opens with the card of each
    /// workflow a datasheet (a PDF) offers; one already waiting stays.
    fn offer_datasheet_workflows(&mut self) {
        self.workflow_cards
            .signal(&Signal::Attachment(offer::PDF.to_owned()), "");
        self.assistant.open = true;
        self.assistant.folded = false;
    }

    /// What the person chose on a workflow card: Open in editor runs in the
    /// window, Choose… opens the picker; the rest goes to the run host
    /// (`app/assistant_run.rs`) through the cards' outbox.
    pub(super) fn workflow_card_action(&mut self, action: CardAction) {
        match action {
            CardAction::Show { call, params } if call == "view.open" => {
                let opened = serde_json::from_value::<OpenParams>(params)
                    .map_err(|e| e.to_string())
                    .and_then(|params| {
                        let (mut window, engine) = self.window_and_engine();
                        window.open(engine, &params).map_err(|e| e.message)
                    });
                if let Err(why) = opened {
                    self.messages.push(Level::Error, &why, self.now);
                    self.toast(
                        Level::Error,
                        crate::refusal::clip(&why, crate::refusal::MAX_SHOWN),
                        Vec::new(),
                    );
                }
            }
            CardAction::PickFiles { card } => {
                self.workflow_pick = Some((card, self.files.pick_pdfs()));
            }
            other => self.workflow_cards.outbox.push(other),
        }
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        // A workflow may name the workspace shown (`offer_when`).
        let workspace = match self.workspace {
            crate::shell::Workspace::Part => "component",
            crate::shell::Workspace::Schematic => "schematic",
            crate::shell::Workspace::Pcb => "pcb",
        };
        self.workflow_cards.workspace(workspace);
        let paths: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if paths.is_empty() {
            return;
        }
        for line in self.workflow_cards.drop_files(paths) {
            self.messages.push(Level::Info, &line, self.now);
            self.toast(
                Level::Info,
                crate::refusal::clip(&line, crate::refusal::MAX_SHOWN),
                Vec::new(),
            );
        }
    }
}

#[cfg(test)]
#[path = "workflow_host_tests.rs"]
mod tests;

//! Requests waiting for a generator or the online library. Neither is
//! waited for on the window's thread ([`Deferred`], [`OnlineCalls`]): a
//! request that met one still running changed nothing, is kept here and
//! runs again when its answer is there. The application bar shows what the
//! window waits for, with Stop.
use super::App;
use crate::assistant::worker::ToolRequest;
use crate::document_version::DocumentVersion;
use crate::kit::severity::Level;
use crate::online_window::OnlineCalls;
use crate::refusal::Origin;
use serde_json::{Value, json};
use sparksmith_assist::desk::Called;
use sparksmith_automation::live::{Admitted, ControlRequest};
use sparksmith_automation::session::generators::Deferred;
use sparksmith_core::{Error, Operation};
use std::sync::Arc;

pub(super) enum Waiting {
    /// The person's own command.
    Command {
        origin: Origin,
        operation: Box<Operation>,
        version: DocumentVersion,
    },
    /// A live MCP request; `asked`: the person allowed it.
    Live {
        request: ControlRequest,
        admitted: Box<Admitted>,
        asked: bool,
    },
    /// A call of the chat or of a workflow run; `allowed`: the person
    /// allowed it on its card.
    Chat { request: ToolRequest, allowed: bool },
}

impl Waiting {
    /// Whoever sent it gave it up.
    fn gone(&self) -> bool {
        matches!(self, Self::Live { request, .. } if request.ticket.is_cancelled())
    }
}

/// The window's generator runtime, its online requests and what waits for
/// them.
pub(super) struct GeneratorWait {
    pub runtime: Arc<Deferred>,
    pub online: OnlineCalls,
    waiting: Vec<Waiting>,
}

impl GeneratorWait {
    pub fn new(ctx: &egui::Context) -> Self {
        let (wake, answered) = (ctx.clone(), ctx.clone());
        Self {
            runtime: sparksmith_automation::session::generators::window_runtime(move || {
                wake.request_repaint();
            }),
            online: OnlineCalls::new(move || answered.request_repaint()),
            waiting: Vec::new(),
        }
    }

    /// Whether a call met something still running since this was last
    /// asked.
    pub fn take_met(&self) -> bool {
        let generator = self.runtime.take_met();
        self.online.take_met() || generator
    }

    /// End every generator call and online request of the window.
    fn end(&self) {
        self.runtime.stop();
        self.online.stop();
    }

    pub fn hold(&mut self, waiting: Waiting) {
        self.waiting.push(waiting);
    }

    pub fn is_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// Stop waiting: every request is refused, nothing is changed. Where
    /// the person's own commands came from comes back, to be told.
    pub fn stop(&mut self) -> Vec<Origin> {
        let mut origins = Vec::new();
        for waiting in std::mem::take(&mut self.waiting) {
            match waiting {
                Waiting::Command { origin, .. } => origins.push(origin),
                Waiting::Live { request, .. } => {
                    let _ = request.reply.send(stopped());
                }
                Waiting::Chat { request, .. } => {
                    let _ = request.reply.send(Called::refused(stopped()));
                }
            }
        }
        self.end();
        origins
    }

    /// Stop what the chat and its workflow run wait for.
    pub fn stop_chat(&mut self) {
        self.waiting.retain_mut(|waiting| {
            let Waiting::Chat { request, .. } = waiting else {
                return true;
            };
            let _ = request.reply.send(Called::refused(stopped()));
            false
        });
        self.end_unneeded();
    }

    /// What nothing waits for any more is stopped.
    fn end_unneeded(&mut self) {
        if self.waiting.is_empty() {
            self.end();
        }
    }
}

fn stopped() -> Value {
    json!({"ok": false, "error": {"code": "generator.cancelled",
        "message": "Not run: it was stopped while it waited. Nothing was changed."}})
}

impl App {
    /// Run what waited once nothing is running.
    pub(super) fn serve_generator_wait(&mut self, ctx: &egui::Context) {
        let held = !self.generators.waiting.is_empty();
        self.generators.waiting.retain(|w| !w.gone());
        if held {
            self.generators.end_unneeded();
        }
        if self.generators.runtime.waiting() + self.generators.online.waiting() > 0 {
            return;
        }
        for waiting in std::mem::take(&mut self.generators.waiting) {
            match waiting {
                Waiting::Command {
                    origin,
                    operation,
                    version,
                } => {
                    if version.is_current(&self.engine) {
                        self.apply(ctx, origin, Some(*operation));
                    } else {
                        let changed = "generator.cancelled: the project changed while a generator ran; nothing was done. Try again.";
                        self.refuse(ctx, origin, Error::Invalid(changed.into()), None);
                    }
                }
                Waiting::Live {
                    request,
                    admitted,
                    asked,
                } => {
                    let busy = if asked { None } else { self.live_busy(ctx) };
                    self.run_live(request, *admitted, busy, asked);
                }
                Waiting::Chat { request, allowed } => self.run_chat_call(ctx, request, allowed),
            }
        }
        if self.generators.waiting.is_empty() {
            self.generators.runtime.trim();
            self.generators.online.forget();
        }
    }

    /// Work the application bar offers Stop for, in words.
    pub(super) fn stoppable(&self) -> Option<&'static str> {
        if self.generators.is_waiting() && self.generators.online.waiting() > 0 {
            Some("Waiting for the online library\u{2026}")
        } else if self.generators.is_waiting() {
            Some("Waiting for a generator\u{2026}")
        } else if self.schematic.making().is_some() || self.part.making().is_some() {
            Some("Making a part\u{2026}")
        } else if self.sources.online().is_some_and(|o| o.downloading()) {
            Some("Downloading a part\u{2026}")
        } else {
            None
        }
    }

    /// Stop on it.
    pub(super) fn stop_work(&mut self, ctx: &egui::Context) {
        for origin in self.generators.stop() {
            let stopped = "generator.cancelled: stopped; nothing was changed";
            self.refuse(ctx, origin, Error::Invalid(stopped.into()), None);
        }
        for making in [self.schematic.making(), self.part.making()]
            .into_iter()
            .flatten()
        {
            making.stop();
        }
        if let Some(online) = self.sources.online() {
            online.stop_downloads();
        }
        self.say(Level::Info, "Stopped; nothing was changed");
    }
}

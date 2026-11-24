//! A workflow run in the chat (`docs/assistant-panel.md` §3–§5, slice W2).
//!
//! Accept and continue on a workflow card starts the run off the UI thread
//! ([`crate::assistant::worker::start_run`]) on the one runner
//! (`sparksmith_assist::workflow::runner`), on the model Settings › AI sets
//! up, under the chat's grant held to the workflow's reach. The run's calls
//! and its work on the chat's session come back here and run between frames
//! on the window's engine, through the chat's gate and Ask cards
//! ([`App::run_chat_call`]), in the one turn the run opens, so the whole run
//! undoes as one step. Its events become the cards in the transcript
//! ([`crate::workflow_cards`]); the person's answers, Stop and Undo run come
//! back from the cards' outbox. When the run ends, the model is told how it
//! ended and its result with the person's next message; Just chat tells the
//! model the card was declined.
use super::App;
use crate::assistant::worker::{self, FromRun, RunStart};
use crate::assistant::{RunDone, State};
use crate::kit::severity::Level;
use crate::workflow_cards::CardAction;
use sparksmith_assist::model::{Budget, ModelProfile};
use sparksmith_assist::workflow::control::{ControlAnswer, ControlCard};
use sparksmith_assist::workflow::event::{OfferedBy, RunEnd, WorkflowEvent};
use sparksmith_assist::workflow::runner::WorkflowOutcome;
use sparksmith_assist::{AssistLibrary, RunConfig};
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;

/// Messages from a run handled per frame, so a flood cannot starve drawing.
const PER_FRAME: usize = 64;
/// Characters of a run's result the model is told; it reads more with the
/// verbs.
const RESULT_CHARS: usize = 4_000;

/// The run's model configuration: the run profile Settings › AI's model
/// selects (shipped or the person's own), the person's limits as `budget`
/// (the runner holds a budget the workflow declares within them).
fn run_config(
    profile: ModelProfile,
    budget: Budget,
    config_dir: Option<&std::path::Path>,
) -> Result<RunConfig, String> {
    let shipped = AssistLibrary::locate().map_err(|e| e.to_string())?;
    let folder = config_dir.map(sparksmith_assist::profile::user_folder);
    let mut config =
        RunConfig::new(&shipped, profile, None, folder.as_deref()).map_err(|e| e.to_string())?;
    config.budget = budget;
    Ok(config)
}

/// How a run ended, for the model.
fn end_words(end: &RunEnd) -> String {
    match end {
        RunEnd::Done => "done".into(),
        RunEnd::Stopped => "stopped by the person; what completed stays".into(),
        RunEnd::Declined { step } => format!("declined by the person at {step}"),
        RunEnd::Limit { why } => format!("its limit was reached ({why})"),
        RunEnd::Failed { step, why } => format!("{step} failed: {why}"),
    }
}

/// `text` cut to [`RESULT_CHARS`], saying so.
fn cut(mut text: String) -> String {
    if text.len() > RESULT_CHARS {
        let mut end = RESULT_CHARS;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("… [cut: read the drafts with the tools]");
    }
    text
}

impl App {
    /// What the person chose on workflow cards: Accept and continue starts a run once
    /// the chat is free (until then it waits in the outbox), Just chat is
    /// told to the model, a run's answers and Stop reach the run, and Undo
    /// run takes the run back.
    pub(super) fn serve_cards(&mut self, ctx: &egui::Context) {
        let mut waiting = Vec::new();
        for action in std::mem::take(&mut self.workflow_cards.outbox) {
            match action {
                CardAction::Start {
                    card,
                    workflow,
                    inputs,
                } => {
                    if self.assistant.job.is_some() || self.assistant.run.is_some() {
                        waiting.push(CardAction::Start {
                            card,
                            workflow,
                            inputs,
                        });
                    } else {
                        self.start_run(ctx, &workflow, inputs);
                    }
                }
                CardAction::JustChat { card } => {
                    if let Some(WorkflowEvent::Offered { title, .. }) =
                        self.workflow_cards.offered_event(&card, OfferedBy::Person)
                    {
                        self.assistant.notes.push(format!(
                            "The person chose Just chat on the workflow card \"{title}\": go on without the workflow."
                        ));
                    }
                }
                CardAction::Answer {
                    run,
                    answer: ControlAnswer::UndoRun,
                    ..
                } => self.undo_run(&run),
                CardAction::Answer { run, step, answer } => {
                    if let Some(job) = &self.assistant.run
                        && job.run.as_deref() == Some(run.as_str())
                    {
                        let _ = job.answers.send((step, answer));
                        self.assistant.conversation.state = State::Running;
                    }
                }
                CardAction::Stop { run } => {
                    if self
                        .assistant
                        .run
                        .as_ref()
                        .is_some_and(|job| job.run.as_deref() == Some(run.as_str()))
                    {
                        self.generators.stop_chat();
                        self.assistant.stop();
                    }
                }
                // The window carries these out when they are chosen.
                CardAction::Show { .. } | CardAction::PickFiles { .. } => {}
            }
        }
        self.workflow_cards.outbox.extend(waiting);
    }

    /// Start workflow `workflow` with the inputs its card gave.
    fn start_run(
        &mut self,
        ctx: &egui::Context,
        workflow: &str,
        inputs: serde_json::Map<String, serde_json::Value>,
    ) {
        let loaded = self
            .workflow_cards
            .listed()
            .iter()
            .find(|l| l.id == workflow && l.available())
            .and_then(|l| l.loaded.clone());
        let Some(loaded) = loaded else {
            self.assistant.conversation.state =
                State::Error(format!("The workflow {workflow} cannot run here"));
            return;
        };
        let (model, budget, permissions) = match self.chat_model() {
            Ok(chosen) => (
                chosen.model,
                // The runner holds a budget the workflow declares within the
                // person's limits, when they set any.
                sparksmith_assist::settings::run_budget(chosen.limits.as_ref()),
                chosen.permissions,
            ),
            Err(why) => {
                self.assistant.conversation.state = State::NotSetUp(why);
                return;
            }
        };
        self.assistant.permissions = Some(permissions);
        let started = self
            .chat_grant()
            .and_then(|(grant, _)| {
                Ok((
                    grant,
                    run_config(
                        model.profile.clone(),
                        budget,
                        self.settings_window.ai.config_dir().as_deref(),
                    )?,
                ))
            })
            .and_then(|(grant, config)| {
                let wake = ctx.clone();
                worker::start_run(
                    RunStart {
                        loaded,
                        inputs,
                        grant,
                        config,
                    },
                    model.connect,
                    Arc::new(move || wake.request_repaint()),
                )
            });
        match started {
            Ok(job) => {
                self.assistant.run = Some(job);
                self.assistant.conversation.state = State::Running;
            }
            Err(why) => self.assistant.conversation.state = State::Error(why),
        }
    }

    /// What the run sent since the last frame: its events, its calls, its
    /// work on the chat's session and its end.
    pub(super) fn serve_run(&mut self, ctx: &egui::Context) {
        for _ in 0..PER_FRAME {
            if self.assistant.held.is_some() {
                return;
            }
            let Some(job) = &self.assistant.run else {
                return;
            };
            let message = match job.rx.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    FromRun::Done(Box::new(Err("the run ended without an answer".into())))
                }
            };
            match message {
                FromRun::Workflow(event) => self.run_event(&event),
                FromRun::Tool(request) => self.run_chat_call(ctx, request, false),
                FromRun::Work(work, done) => {
                    let host = self.files.host_file();
                    let (_, engine, assistant) = self.window_for_assistant(None);
                    let ((), changes) = assistant.session.lend_file(engine, &host, work);
                    self.file_changes(changes, true);
                    let _ = done.send(());
                }
                FromRun::Done(result) => self.finish_run(*result),
            }
        }
        // More are waiting: take them next frame.
        ctx.request_repaint();
    }

    fn run_event(&mut self, event: &WorkflowEvent) {
        match event {
            WorkflowEvent::Started { run, .. } => {
                if let Some(job) = &mut self.assistant.run {
                    job.run = Some(run.clone());
                }
            }
            WorkflowEvent::Step { .. } => self.assistant.conversation.state = State::Running,
            WorkflowEvent::Card { card, .. }
                if !matches!(
                    card,
                    ControlCard::Progress { .. }
                        | ControlCard::Meter { .. }
                        | ControlCard::Result { .. }
                ) =>
            {
                self.assistant.conversation.state = State::Waiting;
            }
            _ => {}
        }
        self.workflow_cards.event(event);
    }

    /// The run ended: its result card's Undo run is kept, and the model is
    /// told how it ended, with its result.
    fn finish_run(&mut self, result: Result<WorkflowOutcome, String>) {
        let Some(job) = self.assistant.run.take() else {
            return;
        };
        self.assistant.held = None;
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(why) => {
                self.assistant
                    .notes
                    .push(format!("The workflow you offered could not start: {why}"));
                self.assistant.conversation.state = State::Error(why);
                return;
            }
        };
        if let Some(run) = job.run {
            self.assistant.runs.push(RunDone {
                run,
                label: outcome.label.clone(),
                after: self.engine.revision(),
            });
        }
        self.assistant.conversation.state = match outcome.end {
            RunEnd::Stopped => State::Stopped,
            _ => State::Idle,
        };
        let mut note = format!(
            "The workflow run \"{}\" ended: {}.",
            outcome.label,
            end_words(&outcome.end)
        );
        if let Some(result) = outcome.results.get(&job.result_step) {
            note.push_str(&format!(" Its result: {}", cut(result.to_string())));
        }
        self.assistant.notes.push(note);
    }

    /// Undo run on a result card: the run's one undo step, while the
    /// document still stands where the run left it.
    fn undo_run(&mut self, run: &str) {
        let undoable = self.assistant.runs.iter().any(|done| {
            done.run == run
                && done.after == self.engine.revision()
                && self.engine.undo_step().map(|s| s.line()).as_deref() == Some(done.label.as_str())
        });
        if undoable {
            self.history(false);
        } else {
            self.say(
                Level::Warning,
                "The run can't be undone from its card: the design changed since it ran",
            );
        }
    }
}

#[cfg(test)]
#[path = "assistant_run_tests.rs"]
mod tests;

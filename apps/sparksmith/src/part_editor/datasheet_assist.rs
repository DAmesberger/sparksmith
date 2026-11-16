//! "Create from datasheet…" on the Component workspace start page: the
//! datasheet assistant (`sparksmith-assist`) on a background thread, with
//! progress, tokens and Cancel. The run works in its own headless session
//! through the Sparksmith tools and saves a new project of drafts with its
//! review report beside it; nothing is published, and the open project is
//! not touched.
//!
//! The model is the endpoint set up in Settings › AI (the assistant's,
//! else the first), with what it can do declared there; nothing is
//! detected. Its key is the one Settings › AI keeps in the system keychain
//! for that endpoint; without one, an Anthropic run falls back to
//! `ANTHROPIC_API_KEY`. The run's budget is the datasheet workflow's own,
//! within Settings › AI's limits when the person set any. This form goes
//! when the chat runs the datasheet workflow (slice W2).
use crate::kit;
use egui::Ui;
use sparksmith_assist::model::{self, Connection, Endpoint, KeySource, Provider, keys};
use sparksmith_assist::profile as run_profile;
use sparksmith_assist::settings::{self as ai_settings, EndpointSetting, Limits};
use sparksmith_assist::{AssistLibrary, CallState, Datasheet, Event, Goal, RunConfig};
use sparksmith_automation::Session;
use sparksmith_ui::ui_kit::{self, Button};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// Lines of progress kept.
const MAX_LOG: usize = 200;

/// The shipped workflow this form runs the process of; its budget is the
/// run's.
const WORKFLOW: &str = "datasheet-to-component";

/// The endpoint and the person's limits, if any, from Settings › AI.
#[derive(Clone, Debug, PartialEq)]
pub struct Chosen {
    pub endpoint: EndpointSetting,
    pub limits: Option<Limits>,
}

/// Settings › AI's endpoint for a model run and its limits, or why there
/// is none.
fn chosen_from_settings() -> Result<Chosen, String> {
    let dirs = sparksmith_io::settings::UserDirs::from_env().map_err(|e| e.to_string())?;
    let settings =
        ai_settings::read(&ai_settings::user_file(&dirs.config)).map_err(|e| e.to_string())?;
    let endpoint = settings
        .run_endpoint()
        .cloned()
        .ok_or_else(|| "No model endpoint: add one in Settings › AI".to_owned())?;
    Ok(Chosen {
        endpoint,
        limits: settings.limits,
    })
}

/// A finished run.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub stop: String,
    pub output_tokens: u64,
    pub project: PathBuf,
    pub review: PathBuf,
}

enum Message {
    Line(String),
    Done(Result<Finished, String>),
}

struct Job {
    rx: Receiver<Message>,
    cancel: Arc<AtomicBool>,
}

/// The form, the running job and its log.
#[derive(Default)]
pub struct DatasheetAssist {
    pub open: bool,
    /// Scroll the form into view on its next draw (it was just opened).
    reveal: bool,
    datasheets: String,
    name: String,
    out: String,
    /// Settings › AI's endpoint and limits, read when the form opens.
    chosen: Option<Result<Chosen, String>>,
    job: Option<Job>,
    log: Vec<String>,
    result: Option<Result<Finished, String>>,
}

/// What a run needs, taken from the form.
struct Plan {
    goal: Goal,
    config: RunConfig,
    endpoint: Endpoint,
}

impl DatasheetAssist {
    /// Open the form, scrolled into view on its next draw, with the
    /// endpoint Settings › AI holds now.
    pub fn show(&mut self) {
        self.open = true;
        self.reveal = true;
        self.chosen = None;
    }

    /// Take the background thread's messages.
    fn poll(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let mut done = None;
        while let Ok(message) = job.rx.try_recv() {
            match message {
                Message::Line(line) => {
                    self.log.push(line);
                    if self.log.len() > MAX_LOG {
                        self.log.remove(0);
                    }
                }
                Message::Done(result) => done = Some(result),
            }
        }
        if let Some(result) = done {
            self.result = Some(result);
            self.job = None;
        }
    }

    /// The form and progress; on the start page under New.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.poll();
        if !self.open {
            return;
        }
        let heading = kit::form_heading(ui, "Create from datasheet");
        self.form(ui);
        if std::mem::take(&mut self.reveal) {
            // Just opened: the whole form in view, from its heading.
            let form = heading.union(egui::Rect::from_min_max(
                heading.min,
                egui::pos2(heading.right(), ui.min_rect().bottom()),
            ));
            ui.scroll_to_rect(form, Some(egui::Align::TOP));
        }
    }

    fn form(&mut self, ui: &mut Ui) {
        kit::note(
            ui,
            concat!(
                "A model reads the datasheets and drafts the component, symbol, package, footprint and 3D model through the ",
                sparksmith_core::product_name!(),
                " tools. The result is a new project of unreviewed drafts with a review report; nothing is published."
            ),
        );
        kit::note(
            ui,
            "Or use your own AI assistant: connect it with Help › Connect AI assistants… and ask it to run the datasheet_to_component prompt with these datasheets.",
        );
        if let Some(job) = &self.job {
            for line in self.log.iter().rev().take(8).rev() {
                kit::note(ui, line);
            }
            if ui_kit::button(ui, "Cancel", Button::Normal)
                .on_hover_text("Stops after the current request")
                .clicked()
            {
                job.cancel.store(true, Ordering::Relaxed);
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        kit::multiline_row(ui, "Datasheets", &mut self.datasheets, false)
            .on_hover_text("One PDF path or https URL per line");
        kit::text_row(ui, "Part name", &mut self.name);
        kit::text_row(ui, "Save as", &mut self.out)
            .on_hover_text("A new .sparksmith file; the review is written beside it");
        let chosen = self.chosen.get_or_insert_with(chosen_from_settings).clone();
        let model = match &chosen {
            Ok(chosen) => format!("{}: {}", chosen.endpoint.name, chosen.endpoint.summary()),
            Err(_) => "None set up".to_owned(),
        };
        kit::value_row(ui, "Model", &model);
        kit::hint(
            ui,
            "The model, its key and the limits are set in Settings › AI",
            "Settings › AI › Connections: the assistant's endpoint, else the first one.",
        );
        match &self.result {
            Some(Ok(done)) => {
                kit::value_row(
                    ui,
                    "Result",
                    &format!("{} ({} output tokens)", done.stop, done.output_tokens),
                );
                kit::value_row(ui, "Project", &done.project.display().to_string());
                kit::value_row(ui, "Review", &done.review.display().to_string());
            }
            Some(Err(message)) => {
                kit::refusal(ui, message);
            }
            None => {
                if let Err(why) = &chosen {
                    kit::refusal(ui, why);
                }
            }
        }
        ui.horizontal(|ui| {
            let ready = !self.datasheets.trim().is_empty()
                && !self.name.trim().is_empty()
                && self.out.trim().ends_with(".sparksmith")
                && chosen.is_ok();
            if ui
                .add_enabled_ui(ready, |ui| ui_kit::button(ui, "Start", Button::Primary))
                .inner
                .clicked()
            {
                self.start();
            }
            if ui_kit::button(ui, "Close", Button::Ghost).clicked() {
                self.open = false;
            }
        });
    }

    /// The run the form describes, or why it can't start.
    fn plan(&self) -> Result<Plan, String> {
        let chosen = match &self.chosen {
            Some(chosen) => chosen.clone(),
            None => chosen_from_settings(),
        }?;
        let endpoint = chosen.endpoint.endpoint().map_err(|e| e.to_string())?;
        let profile = chosen.endpoint.model_profile().map_err(|e| e.to_string())?;
        // The model selects its run profile: the user's file listing it,
        // else a built-in one, else the default.
        let folder = sparksmith_io::settings::UserDirs::from_env()
            .ok()
            .map(|dirs| run_profile::user_folder(&dirs.config));
        let shipped = AssistLibrary::locate().map_err(|e| e.to_string())?;
        let mut config = RunConfig::new(&shipped, profile, None, folder.as_deref())
            .map_err(|e| e.to_string())?;
        // The datasheet workflow's own budget, within the person's limits.
        let library =
            sparksmith_io::shipped::ShippedLibrary::locate().map_err(|e| e.to_string())?;
        let own = ai_settings::workflow_budget(&library, WORKFLOW).map_err(|e| e.to_string())?;
        if let Some(budget) = ai_settings::run_budget(chosen.limits.as_ref(), Some(&own)) {
            config.budget = budget;
        }
        let goal = Goal {
            name: self.name.trim().to_owned(),
            datasheets: self
                .datasheets
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(Datasheet::parse)
                .collect(),
            notes: String::new(),
        };
        Ok(Plan {
            goal,
            config,
            endpoint,
        })
    }

    fn start(&mut self) {
        let plan = match self.plan() {
            Ok(plan) => plan,
            Err(why) => {
                self.result = Some(Err(why));
                return;
            }
        };
        let out = PathBuf::from(self.out.trim());
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        self.log.clear();
        self.result = None;
        let spawned = std::thread::Builder::new()
            .name("datasheet-assist".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let result = work(plan, &out, &tx, &flag);
                let _ = tx.send(Message::Done(result));
            }));
        match spawned {
            Ok(_) => self.job = Some(Job { rx, cancel }),
            Err(e) => self.result = Some(Err(format!("could not start: {e}"))),
        }
    }
}

/// Where this run's key comes from: the one Settings › AI keeps for this
/// endpoint; otherwise, for Anthropic, the environment; otherwise none (a
/// local server).
fn key_source(plan: &Plan) -> Result<KeySource, String> {
    if plan.endpoint.check_key_allowed().is_ok() && keys::keychain_has(&plan.endpoint) {
        return Ok(KeySource::Keychain);
    }
    Ok(match plan.config.profile.provider {
        Provider::Anthropic => KeySource::Env("ANTHROPIC_API_KEY".into()),
        Provider::OpenAiCompatible => KeySource::None,
    })
}

/// The run on the background thread.
fn work(
    plan: Plan,
    out: &std::path::Path,
    tx: &Sender<Message>,
    cancel: &AtomicBool,
) -> Result<Finished, String> {
    for path in std::iter::once(out.to_path_buf()).chain(sparksmith_assist::review_paths(out)) {
        if path.exists() {
            return Err(format!("{} exists; choose a new name", path.display()));
        }
    }
    let connection = Connection {
        key: key_source(&plan)?,
        endpoint: plan.endpoint.clone(),
    };
    let context =
        sparksmith_core::library::standards::Context::current().map_err(|e| e.to_string())?;
    let mut backend =
        model::connect(&plan.config.profile, &connection).map_err(|e| e.to_string())?;
    let mut session = Session::new(
        sparksmith_io::new_project::profiled_engine(&context).map_err(|e| e.to_string())?,
    );
    let _ = tx.send(Message::Line(format!(
        "Run profile {}",
        plan.config.run_profile.name
    )));
    let outcome = sparksmith_assist::run(
        &plan.goal,
        &plan.config,
        backend.as_mut(),
        &mut session,
        &mut |event| {
            let line = match event {
                Event::Request { n, prompt_tokens } => {
                    format!("Request {n} (~{prompt_tokens} prompt tokens)")
                }
                Event::Response {
                    n,
                    stop_reason,
                    output_tokens,
                    seconds,
                } => {
                    format!("Response {n}: {stop_reason}, {output_tokens} tokens in {seconds:.0} s")
                }
                Event::Call {
                    name,
                    state: CallState::Done { step, .. },
                    ..
                } => format!(
                    "{name}: ok{}",
                    step.map(|s| format!(" ({s})")).unwrap_or_default()
                ),
                Event::Call {
                    name,
                    state: CallState::Refused { .. },
                    ..
                } => format!("{name}: refused"),
                Event::Call {
                    name,
                    state: CallState::NotRun { why },
                    ..
                } => format!("{name}: not run ({why})"),
                // The log shows lines, not pieces as they arrive.
                Event::TextDelta(_)
                | Event::ReasoningDelta(_)
                | Event::Usage(_)
                | Event::Call {
                    state: CallState::Writing | CallState::Running { .. },
                    ..
                } => return,
                Event::Text(text) => text.chars().take(160).collect(),
                Event::Warning(text) => format!("Warning: {text}"),
            };
            let _ = tx.send(Message::Line(line));
        },
        cancel,
    )
    .map_err(|e| e.to_string())?;
    let [_, review] =
        sparksmith_assist::write_outputs(&mut session, out, &outcome).map_err(|e| e.to_string())?;
    Ok(Finished {
        stop: outcome.stop.to_string(),
        output_tokens: outcome.facts.output_tokens,
        project: out.to_path_buf(),
        review,
    })
}

#[cfg(test)]
mod tests;

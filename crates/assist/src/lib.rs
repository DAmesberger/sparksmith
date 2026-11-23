//! AI-assisted component design from datasheets, on any configured model.
//!
//! A run gives a model the datasheets and a goal with the six verbs MCP
//! serves ([`verbs`]: `find`, `describe`, `query`, `apply`, `act`, `show`)
//! and runs every call in process through the front door of an automation
//! [`Session`] as the built-in agent, held to the run's reach profile
//! ([`profile::ReachProfile`]): the same catalog, gate and engine the MCP
//! server, CLI and desktop use. The run ends when the review check accepts
//! the model's [`report::ReviewReport`] (every value with its page and
//! table, confidence and open questions), with drafts in the session.
//! Nothing is published: the drafts carry the import review state and wait
//! for a person, and the reach profile keeps publishing, files and settings
//! out of reach.
//!
//! The model layer ([`model`]) is provider-neutral: a conversation of
//! messages, tool calls and results, images and documents; capabilities
//! declared per configured model; Anthropic Messages and OpenAI-compatible
//! backends; and one recording format. A model that can't read PDFs gets
//! text and rendered pages ([`documents`]), and the review says which.
//!
//! Caps are part of the loop ([`run()`]): requests, output tokens, total
//! tokens and time, the same for every model, hosted or local, each checked
//! before a request against the worst case of the next one. Sparksmith keeps
//! no prices and no spend: billing is the provider's. Keys come from the system
//! keychain per connection or a named environment variable, and are never
//! written anywhere ([`model::keys`]).
//!
//! This crate sits above `sparksmith-automation` and below the apps
//! (`cargo xtask layers`); the HTTP client stays out of the core.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod board_script;
pub mod chat;
pub mod desk;
pub mod documents;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixtures;
pub mod harness;
pub mod legacy;
pub mod model;
pub mod pins;
pub mod profile;
pub mod registry;
pub mod report;
mod request;
mod run;
pub mod settings;
pub mod usage;
pub mod verbs;
pub mod workflow;

use report::{ReviewReport, RunFacts};
use serde_json::json;
use sparksmith_automation::Session;
use std::path::PathBuf;
use workflow::control::Gauge;

pub use documents::{MAX_DATASHEET_BYTES, TOKENS_PER_PAGE, pdf_pages};
pub use profile::AssistLibrary;
pub use run::{MAX_RESULT_CHARS, RunConfig, run};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration: {0}")]
    Config(String),
    /// A feature that can't work on the configured model, and why.
    #[error("not possible with this model: {0}")]
    Unsupported(String),
    #[error("HTTP: {0}")]
    Http(String),
    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },
    /// The server answered, but not in the shape its protocol has.
    #[error("unexpected reply: {0}")]
    Protocol(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A datasheet: a local PDF or an https URL the API fetches.
#[derive(Clone, Debug, PartialEq)]
pub enum Datasheet {
    File(PathBuf),
    Url(String),
}

impl Datasheet {
    /// `https://…` is a URL; anything else a file path.
    pub fn parse(text: &str) -> Self {
        if text.starts_with("https://") {
            Self::Url(text.to_owned())
        } else {
            Self::File(PathBuf::from(text))
        }
    }
}

/// What to build.
#[derive(Clone, Debug)]
pub struct Goal {
    /// The part name the drafts take.
    pub name: String,
    pub datasheets: Vec<Datasheet>,
    /// Extra instructions (variant, package, what to include).
    pub notes: String,
}

/// Progress of a run, in the order it happens: what the chat draws
/// (`docs/assistant-panel.md` §7). Per request: [`Event::Request`], the
/// deltas and [`Event::Call`] rows in `Writing` while the reply streams,
/// [`Event::Response`] and [`Event::Usage`], the reply's [`Event::Text`],
/// then each call's row through `Running` to how it ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Request `n` is about to go out with an estimated prompt size.
    Request { n: u32, prompt_tokens: u64 },
    /// Answer text as it arrives.
    TextDelta(String),
    /// Reasoning as it arrives; the chat folds it.
    ReasoningDelta(String),
    /// Its response: why it stopped, its output tokens (reasoning
    /// included) and how long it took.
    Response {
        n: u32,
        stop_reason: String,
        output_tokens: u64,
        seconds: f64,
    },
    /// Use against the run's limits, after each response.
    Usage(Meter),
    /// The reply's whole answer text, once it ended: it replaces what the
    /// deltas showed.
    Text(String),
    /// A tool-call row. `call` counts request `request`'s calls from 0, so
    /// the two name the row; each new state replaces the last.
    Call {
        request: u32,
        call: u32,
        name: String,
        state: CallState,
    },
    /// Something a person should know about this run's result.
    Warning(String),
    /// The chat condensed what the model is sent (`docs/assistant-panel.md`
    /// §6.5): the divider row. The transcript is unchanged.
    Condensed(chat::context::Condensed),
    /// The harness sent the model its nudge (`note`) after `tokens` of
    /// reading with nothing changed; the turn goes on.
    Nudged { tokens: u64, note: String },
}

/// Where a tool call is.
#[derive(Clone, Debug, PartialEq)]
pub enum CallState {
    /// The model is writing it.
    Writing,
    /// It runs, with this input.
    Running { input: serde_json::Value },
    /// It ran: the reply the model reads, and the run log's step when it
    /// changed the document.
    Done { reply: String, step: Option<String> },
    /// It was refused or failed; the model reads why.
    Refused { reply: String },
    /// It never ran, and why: cut off at the output limit, or stopped.
    NotRun { why: String },
}

/// Use against a run's limits: requests, output tokens, all tokens and
/// whole minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter {
    pub requests: Gauge,
    pub output_tokens: Gauge,
    pub tokens: Gauge,
    pub minutes: Gauge,
    /// The tokens as the provider reported them: uncached, cache written,
    /// cache read and output.
    pub reported: crate::usage::Usage,
}

/// Why a run stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    /// The model submitted its review report.
    Submitted,
    /// The model answered a workflow step with `assist.step_result`.
    Answered,
    /// A cap would be exceeded by the next request.
    Budget(String),
    /// Replies kept stopping at the output limit.
    CutOff(String),
    Cancelled,
    /// The model declined.
    Refused(String),
    /// The model ended without a report.
    Ended,
    /// The model kept sending a call that was refused, unchanged.
    Repeated(String),
    /// The API or transport failed.
    Failed(String),
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Submitted => write!(f, "review report submitted"),
            Self::Answered => write!(f, "step answered"),
            Self::Budget(why) => write!(f, "cap reached: {why}"),
            Self::CutOff(why) => write!(f, "cut off: {why}"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Refused(why) => write!(f, "the model declined: {why}"),
            Self::Ended => write!(f, "the model ended without a report"),
            Self::Repeated(why) => write!(f, "stopped a repeating call: {why}"),
            Self::Failed(why) => write!(f, "failed: {why}"),
        }
    }
}

/// The result of a run: the report (if submitted) and what the runner
/// measured. The drafts are in the session.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub report: Option<ReviewReport>,
    pub stop: Stop,
    pub facts: RunFacts,
}

/// `NAME.review.json` and `NAME.review.md` beside the project `out`.
pub fn review_paths(out: &std::path::Path) -> [PathBuf; 2] {
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    [
        out.with_file_name(format!("{stem}.review.json")),
        out.with_file_name(format!("{stem}.review.md")),
    ]
}

/// Save the run's drafts as a new project at `out` (`project.save`) and
/// write the review beside it as JSON and Markdown, refusing to replace
/// any existing file. File effects, not document changes. Returns the
/// review paths.
pub fn write_outputs(
    session: &mut Session,
    out: &std::path::Path,
    outcome: &Outcome,
) -> Result<[PathBuf; 2]> {
    let reviews = review_paths(out);
    for path in std::iter::once(out).chain(reviews.iter().map(PathBuf::as_path)) {
        if path.try_exists()? {
            return Err(Error::Config(format!(
                "{} exists; choose a new output name",
                path.display()
            )));
        }
    }
    let saved = session.dispatch(json!({"command": "project.save",
        "params": {"path": out.to_string_lossy()}}));
    if saved["ok"] != true {
        return Err(Error::Config(format!("saving failed: {}", saved["error"])));
    }
    let json_report = json!({
        "report": outcome.report,
        "run": outcome.facts,
        "project": saved["result"],
    });
    let [json_path, md_path] = &reviews;
    write_new(
        json_path,
        serde_json::to_string_pretty(&json_report)?.as_bytes(),
    )?;
    write_new(
        md_path,
        report::markdown(outcome.report.as_ref(), &outcome.facts).as_bytes(),
    )?;
    Ok(reviews)
}

/// Create `path` with `bytes` atomically, refusing an existing file: a
/// failed or interrupted write leaves no partial file behind.
pub(crate) fn write_new(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    sparksmith_io::persistence::atomic_create(path, bytes).map_err(|error| match error {
        sparksmith_io::Error::Io(error) => Error::Io(error),
        other => Error::Config(format!("{}: {other}", path.display())),
    })
}

#[cfg(test)]
mod tests;

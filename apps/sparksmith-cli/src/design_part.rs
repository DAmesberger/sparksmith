//! `design-part`: author a component from datasheets with a configured model
//! (`sparksmith-assist`): Anthropic Messages or any OpenAI-compatible endpoint,
//! hosted or on this computer. It runs the datasheet workflow
//! (`library/assist/workflows/datasheet-to-component`) headless through the
//! one runner the chat uses, in an in-process session: the pinout import is
//! applied without asking and the model's generator choice stands. The
//! result is saved as a new project of drafts (component, symbol, package,
//! footprint, binding, generated 3D model), with the review report beside
//! it as `.review.json` and `.review.md`, however the run ended; Ctrl-C ends
//! it like the app's Stop. Nothing is published. `--record`
//! writes a recording that names the workflow; `--replay` runs one against
//! the same workflow version only; a recording that names no workflow (the
//! reference conversations in `examples/`) replays as the conversation it
//! holds.
//!
//! The model's capabilities are what the flags declare; nothing is detected.
//! A key comes from a named environment variable or the system keychain, is
//! never printed or written, and is refused over plain HTTP to another
//! computer. `llm-key` stores one in the keychain for one endpoint.
use super::{Result, interrupt};
use serde_json::Value;
use sparksmith_assist::model::profile::ANTHROPIC_BASE_URL;
use sparksmith_assist::model::record::{Recording, Replay};
use sparksmith_assist::model::{
    self, Budget, Connection, Endpoint, KeySource, ModelBackend, ModelProfile, OutputLimits,
    Provider, Secret, keys,
};
use sparksmith_assist::profile as run_profile;
use sparksmith_assist::workflow::control::{ControlAnswer, ControlCard};
use sparksmith_assist::workflow::event::WorkflowEvent;
use sparksmith_assist::workflow::runner::{self, Person, WorkflowRun};
use sparksmith_assist::workflow::{self, Loaded};
use sparksmith_assist::{AssistLibrary, CallState, Datasheet, Event, Outcome, RunConfig};
use sparksmith_automation::Session;
use sparksmith_core::product::SHORT_NAME as CLI;
use std::io::Read as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const USAGE: &str = "design-part --datasheet PDF|URL [--datasheet …] --name NAME --out NEW.sparksmith
              [--provider anthropic|openai] [--base-url URL] [--model ID] [--key none|keychain|env:NAME]
              [--context-tokens N] [--max-output-tokens N] [--vision yes|no] [--native-pdf yes|no]
              [--tools yes|no] [--effort LEVEL] [--request-timeout SECONDS]
              [--max-requests N] [--max-tokens N] [--max-seconds N]
              [--profile NAME] [--notes TEXT] [--record FILE.jsonl | --replay FILE.jsonl]
                                   draft a part from datasheets with a configured model (the
                                   datasheet workflow, headless); writes the project and its
                                   .review.md/.review.json, publishes nothing
  llm-key set|delete|status --base-url URL
                                   keep a model API key (read from stdin) in the system keychain,
                                   for that endpoint only
  llm-profile list | copy NAME     list run profiles, or copy a shipped one to the config folder to edit";

/// The workflow `design-part` runs.
const WORKFLOW: &str = "datasheet-to-component";

struct Args {
    datasheets: Vec<Datasheet>,
    name: String,
    out: PathBuf,
    notes: String,
    record: Option<PathBuf>,
    replay: Option<PathBuf>,
    config: RunConfig,
    connection: Connection,
}

/// The flags as given, before the profile is built from them.
#[derive(Default)]
struct Flags {
    provider: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    key: Option<String>,
    context: Option<u64>,
    max_output: Option<u32>,
    vision: Option<bool>,
    native_pdf: Option<bool>,
    tools: Option<bool>,
    effort: Option<String>,
    request_timeout: Option<u64>,
    profile: Option<String>,
}

fn yes_no(flag: &str, value: &str) -> Result<bool> {
    match value {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(format!("{flag} is yes or no").into()),
    }
}

fn count<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T> {
    value
        .parse::<T>()
        .map_err(|_| format!("{flag} takes a whole number").into())
}

fn key_source(text: &str) -> Result<KeySource> {
    match text {
        "none" => Ok(KeySource::None),
        "keychain" => Ok(KeySource::Keychain),
        other => match other.strip_prefix("env:") {
            Some(name) if !name.is_empty() => Ok(KeySource::Env(name.to_owned())),
            _ => Err("--key is none, keychain or env:NAME".into()),
        },
    }
}

/// The profile and connection the flags declare.
fn configure(flags: Flags, shipped: &AssistLibrary) -> Result<(RunConfig, Connection)> {
    let provider = Provider::parse(flags.provider.as_deref().unwrap_or("anthropic"))?;
    let (mut profile, base, key) = match provider {
        Provider::Anthropic => {
            let model = flags.model.unwrap_or_else(|| shipped.default_model.clone());
            let base = flags.base_url.unwrap_or_else(|| {
                std::env::var("ANTHROPIC_BASE_URL")
                    .ok()
                    .filter(|b| b.starts_with("https://"))
                    .unwrap_or_else(|| ANTHROPIC_BASE_URL.into())
            });
            let mut profile = ModelProfile::anthropic(&model);
            if let Some(context) = flags.context {
                profile.capabilities.context_tokens = context;
            }
            let key = KeySource::Env("ANTHROPIC_API_KEY".into());
            (profile, base, key)
        }
        Provider::OpenAiCompatible => {
            let base = flags.base_url.ok_or(
                "--provider openai needs --base-url (for example http://localhost:11434/v1)",
            )?;
            let model = flags.model.ok_or("--provider openai needs --model")?;
            let context = flags.context.ok_or(
                "--provider openai needs --context-tokens: the context the server gives this model",
            )?;
            let profile = ModelProfile::openai_compatible(&model, context);
            (profile, base, KeySource::None)
        }
    };
    if let Some(n) = flags.max_output {
        let after = OutputLimits::for_context(profile.capabilities.context_tokens).after_cut;
        profile.output = OutputLimits {
            per_request: n,
            after_cut: after.max(n),
        };
    }
    if let Some(v) = flags.vision {
        profile.capabilities.vision = v;
    }
    if let Some(v) = flags.native_pdf {
        profile.capabilities.native_pdf = v;
    }
    if let Some(v) = flags.tools {
        profile.capabilities.tool_calling = v;
    }
    if let Some(effort) = flags.effort {
        let allowed: &[&str] = match provider {
            Provider::Anthropic => &["low", "medium", "high", "xhigh", "max"],
            Provider::OpenAiCompatible => &["none", "minimal", "low", "medium", "high"],
        };
        if !allowed.contains(&effort.as_str()) {
            return Err(format!("--effort is one of {}", allowed.join(", ")).into());
        }
        profile.effort = Some(effort);
    }
    if let Some(seconds) = flags.request_timeout {
        profile.request_timeout = Duration::from_secs(seconds);
    }
    let config = RunConfig::new(
        shipped,
        profile,
        flags.profile.as_deref(),
        user_profiles().as_deref(),
    )?;
    let key = match flags.key {
        Some(text) => key_source(&text)?,
        None => key,
    };
    let connection = Connection {
        endpoint: Endpoint::parse(&base)?,
        key,
    };
    Ok((config, connection))
}

/// The model's flags (provider, key, capabilities, limits, profile) and
/// `--record`/`--replay`, which `design-part` and `design-body` share.
#[derive(Default)]
pub(super) struct ModelFlags {
    flags: Flags,
    budget: Budget,
    pub(super) record: Option<PathBuf>,
    pub(super) replay: Option<PathBuf>,
}

impl ModelFlags {
    /// Take `flag` with its `value` when it is a model flag; `false` when
    /// it is another command's.
    pub(super) fn take(&mut self, f: &str, value: String) -> Result<bool> {
        let (flags, budget) = (&mut self.flags, &mut self.budget);
        match f {
            "--provider" => flags.provider = Some(value),
            "--base-url" => flags.base_url = Some(value),
            "--model" => flags.model = Some(value),
            "--key" => flags.key = Some(value),
            "--context-tokens" => flags.context = Some(count(f, &value)?),
            "--max-output-tokens" => flags.max_output = Some(count(f, &value)?),
            "--vision" => flags.vision = Some(yes_no(f, &value)?),
            "--native-pdf" => flags.native_pdf = Some(yes_no(f, &value)?),
            "--tools" => flags.tools = Some(yes_no(f, &value)?),
            "--effort" => flags.effort = Some(value),
            "--request-timeout" => flags.request_timeout = Some(count(f, &value)?),
            "--profile" => flags.profile = Some(value),
            "--max-requests" => budget.max_requests = Some(count(f, &value)?),
            "--max-tokens" => budget.max_total_tokens = Some(count(f, &value)?),
            "--max-seconds" => {
                budget.max_duration = Some(Duration::from_secs(count(f, &value)?));
            }
            "--record" => self.record = Some(PathBuf::from(value)),
            "--replay" => self.replay = Some(PathBuf::from(value)),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The run's configuration and connection, the shipped defaults filled
    /// in.
    pub(super) fn configure(self) -> Result<(RunConfig, Connection)> {
        if self.record.is_some() && self.replay.is_some() {
            return Err("--record and --replay exclude each other".into());
        }
        let shipped = AssistLibrary::locate()?;
        let (mut config, connection) = configure(self.flags, &shipped)?;
        config.budget = self.budget;
        Ok((config, connection))
    }
}

fn parse(args: &[String]) -> Result<Args> {
    let mut datasheets = Vec::new();
    let (mut name, mut out, mut notes) = (String::new(), PathBuf::new(), String::new());
    let mut model = ModelFlags::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it
            .next()
            .ok_or_else(|| format!("{flag} needs a value\nusage: {CLI} {USAGE}"))?
            .clone();
        match flag.as_str() {
            "--datasheet" => datasheets.push(Datasheet::parse(&value)),
            "--name" => name = value,
            "--out" => out = PathBuf::from(value),
            "--notes" => notes = value,
            other => {
                if !model.take(other, value)? {
                    return Err(format!("unknown option {other}\nusage: {CLI} {USAGE}").into());
                }
            }
        }
    }
    if datasheets.is_empty() || name.trim().is_empty() || out.as_os_str().is_empty() {
        return Err(format!("usage: {CLI} {USAGE}").into());
    }
    let (record, replay) = (model.record.clone(), model.replay.clone());
    let (config, connection) = model.configure()?;
    Ok(Args {
        datasheets,
        name,
        out,
        notes,
        record,
        replay,
        config,
        connection,
    })
}

pub(super) fn print_event(event: Event) {
    match event {
        Event::Request { n, prompt_tokens } => {
            eprintln!("request {n}: ~{prompt_tokens} prompt tokens");
        }
        Event::Response {
            n,
            stop_reason,
            output_tokens,
            seconds,
        } => {
            eprintln!(
                "response {n}: {stop_reason}; {output_tokens} output tokens in {seconds:.0} s"
            );
        }
        Event::Call {
            name,
            state: CallState::Done { step, .. },
            ..
        } => eprintln!(
            "  {name}: ok{}",
            step.map(|s| format!(" — {s}")).unwrap_or_default()
        ),
        Event::Call {
            name,
            state: CallState::Refused { reply },
            ..
        } => eprintln!("  {name}: refused: {}", refusal_line(&reply)),
        Event::Call {
            name,
            state: CallState::NotRun { why },
            ..
        } => eprintln!("  {name}: not run ({why})"),
        // Lines, not pieces as they arrive.
        Event::TextDelta(_)
        | Event::ReasoningDelta(_)
        | Event::Condensed(_)
        | Event::Nudged { .. }
        | Event::Usage(_)
        | Event::Call {
            state: CallState::Writing | CallState::Running { .. },
            ..
        } => {}
        Event::Text(text) => {
            let line: String = text.chars().take(200).collect();
            eprintln!("  model: {line}");
        }
        Event::Warning(text) => eprintln!("warning: {text}"),
    }
}

/// Most characters of a refusal shown on its line.
const REFUSAL_CHARS: usize = 300;

/// Why a call was refused, as one line: what the model read, cut to its
/// first [`REFUSAL_CHARS`] characters, so a headless run's log says what
/// the model was told.
fn refusal_line(reply: &str) -> String {
    let mut line: String = reply.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((end, _)) = line.char_indices().nth(REFUSAL_CHARS) {
        line.truncate(end);
        line.push('…');
    }
    line
}

/// The person of a headless run: the run's steps and cards as lines on
/// standard error, each finished step with its seconds; the pinout import
/// is applied (the drafts wait for review in the new project, and nothing
/// is published); a choice the model made stands.
#[derive(Default)]
pub(super) struct Console {
    /// When the current step (or its retry) began.
    step_started: Option<Instant>,
    /// Seconds spent waiting for model replies, summed over the run.
    model_seconds: f64,
}

impl Console {
    /// Seconds since the current step began, restarting the clock.
    fn lap(&mut self) -> f64 {
        let now = Instant::now();
        self.step_started
            .replace(now)
            .map_or(0.0, |started| (now - started).as_secs_f64())
    }
}

impl Person for Console {
    fn event(&mut self, event: &WorkflowEvent) {
        match event {
            WorkflowEvent::Started { label, .. } => eprintln!("{label}"),
            WorkflowEvent::Step {
                step, done, total, ..
            } => {
                self.step_started = Some(Instant::now());
                eprintln!("step {}/{total}: {step}", done + 1);
            }
            WorkflowEvent::StepDone { summary, .. } => {
                let seconds = self.lap();
                if summary.is_empty() {
                    eprintln!("  ({seconds:.1} s)");
                } else {
                    eprintln!("  {summary} ({seconds:.1} s)");
                }
            }
            WorkflowEvent::Retry {
                step,
                attempt,
                of,
                why,
                ..
            } => {
                let seconds = self.lap();
                let why: String = why.chars().take(200).collect();
                eprintln!("  back to {step} ({attempt} of {of}) after {seconds:.1} s: {why}");
            }
            WorkflowEvent::Card {
                card: ControlCard::Choice { field, value, .. },
                ..
            } => eprintln!("  {}: {}", field.title, value.join(", ")),
            WorkflowEvent::Ended { end, .. } => eprintln!("{end:?}"),
            _ => {}
        }
    }

    fn model_event(&mut self, event: &Event) {
        if let Event::Response { seconds, .. } = event {
            self.model_seconds += seconds;
        }
        print_event(event.clone());
    }

    fn answer(&mut self, _step: &str, cards: &[ControlCard]) -> Option<ControlAnswer> {
        cards
            .iter()
            .any(|card| matches!(card, ControlCard::Impact { .. }))
            .then(|| ControlAnswer::Apply {
                leave_out: Vec::new(),
            })
    }
}

/// How long a key read may take before the run says what it waits for.
const KEY_PATIENCE: Duration = Duration::from_secs(2);

/// `work` on its own thread; when it takes longer than `patience`, `say`
/// gets `notice` once while the wait goes on. The result and its seconds.
fn waited<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    patience: Duration,
    notice: &str,
    say: &mut dyn FnMut(&str),
) -> Result<(T, f64)> {
    let started = Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("key-read".into())
        .spawn(move || {
            let _ = tx.send(work());
        })?;
    let value = match rx.recv_timeout(patience) {
        Ok(value) => value,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            say(notice);
            rx.recv().map_err(|_| "the key could not be read")?
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            return Err("the key could not be read".into());
        }
    };
    Ok((value, started.elapsed().as_secs_f64()))
}

/// The connection's backend, its key read first. A keychain read can wait
/// on the system: macOS asks before a program the key was not stored by
/// (a new build too) may read it, and nothing moves until that dialog is
/// answered. So a slow read says what it waits for, and a keychain read
/// prints its seconds.
pub(super) fn connect(
    config: &RunConfig,
    connection: &Connection,
) -> Result<(Box<dyn ModelBackend>, f64)> {
    let source = connection.key.clone();
    let endpoint = connection.endpoint.clone();
    let keychain = source == KeySource::Keychain;
    let notice = format!(
        "waiting for the system keychain: allow access to the key for {} in the system's dialog \
         (it asks again after each new build; --key env:NAME does not wait)",
        endpoint.base()
    );
    let (key, seconds) = waited(
        move || source.resolve(&endpoint),
        KEY_PATIENCE,
        &notice,
        &mut |line| eprintln!("{line}"),
    )?;
    let key = key?;
    if keychain {
        eprintln!("key: read from the system keychain in {seconds:.1} s");
    }
    let backend = model::connect_with_key(&config.profile, &connection.endpoint, key)?;
    Ok((backend, seconds))
}

/// The workflow `id` as this program lists it.
pub(super) fn installed_workflow(id: &str) -> Result<Loaded> {
    let listed = workflow::load::host();
    let found = listed
        .iter()
        .find(|l| l.id == id)
        .ok_or_else(|| format!("the workflow {id} is not installed"))?;
    if !found.why_not.is_empty() {
        return Err(format!("{id} cannot run: {}", found.why_not.join("; ")).into());
    }
    Ok(found
        .loaded
        .clone()
        .ok_or_else(|| format!("{id} did not load"))?)
}

/// The workflow's inputs from the flags: the datasheets, the part name and
/// the notes.
fn inputs(args: &Args) -> serde_json::Map<String, Value> {
    let mut inputs = serde_json::Map::new();
    let sheets: Vec<Value> = args
        .datasheets
        .iter()
        .map(|sheet| match sheet {
            Datasheet::File(path) => Value::String(path.display().to_string()),
            Datasheet::Url(url) => Value::String(url.clone()),
        })
        .collect();
    inputs.insert("datasheets".into(), Value::Array(sheets));
    inputs.insert("part".into(), Value::String(args.name.clone()));
    if !args.notes.trim().is_empty() {
        inputs.insert("notes".into(), Value::String(args.notes.clone()));
    }
    inputs
}

/// `design-part`: the datasheet workflow, run headless through the one
/// runner the chat uses, on a new project saved at `--out` with its review.
pub fn run(args: &[String]) -> Result<()> {
    let started = Instant::now();
    let args = parse(args)?;
    for path in std::iter::once(args.out.clone()).chain(sparksmith_assist::review_paths(&args.out))
    {
        if path.try_exists()? {
            return Err(format!("{} exists; choose a new output name", path.display()).into());
        }
    }
    let replay = match args.replay.as_deref().map(Replay::from_file).transpose()? {
        Some(replay) if replay.workflow.is_none() => return replay_conversation(&args, replay),
        other => other,
    };
    let loaded = installed_workflow(WORKFLOW)?;
    let (mut backend, key_seconds): (Box<dyn ModelBackend>, f64) = match (replay, &args.record) {
        (Some(replay), _) => (Box::new(replay), 0.0),
        (None, Some(path)) => {
            let (inner, seconds) = connect(&args.config, &args.connection)?;
            let recording = Recording::for_workflow(inner, path.clone(), runner::mark(&loaded))?;
            (Box::new(recording), seconds)
        }
        (None, None) => connect(&args.config, &args.connection)?,
    };
    let mut session =
        super::user_session(sparksmith_io::new_project::run_engine(&super::context()?)?);
    let cancel = interrupt::stop_on_ctrl_c()?;
    let run = WorkflowRun {
        loaded: &loaded,
        inputs: inputs(&args),
        grant: args.config.reach.grant(),
        config: &args.config,
    };
    let mut console = Console::default();
    let outcome = runner::run_workflow(run, backend.as_mut(), &mut session, &mut console, &cancel)?;
    let review = outcome.review(&session);
    let saved = save(&mut session, &args, &review);
    if let Some(path) = &args.record {
        eprintln!("recording: {}", path.display());
    }
    let total = started.elapsed().as_secs_f64();
    let model = console.model_seconds;
    let local = (total - key_seconds - model).max(0.0);
    eprintln!(
        "time: {total:.1} s in all: {model:.1} s waiting for the model, \
         {key_seconds:.1} s reading the key, {local:.1} s of local work"
    );
    saved
}

/// A recording that names no workflow: the reference conversations in
/// `examples/` (one conversation with the tools, before the datasheet task
/// was a workflow), replayed as recorded to the project and review they
/// made. A model run never takes this path.
fn replay_conversation(args: &Args, mut replay: Replay) -> Result<()> {
    let goal = sparksmith_assist::Goal {
        name: args.name.clone(),
        datasheets: args.datasheets.clone(),
        notes: args.notes.clone(),
    };
    let mut session =
        super::user_session(sparksmith_io::new_project::run_engine(&super::context()?)?);
    let outcome = sparksmith_assist::run(
        &goal,
        &args.config,
        &mut replay,
        &mut session,
        &mut print_event,
        interrupt::stop_on_ctrl_c()?.as_ref(),
    )?;
    save(&mut session, args, &outcome)
}

/// Save the drafts at `--out` and the review beside them; a run without an
/// accepted review report is an error, after the files are written.
fn save(session: &mut Session, args: &Args, outcome: &Outcome) -> Result<()> {
    let [_, written] = sparksmith_assist::write_outputs(session, &args.out, outcome)?;
    let facts = &outcome.facts;
    eprintln!(
        "{}: {} requests, {} output tokens, {:.0} s; drafts saved to {} (review {})",
        outcome.stop,
        facts.requests,
        facts.output_tokens,
        facts.elapsed_seconds,
        args.out.display(),
        written.display()
    );
    if outcome.report.is_none() {
        return Err(format!("no review report: {}", outcome.stop).into());
    }
    Ok(())
}

/// The user's run-profile folder, when the user directories are known.
fn user_profiles() -> Option<PathBuf> {
    sparksmith_io::settings::UserDirs::from_env()
        .ok()
        .map(|dirs| run_profile::user_folder(&dirs.config))
}

/// `llm-profile list | copy NAME`.
pub fn profile(args: &[String]) -> Result<()> {
    let folder = user_profiles().ok_or("no config folder: set SPARKSMITH_CONFIG_DIR or HOME")?;
    let shipped = AssistLibrary::locate()?;
    match args {
        [action] if action == "list" => {
            for name in shipped.built_in_names() {
                let p = shipped.built_in(name).ok_or("listed")?;
                println!("{name} (shipped) {}", p.models.join(", "));
            }
            if let Ok(entries) = std::fs::read_dir(&folder) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    match run_profile::read_file(&path) {
                        Ok(p) => {
                            println!("{} ({}) {}", p.name, path.display(), p.models.join(", "))
                        }
                        Err(e) => println!("broken: {e}"),
                    }
                }
            }
        }
        [action, name] if action == "copy" => {
            let path = shipped.copy_to_user(name, &folder)?;
            eprintln!(
                "copied to {}; edit it, and it takes the shipped one's place",
                path.display()
            );
        }
        _ => return Err(format!("usage: {CLI} {USAGE}").into()),
    }
    Ok(())
}

/// `llm-key set|delete|status --base-url URL`.
pub fn key(args: &[String]) -> Result<()> {
    let [action, flag, url] = args else {
        return Err(format!("usage: {CLI} {USAGE}").into());
    };
    if flag != "--base-url" {
        return Err(format!("usage: {CLI} {USAGE}").into());
    }
    let endpoint = Endpoint::parse(url)?;
    match action.as_str() {
        "set" => {
            let mut text = String::new();
            std::io::stdin().take(16 * 1024).read_to_string(&mut text)?;
            let secret = Secret::new(&text).ok_or("give the key on standard input")?;
            keys::keychain_store(&endpoint, &secret)?;
            eprintln!("key stored in the system keychain for {}", endpoint.base());
        }
        "delete" => {
            let removed = keys::keychain_delete(&endpoint)?;
            eprintln!(
                "{} {}",
                if removed {
                    "key removed for"
                } else {
                    "no key for"
                },
                endpoint.base()
            );
        }
        "status" => {
            let has = keys::keychain_has(&endpoint);
            eprintln!(
                "{}: {} ({})",
                endpoint.base(),
                if has {
                    "a key is stored"
                } else {
                    "no key stored"
                },
                endpoint.location()
            );
        }
        _ => return Err(format!("usage: {CLI} {USAGE}").into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn a_local_model_is_configured_from_its_flags() -> Result<()> {
        let parsed = parse(&args(
            "--datasheet a.pdf --name X --out x.sparksmith --provider openai \
             --base-url http://localhost:11434/v1 --model qwen --context-tokens 65536 \
             --vision yes --max-seconds 3600",
        ))?;
        let p = &parsed.config.profile;
        assert_eq!(p.provider, Provider::OpenAiCompatible);
        assert_eq!(p.model, "qwen");
        assert!(p.capabilities.vision && !p.capabilities.native_pdf);
        assert_eq!(p.capabilities.context_tokens, 65_536);
        assert_eq!(p.output.per_request, 32_000);
        assert_eq!(parsed.connection.key, KeySource::None);
        assert!(parsed.connection.endpoint.is_loopback());
        // Only what a flag sets is limited.
        assert_eq!(
            parsed.config.budget,
            Budget {
                max_duration: Some(Duration::from_secs(3600)),
                ..Budget::default()
            }
        );
        Ok(())
    }

    #[test]
    fn the_defaults_are_todays_anthropic_run() -> Result<()> {
        let parsed = parse(&args("--datasheet a.pdf --name X --out x.sparksmith"))?;
        let defaults = RunConfig::shipped_default(&AssistLibrary::locate()?)?;
        assert_eq!(parsed.config.profile, defaults.profile);
        assert_eq!(parsed.config.run_profile, defaults.run_profile);
        assert_eq!(parsed.config.base_prompt, defaults.base_prompt);
        assert_eq!(parsed.config.budget, Budget::default());
        // The run reaches what the shipped datasheet profile lets it: no
        // files, so it names no folder; the CLI writes the outputs itself.
        assert_eq!(parsed.config.reach, defaults.reach);
        assert!(parsed.config.reach.grant().folders.is_empty());
        assert_eq!(
            parsed.connection.key,
            KeySource::Env("ANTHROPIC_API_KEY".into())
        );
        Ok(())
    }

    /// A key read the system holds up (the macOS keychain's access dialog)
    /// says what it waits for once, and its seconds count; a quick one says
    /// nothing.
    #[test]
    fn a_slow_key_read_says_what_it_waits_for() -> Result<()> {
        let mut said = Vec::new();
        let (value, seconds) = waited(
            || {
                std::thread::sleep(Duration::from_millis(300));
                7
            },
            Duration::from_millis(20),
            "waiting",
            &mut |line| said.push(line.to_owned()),
        )?;
        assert_eq!((value, said), (7, vec!["waiting".to_owned()]));
        assert!(seconds >= 0.3, "{seconds}");
        let mut said = Vec::new();
        let (value, _) = waited(|| 8, Duration::from_secs(5), "waiting", &mut |line| {
            said.push(line.to_owned())
        })?;
        assert_eq!(value, 8);
        assert!(said.is_empty());
        Ok(())
    }

    /// A refused call's line says why, on one line and bounded.
    #[test]
    fn a_refusal_is_shown_as_one_bounded_line() {
        assert_eq!(
            refusal_line("{\"code\":\"not_found\",\n  \"message\":\"nothing at draft:5\"}"),
            "{\"code\":\"not_found\", \"message\":\"nothing at draft:5\"}"
        );
        let long = refusal_line(&"é".repeat(REFUSAL_CHARS + 10));
        assert_eq!(long.chars().count(), REFUSAL_CHARS + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn missing_declarations_and_bad_values_are_refused() {
        for bad in [
            "--provider openai --model m --context-tokens 1000",
            "--provider openai --base-url http://localhost:1/v1 --context-tokens 1000",
            "--provider openai --base-url http://localhost:1/v1 --model m",
            "--provider other",
            "--vision maybe",
            "--key secret",
            "--effort extreme",
            "--provider openai --base-url ftp://x --model m --context-tokens 1",
        ] {
            let text = format!("--datasheet a.pdf --name X --out x.sparksmith {bad}");
            assert!(parse(&args(&text)).is_err(), "{bad}");
        }
    }
}

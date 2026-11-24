//! The tool loop of a run, on any backend: a goal, the six verbs, a budget,
//! events and a recording. Every tool call goes through
//! [`dispatch_tool_call`] into the verbs ([`crate::verbs`]), and so through
//! the front door and its gate as the built-in agent.
//!
//! The run ends when the review check (`query library.review_check`)
//! accepts the model's report: that report is the run's. The same loop
//! ([`converse`]) runs each model step of a workflow
//! ([`crate::workflow::runner`]), which ends when `assist.step_result`
//! accepts the step's answer.
use crate::desk::{self, Called, Desk};
use crate::documents::{self, Prepared};
use crate::harness::{CondenseHistory, ShapeReply, SurfaceStrategy};
use crate::model::{
    Budget, Image, Message, ModelBackend, ModelProfile, ModelRequest, Part, Role, StopReason,
    ToolSpec, appended_chars,
};
use crate::profile::{AssistLibrary, ReachProfile, RunProfile};
use crate::report::{self, RequestFacts, ReviewReport, RunFacts};
use crate::usage::Tally;
use crate::verbs;
use crate::{CallState, Error, Event, Goal, Outcome, Result, Stop, legacy};
use serde_json::{Value, json};
use sparksmith_automation::session::assist::Documents;
use sparksmith_automation::{Grant, Session};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Model text answers before a nudge to submit stops being sent.
const MAX_NUDGES: u32 = 2;
/// Replies in a row cut off at the output limit before the run gives up: the
/// model is making no progress, and that is reported as what it is.
const MAX_CUTS_IN_A_ROW: u32 = 3;
/// Least room for output worth sending a request: a tool call took 806 tokens
/// on a local model, reasoning included, and even that was a simple one.
const MIN_OUTPUT_ROOM: u64 = 4_000;
/// Characters of one tool result sent back; longer results are cut with a
/// note (the model can ask for a narrower view).
pub const MAX_RESULT_CHARS: usize = 120_000;
/// Added to the estimate of the next prompt.
const ESTIMATE_MARGIN: u64 = 4_000;

/// What a run may use, on which model.
#[derive(Clone, Debug)]
pub struct RunConfig {
    pub profile: ModelProfile,
    /// What the model is told and how the loop reacts ([`crate::profile`]).
    pub run_profile: RunProfile,
    /// The installed and user harness profiles available to the selected connection.
    pub harnesses: crate::harness::HarnessLibrary,
    /// The shipped base system prompt the run profile builds on
    /// ([`AssistLibrary::system`]).
    pub base_prompt: String,
    pub budget: Budget,
    /// What the run may reach: the catalog filter whose modules are the
    /// grant the gate holds every call to ([`ReachProfile`]).
    pub reach: ReachProfile,
}

impl RunConfig {
    /// A run of `profile` with the run profile it selects from `shipped`
    /// and the user's profile `folder`, and no limits. The profile's
    /// declarations are set on the model.
    pub fn new(
        shipped: &AssistLibrary,
        mut profile: ModelProfile,
        run_profile: Option<&str>,
        folder: Option<&std::path::Path>,
    ) -> Result<Self> {
        let run_profile = shipped.select(run_profile, &profile.model, folder)?;
        run_profile.declare(&mut profile);
        let mut harnesses = shipped.harnesses.clone();
        harnesses.user_folder = folder
            .and_then(std::path::Path::parent)
            .map(|config| config.join("assist-harness"));
        Ok(Self {
            profile,
            run_profile,
            harnesses,
            base_prompt: shipped.system.clone(),
            budget: Budget::default(),
            reach: shipped.reach.clone(),
        })
    }

    /// A workflow step resolves its profile before opening a turn. An
    /// older run profile's repeat override belongs to the model level.
    pub fn harness_for(
        &self,
        workflow: Option<&str>,
        step: Option<&str>,
    ) -> Result<crate::harness::HarnessProfile> {
        let selection = crate::harness::HarnessSelection {
            model: self.run_profile.harness_profile.as_deref(),
            workflow,
            step,
            connection: self.profile.harness_profile.as_deref(),
        };
        let mut harness = self.harnesses.select(selection)?;
        if workflow.is_none()
            && step.is_none()
            && selection.connection.is_none()
            && let Some(repeat) = self.run_profile.harness.repeat_limit
        {
            harness.recover.repeat_limit = repeat;
        }
        Ok(harness)
    }

    /// A run of the shipped default model on the Messages API.
    pub fn shipped_default(shipped: &AssistLibrary) -> Result<Self> {
        Self::new(
            shipped,
            ModelProfile::anthropic(&shipped.default_model),
            None,
            None,
        )
    }
}

/// What a tool call produced.
#[derive(Debug)]
pub(crate) enum ToolOutcome {
    /// Text for the model, whether it is an error, and a one-line step
    /// for the run log when the call changed the document.
    Reply {
        text: String,
        error: bool,
        step: Option<String>,
        /// Images that go with the text (a rendered datasheet page).
        images: Vec<Image>,
    },
    /// The review check accepted the model's report: the run is complete.
    Report(Box<ReviewReport>),
    /// `assist.step_result` accepted the answer of a workflow's model step.
    Step(Value),
}

/// What ends a conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Finish {
    /// `library.review_check` accepts the report: a whole datasheet run.
    Report,
    /// `assist.step_result` accepts the answer: a workflow's model step.
    StepResult,
}

/// What a finished conversation produced.
#[derive(Debug)]
pub(crate) enum Finished {
    Report(Box<ReviewReport>),
    Step(Value),
}

/// What tools act on during a run.
pub(crate) struct ToolContext<'a> {
    /// Where the calls run: the run's own session, or the window's.
    pub(crate) desk: &'a mut dyn Desk,
    pub(crate) grant: Grant,
    /// The open turn every call joins (a workflow run's).
    pub(crate) turn: Option<String>,
    /// The replies call the tools of before the verbs ([`legacy`]).
    pub(crate) legacy: bool,
    /// Reported in the review: which document's pages the model looked at.
    pub(crate) facts: Vec<documents::DocumentFacts>,
    /// What ends the conversation.
    pub(crate) finish: Finish,
}

/// One tool call, whatever the model asked: the one door between a model and
/// the session, answering with the label the run log gives the call. A call
/// whose arguments could not be read never runs; its error goes back so the
/// model can send it again.
fn dispatch_tool_call(
    context: &mut ToolContext<'_>,
    name: &str,
    input: &Value,
    malformed: Option<&str>,
    harness: &crate::harness::HarnessProfile,
) -> (String, ToolOutcome) {
    if let Some(why) = malformed {
        return (
            name.to_owned(),
            ToolOutcome::Reply {
                text: format!("This call was not run: {why}. Send it again with valid arguments."),
                error: true,
                step: None,
                images: Vec::new(),
            },
        );
    }
    let (verb, arguments, label) = if verbs::NAMES.contains(&name) {
        (name, input.clone(), label(name, input))
    } else {
        let translated = if context.legacy {
            let (name, input) = (name.to_owned(), input.clone());
            desk::with(context.desk, move |session| {
                legacy::translate(session, &name, &input)
            })
            .flatten()
        } else {
            None
        };
        match translated {
            Some(Ok((verb, arguments))) => (verb, arguments, name.to_owned()),
            Some(Err(why)) => {
                return (name.to_owned(), reply(json!({"refused": why}), true, None));
            }
            None => {
                let refused = verb_call(context, name, input.clone()).envelope;
                return (name.to_owned(), reply(refused["error"].clone(), true, None));
            }
        }
    };
    let Called {
        envelope: answer,
        before,
        after,
    } = verb_call(context, verb, arguments.clone());
    if answer["ok"] != true {
        return (label, reply(answer["error"].clone(), true, None));
    }
    let mut result = answer["result"].clone();
    let entry = arguments["name"].as_str().unwrap_or_default();
    if verb == verbs::QUERY && entry == "library.review_check" && context.finish == Finish::Report {
        return (label, review(&arguments, result));
    }
    if verb == verbs::QUERY
        && entry == "assist.step_result"
        && context.finish == Finish::StepResult
        && result["accepted"] == true
    {
        let mut params = arguments["params"].clone();
        if sparksmith_automation::mcp::lenient::read_params(entry, &mut params).is_ok() {
            return (label, ToolOutcome::Step(params["result"].clone()));
        }
    }
    if verb == verbs::QUERY && entry == "assist.document_page" {
        let mut images = Vec::new();
        if let Some(image) = result.as_object_mut().and_then(|r| r.remove("image")) {
            images.push(Image {
                media_type: image["media_type"].as_str().unwrap_or_default().to_owned(),
                data: image["data"].as_str().unwrap_or_default().to_owned(),
            });
        }
        let viewed = context
            .facts
            .iter_mut()
            .find(|f| result["document"].as_str() == Some(f.title.as_str()));
        if let (Some(facts), Some(page)) = (viewed, result["page"].as_u64()) {
            facts
                .pages_viewed
                .push(u32::try_from(page).unwrap_or(u32::MAX));
        }
        let text = result["text"].as_str().unwrap_or_default().to_owned();
        return (
            label,
            ToolOutcome::Reply {
                text,
                error: false,
                step: None,
                images,
            },
        );
    }
    let step = (after != before).then(|| {
        format!(
            "{label}: revision {after} ({})",
            summary(result.get("outcome").unwrap_or(&result["outcomes"][0]))
        )
    });
    integers_as_text(&mut result);
    let text =
        harness
            .reply_strategy()
            .render(verb, &arguments, result, harness.policy().reply_chars);
    (
        label,
        ToolOutcome::Reply {
            text,
            error: false,
            step,
            images: Vec::new(),
        },
    )
}

fn verb_call(context: &mut ToolContext<'_>, verb: &str, input: Value) -> Called {
    context
        .desk
        .verb(&context.grant, context.turn.as_deref(), verb, input)
}

/// How the run log names a verb call: the verb and the entries it names.
fn label(verb: &str, input: &Value) -> String {
    let names: Vec<&str> = if verb == verbs::APPLY {
        input["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|step| step["op"].as_str())
            .collect()
    } else {
        input["name"].as_str().into_iter().collect()
    };
    if names.is_empty() {
        verb.to_owned()
    } else {
        format!("{verb} {}", names.join(", "))
    }
}

/// The review check's answer: the report when it was accepted, else the
/// problems as a refusal the model reads.
fn review(arguments: &Value, result: Value) -> ToolOutcome {
    if result["accepted"] != true {
        return reply(result, true, None);
    }
    let mut params = arguments["params"].clone();
    if sparksmith_automation::mcp::lenient::read_params("library.review_check", &mut params)
        .is_err()
    {
        return reply(result, true, None);
    }
    match serde_json::from_value::<ReviewReport>(params["report"].clone()) {
        Ok(report) => ToolOutcome::Report(Box::new(report)),
        Err(e) => reply(
            json!({"refused": format!("not a review report: {e}")}),
            true,
            None,
        ),
    }
}

fn reply(mut value: Value, error: bool, step: Option<String>) -> ToolOutcome {
    integers_as_text(&mut value);
    let mut text = value.to_string();
    if text.len() > MAX_RESULT_CHARS {
        let mut end = MAX_RESULT_CHARS;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("… [cut: the result exceeded the tool reply limit; ask for a narrower view]");
    }
    ToolOutcome::Reply {
        text,
        error,
        step,
        images: Vec::new(),
    }
}

/// Largest integer a JSON reader that parses numbers as doubles keeps exact.
const MAX_EXACT_INTEGER: u64 = 1 << 53;

/// Integers beyond [`MAX_EXACT_INTEGER`] as text, so a model (or a server
/// that parses JSON numbers as doubles) can't round them: a model once
/// copied a preview's 19-digit base as a float and was refused 54 times.
/// The verbs read such text back as the integer.
pub(crate) fn integers_as_text(value: &mut Value) {
    match value {
        Value::Number(n) => {
            let big = n.as_u64().is_some_and(|v| v > MAX_EXACT_INTEGER)
                || n.as_i64()
                    .is_some_and(|v| v.unsigned_abs() > MAX_EXACT_INTEGER);
            if big {
                *value = Value::String(n.to_string());
            }
        }
        Value::Array(items) => items.iter_mut().for_each(integers_as_text),
        Value::Object(map) => map.values_mut().for_each(integers_as_text),
        _ => {}
    }
}

/// A short description of an outcome for the run log: its keys, and how
/// a package replacement matched the terminals.
pub(crate) fn summary(outcome: &Value) -> String {
    match outcome {
        Value::Object(map) => {
            let keys = map.keys().take(4).cloned().collect::<Vec<_>>().join(", ");
            match map
                .get("plan")
                .and_then(|p| p["terminals"]["summary"].as_str())
            {
                Some(terminals) => format!("{keys}; {terminals}"),
                None => keys,
            }
        }
        Value::String(s) => s.clone(),
        Value::Null => "changed".into(),
        other => other.to_string().chars().take(80).collect(),
    }
}

/// The first request's message: datasheets, then the goal. `note` tells a
/// model what it got instead of the PDFs.
fn first_message(goal: &Goal, prepared: &Prepared) -> Message {
    let mut content = prepared.parts.clone();
    let mut text = format!(
        "Create the part \"{}\" from these datasheets. {}",
        goal.name, goal.notes
    );
    if let Some(note) = &prepared.note {
        text = format!("{note}\n\n{text}");
    }
    content.push(Part::text(text));
    Message {
        role: Role::User,
        content,
    }
}

/// The checks that can fail before anything is sent.
fn preflight(config: &RunConfig, prepared: &Prepared, prompt: u64) -> Result<()> {
    let profile = &config.profile;
    let caps = &profile.capabilities;
    if !caps.tool_calling {
        return Err(Error::Unsupported(format!(
            "Create from datasheet works through tools, and {} is declared without tool calling",
            profile.model
        )));
    }
    if prompt.saturating_add(MIN_OUTPUT_ROOM) > caps.context_tokens {
        return Err(Error::Unsupported(format!(
            "the datasheets and tools come to about {prompt} tokens ({} of them the datasheets), which does not fit the {} tokens declared for {} with room to answer. Give fewer or shorter datasheets, or a larger context",
            prepared.tokens, caps.context_tokens, profile.model
        )));
    }
    Ok(())
}

/// The system prompt of a run: the run profile's additions on the shipped
/// base prompt.
pub(crate) fn system_prompt(config: &RunConfig) -> Result<String> {
    let system = config.run_profile.system_prompt(&config.base_prompt);
    if system.trim().is_empty() {
        return Err(Error::Config(
            "the run has no system prompt: the shipped base prompt was not loaded".into(),
        ));
    }
    Ok(system)
}

/// The six verbs, less what the run profile leaves out.
pub(crate) fn tool_specs(run_profile: &RunProfile) -> Vec<ToolSpec> {
    let mut specs = verbs::tools();
    specs.retain(|t| !run_profile.omit_tools.contains(&t.name));
    specs
}

/// A prompt estimate without a count: `document_tokens`, the tools and the
/// system prompt.
pub(crate) fn first_estimate(system: &str, specs: &[ToolSpec], document_tokens: u64) -> u64 {
    let tool_chars = serde_json::to_string(specs).map_or(0, |t| t.len() as u64);
    document_tokens
        .saturating_add(crate::usage::estimated_tokens(
            tool_chars.saturating_add(system.len() as u64),
        ))
        .saturating_add(ESTIMATE_MARGIN)
}

/// The facts a run starts with: the model, its declarations and the
/// transport.
pub(crate) fn run_facts(config: &RunConfig, backend: &dyn ModelBackend) -> RunFacts {
    let profile = &config.profile;
    RunFacts {
        model: profile.model.clone(),
        provider: profile.provider.label().to_owned(),
        capabilities: profile.declared(),
        transport: backend.label(),
        run_profile: config.run_profile.name.clone(),
        ..RunFacts::default()
    }
}

/// Run the tool loop until the model submits its report, a cap is reached,
/// the run is cancelled or the backend fails. Drafts land in `session`;
/// nothing is saved or published here.
pub fn run(
    goal: &Goal,
    config: &RunConfig,
    backend: &mut dyn ModelBackend,
    session: &mut Session,
    events: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
) -> Result<Outcome> {
    let profile = &config.profile;
    profile.check()?;
    let harness = config.harness_for(None, None)?;
    let mut prepared = documents::prepare(&goal.datasheets, &profile.capabilities)?;
    let system = system_prompt(config)?;
    let specs = tool_specs(&config.run_profile);
    // Without a count: the documents, the tools and the system prompt.
    let estimate = first_estimate(&system, &specs, prepared.tokens);
    preflight(config, &prepared, estimate)?;
    let messages = vec![first_message(goal, &prepared)];
    session.ensure_generators();
    let lent: Box<dyn Documents> = Box::new(std::mem::take(&mut prepared.pages));
    let attached = session.attach_documents(Some(lent));
    let mut context = ToolContext {
        desk: &mut *session,
        grant: config.reach.grant(),
        turn: None,
        legacy: backend.legacy_tools(),
        facts: std::mem::take(&mut prepared.facts),
        finish: Finish::Report,
    };
    let mut tally = Tally::default();
    let mut facts = run_facts(config, backend);
    let started = Instant::now();
    let conversation = Conversation {
        harness: &harness,
        system: &system,
        specs: &specs,
        messages,
        first_estimate: estimate,
        document_tokens: prepared.tokens,
        max_requests: config.budget.max_requests,
    };
    let (stop, finished) = converse(
        &mut Driver {
            config,
            backend,
            context: &mut context,
            tally: &mut tally,
            facts: &mut facts,
            started,
            events,
            cancel,
        },
        conversation,
    );
    let report = match finished {
        Some(Finished::Report(report)) => Some(*report),
        _ => None,
    };
    facts.elapsed_seconds = started.elapsed().as_secs_f64();
    facts.documents = std::mem::take(&mut context.facts);
    drop(context);
    session.attach_documents(attached);
    report::finish_facts(&mut facts, &tally, &stop, session);
    Ok(Outcome {
        report,
        stop,
        facts,
    })
}

/// One conversation's opening: the system prompt, the tools, the first
/// messages, and what the loop estimates and stops by.
pub(crate) struct Conversation<'a> {
    pub(crate) harness: &'a crate::harness::HarnessProfile,
    pub(crate) system: &'a str,
    pub(crate) specs: &'a [ToolSpec],
    pub(crate) messages: Vec<Message>,
    /// The first prompt's size when the backend cannot count it.
    pub(crate) first_estimate: u64,
    /// The datasheets' tokens in the prompt, for the truncation warning.
    pub(crate) document_tokens: u64,
    /// The conversation stops once the tally reaches this many requests.
    pub(crate) max_requests: Option<u32>,
}

/// What drives conversations, shared by a workflow's model steps: the
/// run's model, budget and limits, its session through the verbs, and
/// what it has used so far.
pub(crate) struct Driver<'a, 'c> {
    pub(crate) config: &'a RunConfig,
    pub(crate) backend: &'a mut dyn ModelBackend,
    pub(crate) context: &'a mut ToolContext<'c>,
    pub(crate) tally: &'a mut Tally,
    pub(crate) facts: &'a mut RunFacts,
    /// When the run started: its time limit counts from here.
    pub(crate) started: Instant,
    pub(crate) events: &'a mut dyn FnMut(Event),
    pub(crate) cancel: &'a AtomicBool,
}

/// The tool loop of one conversation: requests until the finishing call is
/// accepted ([`Finish`]), a cap is reached, it is cancelled or the backend
/// fails. Usage adds to the driver's tally and facts.
pub(crate) fn converse(
    driver: &mut Driver<'_, '_>,
    conversation: Conversation<'_>,
) -> (Stop, Option<Finished>) {
    let config = driver.config;
    let profile = &config.profile;
    let budget = &config.budget;
    let Conversation {
        harness,
        system,
        specs,
        mut messages,
        first_estimate,
        document_tokens,
        max_requests,
    } = conversation;
    let request_budget = Budget {
        max_requests,
        ..*budget
    };
    let finish = driver.context.finish;
    let started = driver.started;
    let cancel = driver.cancel;
    let events = &mut *driver.events;
    let backend = &mut *driver.backend;
    let mut finished: Option<Finished> = None;
    let mut limit = profile.output.per_request;
    let mut nudges = 0;
    let mut cuts = 0;
    let mut last_prompt = 0u64;
    let mut appended = 0u64;
    let mut first = true;
    let fixed_chars = (system.len() + serde_json::to_string(specs).map_or(0, |t| t.len())) as u64;
    // Refused calls by tool and exact input since the last call that
    // succeeded, to notice a model repeating one.
    let mut refused: HashMap<(String, String), u32> = HashMap::new();
    let mut repeated: Option<String> = None;
    let stop = loop {
        let tally = &mut *driver.tally;
        let facts = &mut *driver.facts;
        let allowance = match (crate::request::Driver {
            backend,
            budget: &request_budget,
            started,
            tally,
            cancel,
            events,
        })
        .allowance()
        {
            Ok(allowance) => allowance,
            Err(stop) => break stop.into(),
        };
        if let (Some(left), Some(max)) = (allowance.output, budget.max_output_tokens)
            && left < u64::from(limit)
        {
            break Stop::Budget(format!("{max} output tokens"));
        }
        // Near the context's end, earlier tool replies give way to stubs
        // naming their calls, as in the chat (`chat::context::condense`):
        // the model reads a page again when it needs it.
        let mut condensed_now = false;
        if !first {
            // What the server counted last, and what came since; after
            // clearing, the characters left.
            let grown = last_prompt
                .saturating_add(crate::usage::estimated_tokens(appended))
                .saturating_add(ESTIMATE_MARGIN);
            let counted = |_: &[Message]| grown;
            if let Some(condensed) = harness.history_strategy().condense(
                &mut messages,
                // Of the whole context: a run's history only grows, so its
                // prompt cache holds until the context truly fills.
                &crate::chat::Policy {
                    window_tokens: u64::MAX,
                    ..harness.policy()
                },
                profile.capabilities.context_tokens,
                &counted,
            ) {
                let after = crate::usage::estimated_tokens(
                    messages
                        .iter()
                        .map(|m| appended_chars(&m.content))
                        .fold(fixed_chars, u64::saturating_add),
                );
                events(Event::Condensed(crate::chat::Condensed {
                    after_tokens: after,
                    ..condensed
                }));
                last_prompt = after;
                appended = 0;
                condensed_now = true;
            }
        }
        let mut request = ModelRequest {
            system,
            tools: specs,
            messages: &messages,
            max_output_tokens: limit,
            timeout: allowance.timeout_within(profile.request_timeout),
        };
        let prompt_tokens = if first {
            // Count exactly when the backend can; otherwise the estimate.
            match backend.count_prompt_tokens(&request) {
                Ok(Some(n)) => n,
                _ => first_estimate,
            }
        } else if condensed_now {
            last_prompt + ESTIMATE_MARGIN
        } else {
            last_prompt
                .saturating_add(crate::usage::estimated_tokens(appended))
                .saturating_add(ESTIMATE_MARGIN)
        };
        // The context holds prompt and output: never ask for more than fits.
        let room = profile
            .capabilities
            .context_tokens
            .saturating_sub(prompt_tokens);
        if room < MIN_OUTPUT_ROOM {
            break Stop::Budget(format!(
                "the conversation, about {prompt_tokens} tokens, leaves no room to answer in the {} tokens declared for {}",
                profile.capabilities.context_tokens, profile.model
            ));
        }
        request.fit_output(profile.capabilities.context_tokens, prompt_tokens);
        let requested = u64::from(request.max_output_tokens);
        let received = match (crate::request::Driver {
            backend,
            budget: &request_budget,
            started,
            tally,
            cancel,
            events,
        })
        .send(&request, prompt_tokens, true)
        {
            Ok(received) => received,
            Err(stop) => break stop.into(),
        };
        let n = received.n;
        let seconds = received.seconds;
        let reply = received.reply;
        first = false;
        facts.usage_estimated |= reply.usage_estimated;
        if let Some(warning) = check_prompt(&reply, n, last_prompt, document_tokens) {
            events(Event::Warning(warning.clone()));
            facts.warnings.push(warning);
        }
        last_prompt = reply.usage.prompt();
        let stop_label = reply.stop.label();
        let was_cut = reply.stop == StopReason::CutOff;
        // One retry with room to finish after a cut-off, then back to the
        // normal limit.
        limit = if was_cut {
            profile.output.after_cut.max(profile.output.per_request)
        } else {
            profile.output.per_request
        };
        facts.request_log.push(RequestFacts {
            n,
            stop_reason: stop_label.clone(),
            input_tokens: reply.usage.input_tokens,
            cache_write_tokens: reply.usage.cache_write_tokens,
            cache_read_tokens: reply.usage.cache_read_tokens,
            output_tokens: reply.usage.output_tokens,
            seconds,
            tools: Vec::new(),
        });
        if let Some(stop) = received.terminal {
            break stop.into();
        }
        if reply.stop == StopReason::Stopped {
            // Stopped between chunks: the incomplete reply is never used.
            break Stop::Cancelled;
        }
        for part in &reply.content {
            if let Part::Text { text, .. } = part {
                events(Event::Text(text.clone()));
            }
        }
        if let StopReason::Refusal { category } = &reply.stop {
            break Stop::Refused(category.clone());
        }
        // The history is append-only: the reply goes back as it came.
        appended = appended_chars(&reply.content);
        let content = reply.content;
        let calls: Vec<(String, String, Value, Option<String>)> = content
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall {
                    id,
                    name,
                    input,
                    malformed,
                    ..
                } => Some((id.clone(), name.clone(), input.clone(), malformed.clone())),
                _ => None,
            })
            .collect();
        messages.push(Message {
            role: Role::Assistant,
            content,
        });
        cuts = if was_cut { cuts + 1 } else { 0 };
        if cuts >= MAX_CUTS_IN_A_ROW {
            break Stop::CutOff(format!(
                "{cuts} replies in a row stopped at the output limit (last {} tokens) before finishing; a cut-off reply is never used as an answer. Raise the model's output limit",
                requested
            ));
        }
        if calls.is_empty() {
            if was_cut || nudges < MAX_NUDGES {
                nudges += 1;
                let text = if was_cut {
                    "Your reply was cut off at the output limit. Continue with smaller steps (pin rows in chunks of at most 40 with assist.pinout_edit)."
                } else {
                    match finish {
                        Finish::Report => {
                            "Continue with the tools; finish with query library.review_check."
                        }
                        Finish::StepResult => {
                            "Continue with the tools; finish with query assist.step_result."
                        }
                    }
                };
                appended += text.len() as u64;
                messages.push(Message {
                    role: Role::User,
                    content: vec![Part::text(text)],
                });
                continue;
            }
            break Stop::Ended;
        }
        let mut results = Vec::new();
        for (call, (id, name, mut input, malformed)) in (0u32..).zip(calls) {
            let logged = facts.request_log.last_mut();
            let row = |state| Event::Call {
                request: n,
                call,
                name: name.clone(),
                state,
            };
            if cancel.load(Ordering::Relaxed) {
                // Stopped between two calls: the rest never run.
                events(row(CallState::NotRun {
                    why: "stopped".into(),
                }));
                results.push(result(
                    id,
                    "Not run: the person stopped the run.",
                    true,
                    Vec::new(),
                ));
                continue;
            }
            if was_cut {
                // The call's input may be incomplete: never run it.
                if let Some(r) = logged {
                    r.tools.push(format!("{name}!"));
                }
                events(row(CallState::NotRun {
                    why: "cut off at the output limit".into(),
                }));
                results.push(result(
                    id,
                    "Cut off at the output limit before this call was complete; send it again in smaller pieces (pin rows in chunks of at most 40 with assist.pinout_edit; assist.pinout_preview and assist.pinout_apply take no document).",
                    true,
                    Vec::new(),
                ));
                continue;
            }
            harness.surface_view.apply(&name, &mut input);
            events(row(CallState::Running {
                input: input.clone(),
            }));
            let (label, outcome) =
                dispatch_tool_call(driver.context, &name, &input, malformed.as_deref(), harness);
            if let Some(r) = logged {
                let failed = matches!(outcome, ToolOutcome::Reply { error: true, .. });
                r.tools.push(if failed {
                    format!("{label}!")
                } else {
                    label.clone()
                });
            }
            match outcome {
                ToolOutcome::Report(r) => {
                    let text = "Report received.";
                    events(row(CallState::Done {
                        reply: text.into(),
                        step: None,
                    }));
                    finished = Some(Finished::Report(r));
                    results.push(result(id, text, false, Vec::new()));
                }
                ToolOutcome::Step(answer) => {
                    let text = "Answer accepted.";
                    events(row(CallState::Done {
                        reply: text.into(),
                        step: None,
                    }));
                    finished = Some(Finished::Step(answer));
                    results.push(result(id, text, false, Vec::new()));
                }
                ToolOutcome::Reply {
                    mut text,
                    error,
                    step,
                    images,
                } => {
                    if !error {
                        // Something may have changed (a preview refused, the
                        // document edited, the same preview fine): only
                        // refusals with nothing succeeding between count.
                        refused.clear();
                    } else {
                        let seen = refused
                            .entry((label.clone(), input.to_string()))
                            .or_insert(0);
                        *seen += 1;
                        if *seen > 1 {
                            text.push_str(&format!(
                                "\n\nThis exact call, with the same input, was refused {seen} times. Sending it again will not change the answer: change what the refusal names, or take another step."
                            ));
                        }
                        if *seen >= harness.recover.repeat_limit && repeated.is_none() {
                            repeated = Some(format!(
                                "the model sent the same refused {label} call {seen} times"
                            ));
                        }
                    }
                    if let Some(step) = &step {
                        facts.steps.push(step.clone());
                    }
                    events(row(if error {
                        CallState::Refused {
                            reply: text.clone(),
                        }
                    } else {
                        CallState::Done {
                            reply: text.clone(),
                            step,
                        }
                    }));
                    results.push(result(id, &text, error, images));
                }
            }
        }
        appended = appended.saturating_add(appended_chars(&results));
        messages.push(Message {
            role: Role::User,
            content: results,
        });
        if finished.is_some() {
            break match finish {
                Finish::Report => Stop::Submitted,
                Finish::StepResult => Stop::Answered,
            };
        }
        if let Some(why) = repeated.take() {
            break Stop::Repeated(why);
        }
    };
    (stop, finished)
}

fn result(id: String, text: &str, error: bool, images: Vec<Image>) -> Part {
    Part::ToolResult {
        call_id: id,
        text: text.to_owned(),
        error,
        images,
    }
}

/// A warning when the server's prompt count shows it may have dropped part
/// of the conversation: the history only grows, and some servers silently
/// truncate a prompt that exceeds the context they gave the model.
fn check_prompt(
    reply: &crate::model::ModelReply,
    n: u32,
    last_prompt: u64,
    document_tokens: u64,
) -> Option<String> {
    if reply.usage_estimated {
        return None;
    }
    let prompt = reply.usage.prompt();
    let shrank = n > 1 && prompt.saturating_add(prompt / 10) < last_prompt;
    let far_below = n == 1 && prompt.saturating_mul(2) < document_tokens;
    (shrank || far_below).then(|| {
        format!(
            "Request {n}: the server counted {prompt} prompt tokens, less than this conversation holds (last request {last_prompt}, datasheets about {document_tokens}). It may have dropped part of it to fit its context, and a result from a truncated prompt can't be trusted. Check the context size the server gives this model."
        )
    })
}

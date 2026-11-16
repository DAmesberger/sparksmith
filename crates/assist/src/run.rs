//! The tool loop of a run, on any backend: a goal, the six verbs, a budget,
//! events and a recording. Every tool call goes through
//! [`dispatch_tool_call`] into the verbs ([`crate::verbs`]), and so through
//! the front door and its gate as the built-in agent.
//!
//! The run ends when the review check (`query library.review_check`)
//! accepts the model's report: that report is the run's.
use crate::documents::{self, Prepared};
use crate::model::{
    Budget, Image, Message, ModelBackend, ModelProfile, ModelRequest, Part, Role, StopReason,
    appended_chars,
};
use crate::profile::{AssistLibrary, ReachProfile, RunProfile};
use crate::report::{self, RequestFacts, ReviewReport, RunFacts};
use crate::usage::Tally;
use crate::verbs::{self, Agent};
use crate::{Error, Event, Goal, Outcome, Result, Stop, legacy};
use serde_json::{Value, json};
use sparksmith_automation::session::assist::Documents;
use sparksmith_automation::front::NoOne;
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
/// Characters per token of what the conversation grows by, for the estimate.
const CHARS_PER_TOKEN: u64 = 3;
/// Added to the estimate of the next prompt.
const ESTIMATE_MARGIN: u64 = 4_000;

/// What a run may use, on which model.
#[derive(Clone, Debug)]
pub struct RunConfig {
    pub profile: ModelProfile,
    /// What the model is told and how the loop reacts ([`crate::profile`]).
    pub run_profile: RunProfile,
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
    /// and the user's profile `folder`, the default budget and policy. The
    /// profile's declarations are set on the model.
    pub fn new(
        shipped: &AssistLibrary,
        mut profile: ModelProfile,
        run_profile: Option<&str>,
        folder: Option<&std::path::Path>,
    ) -> Result<Self> {
        let run_profile = shipped.select(run_profile, &profile.model, folder)?;
        run_profile.declare(&mut profile);
        Ok(Self {
            profile,
            run_profile,
            base_prompt: shipped.system.clone(),
            budget: Budget::default(),
            reach: shipped.reach.clone(),
        })
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
}

/// What tools act on during a run.
struct ToolContext<'a> {
    session: &'a mut Session,
    grant: Grant,
    reach: &'a ReachProfile,
    /// The replies call the tools of before the verbs ([`legacy`]).
    legacy: bool,
    /// Reported in the review: which document's pages the model looked at.
    facts: Vec<documents::DocumentFacts>,
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
            legacy::translate(context.session, name, input)
        } else {
            None
        };
        match translated {
            Some(Ok((verb, arguments))) => (verb, arguments, name.to_owned()),
            Some(Err(why)) => {
                return (name.to_owned(), reply(json!({"refused": why}), true, None));
            }
            None => {
                let refused = agent(context, &mut NoOne).verb(name, input.clone());
                return (name.to_owned(), reply(refused["error"].clone(), true, None));
            }
        }
    };
    let before = context.session.engine().revision();
    let answer = agent(context, &mut NoOne).verb(verb, arguments.clone());
    let after = context.session.engine().revision();
    if answer["ok"] != true {
        return (label, reply(answer["error"].clone(), true, None));
    }
    let mut result = answer["result"].clone();
    let entry = arguments["name"].as_str().unwrap_or_default();
    if verb == verbs::QUERY && entry == "library.review_check" {
        return (label, review(&arguments, result));
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
    (label, reply(result, false, step))
}

fn agent<'a>(context: &'a mut ToolContext<'_>, asker: &'a mut NoOne) -> Agent<'a> {
    Agent {
        session: context.session,
        grant: &context.grant,
        reach: context.reach,
        window: None,
        turn: None,
        asker,
    }
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
fn summary(outcome: &Value) -> String {
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
    let budget = &config.budget;
    let mut prepared = documents::prepare(&goal.datasheets, &profile.capabilities)?;
    let run_profile = &config.run_profile;
    let system = run_profile.system_prompt(&config.base_prompt);
    if system.trim().is_empty() {
        return Err(Error::Config(
            "the run has no system prompt: the shipped base prompt was not loaded".into(),
        ));
    }
    let mut specs = verbs::tools();
    specs.retain(|t| !run_profile.omit_tools.contains(&t.name));
    let tool_chars = serde_json::to_string(&specs)?.len() as u64;
    // Without a count: the documents, the tools and the system prompt.
    let first_estimate = prepared
        .tokens
        .saturating_add((tool_chars + system.len() as u64) / CHARS_PER_TOKEN)
        .saturating_add(ESTIMATE_MARGIN);
    preflight(config, &prepared, first_estimate)?;
    let mut messages = vec![first_message(goal, &prepared)];
    session.ensure_generators();
    let lent: Box<dyn Documents> = Box::new(std::mem::take(&mut prepared.pages));
    let attached = session.attach_documents(Some(lent));
    let mut context = ToolContext {
        session,
        grant: config.reach.grant(),
        reach: &config.reach,
        legacy: backend.legacy_tools(),
        facts: std::mem::take(&mut prepared.facts),
    };
    let mut tally = Tally::default();
    let mut facts = RunFacts {
        model: profile.model.clone(),
        provider: profile.provider.label().to_owned(),
        capabilities: profile.declared(),
        transport: backend.label(),
        run_profile: run_profile.name.clone(),
        ..RunFacts::default()
    };
    let started = Instant::now();
    let mut report: Option<ReviewReport> = None;
    let mut limit = profile.output.per_request;
    let mut nudges = 0;
    let mut cuts = 0;
    let mut last_prompt = 0u64;
    let mut appended = 0u64;
    // Refused calls by tool and exact input since the last call that
    // succeeded, to notice a model repeating one.
    let mut refused: HashMap<(String, String), u32> = HashMap::new();
    let mut repeated: Option<String> = None;
    let stop = loop {
        if cancel.load(Ordering::Relaxed) {
            break Stop::Cancelled;
        }
        if tally.requests >= budget.max_requests {
            break Stop::Budget(format!("{} requests", budget.max_requests));
        }
        let left = budget.max_duration.saturating_sub(started.elapsed());
        if left.is_zero() {
            break Stop::Budget(format!(
                "{} seconds of run time",
                budget.max_duration.as_secs()
            ));
        }
        let output_left = budget
            .max_output_tokens
            .saturating_sub(tally.usage.output_tokens);
        if output_left < u64::from(limit) {
            break Stop::Budget(format!("{} output tokens", budget.max_output_tokens));
        }
        let mut request = ModelRequest {
            system: &system,
            tools: &specs,
            messages: &messages,
            max_output_tokens: limit,
            timeout: profile.request_timeout.min(left),
        };
        let prompt_tokens = if tally.requests == 0 {
            // Count exactly when the backend can; otherwise the estimate.
            match backend.count_prompt_tokens(&request) {
                Ok(Some(n)) => n,
                _ => first_estimate,
            }
        } else {
            last_prompt + appended / CHARS_PER_TOKEN + ESTIMATE_MARGIN
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
        request.max_output_tokens = u32::try_from(room.min(u64::from(limit))).unwrap_or(limit);
        let requested = u64::from(request.max_output_tokens);
        let used = tally
            .usage
            .prompt()
            .saturating_add(tally.usage.output_tokens);
        if used.saturating_add(prompt_tokens).saturating_add(requested) > budget.max_total_tokens {
            break Stop::Budget(format!(
                "{} tokens: {used} used, and the next request could take {}",
                budget.max_total_tokens,
                prompt_tokens + requested
            ));
        }
        let n = tally.requests + 1;
        events(Event::Request { n, prompt_tokens });
        let sent = Instant::now();
        let reply = match backend.send(&request) {
            Ok(r) => r,
            Err(e) => {
                let over = budget
                    .max_duration
                    .saturating_sub(started.elapsed())
                    .is_zero();
                break if over {
                    Stop::Budget(format!(
                        "{} seconds of run time, during request {n}",
                        budget.max_duration.as_secs()
                    ))
                } else {
                    Stop::Failed(e.to_string())
                };
            }
        };
        let seconds = sent.elapsed().as_secs_f64();
        tally.book(&reply.usage);
        facts.usage_estimated |= reply.usage_estimated;
        if let Some(warning) = check_prompt(&reply, n, last_prompt, prepared.tokens) {
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
        events(Event::Response {
            n,
            stop_reason: stop_label,
            output_tokens: reply.usage.output_tokens,
            seconds,
        });
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
                    "Continue with the tools; finish with query library.review_check."
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
        for (id, name, input, malformed) in calls {
            let logged = facts.request_log.last_mut();
            if was_cut {
                // The call's input may be incomplete: never run it.
                if let Some(r) = logged {
                    r.tools.push(format!("{name}!"));
                }
                events(Event::Tool {
                    name: name.clone(),
                    error: true,
                    step: None,
                });
                results.push(result(
                    id,
                    "Cut off at the output limit before this call was complete; send it again in smaller pieces (pin rows in chunks of at most 40 with assist.pinout_edit; assist.pinout_preview and assist.pinout_apply take no document).",
                    true,
                    Vec::new(),
                ));
                continue;
            }
            let (label, outcome) =
                dispatch_tool_call(&mut context, &name, &input, malformed.as_deref());
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
                    events(Event::Tool {
                        name: name.clone(),
                        error: false,
                        step: None,
                    });
                    report = Some(*r);
                    results.push(result(id, "Report received.", false, Vec::new()));
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
                        if *seen >= run_profile.harness.repeat_limit && repeated.is_none() {
                            repeated = Some(format!(
                                "the model sent the same refused {label} call {seen} times"
                            ));
                        }
                    }
                    if let Some(step) = &step {
                        facts.steps.push(step.clone());
                    }
                    events(Event::Tool {
                        name: name.clone(),
                        error,
                        step,
                    });
                    results.push(result(id, &text, error, images));
                }
            }
        }
        appended = appended.saturating_add(appended_chars(&results));
        messages.push(Message {
            role: Role::User,
            content: results,
        });
        if report.is_some() {
            break Stop::Submitted;
        }
        if let Some(why) = repeated.take() {
            break Stop::Repeated(why);
        }
    };
    facts.elapsed_seconds = started.elapsed().as_secs_f64();
    facts.documents = std::mem::take(&mut context.facts);
    let session = context.session;
    session.attach_documents(attached);
    report::finish_facts(&mut facts, &tally, &stop, session);
    Ok(Outcome {
        report,
        stop,
        facts,
    })
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

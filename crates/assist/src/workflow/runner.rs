//! The one generic runner (`docs/assistant-panel.md` §2.3, W1): it drives a
//! workflow's steps in order on a session, as the caller's principal held
//! to the workflow's reach, inside one turn so the run undoes as one step.
//!
//! - **input** asks the person ([`Person::answer`]) for the step's inputs
//!   the run was not given; a `files` input is attached to the session as
//!   its documents when its step completes, and pointers read it as the
//!   attached documents, `[{title, pages}]`, never as paths (so the run's
//!   reads stay out of the `files` module).
//! - **tool** is one call through the front door (`each` repeats it per
//!   item of a list), with its parameters' pointers filled in.
//! - **model** is one conversation on the model ([`crate::run::converse`])
//!   with the step's goal, its section of `method.md`, the inputs and the
//!   earlier steps' results; it ends when `assist.step_result` accepts the
//!   answer.
//! - **review** shows a card: `confirm` waits for Apply (then makes its
//!   call) or Decline; `notify` goes on unless the person changes the
//!   choice, which sends the run back to the step `change` names.
//! - **result** shows the result card and ends the run.
//!
//! A step's `check` names a later tool step whose refusal sends the run
//! back to the step with the refusal's text, at most `retries` times. A
//! tool step refuses when its call fails; one a check names also when its
//! reply says `"accepted": false` or lists a finding that blocks.
//!
//! The run's budget is the smaller of the workflow's and the configured
//! limits ([`RunConfig::budget`]), counted over all its model steps; a
//! model step's `requests` caps that step. The runner has no rights of its
//! own: every call, its own and the model's, is the caller's principal.
use super::control::{
    Action, Choice, ControlAnswer, ControlCard, Count, Field, Finding, Gauge, Item,
};
use super::event::{RunEnd, StepRow, WorkflowEvent};
use super::{
    Catalog, DEFAULT_RETRIES, HostCatalog, InputType, Loaded, Pointer, Step, StepKind,
    method_section,
};
use crate::documents;
use crate::model::record::WorkflowMark;
use crate::model::{Budget, Message, ModelBackend, Part, Role};
use crate::report::{self, RunFacts};
use crate::run::{self, Conversation, Driver, Finish, Finished, RunConfig, ToolContext};
use crate::usage::Tally;
use crate::verbs::Agent;
use crate::{Datasheet, Error, Event, Result, Stop};
use serde_json::{Map, Value, json};
use sparksmith_automation::front::Call;
use sparksmith_automation::session::assist::{Documents, OpenStep};
use sparksmith_automation::session::metadata;
use sparksmith_automation::{Grant, Session};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Characters of one earlier step's result a model step is given; longer
/// results are cut with a note (the model can read more with the verbs).
pub const MAX_EARLIER_CHARS: usize = run::MAX_RESULT_CHARS;
/// How often an input step asks again for values that do not pass.
const MAX_ASKS: u32 = 3;

/// The person a run reaches: the chat's cards, a headless host's answers.
pub trait Person {
    /// What happens in the run, cards included.
    fn event(&mut self, event: &WorkflowEvent);
    /// What the model loop reports (requests, tool calls, text).
    fn model_event(&mut self, _event: &Event) {}
    /// The person's answer to `cards` of `step`: an input step's values
    /// (or Decline, Stop), a confirm gate's Apply, Decline or Stop, a
    /// notify card's Change. `None`: no answer, which goes on past a
    /// notify card and ends the run at any other.
    fn answer(&mut self, step: &str, cards: &[ControlCard]) -> Option<ControlAnswer>;
}

/// A workflow run to start.
pub struct WorkflowRun<'a> {
    pub loaded: &'a Loaded,
    /// Inputs already known (an offer's), by name; the input steps ask for
    /// the rest. A `files` input is a list of file paths.
    pub inputs: Map<String, Value>,
    /// The chat's grant; the run's is this held to the workflow's reach.
    pub grant: Grant,
    /// The model, its run profile, the base prompt, and the configured
    /// limits as `budget`.
    pub config: &'a RunConfig,
}

/// How a run ended and what it made. What completed stays in the session
/// as one undo step under `label`.
#[derive(Clone, Debug)]
pub struct WorkflowOutcome {
    pub end: RunEnd,
    /// Each step's result, by id: a tool step's reply (a list with `each`),
    /// a model step's accepted answer, a gate's plan or choice, the result
    /// card.
    pub results: Map<String, Value>,
    /// The inputs as the run read them (files as attached documents).
    pub inputs: Map<String, Value>,
    /// The turn's label, which its undo step reads.
    pub label: String,
    /// Requests, tokens and the model's facts over every model step.
    pub facts: RunFacts,
}

/// The mark a recording of a run of `loaded` carries.
pub fn mark(loaded: &Loaded) -> WorkflowMark {
    WorkflowMark {
        id: loaded.workflow.id.clone(),
        version: loaded.workflow.version.clone(),
        digest: loaded.digest.clone(),
    }
}

/// The budget of a run: the smaller of the workflow's and `limits`.
pub fn budget(workflow: &super::Budget, limits: &Budget) -> Budget {
    Budget {
        max_requests: workflow.requests.min(limits.max_requests),
        max_output_tokens: workflow.output_tokens.min(limits.max_output_tokens),
        max_total_tokens: workflow
            .total_tokens
            .map_or(limits.max_total_tokens, |t| t.min(limits.max_total_tokens)),
        max_duration: Duration::from_secs(u64::from(workflow.minutes) * 60)
            .min(limits.max_duration),
    }
}

/// Run `run` on `session`: its model steps on `backend`, its cards and
/// events through `person`, until the result step, a decline, Stop, a
/// limit or a failure. Refused before anything runs: a workflow this host
/// cannot run, a given input it does not have or that does not pass, and a
/// replay recorded with another workflow or version.
pub fn run_workflow(
    run: WorkflowRun<'_>,
    backend: &mut dyn ModelBackend,
    session: &mut Session,
    person: &mut dyn Person,
    cancel: &AtomicBool,
) -> Result<WorkflowOutcome> {
    let loaded = run.loaded;
    let workflow = &loaded.workflow;
    let ours = mark(loaded);
    if let Some(recorded) = backend.recorded_workflow()
        && *recorded != ours
    {
        return Err(Error::Config(format!(
            "the recording is of workflow {} {} ({}), and this is {} {} ({}): a replay runs only against the workflow it was recorded with",
            recorded.id, recorded.version, recorded.digest, ours.id, ours.version, ours.digest
        )));
    }
    let why_not = workflow.availability(&HostCatalog);
    if !why_not.is_empty() {
        return Err(Error::Config(format!(
            "workflow {} cannot run here: {}",
            workflow.id,
            why_not.join("; ")
        )));
    }
    for (name, value) in &run.inputs {
        let input = workflow.input(name).ok_or_else(|| {
            Error::Config(format!("workflow {} has no input {name}", workflow.id))
        })?;
        input
            .check_value(value)
            .map_err(|why| Error::Config(format!("input {name}: {why}")))?;
    }
    let mut config = run.config.clone();
    config.budget = budget(&workflow.budget, &run.config.budget);
    config.profile.check()?;
    let system = run::system_prompt(&config)?;
    let specs = run::tool_specs(&config.run_profile);
    session.ensure_generators();
    let lent = session.attach_documents(None);
    let opened = session.open_step(None);
    let legacy = backend.legacy_tools();
    let mut facts = run::run_facts(&config, backend);
    let mut runner = Runner {
        loaded,
        config: &config,
        system,
        specs,
        context: ToolContext {
            session,
            grant: run.grant.narrowed(&workflow.reach),
            turn: None,
            legacy,
            facts: Vec::new(),
            finish: Finish::StepResult,
        },
        given: run.inputs,
        inputs: Map::new(),
        results: Map::new(),
        tally: Tally::default(),
        started: Instant::now(),
        run_id: format!("run_{}", ours.digest.get(7..19).unwrap_or("0")),
        label: workflow.title.clone(),
    };
    let end = runner.steps(backend, person, cancel, &mut facts);
    let closed = runner.close_turn();
    let Runner {
        context,
        tally,
        started,
        results,
        inputs,
        label,
        run_id,
        ..
    } = runner;
    let session = context.session;
    session.open_step(opened);
    session.attach_documents(lent);
    let end = match (end, closed) {
        (RunEnd::Done, Err(why)) => RunEnd::Failed {
            step: "turn".into(),
            why,
        },
        (end, _) => end,
    };
    person.event(&WorkflowEvent::Ended {
        run: run_id,
        end: end.clone(),
    });
    facts.elapsed_seconds = started.elapsed().as_secs_f64();
    facts.documents = context.facts;
    report::tally_facts(&mut facts, &tally);
    facts.stop = describe(&end);
    Ok(WorkflowOutcome {
        end,
        results,
        inputs,
        label,
        facts,
    })
}

fn describe(end: &RunEnd) -> String {
    match end {
        RunEnd::Done => "done".into(),
        RunEnd::Stopped => "stopped".into(),
        RunEnd::Declined { step } => format!("declined at {step}"),
        RunEnd::Limit { why } => format!("limit reached: {why}"),
        RunEnd::Failed { step, why } => format!("{step} failed: {why}"),
    }
}

/// A run in progress.
struct Runner<'a, 's> {
    loaded: &'a Loaded,
    config: &'a RunConfig,
    system: String,
    specs: Vec<crate::model::ToolSpec>,
    context: ToolContext<'s>,
    /// Inputs given and not yet asked for by their step.
    given: Map<String, Value>,
    /// Inputs asked for so far, as pointers read them.
    inputs: Map<String, Value>,
    results: Map<String, Value>,
    tally: Tally,
    started: Instant,
    run_id: String,
    label: String,
}

/// What one step did.
enum Next {
    /// Go on to the next step.
    On,
    /// Go back to the step at this index, with this text for it.
    Back(usize, String),
    /// The run ends.
    End(RunEnd),
}

impl Runner<'_, '_> {
    fn steps(
        &mut self,
        backend: &mut dyn ModelBackend,
        person: &mut dyn Person,
        cancel: &AtomicBool,
        facts: &mut RunFacts,
    ) -> RunEnd {
        let loaded = self.loaded;
        let steps = &loaded.workflow.steps;
        let total = u32::try_from(steps.len()).unwrap_or(u32::MAX);
        person.event(&WorkflowEvent::Started {
            run: self.run_id.clone(),
            workflow: self.loaded.workflow.id.clone(),
            version: self.loaded.workflow.version.clone(),
            digest: self.loaded.digest.clone(),
            label: self.label.clone(),
            steps: steps
                .iter()
                .map(|s| StepRow {
                    id: s.id.clone(),
                    title: s.title.clone(),
                })
                .collect(),
        });
        let mut attempts: HashMap<usize, u8> = HashMap::new();
        let mut notes: HashMap<usize, String> = HashMap::new();
        let mut k = 0;
        while let Some(step) = steps.get(k) {
            if cancel.load(Ordering::Relaxed) {
                return RunEnd::Stopped;
            }
            person.event(&WorkflowEvent::Step {
                run: self.run_id.clone(),
                step: step.id.clone(),
                done: u32::try_from(k).unwrap_or(u32::MAX),
                total,
            });
            if step.kind != StepKind::Input
                && let Err(why) = self.open_turn()
            {
                return RunEnd::Failed {
                    step: step.id.clone(),
                    why,
                };
            }
            let note = notes.remove(&k);
            let next = match step.kind {
                StepKind::Input => self.input(step, person),
                StepKind::Tool => self.tool(k, step),
                StepKind::Model => {
                    self.model(step, note.as_deref(), backend, person, cancel, facts)
                }
                StepKind::Review => self.review(k, step, person),
                StepKind::Result => self.result(step, person),
            };
            match next {
                Next::On => {
                    let summary = self
                        .results
                        .get(&step.id)
                        .map(run::summary)
                        .unwrap_or_default();
                    person.event(&WorkflowEvent::StepDone {
                        run: self.run_id.clone(),
                        step: step.id.clone(),
                        summary,
                    });
                    if step.kind == StepKind::Result {
                        return RunEnd::Done;
                    }
                    k += 1;
                }
                Next::Back(to, why) => {
                    let Some(target) = steps.get(to) else {
                        return RunEnd::Failed {
                            step: step.id.clone(),
                            why,
                        };
                    };
                    let of = target.retries.unwrap_or(DEFAULT_RETRIES);
                    let attempt = attempts.entry(to).or_insert(0);
                    if *attempt >= of {
                        return RunEnd::Failed {
                            step: step.id.clone(),
                            why: format!("still refused after {of} tries: {why}"),
                        };
                    }
                    *attempt += 1;
                    person.event(&WorkflowEvent::Retry {
                        run: self.run_id.clone(),
                        step: target.id.clone(),
                        attempt: *attempt,
                        of,
                        why: why.clone(),
                    });
                    notes.insert(to, why);
                    k = to;
                }
                Next::End(end) => return end,
            }
        }
        RunEnd::Failed {
            step: String::new(),
            why: "the workflow has no result step".into(),
        }
    }

    /// The turn the run's calls join, opened before its first step that
    /// can change something, under the workflow's title and its first
    /// text input ("Datasheet → component: TLV75733PDBVR").
    fn open_turn(&mut self) -> std::result::Result<(), String> {
        if self.context.turn.is_some() {
            return Ok(());
        }
        let loaded = self.loaded;
        let workflow = &loaded.workflow;
        let subject = workflow
            .steps
            .iter()
            .filter(|s| s.kind == StepKind::Input)
            .flat_map(|s| s.inputs.iter())
            .find(|name| {
                workflow
                    .input(name)
                    .is_some_and(|i| i.ty == InputType::Text)
            })
            .and_then(|name| self.inputs.get(name))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());
        if let Some(subject) = subject {
            self.label = format!("{}: {}", workflow.title, subject.trim());
        }
        let reply = self.call("project.turn_begin", json!({"label": self.label}))?;
        let turn = reply["turn"]
            .as_str()
            .ok_or_else(|| format!("project.turn_begin answered no turn: {reply}"))?;
        self.context.turn = Some(turn.to_owned());
        Ok(())
    }

    fn close_turn(&mut self) -> std::result::Result<(), String> {
        let Some(turn) = self.context.turn.take() else {
            return Ok(());
        };
        self.call("project.turn_end", json!({"turn": turn}))
            .map(|_| ())
    }

    /// One call through the front door as the run's principal, in its turn:
    /// the result, or the error as text.
    fn call(&mut self, entry: &str, params: Value) -> std::result::Result<Value, String> {
        let expected_revision = metadata::find(entry)
            .filter(|meta| meta.revision)
            .map(|_| self.context.session.engine().revision());
        let call = Call {
            entry: entry.to_owned(),
            params,
            project: None,
            expected_revision,
            dry_run: false,
            confirm: None,
            turn: None,
            job: false,
        };
        let reply = Agent {
            session: self.context.session,
            grant: &self.context.grant,
            turn: self.context.turn.as_deref(),
        }
        .run(call);
        if reply["ok"] == true {
            Ok(reply["result"].clone())
        } else {
            Err(reply["error"].to_string())
        }
    }

    /// What a pointer reads now; `None` when it reads nothing.
    fn read(&self, text: &str, item: Option<&Value>) -> Option<Value> {
        let (base, rest) = match Pointer::parse(text).ok()? {
            Pointer::Input { name, rest } => (self.inputs.get(name)?, rest),
            Pointer::Step { id, rest } => (self.results.get(id)?, rest),
            Pointer::Item { rest } => (item?, rest),
        };
        base.pointer(rest).cloned()
    }

    /// `value` with its pointers filled in; a pointer to nothing leaves its
    /// field out.
    fn fill(&self, value: &Value, item: Option<&Value>) -> Option<Value> {
        match value {
            Value::String(text) if text.starts_with('/') => self.read(text, item),
            Value::Object(map) if map.len() == 1 && map.contains_key("literal") => {
                map.get("literal").cloned()
            }
            Value::Object(map) => Some(Value::Object(
                map.iter()
                    .filter_map(|(k, v)| Some((k.clone(), self.fill(v, item)?)))
                    .collect(),
            )),
            Value::Array(list) => Some(Value::Array(
                list.iter().filter_map(|v| self.fill(v, item)).collect(),
            )),
            other => Some(other.clone()),
        }
    }

    fn input(&mut self, step: &Step, person: &mut dyn Person) -> Next {
        let loaded = self.loaded;
        let workflow = &loaded.workflow;
        let mut values: Map<String, Value> = Map::new();
        for name in &step.inputs {
            let given = self
                .given
                .remove(name)
                .or_else(|| workflow.input(name).and_then(|i| i.default.clone()));
            if let Some(value) = given {
                values.insert(name.clone(), value);
            }
        }
        let mut asks = 0;
        loop {
            let missing: Vec<&String> = step
                .inputs
                .iter()
                .filter(|name| {
                    !values.contains_key(*name) && workflow.input(name).is_some_and(|i| !i.optional)
                })
                .collect();
            if missing.is_empty() {
                break;
            }
            if asks >= MAX_ASKS {
                return Next::End(RunEnd::Failed {
                    step: step.id.clone(),
                    why: format!(
                        "no value for {}",
                        missing
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
            }
            asks += 1;
            let cards: Vec<ControlCard> = step
                .inputs
                .iter()
                .filter(|name| !values.contains_key(*name))
                .filter_map(|name| {
                    let input = workflow.input(name)?;
                    Some(ControlCard::for_input(name, input, None, Vec::new()))
                })
                .collect();
            for card in &cards {
                person.event(&WorkflowEvent::Card {
                    run: self.run_id.clone(),
                    step: step.id.clone(),
                    card: card.clone(),
                });
            }
            match person.answer(&step.id, &cards) {
                Some(ControlAnswer::Inputs { values: given }) => {
                    for (name, value) in given {
                        let fits = step.inputs.contains(&name)
                            && workflow
                                .input(&name)
                                .is_some_and(|i| i.check_value(&value).is_ok());
                        if fits {
                            values.insert(name, value);
                        }
                    }
                }
                Some(ControlAnswer::Stop) => return Next::End(RunEnd::Stopped),
                Some(ControlAnswer::Decline) => {
                    return Next::End(RunEnd::Declined {
                        step: step.id.clone(),
                    });
                }
                _ => {
                    return Next::End(RunEnd::Failed {
                        step: step.id.clone(),
                        why: format!(
                            "no one gave {}",
                            missing
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    });
                }
            }
        }
        let attach = step.inputs.iter().any(|name| {
            workflow
                .input(name)
                .is_some_and(|i| i.ty == InputType::Files)
        });
        self.inputs.extend(values);
        if attach && let Err(why) = self.attach() {
            return Next::End(RunEnd::Failed {
                step: step.id.clone(),
                why,
            });
        }
        self.results
            .insert(step.id.clone(), Value::Object(self.inputs.clone()));
        Next::On
    }

    /// Attach every `files` input given so far as the session's documents,
    /// their text read on this computer, and read each as its documents.
    fn attach(&mut self) -> std::result::Result<(), String> {
        let loaded = self.loaded;
        let workflow = &loaded.workflow;
        let names: Vec<String> = workflow
            .inputs
            .iter()
            .filter(|(name, i)| i.ty == InputType::Files && self.inputs.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect();
        let mut sheets = Vec::new();
        let mut owners = Vec::new();
        for name in &names {
            let paths = self
                .inputs
                .get(name)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for path in paths {
                let Some(text) = path.as_str() else {
                    return Err(format!("{name}: give file paths"));
                };
                sheets.push(Datasheet::parse(text));
                owners.push(name.clone());
            }
        }
        if sheets.is_empty() {
            return Ok(());
        }
        // Read on this computer whatever the model: assist.document_text
        // serves the text, by title.
        let mut capabilities = self.config.profile.capabilities.clone();
        capabilities.native_pdf = false;
        let mut prepared = documents::prepare(&sheets, &capabilities).map_err(|e| e.to_string())?;
        let mut attached: Map<String, Value> = Map::new();
        for (owner, fact) in owners.iter().zip(&prepared.facts) {
            let entry = attached
                .entry(owner.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(list) = entry {
                list.push(json!({"title": fact.title, "pages": fact.pages}));
            }
        }
        for name in names {
            let documents = attached.remove(&name).unwrap_or(Value::Array(Vec::new()));
            self.inputs.insert(name, documents);
        }
        self.context.facts = std::mem::take(&mut prepared.facts);
        let store: Box<dyn Documents> = Box::new(std::mem::take(&mut prepared.pages));
        self.context.session.attach_documents(Some(store));
        Ok(())
    }

    /// The text of a refusal by a tool step's reply, when it refuses. A
    /// query's reply is tagged (`{report, result}`): both levels count.
    fn refusal(reply: &Value) -> Option<String> {
        let refuses = |value: &Value| {
            value["accepted"] == false
                || value["findings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|f| f["blocks"] == true || f["blocking"] == true)
        };
        (refuses(reply) || reply.get("result").is_some_and(refuses)).then(|| reply.to_string())
    }

    /// The earlier step whose `check` is the step at `k`.
    fn checked_by(&self, k: usize) -> Option<usize> {
        let steps = &self.loaded.workflow.steps;
        let id = &steps.get(k)?.id;
        steps
            .iter()
            .take(k)
            .rposition(|s| s.check.as_deref() == Some(id.as_str()))
    }

    fn tool(&mut self, k: usize, step: &Step) -> Next {
        let Some(entry) = step.call.clone() else {
            return Next::End(RunEnd::Failed {
                step: step.id.clone(),
                why: "a tool step without a call".into(),
            });
        };
        let params = step.params.clone().unwrap_or_else(|| json!({}));
        let outcome = match &step.each {
            Some(each) => {
                let items = self
                    .read(each, None)
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let mut replies = Vec::new();
                let mut failed = None;
                for item in &items {
                    let filled = self.fill(&params, Some(item)).unwrap_or_else(|| json!({}));
                    match self.call(&entry, filled) {
                        Ok(reply) => replies.push(reply),
                        Err(why) => {
                            failed = Some(why);
                            break;
                        }
                    }
                }
                match failed {
                    Some(why) => Err(why),
                    None => Ok(Value::Array(replies)),
                }
            }
            None => {
                let filled = self.fill(&params, None).unwrap_or_else(|| json!({}));
                self.call(&entry, filled)
            }
        };
        // A reply that answers "not yet" refuses only where a check waits
        // for it; elsewhere it is the step's result.
        let refused = match &outcome {
            Ok(reply) => self.checked_by(k).and_then(|_| Self::refusal(reply)),
            Err(why) => Some(why.clone()),
        };
        if let Ok(reply) = outcome {
            self.results.insert(step.id.clone(), reply);
        }
        match refused {
            None => Next::On,
            Some(why) => match self.checked_by(k) {
                Some(back) => Next::Back(back, format!("{entry} refused it: {why}")),
                None => Next::End(RunEnd::Failed {
                    step: step.id.clone(),
                    why,
                }),
            },
        }
    }

    /// The first message of a model step.
    fn brief(&self, step: &Step, note: Option<&str>) -> String {
        let workflow = &self.loaded.workflow;
        let mut text = format!(
            "You are doing one step of the workflow \"{}\": {}.\n\nGoal: {}\n",
            workflow.title,
            step.title,
            step.goal.as_deref().unwrap_or_default()
        );
        if let Some(output) = step.output.as_ref().and_then(|o| workflow.outputs.get(o)) {
            text.push_str(&format!(
                "\nWhen you are done, call query assist.step_result with {{\"step\": \"{}\", \"result\": {{…}}}}: {}\nThe result's fields:\n",
                step.id, output.help
            ));
            for (name, field) in &output.fields {
                text.push_str(&format!(
                    "- {name} ({}{}){}{}\n",
                    field.ty.name(),
                    if field.optional { ", optional" } else { "" },
                    if field.help.is_empty() { "" } else { ": " },
                    field.help
                ));
            }
            if let Some(example) = &output.example {
                text.push_str(&format!("For example: {example}\n"));
            }
        }
        text.push_str("\nInputs:\n");
        for (name, value) in &self.inputs {
            let title = workflow
                .input(name)
                .map_or(name.as_str(), |i| i.title.as_str());
            text.push_str(&format!("- {title} ({name}): {value}\n"));
        }
        let earlier: Vec<&Step> = workflow
            .steps
            .iter()
            .take_while(|s| s.id != step.id)
            .filter(|s| matches!(s.kind, StepKind::Tool | StepKind::Model | StepKind::Review))
            .collect();
        if !earlier.is_empty() {
            text.push_str("\nWhat the earlier steps returned:\n");
        }
        for s in earlier {
            let Some(result) = self.results.get(&s.id) else {
                continue;
            };
            let mut shown = result.to_string();
            if shown.len() > MAX_EARLIER_CHARS {
                let mut end = MAX_EARLIER_CHARS;
                while !shown.is_char_boundary(end) {
                    end -= 1;
                }
                shown.truncate(end);
                shown.push_str("… [cut: read more with the tools]");
            }
            text.push_str(&format!("\n### {} ({})\n{shown}\n", s.id, s.title));
        }
        if let Some(note) = note {
            text.push_str(&format!(
                "\nYour last answer to this step was sent back: {note}\nFix what it names and answer again.\n"
            ));
        }
        let section = step
            .method
            .as_deref()
            .and_then(|m| m.split_once('#'))
            .and_then(|(_, anchor)| method_section(&self.loaded.method, anchor));
        if let Some(section) = section {
            text.push_str(&format!("\nThe method for this step:\n\n{section}"));
        }
        text
    }

    fn model(
        &mut self,
        step: &Step,
        note: Option<&str>,
        backend: &mut dyn ModelBackend,
        person: &mut dyn Person,
        cancel: &AtomicBool,
        facts: &mut RunFacts,
    ) -> Next {
        let loaded = self.loaded;
        let workflow = &loaded.workflow;
        let Some(output) = step.output.as_ref().and_then(|o| workflow.outputs.get(o)) else {
            return Next::End(RunEnd::Failed {
                step: step.id.clone(),
                why: "a model step without its output".into(),
            });
        };
        let brief = self.brief(step, note);
        let budget = &self.config.budget;
        let max_requests = match step.requests {
            Some(cap) => budget
                .max_requests
                .min(self.tally.requests.saturating_add(cap)),
            None => budget.max_requests,
        };
        self.context.session.open_step(Some(OpenStep {
            step: step.id.clone(),
            output: output.clone(),
        }));
        let estimate = run::first_estimate(&self.system, &self.specs, 0)
            .saturating_add((brief.len() as u64).div_ceil(3));
        let conversation = Conversation {
            system: &self.system,
            specs: &self.specs,
            messages: vec![Message {
                role: Role::User,
                content: vec![Part::text(brief)],
            }],
            first_estimate: estimate,
            document_tokens: 0,
            max_requests,
        };
        let mut events = |event: Event| person.model_event(&event);
        let (stop, finished) = run::converse(
            &mut Driver {
                config: self.config,
                backend,
                context: &mut self.context,
                tally: &mut self.tally,
                facts,
                started: self.started,
                events: &mut events,
                cancel,
            },
            conversation,
        );
        self.context.session.open_step(None);
        person.event(&self.meter());
        match (stop, finished) {
            (_, Some(Finished::Step(answer))) => {
                self.results.insert(step.id.clone(), answer);
                Next::On
            }
            (Stop::Cancelled, _) => Next::End(RunEnd::Stopped),
            (Stop::Budget(why), _) => Next::End(RunEnd::Limit { why }),
            (stop, _) => Next::End(RunEnd::Failed {
                step: step.id.clone(),
                why: stop.to_string(),
            }),
        }
    }

    /// Use against the run's limits.
    fn meter(&self) -> WorkflowEvent {
        let budget = &self.config.budget;
        WorkflowEvent::Meter {
            run: self.run_id.clone(),
            requests: Gauge {
                used: u64::from(self.tally.requests),
                limit: u64::from(budget.max_requests),
            },
            output_tokens: Gauge {
                used: self.tally.usage.output_tokens,
                limit: budget.max_output_tokens,
            },
            minutes: Gauge {
                used: self.started.elapsed().as_secs() / 60,
                limit: budget.max_duration.as_secs() / 60,
            },
        }
    }

    fn review(&mut self, k: usize, step: &Step, person: &mut dyn Person) -> Next {
        match step.gate {
            Some(super::Gate::Confirm) => self.confirm(k, step, person),
            _ => self.notify(step, person),
        }
    }

    fn notify(&mut self, step: &Step, person: &mut dyn Person) -> Next {
        let shown = step
            .show
            .as_deref()
            .and_then(|p| self.read(p, None))
            .unwrap_or(Value::Null);
        let mut options = Vec::new();
        if let Some(from) = &step.options {
            for item in self
                .read(&from.list, None)
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
            {
                let Some(value) = item.pointer(&from.value).and_then(Value::as_str) else {
                    continue;
                };
                let title = from
                    .title
                    .as_deref()
                    .and_then(|t| item.pointer(t))
                    .and_then(Value::as_str)
                    .unwrap_or(value);
                options.push(Choice {
                    value: value.to_owned(),
                    title: title.to_owned(),
                });
            }
        }
        let field = Field {
            input: step.id.clone(),
            title: step.title.clone(),
            help: String::new(),
            optional: false,
        };
        let card = ControlCard::Choice {
            field,
            multiple: false,
            options,
            value: shown.as_str().map(str::to_owned).into_iter().collect(),
        };
        person.event(&WorkflowEvent::Card {
            run: self.run_id.clone(),
            step: step.id.clone(),
            card: card.clone(),
        });
        self.results.insert(step.id.clone(), shown.clone());
        match person.answer(&step.id, &[card]) {
            Some(ControlAnswer::Stop) => Next::End(RunEnd::Stopped),
            Some(ControlAnswer::Change { value }) if value != shown => {
                let loaded = self.loaded;
                let steps = &loaded.workflow.steps;
                match step
                    .change
                    .as_deref()
                    .and_then(|id| steps.iter().position(|s| s.id == id))
                {
                    Some(back) => Next::Back(
                        back,
                        format!("the person chose {value} instead of {shown}; use their choice"),
                    ),
                    None => Next::On,
                }
            }
            _ => Next::On,
        }
    }

    fn confirm(&mut self, k: usize, step: &Step, person: &mut dyn Person) -> Next {
        let plan = step
            .plan
            .as_deref()
            .and_then(|p| self.read(p, None))
            .unwrap_or(Value::Null);
        let counts = match plan.get("change_counts").and_then(Value::as_object) {
            Some(map) => map
                .iter()
                .map(|(label, n)| Count {
                    label: label.clone(),
                    n: n.as_u64().unwrap_or(0),
                })
                .collect(),
            None => plan
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(label, v)| {
                    Some(Count {
                        label: label.clone(),
                        n: v.as_array()?.len() as u64,
                    })
                })
                .collect(),
        };
        let findings = plan["findings"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| Finding {
                blocking: f["blocks"] == true || f["blocking"] == true,
                message: f["message"].as_str().unwrap_or_default().to_owned(),
            })
            .collect();
        let card = ControlCard::Impact {
            title: step.title.clone(),
            counts,
            findings,
            rows: Vec::new(),
        };
        person.event(&WorkflowEvent::Card {
            run: self.run_id.clone(),
            step: step.id.clone(),
            card: card.clone(),
        });
        match person.answer(&step.id, &[card]) {
            Some(ControlAnswer::Apply { .. }) => {}
            Some(ControlAnswer::Stop) => return Next::End(RunEnd::Stopped),
            _ => {
                return Next::End(RunEnd::Declined {
                    step: step.id.clone(),
                });
            }
        }
        let Some(then) = &step.then else {
            return Next::On;
        };
        let params = self.fill(&then.params, None).unwrap_or_else(|| json!({}));
        match self.call(&then.call, params) {
            Ok(reply) => {
                self.results.insert(step.id.clone(), reply);
                Next::On
            }
            Err(why) => match self.checked_by(k) {
                Some(back) => Next::Back(back, format!("{} refused it: {why}", then.call)),
                None => Next::End(RunEnd::Failed {
                    step: step.id.clone(),
                    why,
                }),
            },
        }
    }

    fn result(&mut self, step: &Step, person: &mut dyn Person) -> Next {
        let items: Vec<Item> = step
            .items
            .iter()
            .map(|item| Item {
                title: item.title.clone(),
                value: self.read(&item.from, None).unwrap_or(Value::Null),
            })
            .collect();
        // A button whose entry this host lacks (view.open headless) is left
        // off the card.
        let actions: Vec<Action> = step
            .actions
            .iter()
            .filter(|a| HostCatalog.entry(&a.call).is_some())
            .map(|a| Action {
                title: a.title.clone(),
                call: a.call.clone(),
                params: self.fill(&a.params, None).unwrap_or_else(|| json!({})),
            })
            .collect();
        let card = ControlCard::Result {
            title: self.label.clone(),
            items: items.clone(),
            findings: Vec::new(),
            actions,
            undo: true,
        };
        self.results.insert(
            step.id.clone(),
            Value::Object(
                items
                    .into_iter()
                    .map(|item| (item.title, item.value))
                    .collect(),
            ),
        );
        person.event(&WorkflowEvent::Card {
            run: self.run_id.clone(),
            step: step.id.clone(),
            card,
        });
        Next::On
    }
}

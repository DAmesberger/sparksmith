//! The workflow runner on the shipped datasheet workflow, with a scripted
//! model (no network, no paid calls): the deterministic steps run on the
//! session, each model step is one answer through `assist.step_result`,
//! a check sends a step back with the refusal, the run is one undo step,
//! and a replay of another workflow version is refused.
use super::*;
use crate::model::record::{Replay, WorkflowMark};
use crate::model::{Budget, ModelReply, ModelRequest, Part, StopReason};
use crate::workflow::control::{ControlAnswer, ControlCard, Gauge};
use crate::workflow::event::{RunEnd, WorkflowEvent};
use crate::workflow::runner::{self, Person, WorkflowOutcome, WorkflowRun};
use crate::workflow::{Catalog, HostCatalog, Loaded, Source, load};
use std::path::PathBuf;

/// The shipped datasheet workflow, as the host lists it.
fn shipped() -> Fallible<Loaded> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../library/assist/workflows/datasheet-to-component");
    Ok(load::load_folder(&folder, Source::Shipped)?)
}

/// A person who answers from a script and keeps every event.
#[derive(Default)]
struct Scripted {
    answers: Vec<(String, ControlAnswer)>,
    events: Vec<WorkflowEvent>,
    asked: Vec<(String, Vec<ControlCard>)>,
}

impl Person for Scripted {
    fn event(&mut self, event: &WorkflowEvent) {
        self.events.push(event.clone());
    }

    fn answer(&mut self, step: &str, cards: &[ControlCard]) -> Option<ControlAnswer> {
        self.asked.push((step.to_owned(), cards.to_vec()));
        let at = self.answers.iter().position(|(s, _)| s == step)?;
        Some(self.answers.remove(at).1)
    }
}

/// A model's answer to step `step`.
fn answer(id: &str, step: &str, result: Value) -> Value {
    chat_call(
        id,
        "query",
        &json!({"name": "assist.step_result", "params": {"step": step, "result": result}})
            .to_string(),
    )
}

fn plan() -> Value {
    json!({"generator": "module", "params": plan_params(),
        "why": "EXR-22 module, drawing on page 9",
        "pages": "exr22.pdf: pinout 1, drawing 2, land pattern 2"})
}

fn report(values: Value) -> Value {
    json!({"report": {
        "part": "EXR-22", "summary": "Synthetic module.",
        "sources": [{"name": "DS", "title": "EXR-22 datasheet", "revision": "1"}],
        "values": values,
        "open_questions": ["Ground array optional?"]}})
}

fn good_values() -> Value {
    json!([{"item": "pitch", "value": "1.27 mm", "source": {"document": "DS", "page": 9, "location": "Figure 4"}, "confidence": "high"}])
}

/// The three judgement steps answered once each.
fn script() -> Vec<Value> {
    vec![
        answer("a1", "plan", plan()),
        answer("a2", "pinout", json!({"document": pinout()})),
        answer("a3", "report", report(good_values())),
    ]
}

fn inputs(dir: &tempfile::TempDir) -> Fallible<serde_json::Map<String, Value>> {
    let Datasheet::File(path) = text_datasheet(dir)? else {
        return Err("a file".into());
    };
    let mut inputs = serde_json::Map::new();
    inputs.insert("datasheets".into(), json!([path.to_string_lossy()]));
    inputs.insert("part".into(), json!("EXR-22"));
    Ok(inputs)
}

fn start(
    loaded: &Loaded,
    config: &RunConfig,
    inputs: serde_json::Map<String, Value>,
    backend: &mut dyn ModelBackend,
    session: &mut Session,
    person: &mut Scripted,
) -> Fallible<WorkflowOutcome> {
    let run = WorkflowRun {
        loaded,
        inputs,
        grant: config.reach.grant(),
        config,
    };
    Ok(runner::run_workflow(
        run,
        backend,
        session,
        person,
        &AtomicBool::new(false),
    )?)
}

fn apply_import() -> (String, ControlAnswer) {
    (
        "import".into(),
        ControlAnswer::Apply {
            leave_out: Vec::new(),
        },
    )
}

/// The published (not drafted) component's terminals: each name with its
/// functions' directions, in terminal order.
fn published_terminals(session: &Session) -> Fallible<Vec<(String, Vec<&'static str>)>> {
    use sparksmith_core::library::TerminalClass;
    let library = &session.engine().project().library;
    let [component] = library.components.as_slice() else {
        return Err(format!("one component, not {}", library.components.len()).into());
    };
    Ok(component
        .definition
        .terminals
        .iter()
        .map(|t| {
            let directions = match &t.class {
                TerminalClass::Electrical(f) => {
                    f.functions.iter().map(|f| f.direction.wire()).collect()
                }
                _ => Vec::new(),
            };
            (t.name.clone(), directions)
        })
        .collect())
}

#[test]
fn an_unreadable_step_harness_stops_before_the_workflow_changes_anything() -> Fallible {
    let mut loaded = shipped()?;
    let step = loaded
        .workflow
        .steps
        .iter_mut()
        .rfind(|step| step.kind == crate::workflow::StepKind::Model)
        .ok_or("model step")?;
    step.harness_profile = Some("missing-model-step-harness".into());
    let config = local_config(131_072)?;
    let mut backend = openai(Vec::new(), &config);
    let mut session = profiled_session();
    let before = session.engine().revision();
    let mut person = Scripted::default();
    let error = start(
        &loaded,
        &config,
        Default::default(),
        &mut backend,
        &mut session,
        &mut person,
    )
    .err()
    .ok_or("invalid harness accepted")?;
    assert!(
        error
            .to_string()
            .contains("missing-model-step-harness.json"),
        "{error}"
    );
    assert_eq!(session.engine().revision(), before);
    assert!(person.events.is_empty());
    assert!(person.asked.is_empty());
    Ok(())
}

#[test]
fn the_datasheet_workflow_runs_its_steps_and_undoes_as_one() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    let mut backend = openai(script(), &config);
    let mut session = profiled_session();
    let before = session.engine().revision();
    let mut person = Scripted {
        answers: vec![apply_import()],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:#?}", person.events);
    assert_eq!(outcome.label, "Datasheet → component: EXR-22");
    // Three model requests, one per judgement step.
    assert_eq!(outcome.facts.requests, 3);
    // The files input reads as the attached documents, never as paths.
    assert_eq!(
        outcome.inputs["datasheets"],
        json!([{"title": "exr22.pdf", "pages": 2}])
    );
    // The Read step gives each datasheet's outline, every page by its
    // first lines; the model reads whole pages when it needs them.
    let read = &outcome.results["read"];
    assert!(
        read[0]["outline"]
            .as_str()
            .is_some_and(|t| t.starts_with("1: EXR-22 PIN TABLE") && t.lines().count() == 2),
        "{read}"
    );
    // The runtime pointers read what the replies hold.
    let made = &outcome.results["make"];
    assert!(made.pointer("/outcome/report/root").is_some(), "{made}");
    let drafts = &outcome.results["drafts"];
    let first = drafts
        .pointer("/outcome/ids/0")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{drafts}"))?;
    let component = session
        .engine()
        .drafts()
        .items
        .iter()
        .find(|d| json!(d.id) == json!(first))
        .ok_or("the first draft")?;
    assert!(matches!(
        component.definition,
        sparksmith_core::library::Definition::Component(_)
    ));
    assert!(
        outcome.results["preview"].pointer("/result/base").is_some(),
        "{}",
        outcome.results["preview"]
    );
    assert_eq!(outcome.results["accept"]["accepted"], true);
    // Pins came in from the pinout document.
    let pins = crate::pins::rows(&session.snapshot());
    assert_eq!(pins.len(), 23, "{pins:?}");
    // E2E run 13: the part a person places before publishing the drafts
    // is the one `make` published, so its pins carry the datasheet's
    // names and kinds too, never bare numbers.
    let published = published_terminals(&session)?;
    assert_eq!(published.len(), 23, "{published:?}");
    assert_eq!(published[0], ("GND1".to_owned(), vec!["power_input"]));
    assert_eq!(published[1], ("VCC".to_owned(), vec!["power_input"]));
    assert_eq!(published[2], ("IO0".to_owned(), vec!["input", "output"]));
    assert_eq!(published[22], ("EPAD".to_owned(), vec!["power_input"]));
    // E2E run 13: the BOM had no MPN for it. The part asked for is the
    // component's published manufacturer Part, for the BOM once chosen.
    let parts: Vec<(&str, &str)> = session
        .engine()
        .project()
        .library
        .latest_parts()
        .into_iter()
        .map(|p| {
            (
                p.definition.manufacturer.as_str(),
                p.definition.mpn.as_str(),
            )
        })
        .collect();
    assert_eq!(parts, [("Exemplar Radio Works", "EXR-22")]);
    // The person saw the generator choice (it went on) and the impact card.
    let asked: Vec<&str> = person.asked.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(asked, ["pick", "import"]);
    // The impact card says what the import makes: a new part, by its pins
    // (the module plan names no preset, so no package).
    let ControlCard::Impact { title, .. } = &person.asked[1].1[0] else {
        return Err(format!("an impact card: {:?}", person.asked[1]).into());
    };
    assert_eq!(title, "New part: 23 pins");
    // The result card, without view.open on a host without a window.
    let card = person.events.iter().find_map(|e| match e {
        WorkflowEvent::Card {
            card: ControlCard::Result { items, actions, .. },
            ..
        } => Some((items.len(), actions.len())),
        _ => None,
    });
    let has_view = HostCatalog.module_version("view").is_some();
    assert_eq!(card, Some((4, usize::from(has_view))));
    // The result names the pages the run found and used.
    assert_eq!(
        outcome.results["done"]["Pages used"],
        "exr22.pdf: pinout 1, drawing 2, land pattern 2"
    );
    assert!(matches!(
        person.events.last(),
        Some(WorkflowEvent::Ended {
            end: RunEnd::Done,
            ..
        })
    ));
    // One undo step for the whole run, under the turn's label.
    assert_ne!(session.engine().revision(), before);
    let undo = session.dispatch(json!({"command": "project.undo", "params": {"expected_revision": session.engine().revision()}}));
    assert_eq!(undo["ok"], true, "{undo}");
    assert!(session.engine().drafts().items.is_empty());
    // The step is closed and the documents are taken back.
    let closed = session.dispatch(json!({"command": "assist.step_result",
        "params": {"step": "plan", "result": {}}}));
    assert_eq!(closed["error"]["code"], "assist.no_step", "{closed}");
    Ok(())
}

#[test]
fn a_check_sends_the_step_back_with_the_refusal() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    // The first report cites nothing: the review check refuses it and the
    // run goes back to the report step with why. A wrong answer to
    // assist.step_result is refused inside the step, which goes on.
    let mut replies = script();
    replies.pop();
    replies.push(answer("a3", "report", json!({"report": 3})));
    replies.push(answer("a4", "report", report(json!([]))));
    replies.push(answer("a5", "report", report(good_values())));
    let mut backend = openai(replies, &config);
    let mut session = profiled_session();
    let mut person = Scripted {
        answers: vec![apply_import()],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:#?}", person.events);
    let retries: Vec<(&str, u8)> = person
        .events
        .iter()
        .filter_map(|e| match e {
            WorkflowEvent::Retry { step, attempt, .. } => Some((step.as_str(), *attempt)),
            _ => None,
        })
        .collect();
    assert_eq!(retries, [("report", 1)]);
    // The refusal reached the model in the step's new brief.
    let posts = &backend.wire.posts;
    let last = posts.last().ok_or("a request")?;
    let brief = last.2["messages"].to_string();
    assert!(brief.contains("was sent back"), "{brief}");
    assert!(brief.contains("library.review_check refused it"), "{brief}");
    // With the answer it refused, so the step corrects it instead of
    // writing it again from nothing.
    assert!(brief.contains("Your last answer was:"), "{brief}");
    assert!(brief.contains("Ground array optional?"), "{brief}");
    // The malformed answer came back as a refusal naming the field.
    let refused = posts
        .iter()
        .flat_map(|p| p.2["messages"].as_array().cloned().unwrap_or_default())
        .any(|m| {
            m["role"] == "tool"
                && m["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("\"accepted\":false"))
        });
    assert!(refused);
    Ok(())
}

#[test]
fn declining_the_import_ends_the_run_and_keeps_what_was_made() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    let mut backend = openai(script(), &config);
    let mut session = profiled_session();
    let mut person = Scripted {
        answers: vec![("import".into(), ControlAnswer::Decline)],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(
        outcome.end,
        RunEnd::Declined {
            step: "import".into()
        }
    );
    assert_eq!(outcome.facts.requests, 2);
    // The part was made and stays, as one undo step.
    assert!(!session.engine().drafts().items.is_empty());
    let undo = session.dispatch(json!({"command": "project.undo", "params": {"expected_revision": session.engine().revision()}}));
    assert_eq!(undo["ok"], true, "{undo}");
    assert!(session.engine().drafts().items.is_empty());
    Ok(())
}

#[test]
fn a_missing_input_is_asked_for_and_a_bad_one_refused() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    let mut given = inputs(&dir)?;
    let part = given.remove("part").ok_or("part")?;
    let mut backend = openai(script(), &config);
    let mut session = profiled_session();
    let mut values = serde_json::Map::new();
    values.insert("part".into(), part);
    let mut person = Scripted {
        answers: vec![
            ("ask".into(), ControlAnswer::Inputs { values }),
            apply_import(),
        ],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        given.clone(),
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:#?}", person.events);
    // One confirmation of the step: what was given shows filled in, what
    // is missing is empty to be filled.
    let (step, cards) = person.asked.first().ok_or("asked")?;
    assert_eq!(step, "ask");
    let fields: Vec<(&str, bool)> = cards
        .iter()
        .filter_map(|c| match c {
            ControlCard::Text { field, value, .. } => Some((field.input.as_str(), value.is_some())),
            ControlCard::Datasheet { field, files, .. } => {
                Some((field.input.as_str(), !files.is_empty()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        fields,
        [("datasheets", true), ("part", false), ("notes", false)]
    );
    // An input the workflow does not have is refused before anything runs.
    given.insert("colour".into(), json!("red"));
    let mut session = profiled_session();
    let refused = start(
        &loaded,
        &config,
        given,
        &mut openai(Vec::new(), &config),
        &mut session,
        &mut Scripted::default(),
    );
    assert!(
        refused
            .err()
            .is_some_and(|e| e.to_string().contains("no input colour"))
    );
    Ok(())
}

#[test]
fn a_replay_of_another_workflow_version_is_refused() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    let mut replay = Replay::new(Vec::new());
    replay.workflow = Some(WorkflowMark {
        digest: "sha256:another".into(),
        ..runner::mark(&loaded)
    });
    let mut session = profiled_session();
    let before = session.engine().revision();
    let refused = start(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut replay,
        &mut session,
        &mut Scripted::default(),
    );
    let why = refused.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(
        why.contains("a replay runs only against the workflow it was recorded with"),
        "{why}"
    );
    assert_eq!(session.engine().revision(), before);
    Ok(())
}

#[test]
fn a_run_gets_the_smaller_budget() {
    use std::time::Duration;
    let workflow = crate::workflow::Budget {
        requests: 40,
        output_tokens: 200_000,
        total_tokens: None,
        minutes: 45,
    };
    let limits = Budget {
        max_requests: Some(30),
        max_output_tokens: Some(400_000),
        max_total_tokens: Some(4_000_000),
        max_duration: Some(Duration::from_secs(30 * 60)),
    };
    assert_eq!(
        runner::budget(Some(&workflow), &limits),
        Budget {
            max_output_tokens: Some(200_000),
            ..limits
        }
    );
    // Without the person's limits, the declared ones alone.
    assert_eq!(
        runner::budget(Some(&workflow), &Budget::default()),
        Budget {
            max_requests: Some(40),
            max_output_tokens: Some(200_000),
            max_total_tokens: None,
            max_duration: Some(Duration::from_secs(45 * 60)),
        }
    );
    // Without a declared budget, the person's alone; without both, none.
    assert_eq!(runner::budget(None, &limits), limits);
    assert_eq!(runner::budget(None, &Budget::default()), Budget::default());
}

/// A read that succeeds and changes nothing.
fn read_page(id: &str) -> Value {
    chat_call(
        id,
        "query",
        &json!({"name": "assist.document_text", "params": {"document": "exr22.pdf", "pages": "1"}})
            .to_string(),
    )
}

/// Requests and output tokens on the run's last meter, with their limits.
fn last_meter(person: &Scripted) -> Option<(Gauge, Gauge, Gauge)> {
    person.events.iter().rev().find_map(|e| match e {
        WorkflowEvent::Meter {
            requests,
            output_tokens,
            minutes,
            ..
        } => Some((*requests, *output_tokens, *minutes)),
        _ => None,
    })
}

/// The package step alone makes fifty requests: nothing limits a run no one
/// set limits on.
#[test]
fn a_run_without_limits_goes_on_until_it_is_done() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    assert_eq!(config.budget, Budget::default());
    let dir = tempfile::tempdir()?;
    let mut replies: Vec<Value> = (0..50).map(|n| read_page(&format!("r{n}"))).collect();
    replies.extend(script());
    let mut backend = openai(replies, &config);
    let mut session = profiled_session();
    let mut person = Scripted {
        answers: vec![apply_import()],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:#?}", person.events);
    assert_eq!(outcome.facts.requests, 53);
    let (requests, tokens, minutes) = last_meter(&person).ok_or("a meter")?;
    assert_eq!((requests.used, requests.limit), (53, None));
    assert_eq!((tokens.limit, minutes.limit), (None, None));
    Ok(())
}

/// The budget a workflow of the person's own declares stops its run, and
/// the person's limits hold it.
#[test]
fn a_declared_budget_stops_its_run_within_the_persons_limits() -> Fallible {
    let mut loaded = shipped()?;
    loaded.workflow.budget = Some(crate::workflow::Budget {
        requests: 5,
        output_tokens: 10_000_000,
        total_tokens: None,
        minutes: 45,
    });
    let dir = tempfile::tempdir()?;
    for (limit, stopped_at) in [(None, 5), (Some(3), 3), (Some(9), 5)] {
        let mut config = local_config(131_072)?;
        config.budget.max_requests = limit;
        let replies = (0..50).map(|n| read_page(&format!("r{n}"))).collect();
        let mut backend = openai(replies, &config);
        let mut person = Scripted::default();
        let outcome = start(
            &loaded,
            &config,
            inputs(&dir)?,
            &mut backend,
            &mut profiled_session(),
            &mut person,
        )?;
        assert_eq!(
            outcome.end,
            RunEnd::Limit {
                why: format!("{stopped_at} requests")
            }
        );
        assert_eq!(outcome.facts.requests, stopped_at);
        let (requests, ..) = last_meter(&person).ok_or("a meter")?;
        assert_eq!(requests.limit, Some(u64::from(stopped_at)));
    }
    Ok(())
}

/// Without limits, the same refused call sent again and again still ends
/// the run.
#[test]
fn a_refused_call_repeated_ends_a_run_without_limits() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    // Applying with nothing previewed is refused.
    let apply = |n: usize| {
        chat_call(
            &format!("a{n}"),
            "apply",
            r#"{"expected_revision": 0, "operations": [{"op": "assist.pinout_apply", "params": {"base": "12"}}]}"#,
        )
    };
    let mut backend = openai((0..50).map(apply).collect(), &config);
    let outcome = start(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut profiled_session(),
        &mut Scripted::default(),
    )?;
    let RunEnd::Failed { step, why } = &outcome.end else {
        return Err(format!("{:?}", outcome.end).into());
    };
    assert_eq!(step, "plan");
    assert!(why.contains("the same refused"), "{why}");
    assert_eq!(outcome.facts.requests, 3);
    Ok(())
}

/// A person who presses Stop while request `at` runs.
struct Stopping<'a> {
    at: u32,
    cancel: &'a AtomicBool,
}

impl Person for Stopping<'_> {
    fn event(&mut self, _: &WorkflowEvent) {}

    fn model_event(&mut self, event: &Event) {
        if matches!(event, Event::Request { n, .. } if *n == self.at) {
            self.cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn answer(&mut self, _: &str, _: &[ControlCard]) -> Option<ControlAnswer> {
        None
    }
}

/// A model that reads the same page for ever changes nothing and is refused
/// nothing: Stop ends its run.
#[test]
fn stop_ends_a_looping_run_without_limits() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    let replies = (0..100).map(|n| read_page(&format!("r{n}"))).collect();
    let mut backend = openai(replies, &config);
    let cancel = AtomicBool::new(false);
    let outcome = runner::run_workflow(
        WorkflowRun {
            loaded: &loaded,
            inputs: inputs(&dir)?,
            grant: config.reach.grant(),
            config: &config,
        },
        &mut backend,
        &mut profiled_session(),
        &mut Stopping {
            at: 61,
            cancel: &cancel,
        },
        &cancel,
    )?;
    assert_eq!(outcome.end, RunEnd::Stopped);
    assert_eq!(outcome.facts.requests, 61);
    Ok(())
}

/// The datasheet workflow reads with its own harness profile: a whole
/// datasheet's text in one reply reaches the model uncut.
#[test]
fn the_datasheet_workflow_reads_long_replies_whole() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(262_144)?;
    let profile = config.harness_for(loaded.workflow.harness_profile.as_deref(), None)?;
    assert_eq!(
        (profile.name.as_str(), profile.policy().reply_chars),
        ("datasheet", 120_000)
    );
    let dir = tempfile::tempdir()?;
    let mut replies = vec![chat_call(
        "r1",
        "query",
        &json!({"name": "assist.document_text", "params": {"document": "exr22.pdf", "pages": "1-30"}})
            .to_string(),
    )];
    replies.extend(script());
    let mut backend = openai(replies, &config);
    let mut person = Scripted {
        answers: vec![apply_import()],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        long_inputs(&dir)?,
        &mut backend,
        &mut profiled_session(),
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:#?}", person.events);
    let second = backend.wire.posts.get(1).ok_or("a second request")?;
    let sent = second.2["messages"].to_string();
    for n in 1..=30u32 {
        assert!(sent.contains(&format!("BODY p{n} l40 ")), "page {n}");
    }
    assert!(!sent.contains("_shortened"));
    Ok(())
}

#[test]
fn the_host_lists_its_workflows_through_assist_workflows() -> Fallible {
    let mut session = profiled_session();
    let reach = reach()?;
    let unlent = verb_as(
        &mut session,
        &reach,
        "query",
        query("assist.workflows", json!({})),
    );
    assert_eq!(unlent["error"]["code"], "assist.no_workflows", "{unlent}");
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library/assist/workflows");
    session.lend_workflows(Some(load::list(Some(&folder), None, &HostCatalog)));
    let listed = verb_as(
        &mut session,
        &reach,
        "query",
        query(
            "assist.workflows",
            json!({"text": "make a part from this datasheet"}),
        ),
    );
    assert_eq!(listed["ok"], true, "{listed}");
    let first = &listed["result"]["workflows"][0];
    assert_eq!(first["id"], "datasheet-to-component");
    assert_eq!(first["source"], "shipped");
    assert!(first.get("why_not").is_none(), "{first}");
    Ok(())
}

/// A 41-page datasheet (about 3 000 characters a page, a running header and
/// footer on every page, the pin table on page 3), as a long real one reads.
fn long_inputs(dir: &tempfile::TempDir) -> Fallible<serde_json::Map<String, Value>> {
    let pages: Vec<String> = (1..=41u32)
        .map(|n| {
            let heading = match n {
                3 => "6 Pin Configuration and Functions".to_owned(),
                31 => "11 Mechanical Packaging and Orderable Information".to_owned(),
                _ => format!("Section {n} Electrical Characteristics"),
            };
            let body: Vec<String> = (1..=40)
                .map(|k| format!("BODY p{n} l{k} typical value under the stated test conditions"))
                .collect();
            format!(
                "EXR-22 RADIO MODULE DATASHEET REV 3\n{heading}\n{}\nCopyright Example Radio Company",
                body.join("\n")
            )
        })
        .collect();
    let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
    let path = dir.path().join("exr22.pdf");
    std::fs::write(&path, crate::fixtures::text_pdf(&refs))?;
    let mut inputs = serde_json::Map::new();
    inputs.insert("datasheets".into(), json!([path.to_string_lossy()]));
    inputs.insert("part".into(), json!("EXR-22"));
    Ok(inputs)
}

#[test]
fn a_long_datasheet_reaches_the_model_as_an_outline_and_the_pages_it_reads() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    // The package step reads the pin page, then answers.
    let mut replies = vec![chat_call(
        "r1",
        "query",
        &json!({"name": "assist.document_text", "params": {"document": "exr22.pdf", "pages": "3"}})
            .to_string(),
    )];
    replies.extend(script());
    let mut backend = openai(replies, &config);
    let mut session = profiled_session();
    let mut person = Scripted {
        answers: vec![apply_import()],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        long_inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:#?}", person.events);
    // The Read step is a map of the pages: every page by its first lines,
    // without the running header and footer.
    let outline = outcome.results["read"][0]["outline"]
        .as_str()
        .ok_or_else(|| format!("{}", outcome.results["read"]))?;
    assert_eq!(outline.lines().count(), 41, "{outline}");
    assert!(
        outline.contains("3: 6 Pin Configuration and Functions"),
        "{outline}"
    );
    assert!(!outline.contains("REV 3") && !outline.contains("Copyright"));
    // No request carries a page the model did not ask for: only page 3's
    // body ever reaches it, and only after it asked.
    let posts = &backend.wire.posts;
    assert_eq!(posts.len(), 4);
    for (k, post) in posts.iter().enumerate() {
        let sent = post.2["messages"].to_string();
        for n in (1..=41u32).filter(|n| *n != 3) {
            assert!(
                !sent.contains(&format!("BODY p{n} l20 ")),
                "request {k}: page {n}"
            );
        }
        assert_eq!(sent.contains("BODY p3 l20 "), k == 1, "request {k}");
    }
    // A whole run is far inside the limit the person had set (350 000):
    // every request's prompt, re-sent, adds up to well under it.
    let sent: usize = posts.iter().map(|p| p.2.to_string().len()).sum();
    let tokens = crate::usage::estimated_tokens(sent as u64);
    assert!(tokens < 100_000, "about {tokens} tokens sent");
    Ok(())
}

/// Each request's prompt tokens and the sizes of its tool replies.
type Seen = Vec<(u64, Vec<usize>)>;

/// A model on a server that gives it `context` tokens, counting the request
/// as it is sent (system prompt, tools and messages) at 2.5 bytes a token,
/// denser than any prompt measured on a real server, and refusing a request
/// whose prompt leaves no room to answer.
struct Served {
    context: u64,
    replies: std::collections::VecDeque<Vec<Part>>,
    /// Each request's prompt tokens and its tool replies' sizes.
    seen: Seen,
}

impl ModelBackend for Served {
    fn label(&self) -> String {
        "served".into()
    }

    fn count_prompt_tokens(&mut self, _: &ModelRequest<'_>) -> Result<Option<u64>> {
        Ok(None)
    }

    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        let prompt = request.prompt_bytes()? as u64 * 2 / 5;
        let last = request.messages.last().map(|m| m.content.as_slice());
        self.seen.push((
            prompt,
            last.into_iter()
                .flatten()
                .filter_map(|part| match part {
                    Part::ToolResult { text, .. } => Some(text.len()),
                    _ => None,
                })
                .collect(),
        ));
        if prompt + crate::run::MIN_OUTPUT_ROOM > self.context {
            return Err(Error::Protocol(format!(
                "a prompt of {prompt} tokens leaves no room to answer in {}",
                self.context
            )));
        }
        Ok(ModelReply {
            model: "served".into(),
            content: self.replies.pop_front().unwrap_or_default(),
            stop: StopReason::ToolCalls,
            usage: crate::usage::Usage {
                input_tokens: prompt,
                output_tokens: 300,
                ..Default::default()
            },
            usage_estimated: false,
        })
    }
}

/// The Package step reads, round by round, each round's page ranges in
/// parallel, then the steps are answered.
fn run_reading(context: u64, harness: &str, rounds: &[&[&str]]) -> Fallible<(RunEnd, Seen)> {
    let query = |id: String, input: Value| Part::ToolCall {
        id,
        name: "query".into(),
        input,
        malformed: None,
        replay: None,
    };
    let mut replies = std::collections::VecDeque::new();
    for (n, round) in rounds.iter().enumerate() {
        replies.push_back(
            round
                .iter()
                .map(|pages| {
                    query(
                        format!("r{n}-{pages}"),
                        json!({"name": "assist.document_text",
                            "params": {"document": "exr22.pdf", "pages": pages}}),
                    )
                })
                .collect(),
        );
    }
    for (step, result) in crate::fixtures::exr22_answers() {
        replies.push_back(vec![query(
            step.into(),
            json!({"name": "assist.step_result", "params": {"step": step, "result": result}}),
        )]);
    }
    let mut loaded = shipped()?;
    loaded.workflow.harness_profile = Some(harness.into());
    let dir = tempfile::tempdir()?;
    let mut backend = Served {
        context,
        replies,
        seen: Vec::new(),
    };
    let mut person = Scripted {
        answers: vec![apply_import()],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &local_config(context)?,
        long_inputs(&dir)?,
        &mut backend,
        &mut profiled_session(),
        &mut person,
    )?;
    Ok((outcome.end, backend.seen))
}

/// The datasheet profile's replies scale with the context: one large read
/// and several, in one round and over rounds, leave room to answer.
#[test]
fn large_reads_leave_room_to_answer_in_every_context() -> Fallible {
    let one: &[&[&str]] = &[&["1-41"]];
    let several: &[&[&str]] = &[
        &["1-10", "11-20", "21-30", "31-41"],
        &["1-41", "1-5", "6-10", "11-15", "16-20", "21-25"],
        &["1", "2", "3", "4", "5", "6", "7", "8"],
        &["1-41"],
        &["1-3", "1-6", "1-9", "1-12", "1-15", "1-18"],
    ];
    for context in [32_768, 65_536, 131_072, 262_144] {
        for rounds in [one, several] {
            let (end, seen) = run_reading(context, "datasheet", rounds)?;
            assert_eq!(end, RunEnd::Done, "{context}: {seen:?}");
        }
    }
    // The workflow's own prompt fills most of 16 384 tokens: the run ends
    // before a request, or is done; no reply ever overruns it.
    for rounds in [one, several] {
        let (end, seen) = run_reading(16_384, "datasheet", rounds)?;
        assert!(
            matches!(&end, RunEnd::Done)
                || matches!(&end, RunEnd::Limit { why } if why.contains("leaves no room")),
            "{end:?}: {seen:?}"
        );
    }
    Ok(())
}

#[test]
fn a_run_stopped_by_its_limit_names_what_it_kept_with_undo() -> Fallible {
    let loaded = shipped()?;
    let mut config = local_config(131_072)?;
    // One request: the package step answers, the pinout step finds none.
    config.budget.max_requests = Some(1);
    let dir = tempfile::tempdir()?;
    let mut backend = openai(script(), &config);
    let mut session = profiled_session();
    let mut person = Scripted::default();
    let outcome = start(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert!(
        matches!(outcome.end, RunEnd::Limit { .. }),
        "{:?}",
        outcome.end
    );
    // The step it had answered is kept for the review.
    assert_eq!(
        outcome.facts.unfinished.answers.keys().collect::<Vec<_>>(),
        ["plan"]
    );
    // The drafts it made stay, named on an unfinished result card that
    // offers Undo run, before the run's end.
    assert!(!session.engine().drafts().items.is_empty());
    let card = person.events.iter().rev().find_map(|e| match e {
        WorkflowEvent::Card {
            card: ControlCard::Result {
                title, items, undo, ..
            },
            ..
        } => Some((title.clone(), items.clone(), *undo)),
        _ => None,
    });
    let (title, items, undo) = card.ok_or("no result card")?;
    assert_eq!(title, "Datasheet → component: EXR-22: unfinished");
    assert!(undo);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles.first(), Some(&"Unfinished"), "{titles:?}");
    assert!(titles.contains(&"Drafts"), "{titles:?}");
    let first = items
        .first()
        .map(|i| i.value.to_string())
        .unwrap_or_default();
    assert!(first.contains("Limit reached"), "{first}");
    assert!(matches!(
        person.events.last(),
        Some(WorkflowEvent::Ended {
            end: RunEnd::Limit { .. },
            ..
        })
    ));
    // Undo run removes them, as one step.
    let undo = session.dispatch(json!({"command": "project.undo", "params": {"expected_revision": session.engine().revision()}}));
    assert_eq!(undo["ok"], true, "{undo}");
    assert!(session.engine().drafts().items.is_empty());
    Ok(())
}

#[test]
fn near_the_context_end_earlier_pages_give_way_to_stubs() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(60_000)?;
    let dir = tempfile::tempdir()?;
    // The server counts 44 000 prompt tokens a request: past 70 % of the
    // 60 000 context from the second request on.
    let read = |id: &str, pages: &str| {
        let mut reply = chat_call(
            id,
            "query",
            &json!({"name": "assist.document_text", "params": {"document": "exr22.pdf", "pages": pages}})
                .to_string(),
        );
        reply["usage"]["prompt_tokens"] = json!(44_000);
        reply
    };
    let mut replies = vec![read("r1", "3"), read("r2", "10"), read("r3", "31")];
    replies.extend(script());
    let mut backend = openai(replies, &config);
    let mut session = profiled_session();
    let mut person = Scripted {
        answers: vec![apply_import()],
        ..Scripted::default()
    };
    let outcome = start(
        &loaded,
        &config,
        long_inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:#?}", person.events);
    // The package step's fourth request: the first page read is a stub
    // naming its call, the last two rounds stay whole.
    let fourth = backend.wire.posts.get(3).ok_or("a fourth request")?;
    let sent = fourth.2["messages"].to_string();
    assert!(sent.contains("[Cleared"), "{sent}");
    assert!(!sent.contains("BODY p3 l20 "), "page 3 was sent again");
    assert!(sent.contains("BODY p10 l20 ") && sent.contains("BODY p31 l20 "));
    Ok(())
}

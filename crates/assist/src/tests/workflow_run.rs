//! The workflow runner on the shipped datasheet workflow, with a scripted
//! model (no network, no paid calls): the deterministic steps run on the
//! session, each model step is one answer through `assist.step_result`,
//! a check sends a step back with the refusal, the run is one undo step,
//! and a replay of another workflow version is refused.
use super::*;
use crate::model::record::{Replay, WorkflowMark};
use crate::workflow::control::{ControlAnswer, ControlCard};
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
    json!({"generator": "module", "params": crate::legacy::module_params(&module_values()),
        "why": "EXR-22 module, drawing on page 9"})
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
    let read = &outcome.results["read"];
    assert!(
        read[0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("EXR-22 PIN TABLE")),
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
    // The person saw the generator choice (it went on) and the impact card.
    let asked: Vec<&str> = person.asked.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(asked, ["pick", "import"]);
    assert!(matches!(person.asked[1].1[0], ControlCard::Impact { .. }));
    // The result card, without view.open on a host without a window.
    let card = person.events.iter().find_map(|e| match e {
        WorkflowEvent::Card {
            card: ControlCard::Result { items, actions, .. },
            ..
        } => Some((items.len(), actions.len())),
        _ => None,
    });
    let has_view = HostCatalog.module_version("view").is_some();
    assert_eq!(card, Some((3, usize::from(has_view))));
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
    // The card asked only for what the run was not given.
    let (step, cards) = person.asked.first().ok_or("asked")?;
    assert_eq!(step, "ask");
    let fields: Vec<&str> = cards
        .iter()
        .filter_map(|c| match c {
            ControlCard::Text { field, .. } | ControlCard::PageRanges { field, .. } => {
                Some(field.input.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(fields, ["part", "pages", "notes"]);
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
    let workflow = crate::workflow::Budget {
        requests: 40,
        output_tokens: 200_000,
        total_tokens: None,
        minutes: 45,
    };
    let limits = crate::model::Budget {
        max_requests: 30,
        max_output_tokens: 400_000,
        max_total_tokens: 4_000_000,
        max_duration: std::time::Duration::from_secs(30 * 60),
    };
    let got = runner::budget(&workflow, &limits);
    assert_eq!(got.max_requests, 30);
    assert_eq!(got.max_output_tokens, 200_000);
    assert_eq!(got.max_total_tokens, 4_000_000);
    assert_eq!(got.max_duration.as_secs(), 30 * 60);
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

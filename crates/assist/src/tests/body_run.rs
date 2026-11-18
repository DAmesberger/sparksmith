//! The body workflow (`datasheet-to-body`) on an imported board with a
//! scripted model (no network, no paid calls): the part keeps its
//! footprint, every part of its package gets the body in one undo step,
//! a body that misses the pads is sent back and then ends the run with
//! nothing changed, and a recording replays to the same board.
use super::*;
use crate::model::record::{Recording, Replay};
use crate::workflow::control::ControlAnswer;
use crate::workflow::event::RunEnd;
use crate::workflow::runner::{self, Person, WorkflowRun};
use crate::workflow::{Loaded, Source, load};
use sparksmith_automation::session::body::fixture_engine;
use std::path::PathBuf;

fn shipped() -> Fallible<Loaded> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../library/assist/workflows/datasheet-to-body");
    Ok(load::load_folder(&folder, Source::Shipped)?)
}

/// A person who applies the fit.
struct Applies(Vec<String>);

impl Person for Applies {
    fn event(&mut self, _event: &crate::workflow::event::WorkflowEvent) {}

    fn answer(
        &mut self,
        step: &str,
        _cards: &[crate::workflow::control::ControlCard],
    ) -> Option<ControlAnswer> {
        self.0.push(step.to_owned());
        (step == "apply").then_some(ControlAnswer::Apply {
            leave_out: Vec::new(),
        })
    }
}

fn answer(id: &str, step: &str, result: Value) -> Value {
    chat_call(
        id,
        "query",
        &json!({"name": "assist.step_result", "params": {"step": step, "result": result}})
            .to_string(),
    )
}

fn plan(preset: &str) -> Value {
    json!({"generator": "chip", "preset": preset,
        "pages": "r0603.pdf: drawing 2", "why": "0603 chip resistor, drawing on page 2"})
}

fn report() -> Value {
    json!({"report": {"part": "R0603", "summary": "Chip resistor body.",
        "sources": [{"name": "DS", "title": "R0603 datasheet", "revision": "1"}],
        "values": [{"item": "body length", "value": "1.6 mm",
            "source": {"document": "DS", "page": 2, "location": "Dimensions"}, "confidence": "high"}],
        "open_questions": []}})
}

fn inputs(dir: &tempfile::TempDir) -> Fallible<serde_json::Map<String, Value>> {
    let path = dir.path().join("r0603.pdf");
    std::fs::write(
        &path,
        crate::fixtures::text_pdf(&[
            &"R0603 CHIP RESISTOR THICK FILM ".repeat(12),
            &"DIMENSIONS L 1.6 W 0.8 H 0.45 TERMINATION 0.3 ".repeat(12),
        ]),
    )?;
    let mut inputs = serde_json::Map::new();
    inputs.insert("datasheets".into(), json!([path.to_string_lossy()]));
    inputs.insert("part".into(), json!("R1"));
    Ok(inputs)
}

fn revision_of(session: &Session, designator: &str) -> Option<u32> {
    session
        .engine()
        .project()
        .instances
        .iter()
        .find(|i| i.designator == designator)
        .map(|i| i.component.revision.0)
}

fn run(
    loaded: &Loaded,
    config: &RunConfig,
    inputs: serde_json::Map<String, Value>,
    backend: &mut dyn ModelBackend,
    session: &mut Session,
    person: &mut Applies,
) -> Fallible<runner::WorkflowOutcome> {
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

#[test]
fn the_body_workflow_gives_every_part_of_the_package_its_body_and_replays() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    let recorded = dir.path().join("body.jsonl");
    let script = vec![
        answer("a1", "plan", plan("resistor-0603")),
        answer("a2", "report", report()),
    ];
    let mut backend = Recording::for_workflow(
        openai(script, &config),
        recorded.clone(),
        runner::mark(&loaded),
    )?;
    let mut session = Session::fixture(fixture_engine()?);
    let mut person = Applies(Vec::new());
    let outcome = run(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    assert_eq!(outcome.end, RunEnd::Done, "{:?}", outcome.results);
    assert_eq!(outcome.label, "Datasheet → 3D body: R1");
    assert_eq!(outcome.facts.requests, 2);
    assert_eq!(person.0, ["pick", "apply"]);
    assert_eq!(outcome.results["fit"]["accepted"], true);
    assert_eq!(outcome.results["accept"]["accepted"], true);
    assert_eq!(revision_of(&session, "R1"), Some(2));
    assert_eq!(revision_of(&session, "R2"), Some(2));
    assert_eq!(revision_of(&session, "U1"), Some(1));
    let review = outcome.review(&session);
    assert!(review.report.is_some());
    // One undo step for the whole run.
    let undo = session.dispatch(json!({"command": "project.undo",
        "params": {"expected_revision": session.engine().revision()}}));
    assert_eq!(undo["ok"], true, "{undo}");
    assert_eq!(revision_of(&session, "R1"), Some(1));
    assert_eq!(revision_of(&session, "R2"), Some(1));

    // The recording replays to the same board.
    let mut replay = Replay::from_file(&recorded)?;
    let mut again = Session::fixture(fixture_engine()?);
    let replayed = run(
        &loaded,
        &config,
        inputs(&tempfile::tempdir()?)?,
        &mut replay,
        &mut again,
        &mut Applies(Vec::new()),
    )?;
    assert_eq!(replayed.end, RunEnd::Done);
    assert_eq!(revision_of(&again, "R2"), Some(2));
    Ok(())
}

#[test]
fn a_body_that_misses_the_pads_goes_back_and_then_ends_with_nothing_changed() -> Fallible {
    let loaded = shipped()?;
    let config = local_config(131_072)?;
    let dir = tempfile::tempdir()?;
    // The plan is refused by the fit each time (twice sent back).
    let script = vec![
        answer("a1", "plan", plan("resistor-2512")),
        answer("a2", "plan", plan("resistor-2512")),
        answer("a3", "plan", plan("resistor-2512")),
    ];
    let mut backend = openai(script, &config);
    let mut session = Session::fixture(fixture_engine()?);
    let mut person = Applies(Vec::new());
    let outcome = run(
        &loaded,
        &config,
        inputs(&dir)?,
        &mut backend,
        &mut session,
        &mut person,
    )?;
    let RunEnd::Failed { step, why } = &outcome.end else {
        return Err(format!("a failed run: {:?}", outcome.end).into());
    };
    assert_eq!(step, "fit");
    assert!(why.contains("body.lead_off_land"), "{why}");
    assert!(!person.0.contains(&"apply".to_owned()));
    assert_eq!(revision_of(&session, "R1"), Some(1));
    // Nothing was made: every package is still its first revision.
    let library = &session.engine().project().library;
    assert!(library.packages.iter().all(|p| p.revision.0 == 1));
    Ok(())
}

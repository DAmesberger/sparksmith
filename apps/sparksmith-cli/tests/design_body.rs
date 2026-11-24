//! `design-body` keeps what an early end had worked out: a body that never
//! fits leaves the project, saved as it was, and its review with the
//! model's last answer; a run that asked the model nothing writes nothing.
use serde_json::{Value, json};
use sparksmith_assist::fixtures;
use sparksmith_assist::model::{ModelReply, Part, StopReason};
use sparksmith_assist::usage::Usage;
use sparksmith_assist::workflow::{Source, load, runner};
use sparksmith_automation::Session;
use sparksmith_automation::session::body::fixture_engine;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// A board with R1, and a recording whose body never fits it.
fn prepare(dir: &Path) -> Fallible {
    let mut session = Session::fixture(fixture_engine()?);
    let saved = session.dispatch(json!({"command": "project.save",
        "params": {"path": dir.join("board.sparksmith").to_string_lossy()}}));
    assert_eq!(saved["ok"], true, "{saved}");
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../library/assist/workflows/datasheet-to-body");
    let loaded = load::load_folder(&folder, Source::Shipped)?;
    let mut lines = vec![json!({"workflow": runner::mark(&loaded)}).to_string()];
    for k in 0..3 {
        lines.push(serde_json::to_string(&ModelReply {
            model: "recorded".into(),
            content: vec![Part::ToolCall {
                id: format!("a{k}"),
                name: "query".into(),
                input: json!({"name": "assist.step_result", "params": {"step": "plan",
                    "result": {"generator": "chip", "preset": "resistor-2512",
                        "pages": "r.pdf: drawing 2", "why": "2512 chip resistor"}}}),
                malformed: None,
                replay: None,
            }],
            stop: StopReason::ToolCalls,
            usage: Usage::default(),
            usage_estimated: false,
        })?);
    }
    std::fs::write(dir.join("run.jsonl"), lines.join("\n") + "\n")?;
    Ok(())
}

fn design_body(dir: &Path, datasheet: &Path) -> Fallible<Output> {
    let config = dir.join("config");
    std::fs::create_dir_all(&config)?;
    Ok(Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", &config)
        .env("SPARKSMITH_CACHE_DIR", dir.join("cache"))
        .args(["design-body", "--designator", "R1", "--project"])
        .arg(dir.join("board.sparksmith"))
        .arg("--datasheet")
        .arg(datasheet)
        .arg("--out")
        .arg(dir.join("out.sparksmith"))
        .arg("--replay")
        .arg(dir.join("run.jsonl"))
        .output()?)
}

#[test]
fn a_body_that_never_fits_keeps_the_models_answer() -> Fallible {
    let dir = tempfile::tempdir()?;
    prepare(dir.path())?;
    let sheet = dir.path().join("r.pdf");
    fixtures::exr22_datasheet(&sheet)?;
    let ran = design_body(dir.path(), &sheet)?;
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(!ran.status.success(), "{said}");
    assert!(said.contains("no body was made"), "{said}");
    assert!(said.contains("saved to"), "{said}");
    assert!(dir.path().join("out.sparksmith").exists());
    let review: Value = serde_json::from_str(&std::fs::read_to_string(
        dir.path().join("out.review.json"),
    )?)?;
    assert_eq!(
        review["run"]["unfinished"]["answers"]["plan"]["preset"], "resistor-2512",
        "{review:#}"
    );
    Ok(())
}

#[test]
fn a_run_that_asked_the_model_nothing_writes_nothing() -> Fallible {
    let dir = tempfile::tempdir()?;
    prepare(dir.path())?;
    let ran = design_body(dir.path(), &dir.path().join("missing.pdf"))?;
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(!ran.status.success(), "{said}");
    assert!(said.contains("no body was made"), "{said}");
    assert!(!dir.path().join("out.sparksmith").exists());
    Ok(())
}

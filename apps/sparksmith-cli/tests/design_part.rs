//! `design-part` runs the datasheet workflow headless through the runner:
//! a recording of a workflow run (three answers through
//! `assist.step_result`, no network) replays to a new project of drafts with
//! its review beside it; a recording of another workflow version is
//! refused before anything runs.
use serde_json::{Value, json};
use sparksmith_assist::fixtures;
use sparksmith_assist::model::{ModelReply, Part, StopReason};
use sparksmith_assist::usage::Usage;
use sparksmith_assist::workflow::{Source, load, runner};
use std::path::{Path, PathBuf};
use std::process::Command;

type Fallible = Result<(), Box<dyn std::error::Error>>;

fn workflow() -> Result<sparksmith_assist::workflow::Loaded, String> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../library/assist/workflows/datasheet-to-component");
    load::load_folder(&folder, Source::Shipped)
}

/// A recording of a run of the workflow marked `mark`: one answer per
/// judgement step.
fn recording(path: &Path, mark: Value) -> Fallible {
    let mut lines = vec![json!({"workflow": mark}).to_string()];
    for (k, (step, result)) in fixtures::exr22_answers().into_iter().enumerate() {
        let reply = ModelReply {
            model: "recorded".into(),
            content: vec![Part::ToolCall {
                id: format!("a{k}"),
                name: "query".into(),
                input: json!({"name": "assist.step_result", "params": {"step": step, "result": result}}),
                malformed: None,
                replay: None,
            }],
            stop: StopReason::ToolCalls,
            usage: Usage {
                input_tokens: 2_000,
                output_tokens: 300,
                ..Usage::default()
            },
            usage_estimated: false,
        };
        lines.push(serde_json::to_string(&reply)?);
    }
    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

fn design_part(dir: &Path, replay: &Path, out: &Path) -> std::io::Result<std::process::Output> {
    let sheet = dir.join("exr22.pdf");
    fixtures::exr22_datasheet(&sheet)?;
    let config = dir.join("config");
    std::fs::create_dir_all(&config)?;
    Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", &config)
        .env("SPARKSMITH_CACHE_DIR", dir.join("cache"))
        .args(["design-part", "--datasheet"])
        .arg(&sheet)
        .args(["--name", "EXR-22", "--out"])
        .arg(out)
        .arg("--replay")
        .arg(replay)
        .output()
}

#[test]
fn a_workflow_recording_replays_to_a_project_and_its_review() -> Fallible {
    let dir = tempfile::tempdir()?;
    let loaded = workflow()?;
    let replay = dir.path().join("run.jsonl");
    recording(&replay, serde_json::to_value(runner::mark(&loaded))?)?;
    let out = dir.path().join("exr22.sparksmith");
    let ran = design_part(dir.path(), &replay, &out)?;
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(ran.status.success(), "{said}");
    // The runner's steps, in the workflow's order.
    assert!(said.contains("Datasheet → component"), "{said}");
    assert!(said.contains("step 1/"), "{said}");
    assert!(out.exists());
    let review: Value = serde_json::from_str(&std::fs::read_to_string(
        dir.path().join("exr22.review.json"),
    )?)?;
    assert_eq!(review["report"]["part"], "EXR-22");
    assert_eq!(review["run"]["requests"], 3);
    assert_eq!(review["run"]["stop"], "review report submitted");
    // Pins came in through the pinout import the run applied.
    assert_eq!(
        review["run"]["pins"].as_array().map(Vec::len),
        Some(23),
        "{review:#}"
    );
    let drafts = review["run"]["drafts"].as_array().map_or(0, Vec::len);
    assert!(drafts >= 4, "{review:#}");
    assert!(dir.path().join("exr22.review.md").exists());
    Ok(())
}

/// The local model's TLV75733 conversation (a recording that names no
/// workflow) replays as it was recorded, to the same LDO: pin 1 IN, 2 GND,
/// 3 EN, 4 NC, 5 OUT (TLV757P datasheet Table 4-1). A stand-in PDF: the
/// recorded page views then answer with errors, which the run never needed.
#[test]
fn a_reference_conversation_replays_to_the_same_part() -> Fallible {
    let dir = tempfile::tempdir()?;
    let sheet = dir.path().join("tlv757p.pdf");
    fixtures::exr22_datasheet(&sheet)?;
    let replay = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/tlv75733-local/tlv757.recording.jsonl");
    let out = dir.path().join("tlv75733.sparksmith");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config)?;
    let ran = Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", &config)
        .env("SPARKSMITH_CACHE_DIR", dir.path().join("cache"))
        .args([
            "design-part",
            "--provider",
            "openai",
            "--base-url",
            "http://localhost:11434/v1",
            "--model",
            "qwen3.8:27b",
            "--context-tokens",
            "262144",
            "--vision",
            "yes",
            "--name",
            "TLV75733PDBVR",
            "--datasheet",
        ])
        .arg(&sheet)
        .arg("--out")
        .arg(&out)
        .arg("--replay")
        .arg(&replay)
        .output()?;
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(ran.status.success(), "{said}");
    let review: Value = serde_json::from_str(&std::fs::read_to_string(
        dir.path().join("tlv75733.review.json"),
    )?)?;
    let pins: Vec<(String, String)> = review["run"]["pins"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            (
                p["number"].as_str().unwrap_or_default().to_owned(),
                p["name"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = [
        ("1", "IN"),
        ("2", "GND"),
        ("3", "EN"),
        ("4", "NC"),
        ("5", "OUT"),
    ]
    .iter()
    .map(|(n, s)| ((*n).to_owned(), (*s).to_owned()))
    .collect();
    assert_eq!(pins, expected, "{review:#}");
    assert_eq!(review["run"]["requests"], 29);
    Ok(())
}

#[test]
fn a_recording_of_another_workflow_version_is_refused() -> Fallible {
    let dir = tempfile::tempdir()?;
    let loaded = workflow()?;
    let mut mark = serde_json::to_value(runner::mark(&loaded))?;
    mark["version"] = json!("0.9");
    let replay = dir.path().join("old.jsonl");
    recording(&replay, mark)?;
    let out = dir.path().join("x.sparksmith");
    let ran = design_part(dir.path(), &replay, &out)?;
    let said = String::from_utf8_lossy(&ran.stderr);
    assert!(!ran.status.success());
    assert!(
        said.contains("a replay runs only against the workflow it was recorded with"),
        "{said}"
    );
    assert!(!out.exists());
    Ok(())
}

//! Developer-only evaluation of the six verbs against original generic fixtures.
mod batch;
mod external;
mod lines;
mod live;
mod run;
mod spend;
mod trace;
mod verify;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparksmith_assist::{
    board_script::{self, Scripted},
    chat,
    model::{ModelBackend, ModelProfile, record::WorkflowMark},
};
use sparksmith_automation::{Mode, Session, session::SchemaDetail};
use std::{
    fs,
    path::{Path, PathBuf},
};
use trace::{Metrics, Row};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    version: String,
    mode: Mode,
    reach: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ceilings {
    model_request_bytes: usize,
    estimated_prompt_tokens: u64,
    visible_reply_bytes: usize,
    /// The ECO exposes its save path in one tool reply and the following
    /// model request. Compare equal paths by accounting for JSON escaping
    /// at each layer. All reported run metrics keep the actual path bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_path: Option<PathSize>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PathSize {
    visible: usize,
    prompt: usize,
}

impl PathSize {
    fn of(path: &Path) -> Result<Self> {
        let string = serde_json::to_string(&path.to_string_lossy())?;
        Ok(Self {
            visible: string.len(),
            prompt: serde_json::to_string(&string)?.len(),
        })
    }
}

impl Ceilings {
    fn check(&self, m: &Metrics, path: &Path) -> Result<()> {
        let (mut visible, mut prompt, mut estimated) = (
            self.visible_reply_bytes,
            self.model_request_bytes,
            self.estimated_prompt_tokens,
        );
        if let Some(baseline) = &self.saved_path {
            let actual = PathSize::of(path)?;
            visible = visible
                .saturating_sub(baseline.visible)
                .saturating_add(actual.visible);
            prompt = prompt
                .saturating_sub(baseline.prompt)
                .saturating_add(actual.prompt);
            // The estimator rounds the entire final request. A directory
            // change can move its byte count across a three-byte boundary.
            if actual.visible >= baseline.visible {
                estimated += sparksmith_assist::usage::estimated_tokens(
                    (actual.visible - baseline.visible) as u64,
                );
            } else {
                let reduction = sparksmith_assist::usage::estimated_tokens(
                    (baseline.visible - actual.visible) as u64,
                );
                estimated = estimated.saturating_sub(reduction.saturating_sub(1));
            }
        }
        for (name, measured, limit) in [
            (
                "model request bytes",
                m.model_request_bytes as u64,
                prompt as u64,
            ),
            (
                "estimated prompt tokens",
                m.estimated_prompt_tokens,
                estimated,
            ),
            (
                "model-visible reply bytes",
                m.visible_reply_bytes as u64,
                visible as u64,
            ),
        ] {
            if measured > limit {
                return Err(format!("{name} grew to {measured}, ceiling {limit}").into());
            }
        }
        Ok(())
    }
}

const SCENARIOS: &[&str] = &["board-build", "two-header", "drc-fix", "subcircuit-wiring"];

struct Task {
    mark: WorkflowMark,
    spec: Spec,
    expected: verify::Expected,
    goal: String,
    initial: sparksmith_core::format::EditorFile,
    start: Option<PathBuf>,
    ceilings: Option<Ceilings>,
}

impl Task {
    fn load(name: &str) -> Result<Self> {
        if !SCENARIOS.contains(&name) {
            return Err(format!("unknown task {name:?}").into());
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/agent-tasks")
            .join(name);
        let mut hash = Sha256::new();
        let mut read = |file: &str| -> Result<Vec<u8>> {
            let bytes = fs::read(dir.join(file))?;
            hash.update(file.as_bytes());
            hash.update([0]);
            hash.update(Sha256::digest(&bytes));
            Ok(bytes)
        };
        let spec: Spec = serde_json::from_slice(&read("task.json")?)?;
        let mut goal = String::from_utf8(read("goal.txt")?)?;
        let expected = read("expected.json")?;
        let (expected, start, initial) = match name {
            "subcircuit-wiring" => {
                let (_, session, _) = board_script::wiring::placed()?;
                (
                    verify::Expected::Wiring(serde_json::from_slice(&expected)?),
                    None,
                    session.engine().to_file(),
                )
            }
            "board-build" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct NewBoard {
                    fabrication_profile: String,
                }
                let recipe: NewBoard = serde_json::from_slice(&read("start.json")?)?;
                let session = board_script::fresh_with_profile(&recipe.fabrication_profile)?;
                (
                    verify::Expected::Build(serde_json::from_slice(&expected)?),
                    None,
                    session.engine().to_file(),
                )
            }
            "drc-fix" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Seed {
                    net: String,
                    position_mm: [f64; 2],
                }
                let seed: Seed = serde_json::from_slice(&read("start.json")?)?;
                let base = dir.with_file_name("two-header").join("start.sparksmith");
                let mut engine = verify::engine(sparksmith_io::read_project(&base)?)?;
                let project = engine.project();
                let net = project
                    .nets
                    .iter()
                    .find(|n| n.name == seed.net)
                    .ok_or("repair fixture net is missing")?;
                let via = sparksmith_core::layout::class_via(project, net.id, seed.position_mm);
                engine.apply(
                    engine.revision(),
                    sparksmith_core::Operation::Layout(
                        sparksmith_core::layout::LayoutCommand::AddVia { via },
                    ),
                )?;
                (
                    verify::Expected::Repair(serde_json::from_slice(&expected)?),
                    None,
                    engine.to_file(),
                )
            }
            _ => {
                read("start.sparksmith")?;
                let file = dir.join("start.sparksmith");
                let initial = sparksmith_io::read_project(&file)?;
                (
                    verify::Expected::Change(serde_json::from_slice(&expected)?),
                    Some(file),
                    initial,
                )
            }
        };
        if let verify::Expected::Wiring(wiring) = &expected {
            goal.push('\n');
            goal.push_str(&serde_json::to_string(&wiring.nets)?);
        }
        // A change to the shipped blank board or selected profile invalidates older recordings.
        hash.update(b"starting-document\0");
        hash.update(Sha256::digest(serde_json::to_vec(&initial)?));
        let selected = chat::ChatLibrary::locate()?.reach;
        if selected.name != spec.reach {
            return Err(format!("the task asks for unknown reach profile {:?}", spec.reach).into());
        }
        hash.update(b"reach\0");
        hash.update(Sha256::digest(serde_json::to_vec(&selected)?));
        let ceiling_file = dir.join("ceilings.json");
        let ceilings = if ceiling_file.try_exists()? {
            Some(serde_json::from_slice(&fs::read(ceiling_file)?)?)
        } else {
            None
        };
        let mark = WorkflowMark {
            id: name.to_owned(),
            version: spec.version.clone(),
            digest: format!("sha256:{:x}", hash.finalize()),
        };
        Ok(Self {
            mark,
            spec,
            expected,
            goal,
            initial,
            start,
            ceilings,
        })
    }
    fn session(&self) -> Result<Session> {
        let defaults = sparksmith_io::new_project::shipped()?;
        let mut engine = sparksmith_core::Engine::from_file_with(
            self.initial.clone(),
            &defaults.presets.glass_dk,
        )?;
        engine.set_context(sparksmith_io::standards::shipped_context()?);
        Ok(Session::fixture(engine))
    }
    fn scripted(&self) -> Scripted {
        match &self.expected {
            verify::Expected::Wiring(spec) => board_script::wiring::wire(spec.nets.clone()),
            verify::Expected::Build(_) => Scripted::board_build(),
            verify::Expected::Change(_) => Scripted::two_header(),
            verify::Expected::Repair(spec) => Scripted::drc_fix(spec.finding, false),
        }
    }
}

fn offline(
    task: &Task,
    dir: &Path,
    recording: Option<&Path>,
    detail: SchemaDetail,
    gate: bool,
) -> Result<Row> {
    let (runner, backend, reports_usage): (&str, Box<dyn ModelBackend>, bool) = match recording {
        Some(path) => {
            let replay = trace::replay(path, &task.mark)?;
            let reported = replay.reports_usage();
            ("recorded", Box::new(replay), reported)
        }
        None if task.mark.id == "board-build" && detail == SchemaDetail::Line => {
            ("scripted-lines-first", Box::new(lines::Board::new()), false)
        }
        None => ("scripted", Box::new(task.scripted()), false),
    };
    run::run(
        task,
        dir,
        offline_arm(runner, backend, reports_usage, detail),
        gate,
    )
}

fn offline_arm(
    runner: &str,
    backend: Box<dyn ModelBackend>,
    reports_usage: bool,
    detail: SchemaDetail,
) -> run::Arm {
    run::Arm {
        runner: runner.into(),
        backend,
        profile: ModelProfile::anthropic("claude-opus-5-5"),
        budget: chat::unbounded(),
        detail,
        reports_usage,
    }
}

/// Measure the extra read in an otherwise identical repair. Equal-length
/// directory names keep the save path's contribution to both prompts equal.
fn compare_repair(dir: &Path) -> Result<()> {
    let task = Task::load("drc-fix")?;
    let verify::Expected::Repair(ref repair) = task.expected else {
        return Err("missing repair task".into());
    };
    fs::create_dir(dir)?;
    let delta = offline(&task, &dir.join("delta"), None, SchemaDetail::Input, false)?;
    let arm = offline_arm(
        "scripted-recheck",
        Box::new(Scripted::drc_fix(repair.finding, true)),
        false,
        SchemaDetail::Input,
    );
    let control = run::run(&task, &dir.join("query"), arm, false)?;
    let (
        Row::Finish {
            metrics: a,
            saved_sha256: left,
            ..
        },
        Row::Finish {
            metrics: b,
            saved_sha256: right,
            ..
        },
    ) = (delta, control)
    else {
        return Err("missing repair results".into());
    };
    if left != right
        || a.calls.get("query").copied().unwrap_or(0) + 1
            != b.calls.get("query").copied().unwrap_or(0)
        || a.model_requests + 1 != b.model_requests
    {
        return Err("the paired scenarios did not produce identical boards with one extra query and model request".into());
    }
    Ok(())
}

/// Write proposed limits beside successful traces. Updating tracked limits is an explicit review step.
fn measure(dir: &Path) -> Result<()> {
    fs::create_dir(dir)?;
    for &name in SCENARIOS {
        let task = Task::load(name)?;
        let arm = dir.join(name);
        let run = offline(&task, &arm, None, SchemaDetail::Input, false)?;
        let Row::Finish {
            metrics,
            success: true,
            ..
        } = run
        else {
            return Err(format!("{name}: incomplete or unsuccessful scripted run").into());
        };
        let path_size = if matches!(
            task.expected,
            verify::Expected::Change(_) | verify::Expected::Repair(_)
        ) {
            Some(PathSize::of(
                &arm.canonicalize()?.join("revised.sparksmith"),
            )?)
        } else {
            None
        };
        let limits = Ceilings {
            model_request_bytes: metrics.model_request_bytes,
            estimated_prompt_tokens: metrics.estimated_prompt_tokens,
            visible_reply_bytes: metrics.visible_reply_bytes,
            saved_path: path_size,
        };
        let dest = arm.join("ceilings.json");
        let mut bytes = serde_json::to_vec_pretty(&limits)?;
        bytes.push(b'\n');
        sparksmith_io::persistence::atomic_create(&dest, &bytes)?;
        println!("{}: {}", name, dest.display());
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, out] if command == "compare-drc" => return compare_repair(Path::new(out)),
        [command, operator] if command == "plan" => return batch::plan(Path::new(operator)),
        [command, operator, out, binary] if command == "batch" => {
            return batch::execute(Path::new(operator), Path::new(out), Path::new(binary));
        }
        [command, operator, out] if command == "report" => {
            return batch::print_report(Path::new(operator), Path::new(out));
        }
        [command, binary, out] if command == "external-probe" => {
            return external::probe(Path::new(binary), Path::new(out));
        }
        [command, task, saved, wire, rest @ ..]
            if command == "external-report" && rest.len() <= 1 =>
        {
            return external::external_report(
                &Task::load(task)?,
                Path::new(saved),
                Path::new(wire),
                rest.first().map(Path::new),
            );
        }
        _ => (),
    }

    if let [kind, dir] = args.as_slice()
        && kind == "measure"
    {
        return measure(Path::new(dir));
    }
    let [kind, name, dir, rest @ ..] = args.as_slice() else {
        return Err("usage: cargo xtask agent-bench measure NEW_DIR | compare-drc NEW_DIR | scripted TASK NEW_DIR [line|input|full] | replay TASK NEW_DIR TRACE [line|input|full]".into());
    };
    let task = Task::load(name)?;
    let (recording, rest) = match (kind.as_str(), rest) {
        ("scripted", rest) => (None, rest),
        ("replay", [file, rest @ ..]) => (Some(Path::new(file)), rest),
        _ => return Err("expected scripted or replay with a recording path".into()),
    };
    let detail = match rest {
        [] => SchemaDetail::Input,
        [view] => serde_json::from_value(serde_json::json!(view))?,
        _ => return Err("expected one detail level".into()),
    };
    offline(&task, Path::new(dir), recording, detail, false)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripted_boards_verify_and_stay_within_the_measured_ceilings() -> Result<()> {
        let root = tempfile::tempdir()?;
        for &name in SCENARIOS {
            let task = Task::load(name)?;
            let dir = root.path().join(format!("{name}-a"));
            let first = offline(&task, &dir, None, SchemaDetail::Input, true)?;
            let second = offline(
                &task,
                &root.path().join(format!("{name}-b")),
                Some(&dir.join("trace.jsonl")),
                SchemaDetail::Input,
                false,
            )?;
            let (
                Row::Finish {
                    metrics: a,
                    saved_sha256: first,
                    ..
                },
                Row::Finish {
                    metrics: b,
                    saved_sha256: second,
                    ..
                },
            ) = (first, second)
            else {
                return Err("missing summary".into());
            };
            assert_eq!(first, second);
            assert_eq!(a.model_request_bytes, b.model_request_bytes);
            assert_eq!(a.visible_reply_bytes, b.visible_reply_bytes);
            if name == "two-header" {
                offline(
                    &task,
                    &root.path().join("a-longer-private-directory"),
                    None,
                    SchemaDetail::Input,
                    true,
                )?;
            } else if name == "board-build" {
                // A different segmentation of the same physical route is valid.
                let saved = dir.join("revised.sparksmith");
                let mut changed = sparksmith_io::read_project(&saved)?;
                let id = changed.project.allocate()?;
                let track = changed
                    .project
                    .tracks
                    .first_mut()
                    .ok_or("missing fixture track")?;
                let mut second = track.clone();
                let middle = [
                    f64::midpoint(track.start[0], track.end[0]),
                    f64::midpoint(track.start[1], track.end[1]),
                ];
                track.end = middle;
                second.id = id;
                second.start = middle;
                changed.project.tracks.push(second);
                let segmented = root.path().join("alternate-routing.sparksmith");
                sparksmith_io::write_project(&segmented, &changed)?;
                task.expected.verify(None, &segmented)?;
                changed.project.fills.clear();
                let missing_pour = root.path().join("missing-pour.sparksmith");
                sparksmith_io::write_project(&missing_pour, &changed)?;
                assert!(task.expected.verify(None, &missing_pour).is_err());
            }
            if name == "drc-fix" {
                let start = dir.join("start.sparksmith");
                let saved = dir.join("revised.sparksmith");
                assert!(
                    task.expected.verify(Some(&start), &start).is_err(),
                    "an unchanged board still has the identified defect"
                );
                let mut tampered = sparksmith_io::read_project(&saved)?;
                tampered.project.board_size[0] += 1.0;
                let unrelated = root.path().join("unrelated-board-edit.sparksmith");
                sparksmith_io::persistence::create_project(&unrelated, &tampered)?;
                assert!(task.expected.verify(Some(&start), &unrelated).is_err());
                assert_eq!(a.calls.get("apply"), Some(&1));
                assert_eq!(a.calls.get("query"), Some(&2)); // discovery and revision; no post-edit read
            }
            let mut wrong = task.mark.clone();
            wrong.version.push_str("-different");
            assert!(trace::replay(&dir.join("trace.jsonl"), &wrong).is_err());
        }
        Ok(())
    }
    #[test]
    fn board_build_reads_matching_lines_before_editing_and_verifies_the_saved_board() -> Result<()>
    {
        let root = tempfile::tempdir()?;
        let task = Task::load("board-build")?;
        let dir = root.path().join("lines-first");
        let row = offline(&task, &dir, None, SchemaDetail::Line, false)?;
        assert!(matches!(
            row,
            Row::Finish {
                success: true,
                artifact_verified: Some(true),
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn one_extra_visible_byte_fails_the_gate() {
        let limits = Ceilings {
            model_request_bytes: 500,
            estimated_prompt_tokens: 90,
            visible_reply_bytes: 100,
            saved_path: None,
        };
        let mut m = Metrics {
            model_request_bytes: 500,
            estimated_prompt_tokens: 90,
            visible_reply_bytes: 100,
            ..Default::default()
        };
        assert!(limits.check(&m, Path::new("unneeded")).is_ok());
        m.visible_reply_bytes += 1;
        assert!(limits.check(&m, Path::new("unneeded")).is_err());
    }
}

#[cfg(test)]
mod wiring_tests;

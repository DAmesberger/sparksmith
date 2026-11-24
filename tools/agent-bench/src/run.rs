use crate::trace::{self, Measured, Observer, Row, Trace};
use crate::{Result, Task};
use serde_json::{Value, json};
use sparksmith_assist::verbs::Agent;
use sparksmith_assist::{
    chat::{self, ChatConfig, ChatLibrary, End, Tools},
    model::{Budget, ModelBackend, ModelProfile, Part},
};
use sparksmith_automation::{Grant, Session, front::NoOne, session::SchemaDetail};
use std::{cell::RefCell, path::Path, sync::atomic::AtomicBool, time::Instant};

struct Calls<'a> {
    session: &'a mut Session,
    grant: Grant,
    pending: &'a RefCell<Option<(String, trace::Call)>>,
}
impl Tools for Calls<'_> {
    fn call(&mut self, verb: &str, input: &Value) -> Value {
        let key = json!({"verb":verb,"arguments":input}).to_string();
        let started = Instant::now();
        let reply = Agent {
            session: self.session,
            grant: &self.grant,
            turn: None,
            window: None,
            asker: &mut NoOne,
        }
        .verb(verb, input.clone());
        let server_ms = started.elapsed().as_secs_f64() * 1000.0;
        let entry = trace::entry(input);
        let ok = reply["ok"] == true;
        let body = reply[if ok { "result" } else { "error" }].to_string();
        let wire = trace::Call {
            transport: trace::Transport::InProcess,
            request: None,
            call: None,
            visible_bytes: 0,
            verb: verb.to_owned(),
            entry,
            request_bytes: key.len(),
            ok,
            server_ms,
            reply_bytes: reply.to_string().len(),
            result_sha256: trace::digest(body.as_bytes()),
        };
        *self.pending.borrow_mut() = Some((key, wire));
        reply
    }
}

pub struct Arm {
    pub runner: String,
    pub backend: Box<dyn ModelBackend>,
    pub profile: ModelProfile,
    pub budget: Budget,
    pub detail: SchemaDetail,
    pub reports_usage: bool,
}

/// Creates the arm's directory, owner-only where the platform has modes.
#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().create(dir)
}

pub fn run(task: &Task, dir: &Path, arm: Arm, gate: bool) -> Result<Row> {
    run_with_prompt(task, dir, arm, gate, None, None)
}
pub fn run_with_prompt(
    task: &Task,
    dir: &Path,
    arm: Arm,
    gate: bool,
    system: Option<&str>,
    goal: Option<&str>,
) -> Result<Row> {
    create_private_dir(dir)?;
    let dir = dir.canonicalize()?;
    let started = Instant::now();
    let trace = Trace::create(
        &dir.join("trace.jsonl"),
        &Row::Start {
            workflow: task.mark.clone(),
            runner: arm.runner,
            detail: serde_json::to_string(&arm.detail)?
                .trim_matches('"')
                .to_string(),
        },
    )?;
    if task.start.is_none() {
        // Keep the exact generated starting project with this arm's evidence.
        sparksmith_io::persistence::create_project(&dir.join("start.sparksmith"), &task.initial)?;
    }
    let generated_start = dir.join("start.sparksmith");
    let start = match &task.expected {
        crate::verify::Expected::Repair(_) => Some(generated_start.as_path()),
        _ => task.start.as_deref(),
    };
    let mut session = task.session()?;
    let lib = ChatLibrary::locate()?;
    let mut cfg = ChatConfig::new(&lib, arm.profile, arm.budget)?;
    cfg.surface_view.strategy = arm.detail;
    if let Some(system) = system {
        cfg.system = system.to_owned();
    }

    let mut grant = lib.reach.grant();
    grant.mode = task.spec.mode;
    grant.folders = vec![dir.clone()];
    let path = dir.join("revised.sparksmith");
    if let Some(start) = start {
        // Open the run's private copy. Its original bytes survive unless
        // the model itself saves through the file action.
        std::fs::copy(start, &path)?;
        let revision = session.engine().revision();
        let opened = Agent {
            session: &mut session,
            grant: &grant,
            turn: None,
            window: None,
            asker: &mut NoOne,
        }
        .verb(
            "act",
            json!({"name":"project.open","expected_revision":revision,"params":{"path":path}}),
        );
        if opened["ok"] != true {
            return Err(format!("couldn't open task fixture: {opened}").into());
        }
    }
    let pending = RefCell::new(None);
    let mut calls = Calls {
        session: &mut session,
        grant,
        pending: &pending,
    };
    let mut observer = Observer::new(&trace, &pending, trace::Transport::InProcess);
    let mut io_result = Ok(());
    let mut model = Measured {
        backend: arm.backend,
        trace: &trace,
    };
    let outcome = chat::turn(
        &cfg,
        &mut model,
        &mut Vec::new(),
        vec![Part::text(goal.unwrap_or(task.goal.trim_end()))],
        &mut calls,
        &mut |e| {
            if io_result.is_ok() {
                io_result = observer.event(e);
            }
        },
        &AtomicBool::new(false),
    );
    io_result?;
    if start.is_none() {
        // Build and wiring goals end with the in-memory design.
        sparksmith_io::write_project(&path, &session.engine().to_file())?;
    }
    let metrics = std::mem::take(&mut *trace.metrics.borrow_mut());

    let initial = task.start.as_deref().unwrap_or(&generated_start);
    let verification = task.expected.verify(Some(initial), &path);
    let artifact_verified = verification.is_ok();
    let verification_reason = verification.as_ref().err().map(ToString::to_string);
    let status = (|| -> Result<()> {
        verification?;

        if outcome.end != End::Answered {
            return Err(format!("turn ended: {}", outcome.end).into());
        }
        if gate {
            task.ceilings
                .as_ref()
                .ok_or("the scripted task has no recorded ceilings")?
                .check(&metrics, &path)?;
        }
        Ok(())
    })();
    let row = Row::Finish {
        artifact_verified: Some(artifact_verified),
        verification_reason,
        turn_end: Some(outcome.end.to_string()),
        success: status.is_ok(),
        reason: status.as_ref().err().map(ToString::to_string),
        wall_ms: started.elapsed().as_secs_f64() * 1000.0,
        revision: Some(session.engine().revision()),
        saved_sha256: Some(trace::digest(&std::fs::read(path)?)),
        usage: arm.reports_usage.then_some(outcome.usage.reported),
        metrics,
    };
    trace.write(&row)?;
    println!("{}", serde_json::to_string(&row)?);
    status?;
    Ok(row)
}

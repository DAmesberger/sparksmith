//! One turn of the chat, or one workflow run, off the UI thread: the
//! model's requests (and a run's steps) run here, and each tool call goes
//! back to the window, which runs it on its engine between frames and
//! answers. Stop sets the flag; a call the window holds for the person's
//! answer is given up when they stop.
use serde_json::{Map, Value, json};
use sparksmith_assist::RunConfig;
use sparksmith_assist::chat::{self, ChatConfig, Event, Outcome, Tools};
use sparksmith_assist::desk::{Called, Desk, Work};
use sparksmith_assist::model::{Message, ModelBackend, Part};
use sparksmith_assist::workflow::Loaded;
use sparksmith_assist::workflow::control::{ControlAnswer, ControlCard};
use sparksmith_assist::workflow::event::WorkflowEvent;
use sparksmith_assist::workflow::runner::{self, Person, WorkflowOutcome, WorkflowRun};
use sparksmith_automation::Grant;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// How long a waiting tool call sleeps between looks at Stop.
const POLL: Duration = Duration::from_millis(50);

/// Connects to the model, on the turn's thread (a backend need not be
/// `Send`).
pub type Connect = Arc<dyn Fn() -> Result<Box<dyn ModelBackend>, String> + Send + Sync>;

/// The model a chat uses: what it is declared to do, and how to reach it.
#[derive(Clone)]
pub struct ChatModel {
    pub profile: sparksmith_assist::model::ModelProfile,
    pub connect: Connect,
}

/// A tool call for the window to run; it answers on `reply`.
pub struct ToolRequest {
    pub verb: String,
    pub input: Value,
    pub reply: Sender<Called>,
    /// A workflow run's call; `None` for the chat's own turn.
    pub run: Option<RunCall>,
}

/// What a workflow run's call runs under: the chat's grant held to the
/// workflow's reach, and the run's turn once open. `entry`: the runner's own
/// call of the entry `verb` names, with `input` as its parameters.
#[derive(Clone, Debug)]
pub struct RunCall {
    pub grant: Grant,
    pub turn: Option<String>,
    pub entry: bool,
}

/// What the turn's thread tells the window.
pub enum FromTurn {
    Event(Event),
    Tool(ToolRequest),
    /// The turn ended; the history, grown by it, comes back.
    Done {
        outcome: Outcome,
        history: Vec<Message>,
    },
}

/// A turn running.
pub struct Job {
    pub rx: Receiver<FromTurn>,
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The tool calls of a turn, sent to the window.
struct ToWindow<'a> {
    tx: &'a Sender<FromTurn>,
    cancel: &'a AtomicBool,
    wake: &'a (dyn Fn() + Send + Sync),
}

impl Tools for ToWindow<'_> {
    fn call(&mut self, verb: &str, input: &Value) -> Value {
        let (reply, answer) = channel();
        let request = ToolRequest {
            verb: verb.to_owned(),
            input: input.clone(),
            reply,
            run: None,
        };
        if self.tx.send(FromTurn::Tool(request)).is_err() {
            return stopped();
        }
        (self.wake)();
        loop {
            match answer.recv_timeout(POLL) {
                Ok(called) => return called.envelope,
                Err(RecvTimeoutError::Timeout) if !self.cancel.load(Ordering::Relaxed) => {}
                Err(_) => return stopped(),
            }
        }
    }
}

fn stopped() -> Value {
    json!({"ok": false, "error": {"code": "chat.stopped",
        "message": "Not run: the person stopped the turn. Nothing was changed."}})
}

/// Run one turn of `config` on the model `connect` reaches, with
/// `history` and the person's `message`, on a thread of its own. `wake`
/// asks the window for a frame when there is something to show.
pub fn start(
    config: ChatConfig,
    connect: Connect,
    mut history: Vec<Message>,
    message: Vec<Part>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<Job, String> {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    std::thread::Builder::new()
        .name("assistant-turn".into())
        .spawn(sparksmith_core::library::standards::carried(move || {
            let outcome = match connect() {
                Ok(mut backend) => {
                    let events = tx.clone();
                    let waker = Arc::clone(&wake);
                    let mut tools = ToWindow {
                        tx: &tx,
                        cancel: &flag,
                        wake: wake.as_ref(),
                    };
                    chat::turn(
                        &config,
                        backend.as_mut(),
                        &mut history,
                        message,
                        &mut tools,
                        &mut |event| {
                            let _ = events.send(FromTurn::Event(event));
                            waker();
                        },
                        &flag,
                    )
                }
                Err(why) => Outcome {
                    end: chat::End::Failed(why),
                    usage: chat::Use::default(),
                },
            };
            let _ = tx.send(FromTurn::Done { outcome, history });
            wake();
        }))
        .map_err(|e| format!("could not start the turn: {e}"))?;
    Ok(Job { rx, cancel })
}

/// What a workflow run's thread tells the window.
pub enum FromRun {
    Workflow(WorkflowEvent),
    Tool(ToolRequest),
    /// Work on the chat's session; `done` once it ran.
    Work(Work, Sender<()>),
    /// The run ended, or was refused before anything ran.
    Done(Result<WorkflowOutcome, String>),
}

/// A workflow run in the chat.
pub struct RunJob {
    pub rx: Receiver<FromRun>,
    /// The person's answers, by step.
    pub answers: Sender<(String, ControlAnswer)>,
    pub cancel: Arc<AtomicBool>,
    /// The run's id, once it started.
    pub run: Option<String>,
    /// The workflow's result step, whose result the model is told.
    pub result_step: String,
}

impl RunJob {
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The window's session, as a run's desk: each call and each piece of work
/// is sent to the window and waited for.
struct WindowDesk<'a> {
    tx: &'a Sender<FromRun>,
    wake: &'a (dyn Fn() + Send + Sync),
}

impl WindowDesk<'_> {
    fn ask(&mut self, run: RunCall, verb: &str, input: Value) -> Called {
        let (reply, answer) = channel();
        let request = ToolRequest {
            verb: verb.to_owned(),
            input,
            reply,
            run: Some(run),
        };
        if self.tx.send(FromRun::Tool(request)).is_err() {
            return Called::refused(gone());
        }
        (self.wake)();
        answer.recv().unwrap_or_else(|_| Called::refused(gone()))
    }
}

impl Desk for WindowDesk<'_> {
    fn verb(&mut self, grant: &Grant, turn: Option<&str>, verb: &str, input: Value) -> Called {
        let run = RunCall {
            grant: grant.clone(),
            turn: turn.map(str::to_owned),
            entry: false,
        };
        self.ask(run, verb, input)
    }

    fn call(&mut self, grant: &Grant, turn: Option<&str>, entry: &str, params: Value) -> Value {
        let run = RunCall {
            grant: grant.clone(),
            turn: turn.map(str::to_owned),
            entry: true,
        };
        self.ask(run, entry, params).envelope
    }

    fn session(&mut self, work: Work) {
        let (done, wait) = channel();
        if self.tx.send(FromRun::Work(work, done)).is_ok() {
            (self.wake)();
            let _ = wait.recv();
        }
    }
}

fn gone() -> Value {
    json!({"ok": false, "error": {"code": "chat.stopped",
        "message": "Not run: the person stopped the run or closed the project. Nothing was changed."}})
}

/// The person at the chat: the run's events become its cards; a card the
/// run waits on waits for the person's answer, or Stop.
struct ChatPerson<'a> {
    tx: &'a Sender<FromRun>,
    answers: &'a Receiver<(String, ControlAnswer)>,
    cancel: &'a AtomicBool,
    wake: &'a (dyn Fn() + Send + Sync),
}

impl Person for ChatPerson<'_> {
    fn event(&mut self, event: &WorkflowEvent) {
        let _ = self.tx.send(FromRun::Workflow(event.clone()));
        (self.wake)();
    }

    fn answer(&mut self, step: &str, _cards: &[ControlCard]) -> Option<ControlAnswer> {
        loop {
            match self.answers.recv_timeout(POLL) {
                Ok((at, answer)) if at == step => return Some(answer),
                // An answer to a card the run is past.
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) if self.cancel.load(Ordering::Relaxed) => {
                    return Some(ControlAnswer::Stop);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Some(ControlAnswer::Stop),
            }
        }
    }
}

/// What a run starts from: the workflow, the inputs its card gave, the
/// chat's grant and the run's configuration (model, run profile, the
/// person's limits).
pub struct RunStart {
    pub loaded: Loaded,
    pub inputs: Map<String, Value>,
    pub grant: Grant,
    pub config: RunConfig,
}

/// Run a workflow on the model `connect` reaches, on a thread of its own:
/// its calls and session work go to the window, its cards to the chat.
/// `wake` asks the window for a frame when there is something to do.
pub fn start_run(
    start: RunStart,
    connect: Connect,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<RunJob, String> {
    let (tx, rx) = channel();
    let (answers, answered) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let result_step = start
        .loaded
        .workflow
        .steps
        .last()
        .map(|s| s.id.clone())
        .unwrap_or_default();
    std::thread::Builder::new()
        .name("assistant-run".into())
        .spawn(sparksmith_core::library::standards::carried(move || {
            let outcome = connect().and_then(|mut backend| {
                let run = WorkflowRun {
                    loaded: &start.loaded,
                    inputs: start.inputs,
                    grant: start.grant,
                    config: &start.config,
                };
                runner::run_workflow(
                    run,
                    backend.as_mut(),
                    &mut WindowDesk {
                        tx: &tx,
                        wake: wake.as_ref(),
                    },
                    &mut ChatPerson {
                        tx: &tx,
                        answers: &answered,
                        cancel: &flag,
                        wake: wake.as_ref(),
                    },
                    &flag,
                )
                .map_err(|e| e.to_string())
            });
            let _ = tx.send(FromRun::Done(outcome));
            wake();
        }))
        .map_err(|e| format!("could not start the run: {e}"))?;
    Ok(RunJob {
        rx,
        answers,
        cancel,
        run: None,
        result_step,
    })
}

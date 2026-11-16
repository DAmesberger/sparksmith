//! One turn of the chat off the UI thread: the model's requests run here,
//! and each tool call goes back to the window, which runs it on its engine
//! between frames and answers. Stop sets the turn's flag; a call the window
//! holds for the person's answer is given up when they stop.
use serde_json::{Value, json};
use sparksmith_assist::chat::{self, ChatConfig, Event, Outcome, Tools};
use sparksmith_assist::model::{Message, ModelBackend, Part};
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
    pub reply: Sender<Value>,
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
        };
        if self.tx.send(FromTurn::Tool(request)).is_err() {
            return stopped();
        }
        (self.wake)();
        loop {
            match answer.recv_timeout(POLL) {
                Ok(value) => return value,
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

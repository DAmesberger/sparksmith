//! The session thread. A [`Session`] is synchronous and does long work
//! (fills, checks, exports), and a call into the window waits for the
//! person, so the projects live on a thread of their own, the [`Desk`]; the
//! async handlers send it work over a bounded channel through a [`Handle`]
//! and await the answer, and the runtime stays free to read the next
//! request, or a cancellation.
//!
//! Work runs one job at a time, in the order it reached the channel. A job
//! sees only the desk, whose ways to run a call are [`Desk::call`],
//! [`Desk::open`] and [`Desk::catalog`]. Every call goes through the front
//! door ([`Surface::call`]):
//!
//! - **Headless**, on the desk's own sessions, as a remote agent under the
//!   server's [`Grant`]. Nobody can be asked, so what needs a person is
//!   refused (`gate.needs_person`).
//! - **Live**, in the open window, sent over the live channel
//!   ([`live::Client`]); the window's own gate decides, and asks the person
//!   in the window.
//!
//! After each job the desk publishes every project's revision, for
//! whatever watches them.
//!
//! **Projects have handles** (`docs/mcp-surface.md` §3.2): opaque
//! `prj_<UUID v4>` strings, minted for the project the server starts with
//! and, headless, for each `project.new` or `project.open`, which open
//! another project rather than replacing one, so clients that share this
//! process across conversations keep their work apart. A handle lasts until
//! the process exits. A call may leave the handle out only while one
//! project is open; with several, it must name one. Live, the one project is
//! the window's.
use crate::front::{self, Call, CallCx, NoOne, Surface};
use crate::gate::{Decision, Grant, Principal};
use crate::live;
use crate::session::Session;
use rmcp::ErrorData;
use serde_json::{Value, json};
use sparksmith_core::Diagnostic;
use std::collections::BTreeMap;
use tokio::sync::{mpsc, oneshot, watch};

/// Jobs that may wait for the desk before a sender waits too.
pub const QUEUE: usize = 64;
/// Projects one process holds open at once.
pub const MAX_PROJECTS: usize = 16;
/// What a handle starts with.
pub const HANDLE_PREFIX: &str = "prj_";
/// The longest handle a request may name, in bytes: the prefix and a UUID.
const MAX_HANDLE: usize = 64;
/// How a remote agent on this server is named to the gate.
const CLIENT: &str = "mcp";

/// Every open project's revision, by handle.
pub type Revisions = BTreeMap<String, u64>;

type Job = Box<dyn FnOnce(&mut Desk) + Send>;
/// Makes the session a new project starts from (with the host's library
/// sources, say).
pub type NewSession = Box<dyn Fn() -> Session + Send>;

/// Where the desk's calls run.
enum Projects {
    /// Sessions of this process, gated under `policy`.
    Headless {
        projects: BTreeMap<String, Session>,
        policy: Grant,
        new_session: NewSession,
    },
    /// The project open in the window, under the handle `handle`; its
    /// revision is the last one a reply named.
    Live {
        handle: String,
        window: live::Client,
        revision: Option<u64>,
    },
}

/// What a job may do with the projects.
pub struct Desk {
    projects: Projects,
}

/// Where the verbs' calls run ([`super::tools::dispatch`]): the MCP
/// server's desk, or one session in process for the built-in agent
/// (`sparksmith-assist`). Each answers in the transport envelope
/// (`{"ok", "result"}` or `{"ok": false, "error"}`).
pub trait Calls {
    /// Runs `call` on `project` (the only one when `None`).
    fn call(&mut self, project: Option<&str>, call: Call) -> Value;
    /// Runs a call about the catalog, which no project changes.
    fn catalog(&mut self, call: Call) -> Value;
    /// Runs `project.new` or `project.open`.
    fn open(&mut self, call: Call) -> Value;
    /// Whether the calls run in an open window.
    fn is_live(&self) -> bool;
    /// Whether this caller's grant can reach `entry` at all: `find` lists
    /// only those. Every reachable entry by default; the gate decides each
    /// call either way.
    fn reaches(&self, _entry: &crate::session::metadata::EntryMeta) -> bool {
        true
    }
}

impl Calls for Desk {
    fn call(&mut self, project: Option<&str>, call: Call) -> Value {
        Desk::call(self, project, call)
    }

    fn catalog(&mut self, call: Call) -> Value {
        Desk::catalog(self, call)
    }

    fn open(&mut self, call: Call) -> Value {
        Desk::open(self, call)
    }

    fn is_live(&self) -> bool {
        Desk::is_live(self)
    }
}

impl Desk {
    /// Starts the thread that owns `session`, the first project, and the
    /// ones opened later from `new_session`, all gated under `policy`. It
    /// ends when the last [`Handle`] is dropped.
    pub fn start(
        session: Session,
        policy: Grant,
        new_session: NewSession,
    ) -> Result<Handle, String> {
        let first = mint()?;
        Self::spawn(Self {
            projects: Projects::Headless {
                projects: BTreeMap::from([(first, session)]),
                policy,
                new_session,
            },
        })
    }

    /// Starts the thread that sends every call to the open window `window`
    /// reaches.
    pub fn start_live(window: live::Client) -> Result<Handle, String> {
        Self::spawn(Self {
            projects: Projects::Live {
                handle: mint()?,
                window,
                revision: None,
            },
        })
    }

    fn spawn(mut desk: Self) -> Result<Handle, String> {
        let (jobs, mut inbox) = mpsc::channel::<Job>(QUEUE);
        let (published, revisions) = watch::channel(desk.revisions());
        std::thread::Builder::new()
            .name("sparksmith-mcp-session".into())
            .spawn(move || {
                while let Some(job) = inbox.blocking_recv() {
                    job(&mut desk);
                    let now = desk.revisions();
                    published.send_if_modified(|seen| {
                        let changed = *seen != now;
                        *seen = now;
                        changed
                    });
                }
            })
            .map_err(|e| format!("cannot start the session thread: {e}"))?;
        Ok(Handle { jobs, revisions })
    }

    /// Whether the calls run in the open window.
    pub fn is_live(&self) -> bool {
        matches!(self.projects, Projects::Live { .. })
    }

    /// The handle of the project `project` names, or of the only one open
    /// when it names none. The failure is the transport envelope of an
    /// error, as [`super::outcome`] reads it.
    pub fn project(&self, project: Option<&str>) -> Result<String, Value> {
        let open: Vec<&String> = match &self.projects {
            Projects::Headless { projects, .. } => projects.keys().collect(),
            Projects::Live { handle, .. } => vec![handle],
        };
        match project {
            Some(handle) if open.iter().any(|h| h.as_str() == handle) => Ok(handle.to_owned()),
            Some(handle) => Err(failure(
                "project.unknown",
                format!(
                    "no open project has the handle {}: a handle lasts until the server process exits; act project.open {{path}} opens the file again",
                    shown(handle)
                ),
            )),
            None => match open.as_slice() {
                [only] => Ok((*only).clone()),
                [] => Err(failure(
                    "project.none",
                    "no project is open: act project.new or project.open {path} opens one".into(),
                )),
                _ => Err(failure(
                    "project.ambiguous",
                    "several projects are open: pass project, the handle from your last reply"
                        .into(),
                )),
            },
        }
    }

    /// Runs `call` on `project` through the front door and answers in the
    /// transport envelope, with the project's handle in the result.
    pub fn call(&mut self, project: Option<&str>, call: Call) -> Value {
        let handle = match self.project(project) {
            Ok(handle) => handle,
            Err(failure) => return failure,
        };
        let mut reply = match &mut self.projects {
            Projects::Headless {
                projects, policy, ..
            } => match projects.get_mut(&handle) {
                Some(session) => headless(session, policy, call),
                None => failure("project.unknown", "the project closed".into()),
            },
            Projects::Live {
                window, revision, ..
            } => {
                let reply = in_window(window, &call);
                if let Some(seen) = reply["result"]["revision"].as_u64() {
                    *revision = Some(seen);
                }
                reply
            }
        };
        stamp(&mut reply, &handle);
        reply
    }

    /// Runs a call about the catalog, which no project changes
    /// (`project.schema`), through the front door as [`Self::call`] does,
    /// on any open project.
    pub fn catalog(&mut self, call: Call) -> Value {
        match &mut self.projects {
            Projects::Headless {
                projects, policy, ..
            } => match projects.values_mut().next() {
                Some(session) => headless(session, policy, call),
                None => failure(
                    "project.none",
                    "no project is open: act project.new or project.open {path} opens one".into(),
                ),
            },
            Projects::Live { window, .. } => in_window(window, &call),
        }
    }

    /// Runs `project.new` or `project.open` (`call`). Headless, on a new
    /// session, through the front door as [`Self::call`] does; when it
    /// succeeds, the session becomes a project with a handle of its own,
    /// which the result carries. Nothing open is replaced, and a failure
    /// opens nothing. Live, the window decides: its file is the person's.
    pub fn open(&mut self, mut call: Call) -> Value {
        match &mut self.projects {
            Projects::Headless {
                projects,
                policy,
                new_session,
            } => {
                if projects.len() >= MAX_PROJECTS {
                    return failure(
                        "project.limit",
                        format!(
                            "{MAX_PROJECTS} projects are open, which is as many as this server holds"
                        ),
                    );
                }
                let mut session = new_session();
                // A new project replaces nothing, so it expects the new
                // session's own revision.
                call.expected_revision = Some(session.engine().revision());
                let mut reply = headless(&mut session, policy, call);
                if reply["ok"] != true {
                    return reply;
                }
                let handle = match mint() {
                    Ok(handle) => handle,
                    Err(why) => return failure("project.handle", why),
                };
                stamp(&mut reply, &handle);
                projects.insert(handle, session);
                reply
            }
            Projects::Live { window, .. } => {
                // The window's project is what it would replace.
                if call.expected_revision.is_none() {
                    match window.call(live::REVISION, json!({})) {
                        Ok(reply) => call.expected_revision = reply["result"]["revision"].as_u64(),
                        Err(message) => return unreachable(&message),
                    }
                }
                in_window(window, &call)
            }
        }
    }

    /// What the gate would decide for `call` on `project`, without running
    /// it. Live, the window decides when the call arrives.
    pub fn decide(&self, project: Option<&str>, call: &Call) -> Result<Decision, Value> {
        let handle = self.project(project)?;
        match &self.projects {
            Projects::Headless {
                projects, policy, ..
            } => match projects.get(&handle) {
                Some(session) => Ok(session.decide(&remote(policy), call)),
                None => Err(failure("project.unknown", "the project closed".into())),
            },
            Projects::Live { .. } => Err(failure(
                "live.decides",
                "the window decides each call when it arrives".into(),
            )),
        }
    }

    /// The revision of `project`, when it is open and known.
    pub fn revision(&self, project: &str) -> Option<u64> {
        self.revisions().get(project).copied()
    }

    /// Every open project's revision, as far as it is known: live, the one
    /// the last reply named.
    pub fn revisions(&self) -> Revisions {
        match &self.projects {
            Projects::Headless { projects, .. } => projects
                .iter()
                .map(|(handle, session)| (handle.clone(), session.engine().revision()))
                .collect(),
            Projects::Live {
                handle, revision, ..
            } => revision
                .map(|revision| (handle.clone(), revision))
                .into_iter()
                .collect(),
        }
    }

    /// The session behind `project`, to read; none live.
    pub fn session(&self, project: &str) -> Option<&Session> {
        match &self.projects {
            Projects::Headless { projects, .. } => projects.get(project),
            Projects::Live { .. } => None,
        }
    }
}

/// The principal of every headless call.
fn remote(policy: &Grant) -> Principal<'_> {
    Principal::RemoteAgent {
        grant: policy,
        client: CLIENT,
    }
}

/// `call` on a headless session: no one can be asked.
fn headless(session: &mut Session, policy: &Grant, call: Call) -> Value {
    let cx = CallCx {
        who: remote(policy),
        asker: &mut NoOne,
        verbose: false,
        window: None,
    };
    front::envelope(session.call(cx, call))
}

/// `call` in the window. A lost connection is a failure with the reason,
/// never retried.
fn in_window(window: &mut live::Client, call: &Call) -> Value {
    let envelope = match serde_json::to_value(call) {
        Ok(envelope) => envelope,
        Err(error) => return failure("malformed_request", error.to_string()),
    };
    match window.call(live::CALL, envelope) {
        Ok(reply) => reply,
        Err(message) => unreachable(&message),
    }
}

fn unreachable(message: &str) -> Value {
    failure("live.unreachable", message.to_owned())
}

/// The handlers' way to the desk; cheap to clone.
#[derive(Clone)]
pub struct Handle {
    jobs: mpsc::Sender<Job>,
    revisions: watch::Receiver<Revisions>,
}

impl Handle {
    /// Runs `work` on the desk and returns its answer. Fails, as an internal
    /// error, only when the session thread has stopped.
    pub async fn run<R, F>(&self, work: F) -> Result<R, ErrorData>
    where
        R: Send + 'static,
        F: FnOnce(&mut Desk) -> R + Send + 'static,
    {
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |desk| {
            // A caller that went away no longer needs the answer.
            drop(reply.send(work(desk)));
        });
        self.jobs.send(job).await.map_err(|_| stopped())?;
        answer.await.map_err(|_| stopped())
    }

    /// Every project's revision after the last job, and every later change.
    pub fn revisions(&self) -> watch::Receiver<Revisions> {
        self.revisions.clone()
    }
}

/// A new handle: the prefix and a random UUID v4.
fn mint() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("cannot make a project handle: {e}"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let part = |range: std::ops::Range<usize>| hex.get(range).unwrap_or_default();
    Ok(format!(
        "{HANDLE_PREFIX}{}-{}-{}-{}-{}",
        part(0..8),
        part(8..12),
        part(12..16),
        part(16..20),
        part(20..32)
    ))
}

/// Puts the project's handle into a successful reply's result, unless the
/// result carries the document itself there (`verbose: true`).
fn stamp(reply: &mut Value, handle: &str) {
    if reply["ok"] == true
        && let Some(result) = reply.get_mut("result").and_then(Value::as_object_mut)
    {
        result.entry("project").or_insert_with(|| json!(handle));
    }
}

/// A handle as a message repeats it: cut to its bound.
fn shown(handle: &str) -> String {
    handle.chars().take(MAX_HANDLE).collect()
}

/// The transport envelope of a failure with `code`.
pub fn failure(code: &str, message: String) -> Value {
    let diagnostic = Diagnostic {
        code: code.into(),
        rule: None,
        message,
        expected_revision: None,
        actual_revision: None,
    };
    json!({"ok": false, "error": diagnostic})
}

fn stopped() -> ErrorData {
    ErrorData::internal_error("the session thread has stopped", None)
}

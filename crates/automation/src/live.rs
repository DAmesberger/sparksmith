//! Live MCP: AI assistants edit the project open in the desktop window, on
//! the window's own engine and undo history.
//!
//! The window hands each call its [`WindowHost`], so the `view` entries
//! (selection, workspace, sheet, 2D or 3D) run in it and the busy rule is
//! the front door's ([`front::busy`]).
//!
//! The channel is Solidcraft's control channel (`solidcraft-control`,
//! docs/control-protocol.md there): a loopback-only listener, a per-session
//! token kept in a file only the user can read, one JSON line per request
//! and per reply. Each connection's thread hands its requests to the window
//! ([`Endpoint::next`]), which answers them on the UI thread between frames
//! with [`admit`] and [`run`]: the front door ([`Surface::call`]) the
//! headless transports use, over the window's engine ([`Session::lend`]).
//!
//! Two methods are served: [`CALL`] (one [`Call`] envelope: `entry`,
//! `params`, `expected_revision`, `dry_run`) and [`REVISION`]. A call that
//! changes the project names the revision it was prepared against, which
//! the front door checks as for every transport. Any call but a read or a
//! dry run is refused while the person is typing, dragging or has a menu,
//! dialog or review open. The permission gate decides as for any remote
//! agent, under the window's [`grant`]; Ask waits for the person's Allow in
//! the window. The CLI's `mcp --live` bridges an MCP client to this channel
//! ([`Client`]).
use crate::front::{self, AlreadyAllowed, Asker, Call, CallCx, NoOne, Surface, WindowHost};
use crate::gate::{Decision, Grant, Mode, Principal, Question, failure};
use crate::session::Session;
use serde_json::{Value, json};
use solidcraft_control::Effect;
use solidcraft_control::auth;
pub use solidcraft_control::auth::{App, ClientToken, Listen};
use solidcraft_control::server::{Hub, Protocol, Summary, Waits};
use sparksmith_core::Class;
use std::ffi::OsString;
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use solidcraft_control::ControlRequest;

/// Run one call of the front door: a [`Call`] envelope.
pub const CALL: &str = "surface.call";
/// The project's revision now, and whether it can undo and redo.
pub const REVISION: &str = "project.revision";
/// The first port tried; the next free one of [`auth::PORT_TRIES`] when it
/// is taken. Below the ephemeral ranges, apart from Solidcraft's 27878.
pub const DEFAULT_PORT: u16 = 27979;
/// How a remote agent on the live channel is named to the gate.
const CLIENT: &str = "live";
/// Connections served at once.
const MAX_CONNECTIONS: usize = 8;
/// The longest request line, as the MCP server's.
const MAX_LINE: usize = crate::mcp::MAX_LINE;
/// The longest reply a bridge reads (a `verbose` snapshot of a large board).
const MAX_REPLY: usize = 256 * 1024 * 1024;
/// How long a request waits for the window and for the person's answer: as
/// long as its client does. Its closed connection ends it
/// ([`Protocol::stop_on_disconnect`]).
const WAIT: Duration = Duration::MAX;
/// How long a new connection has to show the token: an unauthenticated
/// program must not keep one of the [`MAX_CONNECTIONS`].
const TOKEN_WAIT: Duration = Duration::from_secs(5);
const ACCEPT_POLL: Duration = Duration::from_millis(25);
/// Where a refused client reads how to connect.
const DOCS: &str = concat!(
    "the ",
    sparksmith_core::product_name!(),
    " README, section Live MCP"
);

/// Sparksmith's control channel: token files in `<config>/control`
/// (`SPARKSMITH_CONFIG_DIR` moves the config folder, `SPARKSMITH_CONTROL_DIR`
/// the token folder) and `SPARKSMITH_CONTROL_TOKEN` for a harness.
pub const APP: App = App {
    name: sparksmith_core::product::NAME,
    data_dir: config_dir,
    env: control_env,
    utc: seconds,
    default_port: DEFAULT_PORT,
};

fn config_dir() -> Option<PathBuf> {
    sparksmith_io::settings::UserDirs::from_env()
        .ok()
        .map(|dirs| dirs.config)
}

fn control_env(name: &str) -> Option<OsString> {
    std::env::var_os(format!("SPARKSMITH_{name}"))
}

fn seconds(secs: u64) -> String {
    format!("at {secs} s after the Unix epoch")
}

/// What the live channel's requests are, for the control server's
/// messages and waits.
struct LiveProtocol;

impl Protocol for LiveProtocol {
    fn summary(&self, method: &str, params: &Value) -> Summary {
        // What the call does is its effective class, as the gate decides
        // it: `project.apply_all` of `library.publish` is Ask, a read
        // naming a path is too. An unreadable one is refused at once.
        let typed = (method == CALL)
            .then(|| serde_json::from_value::<Call>(params.clone()).ok())
            .flatten()
            .and_then(|call| front::read(&call).ok())
            .map(|target| target.request);
        let class = typed.as_ref().map(crate::Request::effective_class);
        let names_path = typed.as_ref().is_some_and(|typed| !typed.names_no_path());
        let entry = params
            .get("entry")
            .and_then(Value::as_str)
            .and_then(crate::session::metadata::find)
            .map(|meta| meta.name.as_str());
        let effect = match (method, class) {
            // A show entry changes only the window, never the project.
            (REVISION, _) | (_, Some(Class::Read | Class::Show)) => Effect::ReadsOnly,
            (_, Some(Class::Ask)) if names_path => Effect::WritesFile,
            _ => Effect::Changes,
        };
        Summary {
            what: match entry {
                Some(entry) => format!("`{method}` ({entry})"),
                None => format!("`{}`", method.chars().take(64).collect::<String>()),
            },
            effect,
            command: entry,
            steps: None,
            long: false,
        }
    }
    /// A request is its client's: one that closed the connection (its call
    /// was cancelled, or it ended) is never run or held any more.
    fn stop_on_disconnect(&self, _method: &str) -> bool {
        true
    }
    fn document(&self) -> &'static str {
        "project"
    }
    fn change_event(&self) -> &'static str {
        "project.changed"
    }
    fn revision_method(&self) -> &'static str {
        REVISION
    }
    fn docs(&self) -> &'static str {
        DOCS
    }
    fn capabilities(&self) -> Value {
        json!({"methods": [CALL, REVISION]})
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The open connections, kept so turning the channel off closes them.
#[derive(Default)]
struct Open {
    next: u64,
    streams: Vec<(u64, TcpStream)>,
}

/// The window's end of the live channel, from turning it on until it is
/// dropped: the listener, the token and its files, and the requests the
/// connections hand over. Dropping it stops listening, closes every
/// connection and removes the token and discovery files.
pub struct Endpoint {
    port: u16,
    token: String,
    app: App,
    requests: Receiver<ControlRequest>,
    stop: Arc<AtomicBool>,
    open: Arc<Mutex<Open>>,
    worker: std::thread::Thread,
    token_file: Option<PathBuf>,
    discovery: bool,
}

impl std::fmt::Debug for Endpoint {
    /// Never the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Endpoint {
    /// Listen on loopback as `listen` says, with a new token (or the app's
    /// `CONTROL_TOKEN`), written to `app`'s token file for the port, which
    /// also becomes the discovery file's port. `wake` makes the window look
    /// at its requests now.
    pub fn start(
        app: App,
        listen: Listen,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let listener = match listen {
            Listen::Exact(port) => TcpListener::bind(("127.0.0.1", port))?,
            Listen::Default => auth::bind_with_fallback(app.default_port, auth::PORT_TRIES)?,
        };
        let port = listener.local_addr()?.port();
        let token = app
            .env_token()
            .or_else(solidcraft_control::server::new_token)
            .ok_or_else(|| std::io::Error::other("the system gave no random token"))?;
        let token_file = app.token_path(port);
        if let Some(path) = &token_file {
            app.write_token_file(path, &token)?;
        }
        let discovery = token_file.is_some() && app.write_current_port(port).is_ok();
        listener.set_nonblocking(true)?;
        let (send, requests) = mpsc::channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let hub_wake = Arc::clone(&wake);
        let hub = Arc::new(
            Hub::new(LiveProtocol, move || hub_wake(), MAX_LINE).with_waits(Waits {
                short: WAIT,
                long: WAIT,
                token: TOKEN_WAIT,
            }),
        );
        let stop = Arc::new(AtomicBool::new(false));
        let open = Arc::new(Mutex::new(Open::default()));
        let (worker_stop, worker_open, worker_token) = (
            Arc::clone(&stop),
            Arc::clone(&open),
            Arc::new(token.clone()),
        );
        let worker = std::thread::Builder::new()
            .name("sparksmith-live-mcp".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(_) => {
                            std::thread::park_timeout(ACCEPT_POLL);
                            continue;
                        }
                    };
                    if stream.set_nonblocking(false).is_err() {
                        continue;
                    }
                    let id = {
                        let mut open = lock(&worker_open);
                        if open.streams.len() >= MAX_CONNECTIONS {
                            drop(open);
                            let mut refused = stream;
                            let _ = refused
                                .write_all(b"{\"ok\":false,\"error\":\"too many connections\"}\n");
                            continue;
                        }
                        let Ok(clone) = stream.try_clone() else {
                            continue;
                        };
                        open.next = open.next.wrapping_add(1);
                        let id = open.next;
                        open.streams.push((id, clone));
                        id
                    };
                    wake();
                    let (hub, send, token, open, wake) = (
                        Arc::clone(&hub),
                        send.clone(),
                        Arc::clone(&worker_token),
                        Arc::clone(&worker_open),
                        Arc::clone(&wake),
                    );
                    let served = std::thread::Builder::new()
                        .name("sparksmith-live-peer".into())
                        .spawn(move || {
                            hub.serve(stream, &token, send, false);
                            lock(&open).streams.retain(|(i, _)| *i != id);
                            wake();
                        });
                    if served.is_err() {
                        lock(&worker_open).streams.retain(|(i, _)| *i != id);
                    }
                }
                for (_, stream) in lock(&worker_open).streams.drain(..) {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
            })?;
        Ok(Self {
            port,
            token,
            app,
            requests,
            stop,
            open,
            worker: worker.thread().clone(),
            token_file,
            discovery,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// This session's token, for a client the host configures itself and
    /// for tests. Never log it.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Connections open now (one that has not shown the token yet closes
    /// within [`TOKEN_WAIT`]).
    pub fn connections(&self) -> usize {
        lock(&self.open).streams.len()
    }

    /// The next request a connection handed over, if any.
    pub fn next(&self) -> Option<ControlRequest> {
        self.requests.try_recv().ok()
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.unpark();
        for (_, stream) in lock(&self.open).streams.drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(path) = &self.token_file {
            auth::remove_token_file(path, &self.token);
        }
        if self.discovery {
            self.app.remove_current_port(self.port, &self.token);
        }
    }
}

/// An MCP client's configuration for the CLI at `cli`: it finds the window
/// through the discovery file and reads the token from its file, so the
/// configuration holds no secret and outlives this session.
pub fn client_config(cli: &str) -> String {
    let config = json!({
        "mcpServers": {sparksmith_core::product::SHORT_NAME: {"command": cli, "args": ["mcp", "--live"]}}
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

/// What a remote agent may do in the window: `mode` and `folders` over
/// every module but `files`. Opening, saving and starting a project belong
/// to the window's own File workflow, which asks the person about unsaved
/// work and owns the file, so they are out of the agent's reach.
pub fn grant(mode: Mode, folders: impl IntoIterator<Item = std::path::PathBuf>) -> Grant {
    Grant::new(mode, folders).without_module("files")
}

/// The board and options behind a held native export. The window checks
/// this snapshot off-thread and presents the results before Allow.
#[derive(Debug)]
pub struct NativeExportReview {
    revision: u64,
    document: sparksmith_core::Document,
    checks: sparksmith_core::layout::CheckSettings,
    context: Option<Arc<sparksmith_core::library::standards::Context>>,
    params: crate::session::ExportNativeParams,
}

impl NativeExportReview {
    /// The document revision whose board will be exported.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The destination, resolved geometry tier and geometry-check override.
    pub fn params(&self) -> &crate::session::ExportNativeParams {
        &self.params
    }

    /// Production findings for the captured board and host check settings.
    pub fn preflight(
        &self,
    ) -> sparksmith_core::Result<sparksmith_core::fabrication::PreflightReport> {
        let _scope = self.context.as_ref().map(|context| context.enter());
        sparksmith_core::fabrication::preflight::report(
            &self.document.project,
            &self.document.drafts,
            None,
            &self.checks,
        )
    }

    fn matches(&self, engine: &sparksmith_core::Engine) -> bool {
        self.revision == engine.revision()
            && Arc::ptr_eq(&self.document.project, &engine.document().project)
            && Arc::ptr_eq(&self.document.drafts, &engine.document().drafts)
            && self.checks == *engine.check_settings()
            && match (&self.context, engine.context()) {
                (Some(reviewed), Some(current)) => Arc::ptr_eq(reviewed, current),
                (None, None) => true,
                _ => false,
            }
    }
}

/// A call the window may run, with the grant it was admitted under.
#[derive(Clone, Debug)]
pub struct Admitted {
    call: Call,
    subject: &'static str,
    class: Class,
    policy: Grant,
    /// Held for the person ([`Verdict::Ask`]): the host runs it only after
    /// their Allow.
    asked: bool,
    native_export: Option<Arc<NativeExportReview>>,
}

impl Admitted {
    /// The operation, query or request it names (`layout.add_via`).
    pub fn subject(&self) -> &'static str {
        self.subject
    }
    /// The snapshot to show when a person is asked to approve a native export.
    pub fn native_export(&self) -> Option<&Arc<NativeExportReview>> {
        self.native_export.as_ref()
    }
    pub fn class(&self) -> Class {
        self.class
    }
    /// Whether it is a dry run, which keeps nothing.
    pub fn dry_run(&self) -> bool {
        self.call.dry_run
    }
}

/// What the window does with one live request.
#[derive(Debug)]
pub enum Verdict {
    /// Answer at once with this reply.
    Reply(Value),
    /// Run it on the window's engine ([`run`]).
    Run(Admitted),
    /// Hold it until the person allows ([`run`]) or denies it.
    Ask(Admitted, Question),
}

fn refusal(code: &str, command: &str, why: String, instead: &str) -> Value {
    failure(&crate::gate::refusal(code, command, &why, instead))
}

fn agent(policy: &Grant) -> Principal<'_> {
    Principal::RemoteAgent {
        grant: policy,
        client: CLIENT,
    }
}

/// Decide one request on the window's session, under `policy` (the
/// window's [`grant`]). While the person holds `window` (typing, dragging,
/// a dialog) anything but a read or a dry run is refused
/// ([`front::busy`]), before a person would be asked.
pub fn admit(
    session: &Session,
    policy: &Grant,
    window: &dyn WindowHost,
    method: &str,
    params: &Value,
) -> Verdict {
    let engine = session.engine();
    match method {
        REVISION => {
            return Verdict::Reply(json!({"ok": true, "result": {
                "revision": engine.revision(),
                "can_undo": engine.can_undo(),
                "can_redo": engine.can_redo(),
            }}));
        }
        CALL => {}
        other => {
            let shown: String = other.chars().take(64).collect();
            return Verdict::Reply(json!({"ok": false, "error": {
                "code": "live.unknown_method",
                "message": format!("unknown method {shown}: use {CALL} or {REVISION}"),
            }}));
        }
    }
    let call = match serde_json::from_value::<Call>(params.clone()) {
        Ok(call) => call,
        Err(error) => {
            return Verdict::Reply(refusal(
                "malformed_request",
                CALL,
                format!("it is not a call: {error}. Nothing was changed."),
                "Send {entry, params, expected_revision?, dry_run?}.",
            ));
        }
    };
    let target = match front::read(&call) {
        Ok(target) => target,
        Err(error) => return Verdict::Reply(failure(&error)),
    };
    let class = target.request.effective_class();
    let mut admitted = Admitted {
        subject: target.request.subject(),
        call,
        class,
        policy: policy.clone(),
        asked: false,
        native_export: None,
    };
    if let Some(busy) = front::busy(window, admitted.subject, class, admitted.call.dry_run) {
        return Verdict::Reply(failure(&busy));
    }
    // A stale change is refused now rather than after the person's Allow;
    // the front door checks it again when it runs.
    if let Some(expected) = target.expected
        && let Err(stale) = engine.check_revision(expected)
    {
        return Verdict::Reply(failure(&front::from_diagnostic(stale.diagnostic())));
    }
    match session.decide(&agent(policy), &admitted.call) {
        Decision::Run => Verdict::Run(admitted),
        // A dry run keeps nothing, so what would ask a person runs.
        Decision::Ask(_) if admitted.call.dry_run => Verdict::Run(admitted),
        Decision::Ask(question) => {
            if let crate::Request::ExportNative(params) = target.request {
                admitted.native_export = Some(Arc::new(NativeExportReview {
                    revision: engine.revision(),
                    document: engine.document().clone(),
                    checks: *engine.check_settings(),
                    context: engine.context().cloned(),
                    params,
                }));
            }
            admitted.asked = true;
            Verdict::Ask(admitted, question)
        }
        Decision::Refuse(refused) => Verdict::Reply(failure(&refused)),
    }
}

/// Run an admitted call on the window's session and answer with the
/// compact reply. The front door gates it again under the grant it was
/// admitted with (a held one as allowed: the host runs it only on the
/// person's Allow) and checks the revision again: the person may have
/// edited while it waited. `window` runs the `view` entries and holds the
/// busy rule again.
pub fn run(session: &mut Session, admitted: &Admitted, window: &mut dyn WindowHost) -> Value {
    if admitted
        .native_export()
        .is_some_and(|review| !review.matches(session.engine()))
    {
        return refusal(
            "live.stale_approval",
            admitted.subject(),
            "The board or its check settings changed after the export was requested.".into(),
            "Request the export again to review the current board.",
        );
    }
    let mut no_one = NoOne;
    let mut allowed = AlreadyAllowed;
    let asker: &mut dyn Asker = if admitted.asked {
        &mut allowed
    } else {
        &mut no_one
    };
    let cx = CallCx {
        who: agent(&admitted.policy),
        asker,
        verbose: false,
        window: Some(window),
    };
    front::envelope(session.call(cx, admitted.call.clone()))
}

/// A bridge's connection to the window: one authenticated connection, kept
/// open and reopened after a failure. A request whose reply was lost is
/// never sent again.
pub struct Client {
    port: u16,
    token: ClientToken,
    next_id: u64,
    connection: Option<(BufReader<Reply>, TcpStream)>,
}

/// Reads a reply for as long as the window takes, unless the call it is for
/// is cancelled.
struct Reply(TcpStream);

impl Read for Reply {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        /// How often a waiting read looks for a cancellation.
        const POLL: Duration = Duration::from_millis(50);
        self.0.set_read_timeout(Some(POLL))?;
        loop {
            if sparksmith_generators::cancel::requested() {
                return Err(std::io::ErrorKind::ConnectionAborted.into());
            }
            match self.0.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                result => return result,
            }
        }
    }
}

impl Client {
    /// The window on `port`, or the one the discovery file names, with
    /// `token`, else the app's `CONTROL_TOKEN`, else its token file.
    pub fn find(app: &App, port: Option<u16>, token: Option<&str>) -> Result<Self, String> {
        let port = port.or_else(|| app.read_current_port()).ok_or(concat!(
            "no ",
            sparksmith_core::product_name!(),
            " window has live MCP on: in ",
            sparksmith_core::product_name!(),
            ", choose Help › Connect AI assistants… and turn it on"
        ))?;
        let token = app.client_token(port, token).ok_or_else(|| {
            format!(
                "no token for the {} window on port {port}: turn live MCP on in that window, or give SPARKSMITH_CONTROL_TOKEN",
                sparksmith_core::product::NAME
            )
        })?;
        Ok(Self::with_token(port, token))
    }

    pub fn with_token(port: u16, token: ClientToken) -> Self {
        Self {
            port,
            token,
            next_id: 0,
            connection: None,
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Send one request and wait for its reply line (`{ok, result | error}`),
    /// as long as the window takes: it may wait for the person. Cancelling
    /// the call this runs for ([`sparksmith_generators::cancel`]) closes the
    /// connection, so the window drops a request it has not run yet.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let unreachable = |error: std::io::Error| {
            format!(
                "could not reach the {} window on port {}: {error}. Is live MCP still on?",
                sparksmith_core::product::NAME,
                self.port
            )
        };
        self.next_id = self.next_id.wrapping_add(1);
        let mut line = json!({"id": self.next_id, "method": method, "params": params});
        let (mut reader, mut writer) = match self.connection.take() {
            Some(connection) => connection,
            None => {
                let stream = TcpStream::connect(("127.0.0.1", self.port)).map_err(unreachable)?;
                let reply = Reply(stream.try_clone().map_err(unreachable)?);
                let reader = BufReader::new(reply);
                line["token"] = Value::String(self.token.token.clone());
                (reader, stream)
            }
        };
        let mut bytes = serde_json::to_vec(&line).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        writer.write_all(&bytes).map_err(unreachable)?;
        let lost = |why: &str| {
            format!(
                "no reply from the {} window ({why}); inspect the project before sending a change again",
                sparksmith_core::product::NAME
            )
        };
        let reply = match crate::lines::next(&mut reader, MAX_REPLY) {
            Ok(Some(crate::lines::Line::Text(text))) => text,
            Ok(Some(_)) => return Err(lost("the reply was not readable")),
            Ok(None) | Err(_) if sparksmith_generators::cancel::requested() => {
                return Err("cancelled: the connection to the window was closed".into());
            }
            Ok(None) => return Err(lost("the connection closed")),
            Err(error) => return Err(lost(&error.to_string())),
        };
        let mut reply: Value = serde_json::from_str(&reply).map_err(|e| lost(&e.to_string()))?;
        if let Some(text) = reply.get("error").and_then(Value::as_str)
            && text.starts_with("unauthorized")
        {
            return Err(self.token.explain_error(text));
        }
        if let Some(object) = reply.as_object_mut() {
            object.remove("id");
        }
        self.connection = Some((reader, writer));
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;

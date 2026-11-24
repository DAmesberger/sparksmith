//! From a recorded MCP run to an engine replay (`sparksmith mcp-replay`):
//! the JSON-lines requests that `sparksmith stdio --operator` takes to
//! build the same project again, without a model.
//!
//! The record is a wire log ([`super::wirelog`], read by [`read_wire`]) or,
//! with less to go on, a client's own transcript ([`read_transcript`]).
//! Either gives the `tools/call` requests in the order they were sent and
//! what each was answered. [`convert`] then decides call by call:
//!
//! - **Mapped by the server's own code.** A call's arguments go through
//!   [`tools::dispatch`], the function the server answers `tools/call`
//!   with, onto a desk that keeps the front-door call instead of running
//!   it ([`Keeper`]). So the lenient argument reader, the catalog and the
//!   `apply` batch are the server's, and a replay line is that call as
//!   JSON lines send it. Each line is read back and compared with the
//!   call before it is kept.
//! - **Kept:** what was accepted and changes something: document and
//!   library edits (`apply`), actions (`act`: files, exports, publishing,
//!   `project.new` and `project.open`), and the reads that hold session
//!   state a later call builds on ([`holds_session_state`]).
//! - **Dropped:** what was refused or failed (it changed nothing), dry
//!   runs, every other read, and `find`, `describe` and `show`.
//! - **Refused loudly,** naming the call: an accepted call this build does
//!   not read, a call without an answer that would have been kept, a
//!   revision the kept calls do not explain (something changed the
//!   project outside the record), an edit moved onto other changes
//!   (`rebased`), a path outside the root, and a call that builds on what
//!   a refused call left behind ([`assist::takes_working_document`]).
//!
//! **One replay is one project.** The server holds several projects, each
//! a session of its own behind a handle; `stdio` holds one. A run that
//! changed several is converted one project at a time ([`Options::session`]).
//!
//! **Paths** inside the run's folder ([`Options::root`], or the one folder
//! the wire log's first row names) come out relative to it, so the replay
//! runs in any copy of that folder's input files.
use super::desk::Calls;
use super::tools::{self, ACT, APPLY, DESCRIBE, FIND, QUERY, SHOW};
use super::wirelog;
use crate::front::{self, Call};
use crate::lines::{self, Line};
use crate::session::assist;
use crate::session::metadata::{self, EntryMeta};
use serde::Serialize;
use serde_json::{Value, json};
use sparksmith_core::Class;
use sparksmith_core::permission::Effect;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Component, Path};

/// The revision a session starts at, which the server names for the
/// `project.new` or `project.open` that opens one ([`super::desk::Desk::open`]).
const START_REVISION: u64 = 0;
/// The longest row of a record: a transport line, escaped or as hex.
pub const MAX_ROW: usize = 8 * super::MAX_LINE;
/// The most calls one record may hold.
pub const MAX_CALLS: usize = 1_000_000;

/// One `tools/call` of a recorded run.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// Where it is in the record, for messages.
    pub at: String,
    pub verb: String,
    pub arguments: Value,
    /// What it was answered; `None` when the record holds no answer.
    pub reply: Option<Reply>,
}

/// What a call was answered, as far as the record shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    /// Its place among the answers, in the order they were given.
    pub order: usize,
    pub ok: bool,
    /// A failure's code.
    pub code: Option<String>,
    /// The project handle the server stamped on the result.
    pub project: Option<String>,
    /// The project's revision after the call.
    pub revision: Option<u64>,
    /// The call was moved onto changes made since it was prepared.
    pub rebased: bool,
}

impl Reply {
    /// The answer with the tool result's JSON `payload`, when it is whole.
    fn of(order: usize, ok: bool, payload: Option<&Value>) -> Self {
        let field = |key: &str| payload.and_then(|p| p.get(key));
        Self {
            order,
            ok,
            code: field("code")
                .filter(|_| !ok)
                .and_then(Value::as_str)
                .map(str::to_owned),
            project: field("project")
                .and_then(Value::as_str)
                .filter(|handle| handle.starts_with(super::desk::HANDLE_PREFIX))
                .map(str::to_owned),
            revision: field("revision").and_then(Value::as_u64),
            rebased: field("rebased").is_some(),
        }
    }
}

/// What to convert.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The folder the replay's paths are relative to. Without it, the one
    /// folder the wire log's first row names.
    pub root: Option<String>,
    /// The project to convert, counted from 1 in the order the run first
    /// used them; needed when the run changed several.
    pub session: Option<usize>,
}

/// A replay and how it was made.
#[derive(Clone, Debug)]
pub struct Replay {
    /// One `stdio` request per line.
    pub lines: Vec<Value>,
    pub summary: Summary,
}

/// What the converter kept and dropped.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    /// `tools/call` requests in the record.
    pub calls: usize,
    /// Lines in the replay.
    pub kept: usize,
    pub edits: usize,
    pub actions: usize,
    /// Reads kept for the session state later calls build on.
    pub state_reads: usize,
    /// Refused or failed calls, by code.
    pub refused: BTreeMap<String, usize>,
    pub dry_runs: usize,
    pub reads: usize,
    /// `find`, `describe`, `show` and help.
    pub catalog: usize,
    /// Calls without an answer in the record; none of them would be kept.
    pub unanswered: usize,
    /// Calls refused because no person could be asked: the run's gate
    /// requests.
    pub needs_person: Vec<String>,
    /// The projects the run used, in order, with their kept calls.
    pub sessions: Vec<usize>,
    /// The project converted, counted from 1.
    pub session: usize,
    /// The files and folders the replay names, relative to the root.
    pub paths: Vec<String>,
}

/// The front-door call a verb makes, kept instead of run.
#[derive(Default)]
struct Keeper {
    kept: Option<Mapped>,
    live: bool,
}

struct Mapped {
    /// The handle the call's arguments name.
    project: Option<String>,
    call: Call,
    how: How,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum How {
    Call,
    Catalog,
    Open,
}

impl Keeper {
    fn keep(&mut self, project: Option<&str>, call: Call, how: How) -> Value {
        self.kept = Some(Mapped {
            project: project.map(str::to_owned),
            call,
            how,
        });
        json!({"ok": true, "result": {}})
    }
}

impl Calls for Keeper {
    fn call(&mut self, project: Option<&str>, call: Call) -> Value {
        self.keep(project, call, How::Call)
    }

    fn catalog(&mut self, call: Call) -> Value {
        self.keep(None, call, How::Catalog)
    }

    fn open(&mut self, call: Call) -> Value {
        self.keep(None, call, How::Open)
    }

    fn is_live(&self) -> bool {
        self.live
    }
}

/// The front-door call `verb` makes of `arguments`; `None` for a verb that
/// reaches no project (`find`, `describe`, `show`, help).
fn mapped(verb: &str, arguments: &Value, live: bool) -> Result<Option<Mapped>, String> {
    match verb {
        FIND | DESCRIBE | SHOW => return Ok(None),
        QUERY | APPLY | ACT => {}
        other => return Err(format!("{other} is none of the six tools")),
    }
    let mut keeper = Keeper { kept: None, live };
    let reply = tools::dispatch(&mut keeper, verb, arguments.clone());
    match keeper.kept {
        Some(mapped) if mapped.how == How::Catalog => Ok(None),
        Some(mut mapped) => {
            // `verbose` only chooses the reply's shape; the front door takes
            // it out before it reads the call.
            if let Some(params) = mapped.call.params.as_object_mut() {
                params.remove("verbose");
            }
            Ok(Some(mapped))
        }
        None => Err(reply
            .as_ref()
            .and_then(|reply| reply["error"]["message"].as_str())
            .unwrap_or("its arguments do not read")
            .to_owned()),
    }
}

/// Whether a read's answer comes from the session rather than the document
/// or the world outside: it is declared to depend on more than the document
/// at a revision (`effect(World)`) while reaching nothing outside the
/// session. Those are the entries that build and read what a session holds
/// between calls (the working pinout documents), so a replay repeats them.
pub fn holds_session_state(entry: &EntryMeta) -> bool {
    entry.class == Class::Read && entry.effect == Effect::World && entry.reach.is_empty()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kept {
    Edit,
    Action,
    StateRead,
}

/// Whether an accepted `call` is kept, and as what.
fn kept(mapped: &Mapped) -> Result<Option<Kept>, String> {
    let entry = metadata::find(&mapped.call.entry)
        .ok_or_else(|| format!("no catalog entry {}", mapped.call.entry))?;
    if mapped.call.dry_run {
        return Ok(None);
    }
    if mapped.how == How::Open {
        return Ok(Some(Kept::Action));
    }
    Ok(match entry.class {
        Class::Read if holds_session_state(entry) => Some(Kept::StateRead),
        Class::Read | Class::Show => None,
        Class::Edit => Some(Kept::Edit),
        Class::Ask => Some(Kept::Action),
        Class::Never => return Err(format!("{} is in the never class", entry.name)),
    })
}

/// One project of the run: a session of the server's.
#[derive(Default)]
struct Project {
    handle: Option<String>,
    lines: Vec<Value>,
    /// The revision after its last kept call, when the record shows it.
    revision: Option<u64>,
    /// The answer order of its last kept call.
    answered: Option<usize>,
    /// A refused call that left session state behind, with its handle.
    left: Option<(String, Value)>,
    /// The paths its lines name.
    paths: Vec<String>,
}

/// The JSON-lines request that sends `call`.
fn line(call: &Call) -> Result<Value, String> {
    let mut params = match &call.params {
        Value::Null => json!({}),
        Value::Object(_) => call.params.clone(),
        _ => return Err("its params are not an object".into()),
    };
    if let (Some(revision), Some(object)) = (call.expected_revision, params.as_object_mut()) {
        object
            .entry("expected_revision")
            .or_insert_with(|| json!(revision));
    }
    let line = json!({"command": call.entry, "params": params});
    // The line must read as the request the run ran.
    let typed = |call: &Call| {
        front::read(call)
            .map_err(|error| error.message.clone())
            .and_then(|target| {
                serde_json::to_value(&target.request)
                    .map(|request| (request, target.expected))
                    .map_err(|e| e.to_string())
            })
    };
    let (back, _) = front::from_request(line.clone()).map_err(|error| error.message.clone())?;
    if typed(call)? != typed(&back)? {
        return Err("its JSON-lines request reads as another request".into());
    }
    Ok(line)
}

/// `path` relative to `root`, with `/` between its parts.
fn relative(path: &str, root: &str) -> Option<String> {
    let rest = Path::new(path).strip_prefix(Path::new(root)).ok()?;
    let mut parts = Vec::new();
    for part in rest.components() {
        match part {
            Component::Normal(part) => parts.push(part.to_str()?),
            _ => return None,
        }
    }
    Some(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

/// Replace every text of `value` that `renamed` names; how many it did.
fn rename(value: &mut Value, renamed: &BTreeMap<String, String>) -> usize {
    match value {
        Value::String(text) => match renamed.get(text.as_str()) {
            Some(new) => {
                text.clone_from(new);
                1
            }
            None => 0,
        },
        Value::Array(items) => items.iter_mut().map(|item| rename(item, renamed)).sum(),
        Value::Object(object) => object.values_mut().map(|item| rename(item, renamed)).sum(),
        _ => 0,
    }
}

/// Make the paths `call` names relative to `root`; the paths it names then.
fn relocate(call: &mut Call, root: Option<&str>) -> Result<Vec<String>, String> {
    let paths = front::read(call)
        .map_err(|error| error.message.clone())?
        .request
        .paths()
        .map_err(|e| e.to_string())?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let root = root.ok_or(
        "it names a path and the record names no single folder to make it relative to: give --root",
    )?;
    let mut renamed = BTreeMap::new();
    for path in &paths {
        let new = relative(path, root).ok_or_else(|| {
            format!("its path {path} is not inside {root}, so a replay elsewhere cannot name it")
        })?;
        renamed.insert(path.clone(), new);
    }
    // Every path is a text of the params, and no other text equals one.
    if rename(&mut call.params, &renamed) != paths.len() {
        return Err(
            "a path it names is also the text of another field, so its paths cannot be told apart"
                .into(),
        );
    }
    Ok(paths
        .iter()
        .filter_map(|path| renamed.get(path).cloned())
        .collect())
}

/// The one folder the wire log's first row grants, when it is one.
fn granted(started: Option<&Value>) -> Option<String> {
    let mut folders: Vec<&str> = started?["folders"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    folders.sort_unstable();
    folders.dedup();
    match folders.as_slice() {
        [only] => Some((*only).to_owned()),
        _ => None,
    }
}

/// The replay of `calls`, a run's `tools/call` requests in the order they
/// were sent. `started` is the wire log's first row.
pub fn convert(
    started: Option<&Value>,
    calls: &[Recorded],
    options: &Options,
) -> Result<Replay, String> {
    let live = started.is_some_and(|s| s["live"] == true);
    let root = options.root.clone().or_else(|| granted(started));
    let root = root.as_deref();
    let mut summary = Summary::default();
    // The first is the project the server started with.
    let mut projects = vec![Project::default()];
    let mut current = 0;
    if let Some(path) = started.and_then(|s| s["project"].as_str()) {
        let mut open = Call {
            entry: "project.open".into(),
            params: json!({"path": path}),
            project: None,
            expected_revision: Some(START_REVISION),
            dry_run: false,
            confirm: None,
            turn: None,
            job: false,
        };
        let at = |why: String| format!("the project the server started on ({path}): {why}");
        let named = relocate(&mut open, root).map_err(at)?;
        let line = line(&open).map_err(at)?;
        if let Some(first) = projects.first_mut() {
            first.lines.push(line);
            first.paths = named;
        }
    }
    for recorded in calls {
        summary.calls += 1;
        let at = &recorded.at;
        let read = mapped(&recorded.verb, &recorded.arguments, live);
        let Some(reply) = &recorded.reply else {
            // Whether it ran is unknown: fatal for a call that would count.
            if let Ok(Some(mapped)) = &read
                && kept(mapped)
                    .map_err(|why| format!("{at}: {why}"))?
                    .is_some()
            {
                return Err(format!(
                    "{at} has no answer in the record, so whether it ran is unknown"
                ));
            }
            summary.unanswered += 1;
            continue;
        };
        if !reply.ok {
            let code = reply.code.clone().unwrap_or_else(|| "unknown".to_owned());
            if code == "gate.needs_person" {
                summary.needs_person.push(at.clone());
            }
            *summary.refused.entry(code).or_default() += 1;
            if let Ok(Some(mapped)) = &read
                && assist::takes_working_document(&mapped.call.entry, &mapped.call.params)
            {
                let index = mapped
                    .project
                    .as_deref()
                    .and_then(|handle| {
                        projects
                            .iter()
                            .position(|p| p.handle.as_deref() == Some(handle))
                    })
                    .unwrap_or(current);
                if let Some(project) = projects.get_mut(index) {
                    let handle = mapped
                        .call
                        .params
                        .get("handle")
                        .cloned()
                        .unwrap_or_default();
                    project.left = Some((at.clone(), handle));
                }
            }
            continue;
        }
        let mapped = read.map_err(|why| {
            format!("{at} was accepted in the run but does not read at this build: {why}")
        })?;
        let Some(mut mapped) = mapped else {
            summary.catalog += 1;
            continue;
        };
        // The project it ran on: the one the server stamped, else the one
        // it named, else (as the server does) the current one for a read
        // and the only one for anything else.
        let index = if mapped.how == How::Open {
            projects.push(Project {
                handle: reply.project.clone(),
                ..Project::default()
            });
            projects.len() - 1
        } else {
            match reply.project.as_deref().or(mapped.project.as_deref()) {
                Some(handle) => {
                    let known = projects
                        .iter()
                        .position(|p| p.handle.as_deref() == Some(handle));
                    let unnamed: Vec<usize> = projects
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.handle.is_none())
                        .map(|(k, _)| k)
                        .collect();
                    match (known, unnamed.as_slice()) {
                        (Some(index), _) => index,
                        (None, [only]) => {
                            if let Some(project) = projects.get_mut(*only) {
                                project.handle = Some(handle.to_owned());
                            }
                            *only
                        }
                        _ => {
                            return Err(format!(
                                "{at}: the record does not show which project {handle} is"
                            ));
                        }
                    }
                }
                None if projects.len() == 1 => 0,
                None => current,
            }
        };
        current = index;
        let Some(kind) = kept(&mapped).map_err(|why| format!("{at}: {why}"))? else {
            if mapped.call.dry_run {
                summary.dry_runs += 1;
            } else {
                summary.reads += 1;
            }
            continue;
        };
        let entry = metadata::find(&mapped.call.entry);
        let Some(project) = projects.get_mut(index) else {
            return Err(format!("{at}: its project is not in the record"));
        };
        if reply.rebased {
            return Err(format!(
                "{at} was moved onto changes made after it was prepared (rebased): something \
                 other than the recorded calls changed the project, so the record cannot rebuild it"
            ));
        }
        if project.answered.is_some_and(|last| reply.order < last) {
            return Err(format!(
                "{at} was answered before a call sent earlier, so the order they ran in cannot \
                 be read from the record"
            ));
        }
        project.answered = Some(reply.order);
        if mapped.how == How::Open {
            mapped.call.expected_revision = Some(START_REVISION);
            if let Some(object) = mapped.call.params.as_object_mut() {
                object.remove("expected_revision");
            }
        } else if let (Some(expected), Some(known)) =
            (mapped.call.expected_revision, project.revision)
            && expected != known
        {
            return Err(format!(
                "{at} was accepted at revision {expected}, but the calls kept before it leave \
                 the project at revision {known}: something the record does not explain changed \
                 the project"
            ));
        }
        if kind == Kept::StateRead
            && let Some((refused, handle)) = &project.left
        {
            let replaces = assist::takes_working_document(&mapped.call.entry, &mapped.call.params)
                && mapped
                    .call
                    .params
                    .get("handle")
                    .cloned()
                    .unwrap_or_default()
                    == *handle;
            if !replaces {
                return Err(format!(
                    "{at} builds on what the refused {refused} left in the session, and a \
                     replay cannot repeat a refused call"
                ));
            }
            project.left = None;
        }
        project.revision = reply.revision.or(project
            .revision
            .filter(|_| entry.is_some_and(|e| !e.revision)));
        let named = relocate(&mut mapped.call, root).map_err(|why| format!("{at}: {why}"))?;
        let line = line(&mapped.call).map_err(|why| format!("{at}: {why}"))?;
        project.lines.push(line);
        project.paths.extend(named);
    }
    summary.sessions = projects.iter().map(|p| p.lines.len()).collect();
    let changed: Vec<usize> = projects
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.lines.is_empty())
        .map(|(k, _)| k)
        .collect();
    let chosen = match (options.session, changed.as_slice()) {
        (Some(n), _) if n >= 1 && n <= projects.len() => n - 1,
        (Some(n), _) => {
            return Err(format!(
                "the run used {} projects; there is no project {n}",
                projects.len()
            ));
        }
        (None, [only]) => *only,
        (None, []) => {
            return Err(
                "the record holds no accepted edit or action, so there is nothing to replay".into(),
            );
        }
        (None, several) => {
            return Err(format!(
                "the run changed {} projects (kept calls per project, in the order the run \
                 used them: {:?}); a replay holds one: convert each with --session N and \
                 replay them in the order the run saved them",
                several.len(),
                summary.sessions
            ));
        }
    };
    let Some(project) = projects.into_iter().nth(chosen) else {
        return Err("the chosen project is not in the record".into());
    };
    summary.session = chosen + 1;
    summary.kept = project.lines.len();
    for line in &project.lines {
        match line["command"]
            .as_str()
            .and_then(metadata::find)
            .map(|entry| entry.class)
        {
            Some(Class::Read) => summary.state_reads += 1,
            Some(Class::Edit) => summary.edits += 1,
            _ => summary.actions += 1,
        }
    }
    summary.paths = project.paths;
    summary.paths.sort();
    summary.paths.dedup();
    Ok(Replay {
        lines: project.lines,
        summary,
    })
}

/// A record's lines, each within [`MAX_ROW`], numbered from 1.
fn rows(
    input: &mut impl BufRead,
    mut each: impl FnMut(usize, &str) -> Result<(), String>,
) -> Result<(), String> {
    let mut n = 0usize;
    loop {
        n = n.saturating_add(1);
        match lines::next(input, MAX_ROW).map_err(|e| format!("line {n}: {e}"))? {
            None => return Ok(()),
            Some(Line::Text(text)) if text.trim().is_empty() => {}
            Some(Line::Text(text)) => each(n, &text).map_err(|why| format!("line {n}: {why}"))?,
            Some(Line::TooLarge) => return Err(format!("line {n} is longer than {MAX_ROW} bytes")),
            Some(Line::NotUtf8) => return Err(format!("line {n} is not UTF-8")),
        }
    }
}

/// A tool result's JSON payload: the first text block, when it is whole.
fn payload(content: &Value) -> Option<Value> {
    let text = match content {
        Value::String(text) => text.as_str(),
        Value::Array(blocks) => blocks.iter().find_map(|block| block["text"].as_str())?,
        _ => return None,
    };
    serde_json::from_str(text).ok()
}

/// The calls a record holds, as they are found.
#[derive(Default)]
struct Found {
    calls: Vec<Recorded>,
    /// Calls waiting for their answer, by request id.
    waiting: BTreeMap<String, usize>,
    answered: usize,
}

impl Found {
    fn sent(
        &mut self,
        id: String,
        source: &str,
        verb: &str,
        arguments: Value,
    ) -> Result<(), String> {
        if self.calls.len() >= MAX_CALLS {
            return Err(format!("the record holds more than {MAX_CALLS} calls"));
        }
        let at = format!(
            "call {} ({source}: {verb} {})",
            self.calls.len() + 1,
            super::trace::entry(&arguments)
        );
        if self.waiting.insert(id, self.calls.len()).is_some() {
            return Err(format!(
                "{at} reuses the id of a call still waiting for its answer"
            ));
        }
        self.calls.push(Recorded {
            at,
            verb: verb.to_owned(),
            arguments,
            reply: None,
        });
        Ok(())
    }

    fn answered(&mut self, id: &str, ok: bool, payload: Option<&Value>, code: Option<String>) {
        let Some(index) = self.waiting.remove(id) else {
            return;
        };
        self.answered += 1;
        if let Some(call) = self.calls.get_mut(index) {
            let mut reply = Reply::of(self.answered, ok, payload);
            if reply.code.is_none() {
                reply.code = code;
            }
            call.reply = Some(reply);
        }
    }
}

/// The first row and the `tools/call` requests of a wire log, in the order
/// the server read them.
pub fn read_wire(input: &mut impl BufRead) -> Result<(Value, Vec<Recorded>), String> {
    let mut started = None;
    let mut found = Found::default();
    rows(input, |n, text| {
        let (incoming, row, bytes) = match wirelog::row(text)? {
            wirelog::Row::Start(start) if n == 1 => {
                started = Some(start);
                return Ok(());
            }
            wirelog::Row::Start(_) => return Err("a second start row".into()),
            wirelog::Row::Bytes { .. } if started.is_none() => {
                return Err("not a wire log: its first row is not the start row".into());
            }
            // A piece of an overlong line is no message.
            wirelog::Row::Bytes { newline: false, .. } => return Ok(()),
            wirelog::Row::Bytes {
                incoming, n, bytes, ..
            } => (incoming, n, bytes),
        };
        // What is not JSON is no message; the server ignored it too.
        let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(());
        };
        let Some(id) = message
            .get("id")
            .filter(|id| id.is_number() || id.is_string())
            .map(Value::to_string)
        else {
            return Ok(());
        };
        if incoming {
            if message["method"] == "tools/call" {
                let params = &message["params"];
                let arguments = match &params["arguments"] {
                    Value::Null => json!({}),
                    arguments => arguments.clone(),
                };
                found.sent(
                    id,
                    &format!("wire row {row}"),
                    params["name"].as_str().unwrap_or_default(),
                    arguments,
                )?;
            }
            return Ok(());
        }
        if let Some(error) = message.get("error") {
            found.answered(&id, false, None, Some(format!("jsonrpc {}", error["code"])));
        } else if let Some(result) = message.get("result") {
            let payload = payload(&result["content"]);
            found.answered(&id, result["isError"] != true, payload.as_ref(), None);
        }
        Ok(())
    })?;
    let started = started.ok_or("not a wire log: it is empty")?;
    Ok((started, found.calls))
}

/// The MCP `tools/call` requests a client's transcript shows: Claude Code's
/// `stream-json` output, whose assistant messages hold `tool_use` blocks
/// named `mcp__SERVER__VERB` and whose user messages hold their results.
///
/// A transcript is the client's view, not the wire: it has the arguments
/// as the model wrote them and whether each result was an error, but no
/// first row (the folders and the project the server started on), and a
/// long result may be cut, which hides its revision and project handle.
pub fn read_transcript(input: &mut impl BufRead) -> Result<Vec<Recorded>, String> {
    let mut found = Found::default();
    rows(input, |n, text| {
        let row: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        let blocks = row["message"]["content"].as_array();
        for block in blocks.into_iter().flatten() {
            match (row["type"].as_str(), block["type"].as_str()) {
                (Some("assistant"), Some("tool_use")) => {
                    let Some(verb) = block["name"]
                        .as_str()
                        .and_then(|name| name.strip_prefix("mcp__"))
                        .and_then(|name| name.rsplit_once("__"))
                        .map(|(_, verb)| verb)
                        .filter(|verb| tools::NAMES.contains(verb))
                    else {
                        continue;
                    };
                    let id = block["id"].to_string();
                    // A message repeated in the stream repeats its calls.
                    if found.waiting.contains_key(&id) {
                        continue;
                    }
                    found.sent(
                        id,
                        &format!("transcript line {n}"),
                        verb,
                        block["input"].clone(),
                    )?;
                }
                (Some("user"), Some("tool_result")) => {
                    let payload = payload(&block["content"]);
                    found.answered(
                        &block["tool_use_id"].to_string(),
                        block["is_error"] != true,
                        payload.as_ref(),
                        None,
                    );
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    Ok(found.calls)
}

#[cfg(test)]
mod tests;

//! The one gate every call passes: given who calls ([`Principal`]) and the
//! entries a call reaches, [`decide_entries`] answers run, ask or refuse,
//! with text a model can act on. It holds whatever a model was told, because
//! datasheets and other documents are data: nothing in a request can widen
//! the grant.
//!
//! [`decide_entries`] is plain rules over plain data, the same for every
//! principal. The front door ([`crate::front`]) applies it to every call:
//! a model never runs a Never-class entry, an entry beyond its grant's
//! ceiling for the entry's module, nor a path outside its folders, and Ask
//! runs only after a person's Allow through the host's
//! [`crate::front::Asker`]. A host with no way to ask refuses Ask, and the
//! message says so.
//!
//! The classes themselves are chosen beside each command enum
//! ([`sparksmith_core::declare_entries!`], [`Request`]'s listing in `session`).
//!
//! The folder limit is a check on requests, not an operating-system sandbox:
//! a link swapped in between the check and the file access is out of scope.
use crate::session::{Request, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sparksmith_core::Class;
use sparksmith_core::permission::{MODULES, Reach, Reaches};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

pub use crate::front::Error;

/// How much a run may do without asking.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Reads run; every edit, file and publish asks.
    AskEveryTime,
    /// Edits run inside the reviewable turn; files and publishing ask. The
    /// founder's default.
    #[default]
    AutoAllowEdits,
    /// Everything runs except the Never class, inside the allowed folders.
    AllowAll,
}

impl Mode {
    pub const NAMES: &'static str = "ask-every-time, auto-allow-edits or allow-all";

    pub const fn name(self) -> &'static str {
        match self {
            Self::AskEveryTime => "ask-every-time",
            Self::AutoAllowEdits => "auto-allow-edits",
            Self::AllowAll => "allow-all",
        }
    }
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, String> {
        [Self::AskEveryTime, Self::AutoAllowEdits, Self::AllowAll]
            .into_iter()
            .find(|mode| mode.name() == text)
            .ok_or_else(|| format!("unknown permission mode {text}: use {}", Self::NAMES))
    }
}

/// What a principal may do: the permission mode, the folders paths may lie
/// in, a ceiling per module and, optionally, what outside the document its
/// entries may touch. With no folders, every request naming a path is
/// refused; name `/` to lift the limit. A module absent from `modules` is
/// out of reach.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Grant {
    pub mode: Mode,
    /// Absolute folders a request may name paths inside.
    pub folders: Vec<PathBuf>,
    /// The highest class each module may be used at, by module id.
    #[serde(default = "every_module")]
    pub modules: BTreeMap<String, Class>,
    /// How much one turn may do ([`crate::session::turns`]).
    #[serde(default)]
    pub limits: Limits,
    /// What outside the document an entry may declare it touches (`files`,
    /// `settings`, `network`, `window`, `runtime`): an entry declaring
    /// anything else is refused (`gate.reach`), and the verbs' `find` does
    /// not list it. `None` leaves it to the module ceilings and the folders.
    /// A path a call names is the folders' to judge, not this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<Vec<Reach>>,
}

/// What one turn of a principal may do: calls that change the document,
/// and calls of any kind. A turn past either stops with `gate.turn_limit`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub changes: u32,
    pub calls: u32,
}

impl Default for Limits {
    /// The defaults of Settings › AI (`docs/assistant-panel.md` §6.3):
    /// 60 changes, below the undo depth, and 400 calls.
    fn default() -> Self {
        Self {
            changes: 60,
            calls: 400,
        }
    }
}

/// Every declared module up to Ask: what a run's flags allow.
fn every_module() -> BTreeMap<String, Class> {
    MODULES
        .iter()
        .map(|module| (module.id.to_owned(), Class::Ask))
        .collect()
}

impl Default for Grant {
    fn default() -> Self {
        Self::new(Mode::default(), [])
    }
}

impl Grant {
    /// `mode` and `folders` over every module up to Ask, as the CLI and MCP
    /// flags give them: only the Never class is out of reach.
    pub fn new(mode: Mode, folders: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            mode,
            folders: folders.into_iter().collect(),
            modules: every_module(),
            limits: Limits::default(),
            reach: None,
        }
    }

    /// The built-in agent's default: auto-allow-edits, the document's
    /// modules up to Edit, and files, fabrication output and the 3D model
    /// up to Ask.
    pub fn local_agent(folders: impl IntoIterator<Item = PathBuf>) -> Self {
        let modules = MODULES
            .iter()
            .map(|module| {
                let ceiling = match module.id {
                    "files" | "fab" | "model3d" => Class::Ask,
                    _ => Class::Edit,
                };
                (module.id.to_owned(), ceiling)
            })
            .collect();
        Self {
            mode: Mode::AutoAllowEdits,
            folders: folders.into_iter().collect(),
            modules,
            limits: Limits::default(),
            reach: None,
        }
    }

    /// This grant with `module` usable up to `ceiling`.
    #[must_use]
    pub fn with_module(mut self, module: &str, ceiling: Class) -> Self {
        self.modules.insert(module.to_owned(), ceiling);
        self
    }

    /// This grant without `module`.
    #[must_use]
    pub fn without_module(mut self, module: &str) -> Self {
        self.modules.remove(module);
        self
    }

    /// Whether an entry that declares it touches `reaches` is within this
    /// grant's reach.
    pub fn reaches(&self, reaches: Reaches) -> bool {
        self.reach
            .as_ref()
            .is_none_or(|allowed| reaches.iter().all(|r| allowed.contains(&r)))
    }

    /// This grant held to `ceiling` as well: each module at the lower of
    /// the two classes, and a module `ceiling` leaves out out of reach (a
    /// workflow's `reach`). Mode, folders, reach and limits stay.
    #[must_use]
    pub fn narrowed(mut self, ceiling: &BTreeMap<String, Class>) -> Self {
        self.modules = std::mem::take(&mut self.modules)
            .into_iter()
            .filter_map(|(module, class)| {
                let other = ceiling.get(&module)?;
                Some((module, class.min(*other)))
            })
            .collect();
        self
    }
}

/// Who calls the surface. Named by the transport, never by the request.
#[derive(Clone, Copy, Debug)]
pub enum Principal<'a> {
    /// The person, through the window: a gesture, the palette, a shortcut.
    Person,
    /// The person's own script (`sparksmith-cli stdio --operator`), as at
    /// the command line.
    Operator,
    /// The built-in agent, in process, on the open project.
    LocalAgent { grant: &'a Grant },
    /// An MCP client or another model-facing transport.
    RemoteAgent { grant: &'a Grant, client: &'a str },
}

impl<'a> Principal<'a> {
    /// The grant that limits it; `None` for the person and the operator,
    /// whose every call runs.
    pub fn grant(&self) -> Option<&'a Grant> {
        match *self {
            Self::Person | Self::Operator => None,
            Self::LocalAgent { grant } | Self::RemoteAgent { grant, .. } => Some(grant),
        }
    }
}

/// How a transport names who sent a request to [`Session::respond`]: a
/// principal, and whether a person already allowed the request.
#[derive(Clone, Copy, Debug)]
pub enum Caller<'a> {
    /// The operator's own script (`sparksmith-cli stdio --operator`): every
    /// class runs, as at the command line. Never a model's.
    Operator,
    /// A model, under `grant`, with no one asked: Never, a module beyond
    /// the grant, a path outside the folders and whatever needs a person
    /// are refused.
    Model(&'a Grant),
    /// A model's request that a person allowed in the host: what needs a
    /// person runs; the rest is refused as for [`Caller::Model`].
    Allowed(&'a Grant),
}

impl<'a> Caller<'a> {
    /// The principal it names, and whether a person already allowed it.
    pub fn principal(self) -> (Principal<'a>, bool) {
        match self {
            Self::Operator => (Principal::Operator, false),
            Self::Model(grant) => (Principal::RemoteAgent { grant, client: "" }, false),
            Self::Allowed(grant) => (Principal::RemoteAgent { grant, client: "" }, true),
        }
    }
}

/// The gate's answer.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Run,
    Ask(Question),
    Refuse(Error),
}

/// What a person is asked to Allow or Deny.
#[derive(Clone, Debug, Serialize)]
pub struct Question {
    pub command: String,
    pub class: Class,
    /// The paths the request names, as given.
    pub paths: Vec<String>,
    pub why: String,
}

impl Question {
    /// The answer for the model when the person chose Deny.
    pub fn denied(&self) -> Error {
        refusal(
            "gate.denied",
            &self.command,
            "the person denied it. Nothing was changed.",
            "Do not retry it unless the person asks; carry on without it or ask them what they want.",
        )
    }

    /// The answer when no one can be asked: the step is the person's, so
    /// it counts as refused.
    pub fn unanswered(&self) -> Error {
        refusal(
            "gate.needs_person",
            &self.command,
            &format!(
                "{} It needs a person's Allow, and this host has no way to ask, so it counts as refused. Nothing was changed.",
                self.why
            ),
            concat!(
                "Finish what you can without it and tell the user what is left for them to do. They can run it themselves in ",
                sparksmith_core::product_name!(),
                ", connect you to their open window (mcp --live), where they are asked, or start this session in a permission mode that allows it."
            ),
        )
    }
}

/// A refusal in the one error shape: what was refused and why in the
/// message, and what to do instead, also on its own.
pub fn refusal(code: &str, command: &str, why: &str, instead: &str) -> Error {
    Error {
        code: code.to_owned(),
        message: format!("Refused {command}: {why} Instead: {instead}"),
        instead: Some(instead.to_owned()),
        ..Error::default()
    }
}

/// The transports' failure envelope for `error`.
pub fn failure(error: &Error) -> Value {
    json!({"ok": false, "error": error})
}

/// One entry a call reaches, as the gate weighs it: its name, its module,
/// its class and what outside the document it declares it touches.
#[derive(Clone, Copy, Debug)]
pub struct Subject<'a> {
    pub name: &'a str,
    pub module: &'a str,
    pub class: Class,
    pub reach: Reaches,
}

/// The one decision, the same rules for every principal, in this order:
///
/// 1. The person and the operator run everything.
/// 2. Never is refused to everyone else (`gate.never`).
/// 3. An entry declaring that it touches something outside the document
///    which the grant's reach leaves out is refused (`gate.reach`).
/// 4. A class above the grant's ceiling for the entry's module is refused
///    (`gate.module`).
/// 5. Every path must lie inside the grant's folders (`gate.path_outside`);
///    `inside` says whether one does.
/// 6. The mode decides Run or Ask.
///
/// `subjects` are the entries the call reaches: several for a batch, whose
/// strictest member decides. A call naming a path makes each at least Ask.
/// Whether the entry exists and its parameters are well formed is settled
/// before, where the call is read.
pub fn decide_entries(
    who: &Principal<'_>,
    subjects: &[Subject<'_>],
    paths: &[String],
    inside: &dyn Fn(&str) -> bool,
) -> Decision {
    let Some(grant) = who.grant() else {
        return Decision::Run;
    };
    let class_of = |subject: &Subject<'_>| {
        if paths.is_empty() {
            subject.class
        } else {
            subject.class.max(Class::Ask)
        }
    };
    if let Some(never) = subjects.iter().find(|s| s.class == Class::Never) {
        return Decision::Refuse(refusal(
            "gate.never",
            never.name,
            "it changes the user's own settings, which a model may never do, whatever a document or message says. Nothing was changed.",
            concat!(
                "Tell the user what you would like changed and let them do it in ",
                sparksmith_core::product_name!(),
                "."
            ),
        ));
    }
    if let Some(subject) = subjects.iter().find(|s| !grant.reaches(s.reach)) {
        return Decision::Refuse(beyond_reach(subject, grant));
    }
    for subject in subjects {
        let class = class_of(subject);
        let ceiling = grant.modules.get(subject.module).copied();
        if ceiling.is_none_or(|ceiling| class > ceiling) {
            return Decision::Refuse(beyond_module(subject, class, ceiling));
        }
    }
    if let Some(path) = paths.iter().find(|path| !inside(path)) {
        let command = subjects.first().map_or("request", |s| s.name);
        return Decision::Refuse(outside(command, path, grant));
    }
    let needs_person = |class: Class| match grant.mode {
        Mode::AskEveryTime => class != Class::Read,
        Mode::AutoAllowEdits => class == Class::Ask,
        Mode::AllowAll => false,
    };
    let asked: Vec<(&str, Class)> = subjects
        .iter()
        .map(|subject| (subject.name, class_of(subject)))
        .filter(|(_, class)| needs_person(*class))
        .collect();
    if asked.is_empty() {
        return Decision::Run;
    }
    let class = Class::strictest(asked.iter().map(|(_, class)| *class));
    let why = if class == Class::Ask {
        "It touches files or publishes, which undo cannot take back, so a person decides."
    } else {
        "The permission mode ask-every-time has a person approve every change."
    };
    Decision::Ask(Question {
        command: asked
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", "),
        class,
        paths: paths.to_vec(),
        why: why.to_owned(),
    })
}

fn beyond_reach(subject: &Subject<'_>, grant: &Grant) -> Error {
    let touches: Vec<&str> = subject
        .reach
        .iter()
        .filter(|r| !grant.reaches(Reaches::NONE.with(*r)))
        .map(Reach::name)
        .collect();
    refusal(
        "gate.reach",
        subject.name,
        &format!(
            "it touches {}, which this run does not reach. Nothing was changed.",
            touches.join(" and ")
        ),
        "Carry on without it and say what is left for the person.",
    )
}

fn beyond_module(subject: &Subject<'_>, class: Class, ceiling: Option<Class>) -> Error {
    let module = subject.module;
    let (why, instead) = match ceiling {
        None => (
            format!("this run is not granted the {module} module. Nothing was changed."),
            format!(
                "Carry on without it, or ask the user to do it themselves in {}.",
                sparksmith_core::product::NAME
            ),
        ),
        Some(ceiling) => (
            format!(
                "this run may use the {module} module only up to {}, and this is {}. Nothing was changed.",
                ceiling.name(),
                class.name()
            ),
            format!(
                "Carry on without it, or ask the user to grant {module} up to {} for this run.",
                class.name()
            ),
        ),
    };
    refusal("gate.module", subject.name, &why, &instead)
}

/// The rules for a model's `request` under `grant`, as written: a path
/// counts only when it is absolute, has no `..` and lies inside an allowed
/// folder. [`Session::check_request`] decides against what is on disk.
pub fn decide(grant: &Grant, request: &Request) -> Decision {
    decide_batch(grant, std::slice::from_ref(request))
}

/// One decision for requests that run together: the strictest member's. Any
/// refusal refuses them all, and any Ask asks for them all.
pub fn decide_batch(grant: &Grant, requests: &[Request]) -> Decision {
    let who = Principal::RemoteAgent { grant, client: "" };
    let mut subjects = Vec::new();
    let mut paths = Vec::new();
    for request in requests {
        let Ok(named) = request.paths() else {
            return Decision::Refuse(unreadable(request.subject()));
        };
        // A path makes its own request at least Ask, not its neighbours.
        let raise = !named.is_empty();
        subjects.extend(request.subjects().into_iter().map(|s| Subject {
            class: if raise {
                s.class.max(Class::Ask)
            } else {
                s.class
            },
            ..s
        }));
        paths.extend(named);
    }
    let inside = |path: &str| {
        lexical(path).is_some_and(|path| grant.folders.iter().any(|f| path.starts_with(f)))
    };
    decide_entries(&who, &subjects, &paths, &inside)
}

/// A request whose paths could not be read is refused rather than read as
/// naming none.
pub(crate) fn unreadable(command: &str) -> Error {
    refusal(
        "gate.unreadable",
        command,
        "its parameters could not be read to find the files it names. Nothing was changed.",
        "Send the request again with plain JSON values.",
    )
}

fn outside(command: &str, path: &str, grant: &Grant) -> Error {
    const SHOWN: usize = 8;
    let folders = grant
        .folders
        .iter()
        .take(SHOWN)
        .map(|folder| folder.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let instead = if folders.is_empty() {
        "This run allows no folders: leave the path out if the request can do without one, or ask the user to allow the folder.".to_owned()
    } else {
        format!(
            "Give an absolute path without `..` inside {folders}, or ask the user to allow the folder."
        )
    };
    refusal(
        "gate.path_outside",
        command,
        &format!(
            "the path {path} is outside this run's allowed folders. Nothing was read or written."
        ),
        &instead,
    )
}

/// `path` when it is absolute and has no `..`.
pub(crate) fn lexical(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    let clean = path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir));
    clean.then(|| path.to_path_buf())
}

/// Where `path` really leads: links followed in its deepest existing part,
/// the rest as written. `None` for a dangling link, which could lead anywhere.
pub(crate) fn real_path(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            return Some(rest.into_iter().rev().fold(real, |p, part| p.join(part)));
        }
        if std::fs::symlink_metadata(existing).is_ok() {
            return None;
        }
        rest.push(existing.file_name()?);
        existing = existing.parent()?;
    }
}

impl Session {
    /// The call for a model's JSON request (`{"command", "params"}`) under
    /// `grant`, before running it: [`crate::front::Surface::decide`] as a
    /// remote agent, against what is on disk.
    pub fn check_request(&self, grant: &Grant, request: &Value) -> Decision {
        use crate::front::Surface;
        let who = Principal::RemoteAgent { grant, client: "" };
        match crate::front::from_request(request.clone()) {
            Ok((call, _)) => self.decide(&who, &call),
            Err(error) => Decision::Refuse(*error),
        }
    }
}

#[cfg(test)]
mod tests;

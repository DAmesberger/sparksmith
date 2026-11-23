//! The front door (`docs/extension-surface.md`): one envelope, [`Call`],
//! from one of the kinds of caller, [`Principal`], through one gate
//! ([`crate::gate::decide_entries`]), into the one engine. Every request a
//! transport sends ([`Session::respond`], [`Session::dispatch`],
//! [`Session::execute`], [`Session::rehearse`]) is a call here, and every
//! failure is the one [`Error`].
//!
//! A call names any catalog entry: an operation (`layout.add_via`), a query
//! (`project.issues`) or a request (`project.save`). `expected_revision` is
//! the envelope's: every entry that changes the document requires it, and
//! the session checks it once, before any file is read or written. The
//! wrappers `project.apply`, `project.apply_all` and `project.query` remain
//! the shapes the JSON-lines transport sends (MCP names entries itself and
//! sends a batch as `project.apply_all`); the gate weighs the operations and
//! queries inside them, never the wrapper.
//!
//! What needs a person is asked through the host's [`Asker`]; a host with
//! no way to ask ([`NoOne`]) refuses it, saying so.
//!
//! A host with a window hands the call its [`WindowHost`]: the `view`
//! entries run there (`host.unavailable` without one), and while the person
//! holds the window (typing, dragging, a dialog) anything but a read or a
//! dry run is refused with `live.busy` ([`busy`]).
use crate::gate::{self, Decision, Question, Subject};
use crate::session::{ApplyParams, Request, Session, metadata, take_verbose};
use serde_json::{Map, Value, json};
use sparksmith_core::{Diagnostic, Operation, Query};
use std::path::PathBuf;

pub use crate::gate::{Grant, Principal};
pub use crate::view::WindowHost;
pub use sparksmith_generator_abi::surface::Error;

/// One call, the same for every transport: the entry, its parameters and
/// the envelope's `expected_revision`, `dry_run` and `turn`. `confirm` and
/// `job` are refused until the slices that serve them; `project` is for the
/// transport that holds several sessions, each of which is one project.
pub type Call = sparksmith_generator_abi::surface::Call<Value>;

/// A person's answer to a [`Question`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    Allow,
    Deny,
    /// This host has no way to ask.
    Unavailable,
}

/// How a host asks a person: the window's Allow prompt, a card in a panel,
/// or no way at all. The model's wording is never the control.
pub trait Asker {
    fn ask(&mut self, question: &Question) -> Answer;
}

/// A host with no way to ask: what needs a person is refused.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoOne;

impl Asker for NoOne {
    fn ask(&mut self, _: &Question) -> Answer {
        Answer::Unavailable
    }
}

/// A request a person already allowed in the host before it was sent (the
/// live window holds it until they do).
#[derive(Clone, Copy, Debug, Default)]
pub struct AlreadyAllowed;

impl Asker for AlreadyAllowed {
    fn ask(&mut self, _: &Question) -> Answer {
        Answer::Allow
    }
}

/// Who calls and how a person is asked, for one call.
pub struct CallCx<'a> {
    pub who: Principal<'a>,
    pub asker: &'a mut dyn Asker,
    /// Answer with the whole document state instead of what the call
    /// changed. `verbose: true` in the parameters asks for it too.
    pub verbose: bool,
    /// The window the call runs in; `None` for a host without one.
    pub window: Option<&'a mut dyn WindowHost>,
}

/// The front door.
pub trait Surface {
    /// The entry `name`, of any family and class.
    fn entry(&self, name: &str) -> Option<&'static metadata::EntryMeta> {
        metadata::find(name)
    }

    /// What the gate decides for `call` from `who`, without running it: an
    /// unknown entry or malformed parameters are refused here too.
    fn decide(&self, who: &Principal<'_>, call: &Call) -> Decision;

    /// Gate `call` for `cx.who`, ask through `cx.asker` where the gate
    /// says so, then run it. A refused or failed call changes nothing.
    fn call(&mut self, cx: CallCx<'_>, call: Call) -> Result<Value, Box<Error>>;
}

/// A call read against the catalog: the typed request that runs it and the
/// revision it expects.
pub(crate) struct Target {
    pub(crate) request: Request,
    pub(crate) expected: Option<u64>,
}

impl Surface for Session {
    fn decide(&self, who: &Principal<'_>, call: &Call) -> Decision {
        match read(call) {
            Ok(target) => self.gate(who, &target.request),
            Err(error) => Decision::Refuse(*error),
        }
    }

    fn call(&mut self, cx: CallCx<'_>, mut call: Call) -> Result<Value, Box<Error>> {
        let verbose = cx.verbose | take_verbose_params(&mut call.params);
        unsupported(&call)?;
        let target = read(&call)?;
        // Validate both the transport wrapper and its selected entry on
        // successful calls exercised by the automation test suite.
        #[cfg(test)]
        let subject = match &target.request {
            Request::Query(_) | Request::Apply(_) => Some(target.request.subject()),
            _ => None,
        };
        // An agent's edit may go on top of what someone else changed since
        // it last looked: only when the document moved without this
        // session answering at the state it is in now.
        let agent = cx.who.grant().is_some();
        let moved_by_others = agent && !self.seen_now();
        // The state the call finds is one a reply may name next.
        if agent {
            self.keep_seen();
        }
        let view = crate::view::Request::of(&target.request);
        if view.is_some() && cx.window.is_none() {
            return Err(crate::view::unavailable(&call.entry));
        }
        // A call naming a turn joins it, after the gate's own rules.
        let gated = self.gate(&cx.who, &target.request);
        // A change a person allowed was allowed as they saw it: it never
        // moves onto another state.
        let asked = matches!(gated, Decision::Ask(_));
        let turn = match &call.turn {
            Some(id) => {
                let admitted = self.turns.admit(id, &call.entry);
                match (&gated, admitted) {
                    (Decision::Refuse(_), _) => None,
                    (_, Ok(admitted)) => Some((id.clone(), admitted)),
                    (_, Err(error)) => return Err(error),
                }
            }
            None => None,
        };
        match gated {
            Decision::Run => {}
            Decision::Refuse(error) => return Err(Box::new(error)),
            // A dry run keeps nothing, so what would ask a person runs.
            Decision::Ask(_) if call.dry_run => {}
            Decision::Ask(question) => match cx.asker.ask(&question) {
                Answer::Allow => {}
                Answer::Deny => return Err(Box::new(question.denied())),
                Answer::Unavailable => return Err(Box::new(question.unanswered())),
            },
        }
        if let Some(window) = &cx.window
            && let Some(refused) = busy(
                &**window,
                target.request.subject(),
                target.request.effective_class(),
                call.dry_run,
            )
        {
            return Err(refused);
        }
        // An agent places symbols on the page, where the person sees them;
        // a person may draw beside it. A sheet the batch adds is the core's
        // to check.
        if cx.who.grant().is_some() {
            let project = self.engine().project();
            for new in target.request.placements() {
                let sheet = new.sheet.unwrap_or_else(|| project.sheets.first());
                if let Some(position) = new.position
                    && project.sheet(sheet).is_some()
                {
                    sparksmith_core::schematic::spot::check_on_page(project, sheet, position)
                        .map_err(|error| Box::new(from_diagnostic(error.diagnostic())))?;
                }
            }
        }
        if let Some(view) = view {
            if call.dry_run {
                return Err(malformed(
                    &call.entry,
                    "it changes only the window, so it has no dry run",
                ));
            }
            let reply = crate::view::run(
                cx.window,
                self.engine(),
                self.lent_workflows(),
                &call.entry,
                view,
            )?;
            #[cfg(test)]
            reply_contract_tests::check(&call.entry, &reply);
            return Ok(reply);
        }
        // An edit prepared before the person's latest changes goes on top
        // of them unless both change the same objects.
        let onto = self.engine().revision();
        let (target, rebased) = if call.dry_run {
            (target, None)
        } else {
            let Target { request, expected } = target;
            let (request, expected, rebased) =
                self.rebase(moved_by_others && !asked, request, expected)?;
            (Target { request, expected }, rebased)
        };
        // A turn's edits join its one undo step (a dry run keeps nothing).
        let grouped = match &turn {
            Some((_, admitted)) if !call.dry_run => {
                let opened = match admitted.mark {
                    Some(mark) => self.engine_mut().continue_group(&admitted.label, mark),
                    None => self.engine_mut().begin_group(&admitted.label),
                };
                opened.map_err(|error| Box::new(from_diagnostic(error.diagnostic())))?;
                true
            }
            _ => false,
        };
        let result = if call.dry_run {
            self.rehearse_target(target)
        } else if verbose {
            self.run(target.request, target.expected)
        } else {
            self.run_compact(target.request, target.expected)
        };
        if let Some((id, admitted)) = turn {
            let mark = if grouped {
                self.engine_mut().end_group()
            } else {
                admitted.mark
            };
            self.turns.ran(&id, mark);
        }
        if agent {
            self.keep_seen();
        }
        let result = result.map(|mut reply| {
            if let Some(from) = rebased {
                crate::session::rebase::note(&mut reply, from, onto);
            }
            #[cfg(test)]
            {
                if let Some(name) = subject.filter(|name| *name != call.entry) {
                    reply_contract_tests::check(name, &reply);
                }
                reply_contract_tests::check(&call.entry, &reply);
            }
            reply
        });
        result.map_err(|error| match error {
            crate::Error::Refused(error) => error,
            error => Box::new(from_diagnostic(error.diagnostic())),
        })
    }
}

impl Session {
    /// The gate for `request` from `who`, against what is on disk: a link
    /// inside an allowed folder cannot lead out of it. `project.save`
    /// without a path counts as naming the file it would overwrite.
    pub(crate) fn gate(&self, who: &Principal<'_>, request: &Request) -> Decision {
        let Some(grant) = who.grant() else {
            return Decision::Run;
        };
        let saved;
        let request = match (request, self.path()) {
            (Request::Save(params), Some(current)) if params.path.is_none() => {
                saved = Request::Save(crate::session::SaveParams {
                    path: Some(current.display().to_string().into()),
                });
                &saved
            }
            (request, _) => request,
        };
        let Ok(paths) = request.paths() else {
            return Decision::Refuse(gate::unreadable(request.subject()));
        };
        let subjects: Vec<Subject<'_>> = request.subjects();
        let folders: Vec<PathBuf> = grant
            .folders
            .iter()
            .filter_map(|f| gate::lexical(&f.to_string_lossy()).and_then(|f| gate::real_path(&f)))
            .collect();
        let inside = |path: &str| {
            gate::lexical(path)
                .and_then(|path| gate::real_path(&path))
                .is_some_and(|path| folders.iter().any(|folder| path.starts_with(folder)))
        };
        gate::decide_entries(who, &subjects, &paths, &inside)
    }
}

/// Entries that only put a card in the chat, which the busy rule lets
/// through.
const CHAT_CARDS: [&str; 2] = ["assist.ask", "assist.offer"];

/// The busy rule: while the person holds `window` (typing, dragging, a
/// menu, dialog or review), a call of `class` other than a read or a dry
/// run is refused with `live.busy`, naming why; a chat card never is.
pub fn busy(
    window: &dyn WindowHost,
    subject: &str,
    class: sparksmith_core::Class,
    dry_run: bool,
) -> Option<Box<Error>> {
    // A card in the chat (a question, an offer) changes neither the
    // document nor what the person is working on.
    if class == sparksmith_core::Class::Read || dry_run || CHAT_CARDS.contains(&subject) {
        return None;
    }
    window.busy().map(|reason| {
        Box::new(gate::refusal(
            "live.busy",
            subject,
            &format!("{reason}. Nothing was changed."),
            "Wait a moment and send it again; check the revision first, as the person may have changed the project.",
        ))
    })
}

/// `{"command", "params"}` (with `verbose` beside `command` or inside
/// `params`), the shape the JSON-lines and MCP transports send, as a call,
/// with whether it asked for the whole document state. The revision a
/// document change's parameters name moves to the envelope.
pub fn from_request(mut request: Value) -> Result<(Call, bool), Box<Error>> {
    let verbose = take_verbose(&mut request);
    let Value::Object(mut object) = request else {
        return Err(malformed(
            "request",
            "a request is an object {command, params}",
        ));
    };
    let entry = match object.remove("command") {
        Some(Value::String(entry)) => entry,
        _ => return Err(malformed("request", "it names no command")),
    };
    if let Some(key) = object.keys().find(|key| key.as_str() != "params") {
        return Err(malformed(&entry, &format!("unknown field `{key}`")));
    }
    let mut params = object.remove("params").unwrap_or_else(|| json!({}));
    let mut expected_revision = None;
    if metadata::find(&entry).is_some_and(|meta| meta.revision)
        && let Some(object) = params.as_object_mut()
        && let Some(revision) = object.get("expected_revision").and_then(Value::as_u64)
    {
        object.remove("expected_revision");
        expected_revision = Some(revision);
    }
    let call = Call {
        entry,
        params,
        project: None,
        expected_revision,
        dry_run: false,
        confirm: None,
        turn: None,
        job: false,
    };
    Ok((call, verbose))
}

/// `request` as a call ([`from_request`] of its JSON form).
pub fn call_of(request: &Request) -> Result<Call, Box<Error>> {
    let value =
        serde_json::to_value(request).map_err(|e| malformed(request.subject(), &e.to_string()))?;
    from_request(value).map(|(call, _)| call)
}

/// The one error from a diagnostic of the engine, a file or the 3D model.
pub fn from_diagnostic(diagnostic: Diagnostic) -> Error {
    Error {
        code: diagnostic.code,
        message: diagnostic.message,
        rule: diagnostic.rule,
        expected_revision: diagnostic.expected_revision,
        actual_revision: diagnostic.actual_revision,
        ..Error::default()
    }
}

/// The transports' envelope: `{"ok": true, "result": …}` or
/// `{"ok": false, "error": …}`.
pub fn envelope(result: Result<Value, Box<Error>>) -> Value {
    match result {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(error) => gate::failure(&error),
    }
}

/// Read `call` against the catalog: the request that runs it and the
/// revision it expects. An unknown entry, malformed parameters and a
/// missing or doubled revision are refused before anything runs.
pub(crate) fn read(call: &Call) -> Result<Target, Box<Error>> {
    let name = call.entry.as_str();
    let Some(meta) = metadata::find(name) else {
        return Err(unknown(name));
    };
    let params = match &call.params {
        Value::Null => Value::Object(Map::new()),
        params => params.clone(),
    };
    match meta.family {
        "operation" => {
            let expected = call
                .expected_revision
                .ok_or_else(|| revision_missing(name))?;
            let Some(crate::session::discovery::Call::Operation(value)) =
                crate::session::discovery::call(name, params)
            else {
                return Err(unknown(name));
            };
            let operation: Operation =
                serde_json::from_value(value).map_err(|e| malformed(name, &e.to_string()))?;
            Ok(Target {
                request: Request::Apply(ApplyParams {
                    expected_revision: expected,
                    operation,
                }),
                expected: Some(expected),
            })
        }
        "query" => {
            let Some(crate::session::discovery::Call::Query(value)) =
                crate::session::discovery::call(name, params)
            else {
                return Err(unknown(name));
            };
            let query: Query =
                serde_json::from_value(value).map_err(|e| malformed(name, &e.to_string()))?;
            Ok(Target {
                request: Request::Query(query),
                expected: None,
            })
        }
        _ => {
            let Value::Object(mut object) = params else {
                return Err(malformed(name, "its params are an object"));
            };
            let expected = if meta.revision {
                let given = object.get("expected_revision").cloned();
                match (call.expected_revision, given) {
                    (Some(revision), None) => {
                        object.insert("expected_revision".into(), json!(revision));
                        Some(revision)
                    }
                    (Some(revision), Some(given)) if given == json!(revision) => Some(revision),
                    (Some(_), Some(_)) => {
                        return Err(malformed(
                            name,
                            "expected_revision is given twice, with different values",
                        ));
                    }
                    (None, Some(given)) => given.as_u64(),
                    (None, None) => return Err(revision_missing(name)),
                }
            } else {
                None
            };
            let request: Request =
                serde_json::from_value(json!({"command": name, "params": object}))
                    .map_err(|e| malformed(name, &e.to_string()))?;
            Ok(Target { request, expected })
        }
    }
}

/// Removes `verbose` from a call's parameters: whether it asked for it.
fn take_verbose_params(params: &mut Value) -> bool {
    params
        .as_object_mut()
        .and_then(|object| object.remove("verbose"))
        .is_some_and(|verbose| verbose == true)
}

/// The envelope fields no slice serves yet are refused, not ignored.
fn unsupported(call: &Call) -> Result<(), Box<Error>> {
    let field = if call.confirm.is_some() {
        "confirm"
    } else if call.job {
        "job"
    } else {
        return Ok(());
    };
    Err(Box::new(gate::refusal(
        "host.unsupported",
        &call.entry,
        &format!("this host does not serve {field} yet. Nothing was changed."),
        &format!("Send the call without {field}."),
    )))
}

fn unknown(name: &str) -> Box<Error> {
    let shown: String = name.chars().take(64).collect();
    let closest = metadata::closest(name, |e| e.kind.is_some());
    let instead = if closest.is_empty() {
        "project.schema without a name lists every entry.".to_owned()
    } else {
        format!(
            "Did you mean {}? project.schema without a name lists every entry.",
            closest.join(", ")
        )
    };
    Box::new(Error {
        closest: closest.into_iter().map(str::to_owned).collect(),
        ..gate::refusal(
            "catalog.unknown",
            &shown,
            "no catalog entry has this name. Nothing was changed.",
            &instead,
        )
    })
}

fn malformed(name: &str, why: &str) -> Box<Error> {
    Box::new(gate::refusal(
        "malformed_request",
        name,
        &format!("it is not a valid request: {why}. Nothing was changed."),
        "Read its schema with describe (project.schema on JSON lines) and send it again.",
    ))
}

fn revision_missing(name: &str) -> Box<Error> {
    Box::new(gate::refusal(
        "malformed_request",
        name,
        "it changes the document, so the call names the revision it was prepared against (expected_revision). Nothing was changed.",
        "Send expected_revision: the revision of your last reply, or project.inspect's.",
    ))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod reply_contract_tests;

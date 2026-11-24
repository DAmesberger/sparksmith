//! An agent's edit prepared against a revision the person has since moved
//! on from is applied on top of their edits, as a collaborator would,
//! unless both changed the same objects (`docs/extension-surface.md`,
//! the front door).
//!
//! The session keeps the last few document states it answered an agent at
//! ([`Seen`]). A document operation (`project.apply`, `project.apply_all`)
//! from a caller with a grant whose `expected_revision` is one of them,
//! while the current state is not (someone else moved the document), and
//! that no person was asked to allow, is tried on a fork at the current
//! revision. When what it changes and what
//! changed since its revision share no object (by collection and id) and no
//! field, it runs at the current revision and its reply says
//! `rebased: {from, onto}`; otherwise it is refused with
//! `revision.conflict`, naming the objects, and nothing changes. Undo,
//! redo, previews bound to a revision and any revision the session never
//! answered at stay as strict as before: a stale revision is refused.
use super::{Request, Session, reply};
use crate::front::Error;
use crate::gate;
use serde_json::{Value, json};
use sparksmith_core::Document;
use std::collections::VecDeque;

/// Document states kept to rebase onto.
const KEPT: usize = 32;
/// Objects a conflict refusal names.
const NAMED: usize = 8;

/// The document states the session answered at, newest last.
#[derive(Default)]
pub(crate) struct Seen(VecDeque<(u64, Document)>);

impl Seen {
    /// Keep `document` as the state at `revision`.
    pub(crate) fn keep(&mut self, revision: u64, document: &Document) {
        // The same state, with the name and identity a save gave it since.
        if let Some((r, kept)) = self.0.back_mut()
            && *r == revision
        {
            *kept = document.clone();
            return;
        }
        self.0.retain(|(r, _)| *r != revision);
        if self.0.len() >= KEPT {
            self.0.pop_front();
        }
        self.0.push_back((revision, document.clone()));
    }

    fn holds(&self, revision: u64) -> bool {
        self.0.iter().any(|(r, _)| *r == revision)
    }

    fn at(&self, revision: u64) -> Option<&Document> {
        self.0
            .iter()
            .find(|(r, _)| *r == revision)
            .map(|(_, document)| document)
    }
}

/// Whether `a` and `b` are states of one document: they still share a
/// section neither changed. A state of another window's document shares
/// none, and is never rebased onto.
fn related(a: &Document, b: &Document) -> bool {
    use sparksmith_core::Shared;
    let (p, q) = (&a.project, &b.project);
    std::sync::Arc::ptr_eq(&a.drafts, &b.drafts)
        || Shared::ptr_eq(&p.instances, &q.instances)
        || Shared::ptr_eq(&p.occurrences, &q.occurrences)
        || Shared::ptr_eq(&p.wires, &q.wires)
        || Shared::ptr_eq(&p.nets, &q.nets)
        || Shared::ptr_eq(&p.placements, &q.placements)
        || Shared::ptr_eq(&p.tracks, &q.tracks)
        || Shared::ptr_eq(&p.vias, &q.vias)
}

/// `a` and `b` name the same place or one holds the other.
fn overlaps(a: &str, b: &str) -> bool {
    let within = |inner: &str, outer: &str| {
        inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    };
    within(a, b) || within(b, a)
}

/// What both changed: the objects, named where they have a name, and the
/// fields.
fn shared(ours: &reply::Touched, theirs: &reply::Touched) -> Vec<String> {
    let mut both = Vec::new();
    for ((path, id), name) in &ours.objects {
        let same = theirs.objects.contains_key(&(path.clone(), id.clone()))
            || theirs.fields.iter().any(|f| overlaps(f, path));
        if same {
            let name = name.clone().or_else(|| {
                theirs
                    .objects
                    .get(&(path.clone(), id.clone()))
                    .cloned()
                    .flatten()
            });
            both.push(match name {
                Some(name) => format!("{name} ({path})"),
                None => format!("{path} {id}"),
            });
        }
    }
    for field in &ours.fields {
        let same = theirs.fields.iter().any(|f| overlaps(f, field))
            || theirs.objects.keys().any(|(path, _)| overlaps(field, path));
        if same && !both.contains(field) {
            both.push(field.clone());
        }
    }
    both
}

impl Session {
    /// Whether the session answered an agent at the current state.
    pub(crate) fn seen_now(&self) -> bool {
        self.seen.holds(self.engine.revision())
    }

    /// Keep the current state as one the session answered at.
    pub(crate) fn keep_seen(&mut self) {
        let revision = self.engine.revision();
        let document = self.engine.document().clone();
        self.seen.keep(revision, &document);
    }

    /// `request` moved onto the current revision when it was prepared
    /// against an earlier one the session answered at and changes nothing
    /// that changed since, with the revision it came from; as it is when
    /// it needs no rebase or cannot have one (the run refuses it as stale).
    /// Refused with `revision.conflict` when both changed the same objects.
    pub(crate) fn rebase(
        &mut self,
        agent: bool,
        request: Request,
        expected: Option<u64>,
    ) -> Result<(Request, Option<u64>, Option<u64>), Box<Error>> {
        let current = self.engine.revision();
        // Only fill copper stored since: the same document, nothing to rebase.
        let engine = &self.engine;
        let moved_on = |r: &u64| *r < current && !engine.still_at(*r);
        let Some(from) = expected.filter(|r| agent && moved_on(r)) else {
            return Ok((request, expected, None));
        };
        if !matches!(request, Request::Apply(_) | Request::ApplyAll(_)) {
            return Ok((request, expected, None));
        }
        let Some(then) = self.seen.at(from).cloned() else {
            return Ok((request, expected, None));
        };
        let now = self.engine.document().clone();
        if !related(&then, &now) {
            return Ok((request, expected, None));
        }
        let moved = match request {
            Request::Apply(mut params) => {
                params.expected_revision = current;
                Request::Apply(params)
            }
            Request::ApplyAll(mut params) => {
                params.expected_revision = current;
                Request::ApplyAll(params)
            }
            other => other,
        };
        let subject = moved.subject().to_owned();
        let diagnostic = |error: crate::Error| match error {
            crate::Error::Refused(error) => error,
            error => Box::new(crate::front::from_diagnostic(error.diagnostic())),
        };
        let theirs = reply::touched(&then, &now).map_err(diagnostic)?;
        // What it would change, tried on a fork: nothing here moves.
        let mut fork = self.engine.fork();
        std::mem::swap(&mut self.engine, &mut fork);
        let baseline = self.baseline.take();
        let tried = self.run(moved.clone(), Some(current));
        std::mem::swap(&mut self.engine, &mut fork);
        self.baseline = baseline;
        // It fails on its own merits: the real run says why.
        if tried.is_err() {
            return Ok((moved, Some(current), Some(from)));
        }
        let ours = reply::touched(&now, fork.document()).map_err(diagnostic)?;
        let both = shared(&ours, &theirs);
        if both.is_empty() {
            return Ok((moved, Some(current), Some(from)));
        }
        let more = both.len().saturating_sub(NAMED);
        let mut named = both.into_iter().take(NAMED).collect::<Vec<_>>().join(", ");
        if more > 0 {
            named.push_str(&format!(" and {more} more"));
        }
        Err(Box::new(Error {
            expected_revision: Some(from),
            actual_revision: Some(current),
            ..gate::refusal(
                "revision.conflict",
                &subject,
                &format!(
                    "since revision {from}, which this edit was prepared against, the project was changed to revision {current}, and those changes and this edit both change {named}. Nothing was changed."
                ),
                &format!(
                    "Read them again (project.inspect) and send the edit at revision {current}, or leave them as they are now."
                ),
            )
        }))
    }
}

/// The note a rebased edit's reply carries.
pub(crate) fn note(reply: &mut Value, from: u64, onto: u64) {
    if let Some(object) = reply.as_object_mut() {
        object.insert("rebased".into(), json!({"from": from, "onto": onto}));
    }
}

#[cfg(test)]
#[path = "rebase_tests.rs"]
mod tests;

//! Turns (`docs/extension-surface.md#turns`): a labelled group of calls
//! whose edits undo as one step, counted against per-turn limits.
//! `project.turn_begin` opens one and gives its handle; a call names it in
//! the envelope's `turn`; `project.turn_end` closes it. The edits a turn's
//! calls make join one undo step through the engine's undo group
//! ([`sparksmith_core::Engine::begin_group`]); an edit by anyone else in
//! between splits it, so undo order stays what happened.
use crate::front::Error;
use crate::gate::{self, Limits};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sparksmith_core::GroupMark;
use std::collections::VecDeque;

/// Turns a session keeps open at once; opening another closes the oldest.
pub const MAX_OPEN: usize = 16;
/// The longest label, in characters.
pub const MAX_LABEL: usize = 200;

/// `project.turn_begin`: open a turn.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnBeginParams {
    /// What the turn's one undo step reads, for people ("AI assistant:
    /// route the USB pair").
    pub label: String,
    /// At most this many calls that change the document; never above the
    /// caller's own limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<u32>,
    /// At most this many calls of any kind; never above the caller's own
    /// limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls: Option<u32>,
}

/// `project.turn_end`: close a turn.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnEndParams {
    /// The handle `project.turn_begin` gave.
    pub turn: String,
}

/// One open turn.
#[derive(Clone, Debug)]
struct Turn {
    id: String,
    label: String,
    /// The turn's own limits, below the caller's.
    changes_limit: Option<u32>,
    calls_limit: Option<u32>,
    calls: u32,
    changes: u32,
    /// Where its undo step stands in the engine's history.
    mark: Option<GroupMark>,
}

impl Turn {
    fn reply(&self) -> Value {
        json!({"turn": self.id, "label": self.label, "calls": self.calls, "changes": self.changes})
    }
}

/// The turns a session holds open.
#[derive(Debug, Default)]
pub struct Turns {
    open: VecDeque<Turn>,
    /// The number the next turn's handle takes.
    next: u64,
}

/// A call admitted to its turn: what the engine's undo group needs.
pub(crate) struct Admitted {
    pub(crate) label: String,
    pub(crate) mark: Option<GroupMark>,
}

impl Turns {
    /// `project.turn_begin`.
    pub(crate) fn begin(&mut self, params: TurnBeginParams) -> Result<Value, Box<Error>> {
        let label = params.label.trim();
        if label.is_empty() || label.chars().count() > MAX_LABEL {
            return Err(Box::new(gate::refusal(
                "malformed_request",
                "project.turn_begin",
                &format!(
                    "a turn's label is 1 to {MAX_LABEL} characters; it names the turn's undo step. Nothing was changed."
                ),
                "Send a short label that says what the turn does.",
            )));
        }
        self.next = self.next.saturating_add(1);
        let turn = Turn {
            id: format!("t_{}", self.next),
            label: label.to_owned(),
            changes_limit: params.changes,
            calls_limit: params.calls,
            calls: 0,
            changes: 0,
            mark: None,
        };
        let reply = turn.reply();
        if self.open.len() >= MAX_OPEN {
            self.open.pop_front();
        }
        self.open.push_back(turn);
        Ok(reply)
    }

    /// `project.turn_end`: what the turn did.
    pub(crate) fn end(&mut self, params: &TurnEndParams) -> Result<Value, Box<Error>> {
        let at = self.index(&params.turn, "project.turn_end")?;
        let turn = self
            .open
            .remove(at)
            .ok_or_else(|| unknown(&params.turn, "project.turn_end"))?;
        Ok(turn.reply())
    }

    /// Admit a call of `entry` to the turn `id`, as one more call and, when
    /// it can change the document, one more change, under the lower of the
    /// turn's limits and `limits` (the caller's; `None` for the person and
    /// the operator). A turn past a limit refuses the call
    /// (`gate.turn_limit`), counting it.
    pub(crate) fn admit(
        &mut self,
        id: &str,
        entry: &str,
        changes: bool,
        limits: Option<&Limits>,
    ) -> Result<Admitted, Box<Error>> {
        let at = self.index(id, entry)?;
        let turn = self.open.get_mut(at).ok_or_else(|| unknown(id, entry))?;
        let lowest = |own: Option<u32>, caller: Option<u32>| match (own, caller) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let calls_limit = lowest(turn.calls_limit, limits.map(|l| l.calls));
        let changes_limit = lowest(turn.changes_limit, limits.map(|l| l.changes));
        if let Some(limit) = calls_limit.filter(|limit| turn.calls >= *limit) {
            return Err(past_limit(entry, &turn.label, limit, "calls"));
        }
        turn.calls = turn.calls.saturating_add(1);
        if changes && let Some(limit) = changes_limit.filter(|limit| turn.changes >= *limit) {
            return Err(past_limit(entry, &turn.label, limit, "changes"));
        }
        Ok(Admitted {
            label: turn.label.clone(),
            mark: turn.mark,
        })
    }

    /// After a call of turn `id` ran: where its undo step now stands and
    /// whether it changed the document.
    pub(crate) fn ran(&mut self, id: &str, mark: Option<GroupMark>, changed: bool) {
        if let Some(turn) = self.open.iter_mut().find(|turn| turn.id == id) {
            turn.mark = mark;
            if changed {
                turn.changes = turn.changes.saturating_add(1);
            }
        }
    }

    fn index(&self, id: &str, entry: &str) -> Result<usize, Box<Error>> {
        self.open
            .iter()
            .position(|turn| turn.id == id)
            .ok_or_else(|| unknown(id, entry))
    }
}

fn unknown(id: &str, entry: &str) -> Box<Error> {
    let shown: String = id.chars().take(32).collect();
    Box::new(gate::refusal(
        "turn.unknown",
        entry,
        &format!(
            "no open turn is {shown}: it was never opened, has ended, or was closed as the oldest of {MAX_OPEN} open turns. Nothing was changed."
        ),
        "Open a turn with project.turn_begin and send its handle, or send the call without turn.",
    ))
}

fn past_limit(entry: &str, label: &str, limit: u32, what: &str) -> Box<Error> {
    Box::new(gate::refusal(
        "gate.turn_limit",
        entry,
        &format!(
            "the turn \"{label}\" has made its {limit} {what}, the most its limit allows, so it stops here. Nothing was changed."
        ),
        "End the turn (project.turn_end) and tell the user what is left to do; they decide whether to go on.",
    ))
}

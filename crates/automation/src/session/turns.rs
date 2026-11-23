//! Turns (`docs/extension-surface.md#turns`): a labelled group of calls
//! whose edits undo as one step.
//! `project.turn_begin` opens one and gives its handle; a call names it in
//! the envelope's `turn`; `project.turn_end` closes it. The edits a turn's
//! calls make join one undo step through the engine's undo group
//! ([`sparksmith_core::Engine::begin_group`]); an edit by anyone else in
//! between splits it, so undo order stays what happened.
use crate::front::Error;
use crate::gate;
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
}

/// `project.turn_end`: close a turn.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnEndParams {
    /// The handle `project.turn_begin` gave.
    pub turn: String,
}

/// One open turn.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub(super) struct Turn {
    #[serde(rename = "turn")]
    id: String,
    label: String,
    /// Where its undo step stands in the engine's history.
    #[serde(skip)]
    mark: Option<GroupMark>,
}

impl Turn {
    fn reply(&self) -> Value {
        json!(self)
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
            mark: None,
        };
        let reply = turn.reply();
        if self.open.len() >= MAX_OPEN {
            self.open.pop_front();
        }
        self.open.push_back(turn);
        Ok(reply)
    }

    /// `project.turn_end`: the turn closed.
    pub(crate) fn end(&mut self, params: &TurnEndParams) -> Result<Value, Box<Error>> {
        let at = self.index(&params.turn, "project.turn_end")?;
        let turn = self
            .open
            .remove(at)
            .ok_or_else(|| unknown(&params.turn, "project.turn_end"))?;
        Ok(turn.reply())
    }

    /// Admit a call of `entry` to the turn `id`.
    pub(crate) fn admit(&self, id: &str, entry: &str) -> Result<Admitted, Box<Error>> {
        let at = self.index(id, entry)?;
        let turn = self.open.get(at).ok_or_else(|| unknown(id, entry))?;
        Ok(Admitted {
            label: turn.label.clone(),
            mark: turn.mark,
        })
    }

    /// After a call of turn `id` ran: where its undo step now stands.
    pub(crate) fn ran(&mut self, id: &str, mark: Option<GroupMark>) {
        if let Some(turn) = self.open.iter_mut().find(|turn| turn.id == id) {
            turn.mark = mark;
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

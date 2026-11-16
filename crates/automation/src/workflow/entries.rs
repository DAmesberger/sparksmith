//! The `assist.*` entries a workflow run uses: their parameters and
//! replies (`docs/assistant-panel.md` §3.1, §2.3). This is the signature
//! only. The entries join the catalog, in the `assist` module, with the
//! runner (W1) and the built-in agent (S6), declared there with
//! `declare_entries!` like every other entry:
//!
//! | Entry | Kind, class | Parameters | Reply |
//! |---|---|---|---|
//! | `assist.workflows` | query, read | [`WorkflowsParams`] | [`WorkflowsReply`] |
//! | `assist.offer` | show, show (a window; headless `host.unavailable`) | [`OfferParams`] | [`OfferReply`] |
//! | `assist.step_result` | query, read (changes no document) | [`StepResultParams`] | [`StepResultReply`] |
use super::{InputType, Problem};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The most workflows `assist.workflows` lists by default.
pub const DEFAULT_LIMIT: u32 = 20;
/// The most it lists at all.
pub const MAX_LIMIT: u32 = 100;
/// The longest search text and `why`, in bytes.
pub const MAX_QUERY: usize = 400;

/// `assist.workflows`: the workflows this host has, ranked by `text`
/// against their title, summary and intents (the same fields as the
/// chat's index).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkflowsParams {
    /// What the person asked; every workflow when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// At most this many ([`DEFAULT_LIMIT`] when absent, at most
    /// [`MAX_LIMIT`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Also list those that cannot run here, with the reason.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unavailable: bool,
}

/// The reply of `assist.workflows`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WorkflowsReply {
    pub workflows: Vec<WorkflowSummary>,
}

/// One workflow as listed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WorkflowSummary {
    pub id: String,
    pub version: String,
    pub title: String,
    pub summary: String,
    pub intents: Vec<String>,
    /// `shipped` or `user`.
    pub source: super::Source,
    /// Its inputs, so an offer can fill what the conversation already has.
    pub inputs: Vec<InputSummary>,
    /// Empty when it can run here; else why not, each reason naming what
    /// is missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub why_not: Vec<String>,
}

/// One input as listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InputSummary {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: InputType,
    pub title: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// `assist.offer`: show the workflow card with the inputs the model
/// already has. Changes no document; the person answers on the card (Use
/// it, Just chat), and the chat tells the model which.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfferParams {
    /// The workflow's id.
    pub workflow: String,
    /// Inputs already known, by name; each is checked against its input.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub inputs: Map<String, Value>,
    /// One sentence on why it fits, shown on the card.
    pub why: String,
}

/// The reply of `assist.offer`: the card is shown. The person's answer
/// arrives as the next message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OfferReply {
    /// The card's id in the conversation.
    pub card: String,
    /// The required inputs the card will still ask for.
    pub missing: Vec<String>,
}

/// `assist.step_result`: a model step's answer. It ends the step when the
/// answer fits the step's `output`; otherwise the problems name each field
/// and the step goes on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepResultParams {
    /// The step's id, as the step's goal names it.
    pub step: String,
    /// The answer: an object with the fields of the step's output.
    pub result: Value,
}

/// The reply of `assist.step_result`, in the shape of
/// `library.review_check`'s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StepResultReply {
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<FieldProblem>,
    /// An answer that would pass, when the workflow gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Value>,
}

/// One problem with an answer: the field, as a pointer into it, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FieldProblem {
    pub at: String,
    pub message: String,
}

impl From<Problem> for FieldProblem {
    fn from(p: Problem) -> Self {
        Self {
            at: p.at,
            message: p.message,
        }
    }
}

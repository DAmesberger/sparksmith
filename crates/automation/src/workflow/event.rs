//! What the chat receives about workflows: offers and a run's progress
//! (`docs/assistant-panel.md` §3.1, §7). Text, reasoning and tool-call rows
//! are the loop's own events (L1); these are the cards and the run block
//! around them. A person's answers are [`super::control::ControlAnswer`].
use super::control::{ControlCard, Gauge};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One event of a workflow in the conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkflowEvent {
    /// A workflow card: by the model (`assist.offer`), a signal
    /// (`offer_when`), the `/` menu, the palette or another entry point.
    Offered {
        card: String,
        workflow: String,
        version: String,
        title: String,
        summary: String,
        inputs: Map<String, Value>,
        missing: Vec<String>,
        why: String,
        by: OfferedBy,
    },
    /// The person chose Accept and continue: a run starts, as one turn with this label.
    Started {
        run: String,
        workflow: String,
        version: String,
        /// `sha256:` of the workflow's files, as a recording names it.
        digest: String,
        label: String,
        steps: Vec<StepRow>,
    },
    /// A step began.
    Step {
        run: String,
        step: String,
        done: u32,
        total: u32,
    },
    /// A step ended; `summary` is its row's text ("38 pages, drawings on
    /// 31, 32").
    StepDone {
        run: String,
        step: String,
        summary: String,
    },
    /// A check sent the run back to `step` with the refusal's text.
    Retry {
        run: String,
        step: String,
        attempt: u8,
        of: u8,
        why: String,
    },
    /// A card for the person; the run waits when it takes an answer.
    Card {
        run: String,
        step: String,
        card: ControlCard,
    },
    /// Use against the run's limits.
    Meter {
        run: String,
        requests: Gauge,
        output_tokens: Gauge,
        minutes: Gauge,
    },
    /// The run ended.
    Ended { run: String, end: RunEnd },
    /// The person chose Just chat; the model is told.
    Declined { card: String },
}

/// Who put a workflow card in the conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OfferedBy {
    Model,
    Signal,
    Person,
}

/// A row of the run's step list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StepRow {
    pub id: String,
    pub title: String,
}

/// How a run ended. What completed stays as one undo step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "end", rename_all = "snake_case")]
pub enum RunEnd {
    /// The result step was reached.
    Done,
    /// The person pressed Stop.
    Stopped,
    /// The person declined a confirm gate.
    Declined { step: String },
    /// A limit was reached; which.
    Limit { why: String },
    /// A step failed past its retries, or the connection failed.
    Failed { step: String, why: String },
}

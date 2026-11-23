//! JSON-lines records shared by model recording, offline replay and the developer bench.
//! Model requests are represented by their byte count. Replies are backend-neutral.
use super::{ModelReply, WorkflowMark};
use serde::{Deserialize, Serialize};
use sparksmith_automation::mcp::trace::Call;
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub model_requests: usize,
    pub model_request_bytes: usize,
    pub estimated_prompt_tokens: u64,
    #[serde(default)]
    pub estimated_output_tokens: u64,
    pub model_ms: f64,
    pub calls: BTreeMap<String, usize>,
    pub refusals: usize,
    pub retries: usize,
    pub server_ms: f64,
    pub visible_reply_bytes: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Row {
    Start {
        workflow: WorkflowMark,
        runner: String,
        detail: String,
    },
    Model {
        bytes: usize,
        ms: f64,
        reply: ModelReply,
    },
    ModelError {
        bytes: usize,
        ms: f64,
    },
    Call(Call),
    Finish {
        metrics: Metrics,
        usage: Option<crate::usage::Usage>,
        success: bool,
        reason: Option<String>,
        wall_ms: f64,
        revision: u64,
        saved_sha256: String,
    },
}

impl Row {
    /// Preserve the position of a failed attempt without storing the
    /// provider's error body, which can contain request details.
    pub fn model(bytes: usize, ms: f64, result: &crate::Result<ModelReply>) -> Self {
        match result {
            Ok(reply) => Self::Model {
                bytes,
                ms,
                reply: reply.clone(),
            },
            Err(_) => Self::ModelError { bytes, ms },
        }
    }
}

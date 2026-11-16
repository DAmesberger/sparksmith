//! The provider-neutral model layer: one conversation shape and one
//! interface, with every vendor's wire format behind a backend.
//!
//! The agent run only ever sees [`Message`], [`Part`], [`ToolSpec`],
//! [`ModelRequest`] and [`ModelReply`]. A backend converts them to its own
//! wire format ([`anthropic`], [`openai`]). What a model can do is declared
//! in its [`ModelProfile`], per configured model, and never inferred from a
//! vendor's name. A backend may hide a block it has to send back unchanged
//! in a part's `replay` field; nothing above the backends reads it.
//!
//! Recordings ([`record`]) hold neutral replies, so a recording made on one
//! backend replays without any network, on any backend.
//!
//! A run streams ([`ModelBackend::stream`]): the backend hands the chat
//! [`Delta`]s as they arrive and checks between them whether the person
//! pressed Stop; the reply it returns is the one a whole response would
//! have given.
pub mod anthropic;
pub mod endpoint;
pub mod keys;
pub mod openai;
pub mod profile;
pub mod record;
pub mod stream;
pub mod wire;

use crate::Result;
use crate::usage::Usage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub use endpoint::Endpoint;
pub use keys::{KeySource, Secret};
pub use profile::{Budget, Capabilities, Connection, ModelProfile, OutputLimits, Provider};
pub use stream::Delta;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// An image, base64 encoded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub media_type: String,
    pub data: String,
}

/// How a document reaches the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentSource {
    /// The PDF itself, base64 encoded (models that read PDFs natively).
    Pdf { data: String },
    /// A URL the provider fetches (models that read PDFs natively).
    Url { url: String },
    /// Text taken from the PDF on this computer.
    Text { text: String },
}

/// One piece of a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
        /// The block exactly as received, when the neutral fields cannot
        /// rebuild it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replay: Option<Value>,
    },
    /// What the model reasoned before answering. It counts against the
    /// output limit and is never an answer.
    Reasoning {
        text: String,
        /// A backend's block to send back unchanged (signed thinking).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replay: Option<Value>,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
        /// Why `input` could not be used: the call is answered with this
        /// error and never run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        malformed: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replay: Option<Value>,
    },
    /// A backend's block with no neutral meaning, kept so that it can be
    /// sent back in place.
    Opaque {
        replay: Value,
    },
    ToolResult {
        call_id: String,
        text: String,
        #[serde(default)]
        error: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<Image>,
    },
    Image {
        image: Image,
    },
    Document {
        title: String,
        /// What the document is to the model ("untrusted data").
        context: String,
        source: DocumentSource,
    },
}

impl Part {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            replay: None,
        }
    }

    /// A rough count of the characters this part adds to a conversation, for
    /// the next request's prompt estimate.
    fn size_hint(&self) -> usize {
        match self {
            Self::Text { text, .. } => text.len(),
            Self::ToolCall { input, .. } => input.to_string().len(),
            Self::ToolResult { text, images, .. } => {
                text.len() + images.len() * IMAGE_SIZE_HINT_CHARS
            }
            Self::Image { .. } => IMAGE_SIZE_HINT_CHARS,
            Self::Reasoning { .. } | Self::Opaque { .. } | Self::Document { .. } => 0,
        }
    }
}

/// An image counted as text, for estimates only: about 2 000 tokens.
const IMAGE_SIZE_HINT_CHARS: usize = 6_000;

/// Characters `parts` add to a conversation, estimated.
pub fn appended_chars(parts: &[Part]) -> u64 {
    parts.iter().map(|p| p.size_hint() as u64).sum()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Part>,
}

/// A tool the model may call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON schema of the input.
    pub input_schema: Value,
}

/// One model request. A backend sends this and nothing else; the history is
/// append-only, so earlier turns reach the model unchanged.
#[derive(Clone, Copy, Debug)]
pub struct ModelRequest<'a> {
    pub system: &'a str,
    pub tools: &'a [ToolSpec],
    pub messages: &'a [Message],
    pub max_output_tokens: u32,
    /// Longest wait for this response.
    pub timeout: Duration,
}

/// Why a reply ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopReason {
    ToolCalls,
    EndTurn,
    /// Stopped at the output limit: whatever came is incomplete, and it is
    /// never treated as an answer.
    CutOff,
    Refusal {
        category: String,
    },
    Other {
        reason: String,
    },
    /// The person pressed Stop while the reply arrived: whatever came is
    /// incomplete, and it is never used.
    Stopped,
}

impl StopReason {
    /// The label in progress lines and the review's Requests table.
    pub fn label(&self) -> String {
        match self {
            Self::ToolCalls => "tool_calls".into(),
            Self::EndTurn => "end_turn".into(),
            Self::CutOff => "cut_off".into(),
            Self::Refusal { .. } => "refusal".into(),
            Self::Stopped => "stopped".into(),
            Self::Other { reason } => reason.clone(),
        }
    }
}

/// A model's answer in neutral form; the format of a recording's lines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelReply {
    /// The model that served it, which may be a fallback.
    pub model: String,
    pub content: Vec<Part>,
    pub stop: StopReason,
    pub usage: Usage,
    /// The server gave no token counts, so `usage` is an estimate from the
    /// characters.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub usage_estimated: bool,
}

/// A connected model.
pub trait ModelBackend {
    /// Where replies come from, for the review report. Never holds a key.
    fn label(&self) -> String;
    /// Send one request; the reply, or a transport or API error.
    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply>;
    /// Send one request and give its pieces to `deltas` as they arrive,
    /// checking `cancel` between them: once it is set, the reply ends with
    /// [`StopReason::Stopped`]. Otherwise the reply is the one
    /// [`Self::send`] gives. A backend that cannot stream sends, then gives
    /// the whole reply's pieces.
    fn stream(
        &mut self,
        request: &ModelRequest<'_>,
        deltas: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<ModelReply> {
        let _ = cancel;
        let reply = self.send(request)?;
        stream::deltas_of(&reply, deltas);
        Ok(reply)
    }
    /// The prompt tokens a request would take, when the backend can count.
    fn count_prompt_tokens(&mut self, request: &ModelRequest<'_>) -> Result<Option<u64>>;
    /// Whether the replies call the tools a run offered before the six
    /// verbs (a recording made before ABI S6): the run reads each such call
    /// as its verb call ([`crate::legacy`]). Only a replay says yes.
    fn legacy_tools(&self) -> bool {
        false
    }
    /// The workflow a recording was made with, when this replays one: a
    /// workflow run refuses a replay of another workflow or version.
    fn recorded_workflow(&self) -> Option<&record::WorkflowMark> {
        None
    }
}

/// Connect to the configured model: the provider's backend over HTTP, with
/// the key this connection is allowed. Refuses a key over plain HTTP to
/// another computer.
pub fn connect(profile: &ModelProfile, connection: &Connection) -> Result<Box<dyn ModelBackend>> {
    let key = connection.key.resolve(&connection.endpoint)?;
    let auth = profile.provider.auth_style();
    let http = wire::HttpWire::new(connection.endpoint.clone(), key, auth)?;
    Ok(match profile.provider {
        Provider::Anthropic => Box::new(anthropic::AnthropicBackend::new(profile.clone(), http)),
        Provider::OpenAiCompatible => Box::new(openai::OpenAiBackend::new(profile.clone(), http)),
    })
}

/// Output tokens [`check_connection`]'s one request may take.
pub const CHECK_OUTPUT_TOKENS: u32 = 16;
/// Longest wait for [`check_connection`]'s reply.
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);

/// Check that `endpoint` answers as `profile` with `key`: one short request
/// (no tools, no reasoning, at most [`CHECK_OUTPUT_TOKENS`] output tokens),
/// so it costs a few tokens. The model that answered, or the error, which
/// never holds the key. Refuses a key over plain HTTP to another computer.
pub fn check_connection(
    profile: &ModelProfile,
    endpoint: Endpoint,
    key: Option<Secret>,
) -> Result<String> {
    profile.check()?;
    let lean = lean(profile);
    let http = wire::HttpWire::new(endpoint, key, profile.provider.auth_style())?;
    match profile.provider {
        Provider::Anthropic => check_on(&mut anthropic::AnthropicBackend::new(lean, http)),
        Provider::OpenAiCompatible => check_on(&mut openai::OpenAiBackend::new(lean, http)),
    }
}

/// `profile` without reasoning, effort, caching or fallbacks.
fn lean(profile: &ModelProfile) -> ModelProfile {
    let mut lean = profile.clone();
    lean.capabilities.adaptive_thinking = false;
    lean.capabilities.caching = false;
    lean.capabilities.server_fallback = false;
    lean.effort = None;
    lean
}

/// [`check_connection`]'s one request on `backend`.
fn check_on(backend: &mut dyn ModelBackend) -> Result<String> {
    let messages = [Message {
        role: Role::User,
        content: vec![Part::text("Reply with OK.")],
    }];
    let reply = backend.send(&ModelRequest {
        system: "This is a connection check.",
        tools: &[],
        messages: &messages,
        max_output_tokens: CHECK_OUTPUT_TOKENS,
        timeout: CHECK_TIMEOUT,
    })?;
    Ok(reply.model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The connection check is one short request: no tools, no reasoning,
    /// no effort, a few output tokens; it names the model that answered.
    #[test]
    fn the_connection_check_sends_one_short_request() -> Result<()> {
        let reply = json!({
            "model": "served-model",
            "content": [{"type": "text", "text": "OK"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 12, "output_tokens": 1}
        });
        let profile = lean(&ModelProfile::anthropic("asked-model"));
        let mut backend =
            anthropic::AnthropicBackend::new(profile, wire::ScriptedWire::new(vec![reply]));
        assert_eq!(check_on(&mut backend)?, "served-model");
        assert_eq!(backend.wire.posts.len(), 1, "one request");
        let body = &backend.wire.posts[0].2;
        assert_eq!(body["max_tokens"], json!(CHECK_OUTPUT_TOKENS));
        assert_eq!(body["tools"], json!([]));
        for absent in ["thinking", "output_config", "cache_control", "fallbacks"] {
            assert!(body.get(absent).is_none(), "{absent} in {body}");
        }
        Ok(())
    }
}

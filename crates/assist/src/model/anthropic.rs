//! Anthropic Messages. Today's behaviour, behind the neutral interface:
//! adaptive thinking, effort, the server-side fallback beta, prompt-cache
//! breakpoints on the system prompt and the last document plus automatic
//! caching of the tail, and every assistant block sent back byte for byte.
//!
//! Replay is exact by construction: [`parse_reply`] keeps a block in its
//! part's `replay` whenever the neutral fields would not rebuild it, thinking
//! blocks always, so a later request carries what the API sent. Opus 5.5
//! binds thinking to the conversation before it, and a rewrite would also
//! re-write the cache.
//!
//! A streamed reply ([`ModelBackend::stream`]) is put back together from its
//! events as the response a whole request gives, and read by the same
//! [`parse_reply`], so it is the same reply block for block.
use super::stream::{self, Delta, SseEvent, Streamed};
use super::wire::{HttpWire, Wire};
use super::{
    DocumentSource, Message, ModelBackend, ModelProfile, ModelReply, ModelRequest, Part, Role,
    StopReason,
};
use crate::usage::Usage;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The API version every request names.
pub const API_VERSION: &str = "2026-10-21";
/// The beta that turns on server-side refusal fallback.
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Longest wait for a token count.
const COUNT_TIMEOUT: Duration = Duration::from_secs(120);

/// The Messages backend over a wire.
pub struct AnthropicBackend<W> {
    profile: ModelProfile,
    pub wire: W,
}

impl std::fmt::Debug for AnthropicBackend<HttpWire> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicBackend")
            .field("wire", &self.wire)
            .finish_non_exhaustive()
    }
}

impl<W: Wire> AnthropicBackend<W> {
    pub fn new(profile: ModelProfile, wire: W) -> Self {
        Self { profile, wire }
    }
}

/// One tool result block.
fn tool_result_block(call_id: &str, text: &str, error: bool, images: &[super::Image]) -> Value {
    let content = if images.is_empty() {
        json!(text)
    } else {
        let mut blocks = vec![json!({"type": "text", "text": text})];
        blocks.extend(images.iter().map(image_block));
        Value::Array(blocks)
    };
    json!({"type": "tool_result", "tool_use_id": call_id, "content": content, "is_error": error})
}

fn image_block(image: &super::Image) -> Value {
    json!({"type": "image", "source": {
        "type": "base64", "media_type": image.media_type, "data": image.data,
    }})
}

/// The wire block of a part, the kept block when there is one.
fn block(part: &Part) -> Value {
    match part {
        Part::Text {
            replay: Some(kept), ..
        }
        | Part::Reasoning {
            replay: Some(kept), ..
        }
        | Part::ToolCall {
            replay: Some(kept), ..
        }
        | Part::Opaque { replay: kept } => kept.clone(),
        Part::Text { text, .. } => json!({"type": "text", "text": text}),
        Part::Reasoning { text, .. } => json!({"type": "thinking", "thinking": text}),
        Part::ToolCall {
            id, name, input, ..
        } => json!({"type": "tool_use", "id": id, "name": name, "input": input}),
        Part::ToolResult {
            call_id,
            text,
            error,
            images,
        } => tool_result_block(call_id, text, *error, images),
        Part::Image { image } => image_block(image),
        Part::Document {
            title,
            context,
            source,
        } => {
            let source = match source {
                DocumentSource::Pdf { data } => {
                    json!({"type": "base64", "media_type": "application/pdf", "data": data})
                }
                DocumentSource::Url { url } => json!({"type": "url", "url": url}),
                DocumentSource::Text { text } => {
                    json!({"type": "text", "media_type": "text/plain", "data": text})
                }
            };
            json!({"type": "document", "source": source, "title": title, "context": context})
        }
    }
}

fn wire_message(message: &Message, caching: bool) -> Value {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    // Reasoning without a kept block came from elsewhere and has no signature,
    // so there is nothing to send back.
    let mut content: Vec<Value> = message
        .content
        .iter()
        .filter(|p| !matches!(p, Part::Reasoning { replay: None, .. }))
        .map(block)
        .collect();
    if caching {
        // The last document's breakpoint: every later request reads the
        // datasheets from the cache.
        if let Some(last) = content.iter_mut().rev().find(|b| b["type"] == "document") {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
    }
    json!({"role": role, "content": content})
}

/// The Messages request body of one request.
pub fn request_body(profile: &ModelProfile, request: &ModelRequest<'_>) -> Value {
    let caps = &profile.capabilities;
    let mut system = json!({"type": "text", "text": request.system});
    if caps.caching {
        system["cache_control"] = json!({"type": "ephemeral"});
    }
    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema}))
        .collect();
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|m| wire_message(m, caps.caching))
        .collect();
    let mut body = json!({
        "model": profile.model,
        "max_tokens": request.max_output_tokens,
        "system": [system],
        "tools": tools,
        "messages": messages,
    });
    if caps.adaptive_thinking {
        body["thinking"] = json!({"type": "adaptive"});
    }
    if let Some(effort) = &profile.effort {
        body["output_config"] = json!({"effort": effort});
    }
    if caps.caching {
        // Automatic caching of the growing conversation tail.
        body["cache_control"] = json!({"type": "ephemeral"});
    }
    if caps.server_fallback {
        body["fallbacks"] = json!("default");
    }
    body
}

/// The neutral part of one response block. A block the neutral fields rebuild
/// exactly carries nothing extra, so a recording stays readable.
fn parse_block(wire: &Value) -> Part {
    let text = || wire["text"].as_str().unwrap_or_default().to_owned();
    let mut part = match wire["type"].as_str() {
        Some("text") => Part::Text {
            text: text(),
            replay: None,
        },
        Some("thinking") => Part::Reasoning {
            text: wire["thinking"].as_str().unwrap_or_default().to_owned(),
            replay: Some(wire.clone()),
        },
        Some("redacted_thinking") => Part::Reasoning {
            text: String::new(),
            replay: Some(wire.clone()),
        },
        Some("tool_use") => {
            let input = wire["input"].clone();
            let malformed = (!input.is_object()).then(|| "the tool input is not an object".into());
            Part::ToolCall {
                id: wire["id"].as_str().unwrap_or_default().to_owned(),
                name: wire["name"].as_str().unwrap_or_default().to_owned(),
                input,
                malformed,
                replay: None,
            }
        }
        _ => Part::Opaque {
            replay: wire.clone(),
        },
    };
    if block(&part) != *wire {
        let kept = Some(wire.clone());
        match &mut part {
            Part::Text { replay, .. } | Part::ToolCall { replay, .. } => *replay = kept,
            _ => {}
        }
    }
    part
}

/// A Messages response in neutral form.
pub fn parse_reply(response: &Value, requested_model: &str) -> Result<ModelReply> {
    let Some(blocks) = response["content"].as_array() else {
        return Err(Error::Protocol("the response has no content".into()));
    };
    let stop = match response["stop_reason"].as_str().unwrap_or_default() {
        "tool_use" => StopReason::ToolCalls,
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "max_tokens" | "model_context_window_exceeded" => StopReason::CutOff,
        "refusal" => StopReason::Refusal {
            category: response["stop_details"]["category"]
                .as_str()
                .unwrap_or("no category")
                .to_owned(),
        },
        other => StopReason::Other {
            reason: other.to_owned(),
        },
    };
    let n = |k: &str| response["usage"][k].as_u64().unwrap_or(0);
    Ok(ModelReply {
        model: response["model"]
            .as_str()
            .unwrap_or(requested_model)
            .to_owned(),
        content: blocks.iter().map(parse_block).collect(),
        stop,
        usage: Usage {
            input_tokens: n("input_tokens"),
            output_tokens: n("output_tokens"),
            cache_write_tokens: n("cache_creation_input_tokens"),
            cache_read_tokens: n("cache_read_input_tokens"),
        },
        usage_estimated: false,
    })
}

/// Most content blocks one streamed reply may open; an index past it is a
/// protocol error, not an allocation.
const MAX_STREAM_BLOCKS: usize = 4096;

/// A Messages event stream put back together as the response a whole
/// request gives: the message from `message_start`, each block from its
/// start and deltas, the stop and usage from `message_delta`.
#[derive(Debug, Default)]
struct Assembly {
    message: serde_json::Map<String, Value>,
    blocks: Vec<Value>,
    /// Tool input JSON as it arrives, by block index.
    inputs: std::collections::BTreeMap<usize, String>,
    calls: u32,
    complete: bool,
}

fn object(value: &mut Value) -> Result<&mut serde_json::Map<String, Value>> {
    value
        .as_object_mut()
        .ok_or_else(|| Error::Protocol("a streamed block is not an object".into()))
}

/// Append `piece` to the string at `key` of `block`.
fn append(block: &mut Value, key: &str, piece: &str) -> Result<()> {
    let slot = object(block)?
        .entry(key)
        .or_insert_with(|| Value::String(String::new()));
    match slot {
        Value::String(text) => text.push_str(piece),
        other => *other = Value::String(piece.to_owned()),
    }
    Ok(())
}

impl Assembly {
    fn index(value: &Value) -> Result<usize> {
        value["index"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < MAX_STREAM_BLOCKS)
            .ok_or_else(|| Error::Protocol(format!("a stream event has no usable index: {value}")))
    }

    fn event(&mut self, event: &SseEvent, deltas: &mut dyn FnMut(Delta)) -> Result<()> {
        if event.data.trim().is_empty() {
            return Ok(());
        }
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|e| Error::Protocol(format!("a stream event is not JSON: {e}")))?;
        let kind = value["type"].as_str().unwrap_or(event.event.as_str());
        match kind {
            "message_start" => {
                if let Some(message) = value["message"].as_object() {
                    self.message = message.clone();
                }
            }
            "content_block_start" => {
                let i = Self::index(&value)?;
                let block = value["content_block"].clone();
                if !block.is_object() {
                    return Err(Error::Protocol("a streamed block is not an object".into()));
                }
                if block.get("input").is_some() {
                    self.inputs.insert(i, String::new());
                }
                if block["type"] == "tool_use" {
                    deltas(Delta::ToolCall {
                        index: self.calls,
                        name: block["name"].as_str().unwrap_or_default().to_owned(),
                    });
                    self.calls = self.calls.saturating_add(1);
                }
                if self.blocks.len() <= i {
                    self.blocks.resize(i + 1, Value::Null);
                }
                if let Some(slot) = self.blocks.get_mut(i) {
                    *slot = block;
                }
            }
            "content_block_delta" => {
                let i = Self::index(&value)?;
                let delta = &value["delta"];
                let Some(block) = self.blocks.get_mut(i).filter(|b| b.is_object()) else {
                    return Err(Error::Protocol(format!(
                        "a delta for block {i}, which did not start"
                    )));
                };
                let text = |key: &str| delta[key].as_str().unwrap_or_default().to_owned();
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        let piece = text("text");
                        append(block, "text", &piece)?;
                        deltas(Delta::Text(piece));
                    }
                    Some("thinking_delta") => {
                        let piece = text("thinking");
                        append(block, "thinking", &piece)?;
                        deltas(Delta::Reasoning(piece));
                    }
                    Some("signature_delta") => append(block, "signature", &text("signature"))?,
                    Some("input_json_delta") => self
                        .inputs
                        .entry(i)
                        .or_default()
                        .push_str(&text("partial_json")),
                    Some("citations_delta") => {
                        let slot = object(block)?
                            .entry("citations")
                            .or_insert_with(|| Value::Array(Vec::new()));
                        if !slot.is_array() {
                            *slot = Value::Array(Vec::new());
                        }
                        if let Value::Array(list) = slot {
                            list.push(delta["citation"].clone());
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => self.close(Self::index(&value)?),
            "message_delta" => {
                for (key, v) in value["delta"].as_object().into_iter().flatten() {
                    self.message.insert(key.clone(), v.clone());
                }
                if let Some(usage) = value["usage"].as_object() {
                    let slot = self
                        .message
                        .entry("usage")
                        .or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if !slot.is_object() {
                        *slot = Value::Object(serde_json::Map::new());
                    }
                    if let Value::Object(kept) = slot {
                        for (key, v) in usage {
                            kept.insert(key.clone(), v.clone());
                        }
                    }
                }
            }
            "message_stop" => self.complete = true,
            "error" => {
                let error = &value["error"];
                let kind = error["type"].as_str().unwrap_or("error");
                let status = match kind {
                    "overloaded_error" => 529,
                    "rate_limit_error" => 429,
                    "api_error" => 500,
                    _ => 400,
                };
                return Err(Error::Api {
                    status,
                    message: format!(
                        "{kind}: {}",
                        error["message"].as_str().unwrap_or("the stream failed")
                    ),
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// A tool block's input, once all of it has arrived.
    fn close(&mut self, i: usize) {
        let (Some(json), Some(block)) = (self.inputs.remove(&i), self.blocks.get_mut(i)) else {
            return;
        };
        if json.trim().is_empty() {
            return;
        }
        let input = serde_json::from_str(&json).unwrap_or(Value::String(json));
        if let Some(block) = block.as_object_mut() {
            block.insert("input".into(), input);
        }
    }

    /// The response, as a whole request would have given it.
    fn finish(mut self) -> Value {
        let open: Vec<usize> = self.inputs.keys().copied().collect();
        for i in open {
            self.close(i);
        }
        let blocks = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        self.message.insert("content".into(), Value::Array(blocks));
        Value::Object(self.message)
    }
}

impl<W: Wire> AnthropicBackend<W> {
    fn headers(body: &Value) -> Vec<(&'static str, &'static str)> {
        let mut headers = vec![("anthropic-version", API_VERSION)];
        if body.get("fallbacks").is_some() {
            headers.push(("anthropic-beta", FALLBACK_BETA));
        }
        headers
    }
}

impl<W: Wire> ModelBackend for AnthropicBackend<W> {
    fn label(&self) -> String {
        format!("Anthropic Messages at {}", self.wire.label())
    }

    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        let body = request_body(&self.profile, request);
        let response = self.wire.post(
            "/v1/messages",
            &Self::headers(&body),
            &body,
            request.timeout,
        )?;
        self.wire
            .clean_reply(parse_reply(&response, &self.profile.model)?)
    }

    fn stream(
        &mut self,
        request: &ModelRequest<'_>,
        deltas: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<ModelReply> {
        let mut redactor = self.wire.redactor();
        let mut emit = |delta| redactor.push(delta, deltas);
        let mut body = request_body(&self.profile, request);
        if let Some(fields) = body.as_object_mut() {
            fields.insert("stream".into(), Value::Bool(true));
        }
        let mut assembly = Assembly::default();
        let streamed = self.wire.post_stream(
            "/v1/messages",
            &Self::headers(&body),
            &body,
            request.timeout,
            &mut |event| {
                assembly.event(&event, &mut emit)?;
                Ok(!cancel.load(Ordering::Relaxed))
            },
        )?;
        let stopped = match streamed {
            Streamed::Whole(response) => {
                let reply = self
                    .wire
                    .clean_reply(parse_reply(&response, &self.profile.model)?)?;
                stream::deltas_of(&reply, &mut emit);
                redactor.finish(true, deltas);
                return Ok(reply);
            }
            Streamed::Stopped => true,
            Streamed::Done if assembly.complete => false,
            Streamed::Done => {
                return Err(Error::Protocol(
                    "the stream ended before the reply was complete".into(),
                ));
            }
        };
        let response = self.wire.clean_value(assembly.finish());
        let mut reply = parse_reply(&response, &self.profile.model)?;
        if stopped {
            reply.stop = StopReason::Stopped;
        }
        redactor.finish(!stopped, deltas);
        self.wire.clean_reply(reply)
    }

    fn count_prompt_tokens(&mut self, request: &ModelRequest<'_>) -> Result<Option<u64>> {
        // The count endpoint takes the prompt only.
        let body = request_body(&self.profile, request);
        let mut prompt = json!({});
        for key in ["model", "system", "tools", "messages", "thinking"] {
            if let Some(v) = body.get(key) {
                prompt[key] = v.clone();
            }
        }
        let reply = self.wire.post(
            "/v1/messages/count_tokens",
            &[("anthropic-version", API_VERSION)],
            &prompt,
            COUNT_TIMEOUT,
        )?;
        Ok(reply["input_tokens"].as_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::super::wire::ScriptedWire;
    use super::super::{Image, ToolSpec};
    use super::*;

    /// A model whose run profile declares the server fallback.
    fn profile() -> ModelProfile {
        let mut p = ModelProfile::anthropic("claude-opus-5-5");
        p.capabilities.server_fallback = true;
        p
    }

    const THINKING: &str = r#"{"type": "thinking", "thinking": "", "signature": "CAQS6g=="}"#;

    #[test]
    fn every_block_round_trips_byte_for_byte() -> Result<()> {
        let blocks: Value = serde_json::from_str(&format!(
            r#"[{THINKING},
            {{"type": "redacted_thinking", "data": "abc"}},
            {{"type": "text", "text": "Calling.", "citations": null}},
            {{"type": "text", "text": "plain"}},
            {{"type": "tool_use", "id": "t1", "name": "x", "input": {{"a": [1, 2]}}}},
            {{"type": "tool_use", "id": "t2", "name": "y", "input": {{}}, "caller": {{"type": "direct"}}}},
            {{"type": "server_tool_use", "id": "s1", "name": "z", "input": {{}}}}]"#
        ))?;
        let reply = parse_reply(
            &json!({"content": blocks, "stop_reason": "tool_use", "model": "m"}),
            "m",
        )?;
        let rebuilt: Vec<Value> = reply.content.iter().map(block).collect();
        assert_eq!(Value::Array(rebuilt), blocks);
        // Only what the neutral fields cannot rebuild is kept twice.
        assert!(matches!(&reply.content[3], Part::Text { replay: None, .. }));
        assert!(matches!(
            &reply.content[4],
            Part::ToolCall { replay: None, .. }
        ));
        assert!(matches!(
            &reply.content[0],
            Part::Reasoning {
                replay: Some(_),
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn the_neutral_recording_keeps_thinking_for_replay() -> Result<()> {
        // A thinking block survives a recording's round trip unchanged.
        let thinking: Value = serde_json::from_str(THINKING)?;
        let reply = parse_reply(
            &json!({"content": [thinking.clone()], "stop_reason": "end_turn", "model": "m"}),
            "m",
        )?;
        let line = serde_json::to_string(&reply)?;
        let back: ModelReply = serde_json::from_str(&line)?;
        assert_eq!(block(&back.content[0]), thinking);
        Ok(())
    }

    #[test]
    fn the_request_is_todays_request() -> Result<()> {
        let tools = [ToolSpec {
            name: "t".into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
        }];
        let messages = [Message {
            role: Role::User,
            content: vec![
                Part::Document {
                    title: "a.pdf".into(),
                    context: "untrusted".into(),
                    source: DocumentSource::Pdf {
                        data: "QQ==".into(),
                    },
                },
                Part::Document {
                    title: "b".into(),
                    context: "untrusted".into(),
                    source: DocumentSource::Url {
                        url: "https://x/y.pdf".into(),
                    },
                },
                Part::text("Create it."),
            ],
        }];
        let request = ModelRequest {
            system: "S",
            tools: &tools,
            messages: &messages,
            max_output_tokens: 32_000,
            timeout: Duration::from_secs(1),
        };
        let expected = json!({
            "model": "claude-opus-5-5",
            "max_tokens": 32_000,
            "system": [{"type": "text", "text": "S", "cache_control": {"type": "ephemeral"}}],
            "tools": [{"name": "t", "description": "d", "input_schema": {"type": "object"}}],
            "messages": [{"role": "user", "content": [
                {"type": "document", "title": "a.pdf", "context": "untrusted",
                 "source": {"type": "base64", "media_type": "application/pdf", "data": "QQ=="}},
                {"type": "document", "title": "b", "context": "untrusted",
                 "source": {"type": "url", "url": "https://x/y.pdf"},
                 "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "Create it."},
            ]}],
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": "high"},
            "cache_control": {"type": "ephemeral"},
            "fallbacks": "default",
        });
        assert_eq!(request_body(&profile(), &request), expected);
        // The beta header goes with the fallback, the path and version as ever.
        let mut backend = AnthropicBackend::new(
            profile(),
            ScriptedWire::new(vec![json!({"content": [], "stop_reason": "end_turn"})]),
        );
        backend.send(&request)?;
        let (path, headers, _, _) = &backend.wire.posts[0];
        assert_eq!(path, "/v1/messages");
        assert!(headers.contains(&("anthropic-version".into(), API_VERSION.into())));
        assert!(headers.contains(&("anthropic-beta".into(), FALLBACK_BETA.into())));
        Ok(())
    }

    #[test]
    fn capabilities_not_declared_are_not_sent() {
        let mut p = profile();
        p.capabilities.caching = false;
        p.capabilities.adaptive_thinking = false;
        p.capabilities.server_fallback = false;
        p.effort = None;
        let messages = [Message {
            role: Role::User,
            content: vec![Part::Document {
                title: "a".into(),
                context: "c".into(),
                source: DocumentSource::Pdf {
                    data: "QQ==".into(),
                },
            }],
        }];
        let body = request_body(
            &p,
            &ModelRequest {
                system: "S",
                tools: &[],
                messages: &messages,
                max_output_tokens: 10,
                timeout: Duration::from_secs(1),
            },
        );
        for key in ["thinking", "output_config", "cache_control", "fallbacks"] {
            assert!(body.get(key).is_none(), "{key}");
        }
        assert!(body["system"][0].get("cache_control").is_none());
        assert!(
            body["messages"][0]["content"][0]
                .get("cache_control")
                .is_none()
        );
    }

    fn request<'a>() -> ModelRequest<'a> {
        ModelRequest {
            system: "S",
            tools: &[],
            messages: &[],
            max_output_tokens: 100,
            timeout: Duration::from_secs(1),
        }
    }

    /// A reply with every kind of block a stream carries.
    fn whole() -> Result<Value> {
        let thinking: Value = serde_json::from_str(
            r#"{"type": "thinking", "thinking": "The pin table is on page 3.", "signature": "CAQS6g=="}"#,
        )?;
        Ok(
            json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [thinking,
                {"type": "redacted_thinking", "data": "abc"},
                {"type": "text", "text": "Reading the pinout — µ-sized steps."},
                {"type": "tool_use", "id": "t1", "name": "query", "input": {"name": "assist.document_text", "params": {"pages": [3, 4]}}},
                {"type": "tool_use", "id": "t2", "name": "find", "input": {}}],
            "stop_reason": "tool_use", "stop_sequence": null,
            "usage": {"input_tokens": 12, "output_tokens": 345, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 6000}}),
        )
    }

    fn streamed(
        events: Vec<Value>,
        cancel_after: Option<usize>,
    ) -> (
        Result<ModelReply>,
        Vec<Delta>,
        AnthropicBackend<ScriptedWire>,
    ) {
        let mut backend = AnthropicBackend::new(profile(), ScriptedWire::new(events));
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let reply = backend.stream(
            &request(),
            &mut |d| {
                seen.push(d);
                if cancel_after.is_some_and(|n| seen.len() >= n) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            &cancel,
        );
        (reply, seen, backend)
    }

    #[test]
    fn a_streamed_reply_is_the_whole_reply_byte_for_byte() -> Result<()> {
        let whole = whole()?;
        let (reply, deltas, backend) = streamed(vec![stream::fixtures::messages(&whole)], None);
        let reply = reply?;
        assert_eq!(reply, parse_reply(&whole, "m")?);
        // Every block goes back exactly as a whole reply's would.
        let rebuilt: Vec<Value> = reply.content.iter().map(block).collect();
        assert_eq!(Value::Array(rebuilt), whole["content"]);
        assert_eq!(reply.usage.output_tokens, 345);
        assert_eq!(reply.usage.cache_read_tokens, 6000);
        // The pieces: reasoning, text, then the two calls in order.
        let text: String = deltas
            .iter()
            .filter_map(|d| match d {
                Delta::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Reading the pinout — µ-sized steps.");
        assert!(matches!(deltas.first(), Some(Delta::Reasoning(_))));
        let calls: Vec<&Delta> = deltas
            .iter()
            .filter(|d| matches!(d, Delta::ToolCall { .. }))
            .collect();
        assert_eq!(
            calls,
            [
                &Delta::ToolCall {
                    index: 0,
                    name: "query".into()
                },
                &Delta::ToolCall {
                    index: 1,
                    name: "find".into()
                }
            ]
        );
        let (_, _, body, _) = &backend.wire.posts[0];
        assert_eq!(body["stream"], true);
        Ok(())
    }

    #[test]
    fn stop_between_chunks_ends_the_reply_as_stopped() -> Result<()> {
        let (reply, deltas, _) = streamed(vec![stream::fixtures::messages(&whole()?)], Some(2));
        let reply = reply?;
        assert_eq!(reply.stop, StopReason::Stopped);
        assert_eq!(deltas.len(), 2, "nothing read after the stop: {deltas:?}");
        Ok(())
    }

    #[test]
    fn a_broken_stream_is_an_error_never_a_reply() -> Result<()> {
        let error = json!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{}}}\n\nevent: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n"
        );
        let (reply, _, _) = streamed(vec![error], None);
        assert!(
            matches!(reply, Err(Error::Api { status: 529, .. })),
            "{reply:?}"
        );
        // Cut before message_stop: incomplete, not an answer.
        let Value::String(full) = stream::fixtures::messages(&whole()?) else {
            return Err(Error::Protocol("a stream".into()));
        };
        let cut = full
            .split("event: message_delta")
            .next()
            .unwrap_or_default();
        let (reply, _, _) = streamed(vec![json!(cut)], None);
        assert!(matches!(reply, Err(Error::Protocol(_))), "{reply:?}");
        // A delta for a block that never started, and a huge index.
        for bad in [
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"x\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":99999999,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: not json\n\n",
        ] {
            let (reply, _, _) = streamed(vec![json!(bad)], None);
            assert!(matches!(reply, Err(Error::Protocol(_))), "{bad}: {reply:?}");
        }
        Ok(())
    }

    #[test]
    fn a_server_that_does_not_stream_still_answers() -> Result<()> {
        let whole = whole()?;
        let (reply, deltas, _) = streamed(vec![whole.clone()], None);
        assert_eq!(reply?, parse_reply(&whole, "m")?);
        assert_eq!(deltas.len(), 4, "reasoning, text and two calls: {deltas:?}");
        Ok(())
    }

    #[test]
    fn stops_and_a_page_image_in_a_tool_result_map() -> Result<()> {
        let reply = |stop: &str| {
            parse_reply(
                &json!({"content": [], "stop_reason": stop, "stop_details": {"category": "cyber"}}),
                "m",
            )
        };
        assert_eq!(reply("max_tokens")?.stop, StopReason::CutOff);
        assert_eq!(reply("end_turn")?.stop, StopReason::EndTurn);
        assert_eq!(reply("tool_use")?.stop, StopReason::ToolCalls);
        assert_eq!(
            reply("refusal")?.stop,
            StopReason::Refusal {
                category: "cyber".into()
            }
        );
        assert!(parse_reply(&json!({"error": "x"}), "m").is_err());
        let result = block(&Part::ToolResult {
            call_id: "c".into(),
            text: "page".into(),
            error: false,
            images: vec![Image {
                media_type: "image/png".into(),
                data: "QQ==".into(),
            }],
        });
        assert_eq!(result["content"][1]["source"]["media_type"], "image/png");
        Ok(())
    }
}

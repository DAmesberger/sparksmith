//! Streaming: what a reply looks like while it arrives, and the
//! server-sent-events reader both wire formats share.
//!
//! A backend turns its provider's events into [`Delta`]s for the chat and
//! builds, from the same events, the response its non-streaming parser
//! reads. So a streamed reply and a whole one become the same
//! [`super::ModelReply`], and a recording holds no trace of how it came.
use super::{ModelReply, Part};
use crate::{Error, Result};
use std::io::BufRead;

/// A piece of a reply as it arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delta {
    /// Answer text.
    Text(String),
    /// Reasoning text; the chat folds it.
    Reasoning(String),
    /// The model began writing a tool call: `index` counts the reply's calls
    /// from 0, in the order of its content.
    ToolCall { index: u32, name: String },
}

/// One server-sent event: its `event:` name (empty when none) and its
/// `data:` lines joined by newlines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// How a streaming POST ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Streamed {
    /// Every event was delivered.
    Done,
    /// The reader was told to stop between two events.
    Stopped,
    /// The server answered with one JSON reply instead of a stream, as some
    /// local servers do: the backend reads it as a whole reply.
    Whole(serde_json::Value),
}

/// Longest line of an event stream read; a longer one is a protocol error,
/// not an allocation.
const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;

/// Read server-sent events from `reader` until it ends or `on` answers
/// false (stop), cleaning each line with `clean` first. Returns whether it
/// stopped. A final event without its blank line is still delivered.
pub fn read_sse(
    reader: &mut dyn BufRead,
    clean: &dyn Fn(&str) -> String,
    on: &mut dyn FnMut(SseEvent) -> Result<bool>,
) -> Result<bool> {
    let mut event = SseEvent::default();
    let mut has_data = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = std::io::Read::take(&mut *reader, MAX_LINE_BYTES).read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.last() != Some(&b'\n') && n as u64 >= MAX_LINE_BYTES {
            return Err(Error::Protocol(format!(
                "an event-stream line exceeds {MAX_LINE_BYTES} bytes"
            )));
        }
        let text = clean(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']));
        if text.is_empty() {
            if has_data && !on(std::mem::take(&mut event))? {
                return Ok(true);
            }
            event = SseEvent::default();
            has_data = false;
            continue;
        }
        if text.starts_with(':') {
            continue;
        }
        let (field, value) = text.split_once(':').unwrap_or((text.as_str(), ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event.event = value.to_owned(),
            "data" => {
                if has_data {
                    event.data.push('\n');
                }
                event.data.push_str(value);
                has_data = true;
            }
            _ => {}
        }
    }
    if has_data && !on(event)? {
        return Ok(true);
    }
    Ok(false)
}

/// The deltas of a whole reply, for backends that do not stream (a replay):
/// the chat sees the same events either way.
pub fn deltas_of(reply: &ModelReply, deltas: &mut dyn FnMut(Delta)) {
    let mut calls = 0u32;
    for part in &reply.content {
        match part {
            Part::Text { text, .. } if !text.is_empty() => deltas(Delta::Text(text.clone())),
            Part::Reasoning { text, .. } if !text.is_empty() => {
                deltas(Delta::Reasoning(text.clone()));
            }
            Part::ToolCall { name, .. } => {
                deltas(Delta::ToolCall {
                    index: calls,
                    name: name.clone(),
                });
                calls = calls.saturating_add(1);
            }
            _ => {}
        }
    }
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// Splits streamed content into reasoning and answer for servers that put
/// reasoning in `<think>` tags at the start of the content, as the whole
/// reply's parser does: what is inside the tags is reasoning, and an
/// unclosed tag is all reasoning.
#[derive(Debug, Default)]
pub struct ThinkSplitter {
    state: ThinkState,
    held: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum ThinkState {
    /// Nothing but whitespace yet: the tag may still come.
    #[default]
    Start,
    Inside,
    /// Just after the closing tag: leading whitespace is dropped.
    After,
    Answer,
}

impl ThinkSplitter {
    /// Feed one piece of content.
    pub fn push(&mut self, piece: &str, deltas: &mut dyn FnMut(Delta)) {
        self.held.push_str(piece);
        loop {
            match self.state {
                ThinkState::Start => {
                    let trimmed = self.held.trim_start();
                    if trimmed.is_empty()
                        || (trimmed.len() < OPEN.len() && OPEN.starts_with(trimmed))
                    {
                        return;
                    }
                    if let Some(rest) = trimmed.strip_prefix(OPEN) {
                        self.held = rest.to_owned();
                        self.state = ThinkState::Inside;
                    } else {
                        self.state = ThinkState::Answer;
                    }
                }
                ThinkState::Inside => {
                    if let Some((reasoning, rest)) = self.held.split_once(CLOSE) {
                        if !reasoning.is_empty() {
                            deltas(Delta::Reasoning(reasoning.to_owned()));
                        }
                        self.held = rest.to_owned();
                        self.state = ThinkState::After;
                        continue;
                    }
                    // Hold back what could be the start of the closing tag.
                    let mut keep = self.held.len().saturating_sub(CLOSE.len() - 1);
                    while !self.held.is_char_boundary(keep) {
                        keep -= 1;
                    }
                    if keep > 0 {
                        let rest = self.held.split_off(keep);
                        deltas(Delta::Reasoning(std::mem::replace(&mut self.held, rest)));
                    }
                    return;
                }
                ThinkState::After => {
                    let trimmed = self.held.trim_start();
                    if trimmed.is_empty() {
                        self.held.clear();
                        return;
                    }
                    self.held = trimmed.to_owned();
                    self.state = ThinkState::Answer;
                }
                ThinkState::Answer => {
                    if !self.held.is_empty() {
                        deltas(Delta::Text(std::mem::take(&mut self.held)));
                    }
                    return;
                }
            }
        }
    }

    /// The stream ended: deliver what was held back.
    pub fn finish(&mut self, deltas: &mut dyn FnMut(Delta)) {
        let held = std::mem::take(&mut self.held);
        match self.state {
            ThinkState::Inside if !held.is_empty() => deltas(Delta::Reasoning(held)),
            ThinkState::Start | ThinkState::Answer if !held.trim().is_empty() => {
                deltas(Delta::Text(held));
            }
            _ => {}
        }
    }
}

/// Event streams made from whole responses, so every recorded response a
/// test holds also replays as the stream a server would have sent.
#[cfg(test)]
pub mod fixtures {
    use serde_json::{Value, json};

    /// `text` in pieces of a few characters, as servers send it.
    pub fn pieces(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        chars.chunks(7).map(|c| c.iter().collect()).collect()
    }

    fn event(out: &mut String, name: Option<&str>, data: &Value) {
        if let Some(name) = name {
            out.push_str(&format!("event: {name}\n"));
        }
        out.push_str(&format!("data: {data}\n\n"));
    }

    /// A Messages response as its event stream.
    pub fn messages(response: &Value) -> Value {
        let mut out = String::new();
        let mut start = response.clone();
        if let Some(fields) = start.as_object_mut() {
            fields.insert("content".into(), json!([]));
            fields.insert("stop_reason".into(), Value::Null);
            fields.remove("stop_details");
        }
        event(
            &mut out,
            Some("message_start"),
            &json!({"type": "message_start", "message": start}),
        );
        out.push_str(": a comment line\n\n");
        let blocks = response["content"].as_array().cloned().unwrap_or_default();
        for (i, block) in blocks.iter().enumerate() {
            let mut skeleton = block.clone();
            let mut deltas = Vec::new();
            match block["type"].as_str() {
                Some("text") => {
                    skeleton["text"] = json!("");
                    for p in pieces(block["text"].as_str().unwrap_or_default()) {
                        deltas.push(json!({"type": "text_delta", "text": p}));
                    }
                }
                Some("thinking") => {
                    skeleton["thinking"] = json!("");
                    if let Some(fields) = skeleton.as_object_mut() {
                        fields.remove("signature");
                    }
                    for p in pieces(block["thinking"].as_str().unwrap_or_default()) {
                        deltas.push(json!({"type": "thinking_delta", "thinking": p}));
                    }
                    if let Some(signature) = block.get("signature") {
                        deltas.push(json!({"type": "signature_delta", "signature": signature}));
                    }
                }
                Some("tool_use") => {
                    skeleton["input"] = json!({});
                    let input = block["input"].to_string();
                    if input != "{}" {
                        for p in pieces(&input) {
                            deltas.push(json!({"type": "input_json_delta", "partial_json": p}));
                        }
                    }
                }
                _ => {}
            }
            event(
                &mut out,
                Some("content_block_start"),
                &json!({"type": "content_block_start", "index": i, "content_block": skeleton}),
            );
            if i == 0 {
                event(&mut out, Some("ping"), &json!({"type": "ping"}));
            }
            for delta in deltas {
                event(
                    &mut out,
                    Some("content_block_delta"),
                    &json!({"type": "content_block_delta", "index": i, "delta": delta}),
                );
            }
            event(
                &mut out,
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": i}),
            );
        }
        let mut delta = json!({"stop_reason": response["stop_reason"], "stop_sequence": null});
        if let Some(details) = response.get("stop_details") {
            delta["stop_details"] = details.clone();
        }
        event(
            &mut out,
            Some("message_delta"),
            &json!({"type": "message_delta", "delta": delta,
                "usage": {"output_tokens": response["usage"]["output_tokens"]}}),
        );
        event(
            &mut out,
            Some("message_stop"),
            &json!({"type": "message_stop"}),
        );
        Value::String(out)
    }

    /// A chat completion as its chunk stream, usage last.
    pub fn chat(response: &Value) -> Value {
        let mut out = String::new();
        let model = &response["model"];
        let chunk = |delta: Value, finish: Value| {
            json!({"id": "chatcmpl-s", "object": "chat.completion.chunk", "model": model,
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        };
        let message = &response["choices"][0]["message"];
        event(
            &mut out,
            None,
            &chunk(json!({"role": "assistant"}), Value::Null),
        );
        for key in ["reasoning", "reasoning_content", "thinking"] {
            for p in pieces(message[key].as_str().unwrap_or_default()) {
                let mut delta = serde_json::Map::new();
                delta.insert(key.to_owned(), Value::String(p));
                event(&mut out, None, &chunk(Value::Object(delta), Value::Null));
            }
        }
        for p in pieces(message["content"].as_str().unwrap_or_default()) {
            event(&mut out, None, &chunk(json!({"content": p}), Value::Null));
        }
        let calls = message["tool_calls"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (k, call) in calls.iter().enumerate() {
            let arguments = &call["function"]["arguments"];
            let first = json!({"index": k, "id": call["id"], "type": "function",
                "function": {"name": call["function"]["name"], "arguments": ""}});
            event(
                &mut out,
                None,
                &chunk(json!({"tool_calls": [first]}), Value::Null),
            );
            let parts = match arguments.as_str() {
                Some(text) => pieces(text).into_iter().map(Value::String).collect(),
                None => vec![arguments.clone()],
            };
            for p in parts {
                let piece = json!({"index": k, "function": {"arguments": p}});
                event(
                    &mut out,
                    None,
                    &chunk(json!({"tool_calls": [piece]}), Value::Null),
                );
            }
        }
        event(
            &mut out,
            None,
            &chunk(json!({}), response["choices"][0]["finish_reason"].clone()),
        );
        if response["usage"].is_object() {
            event(
                &mut out,
                None,
                &json!({"id": "chatcmpl-s", "model": model, "choices": [], "usage": response["usage"]}),
            );
        }
        out.push_str("data: [DONE]\n\n");
        Value::String(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(text: &str) -> Result<(Vec<SseEvent>, bool)> {
        let mut seen = Vec::new();
        let stopped = read_sse(&mut text.as_bytes(), &|s| s.to_owned(), &mut |e| {
            seen.push(e);
            Ok(true)
        })?;
        Ok((seen, stopped))
    }

    #[test]
    fn events_are_read_with_names_comments_and_joined_data() -> Result<()> {
        let (seen, stopped) = events(
            ": keep-alive\r\nevent: ping\r\ndata: {}\r\n\r\ndata: a\ndata: b\n\nretry: 5\n\ndata: [DONE]",
        )?;
        assert!(!stopped);
        assert_eq!(
            seen,
            vec![
                SseEvent {
                    event: "ping".into(),
                    data: "{}".into()
                },
                SseEvent {
                    event: String::new(),
                    data: "a\nb".into()
                },
                SseEvent {
                    event: String::new(),
                    data: "[DONE]".into()
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn the_reader_stops_between_events_and_cleans_lines() -> Result<()> {
        let mut seen = Vec::new();
        let stopped = read_sse(
            &mut "data: one sk-1\n\ndata: two\n\ndata: three\n\n".as_bytes(),
            &|s| s.replace("sk-1", "[redacted]"),
            &mut |e| {
                seen.push(e.data);
                Ok(seen.len() < 2)
            },
        )?;
        assert!(stopped);
        assert_eq!(seen, vec!["one [redacted]", "two"]);
        Ok(())
    }

    fn split(pieces: &[&str]) -> Vec<Delta> {
        let mut out = Vec::new();
        let mut splitter = ThinkSplitter::default();
        for piece in pieces {
            splitter.push(piece, &mut |d| out.push(d));
        }
        splitter.finish(&mut |d| out.push(d));
        // Join neighbouring pieces of one kind, as the chat shows them.
        let mut joined: Vec<Delta> = Vec::new();
        for d in out {
            match (joined.last_mut(), d) {
                (Some(Delta::Text(a)), Delta::Text(b))
                | (Some(Delta::Reasoning(a)), Delta::Reasoning(b)) => a.push_str(&b),
                (_, d) => joined.push(d),
            }
        }
        joined
    }

    #[test]
    fn think_tags_split_across_pieces_become_reasoning() {
        assert_eq!(
            split(&["\n<th", "ink>let me", " see</th", "ink>\n\n", "Answer", "."]),
            vec![
                Delta::Reasoning("let me see".into()),
                Delta::Text("Answer.".into())
            ]
        );
        assert_eq!(
            split(&["Plain ", "answer"]),
            vec![Delta::Text("Plain answer".into())]
        );
        // Cut off inside the tag: all reasoning.
        assert_eq!(
            split(&["<think>still ", "going"]),
            vec![Delta::Reasoning("still going".into())]
        );
        // A tag later in the text is text, and a lone partial tag too.
        assert_eq!(
            split(&["a <think>b</think>"]),
            vec![Delta::Text("a <think>b</think>".into())]
        );
        assert_eq!(split(&["<thi"]), vec![Delta::Text("<thi".into())]);
        // Multi-byte text next to the held-back tail.
        assert_eq!(
            split(&["<think>µµµµµµµµ", "</think>ok"]),
            vec![
                Delta::Reasoning("µµµµµµµµ".into()),
                Delta::Text("ok".into())
            ]
        );
    }
}

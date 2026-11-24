//! The HTTP seam under the backends: one JSON POST in, one JSON value out,
//! or a stream of server-sent events. [`HttpWire`] owns the connection key
//! and lends an opaque per-stream redactor to the backend. It follows no
//! redirects, bounds responses, and cleans both decoded observer deltas
//! and final replies (including decoded tool arguments).
use super::stream::{self, SseEvent, Streamed};
use super::{Endpoint, Secret};
use crate::http::{self, Agents, Next, Silence};
use crate::{Error, Result};
use serde_json::Value;
use std::time::Duration;

/// Largest response body read.
const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
/// Who a silent request waited for, in words.
const MODEL: &str = "the model";

/// How a key is presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStyle {
    /// `x-api-key: KEY`.
    ApiKeyHeader,
    /// `Authorization: Bearer KEY`.
    Bearer,
}

/// Where a backend's requests go.
pub trait Wire {
    /// Where this goes, for the review report; never holds a key.
    fn label(&self) -> String;
    /// POST `body` to the endpoint's `path`: at most `timeout` in all when
    /// the person limits the run's time, and no longer than the set silence
    /// hears nothing ([`Self::set_silence`]). A cancelled call ends it.
    fn post(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Option<Duration>,
    ) -> Result<Value>;
    /// POST `body` and read the reply as server-sent events, giving each to
    /// `on` until the stream ends or `on` answers false; a server that
    /// answers with one JSON reply instead gives [`Streamed::Whole`].
    fn post_stream(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Option<Duration>,
        on: &mut dyn FnMut(SseEvent) -> Result<bool>,
    ) -> Result<Streamed>;
    /// How long the requests from here on may hear nothing.
    fn set_silence(&mut self, _silence: Silence) {}
    /// `value` without the key, for a reply put together from many events:
    /// a key split across two of them is whole again here.
    fn clean_value(&self, value: Value) -> Value {
        value
    }
    /// Filter decoded content before it reaches a streaming observer.
    fn redactor(&self) -> super::redact::Redactor {
        super::redact::Redactor::default()
    }
    /// Clean the neutral reply too, after tool arguments have been decoded.
    fn clean_reply(&self, reply: super::ModelReply) -> Result<super::ModelReply> {
        Ok(reply)
    }
}

/// The real thing, over ureq ([`crate::http`]): a request is given up when
/// its server sends nothing for as long as the person allows, and ends at
/// once when its call is cancelled.
pub struct HttpWire {
    agents: Agents,
    http: Http,
}

#[derive(Clone)]
struct Http {
    endpoint: Endpoint,
    key: Option<Secret>,
    auth: AuthStyle,
    silence: Silence,
}

/// What a streaming request's thread says.
enum Step {
    Event(SseEvent),
    End(Result<Streamed>),
}

fn owned(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

impl std::fmt::Debug for HttpWire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpWire")
            .field("endpoint", &self.http.endpoint)
            .finish_non_exhaustive()
    }
}

impl HttpWire {
    pub fn new(endpoint: Endpoint, key: Option<Secret>, auth: AuthStyle) -> Result<Self> {
        if key.is_some() {
            endpoint.check_key_allowed()?;
        }
        Ok(Self {
            agents: Agents::new(endpoint.is_loopback()),
            http: Http {
                endpoint,
                key,
                auth,
                silence: Silence::default(),
            },
        })
    }
}

impl Wire for HttpWire {
    fn label(&self) -> String {
        let endpoint = &self.http.endpoint;
        format!("{} ({})", endpoint.base(), endpoint.location())
    }

    fn post(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let (path, headers, body) = (path.to_owned(), owned(headers), body.clone());
        let http = self.http.clone();
        let running = self
            .agents
            .start(move |agent, said| {
                let _ = said.send(http.post(agent, &path, &headers, &body, timeout));
            })
            .map_err(Error::Http)?;
        match running.next() {
            Next::Step(answer) => {
                running.done();
                answer.map(|value| self.clean_value(value))
            }
            Next::Cancelled => Err(Error::Http("the request was stopped".into())),
            Next::Gone => Err(Error::Http("the request ended without an answer".into())),
        }
    }

    fn post_stream(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Option<Duration>,
        on: &mut dyn FnMut(SseEvent) -> Result<bool>,
    ) -> Result<Streamed> {
        let (path, headers, body) = (path.to_owned(), owned(headers), body.clone());
        let http = self.http.clone();
        let running = self
            .agents
            .start(move |agent, said| {
                let end = http.post_stream(agent, &path, &headers, &body, timeout, &mut |event| {
                    Ok(said.send(Step::Event(event)).is_ok())
                });
                let _ = said.send(Step::End(end));
            })
            .map_err(Error::Http)?;
        // Dropping `running` on a stop closes the connection.
        loop {
            match running.next() {
                Next::Cancelled => return Ok(Streamed::Stopped),
                Next::Gone => {
                    return Err(Error::Http("the request ended without an answer".into()));
                }
                Next::Step(Step::Event(event)) => {
                    let more =
                        on(event).map_err(|e| Error::Http(self.http.clean(&e.to_string())))?;
                    if !more {
                        return Ok(Streamed::Stopped);
                    }
                }
                Next::Step(Step::End(end)) => {
                    running.done();
                    return Ok(match end? {
                        Streamed::Whole(value) => Streamed::Whole(self.clean_value(value)),
                        other => other,
                    });
                }
            }
        }
    }

    fn set_silence(&mut self, silence: Silence) {
        self.http.silence = silence;
    }

    fn clean_value(&self, mut value: Value) -> Value {
        let Some(key) = &self.http.key else {
            return value;
        };
        fn clean(value: &mut Value, key: &Secret) {
            match value {
                Value::String(text) => *text = key.redact(text),
                Value::Array(items) => items.iter_mut().for_each(|v| clean(v, key)),
                Value::Object(fields) => {
                    *fields = std::mem::take(fields)
                        .into_iter()
                        .map(|(name, mut v)| {
                            clean(&mut v, key);
                            (key.redact(&name), v)
                        })
                        .collect();
                }
                _ => {}
            }
        }
        clean(&mut value, key);
        value
    }

    fn redactor(&self) -> super::redact::Redactor {
        super::redact::Redactor::new(self.http.key.clone())
    }

    fn clean_reply(&self, reply: super::ModelReply) -> Result<super::ModelReply> {
        let Some(key) = &self.http.key else {
            return Ok(reply);
        };
        let reply: super::ModelReply =
            serde_json::from_value(self.clean_value(serde_json::to_value(reply)?))?;
        // Separate content blocks can reconstruct a credential even though
        // every individual JSON string is clean. Refuse the final reply:
        // rewriting block boundaries or signed replay would change its meaning.
        // HTTP response size already bounds these temporary channel buffers.
        for reasoning in [false, true] {
            let channel: String = reply
                .content
                .iter()
                .filter_map(|part| match part {
                    super::Part::Text { text, .. } if !reasoning => Some(text.as_str()),
                    super::Part::Reasoning { text, .. } if reasoning => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if channel.contains(key.expose()) {
                return Err(Error::Protocol(
                    "the model response contained a credential across content blocks".into(),
                ));
            }
        }
        Ok(reply)
    }
}

impl Http {
    fn clean(&self, text: &str) -> String {
        match &self.key {
            Some(key) => key.redact(text),
            None => text.to_owned(),
        }
    }

    /// A request that failed, in words without the key.
    fn failed(&self, error: &ureq::Error) -> Error {
        if http::silent(error) {
            return Error::Silent(self.silence.why(MODEL));
        }
        Error::Http(self.clean(&error.to_string()))
    }

    fn post(
        &self,
        agent: &ureq::Agent,
        path: &str,
        headers: &[(String, String)],
        body: &Value,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let mut response = self.send(agent, path, headers, body, timeout)?;
        let text = self.read_text(&mut response)?;
        reply_value(&text)
    }

    fn post_stream(
        &self,
        agent: &ureq::Agent,
        path: &str,
        headers: &[(String, String)],
        body: &Value,
        timeout: Option<Duration>,
        on: &mut dyn FnMut(SseEvent) -> Result<bool>,
    ) -> Result<Streamed> {
        let mut response = self.send(agent, path, headers, body, timeout)?;
        let events = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));
        if !events {
            // Not a stream: one reply, or events under another type.
            let text = self.read_text(&mut response)?;
            return match serde_json::from_str::<Value>(&text) {
                Ok(value) => Ok(Streamed::Whole(value)),
                Err(_) => {
                    let stopped =
                        stream::read_sse(&mut text.as_bytes(), &|line| line.to_owned(), on)
                            .map_err(|e| Error::Http(self.clean(&e.to_string())))?;
                    Ok(if stopped {
                        Streamed::Stopped
                    } else {
                        Streamed::Done
                    })
                }
            };
        }
        let reader = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .reader();
        let mut reader = std::io::BufReader::new(reader);
        let clean = |line: &str| self.clean(line);
        let stopped = match stream::read_sse(&mut reader, &clean, on) {
            Ok(stopped) => stopped,
            Err(Error::Io(error)) if http::silent_io(&error) => {
                return Ok(Streamed::Silent(self.silence.why(MODEL)));
            }
            Err(error) => return Err(Error::Http(self.clean(&error.to_string()))),
        };
        Ok(if stopped {
            Streamed::Stopped
        } else {
            Streamed::Done
        })
    }

    /// Send one POST: the response with a success status, or the error.
    fn send(
        &self,
        agent: &ureq::Agent,
        path: &str,
        headers: &[(String, String)],
        body: &Value,
        timeout: Option<Duration>,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let url = format!("{}{path}", self.endpoint.base());
        let silence = self.silence.after;
        let mut request = agent
            .post(&url)
            .config()
            .timeout_global(timeout)
            .timeout_per_read(silence)
            .timeout_per_write(silence)
            .timeout_connect(silence)
            .timeout_resolve(silence)
            .build()
            .header("content-type", "application/json");
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(key) = &self.key {
            request = match self.auth {
                AuthStyle::ApiKeyHeader => request.header("x-api-key", key.expose()),
                AuthStyle::Bearer => {
                    request.header("authorization", format!("Bearer {}", key.expose()))
                }
            };
        }
        let mut response = request.send_json(body).map_err(|e| self.failed(&e))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(Error::Api {
                status,
                message: "the endpoint redirected the request; give the final URL".into(),
            });
        }
        if status >= 400 {
            let text = self.read_text(&mut response)?;
            let value = serde_json::from_str::<Value>(&text);
            return Err(Error::Api {
                status,
                message: self.clean(&error_message(value.as_ref().ok(), &text)),
            });
        }
        Ok(response)
    }

    /// The whole body as text, bounded, without the key: a server that
    /// echoes the key cannot put it in a log or a recording.
    fn read_text(&self, response: &mut ureq::http::Response<ureq::Body>) -> Result<String> {
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|e| self.failed(&e))?;
        Ok(self.clean(&text))
    }
}

fn reply_value(text: &str) -> Result<Value> {
    serde_json::from_str::<Value>(text)
        .map_err(|e| Error::Protocol(format!("the reply is not JSON: {e}")))
}

/// The message of an error body, in any of the common shapes.
fn error_message(value: Option<&Value>, text: &str) -> String {
    const MAX: usize = 2000;
    let found = value.and_then(|v| {
        let error = &v["error"];
        if let Some(message) = error["message"].as_str() {
            let kind = error["type"].as_str().unwrap_or("error");
            Some(format!("{kind}: {message}"))
        } else if let Some(message) = error.as_str() {
            Some(message.to_owned())
        } else {
            v["message"].as_str().map(str::to_owned)
        }
    });
    let message = found.unwrap_or_else(|| text.trim().to_owned());
    message.chars().take(MAX).collect()
}

/// One POST a test script received: path, headers, body and timeout.
#[cfg(test)]
pub type Post = (String, Vec<(String, String)>, Value, Option<Duration>);

/// Canned wire replies for tests of the backends, with each request kept.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct ScriptedWire {
    pub replies: std::collections::VecDeque<Result<Value>>,
    /// (path, headers, body, timeout) of each POST.
    pub posts: Vec<Post>,
    /// What a count request answers.
    pub count: Option<u64>,
}

#[cfg(test)]
impl ScriptedWire {
    pub fn new(replies: Vec<Value>) -> Self {
        Self {
            replies: replies.into_iter().map(Ok).collect(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
impl Wire for ScriptedWire {
    fn label(&self) -> String {
        "a test script".into()
    }

    fn post(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        if path.ends_with("count_tokens") {
            return match self.count {
                Some(n) => Ok(serde_json::json!({"input_tokens": n})),
                None => Err(Error::Http("no count".into())),
            };
        }
        self.posts.push((
            path.to_owned(),
            headers
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect(),
            body.clone(),
            timeout,
        ));
        self.replies
            .pop_front()
            .unwrap_or_else(|| Err(Error::Http("the recording has no more responses".into())))
    }

    /// A scripted reply that is a string is an event stream; anything else
    /// is a whole reply.
    fn post_stream(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Option<Duration>,
        on: &mut dyn FnMut(SseEvent) -> Result<bool>,
    ) -> Result<Streamed> {
        match self.post(path, headers, body, timeout)? {
            Value::String(events) => {
                let stopped = stream::read_sse(&mut events.as_bytes(), &|l| l.to_owned(), on)?;
                Ok(if stopped {
                    Streamed::Stopped
                } else {
                    Streamed::Done
                })
            }
            whole => Ok(Streamed::Whole(whole)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A one-shot HTTP server on this computer: answers `reply` once and
    /// returns the request text it received.
    fn server(reply: String) -> Result<(String, std::thread::JoinHandle<String>)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            if let Ok((mut stream, _)) = listener.accept() {
                let mut chunk = [0u8; 8192];
                while let Ok(n) = stream.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    seen.extend_from_slice(chunk.get(..n).unwrap_or_default());
                    let text = String::from_utf8_lossy(&seen);
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if seen.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let _ = stream.write_all(reply.as_bytes());
            }
            String::from_utf8_lossy(&seen).into_owned()
        });
        Ok((base, handle))
    }

    fn http(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn streamed_key_split_at_every_boundary_is_hidden_before_observers() -> Result<()> {
        use crate::model::{Delta, ModelBackend, ModelProfile, ModelRequest};
        use std::sync::atomic::AtomicBool;
        let key = "sk-dummy-review-key";
        for split in 0..=key.len() {
            let events = [
                json!({"choices": [{"delta": {"content": format!("Before {}", &key[..split])}}]}),
                json!({"choices": [{"delta": {"content": format!("{} after", &key[split..])}, "finish_reason": "stop"}]}),
            ];
            let body = format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                events[0], events[1]
            );
            let (base, handle) = server(format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            ))?;
            let wire = HttpWire::new(Endpoint::parse(&base)?, Secret::new(key), AuthStyle::Bearer)?;
            let mut backend = crate::model::openai::OpenAiBackend::new(
                ModelProfile::openai_compatible("test", 32_768),
                wire,
            );
            let request = ModelRequest {
                system: "",
                tools: &[],
                messages: &[],
                max_output_tokens: 100,
                timeout: None,
            };
            let mut shown = String::new();
            let reply = backend.stream(
                &request,
                &mut |d| {
                    if let Delta::Text(t) = d {
                        shown.push_str(&t);
                    }
                },
                &AtomicBool::new(false),
            )?;
            let _ = handle.join();
            assert_eq!(shown, "Before [redacted] after", "split {split}");
            assert!(!format!("{reply:?}").contains(key));
        }
        Ok(())
    }

    #[test]
    fn anthropic_text_and_thinking_redact_split_keys_before_observers() -> Result<()> {
        use crate::model::{Delta, ModelBackend, ModelProfile, ModelRequest};
        use std::sync::atomic::AtomicBool;
        let key = "sk-dummy-review-key";
        for field in ["text", "thinking"] {
            for split in 0..=key.len() {
                let mut events = vec![
                    json!({"type": "message_start", "message": {"model": "test", "usage": {"input_tokens": 1}}}),
                    json!({"type": "content_block_start", "index": 0, "content_block": {"type": field, (field): ""}}),
                ];
                for text in [
                    format!("Before {}", &key[..split]),
                    format!("{} after", &key[split..]),
                ] {
                    events.push(json!({"type": "content_block_delta", "index": 0, "delta": {"type": format!("{field}_delta"), (field): text}}));
                }
                events.extend([
                    json!({"type": "content_block_stop", "index": 0}),
                    json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 1}}),
                    json!({"type": "message_stop"}),
                ]);
                let body: String = events
                    .iter()
                    .map(|v| {
                        format!(
                            "event: {}\ndata: {v}\n\n",
                            v["type"].as_str().unwrap_or_default()
                        )
                    })
                    .collect();
                let (base, handle) = server(format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                ))?;
                let wire = HttpWire::new(
                    Endpoint::parse(&base)?,
                    Secret::new(key),
                    AuthStyle::ApiKeyHeader,
                )?;
                let mut backend = crate::model::anthropic::AnthropicBackend::new(
                    ModelProfile::anthropic("test"),
                    wire,
                );
                let request = ModelRequest {
                    system: "",
                    tools: &[],
                    messages: &[],
                    max_output_tokens: 100,
                    timeout: None,
                };
                let mut shown = String::new();
                let reply = backend.stream(
                    &request,
                    &mut |d| match d {
                        Delta::Text(t) | Delta::Reasoning(t) => shown.push_str(&t),
                        _ => {}
                    },
                    &AtomicBool::new(false),
                )?;
                let _ = handle.join();
                assert_eq!(shown, "Before [redacted] after", "{field} split {split}");
                assert!(!format!("{reply:?}").contains(key));
            }
        }
        Ok(())
    }

    #[test]
    fn escaped_json_strings_and_decoded_tool_arguments_are_cleaned() -> Result<()> {
        let key = "sk-quote\"key";
        let (base, handle) = server(http("200 OK", &json!({"value": key}).to_string()))?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, Secret::new(key), AuthStyle::Bearer)?;
        let value = wire.post("/test", &[], &json!({}), None)?;
        let _ = handle.join();
        assert_eq!(value["value"], "[redacted]");
        let reply = super::super::ModelReply {
            model: "script".into(),
            content: vec![super::super::Part::ToolCall {
                id: "call".into(),
                name: "query".into(),
                input: json!({"value": key}),
                malformed: None,
                replay: None,
            }],
            stop: super::super::StopReason::ToolCalls,
            usage: Default::default(),
            usage_estimated: false,
        };
        let cleaned = wire.clean_reply(reply)?;
        assert!(!format!("{cleaned:?}").contains(key));
        assert!(serde_json::to_string(&cleaned)?.contains("[redacted]"));
        Ok(())
    }

    #[test]
    fn keys_split_across_final_blocks_are_refused_without_rewriting_replay() -> Result<()> {
        use crate::model::{ModelBackend, ModelProfile, ModelRequest};
        use std::sync::atomic::AtomicBool;
        let key = "sk-dummy-review-key";
        for field in ["text", "thinking"] {
            for streamed in [false, true] {
                let body = json!({
                    "model": "test", "stop_reason": "end_turn",
                    "usage": {"input_tokens": 1, "output_tokens": 1},
                    "content": [
                        {"type": field, (field): format!("Before {}", &key[..6]), "signature": "signed-a"},
                        {"type": "tool_use", "id": "call", "name": "query", "input": {}},
                        {"type": field, (field): format!("{} after", &key[6..]), "signature": "signed-b"}
                    ]
                });
                let (base, handle) = server(http("200 OK", &body.to_string()))?;
                let wire = HttpWire::new(
                    Endpoint::parse(&base)?,
                    Secret::new(key),
                    AuthStyle::ApiKeyHeader,
                )?;
                let mut backend = crate::model::anthropic::AnthropicBackend::new(
                    ModelProfile::anthropic("test"),
                    wire,
                );
                let request = ModelRequest {
                    system: "",
                    tools: &[],
                    messages: &[],
                    max_output_tokens: 100,
                    timeout: None,
                };
                let mut deltas = Vec::new();
                let result = if streamed {
                    backend.stream(
                        &request,
                        &mut |delta| deltas.push(delta),
                        &AtomicBool::new(false),
                    )
                } else {
                    backend.send(&request)
                };
                let _ = handle.join();
                assert!(
                    matches!(result, Err(Error::Protocol(ref why)) if why == "the model response contained a credential across content blocks"),
                    "{field}: {result:?}"
                );
                assert!(
                    deltas.is_empty(),
                    "whole responses are checked before observers"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn decoded_http_errors_never_echo_a_json_escaped_key() -> Result<()> {
        let key = "sk-dummy-review-key";
        let body = json!({"error": {"type": key, "message": format!("Invalid {key}")}})
            .to_string()
            .replace("dummy", "\\u0064ummy");
        let (base, handle) = server(http("400 Bad Request", &body))?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, Secret::new(key), AuthStyle::Bearer)?;
        let error = wire
            .post("/test", &[], &json!({}), None)
            .err()
            .ok_or_else(|| Error::Protocol("expected API error".into()))?;
        let _ = handle.join();
        assert!(!error.to_string().contains(key));
        assert!(error.to_string().contains("[redacted]"));
        Ok(())
    }

    #[test]
    fn decoded_errors_in_mislabeled_event_streams_are_redacted() -> Result<()> {
        use crate::model::{ModelBackend, ModelProfile, ModelRequest};
        let key = "sk-dummy-review-key";
        let event = json!({"type": "error", "error": {"type": "api_error", "message": key}})
            .to_string()
            .replace("dummy", "\\u0064ummy");
        let body = format!("event: error\ndata: {event}\n\n");
        // JSON content-type, but an SSE body: this takes the fallback reader.
        let (base, handle) = server(http("200 OK", &body))?;
        let wire = HttpWire::new(
            Endpoint::parse(&base)?,
            Secret::new(key),
            AuthStyle::ApiKeyHeader,
        )?;
        let mut backend =
            crate::model::anthropic::AnthropicBackend::new(ModelProfile::anthropic("test"), wire);
        let request = ModelRequest {
            system: "",
            tools: &[],
            messages: &[],
            max_output_tokens: 100,
            timeout: None,
        };
        let error = backend
            .stream(
                &request,
                &mut |_| {},
                &std::sync::atomic::AtomicBool::new(false),
            )
            .err()
            .ok_or_else(|| Error::Protocol("expected stream error".into()))?;
        let _ = handle.join();
        assert!(!error.to_string().contains(key));
        assert!(error.to_string().contains("[redacted]"));
        Ok(())
    }

    #[test]
    fn interleaved_streams_redact_each_observer_channel() -> Result<()> {
        use crate::model::{Delta, ModelBackend, ModelProfile, ModelRequest};
        let key = "sk-dummy-review-key";
        for (field, other) in [
            ("content", "reasoning_content"),
            ("reasoning_content", "content"),
        ] {
            let events = [
                json!({"choices": [{"delta": {(field): "sk-dummy-"}}]}),
                json!({"choices": [{"delta": {(other): "unrelated middle"}}]}),
                json!({"choices": [{"delta": {(field): "review-key"}, "finish_reason": "stop"}]}),
            ];
            let body = format!(
                "{}data: [DONE]\n\n",
                events
                    .iter()
                    .map(|e| format!("data: {e}\n\n"))
                    .collect::<String>()
            );
            let (base, handle) = server(format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            ))?;
            let wire = HttpWire::new(Endpoint::parse(&base)?, Secret::new(key), AuthStyle::Bearer)?;
            let mut backend = crate::model::openai::OpenAiBackend::new(
                ModelProfile::openai_compatible("test", 32_768),
                wire,
            );
            let request = ModelRequest {
                system: "",
                tools: &[],
                messages: &[],
                max_output_tokens: 100,
                timeout: None,
            };
            let (mut text, mut thinking) = (String::new(), String::new());
            backend.stream(
                &request,
                &mut |d| match d {
                    Delta::Text(t) => text.push_str(&t),
                    Delta::Reasoning(t) => thinking.push_str(&t),
                    _ => {}
                },
                &std::sync::atomic::AtomicBool::new(false),
            )?;
            let _ = handle.join();
            let (protected, unchanged) = if field == "content" {
                (&text, &thinking)
            } else {
                (&thinking, &text)
            };
            assert_eq!(protected, "[redacted]");
            assert_eq!(unchanged, "unrelated middle");
        }
        Ok(())
    }

    #[test]
    fn a_real_post_carries_the_key_only_in_its_header_and_hides_an_echoed_key() -> Result<()> {
        let (base, handle) = server(http(
            "200 OK",
            r#"{"echo": "you sent sk-secret-1", "ok": true}"#,
        ))?;
        let mut wire = HttpWire::new(
            Endpoint::parse(&base)?,
            Secret::new("sk-secret-1"),
            AuthStyle::Bearer,
        )?;
        let reply = wire.post("/chat", &[("x-extra", "1")], &json!({"a": 1}), None)?;
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["echo"], "you sent [redacted]");
        let seen = handle.join().unwrap_or_default().to_ascii_lowercase();
        assert!(seen.starts_with("post /chat "), "{seen}");
        assert!(seen.contains("authorization: bearer sk-secret-1"), "{seen}");
        assert!(seen.contains("x-extra: 1"), "{seen}");
        assert!(!format!("{wire:?}").contains("sk-secret-1"));
        Ok(())
    }

    #[test]
    fn an_error_body_is_reported_without_the_key() -> Result<()> {
        let (base, handle) = server(http(
            "401 Unauthorized",
            r#"{"error": {"type": "auth", "message": "bad key sk-secret-2"}}"#,
        ))?;
        let mut wire = HttpWire::new(
            Endpoint::parse(&base)?,
            Secret::new("sk-secret-2"),
            AuthStyle::ApiKeyHeader,
        )?;
        let error = wire.post("/x", &[], &json!({}), None).err();
        let text = error.as_ref().map(ToString::to_string).unwrap_or_default();
        assert!(
            matches!(error, Some(Error::Api { status: 401, .. })),
            "{text}"
        );
        assert!(text.contains("bad key [redacted]"), "{text}");
        assert!(!text.contains("sk-secret-2"));
        let seen = handle.join().unwrap_or_default().to_ascii_lowercase();
        assert!(seen.contains("x-api-key: sk-secret-2"), "{seen}");
        Ok(())
    }

    #[test]
    fn a_redirect_is_not_followed() -> Result<()> {
        let (base, handle) = server(
            "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://192.0.2.1/steal\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                .to_owned(),
        )?;
        let mut wire = HttpWire::new(
            Endpoint::parse(&base)?,
            Secret::new("sk-secret-3"),
            AuthStyle::ApiKeyHeader,
        )?;
        let error = wire.post("/x", &[], &json!({}), None).err();
        assert!(
            matches!(error, Some(Error::Api { status: 307, .. })),
            "{error:?}"
        );
        let _ = handle.join();
        Ok(())
    }

    #[test]
    fn a_non_json_reply_is_an_error_not_a_panic() -> Result<()> {
        let (base, handle) = server(http("200 OK", "<html>gateway</html>"))?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        let error = wire.post("/x", &[], &json!({}), None).err();
        assert!(matches!(error, Some(Error::Protocol(_))), "{error:?}");
        let _ = handle.join();
        Ok(())
    }

    #[test]
    fn error_shapes_of_local_servers_are_read() {
        let v = json!({"error": "model not found"});
        assert_eq!(error_message(Some(&v), ""), "model not found");
        let v = json!({"error": {"message": "too long", "type": "invalid"}});
        assert_eq!(error_message(Some(&v), ""), "invalid: too long");
        assert_eq!(error_message(None, " bad gateway "), "bad gateway");
    }

    fn stream_of(wire: &mut HttpWire) -> Result<(Streamed, Vec<String>)> {
        let mut seen = Vec::new();
        let streamed = wire.post_stream("/s", &[], &json!({}), None, &mut |e| {
            seen.push(e.data);
            Ok(true)
        })?;
        Ok((streamed, seen))
    }

    #[test]
    fn a_real_stream_is_read_event_by_event_without_the_key() -> Result<()> {
        let events = "data: one sk-secret-4\n\ndata: two\n\n";
        let (base, handle) = server(format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{events}"
        ))?;
        let mut wire = HttpWire::new(
            Endpoint::parse(&base)?,
            Secret::new("sk-secret-4"),
            AuthStyle::Bearer,
        )?;
        let (streamed, seen) = stream_of(&mut wire)?;
        assert_eq!(streamed, Streamed::Done);
        assert_eq!(seen, vec!["one [redacted]", "two"]);
        let _ = handle.join();
        // A server that answers with one JSON reply instead.
        let (base, handle) = server(http("200 OK", r#"{"whole": true}"#))?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        let (streamed, seen) = stream_of(&mut wire)?;
        assert_eq!(streamed, Streamed::Whole(json!({"whole": true})));
        assert!(seen.is_empty());
        let _ = handle.join();
        // An error status is an error before any event.
        let (base, handle) = server(http("429 Too Many Requests", r#"{"error": "slow down"}"#))?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        let error = stream_of(&mut wire).err();
        assert!(
            matches!(error, Some(Error::Api { status: 429, .. })),
            "{error:?}"
        );
        let _ = handle.join();
        Ok(())
    }

    /// A server that never answers is waited for without end; cancelling the
    /// call ends the wait, streamed or not, and the next request is answered.
    #[test]
    fn cancelling_ends_the_wait_for_a_silent_server() -> Result<()> {
        use sparksmith_automation::cancel::Cancel;
        for streamed in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let base = format!("http://{}", listener.local_addr()?);
            let cancel = Cancel::default();
            let stop = cancel.clone();
            let (done, wait) = std::sync::mpsc::channel::<()>();
            let silent = std::thread::spawn(move || {
                let accepted = listener.accept();
                stop.set();
                let _ = wait.recv();
                drop(accepted);
            });
            let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
            cancel.scope(|| {
                if streamed {
                    let ended = stream_of(&mut wire)?;
                    assert_eq!(ended, (Streamed::Stopped, Vec::new()));
                } else {
                    assert!(wire.post("/s", &[], &json!({}), None).is_err());
                }
                Result::Ok(())
            })?;
            let _ = done.send(());
            let _ = silent.join();
            let (base, handle) = server(http("200 OK", r#"{"next": true}"#))?;
            let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
            let next = wire.post("/s", &[], &json!({}), None)?;
            assert_eq!(next, json!({"next": true}));
            let _ = handle.join();
        }
        Ok(())
    }

    /// Serve one event stream: `first`, then whatever `rest` does.
    fn stream_server(
        first: &'static [u8],
        rest: impl FnOnce(&mut std::net::TcpStream) + Send + 'static,
    ) -> Result<(String, std::thread::JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.read(&mut [0u8; 8192]);
            let _ = stream.write_all(first);
            rest(&mut stream);
        });
        Ok((base, handle))
    }

    const HEAD: &[u8] =
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";

    fn quiet(millis: u64) -> Silence {
        Silence {
            after: Some(Duration::from_millis(millis)),
            set_in: "the test",
        }
    }

    /// Reads until the client closes the connection.
    fn until_closed(stream: &mut std::net::TcpStream) {
        while stream.read(&mut [0u8; 64]).is_ok_and(|n| n > 0) {}
    }

    /// A server that never answers is given up at the limit, and one that
    /// goes silent after some of its reply too, with what came kept; the
    /// message names the limit and its place. The next request is answered.
    #[test]
    fn a_silent_server_is_given_up_at_the_limit() -> Result<()> {
        let (base, handle) = stream_server(b"", until_closed)?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        wire.set_silence(quiet(50));
        let given_up = |error: Error| match error {
            Error::Silent(why) => Ok(why),
            other => Err(other),
        };
        let why = match wire.post("/s", &[], &json!({}), None) {
            Ok(value) => return Err(Error::Protocol(value.to_string())),
            Err(error) => given_up(error)?,
        };
        assert!(why.contains("the limit set in the test"), "{why}");
        let _ = handle.join();

        let first = [HEAD, b"data: first\n\n".as_slice()].concat().leak();
        let (base, handle) = stream_server(first, until_closed)?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        wire.set_silence(quiet(50));
        let (streamed, seen) = stream_of(&mut wire)?;
        assert!(matches!(streamed, Streamed::Silent(_)), "{streamed:?}");
        assert_eq!(seen, ["first"]);
        let _ = handle.join();

        let (base, handle) = server(http("200 OK", r#"{"next": true}"#))?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        wire.set_silence(quiet(60_000));
        let next = wire.post("/s", &[], &json!({}), None)?;
        assert_eq!(next, json!({"next": true}));
        let _ = handle.join();
        Ok(())
    }

    /// A reply that keeps arriving is never cut, however long it takes in
    /// all: each piece comes well within the limit, the whole well past it.
    #[test]
    fn a_trickling_reply_is_never_cut() -> Result<()> {
        const PIECES: usize = 12;
        let (_pace, paced) = std::sync::mpsc::channel::<()>();
        let (base, handle) = stream_server(HEAD, move |stream| {
            for _ in 0..PIECES {
                // The server's own pace.
                let _ = paced.recv_timeout(Duration::from_millis(150));
                let _ = stream.write_all(b"data: piece\n\n");
            }
        })?;
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        wire.set_silence(quiet(1_000));
        let started = std::time::Instant::now();
        let (streamed, seen) = stream_of(&mut wire)?;
        assert_eq!(streamed, Streamed::Done);
        assert_eq!(seen.len(), PIECES);
        assert!(started.elapsed() > Duration::from_millis(1_000));
        let _ = handle.join();
        Ok(())
    }

    /// Both backends keep what a reply that went silent had said, count its
    /// tokens and name the limit; one that said nothing is the error.
    #[test]
    fn a_reply_that_went_silent_keeps_what_it_said() -> Result<()> {
        use crate::model::{ModelBackend, ModelProfile, ModelRequest, Part, StopReason};
        use std::sync::atomic::AtomicBool;
        let openai = "data: {\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"Half an answer\"}}]}\n\n";
        let anthropic = "data: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":40,\"output_tokens\":1}}}\n\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Half an answer\"}}\n\n";
        for (said, messages) in [(openai, false), (anthropic, true), ("", false), ("", true)] {
            let first = [HEAD, said.as_bytes()].concat().leak();
            let (base, handle) = stream_server(first, until_closed)?;
            let wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
            let mut profile = if messages {
                ModelProfile::anthropic("m")
            } else {
                ModelProfile::openai_compatible("m", 32_768)
            };
            profile.silence = quiet(50);
            let mut backend: Box<dyn ModelBackend> = if messages {
                Box::new(crate::model::anthropic::AnthropicBackend::new(
                    profile, wire,
                ))
            } else {
                Box::new(crate::model::openai::OpenAiBackend::new(profile, wire))
            };
            let request = ModelRequest {
                system: "",
                tools: &[],
                messages: &[],
                max_output_tokens: 100,
                timeout: None,
            };
            let ended = backend.stream(&request, &mut |_| {}, &AtomicBool::new(false));
            let _ = handle.join();
            if said.is_empty() {
                assert!(matches!(ended, Err(Error::Silent(_))), "{ended:?}");
                continue;
            }
            let reply = ended?;
            assert!(matches!(reply.stop, StopReason::Silent { .. }), "{reply:?}");
            assert_eq!(reply.content, [Part::text("Half an answer")]);
            assert!(reply.usage_estimated && reply.usage.output_tokens > 1);
        }
        Ok(())
    }

    #[test]
    fn stop_between_chunks_does_not_wait_for_the_rest() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let (go_on, wait) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut chunk = [0u8; 8192];
            let _ = stream.read(&mut chunk);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: first\n\n",
            );
            let _ = stream.flush();
            // The rest only comes once the test is done, or never.
            let _ = wait.recv_timeout(Duration::from_secs(20));
            let _ = stream.write_all(b"data: second\n\n");
        });
        let mut wire = HttpWire::new(Endpoint::parse(&base)?, None, AuthStyle::Bearer)?;
        let started = std::time::Instant::now();
        let mut seen = Vec::new();
        let streamed = wire.post_stream("/s", &[], &json!({}), None, &mut |e| {
            seen.push(e.data);
            Ok(false)
        })?;
        assert_eq!(streamed, Streamed::Stopped);
        assert_eq!(seen, vec!["first"]);
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = go_on.send(());
        let _ = handle.join();
        Ok(())
    }
}

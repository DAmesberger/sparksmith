//! The HTTP seam under the backends: one JSON POST in, one JSON value out,
//! or a stream of server-sent events. [`HttpWire`] is the only code that
//! holds a key at request time. It follows no redirects (a redirect could
//! carry the key elsewhere), bounds the response, and removes the key from
//! everything it returns.
use super::stream::{self, SseEvent, Streamed};
use super::{Endpoint, Secret};
use crate::{Error, Result};
use serde_json::Value;
use std::time::Duration;

/// Largest response body read.
const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;

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
    /// POST `body` to the endpoint's `path`.
    fn post(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Duration,
    ) -> Result<Value>;
    /// POST `body` and read the reply as server-sent events, giving each to
    /// `on` until the stream ends or `on` answers false; a server that
    /// answers with one JSON reply instead gives [`Streamed::Whole`].
    fn post_stream(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Duration,
        on: &mut dyn FnMut(SseEvent) -> Result<bool>,
    ) -> Result<Streamed>;
    /// `value` without the key, for a reply put together from many events:
    /// a key split across two of them is whole again here.
    fn clean_value(&self, value: Value) -> Value {
        value
    }
}

/// The real thing, over ureq.
pub struct HttpWire {
    agent: ureq::Agent,
    endpoint: Endpoint,
    key: Option<Secret>,
    auth: AuthStyle,
}

impl std::fmt::Debug for HttpWire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpWire")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl HttpWire {
    pub fn new(endpoint: Endpoint, key: Option<Secret>, auth: AuthStyle) -> Result<Self> {
        if key.is_some() {
            endpoint.check_key_allowed()?;
        }
        let mut config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0);
        if endpoint.is_loopback() {
            // Requests for this computer never go through a proxy.
            config = config.proxy(None);
        }
        Ok(Self {
            agent: config.build().into(),
            endpoint,
            key,
            auth,
        })
    }

    fn clean(&self, text: &str) -> String {
        match &self.key {
            Some(key) => key.redact(text),
            None => text.to_owned(),
        }
    }
}

impl Wire for HttpWire {
    fn label(&self) -> String {
        format!("{} ({})", self.endpoint.base(), self.endpoint.location())
    }

    fn post(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Duration,
    ) -> Result<Value> {
        let mut response = self.send(path, headers, body, timeout)?;
        let text = self.read_text(&mut response)?;
        reply_value(&text)
    }

    fn post_stream(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Duration,
        on: &mut dyn FnMut(SseEvent) -> Result<bool>,
    ) -> Result<Streamed> {
        let mut response = self.send(path, headers, body, timeout)?;
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
                        stream::read_sse(&mut text.as_bytes(), &|line| line.to_owned(), on)?;
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
        // Dropping the reader on a stop closes the connection.
        let stopped = stream::read_sse(&mut reader, &clean, on)
            .map_err(|e| Error::Http(self.clean(&e.to_string())))?;
        Ok(if stopped {
            Streamed::Stopped
        } else {
            Streamed::Done
        })
    }

    fn clean_value(&self, value: Value) -> Value {
        let Some(key) = &self.key else {
            return value;
        };
        let text = value.to_string();
        let cleaned = key.redact(&text);
        if cleaned == text {
            return value;
        }
        serde_json::from_str(&cleaned).unwrap_or(value)
    }
}

impl HttpWire {
    /// Send one POST: the response with a success status, or the error.
    fn send(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let url = format!("{}{path}", self.endpoint.base());
        let mut request = self
            .agent
            .post(&url)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .header("content-type", "application/json");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(key) = &self.key {
            request = match self.auth {
                AuthStyle::ApiKeyHeader => request.header("x-api-key", key.expose()),
                AuthStyle::Bearer => {
                    request.header("authorization", format!("Bearer {}", key.expose()))
                }
            };
        }
        let mut response = request
            .send_json(body)
            .map_err(|e| Error::Http(self.clean(&e.to_string())))?;
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
                message: error_message(value.as_ref().ok(), &text),
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
            .map_err(|e| Error::Http(self.clean(&e.to_string())))?;
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
pub type Post = (String, Vec<(String, String)>, Value, Duration);

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
        timeout: Duration,
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
        timeout: Duration,
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

    const LONG: Duration = Duration::from_secs(20);

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
        let reply = wire.post("/chat", &[("x-extra", "1")], &json!({"a": 1}), LONG)?;
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
        let error = wire.post("/x", &[], &json!({}), LONG).err();
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
        let error = wire.post("/x", &[], &json!({}), LONG).err();
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
        let error = wire.post("/x", &[], &json!({}), LONG).err();
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
        let streamed = wire.post_stream("/s", &[], &json!({}), LONG, &mut |e| {
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
        let streamed = wire.post_stream("/s", &[], &json!({}), LONG, &mut |e| {
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

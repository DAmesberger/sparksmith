//! Recording and replay, in one neutral format: JSON lines of
//! [`ModelReply`]. A recording made on any backend replays offline, with no
//! key and no network. Requests carry the datasheets and are not written;
//! the key is never part of a reply. A workflow run's recording starts with
//! one line naming the workflow, its version and the digest of its files
//! ([`WorkflowMark`]), so a replay against another version is refused.
use super::{Delta, Message, ModelBackend, ModelReply, ModelRequest, ToolSpec};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// What a replay kept of a request, for tests to inspect.
#[derive(Clone, Debug, PartialEq)]
pub struct KeptRequest {
    pub system: String,
    pub tools: Vec<ToolSpec>,
    pub messages: Vec<Message>,
    pub max_output_tokens: u32,
    pub timeout: Duration,
}

/// The workflow a recording was made with: its id, version and the
/// `sha256:` digest of its files (`workflow::load::digest`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowMark {
    pub id: String,
    pub version: String,
    pub digest: String,
}

/// A recording's first line when it is a workflow run's.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    workflow: WorkflowMark,
}

/// Recorded replies, answered in order.
#[derive(Debug, Default)]
pub struct Replay {
    /// The workflow the recording names, when it is a workflow run's.
    pub workflow: Option<WorkflowMark>,
    replies: VecDeque<ModelReply>,
    source: String,
    /// Keep each request (they hold the datasheets, so off for real files).
    pub keep_requests: bool,
    pub requests: Vec<KeptRequest>,
    /// What a token count answers.
    pub prompt_tokens: Option<u64>,
    /// Set once a reply naming an old tool was answered, so the run keeps
    /// reading old names after it.
    legacy: bool,
}

impl Replay {
    pub fn new(replies: Vec<ModelReply>) -> Self {
        Self {
            replies: replies.into(),
            source: "a test script".into(),
            keep_requests: true,
            ..Self::default()
        }
    }

    /// Replies from a recording.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut replies = Vec::new();
        let mut workflow = None;
        for (k, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            if replies.is_empty()
                && workflow.is_none()
                && let Ok(header) = serde_json::from_str::<Header>(line)
            {
                workflow = Some(header.workflow);
                continue;
            }
            replies.push(serde_json::from_str(line).map_err(|e| {
                Error::Config(format!(
                    "{} line {} is not a neutral reply: {e}",
                    path.display(),
                    k + 1
                ))
            })?);
        }
        Ok(Self {
            replies: replies.into(),
            source: path.display().to_string(),
            workflow,
            ..Self::default()
        })
    }
}

impl ModelBackend for Replay {
    fn label(&self) -> String {
        format!(
            "a replay of {} (no API calls; usage as recorded)",
            self.source
        )
    }

    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        if self.keep_requests {
            self.requests.push(KeptRequest {
                system: request.system.to_owned(),
                tools: request.tools.to_vec(),
                messages: request.messages.to_vec(),
                max_output_tokens: request.max_output_tokens,
                timeout: request.timeout,
            });
        }
        let reply = self
            .replies
            .pop_front()
            .ok_or_else(|| Error::Http("the recording has no more responses".into()))?;
        self.legacy |= reply.content.iter().any(|part| {
            matches!(part, super::Part::ToolCall { name, .. }
                if !crate::verbs::NAMES.contains(&name.as_str()))
        });
        Ok(reply)
    }

    fn count_prompt_tokens(&mut self, _request: &ModelRequest<'_>) -> Result<Option<u64>> {
        Ok(self.prompt_tokens)
    }

    fn recorded_workflow(&self) -> Option<&WorkflowMark> {
        self.workflow.as_ref()
    }

    fn legacy_tools(&self) -> bool {
        self.replies.iter().any(|reply| {
            reply.content.iter().any(|part| {
                matches!(part, super::Part::ToolCall { name, .. }
                    if !crate::verbs::NAMES.contains(&name.as_str()))
            })
        }) || self.legacy
    }
}

/// Wraps a backend and appends each reply to a JSON-lines file, the format
/// [`Replay::from_file`] reads.
pub struct Recording<B> {
    pub inner: B,
    path: PathBuf,
}

impl<B> Recording<B> {
    pub fn new(inner: B, path: PathBuf) -> Self {
        Self { inner, path }
    }

    /// A workflow run's recording: a new file at `path` whose first line
    /// names `workflow`. An existing file is refused.
    pub fn for_workflow(inner: B, path: PathBuf, workflow: WorkflowMark) -> Result<Self> {
        let line = serde_json::to_string(&Header { workflow })?;
        crate::write_new(&path, format!("{line}\n").as_bytes())?;
        Ok(Self { inner, path })
    }

    fn append(&self, reply: &ModelReply) -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", serde_json::to_string(reply)?)?;
        Ok(())
    }
}

impl<B: ModelBackend + ?Sized> ModelBackend for Box<B> {
    fn label(&self) -> String {
        (**self).label()
    }

    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        (**self).send(request)
    }

    fn stream(
        &mut self,
        request: &ModelRequest<'_>,
        deltas: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<ModelReply> {
        (**self).stream(request, deltas, cancel)
    }

    fn count_prompt_tokens(&mut self, request: &ModelRequest<'_>) -> Result<Option<u64>> {
        (**self).count_prompt_tokens(request)
    }

    fn legacy_tools(&self) -> bool {
        (**self).legacy_tools()
    }

    fn recorded_workflow(&self) -> Option<&WorkflowMark> {
        (**self).recorded_workflow()
    }
}

impl<B: ModelBackend> ModelBackend for Recording<B> {
    fn label(&self) -> String {
        format!(
            "{}, recorded to {}",
            self.inner.label(),
            self.path.display()
        )
    }

    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        let reply = self.inner.send(request)?;
        self.append(&reply)?;
        Ok(reply)
    }

    fn stream(
        &mut self,
        request: &ModelRequest<'_>,
        deltas: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<ModelReply> {
        // A stopped reply is recorded too: its replay stops where it did.
        let reply = self.inner.stream(request, deltas, cancel)?;
        self.append(&reply)?;
        Ok(reply)
    }

    fn count_prompt_tokens(&mut self, request: &ModelRequest<'_>) -> Result<Option<u64>> {
        self.inner.count_prompt_tokens(request)
    }
}

#[cfg(test)]
mod tests {
    use super::super::wire::{AuthStyle, HttpWire};
    use super::super::{Endpoint, ModelProfile, Part, Secret, StopReason, openai};
    use super::*;
    use crate::usage::Usage;
    use std::io::Read;

    fn reply(id: &str) -> ModelReply {
        ModelReply {
            model: "m".into(),
            content: vec![Part::text(id)],
            stop: StopReason::EndTurn,
            usage: Usage::default(),
            usage_estimated: false,
        }
    }

    fn request<'a>() -> ModelRequest<'a> {
        ModelRequest {
            system: "S",
            tools: &[],
            messages: &[],
            max_output_tokens: 1,
            timeout: Duration::from_secs(20),
        }
    }

    #[test]
    fn replay_answers_in_order_and_keeps_requests() -> Result<()> {
        let mut r = Replay::new(vec![reply("1"), reply("2")]);
        assert_eq!(r.send(&request())?.content, vec![Part::text("1")]);
        assert_eq!(r.send(&request())?.content, vec![Part::text("2")]);
        assert!(r.send(&request()).is_err());
        assert_eq!(r.requests.len(), 3);
        Ok(())
    }

    #[test]
    fn a_recording_replays_and_holds_no_key_or_request() -> Result<()> {
        // A real HTTP round trip through the OpenAI-compatible backend, with
        // a key, on this computer; the server echoes the key in its reply.
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = std::thread::spawn(move || {
            use std::io::Write as _;
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut seen = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stream.read(&mut chunk) {
                seen.extend_from_slice(chunk.get(..n).unwrap_or_default());
                if n == 0 || String::from_utf8_lossy(&seen).contains("SYSTEM-END") {
                    break;
                }
            }
            let body = r#"{"model":"m","choices":[{"message":{"content":"key is sk-rec-9"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":6}}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        let wire = HttpWire::new(
            Endpoint::parse(&base)?,
            Secret::new("sk-rec-9"),
            AuthStyle::Bearer,
        )?;
        let backend = openai::OpenAiBackend::new(ModelProfile::openai_compatible("m", 1_000), wire);
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("r.jsonl");
        let mut recording = Recording::new(backend, path.clone());
        let mut asked = request();
        asked.system = "DATASHEET-TEXT SYSTEM-END";
        let reply = recording.send(&asked)?;
        let _ = server.join();
        assert_eq!(reply.usage.output_tokens, 6);
        let mut text = String::new();
        std::fs::File::open(&path)?.read_to_string(&mut text)?;
        assert!(!text.contains("sk-rec-9"), "{text}");
        assert!(text.contains("key is [redacted]"), "{text}");
        assert!(!text.contains("DATASHEET-TEXT"), "{text}");
        assert!(!recording.label().contains("sk-rec-9"));
        let mut replay = Replay::from_file(&path)?;
        assert_eq!(replay.send(&request())?, reply);
        Ok(())
    }

    #[test]
    fn a_workflow_recording_names_its_workflow_and_replays() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("w.jsonl");
        let mark = WorkflowMark {
            id: "datasheet-to-component".into(),
            version: "1.0".into(),
            digest: "sha256:00".into(),
        };
        let mut recording =
            Recording::for_workflow(Replay::new(vec![reply("1")]), path.clone(), mark.clone())?;
        recording.send(&request())?;
        assert!(Recording::for_workflow(Replay::default(), path.clone(), mark.clone()).is_err());
        let mut replay = Replay::from_file(&path)?;
        assert_eq!(replay.recorded_workflow(), Some(&mark));
        assert_eq!(replay.send(&request())?.content, vec![Part::text("1")]);
        assert!(replay.send(&request()).is_err());
        Ok(())
    }

    #[test]
    fn a_line_in_another_format_is_refused_by_name() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("old.jsonl");
        std::fs::write(&path, "{\"content\": [], \"stop_reason\": \"end_turn\"}\n")?;
        let error = Replay::from_file(&path).err().map(|e| e.to_string());
        assert!(
            error.as_deref().is_some_and(|e| e.contains("line 1")),
            "{error:?}"
        );
        Ok(())
    }
}

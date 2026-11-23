//! Model and tool activity share a JSON-lines trace. A workflow or task names
//! its version and fixture digest in its first row. Input prompts are never recorded.
pub mod trace;
use super::{Delta, Message, ModelBackend, ModelReply, ModelRequest, ToolSpec};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// Maximum size of a recording accepted into the in-memory replay queue.
const MAX_RECORDING_BYTES: u64 = 256 * 1024 * 1024;

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
    /// A failed backend attempt occupies its original request slot.
    replies: VecDeque<Option<ModelReply>>,
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
            replies: replies.into_iter().map(Some).collect(),
            source: "a test script".into(),
            keep_requests: true,
            ..Self::default()
        }
    }

    /// Whether every neutral reply carries metered usage from its original provider.
    pub fn reports_usage(&self) -> bool {
        !self.replies.is_empty()
            && self
                .replies
                .iter()
                .all(|r| r.as_ref().is_some_and(|reply| !reply.usage_estimated))
    }

    /// Replies from a recording.
    pub fn from_file(path: &Path) -> Result<Self> {
        Self::load(path, None)
    }

    /// Load a complete benchmark trace, bound to this task and all of its fixtures.
    pub fn from_run(path: &Path, expected: &WorkflowMark) -> Result<Self> {
        Self::load(path, Some(expected))
    }

    fn load(path: &Path, expected: Option<&WorkflowMark>) -> Result<Self> {
        let mut replies = Vec::new();
        let mut workflow = None;
        let mut finished = false;
        let mut bench = false;
        let mut reader = BufReader::new(std::fs::File::open(path)?.take(MAX_RECORDING_BYTES + 1));
        let mut line = String::new();
        let mut k = 0;
        while reader.read_line(&mut line)? > 0 {
            if reader.get_ref().limit() == 0 && reader.buffer().is_empty() {
                return Err(Error::Config(format!(
                    "{}: recording exceeds {MAX_RECORDING_BYTES} bytes",
                    path.display()
                )));
            }
            k += 1;
            if line.trim().is_empty() {
                line.clear();
                continue;
            }
            let invalid =
                |why: &str| Error::Config(format!("{} line {}: {why}", path.display(), k));
            if finished {
                return Err(invalid("unexpected row after the end of the run"));
            }
            let value: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| invalid(&e.to_string()))?;
            if value.get("record").is_some() {
                let row: trace::Row =
                    serde_json::from_value(value).map_err(|e| invalid(&e.to_string()))?;
                match row {
                    trace::Row::Start {
                        workflow: mark,
                        runner,
                        ..
                    } => {
                        bench = runner != "workflow";
                        if workflow.is_some() || !replies.is_empty() || k != 1 {
                            return Err(invalid("unexpected run header"));
                        }
                        workflow = Some(mark);
                    }
                    trace::Row::Model { reply, .. } => replies.push(Some(reply)),
                    trace::Row::Call(_) => (),
                    trace::Row::ModelError { .. } => replies.push(None),
                    trace::Row::Finish { .. } => finished = true,
                }
            } else if workflow.is_none() && replies.is_empty() && value.get("workflow").is_some() {
                // Workflow recordings predating combined run traces used an untagged header.
                let old: Header =
                    serde_json::from_value(value).map_err(|e| invalid(&e.to_string()))?;
                workflow = Some(old.workflow);
            } else {
                // Their remaining rows were bare neutral replies.
                replies.push(Some(
                    serde_json::from_value(value).map_err(|e| invalid(&e.to_string()))?,
                ));
            }
            line.clear();
        }
        if let Some(mark) = expected {
            if workflow.as_ref() != Some(mark) {
                return Err(Error::Config(
                    "the recording belongs to a different task, version or fixture".into(),
                ));
            }
            if bench && !finished {
                return Err(Error::Config("incomplete run trace".into()));
            }
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
            .ok_or_else(|| Error::Http("the recording has no more responses".into()))?
            .ok_or_else(|| Error::Http("the recorded model request failed".into()))?;
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
        self.replies.iter().flatten().any(|reply| {
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
        let line = serde_json::to_string(&trace::Row::Start {
            workflow,
            runner: "workflow".into(),
            detail: String::new(),
        })?;
        crate::write_new(&path, format!("{line}\n").as_bytes())?;
        Ok(Self { inner, path })
    }

    fn append(
        &self,
        request: &ModelRequest<'_>,
        started: Instant,
        result: &Result<ModelReply>,
    ) -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let row = trace::Row::model(
            request.prompt_bytes()?,
            started.elapsed().as_secs_f64() * 1000.0,
            result,
        );
        serde_json::to_writer(&mut file, &row)?;
        writeln!(file)?;
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
        let started = Instant::now();
        let reply = self.inner.send(request);
        self.append(request, started, &reply)?;
        reply
    }

    fn stream(
        &mut self,
        request: &ModelRequest<'_>,
        deltas: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<ModelReply> {
        // A stopped reply is recorded too: its replay stops where it did.
        let started = Instant::now();
        let reply = self.inner.stream(request, deltas, cancel);
        self.append(request, started, &reply)?;
        reply
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
    fn old_records_and_new_records_replay_in_the_same_reader() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("legacy.jsonl");
        let mark = WorkflowMark {
            id: "generic".into(),
            version: "1".into(),
            digest: "sha256:00".into(),
        };
        let old = format!(
            "{}\n{}\n",
            serde_json::to_string(&Header {
                workflow: mark.clone()
            })?,
            serde_json::to_string(&reply("legacy"))?
        );
        std::fs::write(&path, old)?;
        let mut replay = Replay::from_run(&path, &mark)?;
        assert_eq!(replay.send(&request())?.content, vec![Part::text("legacy")]);
        let incomplete = trace::Row::Start {
            workflow: mark.clone(),
            runner: "scripted".into(),
            detail: "input".into(),
        };
        std::fs::write(&path, serde_json::to_vec(&incomplete)?)?;
        assert!(Replay::from_run(&path, &mark).is_err());
        Ok(())
    }

    #[test]
    fn a_failed_attempt_keeps_its_place_between_neutral_replies() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("failed.jsonl");
        let mut file = std::fs::File::create_new(&path)?;
        for row in [
            trace::Row::ModelError { bytes: 3, ms: 0.1 },
            trace::Row::Model {
                bytes: 5,
                ms: 0.2,
                reply: reply("second"),
            },
        ] {
            writeln!(file, "{}", serde_json::to_string(&row)?)?;
        }
        let mut replay = Replay::from_file(&path)?;
        assert!(replay.send(&request()).is_err());
        assert_eq!(replay.send(&request())?.content, vec![Part::text("second")]);
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

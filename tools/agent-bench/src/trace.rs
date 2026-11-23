//! A run's neutral model replies and verb calls in their observed order.
use crate::Result;
use sparksmith_assist::model::Delta;
pub use sparksmith_assist::model::record::trace::{Metrics, Row};
use sparksmith_assist::model::{
    ModelBackend, ModelReply, ModelRequest,
    record::{Replay, WorkflowMark},
};
use sparksmith_assist::{CallState, Event};
pub use sparksmith_automation::mcp::trace::{Call, Transport, digest, entry};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

pub struct Trace {
    writer: RefCell<BufWriter<File>>,
    pub metrics: RefCell<Metrics>,
}

impl Trace {
    pub fn create(path: &Path, start: &Row) -> Result<Self> {
        let trace = Self {
            writer: RefCell::new(BufWriter::new(File::create_new(path)?)),
            metrics: Default::default(),
        };
        trace.write(start)?;
        Ok(trace)
    }
    /// Count a later identical call as a retry after its previous refusal.
    pub fn call(
        &self,
        key: String,
        row: Call,
        refused: &mut BTreeSet<String>,
    ) -> std::io::Result<()> {
        let mut m = self.metrics.borrow_mut();
        *m.calls.entry(row.verb.clone()).or_default() += 1;
        m.server_ms += row.server_ms;
        m.visible_reply_bytes += row.visible_bytes;
        if refused.remove(&key) {
            m.retries += 1;
        }
        if !row.ok {
            m.refusals += 1;
            refused.insert(key);
        }
        self.write(&Row::Call(row))
    }

    pub fn write(&self, row: &Row) -> std::io::Result<()> {
        let mut writer = self.writer.borrow_mut();
        serde_json::to_writer(&mut *writer, row)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

/// Join a transport observation to the reply actually exposed by the harness.
pub struct Observer<'a> {
    trace: &'a Trace,
    pending: &'a RefCell<Option<(String, Call)>>,
    transport: Transport,
    refused: BTreeSet<String>,
}

impl<'a> Observer<'a> {
    pub fn new(
        trace: &'a Trace,
        pending: &'a RefCell<Option<(String, Call)>>,
        transport: Transport,
    ) -> Self {
        Self {
            trace,
            pending,
            transport,
            refused: BTreeSet::new(),
        }
    }

    pub fn event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Request { prompt_tokens, .. } => {
                self.trace.metrics.borrow_mut().estimated_prompt_tokens += prompt_tokens;
            }
            Event::Call {
                request,
                call,
                name,
                state,
                ..
            } => {
                let visible = match state {
                    CallState::Done { reply, .. } | CallState::Refused { reply } => reply,
                    _ => return Ok(()),
                };
                let (key, mut row) = self.pending.take().unwrap_or_else(|| {
                    // Chat can refuse malformed JSON before invoking its transport.
                    let fingerprint = digest(visible.as_bytes());
                    let key = format!("{name}/{fingerprint}");
                    (
                        key,
                        Call {
                            transport: self.transport,
                            request: None,
                            call: None,
                            verb: name,
                            entry: String::new(),
                            ok: false,
                            visible_bytes: 0,
                            request_bytes: 0,
                            reply_bytes: 0,
                            result_sha256: fingerprint,
                            server_ms: 0.0,
                        },
                    )
                });
                row.request = Some(request);
                row.call = Some(call);
                row.visible_bytes = visible.len();
                self.trace.call(key, row, &mut self.refused)?;
            }
            _ => (),
        }
        Ok(())
    }
}

/// Only a trace made for the same task revision and fixture can replay.
pub fn replay(path: &Path, mark: &WorkflowMark) -> Result<Replay> {
    Ok(Replay::from_run(path, mark)?)
}

pub struct Measured<'a> {
    pub backend: Box<dyn ModelBackend>,
    pub trace: &'a Trace,
}

impl ModelBackend for Measured<'_> {
    fn label(&self) -> String {
        self.backend.label()
    }
    fn send(&mut self, req: &ModelRequest<'_>) -> sparksmith_assist::Result<ModelReply> {
        self.measure(req, |backend| backend.send(req))
    }

    fn stream(
        &mut self,
        req: &ModelRequest<'_>,
        deltas: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> sparksmith_assist::Result<ModelReply> {
        self.measure(req, |backend| backend.stream(req, deltas, cancel))
    }

    fn recorded_workflow(&self) -> Option<&WorkflowMark> {
        self.backend.recorded_workflow()
    }

    fn legacy_tools(&self) -> bool {
        self.backend.legacy_tools()
    }

    fn count_prompt_tokens(
        &mut self,
        req: &ModelRequest<'_>,
    ) -> sparksmith_assist::Result<Option<u64>> {
        self.backend.count_prompt_tokens(req)
    }
}

impl Measured<'_> {
    fn measure(
        &mut self,
        req: &ModelRequest<'_>,
        send: impl FnOnce(&mut dyn ModelBackend) -> sparksmith_assist::Result<ModelReply>,
    ) -> sparksmith_assist::Result<ModelReply> {
        let bytes = req.prompt_bytes()?;
        let started = Instant::now();
        let result = send(&mut *self.backend);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut metrics = self.trace.metrics.borrow_mut();
        metrics.model_requests += 1;
        metrics.model_request_bytes += bytes;
        metrics.model_ms += ms;
        if let Ok(reply) = &result {
            metrics.estimated_output_tokens +=
                sparksmith_assist::usage::estimated_output_tokens(&reply.content);
        }
        self.trace.write(&Row::model(bytes, ms, &result))?;
        result
    }
}

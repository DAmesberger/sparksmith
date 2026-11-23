//! Optional JSON-lines tap on the server's own stdio. It sees the exact
//! bytes accepted by the pipes, including framing, and logs only tool calls.
use super::trace::{Call, Transport, digest, entry};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Write},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Instant,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct Pending {
    verb: String,
    entry: String,
    request_bytes: usize,
    started: Instant,
}
struct State {
    file: File,
    pending: BTreeMap<String, Pending>,
}
#[derive(Clone)]
pub(super) struct Tap(Arc<Mutex<State>>);
impl Tap {
    pub(super) fn new(file: File) -> Self {
        Self(Arc::new(Mutex::new(State {
            file,
            pending: BTreeMap::new(),
        })))
    }
    pub(super) fn stream<T>(&self, io: T) -> Tapped<T> {
        Tapped {
            io,
            tap: self.clone(),
            buffer: Vec::new(),
            overlong: false,
        }
    }
    fn line(&self, bytes: &[u8], incoming: bool) -> io::Result<()> {
        let Ok(line) = serde_json::from_slice::<Value>(bytes) else {
            return Ok(());
        };
        let Some(id) = line
            .get("id")
            .filter(|id| id.is_number() || id.is_string())
            .map(Value::to_string)
        else {
            return Ok(());
        };
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("bench trace mutex poisoned"))?;
        if incoming {
            if line["method"] == "tools/call" {
                if state.pending.len() >= super::desk::QUEUE || state.pending.contains_key(&id) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the bench trace received too many pending calls or a reused request id",
                    ));
                }
                let params = &line["params"];
                state.pending.insert(
                    id,
                    Pending {
                        verb: params["name"].as_str().unwrap_or_default().to_owned(),
                        entry: entry(&params["arguments"]),
                        request_bytes: bytes.len(),
                        started: Instant::now(),
                    },
                );
            }
            return Ok(());
        }
        let Some(input) = state.pending.remove(&id) else {
            return Ok(());
        };
        let ms = input.started.elapsed().as_secs_f64() * 1000.0;
        let result = &line["result"];
        let mut visible = String::new();
        if let Some(content) = result["content"].as_array() {
            for block in content {
                if let Some(text) = block["text"].as_str() {
                    visible.push_str(text);
                }
            }
        } else if let Some(error) = line.get("error") {
            visible = error.to_string();
        }
        let row = Call {
            transport: Transport::Mcp,
            request: None,
            call: None,
            verb: input.verb,
            entry: input.entry,
            request_bytes: input.request_bytes,
            reply_bytes: bytes.len(),
            visible_bytes: visible.len(),
            result_sha256: digest(visible.as_bytes()),
            server_ms: ms,
            ok: line.get("error").is_none() && result["isError"] != true,
        };
        #[derive(Serialize)]
        struct TraceRow<'a> {
            record: &'static str,
            #[serde(flatten)]
            payload: &'a Call,
        }
        let mut line = serde_json::to_vec(&TraceRow {
            record: "call",
            payload: &row,
        })?;
        line.push(b'\n');
        state.file.write_all(&line)
    }
}
pub(super) struct Tapped<T> {
    io: T,
    tap: Tap,
    buffer: Vec<u8>,
    overlong: bool,
}
impl<T> Tapped<T> {
    fn observe(&mut self, bytes: &[u8], incoming: bool) -> io::Result<()> {
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            // Match the MCP parser's bound. It decides what to do with
            // invalid frames; the tap merely ignores them.
            if self.buffer.len().saturating_add(part.len()) > super::MAX_LINE + 1 {
                self.overlong = true;
            }
            if !self.overlong {
                self.buffer.extend_from_slice(part);
            }
            if part.ends_with(b"\n") {
                if !self.overlong {
                    self.tap.line(&self.buffer, incoming)?;
                }
                self.buffer.clear();
                self.overlong = false;
            }
        }
        Ok(())
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for Tapped<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let n = buf.filled().len();
        match Pin::new(&mut self.io).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => Poll::Ready(self.observe(&buf.filled()[n..], true)),
            other => other,
        }
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Tapped<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.io).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => Poll::Ready(self.observe(&buf[..n], false).map(|()| n)),
            other => other,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn counts_fragmented_frames_and_pairs_out_of_order_responses_by_id()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("wire.jsonl");
        let tap = Tap::new(File::create_new(&file)?);
        let mut incoming = tap.stream(());
        let mut outgoing = tap.stream(());
        let first=b"{ \"jsonrpc\":\"2.0\", \"id\":1, \"method\":\"tools/call\", \"params\":{\"name\":\"describe\",\"arguments\":{\"name\":\"project.save\"}}}\r\n";
        let second=b"{\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query\",\"arguments\":{\"name\":\"project.inspect\"}}}\n";
        incoming.observe(b"{\"id\":3,\"method\":\"server/discover\"}\n", true)?;
        for fragment in first.chunks(7) {
            incoming.observe(fragment, true)?;
        }
        incoming.observe(second, true)?;
        let content = "é";
        let reply = format!(
            "{}\n",
            json!({"id":2,"result":{"content":[{"type":"text","text":content}]}})
        );
        outgoing.observe(reply.as_bytes(), false)?;
        outgoing.observe(b"{\"id\":1,\"error\":{\"code\":-1}}\n", false)?;
        let rows = std::fs::read_to_string(file)?
            .lines()
            .map(serde_json::from_str)
            .collect::<serde_json::Result<Vec<Value>>>()?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["request_bytes"], second.len());
        assert_eq!(rows[0]["reply_bytes"], reply.len());
        assert_eq!(rows[0]["entry"], "project.inspect");
        assert_eq!(rows[0]["visible_bytes"], content.len());
        assert_eq!(rows[0]["result_sha256"], digest(content.as_bytes()));
        assert_eq!(rows[1]["request_bytes"], first.len());
        assert_eq!(rows[1]["ok"], false);
        assert_eq!(rows[1]["transport"], "mcp");
        Ok(())
    }
}

//! The wire log (`sparksmith mcp --wire-log NEW_FILE`): every line the
//! server reads from its client and every line it writes back, byte for
//! byte, with its direction and the time the server's own pipe saw it. It
//! wraps the same pipes the transport reads and writes, so it sees what
//! the client sent and what the server answered whatever the server makes
//! of it, and it changes neither.
//!
//! The file is JSON lines:
//!
//! - **The first row** is `{"record":"start", …}`: the format, the server's
//!   name and version, the protocol, and how the server was started
//!   (permission mode, allowed folders, the project file; `live` for the
//!   window). The server takes no key and the row holds no environment.
//! - **Every other row** is one line of one direction:
//!   `{"record":"in"|"out","n":N,"t_ms":T,"text":"…"}`. `in` is from the
//!   client. `n` counts rows from 1 in the order the server saw them and
//!   `t_ms` is the wall clock in milliseconds since 1970-01-01 UTC.
//!   `text` is the line without its `\n` (a `\r` before it stays). Bytes
//!   that are not UTF-8 are given as `"hex"` instead. `"nl":false` marks
//!   bytes no newline followed: a piece of a line longer than
//!   [`MAX_PIECE`], which the next row of that direction continues, or
//!   what a pipe held when it closed.
//!
//! Each direction's rows in order, with `\n` after every row whose `nl` is
//! not `false`, are that direction's exact bytes ([`bytes`]).
//!
//! A row is written before the next read or write returns, so a failure
//! to write the log stops the server rather than leaving an unrecorded
//! message.
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The log's format, in its first row.
pub const FORMAT: u32 = 1;
/// The most bytes one row holds: the transport's longest line and its
/// newline, so every message the server accepts is one row.
pub const MAX_PIECE: usize = super::MAX_LINE + 1;

/// How the server was started, for the first row.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Started {
    /// The project in the open window is served (`mcp --live`).
    pub live: bool,
    /// The permission mode; none live, where the window decides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The folders paths are limited to, as granted.
    pub folders: Vec<String>,
    /// The project file the server was started on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// A direction of the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Direction {
    In,
    Out,
}

struct Sink {
    file: File,
    n: u64,
}

/// The open log; cheap to clone for the two directions.
#[derive(Clone)]
pub(super) struct WireLog(Arc<Mutex<Sink>>);

impl WireLog {
    /// A new log at `path`, which must not exist, with its first row.
    pub(super) fn create(path: &Path, started: &Started) -> io::Result<Self> {
        #[derive(Serialize)]
        struct Start<'a> {
            record: &'static str,
            format: u32,
            t_ms: u64,
            server: &'static str,
            version: &'static str,
            protocol: Value,
            #[serde(flatten)]
            started: &'a Started,
        }
        let mut file = File::create_new(path)?;
        let mut row = serde_json::to_vec(&Start {
            record: "start",
            format: FORMAT,
            t_ms: now_ms(),
            server: super::NAME,
            version: env!("CARGO_PKG_VERSION"),
            protocol: serde_json::to_value(super::VERSIONS)?,
            started,
        })?;
        row.push(b'\n');
        file.write_all(&row)?;
        Ok(Self(Arc::new(Mutex::new(Sink { file, n: 0 }))))
    }

    /// `io` with everything read from it (`In`) or written to it (`Out`)
    /// recorded.
    pub(super) fn stream<T>(&self, io: T, direction: Direction) -> Recorded<T> {
        Recorded {
            io,
            log: self.clone(),
            direction,
            line: Vec::new(),
        }
    }

    /// One row: `bytes` of `direction`, followed by a newline or not.
    fn row(&self, direction: Direction, bytes: &[u8], newline: bool) -> io::Result<()> {
        #[derive(Serialize)]
        struct Row<'a> {
            record: &'static str,
            n: u64,
            t_ms: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            text: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            hex: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            nl: Option<bool>,
        }
        let text = std::str::from_utf8(bytes).ok();
        let mut sink = self
            .0
            .lock()
            .map_err(|_| io::Error::other("wire log mutex poisoned"))?;
        sink.n = sink.n.saturating_add(1);
        let mut row = serde_json::to_vec(&Row {
            record: match direction {
                Direction::In => "in",
                Direction::Out => "out",
            },
            n: sink.n,
            t_ms: now_ms(),
            text,
            hex: text
                .is_none()
                .then(|| bytes.iter().map(|b| format!("{b:02x}")).collect()),
            nl: (!newline).then_some(false),
        })?;
        row.push(b'\n');
        sink.file.write_all(&row)
    }
}

/// The wall clock in milliseconds since 1970-01-01 UTC; 0 for a clock set
/// before it.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_millis()).ok())
        .unwrap_or_default()
}

/// A pipe whose bytes are recorded as they pass.
pub(super) struct Recorded<T> {
    io: T,
    log: WireLog,
    direction: Direction,
    /// The line being read or written, up to its newline.
    line: Vec<u8>,
}

impl<T> Recorded<T> {
    fn observe(&mut self, bytes: &[u8]) -> io::Result<()> {
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            let (content, newline) = match part.split_last() {
                Some((b'\n', content)) => (content, true),
                _ => (part, false),
            };
            let mut rest = content;
            // A line longer than a row holds goes out in pieces, in
            // bounded space.
            while self.line.len().saturating_add(rest.len()) > MAX_PIECE {
                let room = MAX_PIECE.saturating_sub(self.line.len());
                let (head, tail) = rest.split_at(room.min(rest.len()));
                self.line.extend_from_slice(head);
                self.log.row(self.direction, &self.line, false)?;
                self.line.clear();
                rest = tail;
            }
            self.line.extend_from_slice(rest);
            if newline {
                self.log.row(self.direction, &self.line, true)?;
                self.line.clear();
            }
        }
        Ok(())
    }

    /// What the pipe held without a newline when it closed.
    fn close(&mut self) -> io::Result<()> {
        if self.line.is_empty() {
            return Ok(());
        }
        let line = std::mem::take(&mut self.line);
        self.log.row(self.direction, &line, false)
    }
}

impl<T> Drop for Recorded<T> {
    fn drop(&mut self) {
        // Nothing can report a failure here; the rows before stay.
        let _ = self.close();
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Recorded<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        match Pin::new(&mut self.io).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let read = buf.filled().get(before..).unwrap_or_default();
                // Nothing read into a buffer with room is the end.
                let recorded = if read.is_empty() && buf.remaining() > 0 {
                    self.close()
                } else {
                    self.observe(read)
                };
                Poll::Ready(recorded)
            }
            other => other,
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Recorded<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.io).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                let written = buf.get(..n).unwrap_or_default();
                Poll::Ready(self.observe(written).map(|()| n))
            }
            other => other,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.io).poll_shutdown(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(self.close()),
            other => other,
        }
    }
}

/// One row of a wire log, as a reader takes it.
#[derive(Debug, PartialEq)]
pub enum Row {
    /// The first row.
    Start(Value),
    /// Bytes of one direction, and whether a newline followed them.
    Bytes {
        incoming: bool,
        n: u64,
        bytes: Vec<u8>,
        newline: bool,
    },
}

/// Read one row of a wire log.
pub fn row(line: &str) -> Result<Row, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
    let incoming = match value["record"].as_str() {
        Some("start") => {
            return match value["format"].as_u64() {
                Some(format) if format == u64::from(FORMAT) => Ok(Row::Start(value)),
                other => Err(format!(
                    "wire log format {other:?} is not format {FORMAT}, which this build reads"
                )),
            };
        }
        Some("in") => true,
        Some("out") => false,
        other => return Err(format!("unknown record {other:?}")),
    };
    let bytes = match (value["text"].as_str(), value["hex"].as_str()) {
        (Some(text), None) => text.as_bytes().to_vec(),
        (None, Some(hex)) => unhex(hex).ok_or("hex is not pairs of hex digits")?,
        _ => return Err("a row holds text or hex".into()),
    };
    Ok(Row::Bytes {
        incoming,
        n: value["n"].as_u64().ok_or("a row has a number n")?,
        bytes,
        newline: value["nl"] != false,
    })
}

fn unhex(hex: &str) -> Option<Vec<u8>> {
    if !hex.is_ascii() || !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect()
}

/// The exact bytes of one direction of `rows`.
pub fn bytes<'a>(rows: impl IntoIterator<Item = &'a Row>, incoming: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        if let Row::Bytes {
            incoming: direction,
            bytes,
            newline,
            ..
        } = row
            && *direction == incoming
        {
            out.extend_from_slice(bytes);
            if *newline {
                out.push(b'\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fallible = Result<(), Box<dyn std::error::Error>>;

    fn rows(path: &Path) -> Result<Vec<Row>, Box<dyn std::error::Error>> {
        Ok(std::fs::read_to_string(path)?
            .lines()
            .map(row)
            .collect::<Result<_, _>>()?)
    }

    /// Whatever passes, in whatever fragments, comes back byte for byte:
    /// JSON and what is not, CRLF, bytes that are not UTF-8, an empty line
    /// and what a closed pipe held.
    #[test]
    fn both_directions_come_back_byte_for_byte() -> Fallible {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("wire.jsonl");
        let started = Started {
            mode: Some("allow-all".into()),
            folders: vec!["/work/run".into()],
            ..Started::default()
        };
        let log = WireLog::create(&path, &started)?;
        let incoming: &[u8] =
            b"{ \"id\":1, \"method\":\"tools/call\" }\r\n\n{not json\n\xff\xfe\"\x00\ntail without newline";
        let outgoing = "{\"id\":1,\"result\":{\"text\":\"é ⟨\\n⟩\"}}\n".as_bytes();
        {
            let mut read = log.stream((), Direction::In);
            let mut write = log.stream((), Direction::Out);
            for fragment in incoming.chunks(5) {
                read.observe(fragment)?;
            }
            for fragment in outgoing.chunks(3) {
                write.observe(fragment)?;
            }
        }
        let rows = rows(&path)?;
        let Some(Row::Start(start)) = rows.first() else {
            return Err("the first row is the start".into());
        };
        assert_eq!(start["mode"], "allow-all");
        assert_eq!(start["folders"][0], "/work/run");
        assert_eq!(start["server"], super::super::NAME);
        assert_eq!(bytes(&rows, true), incoming);
        assert_eq!(bytes(&rows, false), outgoing);
        // Rows are numbered in the order the server saw them.
        let numbers: Vec<u64> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Bytes { n, .. } => Some(*n),
                Row::Start(_) => None,
            })
            .collect();
        assert_eq!(numbers, (1..=numbers.len() as u64).collect::<Vec<_>>());
        Ok(())
    }

    /// A line longer than a row holds is recorded in pieces, in bounded
    /// space, and still comes back whole.
    #[test]
    fn an_overlong_line_is_recorded_in_pieces() -> Fallible {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("wire.jsonl");
        let log = WireLog::create(&path, &Started::default())?;
        let mut long = vec![b'x'; MAX_PIECE * 2 + 7];
        long.push(b'\n');
        long.extend_from_slice(b"{}\n");
        {
            let mut read = log.stream((), Direction::In);
            for fragment in long.chunks(64 * 1024) {
                read.observe(fragment)?;
                assert!(read.line.len() <= MAX_PIECE);
            }
        }
        let rows = rows(&path)?;
        assert_eq!(rows.len(), 5);
        assert_eq!(bytes(&rows, true), long);
        Ok(())
    }

    #[test]
    fn an_existing_log_is_never_replaced() -> Fallible {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("wire.jsonl");
        std::fs::write(&path, "kept")?;
        let refused = WireLog::create(&path, &Started::default());
        assert!(refused.is_err_and(|e| e.kind() == io::ErrorKind::AlreadyExists));
        assert_eq!(std::fs::read_to_string(&path)?, "kept");
        Ok(())
    }

    #[test]
    fn a_log_of_another_format_is_refused() {
        assert!(row(r#"{"record":"start","format":2}"#).is_err());
        assert!(row(r#"{"record":"call"}"#).is_err());
        assert!(row(r#"{"record":"in","n":1,"hex":"0g"}"#).is_err());
    }
}

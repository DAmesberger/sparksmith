//! `mcp-replay`: a recorded MCP run (the wire log of `mcp --wire-log`, or
//! a client's transcript) as the JSON-lines requests that rebuild its
//! project through `stdio --operator`
//! (`sparksmith_automation::mcp::replay`). It reads the record and writes
//! one new file; what it kept and dropped goes to stdout as JSON.
use super::{Result, print};
use sparksmith_automation::mcp::replay::{self, Options};
use sparksmith_core::product::SHORT_NAME as CLI;
use std::io::{BufReader, BufWriter, Write};

pub const USAGE: &str =
    "mcp-replay RECORD.jsonl NEW_REPLAY.jsonl [--transcript] [--root DIR] [--session N]
                                   the stdio requests that rebuild a recorded MCP run's
                                   project: accepted edits, actions and the reads later
                                   calls build on, with paths relative to the run's
                                   folder (--root; else the one the wire log names).
                                   RECORD is a wire log (mcp --wire-log), or with
                                   --transcript a client's stream-json transcript.
                                   --session N picks a project when the run changed
                                   several. Anything it cannot map is an error naming
                                   the call";

pub fn run(args: &[String]) -> Result<()> {
    let [record, output, rest @ ..] = args else {
        return Err(format!("usage: {CLI} {USAGE}").into());
    };
    let mut options = Options::default();
    let mut transcript = false;
    let mut flags = rest.iter();
    while let Some(flag) = flags.next() {
        match (flag.as_str(), flags.clone().next()) {
            ("--transcript", _) => transcript = true,
            ("--root", Some(root)) => {
                options.root = Some(root.clone());
                flags.next();
            }
            ("--session", Some(n)) => {
                options.session =
                    Some(n.parse().map_err(|_| {
                        format!("--session takes a project's number from 1, not {n}")
                    })?);
                flags.next();
            }
            (other, _) => {
                return Err(format!("unexpected argument {other}; usage: {CLI} {USAGE}").into());
            }
        }
    }
    let mut input =
        BufReader::new(std::fs::File::open(record).map_err(|error| format!("{record}: {error}"))?);
    let converted = if transcript {
        let calls = replay::read_transcript(&mut input)?;
        replay::convert(None, &calls, &options)
    } else {
        let (started, calls) = replay::read_wire(&mut input)?;
        replay::convert(Some(&started), &calls, &options)
    }
    .map_err(|why| format!("{record}: {why}"))?;
    // Nothing is written unless the whole record converts, and never over
    // an existing file.
    let file = std::fs::File::create_new(output).map_err(|error| format!("{output}: {error}"))?;
    let mut out = BufWriter::new(file);
    for line in &converted.lines {
        serde_json::to_writer(&mut out, line)?;
        writeln!(out)?;
    }
    out.flush()?;
    print(&serde_json::to_value(&converted.summary)?)
}

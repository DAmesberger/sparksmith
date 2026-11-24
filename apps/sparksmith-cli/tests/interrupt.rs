//! Ctrl-C during `design-part`: the first ends the run like the app's Stop
//! and saves what it made with its recording; a second quits at once.
//! The model is a local server that answers the package step, then streams
//! without end (or says nothing at all).
#![cfg(unix)]
use serde_json::{Value, json};
use sparksmith_assist::fixtures;
use sparksmith_assist::model::record::Replay;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

const PATIENCE: Duration = Duration::from_secs(60);

/// Read one POST, up to the end of its body.
fn read_request(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut length = 0;
    let mut line = String::new();
    while reader.read_line(&mut line)? > 2 {
        if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = n.trim().parse().unwrap_or(0);
        }
        line.clear();
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)
}

fn chunk(stream: &mut TcpStream, value: &Value) -> std::io::Result<()> {
    write!(stream, "data: {value}\n\n")?;
    stream.flush()
}

/// Answers the package step, then streams text for ever (`talk`) or stays
/// silent; tells each request it has begun.
fn serve(listener: TcpListener, begun: mpsc::Sender<u32>, talk: bool) {
    std::thread::spawn(move || {
        for (n, stream) in (1u32..).zip(listener.incoming()) {
            let Ok(mut stream) = stream else { return };
            let begun = begun.clone();
            std::thread::spawn(move || -> std::io::Result<()> {
                read_request(&mut stream)?;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n"
                )?;
                if n == 1 {
                    let (step, result) = &fixtures::exr22_answers()[0];
                    let arguments = json!({"name": "assist.step_result",
                        "params": {"step": step, "result": result}});
                    chunk(
                        &mut stream,
                        &json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "a1",
                            "type": "function", "function": {"name": "query",
                            "arguments": arguments.to_string()}}]}}]}),
                    )?;
                    chunk(
                        &mut stream,
                        &json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
                    )?;
                    return write!(stream, "data: [DONE]\n\n");
                }
                if talk {
                    chunk(
                        &mut stream,
                        &json!({"choices": [{"delta": {"content": "I am"}}]}),
                    )?;
                }
                let _ = begun.send(n);
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    if talk {
                        chunk(
                            &mut stream,
                            &json!({"choices": [{"delta": {"content": " thinking"}}]}),
                        )?;
                    }
                }
            });
        }
    });
}

fn design_part(dir: &Path, base: &str) -> Fallible<Child> {
    let sheet = dir.join("exr22.pdf");
    fixtures::exr22_datasheet(&sheet)?;
    let config = dir.join("config");
    std::fs::create_dir_all(&config)?;
    Ok(Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", &config)
        .env("SPARKSMITH_CACHE_DIR", dir.join("cache"))
        .args(["design-part", "--provider", "openai", "--base-url", base])
        .args(["--model", "scripted", "--context-tokens", "131072"])
        .args(["--name", "EXR-22", "--datasheet"])
        .arg(&sheet)
        .arg("--out")
        .arg(dir.join("exr22.sparksmith"))
        .arg("--record")
        .arg(dir.join("run.jsonl"))
        .stderr(Stdio::piped())
        .spawn()?)
}

fn interrupt(child: &Child) -> Fallible {
    let sent = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()?;
    sent.success().then_some(()).ok_or_else(|| "kill".into())
}

/// The child's end; killed when it outlives the test's patience.
fn ended(child: &mut Child) -> Fallible<ExitStatus> {
    let started = Instant::now();
    while started.elapsed() < PATIENCE {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill()?;
    Err("the run did not end".into())
}

/// The child's standard error, line by line.
fn lines(child: &mut Child) -> Fallible<mpsc::Receiver<String>> {
    let (tx, rx) = mpsc::channel();
    let stderr = child.stderr.take().ok_or("stderr")?;
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    Ok(rx)
}

fn started(talk: bool, dir: &Path) -> Fallible<(Child, mpsc::Receiver<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let base = format!("http://{}/v1", listener.local_addr()?);
    let (tx, begun) = mpsc::channel();
    serve(listener, tx, talk);
    let mut child = design_part(dir, &base)?;
    let said = lines(&mut child)?;
    if begun.recv_timeout(PATIENCE).is_err() {
        child.kill()?;
        child.wait()?;
        let said: Vec<String> = said.try_iter().collect();
        return Err(format!("the second request never came: {said:#?}").into());
    }
    Ok((child, said))
}

#[test]
fn ctrl_c_stops_the_run_and_saves_what_it_made() -> Fallible {
    let dir = tempfile::tempdir()?;
    let (mut child, said) = started(true, dir.path())?;
    interrupt(&child)?;
    let status = ended(&mut child)?;
    let said = said.iter().collect::<Vec<_>>().join("\n");
    assert!(!status.success(), "{said}");
    assert!(said.contains("stopping: the run ends"), "{said}");
    assert!(said.contains("drafts saved to"), "{said}");
    assert!(said.contains("recording: "), "{said}");
    assert!(dir.path().join("exr22.sparksmith").exists());
    let review: Value = serde_json::from_str(&std::fs::read_to_string(
        dir.path().join("exr22.review.json"),
    )?)?;
    assert_eq!(review["run"]["stop"], "cancelled", "{review:#}");
    // The drafts of the answered package step, and the answer itself.
    let drafts = review["run"]["drafts"].as_array().map_or(0, Vec::len);
    assert!(drafts >= 4, "{review:#}");
    assert_eq!(
        review["run"]["unfinished"]["answers"]["plan"]["generator"],
        "module"
    );
    // The recording holds both replies, the second as it was stopped.
    let recorded = std::fs::read_to_string(dir.path().join("run.jsonl"))?;
    assert_eq!(recorded.lines().count(), 3, "{recorded}");
    assert!(recorded.contains("I am"), "{recorded}");
    Replay::from_file(&dir.path().join("run.jsonl"))?;
    Ok(())
}

#[test]
fn a_second_ctrl_c_quits_at_once() -> Fallible {
    let dir = tempfile::tempdir()?;
    let (mut child, said) = started(false, dir.path())?;
    interrupt(&child)?;
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if said.recv_timeout(left)?.contains("stopping: the run ends") {
            break;
        }
    }
    interrupt(&child)?;
    assert_eq!(ended(&mut child)?.code(), Some(130));
    assert!(!dir.path().join("exr22.sparksmith").exists());
    Ok(())
}

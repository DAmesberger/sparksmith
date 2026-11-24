//! A fresh rmcp client drives a separate headless Sparksmith process. The
//! server supplies both its instructions and the six tools at startup.
use crate::{
    Result, Task,
    run::Arm,
    trace::{Measured, Observer, Row, Trace, Transport},
};
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient};
use serde_json::{Value, json};
use sparksmith_assist::{
    chat::{self, ChatConfig, ChatLibrary, End, Tools},
    model::{Part, ToolSpec},
};
use sparksmith_automation::mcp::{self, trace::Call};
use std::{
    cell::RefCell,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::Path,
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tokio::{
    process::{Child, Command},
    runtime::Runtime,
    time::timeout,
};

#[derive(Clone)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("sparksmith-agent-bench", env!("CARGO_PKG_VERSION")),
        )
    }
}
struct Connection {
    runtime: Runtime,
    service: RunningService<RoleClient, Client>,
    child: Child,
    deadline: Instant,
    wire: BufReader<File>,
}
impl Connection {
    fn open(
        binary: &Path,
        dir: &Path,
        project: &Path,
        task: &Task,
        detail: &str,
        duration: Duration,
    ) -> Result<(Self, Vec<ToolSpec>, String)> {
        let default_library = sparksmith_io::shipped::ShippedLibrary::locate()?;
        let library = default_library.root().canonicalize()?;
        let board = project.parent().ok_or("missing project folder")?;
        let tap = dir.join("wire.jsonl");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let deadline = Instant::now()
            .checked_add(duration)
            .ok_or("bench deadline is out of range")?;
        let (child, service, tools, method) = runtime.block_on(async {
            // All user configuration and project paths belong to this run.
            // In particular the server receives no model credentials.
            let mut child = Command::new(binary.canonicalize()?)
                .env_clear()
                .current_dir(board)
                .env("HOME", dir)
                .env("SPARKSMITH_CONFIG_DIR", dir.join("config"))
                .env("SPARKSMITH_CACHE_DIR", dir.join("cache"))
                .env("SPARKSMITH_RECOVERY_DIR", dir.join("recovery"))
                .env(sparksmith_io::shipped::ENV, library)
                .arg("mcp")
                .arg("--permission-mode")
                .arg(task.spec.mode.name())
                .arg("--allow-folder")
                .arg(board)
                .arg("--bench-trace")
                .arg(&tap)
                .arg("--bench-detail")
                .arg(detail)
                .arg(project)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .spawn()?;
            let input = child.stdin.take().ok_or("MCP server lacks stdin")?;
            let output = child.stdout.take().ok_or("MCP server lacks stdout")?;
            let service = timeout(
                duration,
                Client.serve_with_lifecycle(
                    tokio::io::join(output, input),
                    ClientLifecycleMode::Discover {
                        preferred_versions: mcp::VERSIONS.to_vec(),
                    },
                ),
            )
            .await??;
            let method = service
                .peer_info()
                .and_then(|s| s.instructions.clone())
                .ok_or("MCP server omitted its instructions")?;
            let listing = timeout(
                deadline.saturating_duration_since(Instant::now()),
                service.list_tools(None),
            )
            .await??;
            let tools = listing
                .tools
                .into_iter()
                .map(|t| ToolSpec {
                    name: t.name.into_owned(),
                    description: t.description.unwrap_or_default().into_owned(),
                    input_schema: Value::Object((*t.input_schema).clone()),
                })
                .collect();
            Result::Ok((child, service, tools, method))
        })?;
        Ok((
            Self {
                runtime,
                service,
                child,
                deadline,
                wire: BufReader::new(File::open(tap)?),
            },
            tools,
            method,
        ))
    }
    fn call(&mut self, name: &str, input: &Value) -> Result<(Value, Call)> {
        let args = input.as_object().ok_or("MCP arguments must be an object")?;
        let result = self.runtime.block_on(async {
            timeout(
                self.deadline.saturating_duration_since(Instant::now()),
                self.service.call_tool(
                    CallToolRequestParams::new(name.to_owned()).with_arguments(args.clone()),
                ),
            )
            .await
        })??;
        let text = &result
            .content
            .first()
            .and_then(|p| p.as_text())
            .ok_or("missing tool result text")?
            .text;
        let value: Value = serde_json::from_str(text)?;
        let reply = if result.is_error == Some(true) {
            json!({"ok":false,"error":value})
        } else {
            json!({"ok":true,"result":value})
        };
        // The peer can receive the pipe write just before its server-side
        // tap has appended the same call. Wait only within this run's time.
        let mut line = String::new();
        loop {
            self.wire.read_line(&mut line)?;
            if line.ends_with('\n') {
                break;
            }
            if Instant::now() >= self.deadline {
                return Err("MCP trace row did not arrive before the run deadline".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let Row::Call(call) = serde_json::from_str(&line)? else {
            return Err("expected MCP tap call".into());
        };
        if call.verb != name || call.result_sha256 != crate::trace::digest(text.as_bytes()) {
            return Err("the MCP trace row does not match the received tool result".into());
        }
        Ok((reply, call))
    }
    fn close(mut self) -> Result<()> {
        self.runtime.block_on(async {
            timeout(
                self.deadline.saturating_duration_since(Instant::now()),
                self.service.cancel(),
            )
            .await??;
            let status = timeout(
                self.deadline.saturating_duration_since(Instant::now()),
                self.child.wait(),
            )
            .await??;
            if !status.success() {
                return Err(format!("MCP server exited {status}").into());
            }
            Ok(())
        })
    }
}

struct Session<'a> {
    server: &'a mut Connection,
    pending: &'a RefCell<Option<(String, Call)>>,
    fatal: Option<String>,
    revision: Option<u64>,
    cancel: &'a AtomicBool,
}
impl Tools for Session<'_> {
    fn call(&mut self, verb: &str, input: &Value) -> Value {
        match self.server.call(verb, input) {
            Ok((reply, call)) => {
                if let Some(rev) = reply["result"]["revision"].as_u64() {
                    self.revision = Some(rev);
                }
                *self.pending.borrow_mut() = Some((json!([verb, input]).to_string(), call));
                reply
            }
            Err(err) => {
                self.cancel.store(true, Ordering::Relaxed);
                let reason = err.to_string();
                self.fatal = Some(reason.clone());
                json!({"ok":false,"error":{"code":"bench.external","message":reason}})
            }
        }
    }
}

pub fn run(task: &Task, dir: &Path, binary: &Path, arm: Arm, instruction: &str) -> Result<Row> {
    let start = task
        .start
        .as_ref()
        .ok_or("this external arm needs a saved starting project")?;
    fs::create_dir(dir)?;
    let dir = dir.canonicalize()?;
    let board = dir.join("board");
    fs::create_dir(&board)?;
    let path = board.join("revised.sparksmith");
    fs::copy(start, &path)?;
    let detail = serde_json::to_value(arm.detail)?
        .as_str()
        .ok_or("unsupported surface detail")?
        .to_string();
    let started = Instant::now();
    let trace = Trace::create(
        &dir.join("trace.jsonl"),
        &Row::Start {
            workflow: task.mark.clone(),
            runner: "external-mcp".into(),
            detail: detail.clone(),
        },
    )?;
    let duration = arm
        .budget
        .max_duration
        .ok_or("this external arm needs a run time")?;
    let (mut connection, tools, method) =
        Connection::open(binary, &dir, &path, task, &detail, duration)?;
    let mut cfg = ChatConfig::new(&ChatLibrary::locate()?, arm.profile, arm.budget)?;
    cfg.system = method;
    cfg.tools = tools;
    // Both clients send omitted detail through unchanged. The server selects
    // the treatment, preserving the caller's right to explicitly request full.
    cfg.surface_view.strategy = sparksmith_automation::session::SchemaDetail::Input;
    let pending = RefCell::new(None);
    let cancel = AtomicBool::new(false);
    let mut calls = Session {
        server: &mut connection,
        pending: &pending,
        fatal: None,
        revision: None,
        cancel: &cancel,
    };
    let mut model = Measured {
        backend: arm.backend,
        trace: &trace,
    };
    let mut observation = Observer::new(&trace, &pending, Transport::Mcp);
    let mut io_result = Ok(());
    let mut event = |event| {
        if io_result.is_ok() {
            io_result = observation.event(event);
            if io_result.is_err() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    };
    let goal = format!("{}\n\n{}", task.goal.trim_end(), instruction);
    let outcome = chat::turn(
        &cfg,
        &mut model,
        &mut Vec::new(),
        vec![Part::text(goal)],
        &mut calls,
        &mut event,
        &cancel,
    );
    io_result?;
    let revision = calls.revision;
    let fatal = calls.fatal.take();
    let closing = connection.close();
    let verified = task.expected.verify(task.start.as_deref(), &path);
    let artifact_verified = verified.is_ok();
    let verification_reason = verified.as_ref().err().map(ToString::to_string);
    let status = (|| -> Result<()> {
        if let Some(reason) = fatal {
            return Err(reason.into());
        }
        closing?;
        if outcome.end != End::Answered {
            return Err(format!("turn ended: {}", outcome.end).into());
        }
        verified?;
        Ok(())
    })();
    let row = Row::Finish {
        artifact_verified: Some(artifact_verified),
        verification_reason,
        turn_end: Some(outcome.end.to_string()),
        metrics: trace.metrics.take(),
        usage: arm.reports_usage.then_some(outcome.usage.reported),
        success: status.is_ok(),
        reason: status.as_ref().err().map(ToString::to_string),
        wall_ms: started.elapsed().as_secs_f64() * 1000.0,
        revision,
        saved_sha256: fs::read(path).ok().map(|b| crate::trace::digest(&b)),
    };
    trace.write(&row)?;
    status?;
    Ok(row)
}

/// Exercise the exact stdio and trace path without a model or API key.
pub fn probe(binary: &Path, dir: &Path) -> Result<()> {
    let task = Task::load("two-header")?;
    let start = task.start.as_ref().ok_or("missing ECO fixture")?;
    fs::create_dir(dir)?;
    let mut sizes = Vec::new();
    for detail in ["input", "full"] {
        let run = dir.canonicalize()?.join(detail);
        let board = run.join("board");
        fs::create_dir_all(&board)?;
        let project = board.join("revised.sparksmith");
        fs::copy(start, &project)?;
        let (mut server, tools, _) = Connection::open(
            binary,
            &run,
            &project,
            &task,
            detail,
            Duration::from_secs(30),
        )?;
        if tools.len() != mcp::tools::NAMES.len() {
            return Err("discovery returned a different tool set".into());
        }
        let (reply, call) = server.call("describe", &json!({"name":"fabrication.preflight"}))?;
        if !call.ok || reply["ok"] != true {
            return Err("the external describe probe failed".into());
        }
        sizes.push(call.reply_bytes);
        let (_, override_call) = server.call(
            "describe",
            &json!({"name":"fabrication.preflight","detail":"input"}),
        )?;
        if detail == "full" && override_call.result_sha256 == call.result_sha256 {
            return Err("the explicit input view did not override the external arm".into());
        }
        server.close()?;
    }
    if sizes[0] >= sizes[1] {
        return Err("the input describe probe did not shrink the full wire reply".into());
    }
    println!(
        "{}",
        json!({"input_reply_bytes":sizes[0],"full_reply_bytes":sizes[1]})
    );
    Ok(())
}

/// Import the trace produced by an independently chosen stdio client.
/// Its provider counters are available only if supplied by that client.
pub fn external_report(
    task: &Task,
    saved: &Path,
    wire: &Path,
    usage_path: Option<&Path>,
) -> Result<()> {
    let verified = task.expected.verify(task.start.as_deref(), saved);
    let usage = usage_path
        .map(|path| -> Result<sparksmith_assist::usage::Usage> {
            let value: Value = serde_json::from_slice(&fs::read(path)?)?;
            if [
                "input_tokens",
                "cache_write_tokens",
                "cache_read_tokens",
                "output_tokens",
            ]
            .iter()
            .any(|k| value[*k].as_u64().is_none())
            {
                return Err("the client's usage must contain all four counters".into());
            }
            Ok(serde_json::from_value(value)?)
        })
        .transpose()?;
    let mut requests = 0;
    let mut replies = 0;
    let mut visible = 0;
    let mut refusals = 0;
    let mut server_ms = 0.0;
    let mut calls = std::collections::BTreeMap::<String, u64>::new();
    for line in BufReader::new(File::open(wire)?).lines() {
        let Row::Call(call) = serde_json::from_str(&line?)? else {
            return Err("the server tap must contain only MCP call rows".into());
        };
        if !matches!(call.transport, mcp::trace::Transport::Mcp) {
            return Err("the tap contains an in-process call".into());
        }
        *calls.entry(call.verb).or_default() += 1;
        requests += call.request_bytes;
        replies += call.reply_bytes;
        visible += call.visible_bytes;
        refusals += usize::from(!call.ok);
        server_ms += call.server_ms;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "task":task.mark,"success":verified.is_ok(),"reason":verified.as_ref().err().map(ToString::to_string),
            "saved_sha256":fs::read(saved).ok().map(|b|crate::trace::digest(&b)),
            "provider_usage":usage,"model_ms":Value::Null,"wall_ms":Value::Null,"calls":calls,"refusals":refusals,
            "request_bytes":requests,"reply_bytes":replies,"visible_bytes":visible,"server_ms":server_ms
        }))?
    );
    verified
}

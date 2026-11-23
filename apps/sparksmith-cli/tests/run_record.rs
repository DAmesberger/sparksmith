//! The record of an MCP run, proven end to end with no model and no
//! network. A scripted client drives the real `sparksmith mcp` process over
//! its stdio transport with `--wire-log` on and builds a small board: three
//! generated parts, a schematic, a routed layout with a fill, a component
//! imported from a working pinout document, a save and a fabrication
//! export; with a refused edit, a refused path, a dry run and reads on the
//! way. Then:
//!
//! 1. the wire log holds exactly the bytes the client wrote and read;
//! 2. `mcp-replay` turns it into a replay with paths relative to the run's
//!    folder;
//! 3. `stdio --operator` replays it in a fresh folder with no refusal;
//! 4. the saved project is byte-identical, the fabrication files the run
//!    exported are byte-identical, and `fab --date none` of both projects
//!    gives the same files and the same release revision.
//!
//! A second run, under `auto-allow-edits`, shows the gate requests a
//! headless server refuses in the wire log and in the converter's summary.
use serde_json::{Value, json};
use sparksmith_automation::mcp::wirelog;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The checkout's shipped library, which every process of a test reads.
fn library() -> Fallible<PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../library")
        .canonicalize()?)
}

/// The CLI with its own settings under `home`, run in `cwd`.
fn cli(home: &Path, cwd: &Path) -> Fallible<Command> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sparksmith"));
    command
        .current_dir(cwd)
        .env(sparksmith_io::shipped::ENV, library()?)
        .env("SPARKSMITH_CONFIG_DIR", home.join("config"))
        .env("SPARKSMITH_CACHE_DIR", home.join("cache"))
        .env("SPARKSMITH_RECOVERY_DIR", home.join("recovery"));
    Ok(command)
}

/// Run `args` to its end: its standard output, or its error.
fn run(home: &Path, cwd: &Path, args: &[&str]) -> Fallible<String> {
    let out = cli(home, cwd)?.args(args).output()?;
    if !out.status.success() {
        return Err(format!("{args:?}: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// A scripted MCP client on a `sparksmith mcp` process's pipes. It keeps
/// every byte it writes and reads.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
    sent: Vec<u8>,
    received: Vec<u8>,
    /// The project the calls name, once one is open.
    project: Option<String>,
    revision: u64,
}

impl Client {
    fn start(home: &Path, folder: &Path, mode: &str, wire: &Path) -> Fallible<Self> {
        let mut child = cli(home, folder)?
            .args(["mcp", "--permission-mode", mode, "--allow-folder"])
            .arg(folder)
            .arg("--wire-log")
            .arg(wire)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().ok_or("the server's stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
            next: 0,
            sent: Vec::new(),
            received: Vec::new(),
            project: None,
            revision: 0,
        })
    }

    /// One request and its response, as `2026-07-28` frames them.
    fn request(&mut self, method: &str, mut params: Value) -> Fallible<Value> {
        self.next += 1;
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "scripted", "version": "0"},
        });
        let line = format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params})
        );
        let stdin = self.stdin.as_mut().ok_or("the server's stdin")?;
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        self.sent.extend_from_slice(line.as_bytes());
        loop {
            let mut reply = Vec::new();
            if self.stdout.read_until(b'\n', &mut reply)? == 0 {
                return Err(format!("the server closed before answering {method}").into());
            }
            self.received.extend_from_slice(&reply);
            let message: Value = serde_json::from_slice(&reply)?;
            if message["id"] == self.next {
                return Ok(message);
            }
        }
    }

    /// A tool call: whether it failed, and its JSON.
    fn call(&mut self, verb: &str, arguments: Value) -> Fallible<(bool, Value)> {
        let message = self.request("tools/call", json!({"name": verb, "arguments": arguments}))?;
        let result = &message["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .ok_or_else(|| format!("{verb}: no tool result in {message}"))?;
        Ok((result["isError"] == true, serde_json::from_str(text)?))
    }

    fn ok(&mut self, verb: &str, arguments: Value) -> Fallible<Value> {
        match self.call(verb, arguments.clone())? {
            (false, payload) => Ok(payload),
            (true, payload) => Err(format!("{verb} {arguments}: {payload}").into()),
        }
    }

    /// A call that must be refused: its code.
    fn refused(&mut self, verb: &str, arguments: Value) -> Fallible<String> {
        match self.call(verb, arguments.clone())? {
            (true, payload) => Ok(payload["code"].as_str().unwrap_or_default().to_owned()),
            (false, payload) => Err(format!("{verb} {arguments} ran: {payload}").into()),
        }
    }

    fn query(&mut self, name: &str, params: Value) -> Fallible<Value> {
        let project = self.project.clone();
        self.ok(
            "query",
            json!({"project": project, "name": name, "params": params}),
        )
    }

    /// `apply` at the revision of the last edit, which it moves on.
    fn apply(&mut self, operations: Value) -> Fallible<Value> {
        let project = self.project.clone();
        let reply = self.ok(
            "apply",
            json!({"project": project, "expected_revision": self.revision,
                "operations": operations}),
        )?;
        self.revision = reply["revision"].as_u64().ok_or("a revision")?;
        Ok(reply)
    }

    fn act(&mut self, name: &str, params: Value) -> Fallible<(bool, Value)> {
        let project = self.project.clone();
        self.call(
            "act",
            json!({"project": project, "name": name, "params": params}),
        )
    }

    /// Close the client's end and wait for the server: the bytes sent and
    /// received.
    fn finish(mut self) -> Fallible<(Vec<u8>, Vec<u8>)> {
        drop(self.stdin.take());
        self.stdout.read_to_end(&mut self.received)?;
        let status = self.child.wait()?;
        if !status.success() {
            return Err(format!("the server ended with {status}").into());
        }
        Ok((self.sent, self.received))
    }
}

/// The wire log at `path` holds exactly `sent` and `received`.
fn assert_wire(path: &Path, sent: &[u8], received: &[u8]) -> Fallible<Value> {
    let rows = std::fs::read_to_string(path)?
        .lines()
        .map(wirelog::row)
        .collect::<Result<Vec<_>, _>>()?;
    let Some(wirelog::Row::Start(start)) = rows.first() else {
        return Err("the wire log starts with its start row".into());
    };
    assert!(
        wirelog::bytes(&rows, true) == sent,
        "the bytes the client wrote"
    );
    assert!(
        wirelog::bytes(&rows, false) == received,
        "the bytes the client read"
    );
    Ok(start.clone())
}

/// Every file under `dir`, by name.
fn files(dir: &Path) -> Fallible<BTreeMap<String, Vec<u8>>> {
    let mut found = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        found.insert(
            entry.file_name().to_string_lossy().into_owned(),
            std::fs::read(entry.path())?,
        );
    }
    Ok(found)
}

/// The Gerber `.ProjectId` attribute of the first copper film in `fab`:
/// name, project id and release revision.
fn project_id(fab: &BTreeMap<String, Vec<u8>>) -> Fallible<String> {
    let (_, film) = fab
        .iter()
        .find(|(name, _)| name.ends_with("-copper-l1-top.gbr"))
        .ok_or("a top copper film")?;
    String::from_utf8_lossy(film)
        .lines()
        .find(|line| line.starts_with("%TF.ProjectId,"))
        .map(str::to_owned)
        .ok_or_else(|| "a .ProjectId attribute".into())
}

/// The terminal of the pin named `name` in a `schematic.pins` part.
fn pin(part: &Value, name: &str) -> Value {
    let terminal = part["pins"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row[2] == name)
        .map_or(Value::Null, |row| row[0].clone());
    json!({"instance": part["instance"], "terminal": terminal})
}

/// The net and position of `designator`'s pad `pad` in a `layout.pads` reply.
fn pad(pads: &Value, designator: &str, pad: &str) -> Fallible<(Value, [f64; 2])> {
    let row = pads
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["designator"] == designator && row["pad"] == pad)
        .ok_or_else(|| format!("pad {designator}.{pad}"))?;
    let at = |k: usize| row["position"][k].as_f64().ok_or("a pad position");
    Ok((row["net"].clone(), [at(0)?, at(1)?]))
}

/// The parts of the board: what to search for, and the designator.
const PARTS: [(&str, &str); 3] = [
    ("header 1x2", "J1"),
    ("resistor 0603 4.7k", "R1"),
    ("led 0603 red", "D1"),
];

/// Build the board through the verbs, as a model would: ids come from the
/// replies. Returns nothing; the run is in the client's bytes and the files.
fn build(client: &mut Client, folder: &Path) -> Fallible {
    client.request("server/discover", json!({}))?;
    client.request("tools/list", json!({}))?;
    // The project the server started with stays empty; the board is a new
    // one, behind its own handle.
    client.ok("query", json!({"name": "project.inspect"}))?;
    client.ok(
        "find",
        json!({"text": "place a component on the schematic"}),
    )?;
    client.ok("describe", json!({"name": "component.place"}))?;
    let (failed, new) = client.call(
        "act",
        json!({"name": "project.new", "params": {"profile": "jlcpcb-2-layer"}}),
    )?;
    assert!(!failed, "{new}");
    client.project = new["project"].as_str().map(str::to_owned);
    client.revision = new["revision"].as_u64().ok_or("a revision")?;

    // Refused: a stale revision, and a path outside the run's folder.
    let project = client.project.clone();
    let rename = |name: &str| json!([{"op": "project.rename", "params": {"name": name}}]);
    let stale = json!({"project": project, "expected_revision": client.revision + 9,
        "operations": rename("Stale")});
    assert_eq!(client.refused("apply", stale)?, "stale_revision");
    let outside = folder.join("../outside.sparksmith").display().to_string();
    let (failed, refusal) = client.act("project.save", json!({"path": outside}))?;
    assert!(
        failed && refusal["code"] == "gate.path_outside",
        "{refusal}"
    );
    // A dry run keeps nothing.
    let tried = json!({"project": project, "expected_revision": client.revision,
        "dry_run": true, "operations": rename("Tried")});
    assert_eq!(client.ok("apply", tried)?["dry_run"], true);

    client.apply(rename("Recorded board"))?;
    // The parts the search offers to make, each make alone.
    for (words, _) in PARTS {
        let found = client.query("library.search", json!({"query": words}))?;
        client.apply(json!([found["generic"][0]["make"]]))?;
    }
    let mut places = Vec::new();
    for (words, designator) in PARTS {
        let found = client.query("library.search", json!({"query": words}))?;
        let hint = &found["generic"][0];
        let mut params = json!({"component": hint["component"], "physical": hint["physical"],
            "designator": designator});
        if let Some(properties) = hint.get("properties") {
            params["properties"] = properties.clone();
        }
        places.push(json!({"op": "component.place", "params": params}));
    }
    let placed = client.apply(json!(places))?;
    let instances: Vec<Value> = placed["outcomes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|outcome| outcome["instance"].clone())
        .collect();
    let designators: Vec<&str> = PARTS.iter().map(|(_, d)| *d).collect();
    let parts =
        client.query("schematic.pins", json!({"designators": designators}))?["result"].clone();
    // J1.1 to R1.1, R1.2 to the LED's anode, its cathode to J1.2.
    let wires = [
        (pin(&parts[0], "1"), pin(&parts[1], "1")),
        (pin(&parts[1], "2"), pin(&parts[2], "A")),
        (pin(&parts[2], "K"), pin(&parts[0], "2")),
    ]
    .map(|(from, to)| json!({"op": "schematic.connect", "params": {"from": from, "to": to}}));
    client.apply(json!(wires))?;
    client.apply(json!([
        {"op": "layout.set_board_size", "params": {"size": [30.0, 20.0]}},
        {"op": "layout.sync", "params": {}},
    ]))?;
    let moves: Vec<Value> = [
        ([5.0, 10.0], 90.0),
        ([15.0, 6.0], 0.0),
        ([24.0, 10.0], 270.0),
    ]
    .iter()
    .zip(&instances)
    .map(|((at, rotation), instance)| {
        json!({"op": "layout.move", "params": {"instance": instance, "position": at,
                "rotation_deg": rotation, "layer": "Top"}})
    })
    .collect();
    client.apply(json!(moves))?;
    let pads = client.query("layout.pads", json!({}))?["result"].clone();
    let (vin, j1) = pad(&pads, "J1", "1")?;
    let (_, r1a) = pad(&pads, "R1", "1")?;
    let (led, r1b) = pad(&pads, "R1", "2")?;
    let (_, anode) = pad(&pads, "D1", "2")?;
    let (gnd, j2) = pad(&pads, "J1", "2")?;
    let (_, cathode) = pad(&pads, "D1", "1")?;
    let paths = [
        (vin, vec![j1, [j1[0], r1a[1]], r1a]),
        (led, vec![r1b, [anode[0], r1b[1]], anode]),
        (
            gnd.clone(),
            vec![j2, [j2[0], 14.0], [cathode[0], 14.0], cathode],
        ),
    ];
    let mut routes = Vec::new();
    for (net, path) in paths {
        for leg in path.windows(2) {
            routes.push(json!({"op": "layout.route", "params": {"request": {
                "net": net, "start": leg[0], "end": leg[1], "width": 0.25, "layer": "Top"}}}));
        }
    }
    client.apply(json!(routes))?;
    client.apply(json!([
        {"op": "fill.add", "params": {"fill": {
            "layer": "Bottom", "net": gnd, "region": "board", "name": "GND"}}},
        {"op": "fill.refill", "params": {"target": {"fills": "all"}}},
    ]))?;

    // A component from a working pinout document: the document is built
    // and previewed by reads, and the import names neither.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/automation/src/mcp/exm48.pinout.json");
    let pinout: Value = serde_json::from_str(&std::fs::read_to_string(fixture)?)?;
    let pins = pinout["pins"].as_array().ok_or("pins")?;
    let (first, rest) = pins.split_at(40);
    client.query(
        "assist.pinout_edit",
        json!({"format": pinout["format"], "device": pinout["device"],
            "variants": pinout["variants"], "peripherals": pinout["peripherals"],
            "pins": first}),
    )?;
    client.query(
        "assist.pinout_edit",
        json!({"pins": rest, "rules": pinout["rules"]}),
    )?;
    let plan = client.query(
        "assist.pinout_preview",
        json!({"target": {"target": "new", "name": "EXM-48"}}),
    )?;
    assert_eq!(plan["change_counts"]["terminal added"], 49, "{plan}");
    let imported = client.apply(json!([{"op": "assist.pinout_apply", "params": {}}]))?;
    assert!(
        imported["changes"]["added"]["drafts.items"].is_array(),
        "{imported}"
    );

    let ready = client.query("fabrication.preflight", json!({}))?;
    assert_eq!(ready["result"]["blocking"], 0, "{ready}");
    let saved = folder.join("board.sparksmith").display().to_string();
    let (failed, reply) = client.act("project.save", json!({"path": saved}))?;
    assert!(!failed, "{reply}");
    let fab = folder.join("fab").display().to_string();
    let (failed, reply) = client.act("project.export_fabrication", json!({"output": fab}))?;
    assert!(!failed, "{reply}");
    Ok(())
}

#[test]
fn a_recorded_board_build_replays_byte_for_byte() -> Fallible {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    // The record lies beside the folder the client may write, not in it.
    let (folder, record) = (dir.path().join("run/work"), dir.path().join("run"));
    std::fs::create_dir_all(&folder)?;
    let folder = folder.canonicalize()?;
    let wire = record.join("wire.jsonl");

    let mut client = Client::start(&home, &folder, "allow-all", &wire)?;
    build(&mut client, &folder)?;
    let (sent, received) = client.finish()?;

    // 1. The wire log is the wire.
    let start = assert_wire(&wire, &sent, &received)?;
    assert_eq!(start["mode"], "allow-all");
    assert_eq!(start["folders"], json!([folder.display().to_string()]));
    assert_eq!(start["live"], false);
    assert!(start.get("project").is_none());
    // An existing log is refused before anything is served.
    let again = cli(&home, &folder)?
        .args(["mcp", "--wire-log"])
        .arg(&wire)
        .stdin(Stdio::null())
        .output()?;
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("--wire-log"));
    assert_wire(&wire, &sent, &received)?;

    // 2. The replay: what was accepted and counts, with relative paths.
    let replay = record.join("replay.jsonl");
    let (wire_arg, replay_arg) = (wire.display().to_string(), replay.display().to_string());
    let summary: Value = serde_json::from_str(&run(
        &home,
        &record,
        &["mcp-replay", &wire_arg, &replay_arg],
    )?)?;
    let lines: Vec<Value> = std::fs::read_to_string(&replay)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(summary["kept"], lines.len());
    assert_eq!(summary["session"], 2, "{summary}");
    assert_eq!(summary["refused"]["stale_revision"], 1, "{summary}");
    assert_eq!(summary["refused"]["gate.path_outside"], 1, "{summary}");
    assert_eq!(summary["dry_runs"], 1, "{summary}");
    assert_eq!(summary["state_reads"], 3, "{summary}");
    assert_eq!(summary["catalog"], 2, "{summary}");
    assert_eq!(summary["actions"], 3, "{summary}");
    assert_eq!(summary["paths"], json!(["board.sparksmith", "fab"]));
    let commands: Vec<&str> = lines.iter().filter_map(|l| l["command"].as_str()).collect();
    assert_eq!(commands.first(), Some(&"project.new"));
    assert_eq!(
        commands.get(commands.len().saturating_sub(6)..),
        Some(
            &[
                "assist.pinout_edit",
                "assist.pinout_edit",
                "assist.pinout_preview",
                "assist.pinout_apply",
                "project.save",
                "project.export_fabrication",
            ][..]
        )
    );
    // No line names the run's own folder.
    let text = std::fs::read_to_string(&replay)?;
    assert!(!text.contains(&folder.display().to_string()));
    // The converter never replaces a replay.
    assert!(run(&home, &record, &["mcp-replay", &wire_arg, &replay_arg]).is_err());

    // 3. The operator's replay in a fresh folder: no reply is a refusal.
    let fresh = dir.path().join("elsewhere");
    std::fs::create_dir_all(&fresh)?;
    let replies = cli(&home, &fresh)?
        .args(["stdio", "--operator"])
        .stdin(std::fs::File::open(&replay)?)
        .output()?;
    assert!(replies.status.success());
    let replies = String::from_utf8(replies.stdout)?;
    assert_eq!(replies.lines().count(), lines.len());
    for (reply, line) in replies.lines().zip(&lines) {
        let reply: Value = serde_json::from_str(reply)?;
        assert_eq!(reply["ok"], true, "{}: {reply}", line["command"]);
    }

    // 4. The same project, the same fabrication files.
    let built = std::fs::read(folder.join("board.sparksmith"))?;
    let rebuilt = std::fs::read(fresh.join("board.sparksmith"))?;
    assert!(
        !built.is_empty() && built == rebuilt,
        "the saved project differs"
    );
    let exported = files(&folder.join("fab"))?;
    assert!(exported.len() >= 10, "{:?}", exported.keys());
    assert!(
        exported == files(&fresh.join("fab"))?,
        "the exported files differ"
    );
    for cwd in [&folder, &fresh] {
        run(
            &home,
            cwd,
            &["fab", "board.sparksmith", "release", "--date", "none"],
        )?;
    }
    let (release, again) = (
        files(&folder.join("release"))?,
        files(&fresh.join("release"))?,
    );
    assert!(release == again, "the release files differ");
    let id = project_id(&release)?;
    assert_eq!(id, project_id(&again)?);
    // Name, project id and a 16-digit release revision.
    let revision = id
        .trim_end_matches("*%")
        .rsplit(',')
        .next()
        .unwrap_or_default();
    assert!(
        revision.len() == 16 && revision.chars().all(|c| c.is_ascii_hexdigit()),
        "{id}"
    );
    Ok(())
}

/// Under `auto-allow-edits` a headless server refuses what needs a person.
/// The requests and their refusals are in the wire log, and the converter
/// names them as the run's gate requests and replays none of them.
#[test]
fn refused_gate_requests_are_recorded_and_named() -> Fallible {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let (folder, record) = (dir.path().join("run/work"), dir.path().join("run"));
    std::fs::create_dir_all(&folder)?;
    let folder = folder.canonicalize()?;
    let wire = record.join("wire.jsonl");

    let mut client = Client::start(&home, &folder, "auto-allow-edits", &wire)?;
    let seen = client.ok("query", json!({"name": "project.inspect"}))?;
    client.revision = seen["revision"].as_u64().ok_or("a revision")?;
    client.apply(json!([{"op": "project.rename", "params": {"name": "Gated"}}]))?;
    let at = |name: &str| folder.join(name).display().to_string();
    let asked = [
        ("project.new", json!({})),
        ("project.save", json!({"path": at("board.sparksmith")})),
        ("project.export_fabrication", json!({"output": at("fab")})),
        ("schematic.export_pdf", json!({"path": at("board.pdf")})),
        (
            "project.export_native",
            json!({"path": at("board.solidcraft")}),
        ),
    ];
    for (name, params) in &asked {
        let (failed, refusal) = client.call("act", json!({"name": name, "params": params}))?;
        assert!(
            failed && refusal["code"] == "gate.needs_person",
            "{name}: {refusal}"
        );
    }
    // Publishing and a waiver change the document, so they name a revision.
    for (name, params) in [
        ("library.publish", json!({"drafts": [1]})),
        (
            "fabrication.waive",
            json!({"keys": ["fill.stale []"], "reason": "prototype"}),
        ),
    ] {
        let call = json!({"name": name, "params": params, "expected_revision": client.revision});
        assert_eq!(client.refused("act", call)?, "gate.needs_person", "{name}");
    }
    let (sent, received) = client.finish()?;
    assert_wire(&wire, &sent, &received)?;
    assert!(
        std::fs::read_dir(&folder)?.next().is_none(),
        "nothing was written"
    );

    let replay = record.join("replay.jsonl");
    let summary: Value = serde_json::from_str(&run(
        &home,
        &record,
        &[
            "mcp-replay",
            &wire.display().to_string(),
            &replay.display().to_string(),
        ],
    )?)?;
    assert_eq!(summary["refused"]["gate.needs_person"], 7, "{summary}");
    let named = summary["needs_person"].as_array().ok_or("gate requests")?;
    assert_eq!(named.len(), 7);
    for ((name, _), row) in asked.iter().zip(named) {
        assert!(row.as_str().is_some_and(|row| row.contains(name)), "{row}");
    }
    assert_eq!(summary["kept"], 1);
    let line: Value = serde_json::from_str(std::fs::read_to_string(&replay)?.trim())?;
    assert_eq!(line["command"], "project.apply_all");

    // The log is the server's; `stdio` has none.
    let refused = cli(&home, &folder)?
        .args(["stdio", "--wire-log"])
        .arg(record.join("stdio.jsonl"))
        .stdin(Stdio::null())
        .output()?;
    assert!(!refused.status.success());
    Ok(())
}

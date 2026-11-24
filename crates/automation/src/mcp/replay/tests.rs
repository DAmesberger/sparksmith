//! The converter against the server's own verbs: a run through
//! `tools::call` is recorded call by call, converted, and replayed as the
//! operator on a fresh session, which must save the same bytes. Then what
//! it must refuse, each by name, and the two record readers.
use super::super::testing::{Fallible, Tools};
use super::*;
use crate::Caller;
use crate::gate::{Grant, Mode};
use crate::session::Session;

/// A server whose every call is recorded as the wire would show it.
struct Run {
    tools: Tools,
    calls: Vec<Recorded>,
}

impl Run {
    fn new(folder: &Path) -> Fallible<Self> {
        let grant = Grant::new(Mode::AllowAll, [folder.to_path_buf()]);
        Ok(Self {
            tools: Tools::new(Session::default(), grant)?,
            calls: Vec::new(),
        })
    }

    fn call(&mut self, verb: &str, arguments: Value) -> (bool, Value) {
        let (failed, payload) = self.tools.call(verb, arguments.clone());
        let order = self.calls.len() + 1;
        self.calls.push(Recorded {
            at: format!("call {order}"),
            verb: verb.to_owned(),
            arguments,
            reply: Some(Reply::of(order, !failed, Some(&payload))),
        });
        (failed, payload)
    }

    fn ok(&mut self, verb: &str, arguments: Value) -> Fallible<Value> {
        match self.call(verb, arguments.clone()) {
            (false, payload) => Ok(payload),
            (true, payload) => Err(format!("{verb} {arguments}: {payload}").into()),
        }
    }

    fn refused(&mut self, verb: &str, arguments: Value) -> Fallible<String> {
        match self.call(verb, arguments.clone()) {
            (true, payload) => Ok(payload["code"].as_str().unwrap_or_default().to_owned()),
            (false, payload) => Err(format!("{verb} {arguments} ran: {payload}").into()),
        }
    }
}

fn rename(revision: u64, name: &str) -> Value {
    json!({"expected_revision": revision,
        "operations": [{"op": "project.rename", "params": {"name": name}}]})
}

fn revision(reply: &Value) -> Fallible<u64> {
    reply["revision"]
        .as_u64()
        .ok_or_else(|| format!("no revision in {reply}").into())
}

fn started(folder: &Path) -> Value {
    json!({"record": "start", "live": false, "folders": [folder.display().to_string()]})
}

#[test]
fn a_recorded_run_replays_to_the_same_saved_bytes() -> Fallible<()> {
    let dir = tempfile::tempdir()?;
    let (folder, fresh) = (dir.path().join("run"), dir.path().join("fresh"));
    std::fs::create_dir_all(&folder)?;
    std::fs::create_dir_all(&fresh)?;
    let saved = folder.join("board.sparksmith").display().to_string();
    let mut run = Run::new(&folder)?;

    // Reads, catalog calls, a dry run and two refusals: none is replayed.
    let seen = run.ok(QUERY, json!({"name": "project.inspect"}))?;
    let at = revision(&seen)?;
    run.ok(FIND, json!({"text": "rename the project"}))?;
    run.ok(DESCRIBE, json!({"name": "project.rename"}))?;
    let mut tried = rename(at, "Tried");
    tried["dry_run"] = json!(true);
    run.ok(APPLY, tried)?;
    assert_eq!(
        run.refused(APPLY, rename(at + 7, "Stale"))?,
        "stale_revision"
    );
    let outside = dir.path().join("outside.sparksmith").display().to_string();
    assert_eq!(
        run.refused(
            ACT,
            json!({"name": "project.save", "params": {"path": outside}})
        )?,
        "gate.path_outside"
    );

    // An edit sent the way weak models do: the list as text, the revision
    // as digits. The replay holds what the server made of it.
    let edited = run.ok(
        APPLY,
        json!({"expected_revision": at.to_string(), "operations":
            "[{\"op\":\"project.rename\",\"params\":{\"name\":\"Recorded\"}}]"}),
    )?;

    // A working pinout document, built over three reads the import needs.
    let pinout: Value = serde_json::from_str(include_str!("../exm48.pinout.json"))?;
    let pins = pinout["pins"].as_array().ok_or("pins")?;
    let (first, rest) = pins.split_at(assist::MAX_EDIT_PINS);
    run.ok(
        QUERY,
        json!({"name": "assist.pinout_edit", "params": {
            "format": pinout["format"], "device": pinout["device"],
            "variants": pinout["variants"], "peripherals": pinout["peripherals"],
            "pins": first}}),
    )?;
    run.ok(
        QUERY,
        json!({"name": "assist.pinout_edit", "params": {"pins": rest, "rules": pinout["rules"]}}),
    )?;
    run.ok(
        QUERY,
        json!({"name": "assist.pinout_preview",
            "params": {"target": {"target": "new", "name": "EXM-48"}}}),
    )?;
    let imported = run.ok(
        APPLY,
        json!({"expected_revision": revision(&edited)?,
            "operations": [{"op": "assist.pinout_apply", "params": {}}]}),
    )?;
    let sized = run.ok(
        APPLY,
        json!({"expected_revision": revision(&imported)?, "operations": [
            {"op": "layout.set_board_size", "params": {"size": [30.0, 20.0]}}]}),
    )?;
    run.ok(
        ACT,
        json!({"name": "project.save", "params": {"path": saved}}),
    )?;
    assert!(revision(&sized)? > at);

    let replay = convert(Some(&started(&folder)), &run.calls, &Options::default())?;
    let commands: Vec<&str> = replay
        .lines
        .iter()
        .filter_map(|line| line["command"].as_str())
        .collect();
    assert_eq!(
        commands,
        [
            "project.apply_all",
            "assist.pinout_edit",
            "assist.pinout_edit",
            "assist.pinout_preview",
            "assist.pinout_apply",
            "project.apply_all",
            "project.save",
        ]
    );
    let summary = &replay.summary;
    assert_eq!((summary.calls, summary.kept), (13, 7));
    assert_eq!(
        (summary.edits, summary.actions, summary.state_reads),
        (3, 1, 3)
    );
    assert_eq!(
        (summary.reads, summary.dry_runs, summary.catalog),
        (1, 1, 2)
    );
    assert_eq!(summary.refused["stale_revision"], 1);
    assert_eq!(summary.refused["gate.path_outside"], 1);
    assert_eq!(summary.paths, ["board.sparksmith"]);
    // The lenient arguments came out as the typed batch.
    assert_eq!(replay.lines[0]["params"]["expected_revision"], at);
    assert!(replay.lines[0]["params"]["operations"].is_array());
    assert_eq!(replay.lines[6]["params"]["path"], "board.sparksmith");

    // The operator's replay, in another folder: no refusal, the same file.
    let mut session = Session::default();
    for mut line in replay.lines {
        if line["command"] == "project.save" {
            let path = line["params"]["path"].as_str().ok_or("a path")?;
            line["params"]["path"] = json!(fresh.join(path).display().to_string());
        }
        let reply = session.respond(Caller::Operator, line.clone());
        assert_eq!(reply["ok"], true, "{line}: {reply}");
    }
    assert!(std::fs::read(&saved)? == std::fs::read(fresh.join("board.sparksmith"))?);
    Ok(())
}

fn answered(order: usize, revision: Option<u64>) -> Option<Reply> {
    Some(Reply {
        order,
        ok: true,
        revision,
        ..Reply::default()
    })
}

fn recorded(n: usize, verb: &str, arguments: Value, reply: Option<Reply>) -> Recorded {
    Recorded {
        at: format!("call {n}"),
        verb: verb.to_owned(),
        arguments,
        reply,
    }
}

fn refusal(calls: &[Recorded], options: &Options) -> String {
    let started = json!({"record": "start", "live": false, "folders": ["/work/run"]});
    convert(Some(&started), calls, options)
        .err()
        .unwrap_or_default()
}

#[test]
fn what_cannot_be_mapped_is_refused_naming_the_call() {
    let none = Options::default();
    // Accepted in the run, unknown to this build.
    let unknown = json!({"expected_revision": 0,
        "operations": [{"op": "layout.no_such_edit", "params": {}}]});
    let why = refusal(&[recorded(1, APPLY, unknown, answered(1, Some(1)))], &none);
    assert!(
        why.contains("call 1") && why.contains("does not read at this build"),
        "{why}"
    );

    // A revision the kept calls do not explain.
    let calls = [
        recorded(1, APPLY, rename(0, "A"), answered(1, Some(1))),
        recorded(2, APPLY, rename(5, "B"), answered(2, Some(6))),
    ];
    let why = refusal(&calls, &none);
    assert!(
        why.contains("call 2") && why.contains("revision 5"),
        "{why}"
    );

    // Answers in another order than the requests.
    let calls = [
        recorded(1, APPLY, rename(0, "A"), answered(2, None)),
        recorded(2, APPLY, rename(1, "B"), answered(1, None)),
    ];
    let why = refusal(&calls, &none);
    assert!(why.contains("call 2") && why.contains("order"), "{why}");

    // An edit without an answer; a read without one is only counted.
    let calls = [recorded(1, APPLY, rename(0, "A"), None)];
    let why = refusal(&calls, &none);
    assert!(why.contains("call 1") && why.contains("no answer"), "{why}");

    // An edit moved onto changes the record does not hold.
    let mut moved = answered(1, Some(4));
    if let Some(reply) = &mut moved {
        reply.rebased = true;
    }
    let why = refusal(&[recorded(1, APPLY, rename(0, "A"), moved)], &none);
    assert!(why.contains("call 1") && why.contains("rebased"), "{why}");

    // A path outside the run's folder.
    let save = json!({"name": "project.save", "params": {"path": "/elsewhere/b.sparksmith"}});
    let why = refusal(&[recorded(1, ACT, save, answered(1, Some(0)))], &none);
    assert!(
        why.contains("call 1") && why.contains("/elsewhere/b.sparksmith"),
        "{why}"
    );

    // Nothing kept at all.
    let inspect = json!({"name": "project.inspect"});
    let why = refusal(&[recorded(1, QUERY, inspect, None)], &none);
    assert!(why.contains("nothing to replay"), "{why}");
}

#[test]
fn a_read_without_an_answer_is_counted_not_kept() -> Fallible<()> {
    let calls = [
        recorded(1, APPLY, rename(0, "A"), answered(1, Some(1))),
        recorded(2, QUERY, json!({"name": "project.inspect"}), None),
    ];
    let replay = convert(None, &calls, &Options::default())?;
    assert_eq!((replay.summary.kept, replay.summary.unanswered), (1, 1));
    Ok(())
}

/// A run that changed two projects converts one at a time: each replay is
/// one session, and the one the run opened starts with its `project.new`
/// at the revision a session starts with.
#[test]
fn several_projects_convert_one_at_a_time() -> Fallible<()> {
    let on = |handle: &str, order: usize, revision: u64| {
        Some(Reply {
            order,
            ok: true,
            project: Some(handle.to_owned()),
            revision: Some(revision),
            ..Reply::default()
        })
    };
    let (first, second) = ("prj_first", "prj_second");
    let mut later = rename(1, "Second");
    later["project"] = json!(second);
    let mut earlier = rename(1, "First again");
    earlier["project"] = json!(first);
    let calls = [
        recorded(1, APPLY, rename(0, "First"), on(first, 1, 1)),
        recorded(
            2,
            ACT,
            json!({"name": "project.new", "params": {}}),
            on(second, 2, 1),
        ),
        recorded(3, APPLY, later, on(second, 3, 2)),
        recorded(4, APPLY, earlier, on(first, 4, 2)),
    ];
    let why = convert(None, &calls, &Options::default())
        .err()
        .unwrap_or_default();
    assert!(
        why.contains("changed 2 projects") && why.contains("--session"),
        "{why}"
    );
    let pick = |session| Options {
        session: Some(session),
        ..Options::default()
    };
    let one = convert(None, &calls, &pick(1))?;
    assert_eq!(one.lines.len(), 2);
    assert_eq!(one.lines[1]["params"]["expected_revision"], 1);
    let two = convert(None, &calls, &pick(2))?;
    assert_eq!(two.summary.sessions, [2, 2]);
    assert_eq!(
        two.lines[0],
        json!({"command": "project.new", "params": {"expected_revision": 0}})
    );
    assert_eq!(two.lines[1]["command"], "project.apply_all");
    assert!(convert(None, &calls, &pick(3)).is_err());
    Ok(())
}

/// A refused preview keeps the document it was given. A call that builds
/// on it cannot be replayed; one that replaces it can.
#[test]
fn a_call_that_builds_on_a_refused_preview_is_refused() -> Fallible<()> {
    let target = json!({"target": "new", "name": "X"});
    let preview = |document: Value| {
        json!({"name": "assist.pinout_preview",
            "params": {"target": target, "document": document}})
    };
    let failed = Some(Reply {
        order: 1,
        ok: false,
        code: Some("invalid_input".into()),
        ..Reply::default()
    });
    let edit = json!({"name": "assist.pinout_edit", "params": {"pins": [{"row": "1"}]}});
    let calls = [
        recorded(1, QUERY, preview(json!({"format": 1})), failed.clone()),
        recorded(2, QUERY, edit, answered(2, None)),
    ];
    let why = convert(None, &calls, &Options::default())
        .err()
        .unwrap_or_default();
    assert!(
        why.contains("call 2") && why.contains("refused call 1"),
        "{why}"
    );
    let calls = [
        recorded(1, QUERY, preview(json!({"format": 1})), failed),
        recorded(
            2,
            QUERY,
            preview(json!({"format": 1, "pins": []})),
            answered(2, None),
        ),
    ];
    let replay = convert(None, &calls, &Options::default())?;
    assert_eq!(replay.summary.state_reads, 1);
    assert_eq!(replay.summary.refused["invalid_input"], 1);
    Ok(())
}

/// The reads kept are exactly the ones declared to answer from the
/// session: today the `assist` entries.
#[test]
fn the_reads_kept_are_the_ones_that_answer_from_the_session() {
    let kept: Vec<&str> = metadata::entries()
        .iter()
        .filter(|entry| holds_session_state(entry))
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        kept,
        [
            "assist.document_text",
            "assist.document_page",
            "assist.pinout_edit",
            "assist.pinout_preview",
            "assist.workflows",
            "assist.step_result",
        ]
    );
}

fn wire_row(n: usize, record: &str, message: &Value) -> String {
    json!({"record": record, "n": n, "t_ms": 1, "text": message.to_string()}).to_string()
}

#[test]
fn a_wire_log_reads_as_its_calls_and_their_answers() -> Fallible<()> {
    let call = |id: u64, name: &str, arguments: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}})
    };
    let result = |id: u64, failed: bool, payload: Value| {
        json!({"jsonrpc": "2.0", "id": id, "result": {
            "isError": failed, "content": [{"type": "text", "text": payload.to_string()}]}})
    };
    let log = [
        json!({"record": "start", "format": 1, "live": false, "mode": "auto-allow-edits",
            "folders": ["/work/run"]})
        .to_string(),
        wire_row(1, "in", &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})),
        wire_row(2, "out", &json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": []}})),
        wire_row(3, "in", &call(2, APPLY, rename(0, "A"))),
        json!({"record": "in", "n": 4, "t_ms": 1, "text": "{not json"}).to_string(),
        wire_row(5, "out", &result(2, false, json!({"revision": 1, "project": "prj_a"}))),
        wire_row(
            6,
            "in",
            &call(3, ACT, json!({"name": "project.save", "params": {"path": "/work/run/a.sparksmith"}})),
        ),
        wire_row(
            7,
            "out",
            &result(3, true, json!({"code": "gate.needs_person", "message": "Refused"})),
        ),
        wire_row(8, "in", &call(4, "project_inspect", json!({}))),
        wire_row(
            9,
            "out",
            &json!({"jsonrpc": "2.0", "id": 4, "error": {"code": -32602, "message": "unknown tool"}}),
        ),
    ]
    .join("\n");
    let (start, calls) = read_wire(&mut log.as_bytes())?;
    assert_eq!(start["mode"], "auto-allow-edits");
    assert_eq!(calls.len(), 3);
    assert!(calls[0].at.contains("wire row 3") && calls[0].at.contains("project.rename"));
    let reply = calls[0].reply.as_ref().ok_or("an answer")?;
    assert_eq!(
        (reply.ok, reply.revision, reply.project.as_deref()),
        (true, Some(1), Some("prj_a"))
    );
    let replay = convert(Some(&start), &calls, &Options::default())?;
    assert_eq!(replay.summary.kept, 1);
    // The refused gate request is in the record and named, not replayed.
    assert_eq!(replay.summary.refused["gate.needs_person"], 1);
    assert_eq!(replay.summary.refused["jsonrpc -32602"], 1);
    assert_eq!(replay.summary.needs_person.len(), 1);
    assert!(replay.summary.needs_person[0].contains("project.save"));

    assert!(read_wire(&mut "".as_bytes()).is_err());
    let headless = wire_row(1, "in", &call(1, APPLY, rename(0, "A")));
    assert!(read_wire(&mut headless.as_bytes()).is_err());
    Ok(())
}

#[test]
fn a_transcript_reads_as_its_mcp_calls() -> Fallible<()> {
    let transcript = [
        json!({"type": "system", "subtype": "init"}),
        json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "x"}},
            {"type": "tool_use", "id": "t2", "name": "mcp__board__apply", "input": rename(0, "A")},
        ]}}),
        json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "text"},
            {"type": "tool_result", "tool_use_id": "t2", "content": [
                {"type": "text", "text": "{\"revision\":1,\"project\":\"prj_a\"}"}]},
        ]}}),
        json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "t3", "name": "mcp__board__act", "input":
                {"name": "project.save", "params": {"path": "<run>/a.sparksmith"}}},
            {"type": "tool_use", "id": "t4", "name": "mcp__board__query", "input":
                {"name": "project.inspect", "params": {"at": "draft:5"}}},
        ]}}),
        json!({"type": "user", "message": {"content": "a plain prompt"}}),
        json!({"type": "user", "message": {"content": [
            // A long result the transcript cut is no longer JSON.
            {"type": "tool_result", "tool_use_id": "t3", "content": [
                {"type": "text", "text": "{\"revision\":1,\"saved_pa"}]},
            {"type": "tool_result", "tool_use_id": "t4", "is_error": true,
                "content": "{\"code\":\"not_found\",\"message\":\"nothing there\"}"},
        ]}}),
    ]
    .map(|row| row.to_string())
    .join("\n");
    let calls = read_transcript(&mut transcript.as_bytes())?;
    assert_eq!(calls.len(), 3);
    let options = Options {
        root: Some("<run>".into()),
        ..Options::default()
    };
    let replay = convert(None, &calls, &options)?;
    assert_eq!(replay.summary.kept, 2);
    assert_eq!(replay.summary.refused["not_found"], 1);
    assert_eq!(replay.lines[1]["params"]["path"], "a.sparksmith");
    // Without a root a path cannot be made relative.
    let why = convert(None, &calls, &Options::default())
        .err()
        .unwrap_or_default();
    assert!(why.contains("--root"), "{why}");
    Ok(())
}

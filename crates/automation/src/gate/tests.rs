//! The gate through its public functions: the decision table, the folder
//! limit, a run that was told to publish, and MCP under each mode.
use super::*;
use crate::mcp::testing::Tools;
use sparksmith_core::{Engine, Operation, Query};
use std::collections::BTreeSet;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// `path` made absolute on this platform: Windows needs a drive, so
/// `/work` becomes `C:/work` there.
fn abs(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.to_owned()
    }
}

fn request(command: &str, params: Value) -> Fallible<Request> {
    Ok(serde_json::from_value(
        json!({"command": command, "params": params}),
    )?)
}

fn apply(operation: Value) -> Fallible<Request> {
    request(
        "project.apply",
        json!({"expected_revision": 0, "operation": operation}),
    )
}

/// `run`, `ask` or the refusal's code.
fn verdict(policy: &Grant, request: &Request) -> String {
    match decide(policy, request) {
        Decision::Run => "run".into(),
        Decision::Ask(_) => "ask".into(),
        Decision::Refuse(refusal) => refusal.code,
    }
}

const MODES: [Mode; 3] = [Mode::AskEveryTime, Mode::AutoAllowEdits, Mode::AllowAll];

#[test]
fn each_mode_treats_each_class_as_decided() -> Fallible {
    let read = request("project.inspect", json!({}))?;
    let edit = request("project.undo", json!({"expected_revision": 0}))?;
    let ask = request("project.new", json!({"expected_revision": 0}))?;
    let never = request(
        "library.favourite",
        json!({"key": {"source": "project", "lineage": 1}, "favourite": true}),
    )?;
    for (mode, expected) in [
        (Mode::AskEveryTime, ["run", "ask", "ask"]),
        (Mode::AutoAllowEdits, ["run", "run", "ask"]),
        (Mode::AllowAll, ["run", "run", "run"]),
    ] {
        let policy = Grant::new(mode, []);
        let got = [
            verdict(&policy, &read),
            verdict(&policy, &edit),
            verdict(&policy, &ask),
        ];
        assert_eq!(got, expected, "{mode:?}");
        assert_eq!(verdict(&policy, &never), "gate.never", "{mode:?}");
    }
    assert_eq!(Mode::default(), Mode::AutoAllowEdits);
    Ok(())
}

#[test]
fn project_apply_is_classed_by_its_operation() -> Fallible {
    let policy = Grant::default();
    let edit = apply(json!({"editor": "Library", "operation": {
        "command": "library.discard_draft", "params": {"id": 1}}}))?;
    assert_eq!(edit.effective_class(), Class::Edit);
    assert_eq!(verdict(&policy, &edit), "run");

    let publish = apply(json!({"editor": "Library", "operation": {
        "command": "library.publish", "params": {"drafts": []}}}))?;
    assert_eq!(publish.effective_class(), Class::Ask);
    assert_eq!(publish.subject(), "library.publish");
    assert_eq!(verdict(&policy, &publish), "ask");

    let query: Request = serde_json::from_value(
        json!({"command": "project.query", "params": {"query": "project.bom", "params": {}}}),
    )?;
    assert_eq!(query.effective_class(), Class::Read);
    assert_eq!(query.subject(), "project.bom");
    Ok(())
}

#[test]
fn a_batch_takes_its_strictest_member() -> Fallible {
    let policy = Grant::default();
    let read = request("project.inspect", json!({}))?;
    let edit = request("project.undo", json!({"expected_revision": 0}))?;
    let ask = request("project.new", json!({"expected_revision": 0}))?;
    let never = request(
        "library.favourite",
        json!({"key": {"source": "project", "lineage": 1}, "favourite": true}),
    )?;
    assert!(matches!(
        decide_batch(&policy, &[read.clone(), edit.clone()]),
        Decision::Run
    ));
    let Decision::Ask(question) = decide_batch(&policy, &[read.clone(), ask.clone(), edit]) else {
        return Err("a batch with an Ask member asks".into());
    };
    assert_eq!(question.command, "project.new");
    assert_eq!(question.class, Class::Ask);
    let Decision::Refuse(refusal) = decide_batch(&policy, &[read, ask, never]) else {
        return Err("a batch with a Never member is refused".into());
    };
    assert_eq!(refusal.code, "gate.never");
    Ok(())
}

#[test]
fn a_path_outside_the_folders_is_refused_in_every_mode() -> Fallible {
    let open = |path: &str| {
        request(
            "project.open",
            json!({"expected_revision": 0, "path": path}),
        )
    };
    for mode in MODES {
        let policy = Grant::new(mode, [PathBuf::from(abs("/work/board"))]);
        for outside in [
            abs("/work/other/x.sparksmith"),
            // A sibling that shares the folder's text is not inside it.
            abs("/work/board-evil/x.sparksmith"),
            // `..` is refused even where it would land inside.
            abs("/work/board/../other/x.sparksmith"),
            abs("/work/board/../board/x.sparksmith"),
            // Relative paths depend on the process's folder.
            "x.sparksmith".into(),
            "../x.sparksmith".into(),
            String::new(),
        ] {
            assert_eq!(
                verdict(&policy, &open(&outside)?),
                "gate.path_outside",
                "{outside}"
            );
        }
        let inside = verdict(&policy, &open(&abs("/work/board/sub/x.sparksmith"))?);
        let expected = if mode == Mode::AllowAll { "run" } else { "ask" };
        assert_eq!(inside, expected);
    }
    // No folders: every path is refused, and the refusal says what to do.
    let file = abs("/work/board/x.sparksmith");
    let Decision::Refuse(refusal) = decide(&Grant::default(), &open(&file)?) else {
        return Err("no folders allow no path".into());
    };
    let message = refusal.message.clone();
    for part in ["project.open", &file, "Instead:"] {
        assert!(message.contains(part), "{message}");
    }
    Ok(())
}

#[test]
fn naming_a_path_makes_a_request_at_least_ask() -> Fallible {
    let policy = Grant::new(Mode::AutoAllowEdits, [PathBuf::from(abs("/work"))]);
    let builtin = request(
        "library.check_part_file",
        json!({"builtin": "esp32-s31-wroom-3"}),
    )?;
    assert_eq!(builtin.effective_class(), Class::Read);
    assert_eq!(verdict(&policy, &builtin), "run");
    let file = request(
        "library.check_part_file",
        json!({"path": abs("/work/part.solidcraft")}),
    )?;
    assert_eq!(file.effective_class(), Class::Ask);
    assert_eq!(verdict(&policy, &file), "ask");
    let template = request(
        "firmware.export_preview",
        json!({"instance": 1, "emitter": {"emitter": "json"}, "template_file": abs("/elsewhere/t.rhai")}),
    )?;
    assert_eq!(verdict(&policy, &template), "gate.path_outside");
    let no_template = request(
        "firmware.export_preview",
        json!({"instance": 1, "emitter": {"emitter": "json"}}),
    )?;
    assert_eq!(verdict(&policy, &no_template), "run");
    Ok(())
}

fn looks_like_a_path(name: &str) -> bool {
    matches!(name, "path" | "output" | "file" | "folder" | "dir")
        || name.ends_with("_path")
        || name.ends_with("_file")
        || name.ends_with("_folder")
        || name.ends_with("_dir")
}

/// Paths of `schema`'s `properties`, its definitions included, by name.
fn path_properties(schema: &Value, out: &mut BTreeSet<String>) {
    match schema {
        Value::Object(object) => {
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                out.extend(properties.keys().filter(|k| looks_like_a_path(k)).cloned());
            }
            object
                .values()
                .for_each(|inner| path_properties(inner, out));
        }
        Value::Array(items) => items.iter().for_each(|inner| path_properties(inner, out)),
        _ => {}
    }
}
/// Independent of `Request::paths`: the commands whose generated schema has a
/// property named like a path are exactly the ones whose parameters type it
/// as a [`sparksmith_core::permission::FilePath`], which is what the tool
/// descriptions and schema rows say asks with a path, and the gate sees each
/// path such a request names.
#[test]
fn every_path_a_command_can_name_reaches_the_gate() -> Fallible {
    let mut named = BTreeSet::new();
    for info in Request::catalog() {
        let mut found = BTreeSet::new();
        path_properties(&info.params, &mut found);
        if !found.is_empty() {
            named.insert(info.id);
        }
    }
    let typed: BTreeSet<&str> = Request::catalog_shared()
        .iter()
        .filter(|info| info.takes_path)
        .map(|info| info.id)
        .collect();
    assert_eq!(named, typed);
    assert_eq!(typed.len(), 22, "{typed:?}");
    let at = "/samples/f";
    let policy = Grant::new(Mode::AllowAll, [PathBuf::from("/allowed")]);
    for (command, params, count) in [
        (
            "project.open",
            json!({"expected_revision": 0, "path": at}),
            1,
        ),
        ("project.export_fabrication", json!({"output": at}), 1),
        ("library.source.add", json!({"path": at}), 1),
        ("assembly.preview", json!({"path": at, "design": at}), 2),
        (
            "firmware.export",
            json!({"instance": 1, "emitter": {"emitter": "json"}, "path": at}),
            1,
        ),
    ] {
        let sample = request(command, params)?;
        assert_eq!(sample.paths()?, vec![at; count], "{command}");
        assert!(sample.effective_class() >= Class::Ask, "{command}");
        let Decision::Refuse(refusal) = decide(&policy, &sample) else {
            return Err(format!("{command} ran outside the folders").into());
        };
        assert!(
            ["gate.path_outside", "gate.never"].contains(&refusal.code.as_str()),
            "{command}: {}",
            refusal.code
        );
    }
    Ok(())
}
/// The class tables and the catalog agree, so nothing falls back to Never.
#[test]
fn every_command_operation_and_query_has_a_class() -> Fallible {
    let listed: BTreeSet<&str> = Request::ENTRIES.iter().map(|entry| entry.name).collect();
    let catalog: BTreeSet<&str> = Request::catalog().iter().map(|info| info.id).collect();
    assert_eq!(listed, catalog);

    let mut session = Session::default();
    let index = session.execute(Request::Schema(Default::default()))?;
    let row_class = |row: &Value| row["class"].as_str().map(str::to_owned);
    let mut operations = 0usize;
    for (_, rows) in index["operations"].as_object().ok_or("operations")? {
        for row in rows.as_array().ok_or("rows")? {
            operations += 1;
            let name = row["name"].as_str().ok_or("name")?;
            let class = Operation::entries()
                .find(|entry| entry.name == name)
                .map(|entry| entry.class)
                .ok_or_else(|| format!("{name} has no class"))?;
            let expected = (class != Class::Edit).then(|| class.name().to_owned());
            assert_eq!(row_class(row), expected, "{name}");
        }
    }
    assert_eq!(operations, Operation::entries().count());
    let queries = index["queries"].as_array().ok_or("queries")?;
    assert_eq!(queries.len(), Query::ENTRIES.len());
    for row in queries {
        assert_eq!(row_class(row), None, "{row}");
    }
    for row in index["commands"].as_array().ok_or("commands")? {
        assert!(row["class"].is_string(), "{row}");
    }
    Ok(())
}

#[test]
fn a_model_can_see_what_will_need_asking() -> Fallible {
    let mut session = Session::default();
    let schema = session
        .dispatch(json!({"command": "project.schema", "params": {"name": "library.publish"}}));
    assert_eq!(schema["result"]["matches"][0]["class"], "ask", "{schema}");
    // Over MCP, `find` names each hit's class and the verb its kind takes;
    // the Never class is not found at all.
    let tools = Tools::new(Session::default(), Grant::default())?;
    let hit = |text: &str| -> Fallible<Value> {
        let found = tools.ok("find", json!({"text": text, "limit": 50}))?;
        Ok(found["hits"]
            .as_array()
            .and_then(|hits| hits.iter().find(|h| h["name"] == text))
            .cloned()
            .unwrap_or(Value::Null))
    };
    let save = hit("project.save")?;
    assert_eq!(
        (&save["class"], &save["kind"]),
        (&json!("ask"), &json!("action"))
    );
    let inspect = hit("project.inspect")?;
    assert_eq!(
        (&inspect["class"], &inspect["kind"]),
        (&json!("read"), &json!("query"))
    );
    assert_eq!(hit("library.favourite")?, Value::Null);
    Ok(())
}

/// A session with a publishable draft and a writable library folder, in
/// `root`, as a host would have set up before a run starts.
fn host_session(root: &Path) -> Fallible<(Session, Value, Value, Value, PathBuf)> {
    let mut session = Session::new(Engine::demo()?);
    let folder = root.join("library");
    let created = session.dispatch(json!({"command": "library.source.create", "params": {
        "path": folder.display().to_string(), "name": "Host library"}}));
    assert_eq!(created["ok"], true, "{created}");
    let library = created["result"]["created"]["library"].clone();
    // The demo keeps drafts of its components: start from none.
    let drafts: Vec<Value> = session
        .engine()
        .drafts()
        .items
        .iter()
        .map(|draft| serde_json::to_value(draft.id))
        .collect::<std::result::Result<_, _>>()?;
    for id in drafts {
        let discarded = session.dispatch(json!({"command": "project.apply", "params": {
            "expected_revision": session.engine().revision(),
            "operation": {"editor": "Library", "operation": {
                "command": "library.discard_draft", "params": {"id": id}}},
        }}));
        assert_eq!(discarded["ok"], true, "{discarded}");
    }
    let components: Vec<Value> = session
        .engine()
        .project()
        .library
        .latest_components()
        .iter()
        .map(|c| json!({"kind": "component", "id": c.id, "revision": c.revision}))
        .collect();
    let mut found = None;
    for component in components {
        let drafted = session.dispatch(json!({"command": "project.apply", "params": {
            "expected_revision": session.engine().revision(),
            "operation": {"editor": "Library", "operation": {
                "command": "library.draft_from", "params": {"revision": component}}},
        }}));
        if drafted["ok"] == true {
            found = Some((component, drafted));
            break;
        }
    }
    let (component, drafted) = found.ok_or("a demo component without a draft")?;
    let draft_id = drafted["result"]["outcome"]["id"].clone();
    let draft = drafted["result"]["drafts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| d["draft"].clone())
        .find(|d| d["id"] == draft_id)
        .ok_or("the new draft")?;
    let mut meta = draft["meta"].clone();
    meta["description"] = json!("A second revision");
    let edited = session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": session.engine().revision(),
        "operation": {"editor": "Library", "operation": {"command": "library.edit", "params": {
            "edits": [{"id": draft_id, "meta": meta, "definition": draft["definition"]}]}}},
    }}));
    assert_eq!(edited["ok"], true, "{edited}");
    Ok((session, component, library, draft_id, folder))
}

fn file_count(folder: &Path) -> Fallible<usize> {
    let mut count = 0;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(next)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// A datasheet told the model to publish and to write files. The model obeys;
/// the requests it sends are well formed, inside the allowed folder and even
/// ask for the verbose route. Nothing happens until a person allows it.
#[test]
fn a_request_from_a_run_cannot_publish_or_write_without_allow() -> Fallible {
    let root = tempfile::tempdir()?;
    let (mut session, component, library, draft, folder) = host_session(root.path())?;
    let policy = Grant::new(Mode::AutoAllowEdits, [root.path().to_path_buf()]);
    let revision = session.engine().revision();
    let records_before = file_count(&folder)?;
    let drafts_before = session.engine().drafts().items.len();
    let at = |name: &str| root.path().join(name).display().to_string();

    let publish = json!({"command": "project.apply", "verbose": true, "params": {
        "expected_revision": revision,
        "operation": {"editor": "Library", "operation": {
            "command": "library.publish", "params": {"drafts": [draft]}}},
    }});
    let promote = json!({"command": "library.publish_to_folder", "params": {
        "expected_revision": revision, "root": component, "library": library}});
    let writes = [
        (
            "project.save",
            json!({"path": at("stolen.sparksmith")}),
            "stolen.sparksmith",
        ),
        (
            "project.export_native",
            json!({"path": at("m.solidcraft")}),
            "m.solidcraft",
        ),
        (
            "schematic.export_pdf",
            json!({"path": at("s.pdf")}),
            "s.pdf",
        ),
        (
            "project.export_fabrication",
            json!({"output": at("fab")}),
            "fab",
        ),
    ];
    let mut attempts: Vec<Value> = vec![publish.clone(), promote.clone()];
    attempts.extend(
        writes
            .iter()
            .map(|(command, params, _)| json!({"command": command, "params": params})),
    );
    for attempt in &attempts {
        assert!(
            matches!(session.check_request(&policy, attempt), Decision::Ask(_)),
            "{attempt}"
        );
        let reply = session.respond(Caller::Model(&policy), attempt.clone());
        assert_eq!(reply["ok"], false, "{reply}");
        assert_eq!(reply["error"]["code"], "gate.needs_person", "{reply}");
        let message = reply["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("no way to ask"), "{message}");
        assert!(message.contains("Instead:"), "{message}");
    }
    assert_eq!(session.engine().revision(), revision);
    assert_eq!(session.engine().drafts().items.len(), drafts_before);
    assert_eq!(file_count(&folder)?, records_before);
    for (_, _, name) in &writes {
        assert!(!root.path().join(name).exists(), "{name}");
    }

    // The same requests run when the host has a person's Allow: the gate was
    // what stopped them, not a failure of their own.
    let saved = session.respond(Caller::Allowed(&policy), attempts[2].clone());
    assert_eq!(saved["ok"], true, "{saved}");
    assert!(root.path().join("stolen.sparksmith").exists());
    let promoted = session.respond(Caller::Allowed(&policy), promote);
    assert_eq!(promoted["ok"], true, "{promoted}");
    assert!(file_count(&folder)? > records_before);
    let published = session.respond(
        Caller::Allowed(&policy),
        json!({"command": "project.apply", "params": {
            "expected_revision": session.engine().revision(),
            "operation": {"editor": "Library", "operation": {
                "command": "library.publish", "params": {"drafts": [draft]}}},
        }}),
    );
    assert_eq!(published["ok"], true, "{published}");
    assert_eq!(session.engine().drafts().items.len(), drafts_before - 1);
    Ok(())
}

#[test]
fn edits_run_inside_the_turn_and_settings_are_refused_with_no_way_around() -> Fallible {
    let root = tempfile::tempdir()?;
    let (mut session, _, _, draft, _folder) = host_session(root.path())?;
    let policy = Grant::new(Mode::AllowAll, [root.path().to_path_buf()]);
    let revision = session.engine().revision();
    let reply = session.respond(
        Caller::Model(&policy),
        json!({"command": "library.favourite",
            "params": {"key": {"source": "project", "lineage": 1}, "favourite": true}}),
    );
    assert_eq!(reply["error"]["code"], "gate.never", "{reply}");

    let reply = session.respond(
        Caller::Model(&Grant::default()),
        json!({"command": "project.apply", "params": {
            "expected_revision": revision,
            "operation": {"editor": "Library", "operation": {
                "command": "library.discard_draft", "params": {"id": draft}}},
        }}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(session.engine().revision(), revision + 1);
    Ok(())
}

#[test]
fn saving_without_a_path_counts_as_naming_the_current_file() -> Fallible {
    let root = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    let file = elsewhere.path().join("board.sparksmith");
    let mut session = Session::new(Engine::demo()?);
    let saved = session.dispatch(
        json!({"command": "project.save", "params": {"path": file.display().to_string()}}),
    );
    assert_eq!(saved["ok"], true, "{saved}");
    let before = std::fs::read(&file)?;
    let policy = Grant::new(Mode::AllowAll, [root.path().to_path_buf()]);
    let request = json!({"command": "project.save", "params": {}});
    let reply = session.respond(Caller::Model(&policy), request.clone());
    assert_eq!(reply["error"]["code"], "gate.path_outside", "{reply}");
    assert_eq!(std::fs::read(&file)?, before);
    let policy = Grant::new(Mode::AllowAll, [elsewhere.path().to_path_buf()]);
    let reply = session.respond(Caller::Model(&policy), request);
    assert_eq!(reply["ok"], true, "{reply}");
    Ok(())
}

/// Unix only: making a symbolic link on Windows needs a privilege or
/// Developer Mode that an ordinary account lacks.
#[cfg(unix)]
#[test]
fn a_link_cannot_lead_out_of_an_allowed_folder() -> Fallible {
    let allowed = tempfile::tempdir()?;
    let secret = tempfile::tempdir()?;
    std::os::unix::fs::symlink(secret.path(), allowed.path().join("door"))?;
    std::os::unix::fs::symlink(
        secret.path().join("not-yet"),
        allowed.path().join("dangling.sparksmith"),
    )?;
    let session = Session::default();
    let policy = Grant::new(Mode::AllowAll, [allowed.path().to_path_buf()]);
    let check = |path: PathBuf| {
        session.check_request(
            &policy,
            &json!({"command": "project.save", "params": {"path": path.display().to_string()}}),
        )
    };
    for path in [
        allowed.path().join("door/stolen.sparksmith"),
        allowed.path().join("dangling.sparksmith"),
    ] {
        let Decision::Refuse(refusal) = check(path.clone()) else {
            return Err(format!("{} passed", path.display()).into());
        };
        assert_eq!(refusal.code, "gate.path_outside");
    }
    // A file not yet made, in a folder not yet made, is still inside.
    assert!(matches!(
        check(allowed.path().join("new/sub/board.sparksmith")),
        Decision::Run
    ));
    // The allowed folder may itself be reached through a link.
    let via = tempfile::tempdir()?;
    let link = via.path().join("work");
    std::os::unix::fs::symlink(allowed.path(), &link)?;
    let policy = Grant::new(Mode::AllowAll, [link]);
    let reply = session.check_request(
        &policy,
        &json!({"command": "project.save", "params": {
            "path": allowed.path().join("x.sparksmith").display().to_string()}}),
    );
    assert!(matches!(reply, Decision::Run), "{reply:?}");
    Ok(())
}

#[test]
fn a_malformed_request_is_refused_with_its_parse_error() -> Fallible {
    let session = Session::default();
    let Decision::Refuse(refusal) = session.check_request(
        &Grant::default(),
        &json!({"command": "project.save", "params": {"path": 7}}),
    ) else {
        return Err("a malformed request does not run".into());
    };
    assert_eq!(refusal.code, "malformed_request");
    assert!(refusal.message.starts_with("Refused project.save:"));
    assert!(refusal.message.clone().contains("describe"));
    Ok(())
}

#[test]
fn a_denied_question_answers_the_model() -> Fallible {
    let Decision::Ask(question) = decide(
        &Grant::default(),
        &request("project.new", json!({"expected_revision": 0}))?,
    ) else {
        return Err("project.new asks".into());
    };
    assert_eq!(question.denied().code, "gate.denied");
    assert!(question.denied().message.contains("denied"));
    let wire = serde_json::to_value(Decision::Ask(question))?;
    assert_eq!(wire["decision"], "ask");
    assert_eq!(wire["class"], "ask");
    Ok(())
}

/// MCP has no way to ask, so its gate refuses what would need a person;
/// each mode moves that line, and a server without one allows edits only.
/// The Never class is not even in the catalog a model reaches.
#[test]
fn mcp_runs_under_each_mode_and_allows_edits_by_default() -> Fallible {
    let root = tempfile::tempdir()?;
    let file = |name: &str| root.path().join(name).display().to_string();
    for (mode, undo_runs, save_runs) in [
        (Mode::AskEveryTime, false, false),
        (Mode::AutoAllowEdits, true, false),
        (Mode::AllowAll, true, true),
    ] {
        let name = format!("{}.sparksmith", mode.name());
        let policy = Grant::new(mode, [root.path().to_path_buf()]);
        let tools = Tools::new(Session::new(Engine::demo()?), policy)?;

        let (failed, _) = tools.query("project.inspect", json!({}));
        assert!(!failed, "{mode:?}: reads always run");

        let (failed, reply) = tools.apply("project.undo", json!({}));
        if undo_runs {
            // It reaches the engine: the demo has history or says it has none.
            assert_ne!(reply["code"], "gate.needs_person", "{mode:?}: {reply}");
        } else {
            assert!(failed);
            assert_eq!(reply["code"], "gate.needs_person", "{mode:?}: {reply}");
        }

        let (failed, reply) = tools.act("project.save", json!({"path": file(&name)}));
        assert_eq!(failed, !save_runs, "{mode:?}: {reply}");
        assert_eq!(root.path().join(&name).exists(), save_runs, "{mode:?}");
        if !save_runs {
            assert_eq!(reply["code"], "gate.needs_person", "{mode:?}: {reply}");
        }

        let (failed, reply) = tools.act(
            "project.save",
            json!({"path": abs("/elsewhere/x.sparksmith")}),
        );
        assert!(failed);
        assert_eq!(reply["code"], "gate.path_outside", "{mode:?}: {reply}");

        let (failed, reply) = tools.act(
            "library.favourite",
            json!({"key": {"source": "project", "lineage": 1}, "favourite": true}),
        );
        assert!(failed);
        assert_eq!(reply["code"], "catalog.unknown", "{mode:?}: {reply}");
    }

    // I-A1: the CLI's server without a mode, its project's folder allowed:
    // edits run, a save asks and so is refused.
    let policy = Grant::new(Mode::default(), [root.path().to_path_buf()]);
    let tools = Tools::new(Session::new(Engine::demo()?), policy)?;
    let (failed, reply) = tools.act("project.save", json!({"path": file("default.sparksmith")}));
    assert!(failed, "{reply}");
    assert_eq!(reply["code"], "gate.needs_person", "{reply}");
    assert!(!root.path().join("default.sparksmith").exists());
    Ok(())
}

/// Library folders are the person's own settings and files, so adding,
/// creating, removing or turning one off is Ask: a model reaches them, but
/// a headless server in its default mode refuses with `gate.needs_person`,
/// a path outside the run's folders is refused in every mode, and only
/// allow-all (the operator's explicit choice) or a person's Allow runs one.
/// Refused, the settings stay unwritten and the source list empty.
#[test]
fn a_model_adds_a_library_source_only_when_a_person_allows_it() -> Fallible {
    use crate::session::sources::Sources;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("library");
    let dirs = sparksmith_io::settings::UserDirs {
        config: root.path().join("config"),
        cache: root.path().join("cache"),
    };
    let user_session = || -> Fallible<Session> {
        Ok(Session::with_sources(
            Engine::default(),
            Sources::with_dirs(dirs.clone())?,
        ))
    };
    let settings_written = || dirs.config.exists();
    let create = json!({"path": folder.display().to_string(), "name": "Shared"});
    let allow_all = Grant::new(Mode::AllowAll, [root.path().to_path_buf()]);

    // Found and described as an action that asks.
    let edits = Grant::new(Mode::default(), [root.path().to_path_buf()]);
    let tools = Tools::new(user_session()?, edits.clone())?;
    let described = tools.ok("describe", json!({"name": "library.source.create"}))?;
    assert_eq!(described["class"], "ask", "{described}");
    assert_eq!(described["call"]["tool"], "act", "{described}");
    // The default mode cannot ask, so it refuses and writes nothing.
    let (failed, reply) = tools.act("library.source.create", create.clone());
    assert!(failed, "{reply}");
    assert_eq!(reply["code"], "gate.needs_person", "{reply}");
    assert!(!folder.exists());
    assert!(!settings_written());
    // Outside the run's folders, refused even in allow-all.
    let outside = tempfile::tempdir()?;
    let tools = Tools::new(user_session()?, allow_all)?;
    let (failed, reply) = tools.act(
        "library.source.create",
        json!({"path": outside.path().join("lib").display().to_string(), "name": "Elsewhere"}),
    );
    assert!(failed, "{reply}");
    assert_eq!(reply["code"], "gate.path_outside", "{reply}");
    assert!(!settings_written());
    // Allow-all creates the folder and lists it; remove takes it off again.
    let (failed, reply) = tools.act("library.source.create", create);
    assert!(!failed, "{reply}");
    assert!(folder.join("sparksmith-library.json").exists());
    assert_eq!(tools.with_session(|s| s.sources().list().len()), Some(1));
    assert!(settings_written());
    let path = json!({"path": folder.display().to_string()});
    let (failed, reply) = tools.act("library.source.remove", path.clone());
    assert!(!failed, "{reply}");
    assert_eq!(tools.with_session(|s| s.sources().list().len()), Some(0));

    // Over gated JSON lines: refused for a model that cannot be asked, run
    // after the person's Allow.
    let request = json!({"command": "library.source.add", "params": path});
    let mut session = user_session()?;
    let reply = session.respond(Caller::Model(&edits), request.clone());
    assert_eq!(reply["error"]["code"], "gate.needs_person", "{reply}");
    assert!(session.sources().list().is_empty());
    let reply = session.respond(Caller::Allowed(&edits), request);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(session.sources().list().len(), 1);
    Ok(())
}

/// I-A1: the server instructions never tell a model to call a request,
/// operation or query of the Never class, which every model is refused.
#[test]
fn the_instructions_name_no_never_class_command() -> Fallible {
    let instructions = crate::mcp::instructions(true)?;
    let never = Request::ENTRIES
        .iter()
        .chain(Operation::entries())
        .chain(Query::ENTRIES)
        .filter(|entry| entry.class == Class::Never)
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    assert!(never.contains(&"library.favourite"), "{never:?}");
    for name in never {
        assert!(
            !instructions.contains(name),
            "the instructions name {name}, which is never run for a model"
        );
    }
    Ok(())
}

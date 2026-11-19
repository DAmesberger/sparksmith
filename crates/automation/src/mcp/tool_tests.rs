//! P2 and S3 evidence: the six tools and project handles, through rmcp's own
//! client at `2026-07-28` against the server, as a model would call them.
use super::tools::{ACT, APPLY, DESCRIBE, FIND, QUERY, SHOW, TOOLS_BUDGET};
use super::*;
use crate::gate::Mode;
use rmcp::model::{ClientCapabilities, ClientConfig, ErrorCode};
use rmcp::service::{RunningService, ServiceError};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt, RoleClient};
use serde_json::json;
use std::future::Future;
use std::path::Path;

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn block_on<F: Future>(future: F) -> Fallible<F::Output> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future))
}

#[derive(Clone)]
struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("sparksmith-p2-client", "0"),
        )
    }
}

/// A connected client and the server task.
struct Connection {
    client: RunningService<RoleClient, Client>,
    serving: tokio::task::JoinHandle<Result<(), String>>,
}

impl Connection {
    async fn open(mode: Mode, folder: &Path) -> Fallible<Self> {
        let policy = Grant::new(mode, [folder.to_path_buf()]);
        let server = Server::new(Session::default(), policy)?;
        let (client_io, server_io) = tokio::io::duplex(1 << 22);
        let (read, write) = tokio::io::split(server_io);
        let serving = tokio::spawn(server.serve(read, write));
        let client = Client
            .serve_with_lifecycle(
                client_io,
                ClientLifecycleMode::Discover {
                    preferred_versions: VERSIONS.to_vec(),
                },
            )
            .await?;
        Ok(Self { client, serving })
    }

    /// Calls `tool`: the result's JSON, and whether it is an error.
    async fn call(&self, tool: &'static str, arguments: Value) -> Fallible<(Value, bool)> {
        let Value::Object(arguments) = arguments else {
            return Err("arguments are an object".into());
        };
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
            .await?;
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .ok_or("a text result")?;
        Ok((serde_json::from_str(&text)?, result.is_error == Some(true)))
    }

    /// A call that must succeed.
    async fn ok(&self, tool: &'static str, arguments: Value) -> Fallible<Value> {
        let (value, failed) = self.call(tool, arguments.clone()).await?;
        if failed {
            return Err(format!("{tool} {arguments} failed: {value}").into());
        }
        Ok(value)
    }

    /// A call that must fail: its diagnostic code.
    async fn refused(&self, tool: &'static str, arguments: Value) -> Fallible<String> {
        let (value, failed) = self.call(tool, arguments.clone()).await?;
        if !failed {
            return Err(format!("{tool} {arguments} ran: {value}").into());
        }
        Ok(value["code"].as_str().unwrap_or_default().to_owned())
    }

    async fn close(self) -> Fallible<()> {
        self.client.cancel().await?;
        self.serving.await??;
        Ok(())
    }
}

fn revision(reply: &Value) -> Fallible<u64> {
    reply["revision"]
        .as_u64()
        .ok_or_else(|| format!("no revision in {reply}").into())
}

fn board_size(size: [f64; 2]) -> Value {
    json!({"op": "layout.set_board_size", "params": {"size": size}})
}

/// The list is the six verbs in order, each with the annotations of its
/// class, `apply`'s `op` a plain string, within the design's token budget.
#[test]
fn the_tools_are_the_six_verbs() -> Fallible<()> {
    let dir = tempfile::tempdir()?;
    block_on(async move {
        let connection = Connection::open(Mode::AutoAllowEdits, dir.path()).await?;
        let listed = connection.client.list_tools(None).await?.tools;
        let names: Vec<&str> = listed.iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(names, [FIND, DESCRIBE, QUERY, APPLY, ACT, SHOW]);
        let hints = |name: &str| {
            listed
                .iter()
                .find(|t| t.name == name)
                .and_then(|t| t.annotations.clone())
                .map(|a| {
                    (
                        a.read_only_hint,
                        a.destructive_hint,
                        a.idempotent_hint,
                        a.open_world_hint,
                    )
                })
        };
        // An idempotentHint of false is the protocol's default, left out.
        let read = Some((Some(true), None, Some(true), Some(false)));
        assert_eq!(hints(FIND), read);
        assert_eq!(hints(DESCRIBE), read);
        assert_eq!(hints(QUERY), read);
        assert_eq!(
            hints(APPLY),
            Some((Some(false), Some(false), None, Some(false)))
        );
        assert_eq!(
            hints(ACT),
            Some((Some(false), Some(true), None, Some(true)))
        );
        // Steering the window changes no document and takes no undo step.
        assert_eq!(
            hints(SHOW),
            Some((Some(false), Some(false), None, Some(false)))
        );
        for tool in &listed {
            assert!(tool.title.is_some(), "{} has a title", tool.name);
        }

        // `op` is a plain string; the server resolves it (the founder's
        // answer to Q3), so the list carries no operation names.
        let apply = listed.iter().find(|t| t.name == APPLY).ok_or("apply")?;
        assert_eq!(
            apply.input_schema["properties"]["operations"]["items"]["properties"]["op"],
            json!({"type": "string",
                "description": "operation name; find it with `find`, get its schema with `describe`"})
        );
        let wire = serde_json::to_string(&listed)?;
        assert!(
            !wire.contains("layout.add_via"),
            "no operation names listed"
        );

        // The extension design's budget for six verbs: the list is the
        // verbs and nothing else, whatever the catalog grows to.
        let bytes = wire.len();
        assert!(bytes <= TOOLS_BUDGET, "the six tools are {bytes} bytes");
        connection.close().await
    })?
}

/// A misspelt `op` is refused before anything runs, naming the nearest
/// operations, and the document is unchanged; the right name still runs.
#[test]
fn a_misspelt_op_is_refused_with_the_nearest_names() -> Fallible<()> {
    let dir = tempfile::tempdir()?;
    block_on(async move {
        let c = Connection::open(Mode::AutoAllowEdits, dir.path()).await?;
        let at = json!({"name": "project.inspect", "params": {"at": "/project/board_size"}});
        let before = c.ok(QUERY, at.clone()).await?;
        let start = revision(&before)?;

        let (refused, failed) = c
            .call(
                APPLY,
                json!({"expected_revision": start, "operations": [
                    board_size([40.0, 30.0]),
                    {"op": "layout.set_bord_size", "params": {"size": [50.0, 30.0]}}]}),
            )
            .await?;
        assert!(failed, "{refused}");
        assert_eq!(refused["code"], "catalog.unknown", "{refused}");
        assert_eq!(refused["closest"][0], "layout.set_board_size", "{refused}");
        let closest = refused["closest"].as_array().ok_or("closest names")?;
        assert!((1..=3).contains(&closest.len()), "{refused}");
        assert!(
            refused["message"]
                .as_str()
                .is_some_and(|m| m.contains("did you mean layout.set_board_size")),
            "{refused}"
        );
        let after = c.ok(QUERY, at.clone()).await?;
        assert_eq!(
            (revision(&after)?, after["value"].clone()),
            (start, before["value"].clone()),
            "nothing ran"
        );

        // Only the leaf of the name.
        let (refused, _) = c
            .call(
                APPLY,
                json!({"expected_revision": start,
                "operations": [{"op": "set_board_size", "params": {"size": [50.0, 30.0]}}]}),
            )
            .await?;
        assert_eq!(refused["closest"][0], "layout.set_board_size", "{refused}");

        // Nothing near: no guesses.
        let (refused, failed) = c
            .call(
                APPLY,
                json!({"expected_revision": start, "operations": [{"op": "zzzzzzzzzzzzzzzz"}]}),
            )
            .await?;
        assert!(failed && refused["code"] == "catalog.unknown", "{refused}");
        assert!(refused.get("closest").is_none(), "{refused}");

        let applied = c
            .ok(
                APPLY,
                json!({"expected_revision": start, "operations": [board_size([50.0, 30.0])]}),
            )
            .await?;
        assert_eq!(revision(&applied)?, start + 1);
        let now = c.ok(QUERY, at).await?;
        assert_eq!(now["value"], json!([50.0, 30.0]));
        c.close().await
    })?
}

/// `find` names entries by what they do, of the kind asked, within its
/// bounds; `describe` gives one's schema and the call that sends it.
#[test]
fn find_and_describe_reach_the_catalog() -> Fallible<()> {
    let dir = tempfile::tempdir()?;
    block_on(async move {
        let c = Connection::open(Mode::AutoAllowEdits, dir.path()).await?;
        let found = c.ok(FIND, json!({"text": "add a via"})).await?;
        let hits = found["hits"].as_array().ok_or("hits")?;
        let names: Vec<&str> = hits.iter().filter_map(|h| h["name"].as_str()).collect();
        assert!(
            names.iter().take(3).any(|n| *n == "layout.add_via"),
            "{names:?}"
        );
        let via = hits
            .iter()
            .find(|h| h["name"] == "layout.add_via")
            .ok_or("via")?;
        assert_eq!(via["kind"], "operation");
        assert_eq!(via["class"], "edit");
        assert!(via["summary"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(hits.len() <= tools::FIND_LIMIT);

        let bom = c
            .ok(
                FIND,
                json!({"text": "bill of materials", "kind": "query", "limit": 3}),
            )
            .await?;
        let bom = bom["hits"].as_array().ok_or("hits")?;
        assert!(bom.len() <= 3);
        assert!(bom.iter().all(|h| h["kind"] == "query"));
        assert!(bom.iter().any(|h| h["name"] == "project.bom"), "{bom:?}");

        let exact = c.ok(FIND, json!({"text": "project.save"})).await?;
        assert_eq!(exact["hits"][0]["name"], "project.save");
        assert_eq!(exact["hits"][0]["kind"], "action");
        // Never is not in the catalog a model reaches.
        let never = c
            .ok(FIND, json!({"text": "library.favourite", "limit": 50}))
            .await?;
        let never = never["hits"].as_array().ok_or("hits")?;
        assert!(never.iter().all(|h| h["name"] != "library.favourite"));
        let long = "x".repeat(tools::MAX_TEXT + 1);
        assert_eq!(
            c.refused(FIND, json!({"text": long})).await?,
            "arguments.invalid"
        );
        assert_eq!(
            c.refused(FIND, json!({"text": "via", "limit": 0})).await?,
            "arguments.invalid"
        );
        assert_eq!(
            c.refused(FIND, json!({"text": "via", "kind": "tool"}))
                .await?,
            "arguments.invalid"
        );

        let described = c.ok(DESCRIBE, json!({"name": "layout.add_via"})).await?;
        assert_eq!(described["name"], "layout.add_via");
        assert_eq!(described["kind"], "operation");
        assert_eq!(described["call"]["tool"], APPLY);
        assert_eq!(
            described["call"]["arguments"]["operations"][0]["op"],
            "layout.add_via"
        );
        assert!(described["schema"]["properties"].is_object(), "{described}");
        assert!(
            described.get("send").is_none(),
            "today's call shape is gone"
        );
        let query = c.ok(DESCRIBE, json!({"name": "project.bom"})).await?;
        assert_eq!(query["call"]["tool"], QUERY);
        let reviewed = c
            .ok(DESCRIBE, json!({"name": "library.pinout_apply"}))
            .await?;
        assert_eq!(reviewed["person"], "review");
        let save = c.ok(DESCRIBE, json!({"name": "project.save"})).await?;
        assert_eq!(save["call"]["tool"], ACT);
        assert_eq!(save["person"], "approval");

        // A definition the reply names apart is read with def.
        let pinout = c
            .ok(DESCRIBE, json!({"name": "library.pinout_apply"}))
            .await?;
        let def = pinout["$defs"]
            .as_object()
            .and_then(|defs| defs.keys().next().cloned())
            .or_else(|| pinout["more_defs"][0].as_str().map(str::to_owned))
            .ok_or("a definition")?;
        let one = c
            .ok(
                DESCRIBE,
                json!({"name": "library.pinout_apply", "def": def}),
            )
            .await?;
        assert_eq!(one["def"], def.as_str());
        assert!(one["schema"].is_object());

        assert_eq!(
            c.refused(DESCRIBE, json!({"name": "library.favourite"}))
                .await?,
            "catalog.unknown"
        );
        assert_eq!(
            c.refused(DESCRIBE, json!({"name": "project_inspect"}))
                .await?,
            "catalog.unknown"
        );
        assert_eq!(
            c.refused(DESCRIBE, json!({"name": "guide:pinout"})).await?,
            "guide.unknown"
        );
        assert_eq!(
            c.refused(DESCRIBE, json!({"nam": "x"})).await?,
            "arguments.invalid"
        );
        c.close().await
    })?
}

/// `query` reads, `apply` edits atomically at a revision, and `act` asks;
/// each reaches only its own kind, through the gate.
#[test]
fn query_apply_and_act_run_through_the_gate() -> Fallible<()> {
    let dir = tempfile::tempdir()?;
    let saved = dir.path().join("p2.sparksmith");
    let path = saved.display().to_string();
    block_on(async move {
        let c = Connection::open(Mode::AutoAllowEdits, dir.path()).await?;
        let inspected = c.ok(QUERY, json!({"name": "project.inspect"})).await?;
        let start = revision(&inspected)?;
        let project = inspected["project"]
            .as_str()
            .ok_or("a project handle")?
            .to_owned();
        assert!(project.starts_with(desk::HANDLE_PREFIX), "{project}");
        let issues = c
            .ok(
                QUERY,
                json!({"project": project, "name": "project.issues",
            "params": {"scope": {"kind": "all"}}}),
            )
            .await?;
        assert_eq!(issues["project"], project.as_str(), "{issues}");

        // One step.
        let applied = c
            .ok(
                APPLY,
                json!({"project": project, "expected_revision": start,
            "operations": [board_size([40.0, 30.0])]}),
            )
            .await?;
        assert_eq!(revision(&applied)?, start + 1);
        assert_eq!(applied["outcomes"].as_array().map(Vec::len), Some(1));
        assert_eq!(applied["project"], project.as_str());

        // Stale: refused, and nothing moved.
        assert_eq!(
            c.refused(
                APPLY,
                json!({"expected_revision": start, "operations": [board_size([50.0, 30.0])]})
            )
            .await?,
            "stale_revision"
        );
        let now = revision(&c.ok(QUERY, json!({"name": "project.inspect"})).await?)?;
        assert_eq!(now, start + 1);

        // A batch is one revision and one undo step.
        let batch = c
            .ok(
                APPLY,
                json!({"expected_revision": now,
            "operations": [board_size([60.0, 30.0]), board_size([70.0, 35.0])]}),
            )
            .await?;
        assert_eq!(revision(&batch)?, now + 1);
        assert_eq!(batch["outcomes"].as_array().map(Vec::len), Some(2));
        let undone = c
            .ok(
                APPLY,
                json!({"expected_revision": now + 1,
            "operations": [{"op": "project.undo"}]}),
            )
            .await?;
        assert_eq!(revision(&undone)?, now + 2);
        let board = c
            .ok(
                QUERY,
                json!({"name": "project.inspect",
            "params": {"at": "/project/board_size"}}),
            )
            .await?;
        assert_eq!(
            board["value"],
            json!([40.0, 30.0]),
            "one undo took the whole batch"
        );
        let now = now + 2;

        // A failing step anywhere changes nothing, and is named.
        let (failed, error) = c
            .call(
                APPLY,
                json!({"expected_revision": now,
            "operations": [board_size([80.0, 30.0]), board_size([-1.0, 30.0])]}),
            )
            .await?;
        assert!(error, "{failed}");
        assert!(
            failed["message"]
                .as_str()
                .is_some_and(|m| m.contains("operation 2 of 2")),
            "{failed}"
        );
        let same = c
            .ok(
                QUERY,
                json!({"name": "project.inspect", "params": {"at": "/project/board_size"}}),
            )
            .await?;
        assert_eq!(
            (revision(&same)?, same["value"].clone()),
            (now, json!([40.0, 30.0]))
        );

        // A dry run answers the change and keeps nothing.
        let tried = c
            .ok(
                APPLY,
                json!({"expected_revision": now, "dry_run": true,
            "operations": [board_size([90.0, 30.0])]}),
            )
            .await?;
        assert_eq!(tried["dry_run"], true);
        assert_eq!(revision(&tried)?, now);
        assert!(tried["changes"].is_object(), "{tried}");
        let same = c
            .ok(
                QUERY,
                json!({"name": "project.inspect", "params": {"at": "/project/board_size"}}),
            )
            .await?;
        assert_eq!(
            (revision(&same)?, same["value"].clone()),
            (now, json!([40.0, 30.0]))
        );

        // Commands go alone; names reach only their own verb.
        assert_eq!(
            c.refused(
                APPLY,
                json!({"expected_revision": now,
                "operations": [board_size([1.0, 1.0]), {"op": "project.undo"}]})
            )
            .await?,
            "apply.alone"
        );
        assert_eq!(
            c.refused(QUERY, json!({"name": "layout.add_via"})).await?,
            "catalog.other_kind"
        );
        assert_eq!(
            c.refused(
                APPLY,
                json!({"expected_revision": now, "operations": [{"op": "project.save"}]})
            )
            .await?,
            "catalog.other_kind"
        );
        assert_eq!(
            c.refused(ACT, json!({"name": "project.bom"})).await?,
            "catalog.other_kind"
        );
        assert_eq!(
            c.refused(
                ACT,
                json!({"name": "library.favourite", "params": {"favourite": true}})
            )
            .await?,
            "catalog.unknown"
        );
        assert_eq!(
            c.refused(APPLY, json!({"expected_revision": now, "operations": []}))
                .await?,
            "arguments.invalid"
        );
        assert_eq!(
            c.refused(APPLY, json!({"operations": [board_size([1.0, 1.0])]}))
                .await?,
            "arguments.invalid"
        );

        // An action needs a person, and this server cannot ask yet.
        assert_eq!(
            c.refused(
                ACT,
                json!({"name": "project.save", "params": {"path": path}})
            )
            .await?,
            "gate.needs_person"
        );
        assert!(!saved.exists());
        assert_eq!(
            c.refused(ACT, json!({"name": "project.new"})).await?,
            "gate.needs_person"
        );

        // An unknown tool is the protocol's error.
        let unknown = c
            .client
            .call_tool(CallToolRequestParams::new("project_apply"))
            .await;
        assert!(
            matches!(unknown, Err(ServiceError::McpError(e)) if e.code == ErrorCode::INVALID_PARAMS)
        );
        c.close().await
    })?
}

/// `project.new` and `project.open` open further projects behind handles
/// of their own; a call names one, or the only one open.
#[test]
fn projects_are_kept_apart_by_handle() -> Fallible<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("second.sparksmith");
    let path = file.display().to_string();
    let outside = std::env::temp_dir()
        .join("sparksmith-p2-outside.sparksmith")
        .display()
        .to_string();
    let new = json!({"name": "project.new", "params": {
        "profile": sparksmith_core::rules::fixtures::prototype()?}});
    block_on(async move {
        let c = Connection::open(Mode::AllowAll, dir.path()).await?;
        let first = c.ok(QUERY, json!({"name": "project.inspect"})).await?;
        let one = first["project"].as_str().ok_or("handle")?.to_owned();
        let one_rev = revision(&first)?;

        let opened = c.ok(ACT, new.clone()).await?;
        let two = opened["project"].as_str().ok_or("handle")?.to_owned();
        assert_ne!(one, two);
        // A call that names no project goes to the current one: the one
        // last opened, then the one a call last named.
        let current = c.ok(QUERY, json!({"name": "project.inspect"})).await?;
        assert_eq!(current["project"], two.as_str());
        c.ok(QUERY, json!({"project": one, "name": "project.inspect"}))
            .await?;
        let current = c.ok(QUERY, json!({"name": "project.inspect"})).await?;
        assert_eq!(current["project"], one.as_str());
        assert_eq!(
            c.refused(
                QUERY,
                json!({"project": "prj_gone", "name": "project.inspect"})
            )
            .await?,
            "project.unknown"
        );
        assert_eq!(
            c.refused(ACT, json!({"project": one, "name": "project.new"}))
                .await?,
            "project.opens_new"
        );

        // An edit on one project leaves the other as it was.
        let two_rev = revision(
            &c.ok(QUERY, json!({"project": two, "name": "project.inspect"}))
                .await?,
        )?;
        let edited = c
            .ok(
                APPLY,
                json!({"project": two, "expected_revision": two_rev,
            "operations": [board_size([25.0, 15.0])]}),
            )
            .await?;
        assert_eq!(edited["project"], two.as_str());
        let untouched = c
            .ok(QUERY, json!({"project": one, "name": "project.inspect"}))
            .await?;
        assert_eq!(revision(&untouched)?, one_rev);

        // Saved and opened again: a third project with the second's board.
        c.ok(
            ACT,
            json!({"project": two, "name": "project.save", "params": {"path": path}}),
        )
        .await?;
        assert!(file.exists());
        let three = c
            .ok(
                ACT,
                json!({"name": "project.open", "params": {"path": path}}),
            )
            .await?;
        let three = three["project"].as_str().ok_or("handle")?.to_owned();
        assert!(three != one && three != two);
        let board = c
            .ok(
                QUERY,
                json!({"project": three, "name": "project.inspect",
            "params": {"at": "/project/board_size"}}),
            )
            .await?;
        assert_eq!(board["value"], json!([25.0, 15.0]));

        // The gate still holds the folders: a failed open opens nothing.
        assert_eq!(
            c.refused(
                ACT,
                json!({"name": "project.open", "params": {"path": outside}})
            )
            .await?,
            "gate.path_outside"
        );
        // As many projects as the server holds, and no more.
        for _ in 3..desk::MAX_PROJECTS {
            c.ok(ACT, new.clone()).await?;
        }
        assert_eq!(c.refused(ACT, new.clone()).await?, "project.limit");
        c.close().await
    })?
}

/// The most bytes the default `library.generators` reply may take for the
/// shipped library: a model reads it whole, below a client's tool-output
/// limit.
const GENERATORS_BUDGET: usize = 16 * 1024;

/// The generator entries, which the old server hid to fit its list, are
/// found and called like any other: the shipped chip module lists and
/// makes a part through `query` and `apply`.
#[test]
fn generators_are_found_and_called() -> Fallible<()> {
    let mut session = Session::default();
    session.set_shipped(sparksmith_io::shipped::ShippedLibrary::at(Path::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"),
    ))?);
    let tools = super::testing::Tools::new(session, Grant::default())?;
    let found = tools.ok(FIND, json!({"text": "generator", "limit": 50}))?;
    let hits: Vec<&str> = found["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h["name"].as_str())
        .collect();
    for name in [
        "library.generators",
        "library.generate",
        "library.generate_part",
    ] {
        assert!(hits.contains(&name), "find reaches {name}: {found}");
    }
    let described = tools.ok(DESCRIBE, json!({"name": "library.generate_part"}))?;
    assert_eq!(described["call"]["tool"], APPLY, "{described}");

    let listed = tools.ok(QUERY, json!({"name": "library.generators"}))?;
    let shipped = listed["shipped"].as_array().ok_or("shipped")?;
    assert!(shipped.iter().any(|g| g["name"] == "chip"), "{listed}");
    // The default listing names each generator, its families and preset
    // keys, within a reply budget; preset names come with a filter, the
    // parameters by name.
    let compact = listed.to_string().len();
    assert!(compact <= GENERATORS_BUDGET, "{compact} bytes");
    assert!(
        shipped.iter().all(|g| g.get("description").is_none()),
        "{listed}"
    );
    let chip = shipped.iter().find(|g| g["name"] == "chip").ok_or("chip")?;
    assert!(
        chip["presets"]
            .as_array()
            .is_some_and(|p| p.iter().any(|p| p["key"] == "resistor-0603")),
        "{chip}"
    );
    // Filtered, the presets come with their names.
    let found = tools.ok(
        QUERY,
        json!({"name": "library.generators", "params": {"text": "resistor-0603"}}),
    )?;
    assert!(
        found["shipped"][0]["presets"].as_array().is_some_and(|p| p
            .iter()
            .any(|p| p["key"] == "resistor-0603" && p["name"].is_string())),
        "{found}"
    );
    let full = tools.ok(
        QUERY,
        json!({"name": "library.generators", "params": {"detail": true}}),
    )?;
    assert!(full.to_string().len() > compact);
    for (filter, expected) in [
        (json!({"generator": "chip"}), vec!["chip"]),
        (json!({"text": "resistor-0603"}), vec!["chip"]),
        (json!({"family": "no_lead"}), vec!["no-lead"]),
    ] {
        let got = tools.ok(
            QUERY,
            json!({"name": "library.generators", "params": filter}),
        )?;
        let names: Vec<&str> = got["shipped"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g["name"].as_str())
            .collect();
        assert_eq!(names, expected, "{filter}");
    }
    // One generator: its parameters in full, its presets by key, name and
    // basis within the budget; one preset or `detail` gives the values.
    for name in ["no-lead", "gull-wing", "chip"] {
        let one = tools.ok(
            QUERY,
            json!({"name": "library.generators", "params": {"generator": name}}),
        )?;
        let size = one.to_string().len();
        assert!(size <= GENERATORS_BUDGET, "{name}: {size} bytes");
        let description = &one["shipped"][0]["description"];
        assert!(description["params"].is_array(), "{one}");
        let presets = description["presets"].as_array().ok_or("presets")?;
        assert!(
            presets
                .iter()
                .all(|p| p["key"].is_string() && p.get("params").is_none()),
            "{name}: presets by key: {one}"
        );
    }
    let preset = tools.ok(
        QUERY,
        json!({"name": "library.generators", "params": {"preset": "resistor-0603"}}),
    )?;
    let presets = preset["shipped"][0]["description"]["presets"]
        .as_array()
        .ok_or("presets")?;
    let full: Vec<&Value> = presets.iter().filter(|p| p.get("params").is_some()).collect();
    assert_eq!(full.len(), 1, "{preset}");
    assert_eq!(full[0]["key"], "resistor-0603");
    let all = tools.ok(
        QUERY,
        json!({"name": "library.generators", "params": {"generator": "chip", "detail": true}}),
    )?;
    assert!(
        all["shipped"][0]["description"]["presets"]
            .as_array()
            .is_some_and(|p| p.iter().all(|p| p.get("params").is_some())),
        "{all}"
    );
    let (failed, missing) = tools.query("library.generators", json!({"preset": "no-such-preset"}));
    assert!(failed, "{missing}");
    let (failed, unknown) = tools.query("library.generators", json!({"generator": "gullwing"}));
    assert!(failed, "{unknown}");
    assert!(
        unknown.to_string().contains("gull-wing"),
        "the refusal names the generators: {unknown}"
    );
    // A generator is described by its name, as a catalog entry is.
    let chip = tools.ok(DESCRIBE, json!({"name": "chip"}))?;
    assert!(
        chip["generator"]["description"]["params"].is_array(),
        "{chip}"
    );
    let (failed, nothing) = tools.call(DESCRIBE, json!({"name": "no-such-thing"}));
    assert!(failed && nothing["code"] == "catalog.unknown", "{nothing}");
    let before = tools.revision();
    let made = tools.ok(
        APPLY,
        json!({"expected_revision": before, "operations": [{"op": "library.generate_part",
            "params": {"generator": "chip", "scope": "component", "preset": "resistor-0603"}}]}),
    )?;
    assert_eq!(made["outcome"]["report"]["created"], true, "{made}");
    assert_eq!(tools.revision(), before + 1);
    Ok(())
}

/// Headless, there is no window to steer: `show` says so and changes
/// nothing.
#[test]
fn show_needs_the_window() -> Fallible<()> {
    let tools = super::testing::Tools::new(Session::default(), Grant::default())?;
    let before = tools.revision();
    let (failed, shown) = tools.call(SHOW, json!({"name": "view.select", "params": {}}));
    assert!(failed);
    assert_eq!(shown["code"], "host.unavailable", "{shown}");
    let (failed, context) = tools.call(QUERY, json!({"name": "view.context"}));
    assert!(failed);
    assert_eq!(context["code"], "host.unavailable", "{context}");
    let (failed, other) = tools.call(SHOW, json!({"name": "project.undo"}));
    assert!(failed);
    assert_eq!(other["code"], "catalog.other_kind", "{other}");
    assert_eq!(tools.revision(), before);
    Ok(())
}

/// `describe` serves exactly what the verb's `params` take: no operation or
/// query schema wraps it in its tag (`{command, params}`), and every
/// example a description gives (its `call` included) is accepted by the
/// parser `apply` and `query` run, field check and typed read.
#[test]
fn no_describe_answers_with_a_bare_top_level_ref() -> Fallible<()> {
    let tools = super::testing::Tools::new(Session::default(), Grant::default())?;
    let mut inlined = 0;
    for entry in super::catalog::reachable() {
        let name = entry.name.as_str();
        let described = tools.ok(DESCRIBE, json!({"name": name}))?;
        let schema = &described["schema"];
        assert!(
            schema.get("$ref").is_none(),
            "{name}: describe's schema is a bare $ref: {schema}"
        );
        if name == "library.replace_body" {
            inlined += 1;
            assert!(
                schema["properties"].is_object(),
                "{name}: the parameters' properties are in the schema: {schema}"
            );
            assert!(
                schema["description"]
                    .as_str()
                    .is_some_and(|d| d.starts_with("Give a placed part")),
                "{name}: the entry's own description stays: {schema}"
            );
        }
    }
    assert_eq!(inlined, 1, "library.replace_body is reachable");
    Ok(())
}

#[test]
fn every_describe_example_is_what_apply_and_query_accept() -> Fallible<()> {
    let tools = super::testing::Tools::new(Session::default(), Grant::default())?;
    let mut examples = 0;
    for entry in super::catalog::reachable() {
        let sent = match entry.kind {
            Some(crate::session::metadata::Kind::Operation) => super::fields::Sent::Apply,
            Some(crate::session::metadata::Kind::Query) => super::fields::Sent::Query,
            _ => continue,
        };
        let name = entry.name.as_str();
        let described = tools.ok(DESCRIBE, json!({"name": name}))?;
        let properties = &described["schema"]["properties"];
        for tag in ["command", "query", "editor"] {
            assert!(
                properties[tag]["const"].as_str().is_none(),
                "{name}: describe wraps the params in `{tag}`: {described}"
            );
        }
        let given = described["examples"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let sends = if sent == super::fields::Sent::Apply {
            &described["call"]["arguments"]["operations"][0]["params"]
        } else {
            &described["call"]["arguments"]["params"]
        };
        if let Some(first) = given.first() {
            assert_eq!(sends, first, "{name}: the call sends the first example");
        }
        for example in given {
            examples += 1;
            let mut params = example.clone();
            super::lenient::read_params(name, &mut params)
                .map_err(|why| format!("{name} {example}: {why}"))?;
            super::fields::check(name, &params, sent)
                .map_err(|why| format!("{name} {example}: {why}"))?;
            let call = crate::front::Call {
                entry: name.to_owned(),
                params,
                project: None,
                expected_revision: Some(0),
                dry_run: false,
                confirm: None,
                turn: None,
                job: false,
            };
            crate::front::read(&call).map_err(|why| format!("{name} {example}: {why:?}"))?;
        }
    }
    assert!(
        examples >= 6,
        "the rules and fill examples are found: {examples}"
    );
    Ok(())
}

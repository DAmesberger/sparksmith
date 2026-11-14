//! The live channel's decisions over a plain session, and its socket end to
//! end: the token, the window's turn and the MCP verbs through the bridge.
use super::*;
use crate::gate::Mode;
use sparksmith_core::Engine;

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// No token or discovery files: tests never touch the user's folders.
const TEST_APP: App = App {
    name: sparksmith_core::product::NAME,
    data_dir: || None,
    env: |_| None,
    utc: seconds,
    default_port: DEFAULT_PORT,
};

fn policy() -> Grant {
    grant(Mode::AutoAllowEdits, [std::env::temp_dir()])
}

/// A call of `entry` with `params`, at `revision` when it names one.
fn envelope(entry: &str, params: Value, revision: Option<u64>) -> Value {
    let mut call = json!({"entry": entry, "params": params});
    if let Some(revision) = revision {
        call["expected_revision"] = json!(revision);
    }
    call
}

/// `layout.add_via` at `revision`.
fn add_via(session: &Session, revision: u64) -> Value {
    let net = session.engine().project().nets.first().map_or(0, |n| n.id);
    envelope(
        "layout.add_via",
        json!({"via": {"net": net, "position": [1.0, 1.0], "drill": 0.3, "diameter": 0.6, "span": "through", "tented": true}}),
        Some(revision),
    )
}

fn code(reply: &Value) -> &str {
    reply["error"]["code"].as_str().unwrap_or_default()
}

fn decide(session: &Session, busy: Option<&str>, params: &Value) -> Verdict {
    admit(session, &policy(), busy, CALL, params)
}

/// A change at the current revision runs and answers compactly; at an older
/// one, without one, or while the person holds the window it is refused and
/// nothing changes. Reads and dry runs run while the person is busy.
#[test]
fn changes_need_the_current_revision_and_a_free_window() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let before = session.engine().document().clone();

    let stale = decide(
        &session,
        None,
        &add_via(&session, revision.saturating_sub(1)),
    );
    let Verdict::Reply(stale) = stale else {
        return Err("a stale change was admitted".into());
    };
    assert_eq!(code(&stale), "stale_revision", "{stale}");

    let undo = envelope("project.undo", json!({}), None);
    let Verdict::Reply(missing) = decide(&session, None, &undo) else {
        return Err("an undo without a revision was admitted".into());
    };
    assert_eq!(code(&missing), "malformed_request", "{missing}");
    assert!(
        missing["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("expected_revision")),
        "{missing}"
    );

    let Verdict::Reply(busy) = decide(
        &session,
        Some("The person is typing"),
        &add_via(&session, revision),
    ) else {
        return Err("a change was admitted while the person types".into());
    };
    assert_eq!(code(&busy), "live.busy", "{busy}");
    assert!(
        busy["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("typing"))
    );

    let read = envelope("project.inspect", json!({}), None);
    let Verdict::Run(admitted) = decide(&session, Some("The person is typing"), &read) else {
        return Err("a read was refused while the person types".into());
    };
    assert_eq!(run(&mut session, &admitted)["ok"], true);

    // A dry run keeps nothing, so it runs while the person types too.
    let mut tried = add_via(&session, revision);
    tried["dry_run"] = json!(true);
    let Verdict::Run(admitted) = decide(&session, Some("The person is typing"), &tried) else {
        return Err("a dry run was refused while the person types".into());
    };
    assert!(admitted.dry_run());
    let reply = run(&mut session, &admitted);
    assert_eq!(reply["result"]["dry_run"], true, "{reply}");
    assert_eq!(session.engine().document(), &before);
    assert_eq!(session.engine().revision(), revision);

    let Verdict::Run(admitted) = decide(&session, None, &add_via(&session, revision)) else {
        return Err("a current change was not admitted".into());
    };
    assert_eq!(admitted.subject(), "layout.add_via");
    let reply = run(&mut session, &admitted);
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(
        reply["result"]["changes"]["added"]["vias"].is_array(),
        "{reply}"
    );
    // The review measured 5.4 MB per edit for a whole-project reply.
    assert!(
        reply.to_string().len() < 4 * 1024,
        "{} bytes",
        reply.to_string().len()
    );
    assert_eq!(session.engine().revision(), revision + 1);

    let undo = envelope("project.undo", json!({}), Some(revision + 1));
    let Verdict::Run(admitted) = decide(&session, None, &undo) else {
        return Err("a current undo was not admitted".into());
    };
    assert_eq!(run(&mut session, &admitted)["ok"], true);
    assert_eq!(session.engine().document(), &before);
    Ok(())
}

/// Opening, saving and starting a project are the window's own: the files
/// module is outside the window's grant. An Ask call is held for the
/// person; a Never one is refused; an unknown method says what exists.
#[test]
fn the_window_decides_files_and_asks() -> Fallible {
    let session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let file = std::env::temp_dir().join("live.sparksmith");
    for (entry, params) in [
        ("project.new", json!({})),
        ("project.open", json!({"path": file.display().to_string()})),
        ("project.save", json!({})),
    ] {
        let call = envelope(entry, params, Some(revision));
        let Verdict::Reply(reply) = decide(&session, None, &call) else {
            return Err(format!("{entry} was admitted").into());
        };
        assert_eq!(code(&reply), "gate.module", "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("files module")),
            "{reply}"
        );
    }
    let path = std::env::temp_dir().join("live-export.solidcraft");
    let export = envelope(
        "project.export_native",
        json!({"path": path.display().to_string()}),
        None,
    );
    let Verdict::Ask(admitted, question) = decide(&session, None, &export) else {
        return Err("an export was not held for the person".into());
    };
    assert_eq!(admitted.class(), Class::Ask);
    assert_eq!(question.command, "project.export_native");

    let never = envelope(
        "library.favourite",
        json!({"key": {"source": "project", "lineage": 1}, "favourite": true}),
        None,
    );
    let Verdict::Reply(never) = decide(&session, None, &never) else {
        return Err("a never-class request was admitted".into());
    };
    assert_eq!(code(&never), "gate.never", "{never}");
    let Verdict::Reply(unknown) = admit(&session, &policy(), None, "ui.click", &json!({})) else {
        return Err("an unknown method was admitted".into());
    };
    assert_eq!(code(&unknown), "live.unknown_method");
    let Verdict::Reply(malformed) = decide(&session, None, &json!({"command": "project.inspect"}))
    else {
        return Err("a request that is no call was admitted".into());
    };
    assert_eq!(code(&malformed), "malformed_request", "{malformed}");
    let Verdict::Reply(now) = admit(&session, &policy(), Some("busy"), REVISION, &json!({})) else {
        return Err("the revision was not answered".into());
    };
    assert_eq!(now["result"]["revision"], revision);
    Ok(())
}

/// A held change runs only if the project has not moved while it waited.
#[test]
fn an_allowed_change_is_checked_again() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let Verdict::Run(admitted) = decide(&session, None, &add_via(&session, revision)) else {
        return Err("not admitted".into());
    };
    // The person edits meanwhile.
    let net = session.engine().project().nets.first().map_or(0, |n| n.id);
    let mine = json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Layout", "operation": {"command": "layout.add_via", "params": {
            "via": {"net": net, "position": [2.0, 2.0], "drill": 0.3, "diameter": 0.6, "span": "through", "tented": true}
        }}},
    }});
    assert_eq!(session.dispatch(mine)["ok"], true);
    let reply = run(&mut session, &admitted);
    assert_eq!(code(&reply), "stale_revision", "{reply}");
    assert_eq!(session.engine().revision(), revision + 1);
    Ok(())
}

/// Over the socket: a wrong token is refused before anything reaches the
/// window; with the token, the window's answer comes back, and the bridge's
/// lost connection says so.
#[test]
fn the_socket_needs_the_token_and_waits_for_the_window() -> Fallible {
    let endpoint = Endpoint::start(TEST_APP, Listen::Exact(0), || {})?;
    let port = endpoint.port();
    let token = |text: &str| ClientToken {
        token: text.to_owned(),
        file: None,
        app: TEST_APP,
    };
    let mut wrong = Client::with_token(port, token(&"0".repeat(64)));
    let refused = wrong.call(REVISION, json!({}));
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.starts_with("unauthorized")),
        "{refused:?}"
    );
    assert!(endpoint.next().is_none());

    let session = Session::new(Engine::demo()?);
    let mut client = Client::with_token(port, token(endpoint.token()));
    let asked = std::thread::spawn(move || client.call(REVISION, json!({})));
    let request = loop {
        if let Some(request) = endpoint.next() {
            break request;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(request.method, REVISION);
    assert_eq!(endpoint.connections(), 1);
    let Verdict::Reply(reply) = admit(&session, &policy(), None, &request.method, &request.params)
    else {
        return Err("the revision was not answered".into());
    };
    request.reply.send(reply)?;
    let answered = asked.join().map_err(|_| "client thread")??;
    assert_eq!(answered["result"]["revision"], session.engine().revision());
    drop(endpoint);
    let mut late = Client::with_token(port, token("x"));
    assert!(late.call(REVISION, json!({})).is_err());
    Ok(())
}

/// A stand-in for the window: answers the endpoint's requests on `session`
/// as the desktop app does, the person allowing what asks, until `stop`.
fn window(
    endpoint: Endpoint,
    mut session: Session,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<Session> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let Some(request) = endpoint.next() else {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            };
            let reply = match admit(&session, &policy(), None, &request.method, &request.params) {
                Verdict::Reply(reply) => reply,
                Verdict::Run(admitted) | Verdict::Ask(admitted, _) => run(&mut session, &admitted),
            };
            let _ = request.reply.send(reply);
        }
        session
    })
}

/// `sparksmith mcp --live`: the verbs reach the window through the bridge.
/// A read and an edit run there, a dry run keeps nothing, what asks waits
/// for the person (who allows it here), the window's file is refused, and
/// `show` finds no entry yet.
#[test]
fn the_verbs_reach_the_window() -> Fallible {
    let endpoint = Endpoint::start(TEST_APP, Listen::Exact(0), || {})?;
    let client = Client::with_token(
        endpoint.port(),
        ClientToken {
            token: endpoint.token().to_owned(),
            file: None,
            app: TEST_APP,
        },
    );
    let session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let via = add_via(&session, revision)["params"].clone();
    let stop = Arc::new(AtomicBool::new(false));
    let serving = window(endpoint, session, Arc::clone(&stop));
    let tools = crate::mcp::testing::Tools::live(client)?;

    let inspect = tools.ok("query", json!({"name": "project.inspect"}))?;
    assert_eq!(inspect["revision"], revision, "{inspect}");
    let project = inspect["project"].clone();
    assert!(project.is_string(), "{inspect}");

    let tried = tools.ok(
        "apply",
        json!({"expected_revision": revision, "dry_run": true,
            "operations": [{"op": "layout.add_via", "params": via.clone()}]}),
    )?;
    assert_eq!(tried["dry_run"], true, "{tried}");
    assert_eq!(tried["revision"], revision, "{tried}");

    let applied = tools.ok(
        "apply",
        json!({"project": project, "expected_revision": revision,
            "operations": [{"op": "layout.add_via", "params": via.clone()}]}),
    )?;
    assert_eq!(applied["revision"], revision + 1, "{applied}");
    let (failed, stale) = tools.call(
        "apply",
        json!({"expected_revision": revision,
            "operations": [{"op": "layout.add_via", "params": via}]}),
    );
    assert!(failed, "{stale}");
    assert_eq!(stale["code"], "stale_revision", "{stale}");

    // The window's file is the person's: the files module is not granted.
    let (failed, save) = tools.call("act", json!({"name": "project.save"}));
    assert!(failed);
    assert_eq!(save["code"], "gate.module", "{save}");
    let (failed, opened) = tools.call("act", json!({"name": "project.new"}));
    assert!(failed);
    assert_eq!(opened["code"], "gate.module", "{opened}");

    // A read naming a path asks; the window's person allows it.
    let dir = tempfile::tempdir_in(std::env::temp_dir())?;
    let part = dir.path().join("smd-nut.solidcraft");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../library/parts/smd-nut.solidcraft"
        ),
        &part,
    )?;
    let checked = tools.ok(
        "act",
        json!({"name": "library.check_part_file", "params": {"path": part.display().to_string()}}),
    )?;
    assert_eq!(checked["report"]["ok"], true, "{checked}");

    let (failed, shown) = tools.call("show", json!({"name": "view.select"}));
    assert!(failed);
    assert_eq!(shown["code"], "catalog.unknown", "{shown}");

    stop.store(true, Ordering::Release);
    let session = serving.join().map_err(|_| "window thread")?;
    assert_eq!(session.engine().revision(), revision + 1);
    Ok(())
}

/// I-A1: live, `library.source.add` is refused in every mode the window
/// offers, before it is held for the person, and leaves the sources as they
/// were.
#[test]
fn the_window_never_adds_a_library_source() -> Fallible {
    let session = Session::new(Engine::demo()?);
    let call = envelope(
        "library.source.add",
        json!({"path": std::env::temp_dir().join("lib").display().to_string()}),
        None,
    );
    for mode in [Mode::AskEveryTime, Mode::AutoAllowEdits, Mode::AllowAll] {
        let policy = grant(mode, [std::env::temp_dir()]);
        let Verdict::Reply(reply) = admit(&session, &policy, None, CALL, &call) else {
            return Err(format!("{mode:?}: library.source.add was admitted").into());
        };
        assert_eq!(code(&reply), "gate.never", "{mode:?}: {reply}");
    }
    assert!(session.sources().list().is_empty());
    Ok(())
}

/// I-A2: a read that names a path takes no revision. It asks, and on the
/// person's Allow it runs: it is never refused for a revision it cannot
/// send.
#[test]
fn a_read_naming_a_path_runs_on_allow() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let dir = tempfile::tempdir_in(std::env::temp_dir())?;
    let part = dir.path().join("smd-nut.solidcraft");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../library/parts/smd-nut.solidcraft"
        ),
        &part,
    )?;
    let check = envelope(
        "library.check_part_file",
        json!({"path": part.display().to_string()}),
        None,
    );
    let Verdict::Ask(admitted, question) = decide(&session, None, &check) else {
        return Err("a path-named read was not held for the person".into());
    };
    assert_eq!(question.command, "library.check_part_file");
    let reply = run(&mut session, &admitted);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["report"]["ok"], true, "{reply}");

    let preview = envelope(
        "firmware.export_preview",
        json!({"instance": 1, "emitter": {"emitter": "json"},
            "template_file": dir.path().join("t.rhai").display().to_string()}),
        None,
    );
    let Verdict::Ask(admitted, _) = decide(&session, None, &preview) else {
        return Err("a preview naming a template was not held".into());
    };
    let reply = run(&mut session, &admitted);
    assert!(
        !code(&reply).starts_with("live.") && !code(&reply).starts_with("gate."),
        "{reply}"
    );
    Ok(())
}

/// I-A3: the wait comes from the effective class: an operation that asks,
/// a read that names a path and, under ask-every-time, an edit may all wait
/// for the person, so none times out while it is held; a plain read keeps
/// the short wait.
#[test]
fn a_request_that_may_wait_for_the_person_gets_the_long_wait() -> Fallible {
    let session = Session::new(Engine::demo()?);
    let publish = envelope("library.publish", json!({"drafts": [1]}), Some(0));
    let path_read = envelope(
        "library.check_part_file",
        json!({"path": "/tmp/x.solidcraft"}),
        None,
    );
    let edit = add_via(&session, 0);
    let read = envelope("project.inspect", json!({}), None);
    let summary = |params: &Value| LiveProtocol.summary(CALL, params);
    for (name, params) in [
        ("publish", &publish),
        ("path read", &path_read),
        ("edit", &edit),
    ] {
        assert!(summary(params).long, "{name}: {params}");
    }
    assert!(matches!(summary(&publish).effect, Effect::Changes));
    assert_eq!(summary(&publish).command, Some("library.publish"));
    assert!(!summary(&read).long);
    assert!(matches!(summary(&read).effect, Effect::ReadsOnly));
    assert!(!LiveProtocol.summary(REVISION, &json!({})).long);
    Ok(())
}

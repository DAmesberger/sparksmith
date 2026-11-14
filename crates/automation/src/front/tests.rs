//! The front door through its public calls: the one decision over every
//! catalog entry and every principal, module ceilings, the envelope's
//! revision, and calls that change nothing.
use super::*;
use crate::gate::{Mode, decide_entries};
use crate::session::metadata::{self, EntryMeta};
use sparksmith_core::{Class, Engine};
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// `path` made absolute on this platform: Windows needs a drive.
fn abs(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.to_owned()
    }
}

fn call(entry: &str, params: Value, expected_revision: Option<u64>) -> Call {
    Call {
        entry: entry.to_owned(),
        params,
        project: None,
        expected_revision,
        dry_run: false,
        confirm: None,
        turn: None,
        job: false,
    }
}

/// A failed call as a test failure.
fn failed(error: Box<Error>) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

/// Calls `call` as `who`, nobody to ask.
fn run(session: &mut Session, who: Principal<'_>, call: Call) -> Result<Value, Box<Error>> {
    session.call(
        CallCx {
            who,
            asker: &mut NoOne,
            verbose: false,
        },
        call,
    )
}

fn via_params(session: &Session) -> Value {
    let net = session.engine().project().nets.first().map_or(0, |n| n.id);
    json!({"via": {"net": net, "position": [1.0, 1.0], "drill": 0.3, "diameter": 0.6,
        "span": "through", "tented": true}})
}

/// `run`, `ask` or the refusal's code.
fn verdict(decision: &Decision) -> String {
    match decision {
        Decision::Run => "run".into(),
        Decision::Ask(_) => "ask".into(),
        Decision::Refuse(error) => error.code.clone(),
    }
}

/// The principals of the table, each with the grant it carries.
enum Case {
    Person,
    Operator,
    /// A remote agent over every module in a mode.
    Remote(Mode),
    /// The built-in agent's default grant.
    Local,
    /// A remote agent in allow-all whose grant lacks the entry's module.
    WithoutModule,
    /// A remote agent in allow-all whose grant reads the entry's module only.
    ReadOnlyModule,
}

/// What the table expects for an entry of `class` (raised to Ask when the
/// call names a path) in `module`, the path inside the folders or not.
fn expected(case: &Case, class: Class, module: &str, outside: bool) -> &'static str {
    let local_ceiling = match module {
        "files" | "fab" | "model3d" => Class::Ask,
        _ => Class::Edit,
    };
    match (case, class) {
        (Case::Person | Case::Operator, _) => "run",
        (_, Class::Never) => "gate.never",
        (Case::WithoutModule, _) => "gate.module",
        (Case::ReadOnlyModule, Class::Show | Class::Edit | Class::Ask) => "gate.module",
        (Case::Local, class) if class > local_ceiling => "gate.module",
        (_, _) if outside => "gate.path_outside",
        (Case::ReadOnlyModule, Class::Read) => "run",
        (Case::Remote(Mode::AllowAll), _) => "run",
        (Case::Remote(Mode::AutoAllowEdits) | Case::Local, Class::Ask) => "ask",
        (Case::Remote(Mode::AutoAllowEdits) | Case::Local, _) => "run",
        (Case::Remote(Mode::AskEveryTime), Class::Read) => "run",
        (Case::Remote(Mode::AskEveryTime), _) => "ask",
    }
}

/// S2's acceptance: one decision function, a table over every catalog
/// entry and every principal, with no path, a path inside the folders and
/// one outside them for every entry that can name one.
#[test]
fn one_decision_over_every_entry_and_every_principal() {
    let inside_folder = abs("/work");
    let folders = [PathBuf::from(&inside_folder)];
    let inside_path = abs("/work/board.sparksmith");
    let outside_path = abs("/elsewhere/board.sparksmith");
    let inside = |path: &str| Path::new(path).starts_with(&inside_folder);
    let modes = [Mode::AskEveryTime, Mode::AutoAllowEdits, Mode::AllowAll];
    let entries = metadata::entries();
    assert!(entries.len() > 200, "the whole catalog: {}", entries.len());
    // One namespace: a call's entry names exactly one.
    let names: std::collections::BTreeSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), entries.len());
    let mut checked = 0usize;
    for entry in entries {
        assert!(
            sparksmith_core::permission::MODULES
                .iter()
                .any(|m| m.id == entry.module),
            "{} is in a declared module",
            entry.name
        );
        let all = Grant::new(Mode::AllowAll, folders.clone());
        let without = all.clone().without_module(entry.module);
        let read_only = all.clone().with_module(entry.module, Class::Read);
        let local = Grant::local_agent(folders.clone());
        let remote: Vec<Grant> = modes
            .iter()
            .map(|mode| Grant::new(*mode, folders.clone()))
            .collect();
        let mut cases: Vec<(Case, Principal<'_>)> = vec![
            (Case::Person, Principal::Person),
            (Case::Operator, Principal::Operator),
            (Case::Local, Principal::LocalAgent { grant: &local }),
            (
                Case::WithoutModule,
                Principal::RemoteAgent {
                    grant: &without,
                    client: "table",
                },
            ),
            (
                Case::ReadOnlyModule,
                Principal::RemoteAgent {
                    grant: &read_only,
                    client: "table",
                },
            ),
        ];
        for (mode, grant) in modes.iter().zip(&remote) {
            cases.push((
                Case::Remote(*mode),
                Principal::RemoteAgent {
                    grant,
                    client: "table",
                },
            ));
        }
        let subject = Subject {
            name: &entry.name,
            module: entry.module,
            class: entry.class,
        };
        let mut paths: Vec<(Vec<String>, bool)> = vec![(Vec::new(), false)];
        if entry.takes_path {
            paths.push((vec![inside_path.clone()], false));
            paths.push((vec![outside_path.clone()], true));
        }
        for (case, who) in &cases {
            for (named, outside) in &paths {
                let class = if named.is_empty() {
                    entry.class
                } else {
                    entry.class.max(Class::Ask)
                };
                let got = decide_entries(who, &[subject], named, &inside);
                assert_eq!(
                    verdict(&got),
                    expected(case, class, entry.module, *outside),
                    "{} as {who:?} naming {named:?}",
                    entry.name
                );
                checked += 1;
            }
        }
    }
    assert!(checked > entries.len() * 8);
}

/// The table's own decisions are the ones calls get: a model's call is
/// refused for its entry's module whether it names the operation itself
/// or sends it inside `project.apply` or `project.apply_all`, and nothing
/// changes.
#[test]
fn a_grant_without_layout_refuses_add_via_and_changes_nothing() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let before = session.engine().to_file();
    let grant = Grant::new(Mode::AllowAll, []).without_module("layout");
    let who = Principal::RemoteAgent {
        grant: &grant,
        client: "test",
    };
    let params = via_params(&session);
    let operation =
        json!({"editor": "Layout", "operation": {"command": "layout.add_via", "params": params}});
    let calls = [
        call("layout.add_via", params.clone(), Some(revision)),
        call(
            "project.apply",
            json!({"operation": operation}),
            Some(revision),
        ),
        call(
            "project.apply_all",
            json!({"operations": [operation]}),
            Some(revision),
        ),
    ];
    for refused in calls {
        let entry = refused.entry.clone();
        let Err(error) = run(&mut session, who, refused) else {
            return Err(format!("{entry} ran without layout").into());
        };
        assert_eq!(error.code, "gate.module", "{entry}: {error:?}");
        assert!(error.message.contains("layout.add_via"), "{error:?}");
        assert!(error.message.contains("layout module"), "{error:?}");
        assert!(error.instead.is_some());
        assert_eq!(session.engine().revision(), revision, "{entry}");
        assert_eq!(session.engine().to_file(), before, "{entry}");
    }
    // The same through the transports' JSON.
    let reply = session.respond(
        crate::gate::Caller::Model(&grant),
        json!({"command": "project.apply", "params": {"expected_revision": revision, "operation": operation}}),
    );
    assert_eq!(reply["error"]["code"], "gate.module", "{reply}");
    assert_eq!(session.engine().to_file(), before);

    // With layout granted, the very call runs: the refusal was the module's.
    let granted = grant.with_module("layout", Class::Edit);
    let who = Principal::RemoteAgent {
        grant: &granted,
        client: "test",
    };
    let reply = run(
        &mut session,
        who,
        call("layout.add_via", params, Some(revision)),
    )
    .map_err(failed)?;
    assert_eq!(reply["revision"], revision + 1, "{reply}");
    assert!(reply["changes"]["added"].is_object(), "{reply}");
    Ok(())
}

/// A layout grant up to read lets a model read the board but not edit it.
#[test]
fn a_ceiling_lets_a_module_be_read_only() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let grant = Grant::new(Mode::AllowAll, []).with_module("layout", Class::Read);
    let who = Principal::RemoteAgent {
        grant: &grant,
        client: "test",
    };
    let params = via_params(&session);
    let Err(error) = run(
        &mut session,
        who,
        call("layout.add_via", params, Some(revision)),
    ) else {
        return Err("a read-only layout grant added a via".into());
    };
    assert_eq!(error.code, "gate.module");
    assert!(error.message.contains("only up to read"), "{error:?}");
    // Another module's edits still run.
    let undo = run(
        &mut session,
        who,
        call("project.undo", json!({}), Some(revision)),
    );
    assert_ne!(undo.err().map(|e| e.code), Some("gate.module".into()));
    Ok(())
}

/// Every entry is called by its own name, with the revision on the
/// envelope: an operation, a query and a request alike.
#[test]
fn an_entry_is_called_by_its_own_name_with_the_revision_on_the_envelope() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let vias = session.engine().project().vias.len();
    let params = via_params(&session);
    let added = run(
        &mut session,
        Principal::Operator,
        call("layout.add_via", params, Some(revision)),
    )
    .map_err(failed)?;
    assert_eq!(added["revision"], revision + 1, "{added}");
    let bom = run(
        &mut session,
        Principal::Operator,
        call("project.bom", json!({}), None),
    )
    .map_err(failed)?;
    assert!(bom.is_object() || bom.is_array(), "{bom}");
    let undone = run(
        &mut session,
        Principal::Operator,
        call("project.undo", json!({}), Some(revision + 1)),
    )
    .map_err(failed)?;
    assert!(undone["step"].is_object(), "{undone}");
    assert_eq!(session.engine().project().vias.len(), vias);
    Ok(())
}

/// Stale, malformed and outside-folder calls change nothing: no revision,
/// no step, no file.
#[test]
fn stale_malformed_and_outside_folder_calls_change_nothing() -> Fallible {
    let work = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    let saved = work.path().join("board.sparksmith");
    let mut session = Session::new(Engine::demo()?);
    run(
        &mut session,
        Principal::Operator,
        call(
            "project.save",
            json!({"path": saved.display().to_string()}),
            None,
        ),
    )
    .map_err(failed)?;
    let revision = session.engine().revision();
    let before = session.engine().to_file();
    let grant = Grant::new(Mode::AllowAll, [work.path().to_path_buf()]);
    let who = Principal::RemoteAgent {
        grant: &grant,
        client: "test",
    };
    let params = via_params(&session);
    let outside = elsewhere.path().join("copy.sparksmith");
    let refused: Vec<(Call, &str)> = vec![
        // Stale: an operation, a request and a file open.
        (
            call("layout.add_via", params.clone(), Some(revision + 7)),
            "stale_revision",
        ),
        (
            call("project.undo", json!({}), Some(revision.saturating_sub(1))),
            "stale_revision",
        ),
        (
            call(
                "project.open",
                json!({"path": saved.display().to_string()}),
                Some(revision + 1),
            ),
            "stale_revision",
        ),
        // Malformed: a missing revision, a wrong parameter, a doubled
        // revision, an unknown entry, an envelope field not served yet.
        (
            call("layout.add_via", params.clone(), None),
            "malformed_request",
        ),
        (call("project.undo", json!({}), None), "malformed_request"),
        (
            call(
                "layout.add_via",
                json!({"via": {"net": "GND"}}),
                Some(revision),
            ),
            "malformed_request",
        ),
        (
            call(
                "project.undo",
                json!({"expected_revision": revision + 1}),
                Some(revision),
            ),
            "malformed_request",
        ),
        (
            call("layout.add_vias", params.clone(), Some(revision)),
            "catalog.unknown",
        ),
        (
            Call {
                turn: Some("t_1".into()),
                ..call("layout.add_via", params.clone(), Some(revision))
            },
            "host.unsupported",
        ),
        // Outside the folders: a save and an open.
        (
            call(
                "project.save",
                json!({"path": outside.display().to_string()}),
                None,
            ),
            "gate.path_outside",
        ),
        (
            call(
                "project.open",
                json!({"path": outside.display().to_string()}),
                Some(revision),
            ),
            "gate.path_outside",
        ),
    ];
    for (refused, code) in refused {
        let entry = refused.entry.clone();
        let Err(error) = run(&mut session, who, refused) else {
            return Err(format!("{entry} ran").into());
        };
        assert_eq!(error.code, code, "{entry}: {error:?}");
        assert_eq!(session.engine().revision(), revision, "{entry}");
        assert_eq!(session.engine().to_file(), before, "{entry}");
        assert_eq!(session.path(), Some(saved.as_path()), "{entry}");
        assert!(!outside.exists(), "{entry}");
    }
    let unknown = run(
        &mut session,
        who,
        call("layout.add_vias", json!({}), Some(revision)),
    );
    let closest = unknown.err().map(|e| e.closest).unwrap_or_default();
    assert!(
        closest.iter().any(|name| name == "layout.add_via"),
        "{closest:?}"
    );
    // A stale revision names both numbers.
    let Err(stale) = run(
        &mut session,
        Principal::Operator,
        call("project.undo", json!({}), Some(revision + 3)),
    ) else {
        return Err("a stale undo ran".into());
    };
    assert_eq!(stale.expected_revision, Some(revision + 3));
    assert_eq!(stale.actual_revision, Some(revision));
    Ok(())
}

/// What needs a person goes to the host's asker: Allow runs, Deny and no
/// way to ask are refused with nothing changed; a dry run asks no one.
#[test]
fn the_asker_answers_what_needs_a_person() -> Fallible {
    struct Answers(Answer, usize);
    impl Asker for Answers {
        fn ask(&mut self, _: &Question) -> Answer {
            self.1 += 1;
            self.0
        }
    }
    let mut session = Session::new(Engine::demo()?);
    let grant = Grant::new(Mode::AskEveryTime, []);
    let who = Principal::RemoteAgent {
        grant: &grant,
        client: "test",
    };
    let before = session.engine().to_file();
    for (answer, code) in [
        (Answer::Deny, "gate.denied"),
        (Answer::Unavailable, "gate.cannot_ask"),
    ] {
        let revision = session.engine().revision();
        let mut asker = Answers(answer, 0);
        let cx = CallCx {
            who,
            asker: &mut asker,
            verbose: false,
        };
        let Err(error) = session.call(
            cx,
            call("layout.add_via", via_params(&session), Some(revision)),
        ) else {
            return Err("an edit ran without Allow".into());
        };
        assert_eq!(error.code, code);
        assert_eq!(asker.1, 1);
        assert_eq!(session.engine().to_file(), before);
    }
    let revision = session.engine().revision();
    let mut asker = Answers(Answer::Deny, 0);
    let dry = Call {
        dry_run: true,
        ..call("layout.add_via", via_params(&session), Some(revision))
    };
    let tried = session
        .call(
            CallCx {
                who,
                asker: &mut asker,
                verbose: false,
            },
            dry,
        )
        .map_err(failed)?;
    assert_eq!(tried["dry_run"], true, "{tried}");
    assert_eq!(asker.1, 0, "a dry run keeps nothing, so no one is asked");
    assert_eq!(session.engine().to_file(), before);
    let mut asker = Answers(Answer::Allow, 0);
    let ran = session
        .call(
            CallCx {
                who,
                asker: &mut asker,
                verbose: false,
            },
            call("layout.add_via", via_params(&session), Some(revision)),
        )
        .map_err(failed)?;
    assert_eq!(ran["revision"], revision + 1, "{ran}");
    Ok(())
}

/// Every entry that changes the document names the revision it expects,
/// and no read does: the envelope's revision follows from the entry.
#[test]
fn edits_and_only_document_changes_take_the_revision() {
    for entry in metadata::entries() {
        let EntryMeta {
            name,
            class,
            revision,
            ..
        } = entry;
        match class {
            Class::Edit => assert!(revision, "{name} edits the document"),
            Class::Read => assert!(!revision, "{name} only reads"),
            _ => {}
        }
    }
}

/// The transports' JSON is the same call: its revision moves to the
/// envelope, `verbose` asks for the snapshot, and other keys are refused.
#[test]
fn a_request_reads_as_a_call() -> Fallible {
    let (call, verbose) = from_request(json!({"command": "project.undo",
        "params": {"expected_revision": 4}, "verbose": true}))
    .map_err(failed)?;
    assert_eq!(call.entry, "project.undo");
    assert_eq!(call.expected_revision, Some(4));
    assert_eq!(call.params, json!({}));
    assert!(verbose);
    let (query, _) = from_request(json!({"command": "project.query",
        "params": {"query": "project.bom", "params": {}}}))
    .map_err(failed)?;
    assert_eq!(query.expected_revision, None);
    let Err(error) = from_request(json!({"command": "project.undo", "extra": 1})) else {
        return Err("an unknown key was read".into());
    };
    assert_eq!(error.code, "malformed_request");
    Ok(())
}

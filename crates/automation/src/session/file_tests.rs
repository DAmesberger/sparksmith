//! The project file's lifecycle through sessions: two sessions saving one
//! opened file (review finding 9), and a host that lends its engine and
//! owns its file (finding 8).
use super::*;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn ok(session: &mut Session, request: Value) -> Fallible<Value> {
    let reply = session.dispatch(request);
    if reply["ok"] == true {
        Ok(reply["result"].clone())
    } else {
        Err(format!("refused: {}", reply["error"]).into())
    }
}

fn open(session: &mut Session, path: &Path) -> Value {
    let revision = session.engine().revision();
    session.dispatch(json!({"command": "project.open", "params": {
        "expected_revision": revision, "path": path.display().to_string()}}))
}

fn save(session: &mut Session, path: Option<&Path>) -> Value {
    match path {
        Some(path) => session.dispatch(
            json!({"command": "project.save", "params": {"path": path.display().to_string()}}),
        ),
        None => session.dispatch(json!({"command": "project.save", "params": {}})),
    }
}

fn rename(session: &mut Session, name: &str) -> Fallible<Value> {
    let revision = session.engine().revision();
    let operation = serde_json::to_value(Operation::Schematic(
        sparksmith_core::schematic::SchematicCommand::RenameProject { name: name.into() },
    ))?;
    ok(
        session,
        json!({"command": "project.apply", "params": {"expected_revision": revision,
            "operation": operation}}),
    )
}

/// Finding 9: two sessions (two windows, or the CLI beside the app) open
/// one project and both save. The second save does not replace the
/// first: it says the file changed, keeps its own document beside it and
/// saves there from then on.
#[test]
fn a_second_session_saving_one_file_keeps_both_versions() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("board.sparksmith");
    sparksmith_io::write_project(&path, &Engine::demo()?.to_file())?;
    let mut a = Session::new(Engine::default());
    let mut b = Session::new(Engine::default());
    assert_eq!(open(&mut a, &path)["ok"], true);
    assert_eq!(open(&mut b, &path)["ok"], true);
    rename(&mut a, "First editor")?;
    rename(&mut b, "Second editor")?;
    assert_eq!(save(&mut a, None)["ok"], true);
    let refused = save(&mut b, None);
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "file_conflict", "{refused}");
    let kept = dir.path().join("board (conflict 1).sparksmith");
    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("board (conflict 1).sparksmith"),
        "{message}"
    );
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "First editor"
    );
    assert_eq!(
        sparksmith_io::read_project(&kept)?.project.name,
        "Second editor"
    );
    // The second session's file is the kept one now.
    assert_eq!(b.path(), Some(kept.as_path()));
    rename(&mut b, "Second editor, later")?;
    assert_eq!(save(&mut b, None)["ok"], true);
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "First editor"
    );
    // Save As to a file the request names replaces it, as asked.
    assert_eq!(save(&mut b, Some(&path))["ok"], true);
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "Second editor, later"
    );
    Ok(())
}

fn host(path: &Path, unsaved: bool) -> HostFile {
    HostFile {
        path: Some(path.to_path_buf()),
        identity: sparksmith_io::open_project(path).ok().map(|(_, id)| id),
        unsaved,
        busy: false,
    }
}

/// Finding 8: a host lends its engine (the desktop window, to its
/// assistant) with file A. An open of B through the lent session comes
/// back as a file change naming B and its identity, so the host's next
/// save writes B's document to B, never to A.
#[test]
fn a_lent_open_comes_back_as_the_hosts_new_file() -> Fallible {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.sparksmith");
    let b = dir.path().join("b.sparksmith");
    let original = Engine::demo()?.to_file();
    sparksmith_io::write_project(&a, &original)?;
    let mut other = Engine::demo()?;
    other.apply(
        other.revision(),
        Operation::Schematic(
            sparksmith_core::schematic::SchematicCommand::RenameProject {
                name: "Board B".into(),
            },
        ),
    )?;
    sparksmith_io::write_project(&b, &other.to_file())?;
    let mut engine = Engine::from_file(sparksmith_io::read_project(&a)?)?;
    let mut lending = Session::lending();
    let (reply, changes) = lending.lend_file(&mut engine, &host(&a, false), |s| open(s, &b));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(engine.project().name, "Board B");
    let (_, b_identity) = sparksmith_io::open_project(&b)?;
    assert_eq!(
        changes,
        vec![FileChange::Opened {
            path: b.clone(),
            identity: b_identity,
            revision: engine.revision(),
        }]
    );
    // A is untouched; a save in the next lend, with the host's file now
    // B, writes B.
    let (reply, changes) = lending.lend_file(&mut engine, &host(&b, false), |s| save(s, None));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(matches!(&changes[..], [FileChange::Saved { path, .. }] if *path == b));
    assert_eq!(sparksmith_io::read_project(&a)?, original);
    Ok(())
}

/// The unsaved-work transition holds through a lend: with unsaved edits
/// in the host, or edits the lent session made, an open or new project is
/// refused and nothing changes; a save is allowed and then an open is.
#[test]
fn a_lent_open_never_drops_unsaved_work() -> Fallible {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.sparksmith");
    let b = dir.path().join("b.sparksmith");
    sparksmith_io::write_project(&a, &Engine::demo()?.to_file())?;
    sparksmith_io::write_project(&b, &Engine::demo()?.to_file())?;
    let mut engine = Engine::from_file(sparksmith_io::read_project(&a)?)?;
    let mut lending = Session::lending();
    let before = engine.revision();
    let (reply, changes) = lending.lend_file(&mut engine, &host(&a, true), |s| open(s, &b));
    assert_eq!(reply["ok"], false);
    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unsaved changes"), "{message}");
    assert!(changes.is_empty());
    assert_eq!(engine.revision(), before);
    let (reply, _) = lending.lend_file(&mut engine, &host(&a, true), |s| {
        let revision = s.engine().revision();
        s.dispatch(json!({"command": "project.new", "params": {"expected_revision": revision}}))
    });
    assert_eq!(reply["ok"], false);
    // An edit made in the lend is unsaved work too.
    let (reply, changes) = lending.lend_file(&mut engine, &host(&a, false), |s| {
        rename(s, "Edited")?;
        Fallible::Ok(open(s, &b))
    });
    assert_eq!(reply?["ok"], false);
    assert!(changes.is_empty());
    // Saved first, then opened: both reach the host, in order.
    let (reply, changes) = lending.lend_file(&mut engine, &host(&a, true), |s| {
        let saved = save(s, None);
        (saved, open(s, &b))
    });
    assert_eq!(reply.0["ok"], true, "{}", reply.0);
    assert_eq!(reply.1["ok"], true, "{}", reply.1);
    assert!(matches!(
        &changes[..],
        [FileChange::Saved { path: saved, .. }, FileChange::Opened { path: opened, .. }]
            if *saved == a && *opened == b
    ));
    assert_eq!(sparksmith_io::read_project(&a)?.project.name, "Edited");
    // A host that is busy with a file of its own refuses all three.
    let busy = HostFile {
        busy: true,
        ..host(&b, false)
    };
    let (reply, _) = lending.lend_file(&mut engine, &busy, |s| save(s, None));
    assert_eq!(reply["ok"], false);
    // A plain lend (a fork) cannot open, start or save a project.
    let mut fork = engine.fork();
    let reply = lending.lend(&mut fork, Some(&b), |s| save(s, None));
    assert_eq!(reply["ok"], false);
    Ok(())
}

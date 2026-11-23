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

/// The host's file at `path`, holding the revision `kept` (`None`: the
/// host holds work no file has).
fn host(path: &Path, kept: Option<u64>) -> HostFile {
    HostFile {
        path: Some(path.to_path_buf()),
        identity: sparksmith_io::open_project(path).ok().map(|(_, id)| id),
        kept,
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
    let file = host(&a, Some(engine.revision()));
    let (reply, changes) = lending.lend_file(&mut engine, &file, |s| open(s, &b));
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
    let file = host(&b, Some(engine.revision()));
    let (reply, changes) = lending.lend_file(&mut engine, &file, |s| save(s, None));
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
    let file = host(&a, None);
    let (reply, changes) = lending.lend_file(&mut engine, &file, |s| open(s, &b));
    assert_eq!(reply["ok"], false);
    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unsaved changes"), "{message}");
    assert!(changes.is_empty());
    assert_eq!(engine.revision(), before);
    let file = host(&a, None);
    let (reply, _) = lending.lend_file(&mut engine, &file, |s| {
        let revision = s.engine().revision();
        s.dispatch(json!({"command": "project.new", "params": {"expected_revision": revision}}))
    });
    assert_eq!(reply["ok"], false);
    // An edit made in the lend is unsaved work too.
    let file = host(&a, Some(engine.revision()));
    let (reply, changes) = lending.lend_file(&mut engine, &file, |s| {
        rename(s, "Edited")?;
        Fallible::Ok(open(s, &b))
    });
    assert_eq!(reply?["ok"], false);
    assert!(changes.is_empty());
    // Saved first, then opened: both reach the host, in order.
    let file = host(&a, None);
    let (reply, changes) = lending.lend_file(&mut engine, &file, |s| {
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
        ..host(&b, Some(engine.revision()))
    };
    let (reply, _) = lending.lend_file(&mut engine, &busy, |s| save(s, None));
    assert_eq!(reply["ok"], false);
    // A plain lend (a fork) cannot open, start or save a project.
    let mut fork = engine.fork();
    let reply = lending.lend(&mut fork, Some(&b), |s| save(s, None));
    assert_eq!(reply["ok"], false);
    Ok(())
}

/// The spellings of `path` that reach it another way: through `..`, a
/// symbolic link to its folder and, on a volume that ignores letter case,
/// in capitals.
fn other_spellings(path: &Path) -> Fallible<Vec<PathBuf>> {
    let folder = path.parent().ok_or("no folder")?;
    let name = path.file_name().ok_or("no name")?;
    let sub = folder.join("sub");
    std::fs::create_dir_all(&sub)?;
    let mut spellings = vec![sub.join("..").join(name)];
    #[cfg(unix)]
    {
        let link = folder.join("link");
        if !link.exists() {
            std::os::unix::fs::symlink(folder, &link)?;
        }
        spellings.push(link.join(name));
    }
    let upper = folder.join(name.to_string_lossy().to_uppercase());
    if upper.exists() {
        spellings.push(upper);
    }
    Ok(spellings)
}

/// Review finding 16: two editors open one file under different
/// spellings. The second save, under its own spelling, finds the file
/// changed exactly as a same-spelling save does: the first editor's work
/// stays in the file and the second's goes beside it.
#[test]
fn two_spellings_of_one_file_keep_both_editors_work() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("board.sparksmith");
    for (n, other) in other_spellings(&path)?.into_iter().enumerate() {
        for entry in std::fs::read_dir(dir.path())? {
            let entry = entry?.path();
            if entry.extension().is_some_and(|e| e == "sparksmith") {
                std::fs::remove_file(entry)?;
            }
        }
        sparksmith_io::write_project(&path, &Engine::demo()?.to_file())?;
        let mut a = Session::new(Engine::default());
        let mut b = Session::new(Engine::default());
        assert_eq!(open(&mut a, &path)["ok"], true);
        assert_eq!(open(&mut b, &other)["ok"], true);
        rename(&mut a, &format!("First editor {n}"))?;
        rename(&mut b, &format!("Second editor {n}"))?;
        assert_eq!(save(&mut a, None)["ok"], true);
        let first = std::fs::read(&path)?;
        // The second editor saves to the first's spelling, as a person
        // typing the usual path would.
        let refused = save(&mut b, Some(&path));
        assert_eq!(
            refused["error"]["code"], "file_conflict",
            "{other:?}: {refused}"
        );
        assert_eq!(
            std::fs::read(&path)?,
            first,
            "{other:?}: the first save stays"
        );
        let kept = dir.path().join("board (conflict 1).sparksmith");
        assert_eq!(
            sparksmith_io::read_project(&kept)?.project.name,
            format!("Second editor {n}")
        );
        // And to its own spelling: the same file, the same check.
        let mut c = Session::new(Engine::default());
        assert_eq!(open(&mut c, &other)["ok"], true);
        rename(&mut c, "Third editor")?;
        rename(&mut a, &format!("First editor {n}, again"))?;
        assert_eq!(
            save(&mut a, None)["ok"],
            true,
            "the first editor saves again"
        );
        let again = std::fs::read(&path)?;
        let refused = save(&mut c, None);
        assert_eq!(
            refused["error"]["code"], "file_conflict",
            "{other:?}: {refused}"
        );
        assert_eq!(std::fs::read(&path)?, again);
    }
    Ok(())
}

/// Review finding 16: a first save that fails gives the project no
/// identity; the next save gives it the one a first save under that name
/// gives, and a session reopening the file reads the same one.
#[test]
fn a_failed_first_save_leaves_the_project_without_an_identity() -> Fallible {
    let dir = tempfile::tempdir()?;
    let good = dir.path().join("good.sparksmith");
    let bad = dir.path().join("missing").join("bad.sparksmith");
    let mut session = Session::new(Engine::demo()?);
    let before = session.engine().to_file();
    let undo = session.engine().can_undo();
    let failed = save(&mut session, Some(&bad));
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(session.engine().to_file(), before, "nothing changed");
    assert_eq!(session.engine().can_undo(), undo);
    assert!(session.engine().project().production.project_id.is_none());
    assert_eq!(save(&mut session, Some(&good))["ok"], true);
    let saved = sparksmith_io::read_project(&good)?
        .project
        .production
        .project_id;
    assert!(saved.is_some());
    assert_eq!(session.engine().project().production.project_id, saved);
    // A control saved straight to the good name has the same identity.
    let control_dir = tempfile::tempdir()?;
    let control = control_dir.path().join("good.sparksmith");
    let mut direct = Session::new(Engine::demo()?);
    assert_eq!(save(&mut direct, Some(&control))["ok"], true);
    assert_eq!(
        sparksmith_io::read_project(&control)?
            .project
            .production
            .project_id,
        saved
    );
    Ok(())
}

/// Review finding 17: a host lends its engine twice in one frame, the
/// first call an edit, the second a new project, before it has looked at
/// the edit. The host's file holds the revision before the edit; the
/// engine is past it, so the new project is refused and the edit and its
/// undo stay.
#[test]
fn an_edit_the_host_has_not_seen_is_not_taken_for_saved() -> Fallible {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.sparksmith");
    sparksmith_io::write_project(&a, &Engine::demo()?.to_file())?;
    let mut engine = Engine::from_file(sparksmith_io::read_project(&a)?)?;
    let file = host(&a, Some(engine.revision()));
    let mut lending = Session::lending();
    let (renamed, _) = lending.lend_file(&mut engine, &file, |s| rename(s, "Unsaved edit"));
    renamed?;
    // The same host file: the host has not seen the edit yet.
    let (reply, changes) = lending.lend_file(&mut engine, &file, |s| {
        let revision = s.engine().revision();
        s.dispatch(json!({"command": "project.new", "params": {"expected_revision": revision}}))
    });
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(changes.is_empty());
    assert_eq!(engine.project().name, "Unsaved edit");
    assert!(engine.can_undo());
    let (reply, _) = lending.lend_file(&mut engine, &file, |s| open(s, &a));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(engine.project().name, "Unsaved edit");
    Ok(())
}

/// A note of about 8 KiB as saved: quotes are escaped.
fn heavy_note(n: usize) -> Fallible<Operation> {
    use sparksmith_core::SheetItem;
    use sparksmith_core::schematic::SchematicCommand;
    use sparksmith_core::units::{Nm, PointNm};
    let mm = |v: i64| Nm::try_new(v * 1_000_000).ok_or("nm");
    Ok(Operation::Schematic(SchematicCommand::AddDrawing {
        sheet: None,
        item: SheetItem::Text {
            at: PointNm {
                x_nm: mm((n % 200) as i64)?,
                y_nm: mm((n / 200) as i64)?,
            },
            text: format!("{}a", "\"".repeat(4095)),
            height_nm: mm(2)?,
            rotation: Default::default(),
        },
    }))
}

/// Review finding 15 and fix order 4: notes are added until the engine
/// refuses one, which leaves document, revision and history as they were.
/// The last accepted state, within one note of the limit and still
/// without an identity, takes one on its first save, saves, reopens with
/// it, and keeps a recovery snapshot that reads back as it is.
#[test]
fn a_project_filled_to_the_limit_saves_identifies_and_recovers() -> Fallible {
    let mut engine = Engine::demo()?;
    let mut size = sparksmith_core::MAX_BATCH;
    let mut n = 0;
    loop {
        let notes = (0..size)
            .map(|k| heavy_note(n + k))
            .collect::<Fallible<Vec<_>>>()?;
        match engine.apply_all_hosted(engine.revision(), notes, Default::default()) {
            Ok(_) => n += size,
            Err(error) => {
                assert!(error.to_string().contains("could not be saved"), "{error}");
                if size == 1 {
                    break;
                }
                size /= 2;
            }
        }
        assert!(n < 20_000, "the limit was never reached");
    }
    assert!(n > 7_000, "{n} notes");
    let before = (engine.to_file(), engine.revision(), engine.undo_step());
    let refused = engine.apply(engine.revision(), heavy_note(n)?);
    assert!(refused.is_err());
    assert_eq!(
        (engine.to_file(), engine.revision(), engine.undo_step()),
        before
    );
    assert!(engine.project().production.project_id.is_none());

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("full.sparksmith");
    let mut session = Session::new(engine);
    let saved = save(&mut session, Some(&path));
    assert_eq!(saved["ok"], true, "{saved}");
    let id = session.engine().project().production.project_id.clone();
    assert!(id.is_some());
    let reopened = sparksmith_io::read_project(&path)?;
    assert_eq!(reopened.project.production.project_id, id);
    assert_eq!(
        reopened.project.drawings.len(),
        before.0.project.drawings.len()
    );
    assert_eq!(&reopened, &session.engine().to_file());
    // The recovery writer keeps the same state.
    let mut store = sparksmith_io::recovery::RecoveryStore::new(&dir.path().join("recovery"))?;
    store.write(&session.engine().to_file(), Some(&path))?;
    let snapshot = store.path().ok_or("a snapshot")?;
    assert_eq!(
        sparksmith_io::recovery::read(snapshot)?.document,
        session.engine().to_file()
    );
    // The history is the one the edits made: the last batch undoes.
    let drawings = session.engine().project().drawings.len();
    let revision = session.engine().revision();
    ok(
        &mut session,
        json!({"command": "project.undo", "params": {"expected_revision": revision}}),
    )?;
    assert!(session.engine().project().drawings.len() < drawings);
    Ok(())
}

/// Fix order 4, interruption during save: a save that cannot finish (its
/// folder refuses the new file) leaves the file's bytes, the session's
/// file and identity, the document and its undo as they were; the next
/// save, once the folder takes files again, goes through.
#[cfg(unix)]
#[test]
fn a_save_that_cannot_finish_leaves_the_file_and_history() -> Fallible {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let folder = dir.path().join("boards");
    std::fs::create_dir(&folder)?;
    let path = folder.join("board.sparksmith");
    let mut session = Session::new(Engine::demo()?);
    assert_eq!(save(&mut session, Some(&path))["ok"], true);
    let bytes = std::fs::read(&path)?;
    let identity = session.engine().project().production.project_id.clone();
    rename(&mut session, "Edited")?;
    let state = (
        session.engine().to_file(),
        session.engine().revision(),
        session.engine().undo_step(),
    );
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555))?;
    let failed = save(&mut session, None);
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755))?;
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(std::fs::read(&path)?, bytes, "the file is untouched");
    assert_eq!(std::fs::read_dir(&folder)?.count(), 1, "no temporary file");
    assert_eq!(
        (
            session.engine().to_file(),
            session.engine().revision(),
            session.engine().undo_step()
        ),
        state
    );
    assert_eq!(session.path(), Some(path.as_path()));
    assert_eq!(session.engine().project().production.project_id, identity);
    assert_eq!(save(&mut session, None)["ok"], true);
    assert_eq!(sparksmith_io::read_project(&path)?.project.name, "Edited");
    Ok(())
}

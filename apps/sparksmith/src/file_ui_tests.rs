//! Quitting with unsaved work, in headless egui frames with the real file
//! worker: the dialog, each answer, a quit during a save, and an exit that
//! nobody answered. Recovery snapshots go to a temporary folder.
use super::*;
use sparksmith_core::Operation;
use sparksmith_core::schematic::SchematicCommand;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const TITLE: &str = "Save changes before quitting?";

struct Harness {
    ctx: egui::Context,
    files: FileUi,
    engine: Engine,
    dir: tempfile::TempDir,
    now: f64,
    effects: Vec<Effect>,
}

impl Harness {
    fn new() -> Fallible<Self> {
        Self::in_dir(tempfile::tempdir()?)
    }

    /// A session whose recovery folder is `dir`'s, as left by earlier ones.
    fn in_dir(dir: tempfile::TempDir) -> Fallible<Self> {
        let ctx = egui::Context::default();
        sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
        let engine = Engine::demo()?;
        let files = FileUi::with_directory(
            &ctx,
            rfd::AsyncFileDialog::new(),
            &engine,
            dir.path().join("recovery"),
        );
        let mut harness = Self {
            ctx,
            files,
            engine,
            dir,
            now: 0.0,
            effects: Vec::new(),
        };
        // The worker's startup listing (an empty folder) arrives first.
        harness.frame(vec![]);
        Ok(harness)
    }

    /// One frame as the app runs it: file results, then the document, then
    /// the dialogs. Returns what it painted.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(String, egui::Pos2)> {
        let Self {
            ctx,
            files,
            engine,
            now,
            effects,
            ..
        } = self;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            time: Some(*now),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            effects.extend(files.tick(ui.ctx()));
            files.observe(engine, *now);
            files.dialogs(ui.ctx());
        });
        output.textures_delta.clear();
        let mut painted = Vec::new();
        for shape in &output.shapes {
            texts(&shape.shape, &mut painted);
        }
        painted
    }

    /// Frames until `done` holds, waiting for the worker between them.
    fn until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) -> Fallible {
        let deadline = std::time::Instant::now() + crate::app::ax_harness::PATIENCE;
        while std::time::Instant::now() < deadline {
            if done(self) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
            self.frame(vec![]);
        }
        Err(format!("timed out waiting for {what}").into())
    }

    /// Change the project, as an edit in any workspace would.
    fn edit(&mut self, name: &str) -> Fallible {
        self.engine.apply(
            self.engine.revision(),
            Operation::Schematic(SchematicCommand::RenameProject { name: name.into() }),
        )?;
        self.frame(vec![]);
        assert!(self.files.dirty());
        Ok(())
    }

    /// The painted texts once a newly opened dialog shows: egui lays a
    /// new window out invisibly in its first frame.
    fn shown(&mut self) -> Vec<(String, egui::Pos2)> {
        self.frame(vec![]);
        self.frame(vec![])
    }

    /// Click the dialog button labelled `label`.
    fn click(&mut self, label: &str) -> Fallible {
        let painted = self.shown();
        let at = painted
            .iter()
            .find(|(text, _)| text == label)
            .map(|(_, at)| *at)
            .ok_or_else(|| format!("no {label} button among {painted:?}"))?;
        self.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        self.frame(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        Ok(())
    }

    fn closed(&self) -> bool {
        self.effects.iter().any(|e| matches!(e, Effect::Close))
    }

    /// The project names this session's recovery folder holds.
    fn recoverable(&self) -> Fallible<Vec<String>> {
        self.recovery_files()?
            .into_iter()
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext == "sparksmith-recovery")
            })
            .map(|p| Ok(recovery::read(&p)?.document.project.name))
            .collect()
    }

    /// Every file in the recovery folder (none before it exists), empty
    /// session reservations too.
    fn recovery_files(&self) -> Fallible<Vec<PathBuf>> {
        let mut files = Vec::new();
        let entries = match std::fs::read_dir(self.dir.path().join("recovery")) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(files),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            files.push(entry?.path());
        }
        Ok(files)
    }

    /// Wait for the background snapshot of the current edit (taken two
    /// seconds after the last change).
    fn snapshot(&mut self, name: &str) -> Fallible {
        self.now += 3.0;
        self.until("the recovery snapshot", |h| {
            h.recoverable().is_ok_and(|names| names == [name])
        })
    }
}

fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
    match shape {
        egui::Shape::Text(text) => out.push((
            text.galley.job.text.clone(),
            text.pos + text.galley.size() * 0.5,
        )),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
        _ => {}
    }
}

fn shows(painted: &[(String, egui::Pos2)], text: &str) -> bool {
    painted.iter().any(|(t, _)| t == text)
}

/// A restored session has no file of its own yet: Save proposes the name
/// and folder of the file it was restored from, not "untitled" at home.
#[test]
fn save_after_a_restore_proposes_the_original_file() -> Fallible {
    let dir = tempfile::tempdir()?;
    let original = dir.path().join("prec").join("board.sparksmith");
    let engine = Engine::demo()?;
    RecoveryStore::new(&dir.path().join("recovery"))?.write(
        &EditorFile::from_document(engine.document()),
        Some(&original),
    )?;
    let mut h = Harness::in_dir(dir)?;
    h.until("the recovery question", |h| !h.files.recoveries.is_empty())?;
    h.click("Restore")?;
    h.until("the restored document", |h| {
        h.effects.iter().any(|e| {
            matches!(
                e,
                Effect::Replace {
                    recovered: true,
                    ..
                }
            )
        })
    })?;
    // The app adopts it as an unsaved document without a path.
    h.files.adopt(&h.engine, None, None, true)?;
    let (folder, name) = h.files.save_proposal();
    assert_eq!(Some(folder), original.parent());
    assert_eq!(name, "board.sparksmith");
    // Once the project has a file of its own, that one is proposed.
    let own = h.dir.path().join("own.sparksmith");
    h.files.adopt(&h.engine, Some(own), None, false)?;
    assert_eq!(h.files.save_proposal().1, "own.sparksmith");
    Ok(())
}

/// The worker restores a particular source version. Before the desktop
/// protects that copy in its own session, the source path is replaced.
#[test]
fn automatic_cleanup_after_restore_preserves_a_changed_source() -> Fallible {
    let dir = tempfile::tempdir()?;
    let folder = dir.path().join("recovery");
    let mut writer = RecoveryStore::new(&folder)?;
    writer.write(&Engine::demo()?.to_file(), None)?;
    let source = writer.path().ok_or("missing snapshot")?.to_owned();
    drop(writer);

    let mut h = Harness::in_dir(dir)?;
    h.until("the recovery question", |h| !h.files.recoveries.is_empty())?;
    h.click("Restore")?;
    h.until("the restored project", |h| !h.effects.is_empty())?;
    let Some(Effect::Replace {
        document,
        recovered: true,
        ..
    }) = h.effects.pop()
    else {
        return Err("expected the restored project".into());
    };
    h.engine.replace_document(*document)?;
    h.files.adopt(&h.engine, None, None, true)?;
    let mut newer = h.engine.to_file();
    newer.project.name = "Second project".into();
    let mut other = RecoveryStore::new(&folder)?;
    other.write(&newer, None)?;
    std::fs::copy(other.path().ok_or("missing other snapshot")?, &source)?;

    h.now += 3.0;
    h.until("the cleanup refusal", |h| {
        h.files.status.as_ref().is_some_and(|(level, text)| {
            *level == Level::Error && text.contains("snapshot changed")
        })
    })?;
    assert_eq!(recovery::read(&source)?.document, newer);
    let protected = h.recoverable()?;
    assert!(protected.contains(&h.engine.project().name));
    Ok(())
}

/// An original two-resistor KiCad board (written here; no KiCad files).
const KICAD_BOARD: &str = r#"(kicad_pcb (version 20240108) (generator "sparksmith_test")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (setup (pad_to_mask_clearance 0))
  (net 0 "") (net 1 "SIG")
  (footprint "Demo:R_0805" (layer "F.Cu") (at 10 10)
    (property "Reference" "R1" (at 0 -2 0) (layer "F.SilkS"))
    (pad "1" smd rect (at -1 0) (size 0.8 0.6) (layers "F.Cu") (net 1 "SIG"))
    (pad "2" smd rect (at 1 0) (size 0.8 0.6) (layers "F.Cu")))
  (gr_rect (start 0 0) (end 20 20) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
)
"#;

/// File › Import KiCad project… with a project file: the CLI's import
/// runs on the file worker (saying so), the new project arrives with its
/// report, and the app adopts it unsaved, its first save proposing the
/// KiCad project's folder and name.
#[test]
fn a_kicad_import_arrives_unsaved_with_its_report() -> Fallible {
    let mut h = Harness::new()?;
    let kicad = h.dir.path().join("kicad");
    std::fs::create_dir_all(&kicad)?;
    std::fs::write(kicad.join("demo.kicad_pcb"), KICAD_BOARD)?;
    let project = kicad.join("demo.kicad_pro");
    std::fs::write(&project, b"{}")?;
    h.files.request(Action::ImportKicadPath(project.clone()));
    assert!(h.files.importing());
    h.until("the imported project", |h| {
        h.effects
            .iter()
            .any(|e| matches!(e, Effect::Imported { .. }))
    })?;
    assert!(!h.files.importing());
    let Some(Effect::Imported {
        document,
        source,
        report,
    }) = h
        .effects
        .drain(..)
        .find(|e| matches!(e, Effect::Imported { .. }))
    else {
        return Err("no import".into());
    };
    assert_eq!(source, project);
    assert_eq!(report.counts.footprints, 1);
    let names: Vec<_> = document
        .project
        .library
        .components
        .iter()
        .map(|c| c.meta.name.clone())
        .collect();
    assert_eq!(
        names,
        ["R_0805"],
        "the footprint name without its library nickname"
    );
    h.engine.replace_document(*document)?;
    h.files.adopt_imported(&h.engine, source)?;
    assert!(h.files.dirty());
    assert_eq!(h.files.path(), None);
    let (folder, name) = h.files.save_proposal();
    assert_eq!(folder, kicad.as_path());
    assert_eq!(name, "demo.sparksmith");
    Ok(())
}

/// Stop ends a running import: nothing is imported, and the next one runs.
#[test]
fn a_stopped_import_imports_nothing_and_the_next_one_runs() -> Fallible {
    let mut h = Harness::new()?;
    let board = h.dir.path().join("demo.kicad_pcb");
    std::fs::write(&board, KICAD_BOARD)?;
    h.files.request(Action::ImportKicadPath(board.clone()));
    assert!(h.files.import_stoppable());
    h.files.stop_import();
    assert!(!h.files.import_stoppable());
    assert!(
        h.files
            .activity()
            .is_some_and(|(text, _)| text.starts_with("Stopping the import"))
    );
    h.until("the import to stop", |h| !h.files.importing())?;
    assert!(h.effects.is_empty());
    assert_eq!(
        h.files.take_status().map(|(_, text)| text).as_deref(),
        Some("Import stopped; nothing was imported")
    );
    h.files.request(Action::ImportKicadPath(board));
    h.until("the imported project", |h| {
        h.effects
            .iter()
            .any(|e| matches!(e, Effect::Imported { .. }))
    })
}

/// A KiCad file that is not a board says why, and nothing is replaced.
#[test]
fn a_failed_kicad_import_says_why() -> Fallible {
    let mut h = Harness::new()?;
    let board = h.dir.path().join("broken.kicad_pcb");
    std::fs::write(&board, b"(not a board)")?;
    h.files.request(Action::ImportKicadPath(board));
    h.until("the refusal", |h| !h.files.busy())?;
    h.frame(vec![]);
    let status = h.files.take_status().ok_or("no status")?;
    assert_eq!(status.0, Level::Error);
    assert!(status.1.starts_with("Import failed: "), "{}", status.1);
    assert!(
        h.effects
            .iter()
            .all(|e| !matches!(e, Effect::Imported { .. }))
    );
    Ok(())
}

/// Nothing unsaved: quitting closes the window at once, with no dialog.
#[test]
fn quitting_a_saved_project_closes_without_asking() -> Fallible {
    let mut h = Harness::new()?;
    h.files.request(Action::Close);
    let painted = h.shown();
    assert!(!shows(&painted, TITLE));
    h.until("the close", Harness::closed)
}

/// Unsaved work asks Save, Don't Save or Cancel. Cancel leaves everything
/// as it was: the window stays, the work stays unsaved and its recovery
/// snapshot stays on disk.
#[test]
fn cancel_keeps_the_window_and_the_work() -> Fallible {
    let mut h = Harness::new()?;
    h.edit("Sensor board")?;
    h.snapshot("Sensor board")?;
    let revision = h.engine.revision();
    h.files.request(Action::Close);
    let painted = h.shown();
    for text in [TITLE, "Save", DONT_SAVE, "Cancel"] {
        assert!(shows(&painted, text), "{text} missing: {painted:?}");
    }
    h.click("Cancel")?;
    for _ in 0..5 {
        h.frame(vec![]);
    }
    assert!(!h.closed(), "Cancel closed the window");
    assert!(!shows(&h.shown(), TITLE), "the dialog stayed");
    assert!(h.files.dirty());
    assert_eq!(h.engine.revision(), revision);
    assert_eq!(h.engine.project().name, "Sensor board");
    assert_eq!(h.recoverable()?, ["Sensor board"]);
    // Asking again asks again.
    h.files.request(Action::Close);
    assert!(shows(&h.shown(), TITLE));
    Ok(())
}

/// Don't Save closes without saving and keeps no recovery copy, as in
/// Solidcraft: the next launch offers nothing to restore.
#[test]
fn dont_save_closes_and_removes_the_recovery_copy() -> Fallible {
    let mut h = Harness::new()?;
    h.edit("Sensor board")?;
    h.snapshot("Sensor board")?;
    h.files.request(Action::Close);
    h.click(DONT_SAVE)?;
    h.until("the close", Harness::closed)?;
    // Frames still run while the window closes; none brings a snapshot back.
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(5));
        h.frame(vec![]);
    }
    assert!(h.recoverable()?.is_empty());
    // The exit that follows keeps nothing either.
    assert_eq!(h.files.keep_for_recovery(&mut |_| {}), Ok(false));
    assert!(h.recoverable()?.is_empty());
    Ok(())
}

/// Save writes the project to its file, then closes.
#[test]
fn save_writes_the_file_then_closes() -> Fallible {
    let mut h = Harness::new()?;
    let path = h.dir.path().join("board.sparksmith");
    h.files.adopt(&h.engine, Some(path.clone()), None, false)?;
    h.edit("Saved on quit")?;
    h.files.request(Action::Close);
    h.click("Save")?;
    h.until("the close", Harness::closed)?;
    let saved = sparksmith_io::read_project(&path)?;
    assert_eq!(saved.project.name, "Saved on quit");
    assert!(!h.files.dirty());
    Ok(())
}

/// Quitting while a save is being written waits for it, then quits; it is
/// neither lost nor answered twice.
#[test]
fn quitting_during_a_save_quits_once_it_is_written() -> Fallible {
    let mut h = Harness::new()?;
    let path = h.dir.path().join("board.sparksmith");
    h.files.adopt(&h.engine, Some(path.clone()), None, false)?;
    h.edit("Written first")?;
    h.files.request(Action::Save);
    assert!(h.files.busy());
    h.files.request(Action::Close);
    h.until("the close", Harness::closed)?;
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "Written first"
    );
    Ok(())
}

/// What the app does with a first save's one effect; whether the save was
/// current.
fn take_first_save(h: &mut Harness) -> Fallible<bool> {
    let given = h
        .effects
        .iter()
        .position(|e| matches!(e, Effect::FirstSave { .. }));
    let Effect::FirstSave { given, saved } = h.effects.remove(given.ok_or("no first save")?) else {
        return Err("not a first save".into());
    };
    h.engine.adopt_first_save(&given);
    h.files.first_saved(&h.engine, saved);
    Ok(saved)
}

/// E2E run 4, finding 7: the first save of an Untitled project names it
/// after the file the save panel chose. The file holds that name and the
/// engine takes it. Saving changes neither undo nor redo (founder, 24
/// November 2026): no revision, no step, redo kept, and no undo or redo
/// takes the name back. A later save keeps the name.
#[test]
fn a_first_save_names_an_untitled_project_after_its_file() -> Fallible {
    let mut h = Harness::new()?;
    assert_eq!(h.engine.project().name, sparksmith_core::UNTITLED);
    let path = h.dir.path().join("usb-led-e2e.sparksmith");
    let titled = |title: &str| {
        Operation::Schematic(SchematicCommand::SetProperties {
            properties: sparksmith_core::model::ProjectProperties {
                title: title.into(),
                ..Default::default()
            },
        })
    };
    for title in ["One", "Two"] {
        h.engine.apply(h.engine.revision(), titled(title))?;
        h.frame(vec![]);
    }
    h.engine.undo()?;
    h.frame(vec![]);
    let revision = h.engine.revision();
    let steps = (h.engine.undo_step(), h.engine.redo_step());
    assert!(steps.0.is_some() && steps.1.is_some());
    // The save panel's answer, as `pick` sends it.
    let ticket = h.files.begin(Kind::Save, None).ok_or("no save started")?;
    h.files
        .event_sender
        .send(Event::Picked(ticket, Kind::Save, Some(path.clone())))?;
    h.until("the write", |h| !h.files.busy())?;
    assert!(take_first_save(&mut h)?, "the save was current");
    let file = sparksmith_io::read_project(&path)?;
    assert_eq!(file.project.name, "usb-led-e2e");
    assert_eq!(h.engine.to_file(), file, "the engine holds what it wrote");
    h.frame(vec![]);
    assert_eq!(h.engine.project().name, "usb-led-e2e");
    assert!(!h.files.dirty(), "the renamed project is the saved one");
    assert_eq!(h.files.path(), Some(path.as_path()));
    assert_eq!(h.engine.revision(), revision);
    assert_eq!((h.engine.undo_step(), h.engine.redo_step()), steps);
    h.until("the recovery copy cleared", |h| {
        h.recovery_files().is_ok_and(|files| files.is_empty())
    })?;
    h.engine.redo()?;
    h.frame(vec![]);
    assert_eq!(h.engine.project().properties.title, "Two");
    assert_eq!(h.engine.project().name, "usb-led-e2e");
    assert!(h.files.dirty());
    h.engine.undo()?;
    h.frame(vec![]);
    assert!(!h.files.dirty(), "undo returns to the saved content");
    h.engine.undo()?;
    h.frame(vec![]);
    assert_eq!(h.engine.project().name, "usb-led-e2e");
    assert!(h.files.dirty());
    h.engine.redo()?;
    h.frame(vec![]);
    assert!(!h.files.dirty(), "redo returns to the saved content");

    // Saving again under the file's path renames nothing.
    h.edit("Chosen name")?;
    h.files.request(Action::Save);
    h.until("the second write", |h| !h.files.busy())?;
    assert!(
        !h.effects
            .iter()
            .any(|e| matches!(e, Effect::FirstSave { .. }))
    );
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "Chosen name"
    );
    Ok(())
}

/// E2E run 6, finding 6: a project file written while the project was
/// still Untitled (an import, or a first save that did not finish) takes
/// its file's name at the next ⌘S; a name someone chose is kept.
#[test]
fn saving_an_untitled_project_under_its_file_names_it() -> Fallible {
    let mut h = Harness::new()?;
    let path = h.dir.path().join("blinkpad-imported.sparksmith");
    assert_eq!(h.engine.project().name, sparksmith_core::UNTITLED);
    // Opened from its file, as the app opens an imported project.
    h.files.adopt(&h.engine, Some(path.clone()), None, false)?;
    h.files.request(Action::Save);
    h.until("the write", |h| !h.files.busy())?;
    assert!(take_first_save(&mut h)?, "the save was current");
    assert_eq!(h.engine.project().name, "blinkpad-imported");
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "blinkpad-imported"
    );

    // A chosen name is not replaced by the file's.
    h.edit("My blinker")?;
    h.files.request(Action::Save);
    h.until("the second write", |h| !h.files.busy())?;
    assert!(
        !h.effects
            .iter()
            .any(|e| matches!(e, Effect::FirstSave { .. }))
    );
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "My blinker"
    );
    Ok(())
}

/// A save refused because another program has the file open: the project
/// stays unsaved, the dialog says so with the copy's name, Cancel leaves
/// the question it answered open, and Try again saves and removes the copy.
#[test]
fn a_save_another_program_blocks_is_tried_again() -> Fallible {
    let mut h = Harness::new()?;
    let path = h.dir.path().join("blocked.sparksmith");
    let copy = h.dir.path().join("blocked (unsaved 1).sparksmith");
    h.files.adopt(&h.engine, Some(path.clone()), None, false)?;
    h.edit("Blocked")?;
    sparksmith_io::write_project(&copy, &h.engine.to_file())?;
    let kept = sparksmith_io::UnsavedCopy {
        identity: sparksmith_io::open_project(&copy)?.1,
        path: copy.clone(),
    };
    let refuse = |h: &mut Harness| -> Fallible {
        let ticket = h
            .files
            .begin(Kind::Save, Some(Transition::New))
            .ok_or("no save started")?;
        let in_use = sparksmith_io::InUse {
            path: path.clone(),
            cause: "in use".into(),
            copy: Some(kept.clone()),
            copy_problem: None,
        };
        h.files.event_sender.send(Event::Written(
            ticket,
            path.clone(),
            Ok(Wrote::InUse(Box::new(in_use))),
        ))?;
        Ok(())
    };
    refuse(&mut h)?;
    let painted = h.shown();
    assert!(
        painted
            .iter()
            .any(|(text, _)| text.contains("blocked (unsaved 1).sparksmith")),
        "{painted:?}"
    );
    assert!(h.files.dirty() && h.files.prompting());
    h.click("Cancel")?;
    assert!(h.files.in_use.is_none());
    assert_eq!(h.files.controller.prompt, Some(Transition::New));
    h.files.controller.prompt = None;

    refuse(&mut h)?;
    h.click("Try again")?;
    h.until("the write", |h| !h.files.busy())?;
    assert!(h.files.in_use.is_none());
    assert_eq!(sparksmith_io::read_project(&path)?.project.name, "Blocked");
    assert!(!copy.exists());
    assert_eq!(h.files.controller.unsaved, None);
    Ok(())
}

/// E2E run 6, finding 4: a save into a folder whose sync the system
/// refuses (macOS Documents; here a folder that may be written but not
/// opened) is a save: the project is no longer unsaved, the status says
/// Saved, and the recovery snapshot of the edit is cleared, so the next
/// launch does not offer to recover it.
#[cfg(unix)]
#[test]
fn a_save_into_a_folder_that_refuses_its_sync_is_saved() -> Fallible {
    use std::os::unix::fs::PermissionsExt;
    let mut h = Harness::new()?;
    let folder = h.dir.path().join("documents");
    std::fs::create_dir(&folder)?;
    let path = folder.join("blinker.sparksmith");
    h.files.adopt(&h.engine, Some(path.clone()), None, false)?;
    h.edit("Protected")?;
    h.snapshot("Protected")?;
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o300))?;
    h.files.request(Action::Save);
    let saved = h.until("the write", |h| !h.files.busy());
    let status = h.files.take_status();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
    saved?;
    assert!(!h.files.dirty(), "{status:?}");
    assert!(
        matches!(&status, Some((Level::Ok, text)) if text.starts_with("Saved")),
        "{status:?}"
    );
    h.until("the recovery cleared", |h| {
        h.recoverable().is_ok_and(|names| names.is_empty())
    })?;
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "Protected"
    );
    Ok(())
}

/// R7-4: a save says it runs from the request until its file is written,
/// so one the system holds (a permission prompt) is not silent.
#[test]
fn a_save_says_it_runs_until_it_is_written() -> Fallible {
    let mut h = Harness::new()?;
    let path = h.dir.path().join("board.sparksmith");
    h.files.adopt(&h.engine, Some(path.clone()), None, false)?;
    assert!(!h.files.saving());
    h.edit("Saving")?;
    h.files.request(Action::Save);
    assert!(h.files.saving());
    // E2E run 10, finding 4: a save the system holds past a few seconds
    // says what it may wait for. The worker's answer is not read yet, so
    // the save is still running at these times.
    let (text, wake) = h.files.activity().ok_or("no activity while saving")?;
    assert_eq!(text, "Saving\u{2026}");
    assert!(wake.is_some_and(|w| w <= Duration::from_secs(3)));
    let folder = h.dir.path().file_name().ok_or("no folder name")?;
    h.files.observe(&h.engine, 3.5);
    let (text, wake) = h.files.activity().ok_or("no activity while saving")?;
    assert_eq!(
        text,
        format!(
            "Saving\u{2026} waiting for access to {}: check for a system permission prompt",
            folder.to_string_lossy()
        )
    );
    assert_eq!(wake, None);
    h.until("the write", |h| !h.files.saving())?;
    assert_eq!(h.files.activity(), None);
    assert!(!h.files.dirty());
    assert_eq!(sparksmith_io::read_project(&path)?.project.name, "Saving");
    Ok(())
}

/// The system ending the app without asking (Dock › Quit on macOS, a
/// logout) keeps the latest edit for recovery, even one made less than
/// the snapshot delay before.
#[test]
fn an_unanswered_exit_keeps_the_latest_edit_for_recovery() -> Fallible {
    let mut h = Harness::new()?;
    h.edit("First")?;
    h.snapshot("First")?;
    h.edit("Latest")?;
    assert_eq!(h.files.keep_for_recovery(&mut |_| {}), Ok(true));
    assert_eq!(h.recoverable()?, ["Latest"]);
    Ok(())
}

/// The exit waits for a snapshot being written, however long, says so, and
/// then holds the latest edit; what the file worker still had to do does
/// not change it any more.
#[test]
fn an_exit_waits_for_a_held_snapshot_and_has_the_last_word() -> Fallible {
    let mut h = Harness::new()?;
    h.edit("First")?;
    h.snapshot("First")?;
    h.edit("Latest")?;
    let recovery = Arc::clone(&h.files.recovery);
    let (held, holding) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let store = recovery.lock();
        let _ = held.send(());
        let _ = released.recv();
        drop(store);
    });
    holding.recv()?;
    // Queued before the exit, run after it: an older snapshot and a removal.
    let ticket = h.files.controller.allocate()?;
    let older = Engine::demo()?.document().clone();
    h.files.send(Job::Recover(ticket.clone(), older, None));
    h.files.send(Job::ClearRecovery(ticket, false));
    let mut said = Vec::new();
    let kept = h.files.keep_for_recovery(&mut |text| {
        said.push(text.to_owned());
        let _ = release.send(());
    });
    holder.join().map_err(|_| "the holder panicked")?;
    // The worker may have run the queued jobs before this write or after.
    assert_eq!(kept, Ok(true));
    assert_eq!(said, ["Waiting for the recovery copy being written"]);
    let mut done = 0;
    while done < 2 {
        if matches!(h.files.receiver.recv()?, Event::Recovered(..)) {
            done += 1;
        }
    }
    assert_eq!(h.recoverable()?, ["Latest"]);
    Ok(())
}

/// A snapshot that cannot be written is an error with its reason, never a
/// silent loss.
#[test]
fn an_exit_that_cannot_keep_the_work_says_why() -> Fallible {
    let mut h = Harness::new()?;
    h.until("the recovery folder", |h| {
        h.dir.path().join("recovery").is_dir()
    })?;
    h.edit("Lost")?;
    std::fs::remove_dir_all(h.dir.path().join("recovery"))?;
    let kept = h.files.keep_for_recovery(&mut |_| {});
    assert!(kept.as_ref().is_err_and(|why| !why.is_empty()), "{kept:?}");
    assert!(h.files.dirty());
    Ok(())
}

/// A running import is stopped by the exit, which keeps the work without
/// waiting for it.
#[test]
fn an_exit_stops_a_running_import() -> Fallible {
    let mut h = Harness::new()?;
    h.edit("Kept")?;
    let board = h.dir.path().join("demo.kicad_pcb");
    std::fs::write(&board, KICAD_BOARD)?;
    let ticket = h.files.begin(Kind::Import, None).ok_or("no import")?;
    let cancel = Cancel::default();
    h.files.send(Job::Import(ticket, board, cancel.clone()));
    assert_eq!(h.files.keep_for_recovery(&mut |_| {}), Ok(true));
    assert!(cancel.is_set());
    assert_eq!(h.recoverable()?, ["Kept"]);
    Ok(())
}

/// Undo back to the saved content clears the unsaved state; redo away from
/// it sets it again. Nothing is asked on quit at the saved content.
#[test]
fn undo_to_the_saved_content_is_not_unsaved() -> Fallible {
    let mut h = Harness::new()?;
    let path = h.dir.path().join("board.sparksmith");
    h.files.adopt(&h.engine, Some(path), None, false)?;
    h.edit("Edited")?;
    h.engine.undo()?;
    h.frame(vec![]);
    assert!(!h.files.dirty(), "undo to the saved content");
    h.engine.redo()?;
    h.frame(vec![]);
    assert!(h.files.dirty(), "redo away from it");
    h.engine.undo()?;
    h.frame(vec![]);
    h.files.request(Action::Close);
    assert!(!shows(&h.shown(), TITLE));
    h.until("the close", Harness::closed)
}

/// Precision run finding 3: opening a published component for editing
/// drafts it, but the project is not unsaved until the draft is edited.
#[test]
fn an_unedited_draft_is_not_unsaved() -> Fallible {
    let mut h = Harness::new()?;
    // Start from a saved project without drafts.
    let ids: Vec<_> = h.engine.drafts().items.iter().map(|d| d.id).collect();
    for id in ids {
        h.engine.apply(
            h.engine.revision(),
            Operation::Library(sparksmith_core::LibraryCommand::DiscardDraft { id }),
        )?;
    }
    let path = h.dir.path().join("board.sparksmith");
    h.files.adopt(&h.engine, Some(path), None, false)?;
    h.frame(vec![]);
    assert!(!h.files.dirty());
    let published = h
        .engine
        .project()
        .library
        .latest_components()
        .into_iter()
        .find(|c| h.engine.drafts().get(c.id).is_none())
        .map(|c| sparksmith_core::ComponentRef::new(c.id, c.revision))
        .ok_or("a published component without a draft")?;
    let before = h.engine.revision();
    crate::part_editor::run(&mut h.engine, crate::part_editor::Request::Draft(published))?;
    assert!(h.engine.revision() > before, "the draft was created");
    h.frame(vec![]);
    assert!(!h.files.dirty(), "an unedited draft is not unsaved work");
    let mut draft = h
        .engine
        .drafts()
        .get(published.id)
        .cloned()
        .ok_or("no draft")?;
    draft.meta.name.push_str(" edited");
    h.engine.apply(
        h.engine.revision(),
        Operation::Library(sparksmith_core::LibraryCommand::Edit {
            edits: vec![sparksmith_core::library::DraftEdit {
                id: draft.id,
                meta: draft.meta,
                definition: draft.definition,
            }],
        }),
    )?;
    h.frame(vec![]);
    assert!(h.files.dirty(), "an edited draft is");
    Ok(())
}

/// A clean quit leaves no file of this session behind, not even the empty
/// reservation made at launch: after a project was opened, with Don't
/// Save, and with nothing to save.
#[test]
fn a_clean_quit_leaves_no_recovery_file() -> Fallible {
    for edit in [false, true] {
        let mut h = Harness::new()?;
        // The worker's store is ready (it makes the folder) at launch.
        h.until("the recovery folder", |h| {
            h.dir.path().join("recovery").is_dir()
        })?;
        assert_eq!(h.recovery_files()?, Vec::<PathBuf>::new(), "at launch");
        let path = h.dir.path().join("board.sparksmith");
        h.files.adopt(&h.engine, Some(path), None, false)?;
        h.frame(vec![]);
        if edit {
            h.edit("Discarded")?;
        }
        h.files.request(Action::Close);
        if edit {
            h.click(DONT_SAVE)?;
        }
        h.until("the close", Harness::closed)?;
        assert_eq!(h.files.keep_for_recovery(&mut |_| {}), Ok(false));
        assert_eq!(h.recovery_files()?, Vec::<PathBuf>::new(), "edit: {edit}");
    }
    Ok(())
}

/// Without a recovery folder (no home, a relative override) the app says
/// at startup that recovery is off, rather than keeping snapshots in a
/// temporary folder the system may empty.
#[test]
fn a_missing_recovery_folder_is_reported_at_startup() -> Fallible {
    let ctx = egui::Context::default();
    let engine = Engine::demo()?;
    let reason = sparksmith_io::settings::UserDirs::recovery_from_vars(|_| None)
        .err()
        .map(|e| e.to_string())
        .ok_or("no variables still found a recovery folder")?;
    let files = FileUi::with_recovery(&ctx, rfd::AsyncFileDialog::new(), &engine, Err(reason));
    let mut h = Harness {
        ctx,
        files,
        engine,
        dir: tempfile::tempdir()?,
        now: 0.0,
        effects: Vec::new(),
    };
    h.until("the startup report", |h| h.files.status.is_some())?;
    let (level, text) = h.files.take_status().ok_or("no status")?;
    assert_eq!(level, Level::Error);
    assert!(
        text.starts_with("Recovery unavailable: no recovery directory")
            && text.contains("not kept"),
        "{text}"
    );
    assert!(h.files.recoveries.is_empty());
    Ok(())
}

/// More sessions than a listing reads: the newest are offered, the
/// question says how many older ones it left out, and one button discards
/// exactly those.
#[test]
fn older_sessions_past_the_limit_are_named_and_discardable() -> Fallible {
    let dir = tempfile::tempdir()?;
    let folder = dir.path().join("recovery");
    let mut store = RecoveryStore::new(&folder)?;
    store.write(&EditorFile::from_document(Engine::demo()?.document()), None)?;
    let first = store.path().ok_or("no snapshot")?.to_path_buf();
    let base = std::time::SystemTime::now() - Duration::from_secs(100_000);
    let mut paths = Vec::new();
    for n in 0..recovery::MAX_ENTRIES + 3 {
        let path = folder.join(format!("session-n{n:04}.sparksmith-recovery"));
        std::fs::copy(&first, &path)?;
        std::fs::File::options()
            .write(true)
            .open(&path)?
            .set_modified(base + Duration::from_secs(n as u64 * 10))?;
        paths.push(path);
    }
    store.remove_own()?;
    let mut h = Harness::in_dir(dir)?;
    h.until("the recovery question", |h| !h.files.recoveries.is_empty())?;
    assert_eq!(h.files.recoveries.len(), recovery::MAX_ENTRIES);
    assert_eq!(h.files.older_recoveries.len(), 3);
    assert!(shows(&h.shown(), &older_sessions_note(3)));
    h.click(DISCARD_OLDER)?;
    h.until("the older sessions discarded", |h| {
        h.files.older_recoveries.is_empty() && !h.files.recovery_choice_pending
    })?;
    for path in &paths[..3] {
        assert!(!path.exists(), "{}", path.display());
    }
    assert!(paths[3..].iter().all(|p| p.exists()));
    assert_eq!(
        h.files.recoveries.len(),
        recovery::MAX_ENTRIES,
        "still offered"
    );
    Ok(())
}

/// Review finding 9 in the window: another window (or the CLI) saved the
/// project after it was opened here. Save keeps both versions: the file
/// as the other writer left it, and this window's document in a new file
/// beside it, which is the window's file from then on.
#[test]
fn a_save_after_another_writer_keeps_both_versions() -> Fallible {
    let mut h = Harness::new()?;
    let path = h.dir.path().join("board.sparksmith");
    sparksmith_io::write_project(&path, &h.engine.to_file())?;
    let (_, identity) = sparksmith_io::open_project(&path)?;
    h.files
        .adopt(&h.engine, Some(path.clone()), Some(identity), false)?;
    let mut other = Engine::demo()?;
    other.apply(
        other.revision(),
        Operation::Schematic(SchematicCommand::RenameProject {
            name: "Other window".into(),
        }),
    )?;
    sparksmith_io::write_project(&path, &other.to_file())?;
    h.edit("This window")?;
    h.files.request(Action::Save);
    h.until("the save", |h| !h.files.busy())?;
    let kept = h.dir.path().join("board (conflict 1).sparksmith");
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "Other window"
    );
    assert_eq!(
        sparksmith_io::read_project(&kept)?.project.name,
        "This window"
    );
    assert_eq!(h.files.path(), Some(kept.as_path()));
    assert!(!h.files.dirty());
    let status = h.files.take_status();
    assert!(
        matches!(&status, Some((Level::Warning, text)) if text.contains("changed on disk")),
        "{status:?}"
    );
    // The next save goes to the kept file and leaves the other alone.
    h.edit("This window, later")?;
    h.files.request(Action::Save);
    h.until("the second save", |h| !h.files.busy())?;
    assert_eq!(
        sparksmith_io::read_project(&kept)?.project.name,
        "This window, later"
    );
    assert_eq!(
        sparksmith_io::read_project(&path)?.project.name,
        "Other window"
    );
    Ok(())
}

/// Crash recovery cadence: edits every half second never leave two quiet
/// seconds, yet a snapshot is taken once the maximum interval since the
/// first unkept edit has passed. The clock is the test's.
#[test]
fn continuous_editing_is_snapshotted_within_the_maximum_interval() -> Fallible {
    let mut h = Harness::new()?;
    let mut n = 0;
    while h.now < MAX_SNAPSHOT_INTERVAL - 0.5 {
        h.now += 0.5;
        n += 1;
        h.edit(&format!("Edit {n}"))?;
        assert!(
            h.recovery_files()?.is_empty(),
            "no snapshot yet at {}",
            h.now
        );
    }
    h.now += 1.0;
    n += 1;
    h.edit(&format!("Edit {n}"))?;
    let name = format!("Edit {n}");
    // The clock stands still now: only the maximum interval can be due.
    h.until("a snapshot during continuous editing", |h| {
        h.recoverable().is_ok_and(|names| names == [name.as_str()])
    })?;
    Ok(())
}

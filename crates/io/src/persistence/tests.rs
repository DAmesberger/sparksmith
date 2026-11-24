use super::*;
use sparksmith_core::{
    DefinitionKind, Engine, InstanceCommand, LibraryCommand, Operation,
    library::{Definition, DefinitionMeta},
};

/// An empty component draft and the demo binding draft emptied of mappings.
pub(crate) fn unfinished(engine: &mut Engine) -> Result<()> {
    engine.apply(
        engine.revision(),
        Operation::Library(LibraryCommand::NewDraft {
            meta: DefinitionMeta::default(),
            definition: Definition::empty(DefinitionKind::Component),
        }),
    )?;
    let mut binding = engine
        .drafts()
        .items
        .iter()
        .find(|d| d.definition.kind() == DefinitionKind::Binding)
        .cloned()
        .ok_or_else(|| Error::Invalid("demo binding draft".into()))?;
    if let Definition::Binding(b) = &mut binding.definition {
        b.mappings.clear();
    }
    engine.apply(
        engine.revision(),
        Operation::Library(LibraryCommand::Edit {
            edits: vec![sparksmith_core::library::DraftEdit {
                id: binding.id,
                meta: binding.meta,
                definition: binding.definition,
            }],
        }),
    )?;
    Ok(())
}

#[test]
fn saved_project_reopens_offline_with_exact_revisions_and_drafts() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("offline.sparksmith");
    let mut engine = Engine::demo()?;
    unfinished(&mut engine)?;
    write_project(&path, &engine.to_file())?;
    let reopened = Engine::from_file(read_project(&path)?)?;
    assert_eq!(reopened.to_file(), engine.to_file());
    assert!(!reopened.can_undo());
    Ok(())
}

/// The golden project (`tests/fixtures/format/demo.sparksmith`: the demo
/// design, its library and open drafts, at [`format::FILE_VERSION`]) opens
/// and saves byte for byte. A stored-type change regenerates it.
#[test]
fn the_golden_project_opens_and_saves_byte_for_byte() -> Result<()> {
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format/demo.sparksmith");
    let file = read_project(&golden)?;
    assert_eq!(file.version, format::FILE_VERSION);
    Engine::from_file(file.clone())?;
    let dir = tempfile::tempdir()?;
    let saved = dir.path().join("saved.sparksmith");
    write_project(&saved, &file)?;
    assert_eq!(std::fs::read(&saved)?, std::fs::read(&golden)?);
    Ok(())
}

/// Only a `.sparksmith` file name, in any letter case, is a project.
#[test]
fn other_extensions_are_refused() {
    let dir = Path::new("/nonexistent");
    for refused in ["board.json", "board.pcbcraft", "board"] {
        assert!(read_project(&dir.join(refused)).is_err(), "{refused}");
        assert!(check_project_path(&dir.join(refused)).is_err(), "{refused}");
    }
    assert!(check_project_path(&dir.join("board.sparksmith")).is_ok());
}

/// A name the platform recognises as a project (any letter case, as
/// Finder matches it) is one persistence opens and saves: one predicate.
#[test]
fn an_upper_case_extension_opens_and_saves() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("BOARD.SPARKSMITH");
    assert!(sparksmith_core::format::is_project_path(&path));
    let file = Engine::demo()?.to_file();
    write_project(&path, &file)?;
    assert_eq!(read_project(&path)?, file);
    Ok(())
}

#[test]
fn invalid_files_are_refused_without_side_effects() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("bad.sparksmith");
    let mut engine = Engine::demo()?;
    let r1 = engine.project().instances[0].id;
    engine.apply(
        engine.revision(),
        Operation::Instance(InstanceCommand::SetDesignator {
            instance: r1,
            designator: "R10".into(),
        }),
    )?;
    let before = engine.to_file();
    let mut newer = serde_json::to_value(&before)?;
    newer["version"] = serde_json::json!(999);
    for bytes in [b"not json".to_vec(), serde_json::to_vec(&newer)?] {
        std::fs::write(&path, bytes)?;
        assert!(read_project(&path).is_err());
    }
    // A container whose document is of a newer version.
    container::tests::raw_zip(
        &path,
        &[(
            format::CONTAINER_DOCUMENT,
            &serde_json::to_vec(&newer)?,
            zip::CompressionMethod::Deflated,
        )],
    )?;
    assert!(matches!(
        read_project(&path),
        Err(Error::Document(sparksmith_core::Error::Unsupported(_)))
    ));
    let large = std::fs::File::create(&path)?;
    large.set_len(format::CONTAINER_MAX_BYTES + 1)?;
    assert!(matches!(read_project(&path), Err(Error::TooLarge(_))));
    assert!(read_project(&dir.path().join("wrong.json")).is_err());
    assert_eq!(engine.to_file(), before);
    Ok(())
}

/// Both writers opened the original before the first rename. The second
/// writer must retry with the named file before it compares content hashes.
#[cfg(any(windows, unix))]
#[test]
fn a_writer_holding_the_original_handle_detects_the_other_writers_rename() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("shared.sparksmith");
    let original = Engine::demo()?.to_file();
    let disk = save_project(&path, &original, None)?;
    let first = File::open(&path)?;
    let second = File::open(&path)?;
    assert!(still_at(&first, &path)? && still_at(&second, &path)?);
    let mut edited = original.clone();
    edited.project.name = "First writer".into();
    save_project(&path, &edited, Some(disk))?;
    second.lock()?;
    assert!(!still_at(&second, &path)?);
    drop(second);
    let Err(Error::Conflict(conflict)) = save_project(&path, &original, Some(disk)) else {
        return Err(Error::Invalid("expected a preserved conflict copy".into()));
    };
    assert_eq!(read_project(&path)?, edited);
    assert_eq!(read_project(&conflict.kept)?, original);
    Ok(())
}

#[test]
fn failed_saves_preserve_existing_target_and_leave_no_temporary_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let engine = Engine::demo()?;
    let valid = engine.to_file();
    let path = dir.path().join("existing.sparksmith");
    write_project(&path, &valid)?;
    let original = std::fs::read(&path)?;
    let mut invalid = valid.clone();
    invalid.project.name = String::new();
    assert!(write_project(&path, &invalid).is_err());
    assert!(atomic_write(&path, &vec![b' '; MAX_FILE_BYTES + 1]).is_err());
    assert_eq!(std::fs::read(&path)?, original);
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);

    let occupied = dir.path().join("occupied.sparksmith");
    std::fs::create_dir(&occupied)?;
    std::fs::write(occupied.join("sentinel"), b"preserve")?;
    assert!(write_project(&occupied, &valid).is_err());
    assert_eq!(std::fs::read(occupied.join("sentinel"))?, b"preserve");
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 2);
    Ok(())
}

#[test]
fn atomic_create_never_replaces_an_existing_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("pins.h");
    atomic_create(&path, b"first")?;
    assert_eq!(std::fs::read(&path)?, b"first");
    let again = atomic_create(&path, b"second");
    assert!(
        matches!(&again, Err(crate::Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists),
        "{again:?}"
    );
    assert_eq!(std::fs::read(&path)?, b"first");
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

#[cfg(unix)]
fn mode(path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt;
    Ok(std::fs::metadata(path)?.permissions().mode() & 0o777)
}

/// A new file gets the permissions any program's new file gets (0666 less
/// the umask, as `File::create` in the same folder), not tempfile's 0600.
#[cfg(unix)]
#[test]
fn a_new_file_gets_the_usual_permissions() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let reference = dir.path().join("reference.txt");
    std::fs::File::create(&reference)?;
    let written = dir.path().join("written.txt");
    atomic_write(&written, b"new")?;
    let created = dir.path().join("created.txt");
    atomic_create(&created, b"new")?;
    let project = dir.path().join("board.sparksmith");
    write_project(&project, &Engine::demo()?.to_file())?;
    assert_eq!(mode(&written)?, mode(&reference)?);
    assert_eq!(mode(&created)?, mode(&reference)?);
    assert_eq!(mode(&project)?, mode(&reference)?);
    Ok(())
}

/// Replacing a file keeps its permissions, wider or narrower than usual.
#[cfg(unix)]
#[test]
fn a_replaced_file_keeps_its_permissions() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    for kept in [0o640, 0o664, 0o600] {
        let path = dir.path().join(format!("board-{kept:o}.sparksmith"));
        write_project(&path, &Engine::demo()?.to_file())?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(kept))?;
        write_project(&path, &Engine::demo()?.to_file())?;
        assert_eq!(mode(&path)?, kept);
        atomic_write(&path, b"plain")?;
        assert_eq!(mode(&path)?, kept);
    }
    Ok(())
}

/// E2E run 6, finding 4: a folder the system will not let us open for its
/// sync (here: one that may be written but not read, as macOS answers in
/// Documents) does not fail the save. The file was synced and is in place,
/// so a project save succeeds and reads back.
#[cfg(unix)]
#[test]
fn a_folder_that_refuses_its_sync_still_saves() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let folder = dir.path().join("out");
    std::fs::create_dir(&folder)?;
    let path = folder.join("board.sparksmith");
    let file = Engine::demo()?.to_file();
    write_project(&path, &file)?;
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o300))?;
    let refused = std::fs::File::open(&folder).is_err();
    let result = write_project(&path, &file);
    let plain = atomic_write(&folder.join("note.txt"), b"new");
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
    // A privileged user opens it anyway; the save succeeds either way.
    assert!(refused || std::fs::File::open(&folder).is_ok());
    result?;
    plain?;
    assert_eq!(read_project(&path)?.project, file.project);
    assert_eq!(std::fs::read(folder.join("note.txt"))?, b"new");
    Ok(())
}

/// Which folder-sync failures mean "not offered here" (no notice) and
/// which still say the file was written but its folder is unsynced.
#[cfg(unix)]
#[test]
fn only_a_sync_the_system_does_not_offer_is_quiet() {
    use std::io::{Error as IoError, ErrorKind};
    for kind in [
        ErrorKind::PermissionDenied,
        ErrorKind::Unsupported,
        ErrorKind::InvalidInput,
    ] {
        assert!(super::sync_not_offered(&IoError::from(kind)), "{kind:?}");
    }
    // EPERM, as macOS answers for a protected folder.
    assert!(super::sync_not_offered(&IoError::from_raw_os_error(1)));
    for kind in [
        ErrorKind::Other,
        ErrorKind::StorageFull,
        ErrorKind::NotFound,
    ] {
        assert!(!super::sync_not_offered(&IoError::from(kind)), "{kind:?}");
    }
}

#[test]
fn a_competing_creator_wins_after_the_temporary_file_is_written() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("output.bin");
    let result = atomic_create_with(&path, |file| {
        file.write_all(b"losing content")?;
        std::fs::write(&path, b"competing content")?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists));
    assert_eq!(std::fs::read(&path)?, b"competing content");
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

#[test]
fn two_concurrent_creators_publish_exactly_one_complete_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("output.bin");
    let ready = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let create = |bytes: &'static [u8]| {
            let path = &path;
            let ready = &ready;
            scope.spawn(move || {
                atomic_create_with(path, |file| {
                    file.write_all(bytes)?;
                    ready.wait();
                    Ok(())
                })
            })
        };
        let a = create(b"first complete file");
        let b = create(b"second complete file");
        (a.join(), b.join())
    });
    let (a, b) = (
        results
            .0
            .map_err(|_| Error::Invalid("creator panicked".into()))?,
        results
            .1
            .map_err(|_| Error::Invalid("creator panicked".into()))?,
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let loser = if a.is_err() { a } else { b };
    assert!(matches!(loser, Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists));
    let bytes = std::fs::read(&path)?;
    assert!(bytes == b"first complete file" || bytes == b"second complete file");
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

#[test]
fn staged_path_writers_publish_the_replaced_file_and_clean_up() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("output.solidcraft");
    let staged = StagedNew::new(&path)?;
    assert_eq!(staged.path().extension(), path.extension());
    assert!(!path.exists());
    assert!(!staged.path().exists(), "Save As must see a new path");
    std::fs::write(staged.path(), b"initial external output")?;
    atomic_write(staged.path(), b"external writer replaced the stage")?;
    staged.persist()?;
    assert_eq!(std::fs::read(&path)?, b"external writer replaced the stage");
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
    assert!(
        matches!(StagedNew::new(&path), Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists)
    );
    let abandoned = dir.path().join("abandoned.solidcraft");
    let stage = StagedNew::new(&abandoned)?;
    std::fs::write(stage.path(), b"partial")?;
    drop(stage);
    assert!(!abandoned.exists());
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

#[test]
fn staged_path_writers_refuse_a_target_created_before_publication() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("output.solidcraft");
    let staged = StagedNew::new(&path)?;
    atomic_write(staged.path(), b"native content")?;
    std::fs::write(&path, b"competing content")?;
    assert!(
        matches!(staged.persist(), Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists)
    );
    assert_eq!(std::fs::read(&path)?, b"competing content");
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

#[test]
fn creating_a_project_never_replaces_an_existing_container() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("board.sparksmith");
    let file = Engine::demo()?.to_file();
    create_project(&path, &file)?;
    assert_eq!(read_project(&path)?, file);
    let bytes = std::fs::read(&path)?;
    assert!(
        matches!(create_project(&path, &Engine::profiled()?.to_file()), Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists)
    );
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

/// Bytes past which the indented document used to be refused on save
/// (review finding 11): the reproductions below need more than this.
const OLD_SAVE_CAP: usize = 16 * 1024 * 1024;

/// The showcase project with `count` copies of one of its tracks, as a
/// document read from a file would hold them.
fn with_tracks(count: usize) -> Result<format::EditorFile> {
    let mut file = Engine::showcase()?.to_file();
    let project = &mut file.project;
    let track = project
        .tracks
        .first()
        .cloned()
        .ok_or_else(|| Error::Invalid("the showcase has a track".into()))?;
    let mut next = project.next_id;
    for _ in 0..count {
        let mut copy = track.clone();
        copy.id = next;
        next += 1;
        project.tracks.push(copy);
    }
    project.next_id = next;
    file.validate()?;
    Ok(file)
}

/// A container whose document is written compact, as another writer (or
/// an older build) may leave it: smaller than what this build writes.
fn write_compact(path: &Path, file: &format::EditorFile) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let zipped = |e: zip::result::ZipError| Error::Invalid(e.to_string());
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path)?);
    let mut stripped = file.clone();
    for asset in &mut stripped.project.library.assets {
        asset.data = Default::default();
    }
    zip.start_file(format::CONTAINER_DOCUMENT, SimpleFileOptions::default())
        .map_err(zipped)?;
    zip.write_all(&serde_json::to_vec(&stripped)?)?;
    for asset in &file.project.library.assets {
        zip.start_file(
            container::asset_name(asset.hash),
            SimpleFileOptions::default(),
        )
        .map_err(zipped)?;
        zip.write_all(&asset.data[..])?;
    }
    zip.finish().map_err(zipped)?;
    Ok(())
}

/// Read path of finding 11: a project that opens from a compact document
/// under the old cap, whose indented form is past it, saves, keeps a
/// recovery snapshot and reopens unchanged.
#[test]
fn a_project_that_opens_from_a_compact_file_saves_and_recovers() -> Result<()> {
    let file = with_tracks(70_000)?;
    let compact = serde_json::to_vec(&file)?.len();
    let indented = format::container_document_len(&file, usize::MAX)?;
    assert!(compact < OLD_SAVE_CAP, "compact {compact}");
    assert!(indented > OLD_SAVE_CAP, "indented {indented}");
    let dir = tempfile::tempdir()?;
    let opened = dir.path().join("compact.sparksmith");
    write_compact(&opened, &file)?;
    let read = read_project(&opened)?;
    let saved = dir.path().join("saved.sparksmith");
    write_project(&saved, &read)?;
    assert_eq!(read_project(&saved)?, read);
    let mut store = crate::recovery::RecoveryStore::new(&dir.path().join("recovery"))?;
    store.write(&read, Some(&opened))?;
    let snapshot = store
        .path()
        .ok_or_else(|| Error::Invalid("a snapshot path".into()))?;
    assert_eq!(crate::recovery::read(snapshot)?.document, read);
    Ok(())
}

/// Authoring path of finding 11: notes added through the engine grow the
/// saved document past the old cap; the project still saves, recovers
/// and reopens as the engine holds it.
#[test]
fn a_project_authored_past_the_old_cap_saves_and_recovers() -> Result<()> {
    use sparksmith_core::SheetItem;
    use sparksmith_core::schematic::SchematicCommand;
    use sparksmith_core::units::{Nm, PointNm};
    let mut engine = Engine::demo()?;
    // Quotes are escaped when saved: each note writes about 8 KiB.
    let text = format!("{}a", "\"".repeat(4095));
    let note = |n: usize| -> Result<Operation> {
        let mm = |v: i64| Nm::try_new(v * 1_000_000).ok_or_else(|| Error::Invalid("nm".into()));
        Ok(Operation::Schematic(SchematicCommand::AddDrawing {
            sheet: None,
            item: SheetItem::Text {
                at: PointNm {
                    x_nm: mm((n % 200) as i64)?,
                    y_nm: mm((n / 200) as i64)?,
                },
                text: text.clone(),
                height_nm: mm(2)?,
                rotation: Default::default(),
            },
        }))
    };
    for batch in 0..9 {
        let operations = (0..sparksmith_core::MAX_BATCH)
            .map(|n| note(batch * sparksmith_core::MAX_BATCH + n))
            .collect::<Result<Vec<_>>>()?;
        engine.apply_all_hosted(engine.revision(), operations, Default::default())?;
    }
    let file = engine.to_file();
    let indented = format::container_document_len(&file, usize::MAX)?;
    assert!(indented > OLD_SAVE_CAP, "indented {indented}");
    let dir = tempfile::tempdir()?;
    let saved = dir.path().join("authored.sparksmith");
    write_project(&saved, &file)?;
    assert_eq!(Engine::from_file(read_project(&saved)?)?.to_file(), file);
    let mut store = crate::recovery::RecoveryStore::new(&dir.path().join("recovery"))?;
    store.write(&file, None)?;
    let snapshot = store
        .path()
        .ok_or_else(|| Error::Invalid("a snapshot path".into()))?;
    assert_eq!(
        Engine::from_file(crate::recovery::read(snapshot)?.document)?.to_file(),
        file
    );
    Ok(())
}

/// A document the writer could not write again is refused when read,
/// naming why, so no project opens that cannot be saved.
#[test]
fn a_project_too_large_to_save_again_is_refused_at_open() -> Result<()> {
    let file = with_tracks(10)?;
    // Measured as its first save writes it: with the identity it is given.
    let mut identified = file.clone();
    identified.project.production.project_id =
        Some("00000000-0000-4000-8000-000000000000".to_owned());
    let len = format::container_document_len(&identified, usize::MAX)?;
    assert!(container::admit(&file, len).is_ok());
    let refused = container::admit(&file, len - 1)
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        refused.contains("not opened because it could not be saved again"),
        "{refused}"
    );
    Ok(())
}

/// Review finding 15: a document that fits the limit only without the
/// identity its first save gives it is refused at open, as the writer
/// refuses it, so no project opens whose first save fails; with room for
/// the identity it opens.
#[test]
fn a_project_whose_first_save_would_not_fit_is_refused_at_open() -> Result<()> {
    let file = with_tracks(10)?;
    assert!(file.project.production.project_id.is_none());
    let bare = format::container_document_len(&file, usize::MAX)?;
    let mut identified = file.clone();
    identified.project.production.project_id =
        Some("00000000-0000-4000-8000-000000000000".to_owned());
    let with_identity = format::container_document_len(&identified, usize::MAX)?;
    assert!(with_identity > bare);
    let refused = container::admit(&file, bare)
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        refused.contains("not opened because it could not be saved again"),
        "{refused}"
    );
    assert!(container::admit(&file, with_identity).is_ok());
    Ok(())
}

/// Finding 9: two editors open one project. The first save replaces it;
/// the second, still holding what it opened, finds the file changed and
/// keeps both versions: the first editor's file as it saved it, and the
/// second's document in a new file beside it.
#[test]
fn a_second_save_of_one_opened_file_keeps_both_versions() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("shared.sparksmith");
    write_project(&path, &Engine::demo()?.to_file())?;
    let (_, opened_a) = open_project(&path)?;
    let (_, opened_b) = open_project(&path)?;
    assert_eq!(opened_a, opened_b);

    let mut a = Engine::demo()?;
    unfinished(&mut a)?;
    let a = a.to_file();
    let b = Engine::profiled()?.to_file();
    let saved_a = save_project(&path, &a, Some(opened_a))?;
    assert_eq!(open_project(&path)?, (a.clone(), saved_a));

    let Err(Error::Conflict(conflict)) = save_project(&path, &b, Some(opened_b)) else {
        return Err(Error::Invalid(
            "the second save must find a conflict".into(),
        ));
    };
    assert_eq!(conflict.path, path);
    assert_eq!(
        conflict.kept,
        dir.path().join("shared (conflict 1).sparksmith")
    );
    assert_eq!(open_project(&path)?, (a, saved_a), "the first save is kept");
    assert_eq!(
        open_project(&conflict.kept)?,
        (b.clone(), conflict.identity)
    );
    let message = Error::Conflict(conflict.clone()).to_string();
    assert!(
        message.contains("changed on disk since it was opened"),
        "{message}"
    );
    assert!(
        message.contains("shared (conflict 1).sparksmith"),
        "{message}"
    );
    // The kept file is the second editor's file now: its next save there
    // goes through.
    save_project(&conflict.kept, &b, Some(conflict.identity))?;
    // No temporary file is left behind.
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 2);
    Ok(())
}

/// A program that rewrites the file in place (not through these
/// writers) is noticed too: the identity is of the bytes, not the inode.
#[test]
fn a_file_rewritten_in_place_is_not_replaced() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("board.sparksmith");
    write_project(&path, &Engine::demo()?.to_file())?;
    let (_, opened) = open_project(&path)?;
    let other = dir.path().join("other.sparksmith");
    write_project(&other, &Engine::profiled()?.to_file())?;
    std::fs::write(&path, std::fs::read(&other)?)?;
    let before = std::fs::read(&path)?;
    assert!(matches!(
        save_project(&path, &Engine::demo()?.to_file(), Some(opened)),
        Err(Error::Conflict(_))
    ));
    assert_eq!(std::fs::read(&path)?, before);
    Ok(())
}

/// Two writers saving one opened file at once: the check and the
/// replacement are one step, so exactly one replaces it and the other
/// keeps its version beside it.
#[test]
fn concurrent_saves_of_one_opened_file_replace_it_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("race.sparksmith");
    write_project(&path, &Engine::demo()?.to_file())?;
    let (_, opened) = open_project(&path)?;
    let a = Engine::profiled()?.to_file();
    let mut b = Engine::demo()?;
    unfinished(&mut b)?;
    let b = b.to_file();
    for _ in 0..8 {
        write_project(&path, &Engine::demo()?.to_file())?;
        for entry in std::fs::read_dir(dir.path())? {
            let entry = entry?.path();
            if entry != path {
                std::fs::remove_file(entry)?;
            }
        }
        let ready = std::sync::Barrier::new(2);
        let (ra, rb) = std::thread::scope(|scope| {
            let save = |file: format::EditorFile| {
                let (path, ready) = (&path, &ready);
                scope.spawn(move || {
                    ready.wait();
                    save_project(path, &file, Some(opened))
                })
            };
            let (first, second) = (save(a.clone()), save(b.clone()));
            (first.join(), second.join())
        });
        let (ra, rb) = (
            ra.map_err(|_| Error::Invalid("a saver panicked".into()))?,
            rb.map_err(|_| Error::Invalid("a saver panicked".into()))?,
        );
        let replaced = usize::from(ra.is_ok()) + usize::from(rb.is_ok());
        assert_eq!(replaced, 1, "{ra:?} {rb:?}");
        let (winner, loser, kept) = match (&ra, &rb) {
            (Ok(_), Err(Error::Conflict(c))) => (&a, &b, c),
            (Err(Error::Conflict(c)), Ok(_)) => (&b, &a, c),
            other => return Err(Error::Invalid(format!("{other:?}"))),
        };
        assert_eq!(&read_project(&path)?, winner);
        assert_eq!(&read_project(&kept.kept)?, loser);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn saving_through_absolute_and_relative_links_keeps_the_link_and_both_writers() -> Result<()> {
    for relative in [true, false] {
        let dir = tempfile::tempdir()?;
        let boards = dir.path().join("boards");
        std::fs::create_dir(&boards)?;
        let target = boards.join("board.sparksmith");
        let alias = dir.path().join("alias.sparksmith");
        let link = if relative {
            PathBuf::from("boards/board.sparksmith")
        } else {
            target.clone()
        };
        let mut file = Engine::demo()?.to_file();
        write_project(&target, &file)?;
        std::os::unix::fs::symlink(&link, &alias)?;
        let (_, original) = open_project(&alias)?;
        file.project.name = "First edit".into();
        let saved = save_project(&alias, &file, Some(original))?;
        assert_eq!(std::fs::read_link(&alias)?, link);
        assert_eq!(open_project(&target)?, (file.clone(), saved));
        file.project.name = "Concurrent edit".into();
        write_project(&target, &file)?;
        let mut mine = file.clone();
        mine.project.name = "Local edit".into();
        let Err(Error::Conflict(conflict)) = save_project(&alias, &mine, Some(saved)) else {
            return Err(Error::Invalid("expected a competing edit".into()));
        };
        assert_eq!(std::fs::read_link(&alias)?, link);
        assert_eq!(read_project(&alias)?, file);
        assert_eq!(read_project(&conflict.kept)?, mine);
        assert_eq!(
            conflict.kept.parent(),
            Some(boards.canonicalize()?.as_path())
        );
    }
    Ok(())
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Result<Vec<String>> {
    let mut names = std::fs::read_dir(dir)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// A set of new files is published whole or not at all: whichever file is
/// taken meanwhile, the ones already placed are removed again, the taken
/// one is untouched and no staging folder stays. A set dropped before it
/// is published leaves nothing either.
#[test]
fn a_set_of_new_files_is_published_whole_or_not_at_all() -> Result<()> {
    let files = ["a.json", "a.md", "a.sparksmith"];
    let staged = |dir: &Path| -> Result<NewSet> {
        let mut set = NewSet::beside(&dir.join("a.sparksmith"))?;
        for file in files {
            std::fs::write(set.stage(&dir.join(file))?, b"ours")?;
        }
        Ok(set)
    };
    for taken in files {
        let dir = tempfile::tempdir()?;
        let set = staged(dir.path())?;
        std::fs::write(dir.path().join(taken), b"theirs")?;
        let refused = set.publish().err().map(|e| e.to_string());
        assert!(
            refused.as_ref().is_some_and(|e| e.contains(taken)),
            "{refused:?}"
        );
        assert_eq!(names(dir.path())?, [taken]);
        assert_eq!(std::fs::read(dir.path().join(taken))?, b"theirs");
    }
    let dir = tempfile::tempdir()?;
    drop(staged(dir.path())?);
    assert!(names(dir.path())?.is_empty());
    staged(dir.path())?.publish()?;
    assert_eq!(names(dir.path())?, files);
    Ok(())
}

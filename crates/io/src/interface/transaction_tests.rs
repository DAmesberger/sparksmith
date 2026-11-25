use super::*;
use sparksmith_core::ids::InterfaceSourceId;
use sparksmith_core::model::interface::Tier;
use std::time::{Duration, Instant};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn sidecar(revision: u32) -> Result<String> {
    Sidecar {
        format: "sparksmith-board-interface/1".into(),
        source_id: InterfaceSourceId::from_bytes([1; 16]),
        revision,
        hash: sparksmith_core::interface::publish::table_hash(&[], &[], &[])?,
        units: "mm".into(),
        tier: Tier::Interface,
        datums: vec![],
        retired_slots: vec![],
        unresolved_slots: vec![],
        parts: vec![],
    }
    .text()
    .map_err(Into::into)
}

fn initial(folder: &Path) -> Result<PathBuf> {
    let design = folder.join("board.solidcraft");
    std::fs::write(&design, b"1")?;
    std::fs::write(sidecar_path(&design), sidecar(1)?)?;
    Ok(design)
}

fn assert_pair(design: &Path, revision: u32) -> TestResult {
    let publication = Publication::open(design)?;
    assert_eq!(
        publication.sidecar()?.ok_or("sidecar missing")?.revision,
        revision
    );
    assert_eq!(publication.design_bytes()?, revision.to_string().as_bytes());
    Ok(())
}

#[test]
fn an_assembly_replaced_after_payload_staging_refuses_the_commit() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let baseline = Publication::open(&design)?.baseline()?;
    let assembly = dir.path().join("assembly.solidcraft");
    let temporary = dir.path().join("new.solidcraft");
    std::fs::write(&temporary, b"2")?;
    std::fs::write(&assembly, b"reviewed")?;
    let linked = (assembly.clone(), file_hash(b"reviewed"));
    let result = put_reviewed_with(
        &design,
        &temporary,
        &sidecar(2)?,
        &baseline,
        Some(&linked),
        WriteProtected::Refuse,
        &mut |at| {
            if at == Boundary::Staged {
                atomic_write(&assembly, b"saved with wall")?;
            }
            Ok(())
        },
    );
    assert!(
        matches!(result, Err(Error::Document(sparksmith_core::Error::Conflict(m))) if m.contains("interface.preview_changed"))
    );
    assert_pair(&design, 1)?;
    assert!(!sibling(&design, "publish-journal").exists());
    let writer = OpenOptions::new().read(true).write(true).open(&assembly)?;
    let latest = (assembly, file_hash(b"saved with wall"));
    put_reviewed_with(
        &design,
        &temporary,
        &sidecar(2)?,
        &baseline,
        Some(&latest),
        WriteProtected::Refuse,
        &mut |at| {
            if at == Boundary::Committed {
                assert!(matches!(
                    writer.try_lock(),
                    Err(std::fs::TryLockError::WouldBlock)
                ));
            }
            Ok(())
        },
    )?;
    writer.try_lock()?;
    assert_pair(&design, 2)?;
    Ok(())
}

#[test]
fn failure_before_commit_preserves_pair_and_after_commit_recovers() -> TestResult {
    for stop in [
        Boundary::Staged,
        Boundary::Committed,
        Boundary::Design,
        Boundary::Sidecar,
        Boundary::Removed,
    ] {
        let dir = tempfile::tempdir()?;
        let design = initial(dir.path())?;
        let baseline = Publication::open(&design)?.baseline()?;
        let temporary = dir.path().join("new.solidcraft");
        std::fs::write(&temporary, b"2")?;
        let result = put_with(&design, &temporary, &sidecar(2)?, &baseline, &mut |at| {
            if at == stop {
                Err(std::io::Error::other("induced failure").into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err(), "{stop:?}");
        assert_eq!(
            matches!(result, Err(Error::Unsynced(_))),
            stop != Boundary::Staged
        );
        assert_pair(&design, if stop == Boundary::Staged { 1 } else { 2 })?;
        assert!(!sibling(&design, "publish-journal").exists());
    }
    Ok(())
}

/// Invoked only by the parent below. The parent kills this process at the
/// selected boundary, so neither Rust drops nor rollback code can run.
#[test]
fn crash_child() -> TestResult {
    let Some(folder) = std::env::var_os("SPARKSMITH_PAIR_CRASH_FOLDER") else {
        return Ok(());
    };
    let folder = PathBuf::from(folder);
    let stop = std::env::var("SPARKSMITH_PAIR_CRASH_BOUNDARY")?;
    let design = folder.join("board.solidcraft");
    let baseline = Publication::open(&design)?.baseline()?;
    let temporary = folder.join("new.solidcraft");
    std::fs::write(&temporary, b"2")?;
    put_with(&design, &temporary, &sidecar(2)?, &baseline, &mut |at| {
        if format!("{at:?}") == stop {
            std::fs::write(folder.join("kill-now"), b"ready")?;
            loop {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Ok(())
    })?;
    Err("child missed its crash boundary".into())
}

#[test]
fn killed_publishers_recover_at_every_commit_boundary() -> TestResult {
    for first_publish in [false, true] {
        for stop in [
            Boundary::Staged,
            Boundary::Committed,
            Boundary::Design,
            Boundary::Sidecar,
            Boundary::Removed,
        ] {
            let dir = tempfile::tempdir()?;
            let design = if first_publish {
                dir.path().join("board.solidcraft")
            } else {
                initial(dir.path())?
            };
            let mut child = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "interface::transaction::tests::crash_child",
                    "--nocapture",
                ])
                .env("SPARKSMITH_PAIR_CRASH_FOLDER", dir.path())
                .env("SPARKSMITH_PAIR_CRASH_BOUNDARY", format!("{stop:?}"))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(20);
            while !dir.path().join("kill-now").exists() {
                if let Some(status) = child.try_wait()? {
                    return Err(format!("child exited early: {status}").into());
                }
                if Instant::now() > deadline {
                    child.kill()?;
                    child.wait()?;
                    return Err("child boundary timed out".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            child.kill()?;
            assert!(!child.wait()?.success());
            if first_publish && stop == Boundary::Staged {
                assert!(Publication::open(&design)?.sidecar()?.is_none());
                assert!(!design.exists());
            } else {
                assert_pair(&design, if stop == Boundary::Staged { 1 } else { 2 })?;
                assert_pair(&design, if stop == Boundary::Staged { 1 } else { 2 })?;
            }
        }
    }
    Ok(())
}

#[test]
fn competing_publishers_cannot_overwrite_the_winner() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let barrier = std::sync::Barrier::new(2);
    let outcomes = std::thread::scope(|scope| {
        let run = |revision: u32| {
            let baseline = Publication::open(&design)?.baseline()?;
            let temporary = dir.path().join(format!("new-{revision}.solidcraft"));
            std::fs::write(&temporary, revision.to_string())?;
            barrier.wait();
            put_in_place(&design, &temporary, &sidecar(revision)?, &baseline)
        };
        let one = scope.spawn(move || run(2));
        let two = scope.spawn(move || run(3));
        (one.join(), two.join())
    });
    let one = outcomes.0.map_err(|_| "first writer panicked")?;
    let two = outcomes.1.map_err(|_| "second writer panicked")?;
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let loser = if one.is_err() { &one } else { &two };
    assert!(
        loser
            .as_ref()
            .is_err_and(|e| e.to_string().contains("interface.changed"))
    );
    assert_pair(&design, if one.is_ok() { 2 } else { 3 })?;
    Ok(())
}

#[test]
fn participating_readers_never_see_mixed_generations() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    std::thread::scope(|scope| -> TestResult {
        let writer = scope.spawn(|| -> Result<()> {
            let temporary = dir.path().join("new.solidcraft");
            for revision in 2..22 {
                let baseline = Publication::open(&design)?.baseline()?;
                std::fs::write(&temporary, revision.to_string())?;
                put_with(
                    &design,
                    &temporary,
                    &sidecar(revision)?,
                    &baseline,
                    &mut |at| {
                        if at == Boundary::Design {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Ok(())
                    },
                )?;
            }
            Ok(())
        });
        for _ in 0..80 {
            let publication = Publication::open(&design)?;
            let sidecar = publication.sidecar()?.ok_or("sidecar missing")?;
            assert_eq!(
                publication.design_bytes()?,
                sidecar.revision.to_string().as_bytes()
            );
        }
        writer.join().map_err(|_| "writer panicked")??;
        Ok(())
    })?;
    assert_pair(&design, 21)
}

#[test]
fn untrusted_journals_never_redirect_or_replace_files() -> TestResult {
    for transaction in [
        "../outside",
        "/tmp/outside",
        ".interface-",
        ".interface-../outside",
        ".interface-☃",
    ] {
        let dir = tempfile::tempdir()?;
        let design = initial(dir.path())?;
        let journal = Journal {
            version: 1,
            design_name: "board.solidcraft".into(),
            transaction: transaction.into(),
            design_hash: file_hash(b"2"),
            sidecar_hash: file_hash(sidecar(2)?.as_bytes()),
        };
        std::fs::write(
            sibling(&design, "publish-journal"),
            serde_json::to_vec(&journal)?,
        )?;
        assert!(Publication::open(&design).is_err());
        assert_eq!(std::fs::read(&design)?, b"1");
        assert_eq!(std::fs::read_to_string(sidecar_path(&design))?, sidecar(1)?);
    }
    Ok(())
}

#[test]
fn corrupted_payload_refuses_recovery_without_replacing_either_file() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let baseline = Publication::open(&design)?.baseline()?;
    let temporary = dir.path().join("new.solidcraft");
    std::fs::write(&temporary, b"2")?;
    let result = put_with(&design, &temporary, &sidecar(2)?, &baseline, &mut |at| {
        if at == Boundary::Committed {
            Err(std::io::Error::other("stop").into())
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(Error::Unsynced(_))));
    let journal: Journal =
        serde_json::from_slice(&std::fs::read(sibling(&design, "publish-journal"))?)?;
    std::fs::write(
        dir.path()
            .join(journal.transaction)
            .join("design.solidcraft"),
        b"corrupt",
    )?;
    assert!(
        Publication::open(&design)
            .err()
            .is_some_and(|e| e.to_string().contains("hash mismatch"))
    );
    assert_eq!(std::fs::read(&design)?, b"1");
    assert_eq!(std::fs::read_to_string(sidecar_path(&design))?, sidecar(1)?);
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_payloads_and_destinations_are_refused() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let baseline = Publication::open(&design)?.baseline()?;
    let temporary = dir.path().join("new.solidcraft");
    let outside = dir.path().join("outside");
    std::fs::write(&outside, b"2")?;
    std::os::unix::fs::symlink(&outside, &temporary)?;
    assert!(put_in_place(&design, &temporary, &sidecar(2)?, &baseline).is_err());
    std::fs::remove_file(&design)?;
    std::os::unix::fs::symlink(&outside, &design)?;
    assert!(Publication::open(&design).is_err());
    assert_eq!(std::fs::read(&outside)?, b"2");
    Ok(())
}

#[test]
fn failed_design_install_does_not_replace_sidecar() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let baseline = Publication::open(&design)?.baseline()?;
    assert!(
        put_in_place(
            &design,
            &dir.path().join("missing.solidcraft"),
            &sidecar(2)?,
            &baseline
        )
        .is_err()
    );
    assert_pair(&design, 1)
}

/// One write-protected file refuses the pair and is named; told to replace,
/// both are, and it stays write-protected.
#[test]
fn a_write_protected_file_refuses_the_pair() -> TestResult {
    for sidecar_protected in [false, true] {
        let dir = tempfile::tempdir()?;
        let design = initial(dir.path())?;
        let baseline = Publication::open(&design)?.baseline()?;
        let temporary = dir.path().join("new.solidcraft");
        std::fs::write(&temporary, b"2")?;
        let file = if sidecar_protected {
            sidecar_path(&design)
        } else {
            design.clone()
        };
        let writable = std::fs::metadata(&file)?.permissions();
        let mut protected = writable.clone();
        protected.set_readonly(true);
        std::fs::set_permissions(&file, protected.clone())?;
        let before = std::fs::read_dir(dir.path())?.count();
        let refused = put_in_place(&design, &temporary, &sidecar(2)?, &baseline);
        assert!(
            matches!(&refused, Err(Error::WriteProtected(path)) if *path == file),
            "{refused:?}"
        );
        assert_eq!(std::fs::read_dir(dir.path())?.count(), before);
        assert_pair(&design, 1)?;
        put_reviewed_in_place(
            &design,
            &temporary,
            &sidecar(2)?,
            &baseline,
            None,
            WriteProtected::Replace,
        )?;
        assert_pair(&design, 2)?;
        assert_eq!(std::fs::metadata(&file)?.permissions(), protected);
        std::fs::set_permissions(&file, writable)?;
    }
    Ok(())
}

#[test]
fn changed_native_bytes_and_foreign_or_older_sources_are_refused() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let baseline = Publication::open(&design)?.baseline()?;
    let temporary = dir.path().join("new.solidcraft");
    std::fs::write(&temporary, b"2")?;
    let mut foreign = parse_sidecar(sidecar(2)?.as_bytes())?;
    foreign.source_id = InterfaceSourceId::from_bytes([2; 16]);
    let error = put_in_place(&design, &temporary, &foreign.text()?, &baseline);
    assert!(error.is_err_and(|e| e.to_string().contains("interface.source")));
    assert_pair(&design, 1)?;
    put_in_place(&design, &temporary, &sidecar(2)?, &baseline)?;
    let current = Publication::open(&design)?.baseline()?;
    let error = put_in_place(&design, &temporary, &sidecar(1)?, &current);
    assert!(error.is_err_and(|e| e.to_string().contains("interface.changed")));
    assert_pair(&design, 2)?;
    std::fs::write(&design, b"externally replaced")?;
    let error = put_in_place(&design, &temporary, &sidecar(3)?, &current);
    assert!(error.is_err_and(|e| e.to_string().contains("interface.changed")));
    assert_eq!(std::fs::read(&design)?, b"externally replaced");
    assert_eq!(std::fs::read_to_string(sidecar_path(&design))?, sidecar(2)?);
    Ok(())
}

#[test]
fn reader_blocks_between_public_file_replacements() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let baseline = Publication::open(&design)?.baseline()?;
    let temporary = dir.path().join("new.solidcraft");
    std::fs::write(&temporary, b"2")?;
    let (arrived, midway) = std::sync::mpsc::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let (observed, observation) = std::sync::mpsc::channel();
    std::thread::scope(|scope| -> TestResult {
        let design_ref = &design;
        let writer = scope.spawn(move || {
            put_with(design_ref, &temporary, &sidecar(2)?, &baseline, &mut |at| {
                if at == Boundary::Design {
                    arrived
                        .send(())
                        .map_err(|_| invalid("test receiver missing"))?;
                    proceed
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|_| invalid("test writer release timed out"))?;
                }
                Ok(())
            })
        });
        midway.recv_timeout(Duration::from_secs(5))?;
        let reader = scope.spawn(|| -> Result<()> {
            let publication = Publication::open(&design)?;
            let revision = publication
                .sidecar()?
                .ok_or_else(|| invalid("missing sidecar"))?
                .revision;
            observed
                .send((revision, publication.design_bytes()?))
                .map_err(|_| invalid("test observer missing"))?;
            Ok(())
        });
        let blocked = observation.recv_timeout(Duration::from_millis(50)).is_err();
        release.send(())?;
        writer.join().map_err(|_| "writer panicked")??;
        reader.join().map_err(|_| "reader panicked")??;
        assert!(
            blocked,
            "reader escaped the lock between the two replacements"
        );
        assert_eq!(observation.recv()?, (2, b"2".to_vec()));
        Ok(())
    })
}

#[test]
fn incomplete_or_unowned_pairs_are_refused_on_open() -> TestResult {
    for native_only in [true, false] {
        let dir = tempfile::tempdir()?;
        let design = dir.path().join("board.solidcraft");
        if native_only {
            std::fs::write(&design, b"unowned native content")?;
        } else {
            std::fs::write(sidecar_path(&design), sidecar(1)?)?;
        }
        assert!(
            Publication::open(&design)
                .err()
                .is_some_and(|e| e.to_string().contains("incomplete or unowned"))
        );
        assert_eq!(design.exists(), native_only);
        assert_eq!(sidecar_path(&design).exists(), !native_only);
        if native_only {
            assert_eq!(std::fs::read(&design)?, b"unowned native content");
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn an_existing_read_only_lock_still_allows_a_complete_pair_read() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    drop(Publication::open(&design)?);
    std::fs::set_permissions(
        sibling(&design, "publish-lock"),
        std::fs::Permissions::from_mode(0o444),
    )?;
    assert_pair(&design, 1)
}

#[test]
fn unknown_or_oversized_journals_are_refused_before_replacement() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = initial(dir.path())?;
    let path = sibling(&design, "publish-journal");
    let mut journal = Journal {
        version: 2,
        design_name: "board.solidcraft".into(),
        transaction: ".interface-abcd".into(),
        design_hash: file_hash(b"2"),
        sidecar_hash: file_hash(sidecar(2)?.as_bytes()),
    };
    std::fs::write(&path, serde_json::to_vec(&journal)?)?;
    assert!(
        Publication::open(&design)
            .err()
            .is_some_and(|e| e.to_string().contains("version 2"))
    );
    journal.version = 1;
    journal.design_name = "../outside.solidcraft".into();
    std::fs::write(&path, serde_json::to_vec(&journal)?)?;
    assert!(Publication::open(&design).is_err());
    std::fs::write(&path, vec![b' '; JOURNAL_LIMIT as usize + 1])?;
    assert!(matches!(
        Publication::open(&design),
        Err(Error::TooLarge(_))
    ));
    assert_eq!(std::fs::read(&design)?, b"1");
    assert_eq!(std::fs::read_to_string(sidecar_path(&design))?, sidecar(1)?);
    Ok(())
}

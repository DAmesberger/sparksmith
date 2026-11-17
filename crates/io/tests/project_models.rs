//! Original synthetic byte/ZIP fixtures exercise lookup, not a CAD parser.
use sparksmith_io::project_models::{
    MAX_ARCHIVE_BYTES, MAX_ARCHIVE_ENTRIES, MAX_DEPTH, MAX_MODEL_BYTES, MAX_TOTAL_BYTES,
    ProjectModels,
};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;
const BODY: &[u8] = b"Original synthetic model bytes; no third-party CAD data.";

fn archive(path: &Path, entries: &[(&str, &[u8])]) -> Fallible {
    let mut zip = ZipWriter::new(File::create(path)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, bytes) in entries {
        zip.start_file(*name, options)?;
        zip.write_all(bytes)?;
    }
    zip.finish()?;
    Ok(())
}

#[test]
fn project_relative_references_and_case_insensitive_fallback_find_the_original_bytes() -> Fallible {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join("vendor"))?;
    fs::write(dir.path().join("vendor/body.STEP"), BODY)?;
    let mut models = ProjectModels::new(dir.path())?;
    for reference in [
        "${KIPRJMOD}/vendor/body.STEP",
        "$(KIPRJMOD)/vendor/body.STEP",
        "vendor/body.STEP",
        "vendor\\body.STEP",
        "old/path/BODY.step",
    ] {
        let found = models.resolve(reference)?.ok_or("model missing")?;
        assert_eq!(found.bytes, BODY, "{reference}");
        assert!(
            found.source.ends_with("vendor/body.STEP"),
            "{}",
            found.source
        );
        assert!(!Path::new(&found.source).is_absolute());
    }
    assert!(models.resolve("absent.step")?.is_none());
    Ok(())
}

#[test]
fn filename_ambiguity_refuses_but_an_exact_project_path_stays_unambiguous() -> Fallible {
    let dir = tempfile::tempdir()?;
    for folder in ["one", "two"] {
        fs::create_dir(dir.path().join(folder))?;
        fs::write(dir.path().join(folder).join("body.step"), folder)?;
    }
    let mut models = ProjectModels::new(dir.path())?;
    assert_eq!(
        models
            .resolve("one/body.step")?
            .ok_or("direct model")?
            .bytes,
        b"one"
    );
    let error = models
        .resolve("old/body.step")
        .expect_err("ambiguous basename must refuse");
    assert!(error.to_string().contains("ambiguous"), "{error}");
    Ok(())
}

#[test]
fn library_references_never_trigger_project_or_archive_lookup() -> Fallible {
    let dir = tempfile::tempdir()?;
    fs::write(dir.path().join("body.step"), BODY)?;
    // Any attempt to index the folder would fail on this original malformed ZIP.
    fs::write(dir.path().join("broken.zip"), b"not a ZIP")?;
    let mut models = ProjectModels::new(dir.path())?;
    for reference in [
        "${KICAD9_3DMODEL_DIR}/Synthetic/body.step",
        "${KISYS3DMOD}/body.step",
        "/some/kicad/library/body.step",
        "C:\\kicad\\models\\body.step",
    ] {
        assert!(models.resolve(reference)?.is_none(), "{reference}");
    }
    Ok(())
}

#[test]
fn zip_members_resolve_without_extraction_and_record_archive_provenance() -> Fallible {
    let dir = tempfile::tempdir()?;
    archive(
        &dir.path().join("vendor.zip"),
        &[("models/Body.STEP", BODY)],
    )?;
    let mut models = ProjectModels::new(dir.path())?;
    let model = models.resolve("old/body.step")?.ok_or("ZIP model")?;
    assert_eq!(model.bytes, BODY);
    assert!(model.source.ends_with("vendor.zip!models/Body.STEP"));
    assert!(!dir.path().join("models").exists(), "no files extracted");
    Ok(())
}

#[test]
fn zip_unsafe_paths_are_rejected_on_every_host_platform() -> Fallible {
    for name in [
        "../escape.step",
        "/escape.step",
        "C:/escape.step",
        "C:\\escape.step",
        "\\\\server\\share\\escape.step",
    ] {
        let dir = tempfile::tempdir()?;
        archive(&dir.path().join("vendor.zip"), &[(name, BODY)])?;
        let mut models = ProjectModels::new(dir.path())?;
        assert!(models.resolve("escape.step").is_err(), "accepted {name}");
        assert_eq!(fs::read_dir(dir.path())?.count(), 1, "no extraction");
    }
    Ok(())
}

#[test]
fn oversized_files_archives_and_archive_entry_counts_refuse() -> Fallible {
    for (name, size, reference) in [
        ("large.step", MAX_MODEL_BYTES + 1, "large.step"),
        ("large.zip", MAX_ARCHIVE_BYTES + 1, "absent.step"),
    ] {
        let dir = tempfile::tempdir()?;
        File::create(dir.path().join(name))?.set_len(size)?;
        assert!(ProjectModels::new(dir.path())?.resolve(reference).is_err());
    }
    let dir = tempfile::tempdir()?;
    let mut zip = ZipWriter::new(File::create(dir.path().join("many.zip"))?);
    for n in 0..=MAX_ARCHIVE_ENTRIES {
        zip.start_file(format!("{n}.txt"), SimpleFileOptions::default())?;
    }
    zip.finish()?;
    assert!(
        ProjectModels::new(dir.path())?
            .resolve("absent.step")
            .is_err()
    );
    Ok(())
}

#[test]
fn search_depth_and_reference_lengths_are_explicit_limits() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut deepest = dir.path().to_path_buf();
    for _ in 0..=MAX_DEPTH {
        deepest.push("deeper");
        fs::create_dir(&deepest)?;
    }
    fs::write(deepest.join("body.step"), BODY)?;
    let mut models = ProjectModels::new(dir.path())?;
    assert!(models.resolve("body.step").is_err());
    assert!(
        models
            .resolve(&format!("{}.step", "a".repeat(4096)))
            .is_err()
    );
    Ok(())
}

#[test]
fn cumulative_budget_counts_archive_index_rereads_and_inflated_bytes() -> Fallible {
    let dir = tempfile::tempdir()?;
    let payload = vec![b'x'; 1024 * 1024];
    let path = dir.path().join("vendor.zip");
    archive(&path, &[("body.step", &payload)])?;
    let compressed = fs::metadata(path)?.len();
    // One index read, then each resolve reads the archive and its member.
    let allowed = (MAX_TOTAL_BYTES - compressed) / (compressed + payload.len() as u64);
    let mut models = ProjectModels::new(dir.path())?;
    for _ in 0..allowed {
        assert_eq!(models.resolve("body.step")?.ok_or("model")?.bytes, payload);
    }
    assert!(
        models.resolve("body.step").is_err(),
        "cumulative reads must stop"
    );
    Ok(())
}

#[test]
fn archive_replacement_cannot_change_the_indexed_member_name() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("vendor.zip");
    archive(&path, &[("body.step", BODY)])?;
    let mut models = ProjectModels::new(dir.path())?;
    assert!(models.resolve("absent.step")?.is_none());
    archive(&path, &[("../body.step", BODY)])?;
    assert!(models.resolve("body.step").is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn replacing_indexed_files_or_directories_with_external_symlinks_never_reads_them() -> Fallible {
    use std::os::unix::fs::symlink;
    for directory in [false, true] {
        let dir = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        fs::write(outside.path().join("body.step"), b"external sentinel")?;
        fs::create_dir(dir.path().join("models"))?;
        let path = dir.path().join("models/body.step");
        fs::write(&path, BODY)?;
        let mut models = ProjectModels::new(dir.path())?;
        assert!(models.resolve("absent.step")?.is_none());
        if directory {
            fs::remove_dir_all(dir.path().join("models"))?;
            symlink(outside.path(), dir.path().join("models"))?;
        } else {
            fs::remove_file(&path)?;
            symlink(outside.path().join("body.step"), &path)?;
        }
        assert!(
            models.resolve("old/body.step").is_err(),
            "must refuse an escaped cached path"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn renaming_the_root_does_not_redirect_an_existing_resolver_to_another_folder() -> Fallible {
    let parent = tempfile::tempdir()?;
    let root = parent.path().join("project");
    fs::create_dir(&root)?;
    fs::write(root.join("body.step"), BODY)?;
    let mut models = ProjectModels::new(&root)?;
    fs::rename(&root, parent.path().join("original"))?;
    fs::create_dir(&root)?;
    fs::write(root.join("body.step"), b"replacement sentinel")?;
    assert_eq!(
        models.resolve("body.step")?.ok_or("original model")?.bytes,
        BODY
    );
    Ok(())
}

#[test]
fn basename_fallback_does_not_enter_recognized_library_folders_or_zip_members() -> Fallible {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join("kicad"))?;
    fs::write(dir.path().join("kicad/body.step"), BODY)?;
    archive(
        &dir.path().join("synthetic.zip"),
        &[("kicad/body.step", BODY)],
    )?;
    assert!(
        ProjectModels::new(dir.path())?
            .resolve("body.step")?
            .is_none()
    );
    Ok(())
}

#[test]
fn the_model_byte_limit_accepts_exactly_the_limit_then_refuses_the_first_extra_byte() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("body.step");
    let file = File::create(&path)?;
    file.set_len(MAX_MODEL_BYTES)?;
    let model = ProjectModels::new(dir.path())?
        .resolve("body.step")?
        .ok_or("boundary model")?;
    assert_eq!(model.bytes.len() as u64, MAX_MODEL_BYTES);
    file.set_len(MAX_MODEL_BYTES + 1)?;
    assert!(
        ProjectModels::new(dir.path())?
            .resolve("body.step")
            .is_err()
    );
    Ok(())
}

#[test]
fn an_oversized_deflated_member_is_refused_before_inflation() -> Fallible {
    use std::io::Read;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("vendor.zip");
    let mut zip = ZipWriter::new(File::create(&path)?);
    zip.start_file(
        "body.step",
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )?;
    std::io::copy(&mut std::io::repeat(0).take(MAX_MODEL_BYTES + 1), &mut zip)?;
    zip.finish()?;
    assert!(fs::metadata(path)?.len() < MAX_MODEL_BYTES);
    assert!(
        ProjectModels::new(dir.path())?
            .resolve("body.step")
            .is_err()
    );
    Ok(())
}

#[test]
fn archive_symlinks_are_refused_without_following_or_extracting_them() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut zip = ZipWriter::new(File::create(dir.path().join("vendor.zip"))?);
    zip.add_symlink(
        "body.step",
        "../external.step",
        SimpleFileOptions::default(),
    )?;
    zip.finish()?;
    assert!(
        ProjectModels::new(dir.path())?
            .resolve("body.step")
            .is_err()
    );
    assert_eq!(fs::read_dir(dir.path())?.count(), 1);
    Ok(())
}

#[test]
fn archive_scans_share_one_entry_budget_across_the_whole_project() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("one.zip");
    let mut zip = ZipWriter::new(File::create(&path)?);
    for n in 0..MAX_ARCHIVE_ENTRIES {
        zip.start_file(format!("{n}.txt"), SimpleFileOptions::default())?;
    }
    zip.finish()?;
    for n in 0..4 {
        fs::copy(&path, dir.path().join(format!("copy{n}.zip")))?;
    }
    let error = ProjectModels::new(dir.path())?
        .resolve("absent.step")
        .expect_err("five individually valid ZIPs exceed the shared entry budget");
    assert!(error.to_string().contains("20000"), "{error}");
    Ok(())
}

#[test]
fn basename_fallback_supports_unicode_case_insensitive_names() -> Fallible {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join("vendor"))?;
    fs::write(dir.path().join("vendor/ÄλΦ.STEP"), BODY)?;
    archive(&dir.path().join("vendor.zip"), &[("parts/ÖΔ.STEP", BODY)])?;
    let mut models = ProjectModels::new(dir.path())?;
    for reference in ["old/äλφ.step", "old/öδ.step"] {
        assert_eq!(
            models
                .resolve(reference)?
                .ok_or("Unicode name missing")?
                .bytes,
            BODY
        );
    }
    Ok(())
}

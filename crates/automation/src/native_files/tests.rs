use super::*;
use sparksmith_solidcraft::native::Native;

type Fallible = std::result::Result<(), Box<dyn std::error::Error>>;

fn files(path: &Path) -> std::io::Result<Vec<String>> {
    let mut files = std::fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    Ok(files)
}

fn competing_writer(target: &str, write: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
    Writer.write_new(target, &mut |staged| {
        write(staged)?;
        // The native writer finished a valid container before another
        // creator claimed the destination, immediately before publication.
        Native::open(staged)?;
        std::fs::write(target, b"competing creator")?;
        Ok(())
    })
}

#[test]
fn native_save_preserves_a_competing_creators_file() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("model.solidcraft");
    let text = path.to_str().ok_or("path")?;
    let native = Native::new("Synthetic model")?;
    let result = native.save_new(NewFile::new(text, &competing_writer));
    assert!(
        matches!(&result, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&path)?, b"competing creator");
    assert_eq!(files(dir.path())?, vec!["model.solidcraft".to_owned()]);
    Ok(())
}

#[test]
fn native_export_preserves_a_competing_creators_file() -> Fallible {
    use sparksmith_solidcraft::export::{self, Tier, Unfit, Watch};
    let _context =
        sparksmith_core::library::standards::Context::fixture().map(|context| context.enter());
    let engine = sparksmith_core::Engine::profiled()?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("board.solidcraft");
    let text = path.to_str().ok_or("path")?;
    let result = export::board(
        engine.project(),
        Tier::Mechanical,
        NewFile::new(text, &competing_writer),
        &Watch::new(),
        Unfit::Refuse,
    );
    assert!(
        matches!(&result, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&path)?, b"competing creator");
    assert_eq!(files(dir.path())?, vec!["board.solidcraft".to_owned()]);
    Ok(())
}

#[test]
fn native_writer_failures_keep_their_type_and_leave_no_output() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("model.solidcraft");
    let text = path.to_str().ok_or("path")?;
    let result = Writer.write_new(text, &mut |staged| {
        std::fs::write(staged, b"partial output")?;
        Err(Error::Engine {
            command: "file.save_as".into(),
            message: "injected native failure".into(),
        })
    });
    assert!(
        matches!(result, Err(Error::Engine { command, message }) if command == "file.save_as" && message == "injected native failure")
    );
    assert!(!path.exists());
    assert!(files(dir.path())?.is_empty());
    let unsynced = file_error(sparksmith_io::Error::Unsynced(
        "file already written".into(),
    ));
    assert!(matches!(&unsynced, Error::Unsynced(message) if message == "file already written"));
    assert_eq!(unsynced.code(), "written_unsynced");
    Ok(())
}

#[test]
fn native_save_uses_the_final_filename_as_its_design_name() -> Fallible {
    let dir = tempfile::tempdir()?;
    for stem in ["chosen", "Untitled"] {
        let path = dir.path().join(format!("{stem}.solidcraft"));
        let text = path.to_str().ok_or("path")?;
        let native = Native::new("Synthetic model")?;
        native.save_new(new_file(text))?;
        let reopened = Native::open(text)?;
        assert_eq!(reopened.session().doc.name, stem);
        assert_eq!(native.session().doc.name, "Synthetic model");
    }
    assert_eq!(
        files(dir.path())?,
        vec!["Untitled.solidcraft", "chosen.solidcraft"]
    );
    Ok(())
}

#[test]
fn native_save_rebases_links_against_the_final_directory() -> Fallible {
    use sparksmith_solidcraft::linked_design::LinkedDesign;
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("source");
    let destination = dir.path().join("destination");
    std::fs::create_dir(&source)?;
    std::fs::create_dir(&destination)?;
    let part = source.join("part.solidcraft");
    let original = source.join("assembly.solidcraft");
    let final_path = destination.join("copy.solidcraft");
    let part = part.to_str().ok_or("path")?;
    let original = original.to_str().ok_or("path")?;
    let final_text = final_path.to_str().ok_or("path")?;
    let mut model = LinkedDesign::new("Part")?;
    model.solid_box("Body", [0.0; 3], [2.0; 3])?;
    model.save(new_file(part))?;
    let mut assembly = LinkedDesign::new("Assembly")?;
    assembly.link(part, "Part")?;
    assembly.save(new_file(original))?;
    let (opened, _) = LinkedDesign::open(original)?;
    opened.save(new_file(final_text))?;
    let native = Native::open(final_text)?;
    assert_eq!(native.session().doc.name, "copy");
    let link = native.session().doc.links.values().next().ok_or("link")?;
    let resolved = destination.join(&link.path).canonicalize()?;
    assert_eq!(resolved, Path::new(part).canonicalize()?);
    let (mut reopened, _) = LinkedDesign::open(final_text)?;
    let update = reopened.update_links()?;
    assert!(update.failed.is_empty(), "{update:?}");
    assert_eq!(files(&destination)?, vec!["copy.solidcraft"]);
    Ok(())
}

#[test]
fn staging_uses_the_logical_name_and_link_base_without_replacing_it() -> Fallible {
    use sparksmith_solidcraft::linked_design::LinkedDesign;
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("source");
    let destination = dir.path().join("destination");
    let scratch = dir.path().join("scratch/deep/physical");
    for folder in [&source, &destination, &scratch] {
        std::fs::create_dir_all(folder)?;
    }
    let part = source.join("part.solidcraft");
    let original = source.join("assembly.solidcraft");
    let final_path = destination.join("published.solidcraft");
    let output = scratch.join("random-scratch.solidcraft");
    let part_text = part.to_str().ok_or("path")?;
    let original_text = original.to_str().ok_or("path")?;
    let final_text = final_path.to_str().ok_or("path")?;
    let output_text = output.to_str().ok_or("path")?;
    let mut model = LinkedDesign::new("Part")?;
    model.solid_box("Body", [0.0; 3], [2.0; 3])?;
    model.save(new_file(part_text))?;
    let mut assembly = LinkedDesign::new("Assembly")?;
    assembly.link(part_text, "Part")?;
    assembly.save(new_file(original_text))?;
    std::fs::write(&final_path, b"previous publication")?;
    let native = Native::open(original_text)?;
    native.stage_new(new_file(output_text), final_text)?;
    assert_eq!(std::fs::read(&final_path)?, b"previous publication");
    let staged = Native::open(output_text)?;
    assert_eq!(staged.session().doc.name, "published");
    assert_eq!(files(&scratch)?, ["random-scratch.solidcraft"]);
    assert_eq!(files(&destination)?, ["published.solidcraft"]);
    let bytes = std::fs::read(&output)?;
    assert!(native.stage_new(new_file(output_text), final_text).is_err());
    assert_eq!(std::fs::read(&output)?, bytes);
    assert_eq!(std::fs::read(&final_path)?, b"previous publication");
    // Consume the original staged bytes at their intended logical location.
    // Opening scratch alone could resolve links relative to the wrong folder,
    // and inspecting an already opened link may show an absolute path.
    std::fs::copy(&output, &final_path)?;
    let (mut consumer, newer) = LinkedDesign::open(final_text)?;
    assert!(!newer);
    let update = consumer.update_links()?;
    assert!(update.failed.is_empty(), "{update:?}");
    let consumed = Native::open(final_text)?;
    assert_eq!(consumed.session().doc.name, "published");
    let link = consumed
        .session()
        .doc
        .links
        .values()
        .next()
        .ok_or("consumed link")?;
    assert_eq!(
        destination.join(&link.path).canonicalize()?,
        part.canonicalize()?
    );
    Ok(())
}

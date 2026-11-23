use super::*;
#[test]
fn a_broken_topic_names_its_source() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    std::fs::write(root.join(crate::shipped::MARKER), "{}").expect("marker");
    std::fs::create_dir_all(root.join("help/generated")).expect("generated");
    std::fs::create_dir_all(root.join("help/topics")).expect("topics");
    std::fs::write(root.join("help/topics/broken.md"), "no frontmatter").expect("file");
    let err = load(&ShippedLibrary::at(root).expect("library")).expect_err("invalid");
    assert!(err.to_string().contains("broken.md"), "{err}");
}

#[test]
fn installing_a_shared_method_makes_the_same_prose_readable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    fs::write(root.join(crate::shipped::MARKER), "{}")?;
    for folder in ["help/generated", "help/topics", "assist"] {
        fs::create_dir_all(root.join(folder))?;
    }
    let library = ShippedLibrary::at(root)?;
    let id = "guide/agent-method".parse().map_err(Error::Invalid)?;
    assert!(!load(&library)?.contains_key(&id));
    fs::write(
        root.join("assist/method.md"),
        "# Shared method\n\nCheck the result.",
    )?;
    let entry = load(&library)?
        .remove(&id)
        .ok_or_else(|| Error::Invalid("missing method".into()))?;
    assert_eq!(entry.title, "Shared method");
    assert_eq!(
        entry.body,
        vec![Block::Paragraph(
            inlines("Check the result.").map_err(Error::Invalid)?
        )]
    );
    Ok(())
}

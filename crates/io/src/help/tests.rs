use super::*;
fn fixture(root: &Path) -> Result<ShippedLibrary> {
    fs::write(root.join(crate::shipped::MARKER), "{}")?;
    for dir in ["help/generated", "help/topics", "assist"] {
        fs::create_dir_all(root.join(dir))?;
    }
    fs::write(
        root.join("help/manifest.json"),
        r#"{"format":1,"generated":[],"topics":[]}"#,
    )?;
    ShippedLibrary::at(root)
}

#[test]
fn a_broken_topic_names_its_source() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let library = fixture(root)?;
    fs::write(root.join("help/topics/broken.md"), "no frontmatter")?;
    fs::write(
        root.join("help/manifest.json"),
        r#"{"format":1,"generated":[],"topics":["broken.md"]}"#,
    )?;
    let err = load(&library)
        .err()
        .ok_or_else(|| Error::Invalid("accepted broken topic".into()))?;
    assert!(err.to_string().contains("broken.md"), "{err}");
    Ok(())
}

#[test]
fn a_missing_registry_names_the_lost_file() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    fs::write(
        temp.path().join("help/manifest.json"),
        r#"{"format":1,"generated":["settings.json"],"topics":[]}"#,
    )?;
    let err = load(&library)
        .err()
        .ok_or_else(|| Error::Invalid("accepted partial installation".into()))?;
    assert!(err.to_string().contains("generated/settings.json"), "{err}");
    Ok(())
}

#[test]
fn installing_a_shared_method_makes_the_same_prose_readable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let library = fixture(root)?;
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

#[test]
fn screenshot_version_is_the_captured_build_even_when_the_topic_is_older() -> Result<()> {
    let body =
        blocks(r#"![Help search results](help-search.png "1.2.3")"#).map_err(Error::Invalid)?;
    assert!(matches!(body.as_slice(), [Block::Image { version, .. }] if version == "1.2.3"));
    assert!(blocks("![Help search results](help-search.png)").is_err());
    assert!(blocks(r#"![](help-search.png "1.2.3")"#).is_err());
    Ok(())
}

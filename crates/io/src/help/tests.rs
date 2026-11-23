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

#[test]
fn finding_summaries_can_be_authored_independently_of_the_cause() -> Result<()> {
    let finding_id = "finding/board.via_annular_ring";
    let mut index: Index = sparksmith_core::help::finding_entries()
        .map_err(Error::Invalid)?
        .into_iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect();
    let id: HelpId = finding_id.parse().map_err(Error::Invalid)?;
    for summary in [None, Some("Inspect the ring around this via.")] {
        let cause = "The pad leaves too little copper around the drill.";
        let header = serde_json::from_value(serde_json::json!({
            "id": finding_id, "kind": "finding", "since": "0.1.0",
            "cause": cause, "fix": "Enlarge the pad", "summary": summary
        }))?;
        apply_topic(&mut index, header, vec![]).map_err(Error::Invalid)?;
        let entry = index.get(&id).ok_or_else(|| Error::Invalid(id.to_string()))?;
        assert_eq!(entry.summary, summary.unwrap_or(cause));
        assert!(matches!(&entry.facts, Facts::Finding { cause: text, .. } if text == cause));
    }
    Ok(())
}

#[test]
fn publishing_a_new_release_preserves_the_first_release_of_existing_ids() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("entries.json");
    let make = |id: &str, version: &str| -> Result<HelpEntry> {
        Ok(HelpEntry {
            id: id.parse().map_err(Error::Invalid)?,
            title: "Topic".into(),
            summary: "A topic.".into(),
            since: version.into(),
            facts: Facts::Guide,
            related: vec![],
            keywords: vec![],
            body: vec![],
        })
    };
    let original = make("guide/first", "0.1.0")?;
    sync_registry(&path, vec![original.clone()], true)?;
    let mut revised = make("guide/first", "0.2.0")?;
    revised.title = "Revised topic".into();
    let introduced = make("guide/second", "0.2.0")?;
    let release = vec![introduced.clone(), revised.clone()];
    assert!(sync_registry(&path, release.clone(), false).is_err());
    sync_registry(&path, release.clone(), true)?;
    sync_registry(&path, release, false)?;
    let rows: Vec<HelpEntry> = serde_json::from_str(&source(&path)?)?;
    revised.since = original.since;
    assert_eq!(rows, vec![revised, introduced]);
    let duplicated = vec![make("guide/first", "0.3.0")?, make("guide/first", "0.3.0")?];
    assert!(sync_registry(&path, duplicated, true).is_err());
    assert_eq!(serde_json::from_str::<Vec<HelpEntry>>(&source(&path)?)?, rows);
    Ok(())
}

use super::*;
use sparksmith_core::help::Facts;
use std::fs;

fn guide() -> Result<HelpEntry> {
    Ok(HelpEntry {
        id: "guide/route".parse().map_err(Error::Invalid)?,
        title: "Route".into(),
        summary: "Route this board.".into(),
        since: "0.1.0".into(),
        facts: Facts::Guide,
        body: vec![Block::Paragraph(vec![
            Inline::Text("Read ".into()),
            Inline::Reference("panel/issues".parse().map_err(Error::Invalid)?),
        ])],
        related: vec![],
        keywords: vec![],
    })
}
#[test]
fn translating_changes_only_prose_and_a_stale_translation_falls_back() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let en = guide()?;
    let mut tr = translation_source(&en)?;
    tr.text.insert("title".into(), "Routen".into());
    fs::write(root.join("route.json"), serde_json::to_vec(&tr)?)?;
    let index = Index::from([(en.id.clone(), en.clone())]);
    let current = overlay(index.clone(), root)?;
    assert_eq!(current.status[&en.id], Status::Translated);
    assert_eq!(current.index[&en.id].title, "Routen");
    assert_eq!(current.index[&en.id].body, en.body);
    let mut revised = en.clone();
    revised.summary = "Route the current net.".into();
    let older = overlay(Index::from([(revised.id.clone(), revised.clone())]), root)?;
    assert_eq!(older.status[&en.id], Status::Stale);
    assert_eq!(older.index[&en.id], revised);
    tr.text.insert("facts.schema_digest".into(), "bad".into());
    fs::write(root.join("route.json"), serde_json::to_vec(&tr)?)?;
    assert!(overlay(index, root).is_err());
    Ok(())
}

#[test]
fn locale_names_are_languages_and_never_paths() {
    for name in ["de", "en-GB", "zh-Hant-TW"] {
        assert!(is_locale(name));
    }
    for name in ["", ".", "de/../../x", "en_US", "generated", "-fr", "fr-"] {
        assert!(!is_locale(name));
    }
}

use super::*;
use serde_json::json;
use sparksmith_core::help::{Block, Facts, HelpEntry, Inline};

fn guide(id: &str) -> crate::Result<HelpEntry> {
    Ok(HelpEntry {
        id: format!("guide/{id}").parse()?,
        title: "First board".into(),
        summary: "Place and route a board.".into(),
        since: "1.0".into(),
        facts: Facts::Guide,
        body: vec![],
        related: vec![],
        keywords: vec![],
    })
}

#[test]
fn a_missing_link_names_both_topics() -> crate::Result<()> {
    let mut source = guide("first-board")?;
    source.body.push(Block::List(vec![vec![Inline::Reference(
        "entry/layout.missing".parse()?,
    )]]));
    let index = Index::from([(source.id.clone(), source)]);
    let err = validate_entries(&index, Path::new("."))
        .expect_err("dangling list link")
        .to_string();
    assert!(
        err.contains("guide/first-board") && err.contains("entry/layout.missing"),
        "{err}"
    );
    Ok(())
}

#[test]
fn stale_dependency_requires_deliberate_restamping() -> crate::Result<()> {
    let covered = HelpEntry {
        facts: Facts::Setting {
            schema: json!({"type":"integer"}),
            default: json!(12),
            unit: Some("mm".into()),
            source: "Fixture".into(),
        },
        id: "setting/pitch".parse()?,
        ..guide("test")?
    };
    let id = covered.id.clone();
    let stamps = BTreeMap::from([(id.clone(), digest(&covered)?)]);
    let mut changed = covered;
    if let Facts::Setting { default, .. } = &mut changed.facts {
        *default = json!(13);
    }
    let index = Index::from([(id.clone(), changed)]);
    let err = check_covers(&"guide/test".parse()?, &stamps, &index).expect_err("stale");
    assert!(err.to_string().contains("setting/pitch"));
    assert!(err.to_string().contains("guide/test"));
    Ok(())
}

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
        .err()
        .ok_or("dangling list link accepted")?
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
    let err = check_covers(&"guide/test".parse()?, &stamps, &index)
        .err()
        .ok_or("stale stamp accepted")?;
    assert!(err.to_string().contains("setting/pitch"));
    assert!(err.to_string().contains("guide/test"));
    Ok(())
}

#[test]
fn site_writes_both_versions_with_local_links_and_escaped_text() -> crate::Result<()> {
    use sparksmith_io::shipped::{MARKER, ShippedLibrary};
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(MARKER), "{}")?;
    let library = ShippedLibrary::at(dir.path())?;
    let mut first = guide("first-board")?;
    let mut next = guide("next")?;
    next.title = "Trace & pad".into();
    first.body.push(Block::Paragraph(vec![
        Inline::Text("<draft> ".into()),
        Inline::Reference(next.id.clone()),
    ]));
    let index = Index::from([(first.id.clone(), first), (next.id.clone(), next)]);
    let out = dir.path().join("site");
    super::site::render(&out, &index, &library)?;
    for ver in [env!("CARGO_PKG_VERSION"), "latest"] {
        let dir = out.join("docs").join(ver);
        let page = std::fs::read_to_string(dir.join("guide/first-board.html"))?;
        assert!(page.contains(r#"href="../guide/next.html">Trace &amp; pad</a>"#));
        assert!(page.contains("&lt;draft&gt;"));
        let search: Vec<Value> = serde_json::from_slice(&std::fs::read(dir.join("search.json"))?)?;
        assert_eq!(search.len(), 2);
        assert!(dir.join("index.html").is_file());
    }
    let old = out.join("docs/old-release");
    std::fs::create_dir_all(&old)?;
    std::fs::write(old.join("keep"), "archived")?;
    super::site::render(&out, &Index::new(), &library)?;
    assert!(old.join("keep").is_file());
    assert!(!out.join("docs/latest/guide/first-board.html").exists());
    Ok(())
}

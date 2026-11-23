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
fn a_finding_names_the_missing_rule_and_profile() -> crate::Result<()> {
    let mut finding = sparksmith_core::help::finding_entries()?
        .into_iter()
        .find(|e| e.id.name() == "board.via_annular_ring")
        .ok_or("missing annular ring declaration")?;
    if let Facts::Finding {
        rule_key,
        cause,
        fix,
        ..
    } = &mut finding.facts
    {
        *rule_key = Some("via_annular_ring_min".into());
        *cause = "The copper ring is too narrow".into();
        *fix = "Enlarge the via pad".into();
    }
    let rule: HelpId = "rule/fixture/drill_min".parse()?;
    let mut index = Index::from([
        (finding.id.clone(), finding.clone()),
        (
            rule.clone(),
            HelpEntry {
                id: rule,
                ..guide("test")?
            },
        ),
    ]);
    let err = validate_entries(&index, Path::new("."))
        .err()
        .ok_or("missing profile rule accepted")?;
    assert!(
        err.to_string()
            .contains("rule/fixture/via_annular_ring_min")
    );
    if let Facts::Finding { rule_key, .. } = &mut finding.facts {
        *rule_key = Some("misspelled".into());
    }
    index.insert(finding.id.clone(), finding);
    let err = validate_entries(&index, Path::new("."))
        .err()
        .ok_or("unknown rule accepted")?;
    assert!(
        err.to_string()
            .contains("unknown fabrication rule misspelled")
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

#[test]
fn settings_describe_actual_defaults_and_nested_rows() -> crate::Result<()> {
    let library = ShippedLibrary::at(&crate::root()?.join("library"))?;
    let rows = registries::settings_entries(&library)?;
    let get = |key: &str| {
        rows.iter()
            .find(|e| e.id.as_str() == key)
            .ok_or_else(|| format!("missing {key}"))
    };
    assert!(
        matches!(&get("setting/keyboard.reveal.hold_ms")?.facts, Facts::Setting { default,schema,.. }
        if default.as_u64()==Some(sparksmith_io::settings::HOLD_MS_DEFAULT.into()) &&
           schema["minimum"].as_u64()==Some((*sparksmith_io::settings::HOLD_MS_RANGE.start()).into()))
    );
    assert!(
        matches!(&get("setting/sources.enabled")?.facts, Facts::Setting { default,.. } if default.is_null())
    );
    assert!(
        matches!(&get("setting/projects")?.facts, Facts::Setting { schema,default,.. }
        if default==&json!([]) && schema["maxItems"]==json!(sparksmith_io::settings::MAX_RECENT_PROJECTS))
    );
    Ok(())
}

#[test]
fn parameter_urls_resolve_as_pages_and_fragment_targets() -> crate::Result<()> {
    use sparksmith_generators::abi::v1::{Param, ParamType, ParamValue};
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(sparksmith_io::shipped::MARKER), "{}")?;
    let library = ShippedLibrary::at(dir.path())?;
    let param = Param {
        key: "pitch".into(),
        label: "Land pitch".into(),
        help: None,
        ty: ParamType::Length {
            min_nm: 1,
            max_nm: 10,
            step_nm: None,
        },
        default: ParamValue::Int(4),
        group: None,
        context: None,
    };
    let parent = HelpEntry {
        id: "generator/example".parse()?,
        title: "Example footprint".into(),
        facts: Facts::Generator {
            parameters: vec![param],
            presets: vec![],
        },
        ..guide("test")?
    };
    let child = HelpEntry {
        id: "generator/example#pitch".parse()?,
        title: "Land pitch".into(),
        ..parent.clone()
    };
    let index = Index::from([(parent.id.clone(), parent), (child.id.clone(), child)]);
    site::render(dir.path(), &index, &library)?;
    let latest = dir.path().join("docs/latest");
    assert!(
        latest
            .join("generator/example/parameters/pitch.html")
            .is_file()
    );
    let body = std::fs::read_to_string(latest.join("generator/example.html"))?;
    let detail = std::fs::read_to_string(latest.join("generator/example/parameters/pitch.html"))?;
    assert!(detail.contains(r#"href="../../../index.html""#));
    assert!(detail.contains(r#"href="../../../manual.css""#));
    let search: Vec<Value> = serde_json::from_slice(&std::fs::read(latest.join("search.json"))?)?;
    assert!(
        search
            .iter()
            .any(|row| row["id"] == "generator/example#pitch"
                && row["href"] == "generator/example/parameters/pitch.html")
    );
    assert!(body.contains(r#"<tr id="pitch">"#));
    assert!(body.contains(r#"href="../generator/example/parameters/pitch.html">Land pitch</a>"#));
    Ok(())
}

#[test]
fn a_renamed_or_ignored_registry_test_cannot_pass_the_gate() {
    let name = "commands::registry";
    assert!(!registry_test_passed(
        "running 0 tests\ntest result: ok. 0 passed",
        name
    ));
    assert!(!registry_test_passed(
        "test commands::registry ... ignored",
        name
    ));
    assert!(!registry_test_passed(
        "test different::registry ... ok",
        name
    ));
    assert!(registry_test_passed(
        "running 1 test\ntest commands::registry ... ok\n",
        name
    ));
}

#[test]
fn an_oversized_shard_is_named_and_the_previous_file_survives() -> crate::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("generators.json");
    std::fs::write(&path, "[]")?;
    let large = "x".repeat(sparksmith_core::format::MAX_FILE_BYTES + 1);
    let error = compare(&path, &large, true)
        .err()
        .ok_or("accepted oversized shard")?;
    assert!(error.to_string().contains("generators.json"));
    assert_eq!(std::fs::read_to_string(path)?, "[]");
    Ok(())
}

#[test]
fn an_offline_finding_links_to_the_rule_in_each_applicable_profile() -> crate::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(sparksmith_io::shipped::MARKER), "{}")?;
    let library = ShippedLibrary::at(dir.path())?;
    let mut finding = sparksmith_core::help::finding_entries()?
        .into_iter()
        .find(|e| e.id.name() == "board.via_annular_ring")
        .ok_or("missing finding")?;
    let facts = &mut finding.facts;
    if let Facts::Finding {
        cause,
        fix,
        rule_key,
        ..
    } = facts
    {
        *cause = "The copper ring is narrow.".into();
        *fix = "Enlarge the pad".into();
        *rule_key = Some("via_annular_ring_min".into());
        finding.summary.clone_from(cause);
    }
    let mut index = Index::from([(finding.id.clone(), finding)]);
    for profile in ["alpha", "beta"] {
        let id: HelpId = format!("rule/{profile}/via_annular_ring_min").parse()?;
        let rule = HelpEntry {
            id: id.clone(),
            title: format!("Annular ring for {profile}"),
            facts: Facts::Rule {
                value: json!(0.1),
                unit: "mm".into(),
                source: serde_json::from_value(json!({
                    "publisher":"Fixture", "document":"Process sheet", "date":"2026-01-01"
                }))?,
                row: "Ring width".into(),
                author_choice: false,
            },
            ..guide("fixture")?
        };
        index.insert(id, rule);
    }
    let out = dir.path().join("manual");
    site::render(&out, &index, &library)?;
    let html = fs::read_to_string(out.join("docs/latest/finding/board.via_annular_ring.html"))?;
    for profile in ["alpha", "beta"] {
        assert!(html.contains(&format!(
            "href=\"../rule/{profile}/via_annular_ring_min.html\""
        )));
    }
    assert_eq!(html.matches("The copper ring is narrow.").count(), 1);
    Ok(())
}

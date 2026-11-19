use super::*;
use crate::ids::{DefinitionId, DefinitionKind, DefinitionRef, Revision};
use crate::library::generated::{GeneratedFrom, GeneratedPart, GeneratorScope, ParamValue};
use crate::library::standards::fixture;

fn shipped() -> Result<Standards> {
    fixture::shipped()
}

fn tree(json: serde_json::Value) -> Result<Categories> {
    serde_json::from_value(json).map_err(|e| Error::Invalid(e.to_string()))
}

#[test]
fn the_shipped_tree_files_by_function_and_ends_in_other() -> Result<()> {
    let standards = shipped()?;
    let top: Vec<&str> = standards
        .categories
        .categories
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(top.len(), 15, "{top:?}");
    assert_eq!(top.last(), Some(&OTHER));
    assert!(standards.categories.find("ics/processors").is_some());
    assert!(standards.categories.find("passives/resistors").is_some());
    assert!(standards.categories.find("passives/nothing").is_none());
    // Icons come from the data: a category's own, else its parent's, and
    // Other's for a path the tree does not hold.
    let icon = |path| standards.categories.icon(path);
    assert_eq!(icon("passives/capacitors"), Some("part_capacitor"));
    assert_eq!(icon("passives/resistors"), Some("part_resistor"));
    assert_eq!(icon("ics/processors"), Some("part_ic"));
    assert_eq!(icon("nothing/here"), Some("part_other"));
    assert_eq!(icon(OTHER), Some("part_other"));
    // Every family of the grammar names a category of the tree.
    for family in &standards.generic.families {
        let path = family.category.as_deref().unwrap_or_default();
        assert!(
            standards.categories.find(path).is_some(),
            "{}",
            family.label
        );
    }
    Ok(())
}

#[test]
fn a_broken_tree_is_refused_naming_what_is_wrong() -> Result<()> {
    let other = serde_json::json!({"id": "other", "title": "Other"});
    let cases = [
        (
            serde_json::json!([{"id": "a", "title": "A"}, {"id": "a", "title": "A2"}, other]),
            serde_json::json!([]),
            "listed twice",
        ),
        (
            serde_json::json!([{"id": "a", "title": "A", "icon": "Part-IC"}, other]),
            serde_json::json!([]),
            "names an icon",
        ),
        (
            serde_json::json!([{"id": "Big", "title": "A"}, other]),
            serde_json::json!([]),
            "lower-case",
        ),
        (
            serde_json::json!([other, {"id": "a", "title": "A"}]),
            serde_json::json!([]),
            "must end in",
        ),
        (
            serde_json::json!([{"id": "a", "title": "A", "children": [{"id": "b", "title": "B",
                "children": [{"id": "c", "title": "C", "children": [{"id": "d", "title": "D"}]}]}]},
                other]),
            serde_json::json!([]),
            "deeper",
        ),
        (
            serde_json::json!([{"id": "a", "title": "A"}, other]),
            serde_json::json!([{"prefix": "R", "category": "a"}, {"prefix": "R", "category": "a"}]),
            "mapped twice",
        ),
        (
            serde_json::json!([{"id": "a", "title": "A"}, other]),
            serde_json::json!([{"prefix": "R", "category": "a/b"}]),
            "no category",
        ),
        (
            serde_json::json!([{"id": "a", "title": "A"}, other]),
            serde_json::json!([{"prefix": "r1", "category": "a"}]),
            "capital letters",
        ),
    ];
    for (categories, designators, why) in cases {
        let file = tree(serde_json::json!({
            "format": 1, "categories": categories, "designators": designators
        }))?;
        match file.check() {
            Err(e) => assert!(e.to_string().contains(why), "{why}: {e}"),
            Ok(()) => {
                return Err(Error::Invalid(format!(
                    "accepted a tree that should fail with ‘{why}’"
                )));
            }
        }
    }
    Ok(())
}

fn component(prefix: &str) -> ComponentDefinition {
    ComponentDefinition {
        designator_prefix: prefix.into(),
        ..Default::default()
    }
}

/// An imported board's parts carry nothing but their designator prefix;
/// each lands in the category the data maps it to, and an unknown prefix
/// in Other.
#[test]
fn an_imported_part_is_filed_by_its_designator_prefix() -> Result<()> {
    let standards = shipped()?;
    let library = Library::default();
    let cases = [
        ("R", "passives/resistors"),
        ("C", "passives/capacitors"),
        ("L", "magnetics/inductors"),
        ("U", "ics"),
        ("J", "connectors"),
        ("SW", "electromech/switches"),
        ("D", "discretes/diodes"),
        ("H", "board/mounting-holes"),
        ("LS", "electromech/sounders"),
        ("ZZ", OTHER),
        ("", OTHER),
    ];
    for (prefix, want) in cases {
        let got = file_component(&library, &standards, None, &component(prefix));
        assert_eq!(got, want, "prefix {prefix}");
    }
    Ok(())
}

/// A generated part is filed by its generator's family, whatever its
/// prefix says.
#[test]
fn a_generated_part_is_filed_by_its_family() -> Result<()> {
    let standards = shipped()?;
    let root = DefinitionRef {
        kind: DefinitionKind::Component,
        id: DefinitionId(7),
        revision: Revision(1),
    };
    let from = |part: &str| GeneratedFrom {
        generator: "chip".into(),
        version: 1,
        module: crate::ids::ContentHash([0; 32]),
        bundle: None,
        abi: 1,
        scope: GeneratorScope::Component,
        params: [("part".to_owned(), ParamValue::Text(part.into()))].into(),
        result: crate::ids::ContentHash([0; 32]),
        ids: Default::default(),
        preset: None,
    };
    let reference = ComponentRef::new(root.id, root.revision);
    for (part, want) in [("capacitor", "passives/capacitors"), ("led", "opto/leds")] {
        let library = Library {
            generated: vec![GeneratedPart {
                root,
                expansion: vec![root],
                from: from(part),
            }]
            .into(),
            ..Default::default()
        };
        let got = file_component(&library, &standards, Some(reference), &component("ZZ"));
        assert_eq!(got, want, "{part}");
    }
    Ok(())
}

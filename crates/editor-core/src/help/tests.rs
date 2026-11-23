use super::*;

#[test]
fn ids_round_trip_and_cannot_escape_the_library() {
    for id in ["entry/layout.add_via", "rule/fab-a/via_drill", "generator/no-lead#pitch"] {
        let parsed: HelpId = id.parse().expect("registry id");
        assert_eq!(parsed.as_str(), id);
    }
    for id in ["", "bad/topic", "guide/../x", "rule/fab", "guide/a/b", "panel/#x",
        "entry/layout#via", "generator/no-lead#", "guide/http:thing", "term/one two"] {
        assert!(id.parse::<HelpId>().is_err(), "{id}");
        assert!(serde_json::from_value::<HelpId>(id.into()).is_err());
    }
}

#[test]
fn inline_references_and_keys_are_typed() {
    let parts = inlines("Read [[panel/issues]] or press {key:route.width-next}.").expect("text");
    assert!(matches!(&parts[1], Inline::Reference(id) if id.as_str() == "panel/issues"));
    assert!(matches!(&parts[3], Inline::Key(id) if id.as_str() == "command/route.width-next"));
    for broken in ["[[term/x", "{key:}", "[[missing/a]]", "https://example.org/page"] {
        assert!(inlines(broken).is_err(), "{broken}");
    }
}

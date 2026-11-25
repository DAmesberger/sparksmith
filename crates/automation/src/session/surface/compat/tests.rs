use super::*;
use serde_json::json;

/// [`super::classify`] over `{"schema", "defs": [..]}` forms, the
/// definitions keyed by their index.
fn classify(old: &Value, new: &Value) -> Outcome {
    let defs = |form: &Value| -> Defs {
        form["defs"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, def)| (i.to_string(), def.clone()))
            .collect()
    };
    super::classify(&old["schema"], &defs(old), &new["schema"], &defs(new))
}

fn form(schema: Value) -> Value {
    json!({"schema": schema, "defs": []})
}

fn with_defs(schema: Value, defs: Value) -> Value {
    json!({"schema": schema, "defs": defs})
}

/// A parameter object with a nested `zone` definition, as schemars writes
/// one: `{at (required), zone: {priority}}`.
fn fill(zone_properties: Value, zone_required: Value) -> Value {
    with_defs(
        json!({"type": "object",
            "properties": {"at": {"type": "integer"}, "zone": {"$ref": "#/$defs/0"}},
            "required": ["at"]}),
        json!([{"type": "object", "properties": zone_properties, "required": zone_required}]),
    )
}

fn base_fill() -> Value {
    fill(
        json!({"priority": {"type": "integer"}}),
        json!(["priority"]),
    )
}

fn layer_enum(values: &[&str]) -> Value {
    form(json!({"type": "object",
        "properties": {"layer": {"type": "string", "enum": values}}, "required": ["layer"]}))
}

fn tagged(commands: &[(&str, Value)]) -> Value {
    let variants: Vec<Value> = commands
        .iter()
        .map(|(name, params)| {
            json!({"type": "object",
                "properties": {"command": {"type": "string", "const": name}, "params": params},
                "required": ["command", "params"]})
        })
        .collect();
    form(json!({"oneOf": variants}))
}

fn point() -> Value {
    json!({"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"]})
}

fn point_with_z() -> Value {
    json!({"type": "object", "properties": {"x": {"type": "integer"}, "z": {"type": "integer"}},
        "required": ["x"]})
}

/// The classification table (`docs/extension-surface.md#what-a-change-needs`).
#[test]
fn the_classification_table() {
    let cases: Vec<(&str, Value, Value, Change)> = vec![
        ("nothing changed", base_fill(), base_fill(), Change::Same),
        (
            "an optional field added, nested in a definition",
            base_fill(),
            fill(
                json!({"priority": {"type": "integer"}, "spokes": {"type": "integer", "default": 0}}),
                json!(["priority"]),
            ),
            Change::Minor,
        ),
        (
            "an optional field added at the top",
            form(
                json!({"type": "object", "properties": {"a": {"type": "integer"}}, "required": ["a"]}),
            ),
            form(json!({"type": "object",
                "properties": {"a": {"type": "integer"}, "b": {"type": ["string", "null"]}},
                "required": ["a"]})),
            Change::Minor,
        ),
        (
            "a required field added, nested",
            base_fill(),
            fill(
                json!({"priority": {"type": "integer"}, "spokes": {"type": "integer"}}),
                json!(["priority", "spokes"]),
            ),
            Change::Major,
        ),
        (
            "a field removed, nested",
            base_fill(),
            fill(json!({}), json!([])),
            Change::Major,
        ),
        (
            "a field renamed",
            base_fill(),
            fill(json!({"rank": {"type": "integer"}}), json!(["rank"])),
            Change::Major,
        ),
        (
            "an optional field made required",
            fill(json!({"priority": {"type": "integer"}}), json!([])),
            base_fill(),
            Change::Major,
        ),
        (
            "a required field made optional",
            base_fill(),
            fill(json!({"priority": {"type": "integer"}}), json!([])),
            Change::Minor,
        ),
        (
            "a field retyped",
            base_fill(),
            fill(json!({"priority": {"type": "string"}}), json!(["priority"])),
            Change::Major,
        ),
        (
            "integer widened to number",
            base_fill(),
            fill(json!({"priority": {"type": "number"}}), json!(["priority"])),
            Change::Minor,
        ),
        (
            "a field made nullable (Option)",
            base_fill(),
            fill(
                json!({"priority": {"type": ["integer", "null"]}}),
                json!(["priority"]),
            ),
            Change::Minor,
        ),
        (
            "a nullable field made non-null",
            fill(
                json!({"priority": {"type": ["integer", "null"]}}),
                json!(["priority"]),
            ),
            base_fill(),
            Change::Major,
        ),
        (
            "an enum widened",
            layer_enum(&["top", "bottom"]),
            layer_enum(&["top", "bottom", "inner1"]),
            Change::Minor,
        ),
        (
            "an enum narrowed",
            layer_enum(&["top", "bottom"]),
            layer_enum(&["top"]),
            Change::Major,
        ),
        (
            "a union variant added (a new operation)",
            tagged(&[("a", point())]),
            tagged(&[("a", point()), ("b", point())]),
            Change::Minor,
        ),
        (
            "a union variant removed",
            tagged(&[("a", point()), ("b", point())]),
            tagged(&[("a", point())]),
            Change::Major,
        ),
        (
            "an optional field added inside one variant",
            tagged(&[("a", point()), ("b", point())]),
            tagged(&[("a", point()), ("b", point_with_z())]),
            Change::Minor,
        ),
        (
            "variants reordered",
            tagged(&[("a", point()), ("b", point_with_z())]),
            tagged(&[("b", point_with_z()), ("a", point())]),
            Change::Same,
        ),
        (
            "a range narrowed",
            form(json!({"type": "integer", "minimum": 0, "maximum": 10})),
            form(json!({"type": "integer", "minimum": 1, "maximum": 10})),
            Change::Major,
        ),
        (
            "a range widened",
            form(json!({"type": "integer", "minimum": 1, "maximum": 10})),
            form(json!({"type": "integer", "minimum": 0})),
            Change::Minor,
        ),
        (
            "a default changed",
            form(json!({"type": "integer", "default": 1})),
            form(json!({"type": "integer", "default": 2})),
            Change::Major,
        ),
        (
            "unknown properties now refused",
            form(json!({"type": "object", "properties": {}})),
            form(json!({"type": "object", "properties": {}, "additionalProperties": false})),
            Change::Major,
        ),
        (
            "a string became a path",
            form(json!({"type": "string"})),
            form(json!({"type": "string", "x-kind": "path"})),
            Change::Major,
        ),
        (
            "a format added",
            form(json!({"type": "string"})),
            form(json!({"type": "string", "format": "uuid"})),
            Change::Major,
        ),
    ];
    for (what, old, new, expected) in cases {
        let outcome = classify(&old, &new);
        assert_eq!(outcome.change, expected, "{what}: {:?}", outcome.reasons);
        assert_eq!(
            outcome.change == Change::Major,
            !outcome.reasons.is_empty(),
            "{what}: a break names its reason, nothing else does"
        );
    }
}

/// A recursive definition compares without looping, and a change inside
/// it is found.
#[test]
fn recursive_definitions_terminate() {
    let tree = |leaf: Value| {
        with_defs(
            json!({"$ref": "#/$defs/0"}),
            json!([{"type": "object", "properties": {
                "children": {"type": "array", "items": {"$ref": "#/$defs/0"}},
                "leaf": leaf}}]),
        )
    };
    let old = tree(json!({"type": "integer"}));
    assert_eq!(classify(&old, &old).change, Change::Same);
    assert_eq!(
        classify(&old, &tree(json!({"type": "string"}))).change,
        Change::Major
    );
}

/// A break names where it is.
#[test]
fn a_break_names_its_place() {
    let outcome = classify(
        &base_fill(),
        &fill(json!({"priority": {"type": "string"}}), json!(["priority"])),
    );
    assert!(
        outcome.reasons.iter().any(|r| r.contains(".zone.priority")),
        "{:?}",
        outcome.reasons
    );
}

/// A reply as published: its objects opened (`reply_schema::open`), as a
/// stored type with `deny_unknown_fields` is.
fn reply(schema: Value) -> Value {
    let mut schema = schema;
    crate::session::reply_schema::open(&mut schema);
    form(schema)
}

/// An untagged reply union whose second variant gains a tagged variant
/// deep inside is a major change to that variant, and is told as one: of
/// two variants that both change it a major way, it is compared with
/// itself, not with the first (whose fields it never had, which read as
/// "no longer returned").
#[test]
fn an_untagged_variant_that_changed_is_compared_with_itself() {
    let union = |kinds: &[&str]| {
        let refs: Vec<Value> = kinds
            .iter()
            .map(|k| {
                json!({"type": "object", "properties": {"kind": {"const": k}},
                    "required": ["kind"]})
            })
            .collect();
        json!({"type": "array", "items": {"anyOf": [
            {"type": "object", "properties": {"id": {"type": "integer"},
                "name": {"type": "string"}}, "required": ["id", "name"]},
            {"type": "object", "properties": {"check": {"type": "object",
                "properties": {"objects": {"type": "array", "items": {"oneOf": refs}}},
                "required": ["objects"]}},
                "required": ["check"]}
        ]}})
    };
    let old = reply(union(&["track", "via"]));
    let new = reply(union(&["track", "via", "board_text"]));
    let found = classify_reply(&old, &new);
    assert_eq!(found.change, Change::Major, "{found:?}");
    assert!(
        found
            .reasons
            .iter()
            .all(|r| r.contains("|1.check.objects[]|board_text: new reply variant")),
        "{:?}",
        found.reasons
    );
}

fn classify_reply(old: &Value, new: &Value) -> Outcome {
    super::classify_reply(&old["schema"], &Defs::new(), &new["schema"], &Defs::new())
}

/// A stored instance as a reply embeds it: closed by the file format,
/// published open.
fn instance(extra: &[(&str, Value)], required: &[&str]) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert("id".into(), json!({"type": "integer"}));
    properties.insert("designator".into(), json!({"type": "string"}));
    for (k, v) in extra {
        properties.insert((*k).to_owned(), v.clone());
    }
    let mut required_all = vec!["id", "designator"];
    required_all.extend(required);
    json!({"type": "object", "properties": properties, "required": required_all,
        "additionalProperties": false})
}

/// Reply schemas are open objects: a new optional reply field (a stored
/// type's new skipped-when-default field) is a minor change, though the
/// stored type itself refuses unknown fields. A new required field, a
/// removed field and a changed type stay major. Parameters stay strict.
#[test]
fn a_new_optional_reply_field_is_minor_and_parameters_stay_strict() {
    let old = reply(instance(&[], &[]));
    assert_eq!(
        reply(instance(&[], &[]))["schema"].get("additionalProperties"),
        None,
        "published open"
    );
    let flag = json!({"type": "boolean", "default": false});
    let optional = reply(instance(&[("bom_excluded", flag.clone())], &[]));
    assert_eq!(classify_reply(&old, &optional).change, Change::Minor);
    let required = reply(instance(
        &[("bom_excluded", flag.clone())],
        &["bom_excluded"],
    ));
    assert_eq!(classify_reply(&old, &required).change, Change::Minor);
    let mut without = instance(&[], &[]);
    without["properties"]
        .as_object_mut()
        .map(|p| p.remove("designator"));
    without["required"] = json!(["id"]);
    let removed = reply(without);
    assert_eq!(classify_reply(&old, &removed).change, Change::Major);
    let mut retyped = instance(&[], &[]);
    retyped["properties"]["designator"] = json!({"type": "integer"});
    assert_eq!(classify_reply(&old, &reply(retyped)).change, Change::Major);
    // A new required field a reader may now see where a field it relied on
    // was optional before.
    let was_optional = reply(instance(&[("bom_excluded", flag.clone())], &[]));
    let now_dropped = reply(instance(&[], &[]));
    assert_eq!(
        classify_reply(&was_optional, &now_dropped).change,
        Change::Major,
        "a documented field no longer returned"
    );
    // Parameters keep the stored type's closed form.
    let params_old = form(instance(&[], &[]));
    let params_new = form(instance(&[("bom_excluded", flag)], &["bom_excluded"]));
    assert_eq!(classify(&params_old, &params_new).change, Change::Major);
}

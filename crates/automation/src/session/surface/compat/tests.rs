use super::*;
use serde_json::json;

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

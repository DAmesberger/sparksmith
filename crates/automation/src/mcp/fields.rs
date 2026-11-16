//! Field names checked against an entry's schema before the call, so a
//! model that sends a wrong field learns the right one in the same reply
//! (E2E run 12: `library.search {"text": …}` was refused twice with only
//! serde's list of names). A refusal names the field it did not know, the
//! nearest field (by spelling, or the one missing required field of the
//! same type), every field with its type, and a call that works.
//!
//! Only objects whose schema closes them (`additionalProperties: false`,
//! the entry types' `deny_unknown_fields`) are checked, through plain
//! nested objects; anything else (tagged enums, maps, free JSON) is left to
//! the entry's own parameter type, as before.
use super::lenient::deref;
use crate::session::discovery;
use crate::session::metadata::distance;
use serde_json::{Map, Value, json};

/// How deep nested objects are followed.
const MAX_DEPTH: usize = 6;
/// Fields listed in one refusal at most.
const MAX_LISTED: usize = 16;
/// The field the call's envelope fills.
const REVISION: &str = "expected_revision";
/// The reply flag every request takes beside its parameters.
const VERBOSE: &str = "verbose";

/// How the entry is sent, for the example: the verb and whether it is an
/// operation (inside `apply`'s steps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    Query,
    Apply,
    Act,
    Show,
}

/// `Ok` when every field of `params` is one `name`'s schema knows and every
/// required one is there; else the refusal's text.
pub fn check(name: &str, params: &Value, sent: Sent) -> Result<(), String> {
    let Some((schema, root)) = discovery::params_schema(name) else {
        return Ok(());
    };
    let mut problems = Vec::new();
    walk(params, schema, root, "", 0, &mut problems);
    let Some(first) = problems.into_iter().next() else {
        return Ok(());
    };
    let (at, schema, said) = first;
    let fields = listed(schema, root);
    let where_ = if at.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {at}")
    };
    let mut text = format!("{where_} {said}. Its fields: {fields}.");
    if at.is_empty() {
        text.push_str(&format!(
            " Send it like {}.",
            example(name, schema, root, sent)
        ));
    }
    Err(text)
}

/// The first problems of `value` against `schema`, with where they are and
/// the object schema they are in.
fn walk<'a>(
    value: &Value,
    schema: &'a Value,
    root: &'a Value,
    at: &str,
    depth: usize,
    problems: &mut Vec<(String, &'a Value, String)>,
) {
    if depth > MAX_DEPTH || !problems.is_empty() {
        return;
    }
    let schema = deref(schema, root);
    let (Some(properties), Some(given)) = (schema["properties"].as_object(), value.as_object())
    else {
        return;
    };
    if schema.get("additionalProperties") != Some(&Value::Bool(false)) {
        return;
    }
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let missing: Vec<&str> = required
        .iter()
        .copied()
        // The envelope's revision fills an entry's own (project.undo).
        .filter(|r| !given.contains_key(*r) && !(at.is_empty() && *r == REVISION))
        .collect();
    for (key, inner) in given {
        if properties.contains_key(key) || (at.is_empty() && key == VERBOSE) {
            continue;
        }
        let mut said = format!("has no field `{key}`");
        if let Some(near) = nearest(key, inner, properties, &missing, root) {
            said.push_str(&format!(" (did you mean `{near}`?)"));
        }
        problems.push((at.to_owned(), schema, said));
        return;
    }
    if !missing.is_empty() {
        let names: Vec<String> = missing.iter().map(|m| format!("`{m}`")).collect();
        problems.push((at.to_owned(), schema, format!("needs {}", names.join(", "))));
        return;
    }
    for (key, inner) in given {
        if let Some(property) = properties.get(key) {
            walk(
                inner,
                property,
                root,
                &format!("{at}/{key}"),
                depth + 1,
                problems,
            );
        }
    }
}

/// The field `key` most likely meant: a field spelled nearly alike, else
/// the one missing required field that takes `value`'s type.
fn nearest(
    key: &str,
    value: &Value,
    properties: &Map<String, Value>,
    missing: &[&str],
    root: &Value,
) -> Option<String> {
    let chars: Vec<char> = key.to_lowercase().chars().collect();
    let limit = (chars.len() / 3).max(2);
    let spelled = properties
        .keys()
        .map(|p| (distance(&chars, &p.to_lowercase()), p))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, p)| p.clone());
    if spelled.is_some() {
        return spelled;
    }
    let kind = json_type(value);
    let typed: Vec<&&str> = missing
        .iter()
        .filter(|m| {
            properties
                .get(**m)
                .is_some_and(|p| type_name(p, root).split(" or ").any(|t| t == kind))
        })
        .collect();
    match typed.as_slice() {
        [only] => Some((**only).to_owned()),
        _ => None,
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Every field of an object schema with its type, required ones marked.
fn listed(schema: &Value, root: &Value) -> String {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let Some(properties) = schema["properties"].as_object() else {
        return "none".into();
    };
    if properties.is_empty() {
        return "none".into();
    }
    let mut out: Vec<String> = properties
        .iter()
        .take(MAX_LISTED)
        .map(|(name, p)| {
            let mut line = format!("`{name}` ({}", type_name(p, root));
            if required.contains(&name.as_str()) {
                line.push_str(", required");
            }
            line.push(')');
            line
        })
        .collect();
    if properties.len() > MAX_LISTED {
        out.push(format!("{} more", properties.len() - MAX_LISTED));
    }
    out.join(", ")
}

/// A short type of a property: its JSON type(s), a definition's name, or
/// "value".
fn type_name(schema: &Value, root: &Value) -> String {
    let named = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/$defs/"))
        .map(str::to_owned);
    let schema = deref(schema, root);
    let types: Vec<&str> = match &schema["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts
            .iter()
            .filter_map(Value::as_str)
            .filter(|t| *t != "null")
            .collect(),
        _ => Vec::new(),
    };
    if !types.is_empty() {
        return types.join(" or ");
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(options) = schema[key].as_array() {
            let given: Vec<&Value> = options.iter().filter(|o| o["type"] != "null").collect();
            if let [only] = given.as_slice() {
                return type_name(only, root);
            }
        }
    }
    named.unwrap_or_else(|| "value".into())
}

/// A call of `name` with its required fields filled with placeholders.
fn example(name: &str, schema: &Value, root: &Value, sent: Sent) -> String {
    let mut params = Map::new();
    for field in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|f| *f != REVISION)
    {
        let placeholder = match schema["properties"]
            .get(field)
            .map(|p| type_name(p, root))
            .as_deref()
            .and_then(|t| t.split(" or ").next())
        {
            Some("integer") => json!(1),
            Some("number") => json!(1.0),
            Some("boolean") => json!(true),
            Some("array") => json!([]),
            Some("string") => json!("…"),
            _ => json!({}),
        };
        params.insert(field.to_owned(), placeholder);
    }
    let params = Value::Object(params);
    match sent {
        Sent::Apply => format!(
            "apply {}",
            json!({"expected_revision": "<revision>", "operations": [{"op": name, "params": params}]})
        ),
        Sent::Query => format!("query {}", json!({"name": name, "params": params})),
        Sent::Act => format!("act {}", json!({"name": name, "params": params})),
        Sent::Show => format!("show {}", json!({"name": name, "params": params})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_field_name_is_refused_with_the_right_one_and_a_call() {
        // E2E run 12: the model sent `text` to library.search twice.
        let why = check(
            "library.search",
            &json!({"text": "capacitor 0603"}),
            Sent::Query,
        )
        .err()
        .unwrap_or_default();
        assert!(
            why.starts_with("library.search has no field `text` (did you mean `query`?)"),
            "{why}"
        );
        assert!(why.contains("`query` (string, required)"), "{why}");
        assert!(
            why.contains(r#"query {"name":"library.search","params":{"query":"…"}}"#),
            "{why}"
        );
        // A near spelling is named.
        let why = check("library.search", &json!({"querry": "x"}), Sent::Query)
            .err()
            .unwrap_or_default();
        assert!(why.contains("did you mean `query`?"), "{why}");
        // Right fields pass; a missing required one is named.
        assert_eq!(
            check(
                "library.search",
                &json!({"query": "x", "limit": 5}),
                Sent::Query
            ),
            Ok(())
        );
        let why = check("library.search", &json!({}), Sent::Query)
            .err()
            .unwrap_or_default();
        assert!(why.starts_with("library.search needs `query`"), "{why}");
    }

    #[test]
    fn an_operation_s_example_is_an_apply_call() {
        let why = check(
            "layout.set_board_size",
            &json!({"widht": 30.0}),
            Sent::Apply,
        )
        .err()
        .unwrap_or_default();
        assert!(why.contains("apply {"), "{why}");
        assert!(why.contains(r#""op":"layout.set_board_size""#), "{why}");
    }

    #[test]
    fn an_entry_without_a_closed_schema_is_left_to_its_type() {
        assert_eq!(
            check("no.such.entry", &json!({"x": 1}), Sent::Query),
            Ok(())
        );
    }
}

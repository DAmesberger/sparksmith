//! The verbs' argument reader is lenient where weak models are predictable
//! (extension ABI §2.7): every client gets it, and it is tested once.
//!
//! - **Arrays and objects sent as JSON text.** Some models send a long list
//!   as a string (a 4 000-character `edits` list was refused nine times for
//!   it). Where the schema wants an array or an object and not text, text
//!   that starts with `[` or `{` is read as JSON. Text that is not valid
//!   JSON is refused with where it broke: a server's tool parser leaves a
//!   long call with unbalanced brackets as text, and the model needs that,
//!   not "expected a sequence", to fix it.
//! - **Integers sent as digits.** An integer a double would round (a plan's
//!   19-digit base, say) reaches a model as text, and the model may send it
//!   back that way. Where the schema wants an integer and not text, digits
//!   are read as the integer.
//!
//! Nothing else is guessed: a value that is still the wrong type after this
//! is refused by the entry's own parameter type, as before.
use crate::session::discovery;
use serde_json::Value;

/// How deep the reader follows a schema.
const MAX_DEPTH: usize = 24;

/// Read `params` of the entry `name` leniently against its schema.
pub fn read_params(name: &str, params: &mut Value) -> Result<(), String> {
    match discovery::params_schema(name) {
        Some((schema, root)) => read(params, schema, root),
        None => Ok(()),
    }
}

/// Read `value` leniently against `schema`, whose `$ref`s name definitions
/// of `root` (`#/$defs/…`).
pub fn read(value: &mut Value, schema: &Value, root: &Value) -> Result<(), String> {
    walk(value, schema, root, "", 0)
}

fn walk(
    value: &mut Value,
    schema: &Value,
    root: &Value,
    at: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let schema = deref(schema, root);
    if let Value::String(text) = value {
        let wants = Wants::of(schema, root);
        if !wants.string {
            let trimmed = text.trim();
            if (wants.array || wants.object)
                && (trimmed.starts_with('[') || trimmed.starts_with('{'))
            {
                let parsed = parse(text, at, if wants.array { "array" } else { "object" })?;
                if (parsed.is_array() && wants.array) || (parsed.is_object() && wants.object) {
                    *value = parsed;
                }
            } else if wants.integer {
                if let Ok(n) = trimmed.parse::<u64>() {
                    *value = Value::from(n);
                } else if let Ok(n) = trimmed.parse::<i64>() {
                    *value = Value::from(n);
                }
            }
        }
    }
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if let Some(property) = property(schema, root, key, 0) {
                    walk(inner, property, root, &format!("{at}/{key}"), depth + 1)?;
                }
            }
        }
        Value::Array(items) => {
            if let Some(item) = items_schema(schema, root) {
                for (k, inner) in items.iter_mut().enumerate() {
                    walk(inner, item, root, &format!("{at}/{k}"), depth + 1)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The text as JSON, or where it broke.
fn parse(text: &str, at: &str, kind: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(text).map_err(|e| {
        let before: usize = text
            .split_inclusive('\n')
            .take(e.line().saturating_sub(1))
            .map(str::len)
            .sum();
        let here = (before + e.column().saturating_sub(1)).min(text.len());
        let from = (here.saturating_sub(60)..=here)
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(0);
        let to = (here..=text.len().min(here + 20))
            .rev()
            .find(|i| text.is_char_boundary(*i))
            .unwrap_or(here);
        let key = at.rsplit('/').next().filter(|k| !k.is_empty()).unwrap_or("the value");
        format!(
            "{key} arrived as text that is not valid JSON ({e}, near …{}⟨here⟩{}…). Count the brackets and send {key} as a JSON {kind}, not a string; a smaller call is easier to get right",
            text.get(from..here).unwrap_or_default(),
            text.get(here..to).unwrap_or_default()
        )
    })
}

/// What a schema accepts, across its alternatives.
#[derive(Default)]
struct Wants {
    string: bool,
    array: bool,
    object: bool,
    integer: bool,
}

impl Wants {
    fn of(schema: &Value, root: &Value) -> Self {
        let mut wants = Self::default();
        wants.add(schema, root, 0);
        wants
    }

    fn add(&mut self, schema: &Value, root: &Value, depth: usize) {
        if depth > 4 {
            return;
        }
        let schema = deref(schema, root);
        match &schema["type"] {
            Value::String(t) => self.mark(t),
            Value::Array(types) => types
                .iter()
                .filter_map(Value::as_str)
                .for_each(|t| self.mark(t)),
            _ => {}
        }
        if schema.get("properties").is_some() {
            self.object = true;
        }
        if schema.get("const").is_some_and(Value::is_string)
            || schema
                .get("enum")
                .is_some_and(|e| e.as_array().is_some_and(|e| e.iter().any(Value::is_string)))
        {
            self.string = true;
        }
        // A schema that says nothing takes anything, text included.
        if schema.as_bool() == Some(true)
            || schema.as_object().is_some_and(|o| {
                o.keys()
                    .all(|k| matches!(k.as_str(), "description" | "title" | "default"))
            })
        {
            self.string = true;
        }
        for key in ["anyOf", "oneOf", "allOf"] {
            for option in schema[key].as_array().into_iter().flatten() {
                self.add(option, root, depth + 1);
            }
        }
    }

    fn mark(&mut self, t: &str) {
        match t {
            "string" => self.string = true,
            "array" => self.array = true,
            "object" => self.object = true,
            "integer" => self.integer = true,
            _ => {}
        }
    }
}

/// A `$ref` followed into `root`'s definitions.
fn deref<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    let mut schema = schema;
    for _ in 0..8 {
        let Some(name) = schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix("#/$defs/"))
        else {
            break;
        };
        match root["$defs"].get(name) {
            Some(def) => schema = def,
            None => break,
        }
    }
    schema
}

/// The schema of `key` in an object `schema`: its own property, or the
/// property of the first alternative that has it.
fn property<'a>(schema: &'a Value, root: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 4 {
        return None;
    }
    let schema = deref(schema, root);
    if let Some(found) = schema["properties"].get(key) {
        return Some(found);
    }
    if let Some(extra) = schema.get("additionalProperties").filter(|a| a.is_object()) {
        return Some(extra);
    }
    ["anyOf", "oneOf", "allOf"].iter().find_map(|k| {
        schema[*k]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|option| property(option, root, key, depth + 1))
    })
}

/// The schema of an array `schema`'s items.
fn items_schema<'a>(schema: &'a Value, root: &'a Value) -> Option<&'a Value> {
    let schema = deref(schema, root);
    if let Some(items) = schema.get("items").filter(|i| i.is_object()) {
        return Some(items);
    }
    ["anyOf", "oneOf"].iter().find_map(|k| {
        schema[*k]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|option| deref(option, root).get("items").filter(|i| i.is_object()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "edits": {"type": "array", "items": {"type": "object"}},
                "report": {"$ref": "#/$defs/Report"},
                "query": {"type": "string"},
                "base": {"type": ["string", "null"]},
                "revision": {"type": "integer"},
                "any": true,
            },
            "$defs": {"Report": {"type": "object", "properties": {
                "values": {"type": "array", "items": {"type": "object"}},
            }}},
        })
    }

    #[test]
    fn arrays_and_objects_sent_as_text_are_read_as_meant() {
        let schema = schema();
        let mut value = json!({"edits": "[{\"id\": 5}]", "query": "[1]", "any": "[2]"});
        assert_eq!(read(&mut value, &schema, &schema), Ok(()));
        assert_eq!(
            value,
            json!({"edits": [{"id": 5}], "query": "[1]", "any": "[2]"})
        );
        // Text that is not the declared kind stays as it came.
        let mut value = json!({"edits": "{}"});
        assert_eq!(read(&mut value, &schema, &schema), Ok(()));
        assert_eq!(value, json!({"edits": "{}"}));
        // Nested, through a definition.
        let mut value = json!({"report": {"values": "[{\"item\": \"pitch\"}]"}});
        assert_eq!(read(&mut value, &schema, &schema), Ok(()));
        assert_eq!(value, json!({"report": {"values": [{"item": "pitch"}]}}));
        let mut value = json!({"report": "{\"values\": []}"});
        assert_eq!(read(&mut value, &schema, &schema), Ok(()));
        assert_eq!(value, json!({"report": {"values": []}}));
    }

    #[test]
    fn broken_json_text_is_refused_with_where_it_broke() {
        let schema = schema();
        let mut value = json!({"edits": "[{\"id\": 5, \"pads\": [{\"size\": [0.6, 1.1]}}]"});
        let why = read(&mut value, &schema, &schema).err().unwrap_or_default();
        assert!(
            why.contains("edits arrived as text that is not valid JSON")
                && why.contains("[0.6, 1.1]}⟨here⟩}]"),
            "{why}"
        );
    }

    #[test]
    fn integers_sent_as_digits_are_read_where_text_is_not_wanted() {
        let schema = schema();
        let mut value = json!({"revision": "12", "base": "2049172786040315623", "query": "7"});
        assert_eq!(read(&mut value, &schema, &schema), Ok(()));
        assert_eq!(
            value,
            json!({"revision": 12, "base": "2049172786040315623", "query": "7"})
        );
        // Not digits: left for the entry's type to refuse.
        let mut value = json!({"revision": "twelve"});
        assert_eq!(read(&mut value, &schema, &schema), Ok(()));
        assert_eq!(value, json!({"revision": "twelve"}));
    }

    #[test]
    fn an_entry_s_parameters_are_read_against_its_own_schema() {
        let mut params = json!({"edits": "[{\"id\": 5, \"meta\": {}, \"definition\": {}}]"});
        assert_eq!(read_params("library.edit", &mut params), Ok(()));
        assert!(params["edits"].is_array(), "{params}");
        let mut params = json!({"pins": "[{\"row\": \"1\"}]"});
        assert_eq!(read_params("assist.pinout_edit", &mut params), Ok(()));
        assert!(params["pins"].is_array(), "{params}");
    }
}

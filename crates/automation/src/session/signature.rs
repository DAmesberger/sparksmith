//! The signature tier in docs/design/extension-abi-design.md §2.2.
//! Inputs and results come from the published schemas. Required inputs are never elided.
use super::discovery::Entry;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_LINE_BYTES: usize = 160;

#[derive(Clone, Copy)]
struct Field {
    ty: &'static str,
    required: bool,
}
type Fields = BTreeMap<String, Field>;

fn scalar(ty: &str) -> &'static str {
    match ty {
        "string" => "text",
        "integer" => "int",
        "number" => "num",
        "boolean" => "bool",
        "array" => "[…]",
        "object" => "{…}",
        _ => "ref",
    }
}

fn field_type(schema: &Value) -> &'static str {
    if schema
        .get(sparksmith_core::permission::PATH_KIND)
        .and_then(Value::as_str)
        == Some("path")
    {
        return "path";
    }
    if let Some(reference) = schema["$ref"].as_str() {
        return match reference.rsplit('/').next().unwrap_or_default() {
            "FilePath" => "path",
            "Layer" | "CopperLayer" => "layer",
            "Length" => "len",
            "Angle" => "angle",
            name if name.ends_with("Id") => "id",
            _ => "ref",
        };
    }
    if let Some(options) = schema["anyOf"].as_array() {
        let mut values = options.iter().filter(|element| element["type"] != "null");
        if let Some(value) = values.next()
            && values.next().is_none()
        {
            return field_type(value);
        }
    }
    let ty = schema["type"].as_str().or_else(|| {
        schema["type"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .find(|ty| *ty != "null")
    });
    ty.map_or("ref", scalar)
}

fn object_fields(schema: &Value) -> Fields {
    let mut fields = Fields::new();
    if let Some(properties) = schema["properties"].as_object() {
        for (name, value) in properties {
            fields.insert(
                name.clone(),
                Field {
                    ty: field_type(value),
                    required: false,
                },
            );
        }
    }
    for field in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        fields
            .entry(field.to_owned())
            .or_insert(Field {
                ty: "ref",
                required: true,
            })
            .required = true;
    }
    fields
}

fn merge(fields: &mut Fields, others: Fields) {
    for (name, other) in others {
        let field = fields.entry(name).or_insert(other);
        field.required |= other.required;
        if field.ty != other.ty {
            field.ty = "ref";
        }
    }
}

/// A tagged-union field is required only when required by every variant.
/// Follow top-level references, without expanding the types of nested fields.
fn inputs(schema: &Value, root: &Value, visiting: &mut BTreeSet<String>) -> Fields {
    let mut fields = object_fields(schema);
    if let Some(name) = schema["$ref"]
        .as_str()
        .and_then(|s| s.strip_prefix(super::schema_walk::DEFS))
        && visiting.insert(name.to_owned())
    {
        merge(&mut fields, inputs(&root["$defs"][name], root, visiting));
        visiting.remove(name);
    }
    if let Some(all) = schema["allOf"].as_array() {
        for part in all {
            merge(&mut fields, inputs(part, root, visiting));
        }
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(variants) = schema[keyword].as_array() {
            let mut combined: BTreeMap<String, (&'static str, usize)> = BTreeMap::new();
            for variant in variants {
                for (name, field) in inputs(variant, root, visiting) {
                    let (ty, count) = combined.entry(name).or_insert((field.ty, 0));
                    if *ty != field.ty {
                        *ty = "ref";
                    }
                    *count += usize::from(field.required);
                }
            }
            merge(
                &mut fields,
                combined
                    .into_iter()
                    .map(|(name, (ty, count))| {
                        (
                            name,
                            Field {
                                ty,
                                required: count == variants.len(),
                            },
                        )
                    })
                    .collect(),
            );
        }
    }
    fields
}

/// Summarize the successful wire reply. For a report, show the payload type.
/// Opaque unions and large records are available through the full contract.
fn result_hint(schema: &Value) -> String {
    let schema = schema["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix(super::schema_walk::DEFS))
        .and_then(|name| schema["$defs"].get(name))
        .unwrap_or(schema);
    let required = schema["required"].as_array();
    for name in ["result", "revision"] {
        if required.is_some_and(|fields| fields.iter().any(|field| field == name))
            && let Some(value) = schema["properties"].get(name)
        {
            return format!("{{{name}:{},…}}", field_type(value));
        }
    }
    field_type(schema).into()
}

pub(super) fn line(entry: &Entry) -> Option<String> {
    let fields = inputs(&entry.params, entry.root, &mut BTreeSet::new());
    let mut args: Vec<String> = fields
        .iter()
        .filter(|(_, field)| field.required)
        .map(|(name, field)| format!("{name}:{}", field.ty))
        .collect();
    let optional: Vec<_> = fields.iter().filter(|(_, field)| !field.required).collect();
    let result_type = result_hint(entry.reply_schema());
    let render = |args: &[String], omitted: bool, result_type: &str| {
        let mut list = args.join(",");
        if omitted {
            if !list.is_empty() {
                list.push(',');
            }
            list.push('…');
        }
        format!(
            "{}{{{list}}}->{result_type} {}",
            entry.name,
            entry.class.name()
        )
    };
    let mut hint = result_type.as_str();
    let mut result = render(&args, !optional.is_empty(), hint);
    if result.len() > MAX_LINE_BYTES {
        hint = "ref";
        result = render(&args, !optional.is_empty(), hint);
    }
    // If a future entry exhausts its space, keep every required name before
    // dropping type hints. An impossible addition fails the catalog test and
    // produces a named error on the optional line surface.
    if result.len() > MAX_LINE_BYTES {
        args = fields
            .iter()
            .filter(|(_, field)| field.required)
            .map(|(name, _)| name.clone())
            .collect();
        result = render(&args, !optional.is_empty(), hint);
        if result.len() > MAX_LINE_BYTES {
            return None;
        }
    }
    for (i, (name, field)) in optional.iter().enumerate() {
        args.push(format!("{name}?:{}", field.ty));
        let candidate = render(&args, i + 1 < optional.len(), hint);
        if candidate.len() > MAX_LINE_BYTES {
            break;
        }
        result = candidate;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::discovery;
    use serde_json::json;

    /// Determine required names independently of the formatter, starting
    /// from each original input and resolving its top-level reference chain.
    fn expected_names(
        schema: &Value,
        root: &Value,
        visiting: &mut BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut expected: BTreeSet<String> = schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        if let Some(name) = schema["$ref"]
            .as_str()
            .and_then(|p| p.strip_prefix("#/$defs/"))
            && visiting.insert(name.to_owned())
        {
            expected.extend(expected_names(&root["$defs"][name], root, visiting));
            visiting.remove(name);
        }
        for part in schema["allOf"].as_array().into_iter().flatten() {
            expected.extend(expected_names(part, root, visiting));
        }
        for keyword in ["oneOf", "anyOf"] {
            if let Some((first, rest)) = schema[keyword]
                .as_array()
                .and_then(|arms| arms.split_first())
            {
                let mut common = expected_names(first, root, visiting);
                for arm in rest {
                    let required = expected_names(arm, root, visiting);
                    common.retain(|k| required.contains(k));
                }
                expected.extend(common);
            }
        }
        expected
    }

    #[test]
    fn every_reachable_signature_round_trips_its_required_input_names()
    -> Result<(), Box<dyn std::error::Error>> {
        for entry in crate::mcp::catalog::reachable() {
            let (schema, root) = discovery::params_schema(&entry.name).ok_or("missing input")?;
            let expected = expected_names(schema, root, &mut BTreeSet::new());
            let card = entry.line_card()?;
            let signature = card["line"].as_str().ok_or("missing line")?;
            assert!(
                signature.len() <= MAX_LINE_BYTES && !signature.contains(['\r', '\n']),
                "{}",
                entry.name
            );
            let (name, tail) = signature.split_once('{').ok_or("missing entry")?;
            let (args, result) = tail.split_once("}->").ok_or("missing result")?;
            let (result, class) = result.rsplit_once(' ').ok_or("missing class")?;
            assert!(!result.is_empty());
            if name == "schematic.pins" {
                assert_eq!(result, "{result:[…],…}");
            }
            assert_eq!(name, entry.name);
            assert_eq!(class, entry.class.name());
            let parsed: BTreeSet<_> = args
                .split(',')
                .filter(|s| !s.is_empty() && *s != "…")
                .map(|field| field.split(':').next().unwrap_or_default())
                .filter(|f| !f.ends_with('?'))
                .map(str::to_owned)
                .collect();
            assert_eq!(parsed, expected, "{}", entry.name);
        }
        Ok(())
    }

    #[test]
    fn union_preserves_conditional_and_common_inputs() {
        let union = json!({"oneOf":[{"properties":{"op":{"type":"string"},"left":{"type":"integer"}},"required":["op","left"]},
                                  {"properties":{"op":{"type":"string"},"right":{"type":"boolean"}},"required":["op","right"]}]});
        let fields = inputs(&union, &union, &mut BTreeSet::new());
        assert!(fields["op"].required);
        assert!(!fields["left"].required && !fields["right"].required);
        assert_eq!(fields["left"].ty, "int");
    }
}

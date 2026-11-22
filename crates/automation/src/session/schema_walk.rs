//! JSON Schema traversal shared by discovery and the compatibility lock.
use serde_json::Value;

/// Schema keywords that change no call: doc comments and type names.
const IGNORED: &[&str] = &["description", "title", "$comment", "examples", "$schema"];
pub(super) const DEFS: &str = "#/$defs/";

/// A schema's keywords have meaning only on a schema object. Map keys in
/// `properties` are wire names, and values such as `const` are literal JSON.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SchemaContext {
    Schema,
    NamedSchemas,
    Literal,
}

impl SchemaContext {
    pub(super) fn member(self, name: &str) -> Option<Self> {
        match self {
            Self::NamedSchemas => Some(Self::Schema),
            Self::Literal => Some(Self::Literal),
            Self::Schema => match name {
                "$defs" => None,
                name if IGNORED.contains(&name) => None,
                "properties" | "patternProperties" | "dependentSchemas" | "dependencies"
                | "definitions" => Some(Self::NamedSchemas),
                "allOf"
                | "anyOf"
                | "oneOf"
                | "not"
                | "if"
                | "then"
                | "else"
                | "prefixItems"
                | "items"
                | "additionalItems"
                | "contains"
                | "unevaluatedItems"
                | "additionalProperties"
                | "unevaluatedProperties"
                | "propertyNames"
                | "contentSchema" => Some(Self::Schema),
                _ => Some(Self::Literal),
            },
        }
    }
}

/// The name a `$ref` into `$defs` names.
pub(super) fn def_name(schema: &Value) -> Option<&str> {
    schema.get("$ref")?.as_str()?.strip_prefix(DEFS)
}

/// The definitions `schema` references. Text and JSON literals can contain
/// strings of the same spelling and remain opaque.
pub(super) fn visit_refs<'a>(schema: &'a Value, mut on_ref: impl FnMut(&'a str)) {
    fn visit<'a>(value: &'a Value, context: SchemaContext, on_ref: &mut impl FnMut(&'a str)) {
        if context == SchemaContext::Literal {
            return;
        }
        match value {
            Value::Object(object) => {
                if context == SchemaContext::Schema
                    && let Some(name) = def_name(value)
                {
                    on_ref(name);
                }
                for (name, inner) in object {
                    if let Some(child) = context.member(name) {
                        visit(inner, child, on_ref);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, context, on_ref);
                }
            }
            _ => {}
        }
    }
    visit(schema, SchemaContext::Schema, &mut on_ref);
}

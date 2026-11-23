//! Query replies use the report tags declared on each Query entry and the
//! schema of the payload the engine serializes. Some reports also carry facts
//! from the session. The wrapper and named queries apply those additions here.
use super::{Builder, array, object, required};
use serde_json::{Value, json};
use sparksmith_core::catalog::generic::GenericValue;
use sparksmith_core::completeness::DefinitionReport;
use sparksmith_core::fields::PropertyEdit;
use sparksmith_core::ids::{ComponentRef, SourceLineageId};
use sparksmith_core::library::generated::GeneratorParams;
use sparksmith_core::permission::EntryDecl;
use sparksmith_core::{Finding, PhysicalChoice, QueryResult};

pub(super) fn query(cx: &mut Builder, decl: &EntryDecl) -> Value {
    reports(cx, Some(decl.reply_tags))
}

pub(super) fn all(cx: &mut Builder) -> Value {
    reports(cx, None)
}

fn reports(cx: &mut Builder, tags: Option<&[&str]>) -> Value {
    let mut schema = cx.typed::<QueryResult>();
    let Some(variants) = schema["oneOf"].as_array_mut() else {
        return Value::Bool(false);
    };
    if let Some(tags) = tags {
        variants.retain(|variant| {
            variant["properties"]["report"]["const"]
                .as_str()
                .is_some_and(|tag| tags.contains(&tag))
        });
        if variants.is_empty() || variants.len() != tags.len() {
            return Value::Bool(false);
        }
    }
    for variant in variants.iter_mut() {
        match variant["properties"]["report"]["const"].as_str() {
            Some("part_pins" | "net_intent") => required(variant, "revision", cx.typed::<u64>()),
            Some("drafts") => {
                let mut finding = cx.typed::<Finding>();
                finding["properties"]["objects_more"] = cx.typed::<usize>();
                let mut report = cx.typed::<DefinitionReport>();
                report["properties"]["findings"] = array(finding);
                variant["properties"]["result"] = array(report);
            }
            Some("search") => {
                variant["properties"]["generic"] = array(generic(cx));
                variant["properties"]["online"] = online(cx);
            }
            _ => {}
        }
    }
    if variants.len() == 1 {
        return variants.pop().unwrap_or(Value::Bool(false));
    }
    schema
}

fn generic(cx: &mut Builder) -> Value {
    let text = cx.typed::<String>();
    // The make hint supplies these five fields. Its caller supplies the
    // expected revision when applying the proposed operation.
    let params = object(
        &[
            ("generator", text.clone()),
            ("scope", json!({"const": "component"})),
            ("preset", cx.typed::<Option<String>>()),
            ("params", cx.typed::<GeneratorParams>()),
            ("name", text.clone()),
        ],
        &[],
    );
    let make = object(
        &[
            ("op", json!({"const": "library.generate_part"})),
            ("params", params),
        ],
        &[],
    );
    let mut row = object(
        &[("name", text.clone()), ("standing", text.clone())],
        &[
            ("refused", text),
            ("component", cx.typed::<ComponentRef>()),
            ("physical", cx.typed::<PhysicalChoice>()),
            ("properties", cx.typed::<Vec<PropertyEdit>>()),
            ("make", make),
            ("values", cx.typed::<Vec<GenericValue>>()),
        ],
    );
    row["oneOf"] = json!([
        {"required": ["refused"]}, {"required": ["component"]}, {"required": ["make"]}
    ]);
    row
}

fn online(cx: &mut Builder) -> Value {
    let text = cx.typed::<String>();
    let entry = object(
        &[
            ("lineage", cx.typed::<Option<SourceLineageId>>()),
            ("id", text.clone()),
            ("review", text.clone()),
        ],
        &[],
    );
    let success = object(
        &[
            ("source", text.clone()),
            ("snapshot", text.clone()),
            ("total", cx.typed::<usize>()),
            ("offline", cx.typed::<Option<String>>()),
            ("entries", array(entry)),
            ("place", text.clone()),
        ],
        &[],
    );
    let error = object(&[("error", text.clone())], &[("source", text)]);
    json!({"oneOf": [success, error]})
}

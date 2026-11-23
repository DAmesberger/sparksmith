//! The same core outcome may be returned by several operations. Cache a
//! stand-alone contract per outcome shape, including the no-identity edits.
use super::{Builder, required};
use serde_json::{Value, json};
use sparksmith_core::{Operation, Outcome};
use std::{collections::BTreeMap, sync::OnceLock};

use sparksmith_core::permission::EntryDecl;

fn tags(decl: &EntryDecl) -> &'static [&'static str] {
    if decl.reply_tags.is_empty() {
        &["none"]
    } else {
        decl.reply_tags
    }
}

pub(in crate::session) fn operation(decl: &EntryDecl) -> &'static Value {
    type Cache = BTreeMap<&'static [&'static str], OnceLock<Value>>;
    static SHAPES: OnceLock<Cache> = OnceLock::new();
    let shapes = SHAPES.get_or_init(|| {
        Operation::entries()
            .map(|entry| (tags(entry), OnceLock::new()))
            .collect()
    });
    let tags = tags(decl);
    shapes.get(tags).map_or(&Value::Bool(false), |cell| {
        cell.get_or_init(|| {
            super::build(|cx| {
                let mut reply = cx.document();
                required(&mut reply, "outcome", payload(cx, tags));
                reply
            })
        })
    })
}

pub(super) fn payload(cx: &mut Builder, tags: &[&str]) -> Value {
    let union = cx.typed::<Outcome>();
    let variant = |tag: &str| {
        union["oneOf"]
            .as_array()?
            .iter()
            .find(|v| v["properties"]["outcome"]["const"] == tag)
            .cloned()
    };
    let Some(choices) = tags
        .iter()
        .map(|tag| variant(tag))
        .collect::<Option<Vec<_>>>()
    else {
        return Value::Bool(false);
    };
    match choices.as_slice() {
        [one] => one.clone(),
        _ => json!({"oneOf":choices}),
    }
}

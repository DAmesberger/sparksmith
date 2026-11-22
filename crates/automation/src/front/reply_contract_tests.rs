//! Validate the actual wire results emitted by the automation test suite.
//! Validators share the same interned schema as discovery and the version lock.
use jsonschema::Validator;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

fn validator(entry: &str) -> Arc<Validator> {
    let root = crate::session::discovery::result_schema(entry)
        .unwrap_or_else(|| panic!("missing reply contract for {entry}"));
    static VALIDATORS: OnceLock<Mutex<HashMap<usize, Arc<Validator>>>> = OnceLock::new();
    {
        let mut known = VALIDATORS
            .get_or_init(Default::default)
            .lock()
            .expect("validator cache");
        let key = std::ptr::from_ref(root).addr();
        Arc::clone(known.entry(key).or_insert_with(|| {
            Arc::new(
                jsonschema::draft202012::new(root)
                    .unwrap_or_else(|why| panic!("invalid {entry} reply schema: {why}")),
            )
        }))
    }
}

pub(super) fn check(entry: &str, value: &Value) {
    let validator = validator(entry);
    if let Err(why) = validator.validate(value) {
        // Replies may contain an entire project or an embedded file. Keep a
        // failure identifiable without dumping the document into the CI log.
        let actual = value
            .get("outcome")
            .and_then(|v| v.get("outcome"))
            .or_else(|| value.get("report"))
            .and_then(Value::as_str);
        panic!(
            "{entry} emitted a reply outside its contract at '{}' (schema '{}', tag {actual:?})",
            why.instance_path, why.schema_path
        );
    }
}

#[test]
fn every_published_reply_schema_compiles() {
    for entry in crate::session::metadata::entries() {
        let _ = validator(&entry.name);
    }
}

#[test]
fn a_serialized_report_must_keep_its_tag_and_payload_type() {
    use serde_json::json;
    let validator = validator("project.issues");
    for invalid in [
        json!({"report":"other","result":[]}),
        json!({"report":"issues"}),
        json!({"report":"issues","result":123}),
    ] {
        assert!(!validator.is_valid(&invalid), "{invalid}");
    }
}

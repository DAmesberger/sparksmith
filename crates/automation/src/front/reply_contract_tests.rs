//! Validate the actual wire results emitted by the automation test suite.
//! Validators share the same interned schema as discovery and the version lock.
use jsonschema::Validator;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

fn validator(entry: &str) -> Result<Arc<Validator>, String> {
    let root = crate::session::discovery::result_schema(entry)
        .ok_or_else(|| format!("missing reply contract for {entry}"))?;
    static VALIDATORS: OnceLock<Mutex<HashMap<usize, Arc<Validator>>>> = OnceLock::new();
    let mut known = VALIDATORS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "reply validator cache was poisoned".to_owned())?;
    let key = std::ptr::from_ref(root).addr();
    if let Some(validator) = known.get(&key) {
        return Ok(Arc::clone(validator));
    }
    let compiled = jsonschema::draft202012::new(root)
        .map_err(|why| format!("invalid {entry} reply schema: {why}"))?;
    Ok(Arc::clone(
        known.entry(key).or_insert_with(|| Arc::new(compiled)),
    ))
}

fn validate(entry: &str, value: &Value) -> Result<(), String> {
    let validator = validator(entry)?;
    validator.validate(value).map_err(|why| {
        // Replies may contain an entire project or an embedded file. Keep a
        // failure identifiable without dumping the document into the CI log.
        let tag = value
            .get("outcome")
            .and_then(|v| v.get("outcome"))
            .or_else(|| value.get("report"))
            .and_then(Value::as_str);
        format!(
            "{entry} emitted a reply outside its contract at '{}' (schema '{}', tag {tag:?})",
            why.instance_path, why.schema_path
        )
    })
}

pub(super) fn check(entry: &str, value: &Value) {
    assert_eq!(validate(entry, value), Ok(()));
}

#[test]
fn every_published_reply_schema_compiles() -> Result<(), String> {
    for entry in crate::session::metadata::entries() {
        validator(&entry.name)?;
    }
    Ok(())
}

#[test]
fn a_serialized_report_must_keep_its_tag_and_payload_type() -> Result<(), String> {
    use serde_json::json;
    let validator = validator("project.issues")?;
    for invalid in [
        json!({"report":"other","result":[]}),
        json!({"report":"issues"}),
        json!({"report":"issues","result":123}),
    ] {
        assert!(!validator.is_valid(&invalid), "{invalid}");
    }
    Ok(())
}

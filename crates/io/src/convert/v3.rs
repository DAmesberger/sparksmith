//! Step 3 to 4: derived data out and nothing nested as a string (file
//! format slice F2, docs/design/file-format-plan.md §5).
//!
//! - **Computed fill copper leaves the file.** It is derived: a host
//!   refills it after opening (the desktop app off the UI thread, the
//!   headless session before it answers), and fabrication output waits for
//!   it. Fill definitions stay.
//! - **Sketches are JSON.** A footprint source's or board's `sketch` was a
//!   string holding SolidCraft's sketch JSON; it becomes that JSON object.
//! - **Waiver keys are JSON.** A waiver's `key` was the text
//!   `"<code> <objects as JSON>"`; it becomes `{"code", "objects"}`. A key
//!   that is not a finding's key could never match a finding, so it waived
//!   nothing: it is removed and named.
//!
//! The collections' order (sorted by key from version 4) is not changed
//! here: the converted document is written as the app writes it, which
//! sorts them.
use crate::convert::{Options, Result};
use serde_json::{Map, Value};

/// Step 3 to 4 of a document; the number of stored values changed.
pub fn step(file: &mut Value, _options: &Options) -> Result<usize> {
    let mut n = sketches(file)?;
    let Some(project) = file.get_mut("project").and_then(Value::as_object_mut) else {
        return Err("the document has no project".into());
    };
    if let Some(Value::Array(fills)) = project.get_mut("fills") {
        for fill in fills.iter_mut().filter_map(Value::as_object_mut) {
            n += usize::from(fill.remove("computed").is_some());
        }
    }
    if let Some(Value::Array(waivers)) = project
        .get_mut("production")
        .and_then(|p| p.get_mut("waivers"))
    {
        let before = waivers.len();
        waivers.retain(|w| {
            w.get("key")
                .and_then(Value::as_str)
                .is_some_and(|k| key(k).is_some())
        });
        n += before - waivers.len();
        for waiver in waivers.iter_mut().filter_map(Value::as_object_mut) {
            if let Some(stored) = waiver.get("key").and_then(Value::as_str).and_then(key) {
                waiver.insert("key".into(), stored);
                n += 1;
            }
        }
    }
    Ok(n)
}

/// What the step will say about `file` (a version 3 document) beyond
/// counts: the waivers it removes.
pub fn notes(file: &Value) -> Vec<String> {
    file.pointer("/project/production/waivers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|w| w.get("key").and_then(Value::as_str))
        .filter(|k| key(k).is_none())
        .map(|k| {
            format!(
                "the waiver of `{k}` was removed: it is not a finding's key, so it never waived a finding"
            )
        })
        .collect()
}

/// A version 3 waiver key as version 4 stores it: the code and the objects
/// it names. `None` for text that is not a finding's key.
fn key(text: &str) -> Option<Value> {
    let (code, objects) = text.split_once(' ')?;
    let objects: Value = serde_json::from_str(objects).ok()?;
    if code.is_empty() || !objects.is_array() {
        return None;
    }
    let mut stored = Map::new();
    stored.insert("code".into(), Value::String(code.into()));
    stored.insert("objects".into(), objects);
    Some(Value::Object(stored))
}

/// Every sketch under `value`: an object with an `evaluator` and a string
/// `sketch` (a footprint source or a board sketch) gets the sketch as JSON.
fn sketches(value: &mut Value) -> Result<usize> {
    let mut n = 0;
    match value {
        Value::Object(map) => {
            if map.contains_key("evaluator")
                && let Some(Value::String(text)) = map.get("sketch")
            {
                let sketch: Value = serde_json::from_str(text)
                    .map_err(|e| format!("a sketch does not read as JSON: {e}"))?;
                if !sketch.is_object() {
                    return Err("a sketch is not a JSON object".into());
                }
                map.insert("sketch".into(), sketch);
                n += 1;
            }
            for child in map.values_mut() {
                n += sketches(child)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                n += sketches(item)?;
            }
        }
        _ => {}
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn copper_sketches_and_waiver_keys_are_converted() -> Result<()> {
        let mut file = json!({
            "version": 3,
            "project": {
                "fills": [{"id": 4, "computed": {"key": "k", "regions": []}}, {"id": 5}],
                "board_shape": {"shape": "sketch", "sketch": {
                    "evaluator": {"engine": "solidcraft-sketch", "rev": "r"},
                    "sketch": "{\"points\":[{\"id\":\"origin\",\"pos\":[0.0,0.0],\"fixed\":true}]}"
                }},
                "production": {"waivers": [
                    {"key": "board.x [{\"kind\":\"pad\",\"instance\":80,\"pad\":17}]", "reason": "r"},
                    {"key": "not a key", "reason": "r"},
                    {"key": "fill.stale []", "reason": "r"}
                ]}
            },
            "drafts": {"items": []}
        });
        assert_eq!(notes(&file).len(), 1);
        assert_eq!(step(&mut file, &Options::default())?, 1 + 1 + 1 + 2);
        assert_eq!(file["project"]["fills"], json!([{"id": 4}, {"id": 5}]));
        assert_eq!(
            file["project"]["board_shape"]["sketch"]["sketch"],
            json!({"points": [{"id": "origin", "pos": [0.0, 0.0], "fixed": true}]})
        );
        assert_eq!(
            file["project"]["production"]["waivers"],
            json!([
                {"key": {"code": "board.x", "objects": [{"kind": "pad", "instance": 80, "pad": 17}]}, "reason": "r"},
                {"key": {"code": "fill.stale", "objects": []}, "reason": "r"}
            ])
        );
        Ok(())
    }
}

//! Production settings saved with the design: the pre-flight findings the
//! user waived for fabrication output. Data only; `fabrication::preflight`
//! matches waivers to findings and `fabrication::apply` edits them.
use crate::diagnostics::ObjectRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Keys one `fabrication.waive` request names: a bound on the request, not
/// on how many waivers a design keeps (it keeps any number).
pub const MAX_WAIVE_KEYS: usize = 4096;
/// Bytes of a waiver key or reason.
pub const MAX_WAIVER_TEXT: usize = 1024;

/// One waived pre-flight finding: its stable key
/// (`fabrication::PreflightItem::key`, the finding code and the objects it
/// names) and why it is acceptable for this build.
///
/// The key is held as the text commands and findings use
/// ([`waiver_key`]) and stored as JSON (file format version 4): the code
/// and the objects as values ([`StoredKey`]), never as a string holding
/// JSON. A key that is not a finding's key is refused
/// ([`Production::within_bounds`]).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Waiver {
    #[serde(with = "stored_key")]
    #[schemars(with = "StoredKey")]
    pub key: String,
    pub reason: String,
}

/// A waiver key as a file holds it: the finding's code and the objects it
/// names.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoredKey {
    pub code: String,
    pub objects: Vec<ObjectRef>,
}

impl StoredKey {
    /// The parts of a key [`waiver_key`] wrote; `None` for any other text.
    pub fn parse(key: &str) -> Option<Self> {
        let (code, objects) = key.split_once(' ')?;
        let objects: Vec<ObjectRef> = serde_json::from_str(objects).ok()?;
        let parsed = Self {
            code: code.to_owned(),
            objects,
        };
        (parsed.text() == key).then_some(parsed)
    }

    /// The key as text: [`waiver_key`].
    pub fn text(&self) -> String {
        waiver_key(&self.code, &self.objects)
    }
}

mod stored_key {
    use super::StoredKey;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(key: &str, serializer: S) -> Result<S::Ok, S::Error> {
        StoredKey::parse(key)
            .ok_or_else(|| {
                serde::ser::Error::custom(format!("the waiver key `{key}` is not a finding's key"))
            })?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        Ok(StoredKey::deserialize(deserializer)?.text())
    }
}

/// The waiver key of a finding: its code and the objects it names
/// (`fabrication::preflight::key`).
pub fn waiver_key(code: &str, objects: &[ObjectRef]) -> String {
    let objects = serde_json::to_string(objects).unwrap_or_default();
    format!("{code} {objects}")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Production {
    /// In key order, keys unique.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waivers: Vec<Waiver>,
}

impl Production {
    pub fn is_default(&self) -> bool {
        self.waivers.is_empty()
    }

    /// The waiver of `key`, by binary search: keys are sorted on a valid
    /// document ([`Self::within_bounds`]).
    pub fn waiver(&self, key: &str) -> Option<&Waiver> {
        self.waivers
            .binary_search_by(|w| w.key.as_str().cmp(key))
            .ok()
            .and_then(|k| self.waivers.get(k))
    }

    /// Text bounds, printable text, finding keys ([`StoredKey::parse`]),
    /// unique and sorted.
    pub fn within_bounds(&self) -> bool {
        self.waivers.iter().all(|w| {
            !w.key.trim().is_empty()
                && w.key.len() <= MAX_WAIVER_TEXT
                && StoredKey::parse(&w.key).is_some()
                && w.reason.len() <= MAX_WAIVER_TEXT
                && !w.key.chars().chain(w.reason.chars()).any(char::is_control)
        }) && self.waivers.windows(2).all(|p| p[0].key < p[1].key)
    }
}

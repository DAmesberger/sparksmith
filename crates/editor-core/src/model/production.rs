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
/// ([`waiver_key`]) and stored as JSON: the code
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
    /// The project's identity, RFC 4122-shaped (`8-4-4-4-12` lower-case
    /// hex), as every manufacturing file states it (Gerber `.ProjectId`,
    /// the job file's `GUID`): given once, when the project is first saved
    /// ([`project_id_for`]), and kept through renames, edits and undo, so
    /// two boards of one name differ and one board keeps its identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// In key order, keys unique.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waivers: Vec<Waiver>,
}

impl Production {
    pub fn is_default(&self) -> bool {
        self.project_id.is_none() && self.waivers.is_empty()
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
        self.project_id.as_deref().is_none_or(is_guid)
            && self.waivers.iter().all(|w| {
                !w.key.trim().is_empty()
                    && w.key.len() <= MAX_WAIVER_TEXT
                    && StoredKey::parse(&w.key).is_some()
                    && w.reason.len() <= MAX_WAIVER_TEXT
                    && !w.key.chars().chain(w.reason.chars()).any(char::is_control)
            })
            && self.waivers.windows(2).all(|p| p[0].key < p[1].key)
    }
}

/// Whether `text` is RFC 4122-shaped: `8-4-4-4-12` lower-case hex digits.
pub fn is_guid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// The first 16 bytes of `digest` as an RFC 4122-shaped identifier
/// (version nibble 8, custom; the variant bits set).
pub fn guid_of(digest: &[u8]) -> String {
    let mut b = [0u8; 16];
    for (slot, byte) in b.iter_mut().zip(digest) {
        *slot = *byte;
    }
    b[6] = (b[6] & 0x0f) | 0x80;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The identity a project takes when it is first saved: a digest of the
/// host's `salt` (the file's name and an optional nonce,
/// `crate::identify_by_file`) and the whole document at that moment. A copy
/// saved elsewhere keeps the identity it already has.
pub fn project_id_for(salt: &str, document: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"sparksmith project id 1\0");
    hasher.update(salt.as_bytes());
    hasher.update(b"\0");
    hasher.update(document);
    guid_of(&hasher.finalize())
}

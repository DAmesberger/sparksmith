//! Production settings saved with the design: the pre-flight findings the
//! user waived for fabrication output. Data only; `fabrication::preflight`
//! matches waivers to findings and `fabrication::apply` edits them.
use crate::diagnostics::{Measured, ObjectRef, Sense, Unit};
use crate::ids::DefinitionId;
use crate::rules::requirement::number_against;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Keys one `fabrication.waive` request names: a bound on the request, not
/// on how many waivers a design keeps (it keeps any number).
pub const MAX_WAIVE_KEYS: usize = 4096;
/// Bytes of a waiver key or reason.
pub const MAX_WAIVER_TEXT: usize = 1024;
/// Numbers one waiver accepts.
pub const MAX_ACCEPTED: usize = 1024;

/// One waived pre-flight finding: its stable key
/// (`fabrication::PreflightItem::key`, the finding code and the objects it
/// names), what was measured when it was accepted, and why it is
/// acceptable for this build.
///
/// A waiver of a finding that measures something accepts the numbers it had
/// and no worse ones ([`Production::acceptance`]). One without them (of a
/// project saved before waivers held them) counts for nothing until given
/// again. A finding that measures nothing is accepted by its key alone.
///
/// The key is held as the text commands and findings use
/// ([`waiver_key`]) and stored as JSON: the code
/// and the objects as values ([`StoredKey`]), never as a string holding
/// JSON. A key that is not a finding's key is refused
/// ([`Production::within_bounds`]).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Waiver {
    #[serde(with = "stored_key")]
    #[schemars(with = "StoredKey")]
    pub key: String,
    pub reason: String,
    /// What the finding measured when it was accepted: the check's own
    /// numbers, not rounded. Several when findings share the key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<Measured>,
    /// The footprint of each pad the key names, in order: on another
    /// footprint the same pad id is other copper, which nobody accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footprints: Vec<DefinitionId>,
    /// The fab profile and the vias it was given for, where the finding is
    /// what that profile does not state: under another profile, or with
    /// vias of other sizes, it accepts nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given_for: Option<GivenFor>,
}

/// The fab profile and the vias a waiver of what the fabricator does not
/// state was given for.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GivenFor {
    /// The profile's id.
    pub profile: String,
    /// Its version: the day its source was read, and a digest of its source
    /// and what it says of drills.
    pub read: String,
    pub statement: String,
    /// The sizes of the vias concerned, in order, each once.
    pub vias: Vec<GivenVia>,
}

/// One size of via, in whole nanometres.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(deny_unknown_fields)]
pub struct GivenVia {
    pub fill: super::ViaFill,
    pub drill_nm: i64,
    pub diameter_nm: i64,
}

/// What a finding is of beside the objects it names: a waiver holds only
/// for the same.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subject {
    /// The footprint of each pad it names (`instances::pad_footprints`).
    pub footprints: Vec<DefinitionId>,
    pub given_for: Option<GivenFor>,
}

/// What a project's waivers say of a finding.
#[derive(Clone, Debug, PartialEq)]
pub enum Acceptance<'a> {
    /// No waiver names it.
    None,
    Holds(&'a Waiver),
    /// A waiver names it and no longer holds; why, as a sentence without
    /// its full stop.
    Lapsed(&'a Waiver, String),
}

/// `value` of `unit` in words, told apart from `other` where they differ.
fn words(value: f64, other: f64, unit: Unit) -> String {
    let mut text = number_against(value, other);
    // Never shown as equal to what it is not.
    if value != other && text == number_against(other, value) {
        text = value.to_string();
    }
    match unit {
        Unit::Mm => format!("{text} mm"),
        Unit::Degrees => format!("{text}°"),
        Unit::Ratio => format!("{text}:1"),
        Unit::Percent => format!("{text} %"),
        Unit::Amperes => format!("{text} A"),
        Unit::Count => text,
    }
}

fn limit_words(m: &Measured, other: f64) -> String {
    let sense = match m.sense {
        Sense::AtLeast => "at least",
        Sense::AtMost => "at most",
    };
    format!("{sense} {}", words(m.limit, other, m.unit))
}

/// What a waiver accepted, in words ("0.15 mm, needs at least 0.2 mm");
/// `None` when it accepted no number.
pub fn accepted_words(accepted: &[Measured]) -> Option<String> {
    let each: Vec<String> = accepted
        .iter()
        .map(|m| {
            let of = if m.of.is_empty() {
                String::new()
            } else {
                format!("{}: ", m.of)
            };
            let (value, limit) = (words(m.value, m.limit, m.unit), limit_words(m, m.value));
            format!("{of}{value}, needs {limit}")
        })
        .collect();
    (!each.is_empty()).then(|| each.join("; "))
}

/// What `waiver` accepted, in words: [`accepted_words`], but of a finding
/// its profile does not state ([`Waiver::given_for`]) the values alone,
/// since the profile states no limit the board needs.
pub fn waiver_words(waiver: &Waiver) -> Option<String> {
    if waiver.given_for.is_none() {
        return accepted_words(&waiver.accepted);
    }
    let each: Vec<String> = waiver
        .accepted
        .iter()
        .map(|m| {
            let of = if m.of.is_empty() {
                String::new()
            } else {
                format!("{}: ", m.of)
            };
            format!("{of}{}", words(m.value, m.limit, m.unit))
        })
        .collect();
    (!each.is_empty()).then(|| each.join("; "))
}

/// Why accepting `then` does not accept `now`.
fn lapsed_words(then: &Measured, now: &Measured) -> String {
    let (was, is) = (
        words(then.value, now.value, then.unit),
        words(now.value, then.value, now.unit),
    );
    if then.limit == now.limit {
        format!("Was accepted at {was}, is now {is}")
    } else {
        format!(
            "Was accepted at {was} (needs {}), is now {is} (needs {})",
            limit_words(then, now.limit),
            limit_words(now, then.limit)
        )
    }
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

impl GivenFor {
    fn within_bounds(&self) -> bool {
        self.vias.len() <= MAX_ACCEPTED
            && [&self.profile, &self.read, &self.statement]
                .iter()
                .all(|t| t.len() <= MAX_WAIVER_TEXT && !t.chars().any(char::is_control))
    }
}

/// The waiver key of a finding: its code and the objects it names
/// (`fabrication::preflight::key`).
pub fn waiver_key(code: &str, objects: &[ObjectRef]) -> String {
    let objects = serde_json::to_string(objects).unwrap_or_default();
    format!("{code} {objects}")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
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

    /// Whether the waivers accept a finding of `key` that measures
    /// `measured` now, of `subject`. Each number must be one the waiver
    /// accepted, or better, and no farther from its limit, whatever the
    /// limit is now ([`Measured::accepts`]).
    pub fn acceptance(
        &self,
        key: &str,
        measured: &[Measured],
        subject: impl FnOnce() -> Subject,
    ) -> Acceptance<'_> {
        let Some(waiver) = self.waiver(key) else {
            return Acceptance::None;
        };
        let lapsed = |text: String| Acceptance::Lapsed(waiver, text);
        let subject = subject();
        if waiver.footprints != subject.footprints {
            return lapsed(if waiver.footprints.is_empty() {
                "Was accepted without its footprint; accept it again".into()
            } else {
                "Was accepted on another footprint".into()
            });
        }
        match (&waiver.given_for, &subject.given_for) {
            (then, now) if then == now => {}
            (None, _) => {
                return lapsed("Was accepted without its profile and vias; accept it again".into());
            }
            (Some(then), Some(now))
                if (&then.profile, &then.read, &then.statement)
                    == (&now.profile, &now.read, &now.statement) =>
            {
                return lapsed("Was accepted for vias of other sizes".into());
            }
            _ => return lapsed("Was accepted under another fab profile".into()),
        }
        if measured.is_empty() {
            // It had a number and has none now: copper that touches, or
            // lies outside.
            return match waiver.accepted.first() {
                None => Acceptance::Holds(waiver),
                Some(then) => lapsed(format!(
                    "Was accepted at {}, is now no such value",
                    words(then.value, then.limit, then.unit)
                )),
            };
        }
        if waiver.accepted.is_empty() {
            return lapsed("Was accepted without its value; accept it again".into());
        }
        for now in measured {
            if waiver.accepted.iter().any(|then| then.accepts(now)) {
                continue;
            }
            return lapsed(match waiver.accepted.iter().find(|then| then.alike(now)) {
                Some(then) => lapsed_words(then, now),
                None => format!(
                    "Was accepted for another value, is now {}",
                    words(now.value, now.limit, now.unit)
                ),
            });
        }
        Acceptance::Holds(waiver)
    }

    /// Text bounds, printable text, finite numbers, finding keys
    /// ([`StoredKey::parse`]), unique and sorted.
    pub fn within_bounds(&self) -> bool {
        self.project_id.as_deref().is_none_or(is_guid)
            && self.waivers.iter().all(|w| {
                !w.key.trim().is_empty()
                    && w.key.len() <= MAX_WAIVER_TEXT
                    && StoredKey::parse(&w.key).is_some()
                    && w.reason.len() <= MAX_WAIVER_TEXT
                    && !w.key.chars().chain(w.reason.chars()).any(char::is_control)
                    && w.accepted.len() <= MAX_ACCEPTED
                    && w.footprints.len() <= MAX_ACCEPTED
                    && w.given_for.as_ref().is_none_or(GivenFor::within_bounds)
                    && w.accepted.iter().all(|m| {
                        m.value.is_finite()
                            && m.limit.is_finite()
                            && m.of.len() <= MAX_WAIVER_TEXT
                            && !m.of.chars().any(char::is_control)
                    })
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
/// `crate::first_save`) and the whole document at that moment. A copy
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

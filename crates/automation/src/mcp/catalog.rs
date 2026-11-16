//! The catalog behind the tools: the 237 entries of
//! [`crate::session::metadata`] (requests, operations and queries), looked
//! up by name. A model reaches an entry only through the verb of its
//! [`Kind`] (`query` a query, `apply` an operation, `show` a show entry,
//! `act` an action), and
//! never a Never-class entry, which has no kind and is unknown here. P2's
//! verbs resolve their `name` argument through [`resolve`]; `find` and the
//! catalog resources (P4) list [`reachable`].
use crate::session::metadata::{self, EntryMeta, Kind};
use serde_json::{Value, json};
use sparksmith_core::Diagnostic;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Every entry a model can reach, in catalog order.
pub fn reachable() -> impl Iterator<Item = &'static EntryMeta> {
    metadata::entries()
        .iter()
        .filter(|entry| entry.kind.is_some())
}

/// The reachable entry named `name`.
pub fn entry(name: &str) -> Option<&'static EntryMeta> {
    static INDEX: OnceLock<BTreeMap<&'static str, &'static EntryMeta>> = OnceLock::new();
    INDEX
        .get_or_init(|| reachable().map(|e| (e.name.as_str(), e)).collect())
        .get(name)
        .copied()
}

/// The entry `name`, reached as `kind`.
pub fn resolve(kind: Kind, name: &str) -> Result<&'static EntryMeta, Unreachable> {
    let entry = entry(name).ok_or_else(|| Unreachable::unknown(name, Some(kind)))?;
    match entry.kind {
        Some(found) if found != kind => Err(Unreachable::OtherKind {
            name: name.to_owned(),
            asked: kind,
            kind: found,
        }),
        // Reachable entries all have a kind.
        _ => Ok(entry),
    }
}

/// Why a name does not resolve. It is the model's mistake, so it answers as
/// a tool result with `isError` ([`Self::reply`]), not a protocol error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unreachable {
    /// No reachable entry has the name; `closest` are the nearest names of
    /// the kind asked, best first (at most [`MAX_CLOSEST`]).
    Unknown {
        name: String,
        closest: Vec<&'static str>,
    },
    OtherKind {
        name: String,
        asked: Kind,
        kind: Kind,
    },
}

impl Unreachable {
    /// `name` is unknown: with the nearest names of `kind` (any kind when
    /// `None`).
    pub fn unknown(name: &str, kind: Option<Kind>) -> Self {
        Self::Unknown {
            name: name.to_owned(),
            closest: closest(name, kind),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown { .. } => "catalog.unknown",
            Self::OtherKind { .. } => "catalog.other_kind",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Unknown { name, closest } if closest.is_empty() => {
                format!("no catalog entry {name}: find {{text}} names the entries")
            }
            Self::Unknown { name, closest } => format!(
                "no catalog entry {name}: did you mean {}? find {{text}} names the entries",
                closest.join(", ")
            ),
            Self::OtherKind { name, asked, kind } => format!(
                "{name} is {}, not {}: {}",
                article(*kind),
                article(*asked),
                match kind {
                    Kind::Query => "read it with query",
                    Kind::Operation => "make it with apply",
                    Kind::Show => "send it with show",
                    Kind::Action => "run it with act",
                }
            ),
        }
    }

    /// The session envelope of this failure, as [`super::outcome`] reads it.
    pub fn reply(&self) -> Value {
        let diagnostic = Diagnostic {
            code: self.code().into(),
            rule: None,
            message: self.message(),
            expected_revision: None,
            actual_revision: None,
        };
        let mut error = json!(diagnostic);
        if let Self::Unknown { closest, .. } = self
            && !closest.is_empty()
        {
            error["closest"] = json!(closest);
        }
        json!({"ok": false, "error": error})
    }
}

/// The most names [`Unreachable::Unknown`] suggests.
pub const MAX_CLOSEST: usize = metadata::MAX_CLOSEST;

/// The reachable names of `kind` nearest `name` ([`metadata::closest`]).
fn closest(name: &str, kind: Option<Kind>) -> Vec<&'static str> {
    metadata::closest(name, |e| {
        e.kind.is_some() && kind.is_none_or(|kind| e.kind == Some(kind))
    })
}

fn article(kind: Kind) -> &'static str {
    match kind {
        Kind::Query => "a query",
        Kind::Operation => "an operation",
        Kind::Show => "a show entry",
        Kind::Action => "an action",
    }
}

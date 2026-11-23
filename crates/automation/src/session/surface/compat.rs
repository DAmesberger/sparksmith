//! How a parameter schema changed, compared structurally
//! (`docs/extension-surface.md#what-a-change-needs`).
//!
//! Each side is a schema with its definitions by key (references
//! `#/$defs/<key>`). A parameter schema is what a caller sends, so
//! a change is compatible when every value the old schema accepted is
//! still accepted with the same meaning: a new optional property, a new
//! union variant or enum value, a looser bound, a parameter no longer
//! required. Anything else, and any keyword this does not know, is a
//! break.
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// What a change needs, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// The same values, the same meaning.
    Same,
    /// More is accepted: a minor version.
    Minor,
    /// Something accepted before is refused or reads differently: a major
    /// version.
    Major,
}

/// A change and, for a break, where and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub change: Change,
    pub reasons: Vec<String>,
}

impl Outcome {
    fn same() -> Self {
        Self {
            change: Change::Same,
            reasons: Vec::new(),
        }
    }

    fn minor() -> Self {
        Self {
            change: Change::Minor,
            reasons: Vec::new(),
        }
    }

    fn major(reason: String) -> Self {
        Self {
            change: Change::Major,
            reasons: vec![reason],
        }
    }

    fn join(&mut self, other: Outcome) {
        self.change = self.change.max(other.change);
        self.reasons.extend(other.reasons);
    }
}

/// Lower bounds: raising one narrows.
const LOWER: &[&str] = &[
    "minimum",
    "exclusiveMinimum",
    "minLength",
    "minItems",
    "minProperties",
];
/// Upper bounds: lowering one narrows.
const UPPER: &[&str] = &[
    "maximum",
    "exclusiveMaximum",
    "maxLength",
    "maxItems",
    "maxProperties",
];
/// Keywords with their own rule below.
const HANDLED: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "prefixItems",
    "enum",
    "const",
    "default",
    "oneOf",
    "anyOf",
    "allOf",
];
const DEFS: &str = "#/$defs/";

/// The definitions a schema's references name, by key.
pub type Defs = BTreeMap<String, Value>;

/// How the parameters changed from `old` to `new`, each with the
/// definitions its references (`#/$defs/<key>`) name.
pub fn classify(old: &Value, old_defs: &Defs, new: &Value, new_defs: &Defs) -> Outcome {
    inclusion(old, old_defs, new, new_defs, false)
}

/// A reply is what a client receives. Every value the new writer can
/// emit must be readable by an existing consumer. Object readers ignore
/// additional, unknown fields, and rely on the documented ones.
pub fn classify_reply(old: &Value, old_defs: &Defs, new: &Value, new_defs: &Defs) -> Outcome {
    inclusion(new, new_defs, old, old_defs, true)
}

fn inclusion(old: &Value, old_defs: &Defs, new: &Value, new_defs: &Defs, reply: bool) -> Outcome {
    let mut walk = Walk {
        reply,
        old_defs,
        new_defs,
        open: HashSet::new(),
        done: HashMap::new(),
    };
    let mut outcome = walk.compare(old, new, "");
    // A definition met twice reports its break once.
    let mut seen = HashSet::new();
    outcome.reasons.retain(|why| seen.insert(why.clone()));
    outcome
}

/// A tag value as a person reads it: a string without its quotes.
fn label(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

struct Walk<'a> {
    reply: bool,
    old_defs: &'a Defs,
    new_defs: &'a Defs,
    /// Reference pairs being compared: met again, they are taken as the
    /// same (a recursive type).
    open: HashSet<(String, String)>,
    done: HashMap<(String, String), Outcome>,
}

fn reference(schema: &Value) -> Option<&str> {
    schema.get("$ref")?.as_str()?.strip_prefix(DEFS)
}

/// A reference can carry additional constraints in JSON Schema 2020-12.
fn has_ref_siblings(schema: &Value) -> bool {
    schema.as_object().is_some_and(|object| object.len() > 1)
}

/// The variants of a schema that is only a union.
fn pure_union(schema: &Value) -> Option<&[Value]> {
    let object = schema.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object
        .get("oneOf")
        .or_else(|| object.get("anyOf"))?
        .as_array()
        .map(Vec::as_slice)
}

/// A variant's tags: its properties fixed to one value.
fn tags(schema: &Value) -> BTreeSet<(String, String)> {
    let fixed = |property: &Value| {
        property
            .get("const")
            .cloned()
            .or_else(|| match property.get("enum") {
                Some(Value::Array(values)) if values.len() == 1 => values.first().cloned(),
                _ => None,
            })
    };
    let mut out = BTreeSet::new();
    if let Some(constant) = fixed(schema) {
        out.insert((String::new(), label(&constant)));
    }
    for (name, property) in schema["properties"].as_object().into_iter().flatten() {
        if let Some(constant) = fixed(property) {
            out.insert((name.clone(), label(&constant)));
        }
    }
    out
}

fn types(schema: &Map<String, Value>) -> Option<BTreeSet<&str>> {
    match schema.get("type")? {
        Value::String(one) => Some(BTreeSet::from([one.as_str()])),
        Value::Array(many) => Some(many.iter().filter_map(Value::as_str).collect()),
        _ => Some(BTreeSet::new()),
    }
}

impl Walk<'_> {
    fn compare(&mut self, old: &Value, new: &Value, path: &str) -> Outcome {
        match (reference(old), reference(new)) {
            (Some(a), Some(b)) => {
                let mut result = self.pair(a.to_owned(), b.to_owned(), path);
                if has_ref_siblings(old) || has_ref_siblings(new) {
                    let siblings = |schema: &Value| {
                        let mut object = schema.as_object().cloned().unwrap_or_default();
                        object.remove("$ref");
                        object
                    };
                    result.join(self.object(&siblings(old), &siblings(new), path));
                }
                return result;
            }
            (Some(_), None) if has_ref_siblings(old) => {
                return Outcome::major(format!("{path}: constrained reference changed form"));
            }
            (None, Some(_)) if has_ref_siblings(new) => {
                return Outcome::major(format!("{path}: constrained reference changed form"));
            }
            (Some(a), None) => {
                let resolved = self.old_defs.get(a).cloned().unwrap_or(Value::Null);
                return self.compare(&resolved, new, path);
            }
            (None, Some(b)) => {
                let resolved = self.new_defs.get(b).cloned().unwrap_or(Value::Null);
                return self.compare(old, &resolved, path);
            }
            (None, None) => {}
        }
        if pure_union(old).is_some() || pure_union(new).is_some() {
            let olds = pure_union(old).unwrap_or(std::slice::from_ref(old));
            let news = pure_union(new).unwrap_or(std::slice::from_ref(new));
            return self.union(olds, news, path);
        }
        match (old, new) {
            (Value::Bool(true), Value::Bool(true)) => Outcome::same(),
            (_, Value::Bool(true)) => Outcome::minor(),
            (Value::Object(old), Value::Object(new)) => self.object(old, new, path),
            (Value::Bool(true), _) => {
                Outcome::major(format!("{}: accepted anything, now constrained", path))
            }
            (a, b) if a == b => Outcome::same(),
            _ => Outcome::major(format!("{}: changed form", path)),
        }
    }

    fn pair(&mut self, a: String, b: String, path: &str) -> Outcome {
        let key = (a, b);
        if let Some(known) = self.done.get(&key) {
            return known.clone();
        }
        if !self.open.insert(key.clone()) {
            return Outcome::same();
        }
        let old = self.old_defs.get(&key.0).cloned().unwrap_or(Value::Null);
        let new = self.new_defs.get(&key.1).cloned().unwrap_or(Value::Null);
        let outcome = self.compare(&old, &new, path);
        self.open.remove(&key);
        self.done.insert(key, outcome.clone());
        outcome
    }

    /// Every old variant must still be accepted by a new one; a new one
    /// no old variant needed widens.
    fn union(&mut self, olds: &[Value], news: &[Value], path: &str) -> Outcome {
        let mut outcome = Outcome::same();
        let mut used = BTreeSet::new();
        for (i, old) in olds.iter().enumerate() {
            let old_tags = tags(old);
            let tagged: Vec<usize> = if old_tags.is_empty() {
                Vec::new()
            } else {
                (0..news.len())
                    .filter(|&j| news.get(j).is_some_and(|n| tags(n) == old_tags))
                    .collect()
            };
            let label = old_tags
                .iter()
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let here = if label.is_empty() {
                format!("{path}|{i}")
            } else {
                format!("{path}|{label}")
            };
            // A tagged variant is matched by its tags; with none left it was
            // removed. An untagged one is matched by whichever accepts it.
            let candidates: Vec<usize> = match (old_tags.is_empty(), tagged.is_empty()) {
                (true, _) => (0..news.len()).collect(),
                (false, false) => tagged,
                (false, true) => {
                    let reason = if self.reply {
                        "new reply variant"
                    } else {
                        "removed"
                    };
                    outcome.join(Outcome::major(format!("{here}: {reason}")));
                    continue;
                }
            };
            let mut best: Option<(usize, Outcome)> = None;
            for j in candidates {
                let Some(new) = news.get(j) else { continue };
                let tried = self.compare(old, new, &here);
                if best.as_ref().is_none_or(|(_, b)| tried.change < b.change) {
                    best = Some((j, tried));
                }
            }
            match best {
                Some((j, found)) => {
                    used.insert(j);
                    outcome.join(found);
                }
                None if self.reply => {
                    outcome.join(Outcome::major(format!("{here}: new reply variant")))
                }
                None => outcome.join(Outcome::major(format!("{here}: removed"))),
            }
        }
        if used.len() < news.len() {
            outcome.join(Outcome::minor());
        }
        outcome
    }

    fn object(
        &mut self,
        old: &Map<String, Value>,
        new: &Map<String, Value>,
        path: &str,
    ) -> Outcome {
        let mut outcome = Outcome::same();
        let at = path;
        // Mixed unions: the union lists compare as variant sets.
        for key in ["oneOf", "anyOf"] {
            match (old.get(key), new.get(key)) {
                (Some(Value::Array(a)), Some(Value::Array(b))) => {
                    let found = self.union(a, b, path);
                    outcome.join(found);
                }
                (None, None) => {}
                (Some(_), None) => outcome.join(Outcome::minor()),
                _ => outcome.join(Outcome::major(format!("{at}: {key} changed"))),
            }
        }
        match (old.get("allOf"), new.get("allOf")) {
            (Some(Value::Array(a)), Some(Value::Array(b))) if a.len() == b.len() => {
                for (x, y) in a.iter().zip(b) {
                    let found = self.compare(x, y, path);
                    outcome.join(found);
                }
            }
            (None, None) => {}
            _ => outcome.join(Outcome::major(format!("{at}: allOf changed"))),
        }
        outcome.join(type_change(old, new, at));
        outcome.join(self.properties(old, new, path));
        outcome.join(self.additional(old, new, path));
        outcome.join(self.items(old, new, path));
        outcome.join(enum_change(old, new, at, self.reply));
        outcome.join(const_change(old, new, at));
        outcome.join(default_change(old, new, at));
        outcome.join(bounds(old, new, at));
        outcome.join(other_keywords(old, new, at));
        outcome
    }

    fn properties(
        &mut self,
        old: &Map<String, Value>,
        new: &Map<String, Value>,
        path: &str,
    ) -> Outcome {
        let mut outcome = Outcome::same();
        let empty = Map::new();
        let old_props = old
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let new_props = new
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let required = |schema: &Map<String, Value>| -> BTreeSet<String> {
            schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let old_required = required(old);
        let new_required = required(new);
        for (name, was) in old_props {
            let here = format!("{path}.{name}");
            match new_props.get(name) {
                None if self.reply => {
                    // In reply mode `new` is the previous reader's schema.
                    // A reader with an explicit restriction on extra keys
                    // must also accept the newly emitted field's value.
                    if let Some(extra) = new.get("additionalProperties") {
                        outcome.join(self.compare(was, extra, &here));
                    }
                    outcome.join(Outcome::minor());
                }
                None => outcome.join(Outcome::major(format!("{here}: removed"))),
                Some(now) => {
                    let found = self.compare(was, now, &here);
                    outcome.join(found);
                }
            }
        }
        for name in new_props.keys().filter(|n| !old_props.contains_key(*n)) {
            if self.reply {
                outcome.join(Outcome::major(format!("{path}.{name}: no longer returned")));
            } else if new_required.contains(name) {
                outcome.join(Outcome::major(format!("{path}.{name}: added as required")));
            } else {
                outcome.join(Outcome::minor());
            }
        }
        // A new property that is required was named above.
        for name in new_required.difference(&old_required) {
            if old_props.contains_key(name) || !new_props.contains_key(name) {
                let reason = if self.reply {
                    "no longer guaranteed"
                } else {
                    "now required"
                };
                outcome.join(Outcome::major(format!("{path}.{name}: {reason}")));
            }
        }
        if old_required.difference(&new_required).next().is_some() {
            outcome.join(Outcome::minor());
        }
        outcome
    }

    /// `additionalProperties`; absent reads as anything.
    fn additional(
        &mut self,
        old: &Map<String, Value>,
        new: &Map<String, Value>,
        path: &str,
    ) -> Outcome {
        let anything = Value::Bool(true);
        let was = old.get("additionalProperties").unwrap_or(&anything);
        let now = new.get("additionalProperties").unwrap_or(&anything);
        if was == now {
            return Outcome::same();
        }
        match now {
            Value::Bool(false) => Outcome::major(format!("{}: other properties now refused", path)),
            _ => self.compare(was, now, &format!("{path}{{}}")),
        }
    }

    fn items(&mut self, old: &Map<String, Value>, new: &Map<String, Value>, path: &str) -> Outcome {
        let mut outcome = Outcome::same();
        let anything = Value::Bool(true);
        if old.contains_key("items") || new.contains_key("items") {
            let was = old.get("items").unwrap_or(&anything);
            let now = new.get("items").unwrap_or(&anything);
            let found = self.compare(was, now, &format!("{path}[]"));
            outcome.join(found);
        }
        match (old.get("prefixItems"), new.get("prefixItems")) {
            (Some(Value::Array(a)), Some(Value::Array(b))) if a.len() == b.len() => {
                for (i, (x, y)) in a.iter().zip(b).enumerate() {
                    let found = self.compare(x, y, &format!("{path}[{i}]"));
                    outcome.join(found);
                }
            }
            (None, None) => {}
            _ => outcome.join(Outcome::major(format!("{}: tuple changed", path))),
        }
        outcome
    }
}

/// A type set may grow; `integer` reads as part of `number`.
fn type_change(old: &Map<String, Value>, new: &Map<String, Value>, at: &str) -> Outcome {
    match (types(old), types(new)) {
        (None, None) => Outcome::same(),
        (Some(_), None) => Outcome::minor(),
        (None, Some(_)) => Outcome::major(format!("{at}: type now constrained")),
        (Some(was), Some(now)) if was == now => Outcome::same(),
        (Some(was), Some(now)) => {
            let covered = was
                .iter()
                .all(|t| now.contains(t) || (*t == "integer" && now.contains("number")));
            if covered {
                Outcome::minor()
            } else {
                Outcome::major(format!("{at}: type narrowed or changed"))
            }
        }
    }
}

fn enum_change(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    at: &str,
    reply: bool,
) -> Outcome {
    match (old.get("enum"), new.get("enum")) {
        (None, None) => Outcome::same(),
        (Some(_), None) => Outcome::minor(),
        (None, Some(_)) => Outcome::major(format!("{at}: now limited to listed values")),
        (Some(Value::Array(was)), Some(Value::Array(now))) => {
            let removed: Vec<String> = was
                .iter()
                .filter(|v| !now.contains(v))
                .map(Value::to_string)
                .collect();
            if !removed.is_empty() {
                let reason = if reply {
                    "new reply value"
                } else {
                    "removed value"
                };
                Outcome::major(format!("{at}: {reason} {}", removed.join(", ")))
            } else if now.len() > was.len() {
                Outcome::minor()
            } else {
                Outcome::same()
            }
        }
        _ => Outcome::major(format!("{at}: enum changed")),
    }
}

fn const_change(old: &Map<String, Value>, new: &Map<String, Value>, at: &str) -> Outcome {
    match (old.get("const"), new.get("const")) {
        (a, b) if a == b => Outcome::same(),
        (Some(_), None) => Outcome::minor(),
        _ => Outcome::major(format!("{at}: constant changed")),
    }
}

/// A default is what an omitted value means: a new one only names what
/// was assumed, a changed or dropped one changes what a caller gets.
fn default_change(old: &Map<String, Value>, new: &Map<String, Value>, at: &str) -> Outcome {
    match (old.get("default"), new.get("default")) {
        (a, b) if a == b => Outcome::same(),
        (None, Some(_)) => Outcome::minor(),
        _ => Outcome::major(format!("{at}: default changed")),
    }
}

fn bounds(old: &Map<String, Value>, new: &Map<String, Value>, at: &str) -> Outcome {
    let mut outcome = Outcome::same();
    for (keys, lower) in [(LOWER, true), (UPPER, false)] {
        for key in keys {
            match (old.get(*key), new.get(*key)) {
                (a, b) if a == b => {}
                (Some(_), None) => outcome.join(Outcome::minor()),
                (None, Some(_)) => outcome.join(Outcome::major(format!("{at}: {key} added"))),
                (Some(a), Some(b)) => match (a.as_f64(), b.as_f64()) {
                    (Some(a), Some(b)) if (b < a) == lower => outcome.join(Outcome::minor()),
                    _ => outcome.join(Outcome::major(format!("{at}: {key} tightened"))),
                },
                (None, None) => {}
            }
        }
    }
    outcome
}

/// Any other keyword (`format`, `pattern`, `x-kind`, …) is a constraint:
/// dropping it widens, adding or changing it is a break.
fn other_keywords(old: &Map<String, Value>, new: &Map<String, Value>, at: &str) -> Outcome {
    let mut outcome = Outcome::same();
    let known = |key: &str| HANDLED.contains(&key) || LOWER.contains(&key) || UPPER.contains(&key);
    let keys: BTreeSet<&String> = old.keys().chain(new.keys()).filter(|k| !known(k)).collect();
    for key in keys {
        match (old.get(key), new.get(key)) {
            (a, b) if a == b => {}
            (Some(_), None) => outcome.join(Outcome::minor()),
            _ => outcome.join(Outcome::major(format!("{at}: {key} added or changed"))),
        }
    }
    outcome
}

#[cfg(test)]
mod tests;

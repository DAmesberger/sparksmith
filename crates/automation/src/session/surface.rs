//! The catalog as modules, and `surface.lock` (`docs/extension-surface.md`).
//!
//! Every entry belongs to the module its declaration names. A module's
//! digest is the SHA-256 of its canonical description: its entries sorted
//! by name, each with what it declares and the canonical schema of its
//! parameters and successful reply. The lock records per module its version and digest, and per
//! entry its class, a schema digest, a digest per parameter and the
//! required parameters. Beside it, `surface.schemas` keeps each entry's
//! input and reply schemas and each type they reach once ([`Schemas`]), so
//! [`ratchet`] compares old and new structurally ([`compat`]) and tells a compatible change (a new entry, a
//! new optional field at any depth, a wider type: a minor bump) from a
//! breaking one (a removed entry or field, a narrowed type, a new required
//! field, a stricter class: a major bump). `cargo xtask surface --check`
//! runs [`tests::the_lock_holds`].
use super::discovery::{self, Entry};
use super::metadata::{self, EntryMeta};
use super::schema_walk::{DEFS, SchemaContext, def_name, visit_refs};
use serde_json::{Map, Value, json};
use sparksmith_core::permission::{MODULES, ModuleDecl};
use sparksmith_generator_abi::surface::{
    LOCK_FORMAT, Lock, LockedEntry, LockedModule, parse_version, version_text,
};
use sparksmith_generator_abi::{hex, sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The lock's place in the repository, beside this crate's manifest.
pub const LOCK_FILE: &str = "surface.lock";
/// Each entry's input and successful reply schemas, including their referenced types.
pub const SCHEMAS_FILE: &str = "surface.schemas";
/// The format of [`SCHEMAS_FILE`].
const SCHEMAS_FORMAT: u64 = 2;

/// What [`SCHEMAS_FILE`] records: each entry's parameter schema, without
/// the keywords that change no call, and every definition they reach,
/// once. A definition's key is its type's name (`Name~2` for a second,
/// different definition of that name), and references read
/// `#/$defs/<key>`, so a shared type is one line and a change to it one
/// changed line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schemas {
    pub defs: compat::Defs,
    pub entries: BTreeMap<String, Value>,
    pub replies: BTreeMap<String, Value>,
}

/// Hex digits of a short digest.
const SHORT: usize = 16;

/// The parameters' schema of `entry`: the request's own, or the variant's
/// `params` for an operation or query.
fn input(entry: &Entry) -> &Value {
    let schema = &entry.schema;
    let params = &schema["properties"]["params"];
    if entry.kind != "command" && params.is_object() {
        resolve(params, entry.root)
    } else {
        schema
    }
}

pub(super) fn resolve<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix(DEFS))
        .and_then(|name| root["$defs"].get(name))
        .unwrap_or(schema)
}

/// `schema` with the definitions it reaches, renumbered in the order they
/// are first referenced and without the keywords that change no call, so a
/// renamed type or an edited doc comment keeps the form.
pub(super) fn canonical(schema: &Value, root: &Value) -> Value {
    let mut order: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let main = rewrite(schema, &mut order, &mut index);
    let mut defs = Vec::new();
    let mut next = 0;
    while let Some(name) = order.get(next).cloned() {
        let def = root["$defs"].get(&name).cloned().unwrap_or(Value::Null);
        defs.push(rewrite(&def, &mut order, &mut index));
        next += 1;
    }
    json!({"schema": main, "defs": defs})
}

fn rewrite(value: &Value, order: &mut Vec<String>, index: &mut HashMap<String, usize>) -> Value {
    map_refs(value, SchemaContext::Schema, &mut |name| {
        let number = *index.entry(name.to_owned()).or_insert_with(|| {
            order.push(name.to_owned());
            order.len() - 1
        });
        number.to_string()
    })
}

fn map_refs(value: &Value, context: SchemaContext, key: &mut impl FnMut(&str) -> String) -> Value {
    if context == SchemaContext::Literal {
        return value.clone();
    }
    match value {
        Value::Object(object) => {
            let mut out = Map::new();
            for (name, inner) in object {
                let Some(child) = context.member(name) else {
                    continue;
                };
                let mapped = if context == SchemaContext::Schema
                    && name == "$ref"
                    && let Some(target) = inner.as_str().and_then(|r| r.strip_prefix(DEFS))
                {
                    json!(format!("{DEFS}{}", key(target)))
                } else {
                    map_refs(inner, child, key)
                };
                out.insert(name.clone(), mapped);
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| map_refs(v, context, key)).collect())
        }
        other => other.clone(),
    }
}

fn digest(value: &Value) -> String {
    hex(&sha256(value.to_string().as_bytes()))
}

fn short(value: &Value) -> String {
    let mut text = digest(value);
    text.truncate(SHORT);
    text
}

/// One entry as the lock and the module digest see it.
struct Described {
    locked: LockedEntry,
    /// What the module digest covers for this entry.
    contract: Value,
}

fn describe_entry(entry: &Entry, meta: &EntryMeta, reply_digest: &str) -> Described {
    let schema = input(entry);
    let root = entry.root;
    let whole = canonical(schema, root);
    let mut fields: Vec<(String, String)> = schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, property)| (name.clone(), short(&canonical(property, root))))
        .collect();
    fields.sort();
    let mut required: Vec<String> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    required.sort();
    let contract = json!({
        "name": meta.name,
        "kind": meta.kind,
        "class": meta.class,
        "effect": meta.effect,
        "reach": meta.reach.iter().map(|r| r.name()).collect::<Vec<_>>(),
        "needs": meta.needs.iter().map(|f| f.name()).collect::<Vec<_>>(),
        "person": meta.person,
        "long": meta.long,
        "result": meta.result,
        "since": version_text(meta.since),
        "input": digest(&whole),
        "reply": reply_digest,
    });
    Described {
        locked: LockedEntry {
            name: meta.name.clone(),
            class: meta.class,
            schema: short(&whole),
            reply: reply_digest.chars().take(SHORT).collect(),
            fields,
            required,
        },
        contract,
    }
}

/// The catalog as `surface.lock` records it, modules in declaration order.
pub fn current() -> Lock {
    let mut by_module: BTreeMap<&str, Vec<Described>> = BTreeMap::new();
    let mut reply_digests = HashMap::new();
    for (entry, meta) in discovery::entries().iter().zip(metadata::entries()) {
        let reply = entry.reply_schema();
        let reply_digest = reply_digests
            .entry(std::ptr::from_ref(reply))
            .or_insert_with(|| digest(&canonical(reply, reply)));
        by_module
            .entry(meta.module)
            .or_default()
            .push(describe_entry(entry, meta, reply_digest));
    }
    let modules = MODULES
        .iter()
        .map(|module| {
            let mut entries = by_module.remove(module.id).unwrap_or_default();
            entries.sort_by(|a, b| a.locked.name.cmp(&b.locked.name));
            let description = json!({
                "module": module.id,
                "entries": entries.iter().map(|e| &e.contract).collect::<Vec<_>>(),
            });
            LockedModule {
                id: module.id.to_owned(),
                version: version_text(module.version),
                digest: format!("sha256:{}", digest(&description)),
                entries: entries.into_iter().map(|e| e.locked).collect(),
            }
        })
        .collect();
    Lock {
        format: LOCK_FORMAT,
        modules,
    }
}

/// Whether two schemas agree after renaming their definitions and omitting
/// schema annotations. A recursive pair only needs to be dereferenced once.
fn same(
    a: &Value,
    ra: &Value,
    b: &Value,
    rb: &Value,
    assumed: &mut BTreeSet<(String, String)>,
) -> bool {
    same_at(a, ra, b, rb, assumed, SchemaContext::Schema)
}

fn same_at(
    a: &Value,
    ra: &Value,
    b: &Value,
    rb: &Value,
    assumed: &mut BTreeSet<(String, String)>,
    context: SchemaContext,
) -> bool {
    if context == SchemaContext::Literal {
        return a == b;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let count = |map: &Map<String, Value>| {
                map.keys().filter(|k| context.member(k).is_some()).count()
            };
            count(x) == count(y)
                && x.iter().all(|(k, v)| {
                    let Some(child) = context.member(k) else {
                        return true;
                    };
                    let Some(w) = y.get(k) else { return false };
                    if context == SchemaContext::Schema
                        && k == "$ref"
                        && let (Some(i), Some(j)) = (def_name(a), def_name(b))
                    {
                        !assumed.insert((i.to_owned(), j.to_owned()))
                            || same(&ra["$defs"][i], ra, &rb["$defs"][j], rb, assumed)
                    } else {
                        same_at(v, ra, w, rb, assumed, child)
                    }
                })
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|(p, q)| same_at(p, ra, q, rb, assumed, context))
        }
        _ => a == b,
    }
}

/// `value` without schema annotations, its references renamed by `key`.
fn keyed(value: &Value, mut key: &dyn Fn(&str) -> String) -> Value {
    map_refs(value, SchemaContext::Schema, &mut key)
}

/// A definition's key: its name, or `name~n` for the n-th different
/// definition of that name.
fn def_key(name: &str, index: usize) -> String {
    if index == 0 {
        name.to_owned()
    } else {
        format!("{name}~{}", index + 1)
    }
}

/// Every entry's parameter schema and the definitions they reach, each
/// distinct definition once ([`Schemas`]).
pub fn schemas() -> Schemas {
    let mut entries: Vec<&Entry> = discovery::entries().iter().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    // The catalog's roots; a definition is a root's index and its name.
    let mut roots: Vec<&'static Value> = Vec::new();
    let mut reached: Vec<(usize, String)> = Vec::new();
    let mut seen: BTreeSet<(usize, String)> = BTreeSet::new();
    // Retain the input definition names when a reply type has the same
    // Rust name but a different serialized form.
    let inputs = entries.iter().map(|e| (e, input(e), e.root, false));
    let outputs = entries.iter().map(|e| {
        let schema = e.reply_schema();
        (e, schema, schema, true)
    });
    let mut descriptions = Vec::new();
    let mut reply_uses: HashMap<usize, usize> = HashMap::new();
    for (entry, schema, root, reply) in inputs.chain(outputs) {
        let r = match roots
            .iter()
            .position(|candidate| std::ptr::eq(*candidate, root))
        {
            Some(i) => i,
            None => {
                roots.push(root);
                roots.len() - 1
            }
        };
        descriptions.push((&entry.name, schema, r, reply));
        if reply {
            *reply_uses.entry(r).or_default() += 1;
        }
        let mut queue = Vec::new();
        visit_refs(schema, |name| queue.push(name));
        while let Some(name) = queue.pop() {
            if seen.insert((r, name.to_owned())) {
                reached.push((r, name.to_owned()));
                visit_refs(&root["$defs"][name], |target| queue.push(target));
            }
        }
    }
    // Each distinct definition of a name once, in the order first reached.
    let mut keys: HashMap<(usize, String), String> = HashMap::new();
    let mut classes: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (r, name) in &reached {
        let reps = classes.entry(name.clone()).or_default();
        let found = reps.iter().position(|rep| {
            let (Some(a), Some(b)) = (roots.get(*rep), roots.get(*r)) else {
                return false;
            };
            let (x, y) = (&a["$defs"][name.as_str()], &b["$defs"][name.as_str()]);
            same(x, a, y, b, &mut BTreeSet::new())
        });
        let index = found.unwrap_or_else(|| {
            reps.push(*r);
            reps.len() - 1
        });
        keys.insert((*r, name.clone()), def_key(name, index));
    }
    let key_in = |r: usize| {
        let keys = &keys;
        move |name: &str| {
            keys.get(&(r, name.to_owned()))
                .cloned()
                .unwrap_or_else(|| name.to_owned())
        }
    };
    let mut defs = compat::Defs::new();
    for (name, reps) in &classes {
        for (index, r) in reps.iter().enumerate() {
            let Some(root) = roots.get(*r) else { continue };
            let def = keyed(&root["$defs"][name.as_str()], &key_in(*r));
            defs.insert(def_key(name, index), def);
        }
    }
    let mut out = Schemas::default();
    let mut shared_replies = HashMap::new();
    for (name, schema, r, reply) in descriptions {
        if reply && reply_uses.get(&r).is_some_and(|uses| *uses > 1) {
            // Operations returning the same kind of outcome share the entire
            // document-reply schema. Record it once under its first entry.
            let key = shared_replies.entry(r).or_insert_with(|| {
                let key = format!("@reply.{name}");
                defs.insert(key.clone(), keyed(schema, &key_in(r)));
                key
            });
            out.replies
                .insert(name.clone(), json!({"$ref": format!("{DEFS}{key}")}));
        } else {
            let table = if reply {
                &mut out.replies
            } else {
                &mut out.entries
            };
            table.insert(name.clone(), keyed(schema, &key_in(r)));
        }
    }
    // A shared definition is recorded from one root, so another root's
    // copy of a type it names may be named by nothing: keep only what the
    // entries reach.
    let mut queue = Vec::new();
    out.entries
        .values()
        .chain(out.replies.values())
        .for_each(|e| visit_refs(e, |name| queue.push(name)));
    while let Some(key) = queue.pop() {
        if !out.defs.contains_key(key)
            && let Some(def) = defs.get(key)
        {
            out.defs.insert(key.to_owned(), def.clone());
            visit_refs(def, |name| queue.push(name));
        }
    }
    out
}

fn lines(out: &mut String, map: &BTreeMap<String, Value>) -> Result<(), serde_json::Error> {
    for (i, (name, schema)) in map.iter().enumerate() {
        out.push_str(&format!(
            " {}: {}{}\n",
            serde_json::to_string(name)?,
            serde_json::to_string(schema)?,
            if i + 1 < map.len() { "," } else { "" }
        ));
    }
    Ok(())
}

/// [`SCHEMAS_FILE`]'s text: one line per definition and per entry.
pub fn schemas_text(schemas: &Schemas) -> Result<String, serde_json::Error> {
    let mut out = format!("{{\"format\": {SCHEMAS_FORMAT},\n\"defs\": {{\n");
    lines(&mut out, &schemas.defs)?;
    out.push_str("},\n\"entries\": {\n");
    lines(&mut out, &schemas.entries)?;
    out.push_str("},\n\"replies\": {\n");
    lines(&mut out, &schemas.replies)?;
    out.push_str("}}\n");
    Ok(out)
}

/// [`SCHEMAS_FILE`] read back; a format this build does not read is an
/// error that names it.
pub fn read_schemas(text: &str) -> Result<Schemas, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let format = value["format"].as_u64();
    if !matches!(format, Some(1 | SCHEMAS_FORMAT)) {
        return Err(format!(
            "{SCHEMAS_FILE} has format {}; this build reads {SCHEMAS_FORMAT}",
            value["format"]
        ));
    }
    let table = |key: &str| match value.get(key) {
        Some(Value::Object(map)) => Ok(map
            .iter()
            .map(|(name, schema)| (name.clone(), schema.clone()))
            .collect()),
        _ => Err(format!("{SCHEMAS_FILE} has no {key}")),
    };
    Ok(Schemas {
        defs: table("defs")?,
        entries: table("entries")?,
        replies: if format == Some(1) {
            BTreeMap::new()
        } else {
            table("replies")?
        },
    })
}

/// The schemas a ratchet compares: the recorded ones, when there are, and
/// the current ones.
#[derive(Clone, Copy, Default)]
pub struct Compared<'a> {
    pub old: Option<&'a Schemas>,
    pub new: Option<&'a Schemas>,
}

impl Compared<'_> {
    /// How `name`'s parameters changed, when both sides are recorded.
    fn classify(&self, name: &str) -> Option<compat::Outcome> {
        let (old, new) = (self.old?, self.new?);
        Some(compat::classify(
            old.entries.get(name)?,
            &old.defs,
            new.entries.get(name)?,
            &new.defs,
        ))
    }

    fn reply(&self, name: &str) -> Option<compat::Outcome> {
        let (old, new) = (self.old?, self.new?);
        Some(compat::classify_reply(
            old.replies.get(name)?,
            &old.defs,
            new.replies.get(name)?,
            &new.defs,
        ))
    }
}

/// The lock's text: one line per entry, so a change reads as a line.
pub fn text(lock: &Lock) -> Result<String, serde_json::Error> {
    let mut out = format!("{{\"format\": {}, \"modules\": [\n", lock.format);
    for (m, module) in lock.modules.iter().enumerate() {
        out.push_str(&format!(
            " {{\"id\": {}, \"version\": {}, \"digest\": {}, \"entries\": [\n",
            serde_json::to_string(&module.id)?,
            serde_json::to_string(&module.version)?,
            serde_json::to_string(&module.digest)?,
        ));
        for (e, entry) in module.entries.iter().enumerate() {
            out.push_str("  ");
            out.push_str(&serde_json::to_string(entry)?);
            out.push_str(if e + 1 < module.entries.len() {
                ",\n"
            } else {
                "\n"
            });
        }
        out.push_str(if m + 1 < lock.modules.len() {
            " ]},\n"
        } else {
            " ]}\n"
        });
    }
    out.push_str("]}\n");
    Ok(out)
}

/// What a change from `locked` to `current` breaks without the version it
/// needs, in plain words. Empty when every change is numbered.
pub fn ratchet(
    locked: &Lock,
    current: &Lock,
    declared: &[ModuleDecl],
    schemas: Compared<'_>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let since: HashMap<&str, (u16, u16)> = metadata::entries()
        .iter()
        .map(|e| (e.name.as_str(), e.since))
        .collect();
    let since_of = |name: &str| since.get(name).copied();
    for old in &locked.modules {
        if !current.modules.iter().any(|m| m.id == old.id) {
            problems.push(format!("module {} was removed", old.id));
        }
    }
    for new in &current.modules {
        let Some(old) = locked.modules.iter().find(|m| m.id == new.id) else {
            continue;
        };
        let (Some(was), Some(now)) = (parse_version(&old.version), parse_version(&new.version))
        else {
            problems.push(format!("module {} has an unreadable version", new.id));
            continue;
        };
        if now < was {
            problems.push(format!(
                "module {} went back from {} to {}",
                new.id, old.version, new.version
            ));
            continue;
        }
        if old.digest == new.digest {
            continue;
        }
        if now == was {
            problems.push(format!(
                "module {} changed without a version change: raise it from {} ({})",
                new.id,
                old.version,
                breaking(old, new, schemas)
                    .first()
                    .map_or("a minor change", String::as_str)
            ));
            continue;
        }
        // Before 1.0 the minor is the breaking number, as in semver: any
        // raise numbers a break. From 1.0 a break raises the major.
        if now.0 == was.0 && was.0 != 0 {
            for why in breaking(old, new, schemas) {
                problems.push(format!(
                    "module {} {} needs a major version, not {}",
                    new.id, why, new.version
                ));
            }
        }
        let known: BTreeSet<&str> = old.entries.iter().map(|e| e.name.as_str()).collect();
        for entry in new
            .entries
            .iter()
            .filter(|e| !known.contains(e.name.as_str()))
        {
            let declared_version = declared.iter().find(|m| m.id == new.id).map(|m| m.version);
            if since_of(&entry.name).is_some() && since_of(&entry.name) != declared_version {
                problems.push(format!(
                    "{} is new in module {} {}: declare it since({}, {})",
                    entry.name, new.id, new.version, now.0, now.1
                ));
            }
        }
    }
    problems
}

/// The changes from `old` to `new` that break a caller. With both schemas
/// recorded an entry's parameters compare structurally ([`compat`]);
/// without, by the lock's digests (any retyped parameter breaks).
fn breaking(old: &LockedModule, new: &LockedModule, schemas: Compared<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for was in &old.entries {
        let Some(now) = new.entries.iter().find(|e| e.name == was.name) else {
            out.push(format!("removed {}", was.name));
            continue;
        };
        if now.class > was.class {
            out.push(format!(
                "made {} stricter ({} to {})",
                was.name,
                was.class.name(),
                now.class.name()
            ));
        }
        if was.reply != now.reply && !was.reply.is_empty() {
            match schemas.reply(&was.name) {
                Some(change) if change.change == compat::Change::Major => {
                    out.extend(
                        change
                            .reasons
                            .iter()
                            .map(|why| format!("changed {} reply{why}", was.name)),
                    );
                }
                None => out.push(format!(
                    "changed {} reply without comparable schemas",
                    was.name
                )),
                Some(_) => {}
            }
        }
        if let Some(outcome) = schemas.classify(&was.name) {
            if outcome.change == compat::Change::Major {
                out.extend(
                    outcome
                        .reasons
                        .iter()
                        .map(|why| format!("changed {}{why}", was.name)),
                );
            }
            continue;
        }
        for (field, digest) in &was.fields {
            match now.fields.iter().find(|(name, _)| name == field) {
                None => out.push(format!("removed {}.{field}", was.name)),
                Some((_, other)) if other != digest => {
                    out.push(format!("retyped {}.{field}", was.name));
                }
                Some(_) => {}
            }
        }
        for field in &now.required {
            if !was.required.contains(field) {
                out.push(format!("made {}.{field} required", was.name));
            }
        }
    }
    out
}

mod compat;

#[cfg(test)]
mod tests;

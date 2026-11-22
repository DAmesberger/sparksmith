use super::*;
use sparksmith_core::Class;
use std::path::PathBuf;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn lock_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(LOCK_FILE)
}

/// Format 1 lacked reply digests and is accepted for the first update.
fn read_lock(bytes: &[u8]) -> Fallible<Lock> {
    let lock: Lock = serde_json::from_slice(bytes)?;
    if !(1..=LOCK_FORMAT).contains(&lock.format) {
        return Err(format!("{LOCK_FILE} has unsupported format {}", lock.format).into());
    }
    if lock.format == LOCK_FORMAT
        && lock
            .modules
            .iter()
            .flat_map(|m| &m.entries)
            .any(|e| e.reply.is_empty())
    {
        return Err(format!("{LOCK_FILE} format {} requires reply digests", lock.format).into());
    }
    Ok(lock)
}

/// Format 1 stripped schema annotations even inside its property maps, so
/// seven required wire fields had a name but no recorded schema. Compare
/// only the information that lock actually captured during the transition.
/// The new lock always records the full property types and all replies.
fn inputs_as_recorded_in_format_one(schemas: &Schemas) -> Schemas {
    fn legacy(value: &Value) -> Value {
        match value {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "description" | "title" | "$comment" | "examples" | "$schema" | "$defs"
                        )
                    })
                    .map(|(key, child)| (key.clone(), legacy(child)))
                    .collect(),
            ),
            Value::Array(values) => Value::Array(values.iter().map(legacy).collect()),
            value => value.clone(),
        }
    }
    let table = |values: &BTreeMap<String, Value>| {
        values
            .iter()
            .map(|(name, value)| (name.clone(), legacy(value)))
            .collect()
    };
    Schemas {
        defs: table(&schemas.defs),
        entries: table(&schemas.entries),
        ..Schemas::default()
    }
}

#[test]
fn the_upgrade_checks_the_input_constraints_format_one_recorded() {
    let schemas = |kind: &str| Schemas {
        entries: BTreeMap::from([(
            "example".into(),
            json!({"type":"object",
            "required":["title","id"],"properties":{"title":{"type":"string"},"id":{"type":kind}}}),
        )]),
        ..Schemas::default()
    };
    let previous = inputs_as_recorded_in_format_one(&schemas("integer"));
    assert!(
        previous.entries["example"]["properties"]
            .get("title")
            .is_none()
    );
    let updated = inputs_as_recorded_in_format_one(&schemas("string"));
    assert_eq!(
        Compared {
            old: Some(&previous),
            new: Some(&updated)
        }
        .classify("example")
        .map(|outcome| outcome.change),
        Some(compat::Change::Major)
    );
}

/// `cargo xtask surface --check` runs this; `--update` sets
/// `SPARKSMITH_SURFACE_UPDATE=1`, which rewrites the lock once the
/// ratchet passes.
#[test]
fn the_lock_holds() -> Fallible {
    let current = current();
    let path = lock_path();
    let locked: Option<Lock> = match std::fs::read(&path) {
        Ok(bytes) => Some(read_lock(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let schemas_path = lock_path().with_file_name(SCHEMAS_FILE);
    let recorded: Option<Schemas> = match std::fs::read_to_string(&schemas_path) {
        Ok(text) => Some(read_schemas(&text)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let now = schemas();
    let legacy = locked
        .as_ref()
        .filter(|lock| lock.format == 1)
        .map(|_| inputs_as_recorded_in_format_one(&now));
    let compared = Compared {
        old: recorded.as_ref(),
        new: Some(legacy.as_ref().unwrap_or(&now)),
    };
    let problems = locked
        .as_ref()
        .map(|locked| ratchet(locked, &current, MODULES, compared))
        .unwrap_or_default();
    if std::env::var_os("SPARKSMITH_SURFACE_UPDATE").is_some() {
        if !problems.is_empty() {
            return Err(format!("surface.lock not updated:\n{}", problems.join("\n")).into());
        }
        std::fs::write(&path, text(&current)?)?;
        std::fs::write(&schemas_path, schemas_text(&now)?)?;
        return Ok(());
    }
    let Some(locked) = locked else {
        return Err("surface.lock is missing: run cargo xtask surface --update".into());
    };
    if !problems.is_empty() {
        return Err(format!(
            "the catalog changed without the version it needs (crates/automation/surface.lock):\n{}",
            problems.join("\n")
        )
        .into());
    }
    if locked != current || recorded.as_ref() != Some(&now) {
        return Err(
            "surface.lock or surface.schemas is out of date: the versions are right, so run cargo xtask surface --update"
                .into(),
        );
    }
    Ok(())
}

#[test]
fn every_module_has_entries_and_every_entry_one_module() {
    let lock = current();
    let mut seen = BTreeSet::new();
    for module in &lock.modules {
        assert!(!module.entries.is_empty(), "module {} is empty", module.id);
        for entry in &module.entries {
            assert!(seen.insert(entry.name.clone()), "{} twice", entry.name);
        }
    }
    assert_eq!(seen.len(), metadata::entries().len());
}

#[test]
fn the_lock_text_reads_back() -> Fallible {
    let lock = current();
    let read: Lock = serde_json::from_str(&text(&lock)?)?;
    assert_eq!(read, lock);
    Ok(())
}

/// A renamed type or an edited doc comment changes no digest; a retyped
/// field does.
#[test]
fn the_canonical_form_ignores_names_and_words() {
    let a = json!({"properties": {"x": {"$ref": "#/$defs/A", "description": "one"}},
        "$defs": {"A": {"type": "integer", "title": "A"}}});
    let b = json!({"properties": {"x": {"$ref": "#/$defs/Renamed", "description": "two"}},
        "$defs": {"Renamed": {"type": "integer"}}});
    let c = json!({"properties": {"x": {"$ref": "#/$defs/A"}},
        "$defs": {"A": {"type": "string"}}});
    assert_eq!(digest(&canonical(&a, &a)), digest(&canonical(&b, &b)));
    assert_ne!(digest(&canonical(&a, &a)), digest(&canonical(&c, &c)));
}

#[test]
fn discovery_follows_the_schema_of_a_property_named_ref() {
    let schema = json!({
        "properties": {"$ref": {"$ref": "#/$defs/Actual"}},
        "enum": [{"$ref": "#/$defs/Literal"}]
    });
    let definitions = Map::from_iter([
        (
            "Actual".to_owned(),
            json!({"properties": {"$defs": {"$ref": "#/$defs/Nested"}}}),
        ),
        ("Nested".to_owned(), json!({"type": "integer"})),
        ("Literal".to_owned(), json!({"type": "string"})),
    ]);
    let reached = discovery::reachable_defs(&schema, &definitions);
    assert_eq!(reached.len(), 2);
    assert_eq!(reached.get("Actual"), definitions.get("Actual"));
    assert_eq!(reached.get("Nested"), definitions.get("Nested"));
}

#[test]
fn annotations_are_ignored_only_on_schemas() {
    let literal = json!({"title": "wire value", "$ref": "#/$defs/NotAReference"});
    let mut schema = json!({
        "title": "schema annotation",
        "$defs": {"Actual": {"type": "integer"}},
        "properties": {"payload": {"const": literal, "enum": [literal]}},
        "patternProperties": {"description": {"type": "boolean"}},
        "dependentSchemas": {"examples": {"required": ["title"]}}
    });
    let names = [
        "description",
        "title",
        "examples",
        "$schema",
        "$comment",
        "$defs",
        "$ref",
    ];
    for name in names {
        schema["properties"][name] = json!({"$ref": "#/$defs/Actual", "description": "annotation"});
    }
    let canonical = canonical(&schema, &schema);
    let recorded = keyed(&schema, &|_| "Renamed".into());
    assert!(recorded.get("title").is_none());
    for (form, reference) in [
        (&canonical["schema"], "#/$defs/0"),
        (&recorded, "#/$defs/Renamed"),
    ] {
        for name in names {
            assert_eq!(
                form["properties"][name],
                json!({"$ref": reference}),
                "{name}"
            );
        }
        assert_eq!(form["properties"]["payload"]["const"], literal);
        assert_eq!(form["properties"]["payload"]["enum"], json!([literal]));
        assert_eq!(form["patternProperties"], schema["patternProperties"]);
        assert_eq!(form["dependentSchemas"], schema["dependentSchemas"]);
    }
    let mut reached = Vec::new();
    visit_refs(&schema, |name| reached.push(name));
    assert_eq!(reached, vec!["Actual"; names.len()]);
    for name in names {
        let mut changed = schema.clone();
        changed["properties"][name] = json!({"type": "string"});
        assert!(
            !same(&schema, &schema, &changed, &changed, &mut BTreeSet::new()),
            "{name}"
        );
    }
    let mut changed = schema.clone();
    changed["properties"]["payload"]["const"]["title"] = json!("different");
    assert!(!same(
        &schema,
        &schema,
        &changed,
        &changed,
        &mut BTreeSet::new()
    ));
}

fn module(version: &str, digest: &str, entries: Vec<LockedEntry>) -> Lock {
    Lock {
        format: LOCK_FORMAT,
        modules: vec![LockedModule {
            id: "layout".into(),
            version: version.into(),
            digest: digest.into(),
            entries,
        }],
    }
}

fn entry(name: &str, class: Class, fields: &[(&str, &str)], required: &[&str]) -> LockedEntry {
    LockedEntry {
        name: name.into(),
        class,
        schema: "0".into(),
        reply: "0".into(),
        fields: fields
            .iter()
            .map(|(n, d)| ((*n).to_owned(), (*d).to_owned()))
            .collect(),
        required: required.iter().map(|r| (*r).to_owned()).collect(),
    }
}

fn base(version: &str) -> Lock {
    module(
        version,
        "sha256:a",
        vec![
            entry(
                "layout.a",
                Class::Edit,
                &[("at", "1"), ("net", "2")],
                &["at"],
            ),
            entry("layout.b", Class::Read, &[], &[]),
        ],
    )
}

/// The planted changes: each fails without a major bump on a 1.x module
/// and passes with one.
#[test]
fn a_breaking_change_needs_a_major_version() {
    let planted = [
        (
            "a removed entry",
            vec![entry(
                "layout.a",
                Class::Edit,
                &[("at", "1"), ("net", "2")],
                &["at"],
            )],
        ),
        (
            "a retyped field",
            vec![
                entry(
                    "layout.a",
                    Class::Edit,
                    &[("at", "9"), ("net", "2")],
                    &["at"],
                ),
                entry("layout.b", Class::Read, &[], &[]),
            ],
        ),
        (
            "a stricter class",
            vec![
                entry(
                    "layout.a",
                    Class::Ask,
                    &[("at", "1"), ("net", "2")],
                    &["at"],
                ),
                entry("layout.b", Class::Read, &[], &[]),
            ],
        ),
        (
            "a removed field",
            vec![
                entry("layout.a", Class::Edit, &[("at", "1")], &["at"]),
                entry("layout.b", Class::Read, &[], &[]),
            ],
        ),
        (
            "a new required field",
            vec![
                entry(
                    "layout.a",
                    Class::Edit,
                    &[("at", "1"), ("net", "2")],
                    &["at", "net"],
                ),
                entry("layout.b", Class::Read, &[], &[]),
            ],
        ),
    ];
    for (what, entries) in planted {
        let unchanged = module("1.2", "sha256:b", entries.clone());
        assert!(
            !ratchet(&base("1.2"), &unchanged, &[], Compared::default()).is_empty(),
            "{what} passed without a version change"
        );
        let minor = module("1.3", "sha256:b", entries.clone());
        assert!(
            !ratchet(&base("1.2"), &minor, &[], Compared::default()).is_empty(),
            "{what} passed with a minor bump"
        );
        let major = module("2.0", "sha256:b", entries);
        assert_eq!(
            ratchet(&base("1.2"), &major, &[], Compared::default()),
            Vec::<String>::new(),
            "{what}"
        );
    }
}

#[test]
fn a_compatible_change_needs_a_minor_version() {
    let added = vec![
        entry(
            "layout.a",
            Class::Edit,
            &[("at", "1"), ("net", "2"), ("pad", "3")],
            &["at"],
        ),
        entry("layout.b", Class::Read, &[], &[]),
    ];
    let unchanged = module("1.2", "sha256:b", added.clone());
    assert!(!ratchet(&base("1.2"), &unchanged, &[], Compared::default()).is_empty());
    let minor = module("1.3", "sha256:b", added);
    assert_eq!(
        ratchet(&base("1.2"), &minor, &[], Compared::default()),
        Vec::<String>::new()
    );
    let back = module("1.1", "sha256:a", base("1.2").modules[0].entries.clone());
    assert!(!ratchet(&base("1.2"), &back, &[], Compared::default()).is_empty());
}

/// Before 1.0 the minor numbers every break, as in semver.
#[test]
fn before_one_a_break_raises_the_minor() {
    let removed = vec![entry(
        "layout.a",
        Class::Edit,
        &[("at", "1"), ("net", "2")],
        &["at"],
    )];
    let unchanged = module("0.1", "sha256:b", removed.clone());
    assert!(!ratchet(&base("0.1"), &unchanged, &[], Compared::default()).is_empty());
    let minor = module("0.2", "sha256:b", removed);
    assert_eq!(
        ratchet(&base("0.1"), &minor, &[], Compared::default()),
        Vec::<String>::new()
    );
}

#[test]
fn a_removed_module_fails() {
    let empty = Lock {
        format: LOCK_FORMAT,
        modules: Vec::new(),
    };
    assert!(!ratchet(&base("0.1"), &empty, &[], Compared::default()).is_empty());
}

fn schemas_of(zone: Value, required: Value) -> Schemas {
    Schemas {
        replies: BTreeMap::new(),
        defs: BTreeMap::from([(
            "Zone".to_owned(),
            json!({"type": "object", "properties": zone, "required": required}),
        )]),
        entries: BTreeMap::from([(
            "layout.a".to_owned(),
            json!({"type": "object",
                "properties": {"at": {"type": "integer"}, "zone": {"$ref": "#/$defs/Zone"}},
                "required": ["at"]}),
        )]),
    }
}

fn zone_entries(zone_digest: &str) -> Vec<LockedEntry> {
    vec![
        entry(
            "layout.a",
            Class::Edit,
            &[("at", "1"), ("zone", zone_digest)],
            &["at"],
        ),
        entry("layout.b", Class::Read, &[], &[]),
    ]
}

/// With the schemas recorded, a parameter whose digest changed is weighed
/// by what changed inside it: a new optional nested field needs a minor
/// version, a new required one a major. Without them, the digest decides.
#[test]
fn recorded_schemas_decide_a_changed_parameter() {
    let old = schemas_of(json!({"p": {"type": "integer"}}), json!(["p"]));
    let optional = schemas_of(
        json!({"p": {"type": "integer"}, "q": {"type": "integer", "default": 0}}),
        json!(["p"]),
    );
    let required = schemas_of(
        json!({"p": {"type": "integer"}, "q": {"type": "integer"}}),
        json!(["p", "q"]),
    );
    let old_lock = module("1.2", "sha256:a", zone_entries("3"));
    fn both<'a>(old: &'a Schemas, new: &'a Schemas) -> Compared<'a> {
        Compared {
            old: Some(old),
            new: Some(new),
        }
    }
    let minor = module("1.3", "sha256:b", zone_entries("9"));
    assert_eq!(
        ratchet(&old_lock, &minor, &[], both(&old, &optional)),
        Vec::<String>::new()
    );
    assert!(!ratchet(&old_lock, &minor, &[], Compared::default()).is_empty());
    let problems = ratchet(&old_lock, &minor, &[], both(&old, &required));
    assert!(
        problems.iter().any(|p| p.contains("layout.a.zone.q")),
        "{problems:?}"
    );
    let major = module("2.0", "sha256:b", zone_entries("9"));
    assert_eq!(
        ratchet(&old_lock, &major, &[], both(&old, &required)),
        Vec::<String>::new()
    );
}

/// Every reference in the recorded schemas names a recorded definition,
/// every definition is reached, and the catalog compared with itself is
/// unchanged.
#[test]
fn the_recorded_schemas_are_whole() {
    let now = schemas();
    let mut reached = BTreeSet::new();
    let all = now
        .entries
        .values()
        .chain(now.replies.values())
        .chain(now.defs.values());
    for schema in all {
        let mut names = Vec::new();
        visit_refs(schema, |name| names.push(name));
        for name in names {
            assert!(now.defs.contains_key(name), "{name} is not recorded");
            reached.insert(name.to_owned());
        }
    }
    let unreached: Vec<&String> = now.defs.keys().filter(|k| !reached.contains(*k)).collect();
    assert!(unreached.is_empty(), "nothing names {unreached:?}");
    let both = Compared {
        old: Some(&now),
        new: Some(&now),
    };
    for name in now.entries.keys() {
        assert_eq!(
            both.classify(name).map(|o| o.change),
            Some(compat::Change::Same),
            "{name}"
        );
    }
}

/// Two definitions of one name are recorded apart only when they differ.
#[test]
fn a_definition_is_shared_only_when_it_is_the_same() {
    let a = json!({"$defs": {"P": {"type": "integer", "description": "one"}}});
    let b = json!({"$defs": {"P": {"type": "integer"}}});
    let c = json!({"$defs": {"P": {"type": "string"}}});
    let p = json!({"$ref": "#/$defs/P"});
    assert!(same(&p, &a, &p, &b, &mut BTreeSet::new()));
    assert!(!same(&p, &a, &p, &c, &mut BTreeSet::new()));
}

#[test]
fn the_schemas_text_reads_back() -> Fallible {
    let now = schemas();
    assert_eq!(read_schemas(&schemas_text(&now)?)?, now);
    assert!(read_schemas("{\"format\": 99, \"entries\": {}}").is_err());
    Ok(())
}

/// A 1.x reply can gain a field an older reader ignores. Its documented
/// fields and possible status values are contracts for that reader.
#[test]
fn a_reply_change_is_weighed_from_the_readers_side() {
    let reply = |values: &[&str], required: &[&str]| {
        json!({"type":"object","properties":{
            "rows":{"type":"array","items":{"$ref":"#/$defs/Row"}},
            "status":{"enum":values}
        }, "required":required})
    };
    let old = reply(&["ready", "busy"], &["rows", "status"]);
    let with_extra = |mut schema: Value| {
        schema["properties"]["hint"] = json!({"type":"string"});
        schema
    };
    let defs = BTreeMap::from([("Row".to_owned(), json!({"type":"string"}))]);
    let weigh = |new: Value| compat::classify_reply(&old, &defs, &new, &defs).change;
    assert_eq!(weigh(with_extra(old.clone())), compat::Change::Minor);
    let mut closed = old.clone();
    closed["additionalProperties"] = json!(false);
    assert_eq!(
        compat::classify_reply(&closed, &defs, &with_extra(closed.clone()), &defs).change,
        compat::Change::Major
    );
    assert_eq!(
        weigh(reply(&["ready"], &["rows", "status"])),
        compat::Change::Minor
    );
    assert_eq!(
        weigh(reply(&["ready", "busy", "queued"], &["rows", "status"])),
        compat::Change::Major
    );
    assert_eq!(
        weigh(reply(&["ready", "busy"], &["rows"])),
        compat::Change::Major
    );
    let referenced = json!({"$ref": "#/$defs/Row", "maxLength": 4});
    let widened = json!({"$ref": "#/$defs/Row", "maxLength": 8});
    assert_eq!(
        compat::classify_reply(&referenced, &defs, &widened, &defs).change,
        compat::Change::Major
    );
    let changed = BTreeMap::from([("Row".to_owned(), json!({"type":"number"}))]);
    assert_eq!(
        compat::classify_reply(&old, &defs, &old, &changed).change,
        compat::Change::Major
    );
}

#[test]
fn each_locked_entry_has_its_discoverable_reply_recorded() {
    let lock = current();
    let schemas = schemas();
    for entry in lock.modules.iter().flat_map(|module| &module.entries) {
        assert!(schemas.replies.contains_key(&entry.name), "{}", entry.name);
        assert_eq!(entry.reply.len(), SHORT, "{}", entry.name);
    }
}

#[test]
fn a_reply_only_break_requires_a_major_module_version() {
    let mut was = base("1.2");
    let mut now = base("1.3");
    was.modules[0].entries[0].reply = "old-reply".into();
    now.modules[0].entries[0].reply = "new-reply".into();
    now.modules[0].digest = "changed".into();
    let reply = |schema| Schemas {
        replies: BTreeMap::from([("layout.a".into(), schema)]),
        ..Default::default()
    };
    let old = reply(json!({"type":"string","enum":["ready"]}));
    let new = reply(json!({"type":"string","enum":["ready","pending"]}));
    let schemas = Compared {
        old: Some(&old),
        new: Some(&new),
    };
    let reasons = ratchet(&was, &now, &[], schemas);
    assert!(
        reasons
            .iter()
            .any(|why| why.contains("layout.a reply") && why.contains("pending")),
        "{reasons:?}"
    );
    now.modules[0].version = "2.0".into();
    assert!(ratchet(&was, &now, &[], schemas).is_empty());
}

#[test]
fn unsupported_or_incomplete_reply_locks_are_refused() -> Fallible {
    let mut lock = base("1.2");
    lock.format = LOCK_FORMAT + 1;
    assert!(read_lock(text(&lock)?.as_bytes()).is_err());
    lock.format = LOCK_FORMAT;
    lock.modules[0].entries[0].reply.clear();
    assert!(read_lock(text(&lock)?.as_bytes()).is_err());
    lock.format = 1;
    assert_eq!(read_lock(text(&lock)?.as_bytes())?, lock);
    Ok(())
}

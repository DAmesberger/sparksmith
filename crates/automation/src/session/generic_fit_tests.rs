//! A generic part placed from the catalog's words fits an orderable Part
//! from a library folder (the starter-set scenario), through JSON only:
//! the folder's Parts pin the same generic part the chip generator makes,
//! so the resolve search lists them, importing one maps its component onto
//! the placed one, `instance.assign_part` takes it, the BOM shows its MPN,
//! the marking follows its tolerance, and a variant fits a second maker's
//! Part whose rated values match, resolved once a person confirms its
//! pinout. The makers and part numbers are invented.
use super::*;
use sources::Sources;
use sparksmith_io::settings::UserDirs;
use sparksmith_io::shipped::ShippedLibrary;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const FIRST: (&str, &str) = ("ExampleCo", "EX-R0603-10K0-F");
const SECOND: (&str, &str) = ("SampleWorks", "SW-0603-10K-1");

fn session(dirs: &Path) -> Fallible<Session> {
    let mut session = Session::with_sources(
        Engine::default(),
        Sources::with_dirs(UserDirs {
            config: dirs.join("config"),
            cache: dirs.join("cache"),
        })?,
    );
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    session.set_shipped(ShippedLibrary::at(&shipped)?);
    Ok(session)
}

fn ok(session: &mut Session, command: &str, mut params: Value) -> Fallible<Value> {
    if let Some(map) = params.as_object_mut()
        && !map.contains_key("expected_revision")
        && matches!(
            command,
            "project.apply"
                | "library.generate_part"
                | "library.publish_to_folder"
                | "library.import_from_folder"
        )
    {
        map.insert(
            "expected_revision".into(),
            json!(session.engine().revision()),
        );
    }
    let reply = session.dispatch(json!({"command": command, "params": params}));
    assert_eq!(reply["ok"], true, "{command}: {reply}");
    Ok(reply["result"].clone())
}

fn query(session: &mut Session, name: &str, params: Value) -> Fallible<Value> {
    Ok(ok(
        session,
        "project.query",
        json!({"query": name, "params": params}),
    )?["result"]
        .clone())
}

/// The generic part `words` name, made through its generator as the Place
/// palette makes it: the search hint with its component and values.
fn generic(session: &mut Session, words: &str) -> Fallible<Value> {
    let search = |session: &mut Session| -> Fallible<Value> {
        let found = ok(
            session,
            "project.query",
            json!({"query": "library.search", "params": {"query": words}}),
        )?;
        Ok(found["generic"][0].clone())
    };
    let hint = search(session)?;
    if let Some(make) = hint.get("make") {
        let made = ok(session, "library.generate_part", make["params"].clone())?;
        assert_eq!(made["outcome"]["report"]["created"], true, "{made}");
        return search(session);
    }
    Ok(hint)
}

/// A standard field of the open project, as a field reference.
fn field(session: &Session, name: &str) -> Fallible<Value> {
    let _context = session.engine().enter_context();
    let field = sparksmith_core::library::standards::get()?
        .fields
        .local(&session.engine().project().library, name)
        .ok_or_else(|| format!("no {name} field"))?;
    Ok(serde_json::to_value(field)?)
}

/// Author and publish a 10 kΩ ±1 % 1/10 W ±100 ppm/K Part of `maker` on
/// the generic chip resistor `hint` names.
fn author_part(session: &mut Session, hint: &Value, (maker, mpn): (&str, &str)) -> Fallible<Value> {
    let component = hint["component"].clone();
    let binding = {
        let library = &session.engine().project().library;
        let published = library
            .component(serde_json::from_value(component.clone())?)
            .ok_or("the generic component")?;
        serde_json::to_value(
            published
                .definition
                .implementations
                .first()
                .ok_or("no binding")?,
        )?
    };
    let quantity = |si: &str, unit: &str| json!({"type": "quantity", "value": {"si": si, "display_unit": unit}});
    let tolerance =
        |amount: &str| json!({"type": "tolerance", "value": {"minus": amount, "plus": amount}});
    let specs = json!([
        {"field": field(session, "Resistance")?, "value": quantity("10000", "Ω"), "meaning": "exact_nominal"},
        {"field": field(session, "Tolerance")?, "value": tolerance("0.01"), "meaning": "maximum_tolerance"},
        {"field": field(session, "Rated power")?, "value": quantity("0.1", "W"), "meaning": "minimum_rating"},
        {"field": field(session, "Temperature coefficient")?, "value": tolerance("0.0001"), "meaning": "maximum_tolerance"},
    ]);
    let meta = json!({"name": mpn, "description": "Invented 10 kΩ ±1 % 0603 resistor.",
        "provenance": "Original test fixture; the maker is fictitious."});
    let authored = json!({"origin": null, "basis": {"basis": "authored"}, "licence": null});
    let body = json!({"next_local": 1, "manufacturer": maker, "mpn": mpn,
        "component": component, "binding": binding, "specs": specs,
        "references": [], "offers": [], "provenance": authored});
    let created = ok(
        session,
        "project.apply",
        json!({"operation": {"editor": "Library", "operation": {"command": "library.new_draft",
            "params": {"meta": meta, "definition": {"kind": "part", "body": body}}}}}),
    )?;
    let draft = created["outcome"]["id"].clone();
    let published = ok(
        session,
        "project.apply",
        json!({"operation": {"editor": "Library", "operation": {"command": "library.publish",
            "params": {"drafts": [draft]}}}}),
    )?;
    let part = published["outcome"]["revisions"]
        .as_array()
        .and_then(|r| r.iter().find(|r| r["kind"] == "part"))
        .ok_or_else(|| format!("no Part published: {published}"))?;
    Ok(json!({"kind": "part", "id": part["id"], "revision": part["revision"]}))
}

/// The folder Parts a search lists, by MPN.
fn folder_parts(session: &mut Session, text: &str) -> Fallible<Vec<(String, Value)>> {
    let found = query(
        session,
        "library.search",
        json!({"query": text, "limit": 50}),
    )?;
    Ok(found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|hit| &hit["entry"])
        .filter(|e| e["kind"] == "part" && e["source"]["library"].is_string())
        .map(|e| {
            (
                e["mpn"].as_str().unwrap_or_default().to_owned(),
                e["source_lineage"].clone(),
            )
        })
        .collect())
}

fn marking(session: &Session, instance: u64) -> Fallible<Vec<String>> {
    use sparksmith_core::library::marking::LaidElement;
    let _context = session.engine().enter_context();
    let layout = sparksmith_core::commands::instance_marking(session.engine().project(), instance)?
        .ok_or("not marked")?;
    Ok(layout
        .all()
        .flat_map(|l| &l.elements)
        .filter_map(|e| match e {
            LaidElement::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect())
}

#[test]
fn a_generic_resistor_fits_folder_parts_of_two_makers() -> TestResult {
    let temp = tempfile::tempdir()?;
    let folder = temp.path().join("parts");

    // The library's author makes the generic part, rates two makers'
    // Parts on it and publishes both to a folder.
    let mut author = session(&temp.path().join("author"))?;
    let hint = generic(&mut author, "resistor 0603")?;
    let first = author_part(&mut author, &hint, FIRST)?;
    let second = author_part(&mut author, &hint, SECOND)?;
    let created = ok(
        &mut author,
        "library.source.create",
        json!({"path": folder.display().to_string(), "name": "Starter parts"}),
    )?;
    let library = created["created"]["library"].clone();
    for root in [&first, &second] {
        ok(
            &mut author,
            "library.publish_to_folder",
            json!({"root": root, "library": library}),
        )?;
    }

    // A designer places a generic 10k 0603 from the catalog's words.
    let mut designer = session(&temp.path().join("designer"))?;
    ok(
        &mut designer,
        "library.source.add",
        json!({"path": folder.display().to_string()}),
    )?;
    let hint = generic(&mut designer, "10k 0603")?;
    let representation = designer
        .engine()
        .project()
        .library
        .component(serde_json::from_value(hint["component"].clone())?)
        .and_then(|c| c.definition.representations.first().map(|r| r.id))
        .ok_or("no symbol")?;
    let placed = ok(
        &mut designer,
        "project.apply",
        json!({"operation": {"editor": "Instance", "operation": {"command": "component.place",
            "params": {"component": hint["component"], "designator": "R1",
                "properties": hint["properties"], "representation": representation,
                "unit": 1, "position": [0.0, 0.0], "physical": hint["physical"]}}}}),
    )?;
    let r1 = placed["outcome"]["instance"]
        .as_u64()
        .ok_or("no instance")?;
    // Without a Part the EIA-96 code has no tolerance to go on.
    assert_eq!(marking(&designer, r1)?, ["103"]);

    // Both makers' Parts are found by value words and by the resolve
    // group's own search, which names the placed component.
    let mut by_words: Vec<String> = folder_parts(&mut designer, "kind:part 10k 0603 1%")?
        .into_iter()
        .map(|(mpn, _)| mpn)
        .collect();
    by_words.sort();
    assert_eq!(by_words, [FIRST.1, SECOND.1]);
    let groups = query(&mut designer, "library.resolve_groups", json!({}))?;
    let group = groups["groups"][0]["query"]
        .as_str()
        .ok_or("no resolve group")?
        .to_owned();
    let fitting = folder_parts(&mut designer, &group)?;
    assert_eq!(fitting.len(), 2, "{group}: {fitting:?}");

    // Importing the first maker's Part pins the placed generic part: no
    // second chip resistor appears, and the Part is assigned.
    let components = designer.engine().project().library.components.len();
    let lineage = fitting
        .iter()
        .find(|(mpn, _)| mpn == FIRST.1)
        .map(|(_, l)| l.clone())
        .ok_or("the first Part")?;
    let imported = ok(
        &mut designer,
        "library.import_from_folder",
        json!({"library": library, "lineage": lineage}),
    )?;
    assert_eq!(
        designer.engine().project().library.components.len(),
        components,
        "the folder's generic part is the placed one"
    );
    let part = imported["outcome"]["report"]["root"].clone();
    ok(
        &mut designer,
        "project.apply",
        json!({"operation": {"editor": "Instance", "operation": {"command": "instance.assign_part",
            "params": {"instances": [r1], "part": {"id": part["id"], "revision": part["revision"]}}}}}),
    )?;
    let bom = query(&mut designer, "project.bom", json!({}))?;
    let line = &bom["lines"][0];
    assert_eq!(line["mpn"], FIRST.1, "{bom}");
    assert_eq!(line["manufacturer"], FIRST.0);
    assert_eq!(line["unresolved"], false, "{bom}");
    // The fitted Part's ±1 % gives the EIA-96 code.
    assert_eq!(marking(&designer, r1)?, ["01C"]);

    // A variant fits the second maker's Part on the same lands; matching
    // rated values leave only the person's pinout review.
    let lineage = fitting
        .iter()
        .find(|(mpn, _)| mpn == SECOND.1)
        .map(|(_, l)| l.clone())
        .ok_or("the second Part")?;
    let imported = ok(
        &mut designer,
        "library.import_from_folder",
        json!({"library": library, "lineage": lineage}),
    )?;
    let substitute = imported["outcome"]["report"]["root"].clone();
    let added = ok(
        &mut designer,
        "project.apply",
        json!({"operation": {"editor": "Variant", "operation": {"command": "variant.add",
            "params": {"name": "Second source"}}}}),
    )?;
    let variant = added["outcome"]["variant"].clone();
    for operation in [
        json!({"command": "variant.set_active", "params": {"variant": variant}}),
        json!({"command": "variant.set_part", "params": {"instance": r1, "variant": variant,
            "part": {"id": substitute["id"], "revision": substitute["revision"]}}}),
    ] {
        ok(
            &mut designer,
            "project.apply",
            json!({"operation": {"editor": "Variant", "operation": operation}}),
        )?;
    }
    let bom = query(&mut designer, "project.bom", json!({}))?;
    assert_eq!(bom["lines"][0]["mpn"], SECOND.1, "{bom}");
    assert_eq!(bom["lines"][0]["unresolved"], true, "unreviewed: {bom}");
    ok(
        &mut designer,
        "project.apply",
        json!({"operation": {"editor": "Variant", "operation": {"command": "variant.review_substitute",
            "params": {"instance": r1, "variant": variant, "equivalent": true}}}}),
    )?;
    let bom = query(&mut designer, "project.bom", json!({}))?;
    let line = &bom["lines"][0];
    assert_eq!(line["mpn"], SECOND.1, "{bom}");
    assert_eq!(line["manufacturer"], SECOND.0);
    assert_eq!(
        line["unresolved"], false,
        "reviewed, every requirement met: {bom}"
    );
    assert_eq!(bom["unresolved"], 0, "{bom}");
    Ok(())
}

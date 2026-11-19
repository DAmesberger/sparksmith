//! GP1 acceptance through the session (the CLI's and MCP's request path):
//! a part made from the shipped chip module, saved, reopened with the
//! generator folder gone; a changed embedded module and a changed stored
//! part refused with both hashes, the open document untouched.
use super::super::Session;
use serde_json::{Value, json};
use sparksmith_core::Engine;
use sparksmith_core::library::generated::MODULE_MEDIA_TYPE;
use sparksmith_io::shipped::{MARKER, ShippedLibrary};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The repository's chip generator bundle (`cargo xtask generators build`
/// keeps it equal to the crate's module and part files).
const BUNDLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library/generators/chip");
/// Its module.
#[allow(dead_code)]
const CHIP: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../library/generators/chip/chip.wasm"
);

/// A shipped library in `dir` holding only the chip generator's bundle.
fn library(dir: &Path) -> TestResult<PathBuf> {
    let root = dir.join("library");
    std::fs::create_dir_all(root.join("generators"))?;
    std::fs::write(root.join(MARKER), "{\"format\":1}\n")?;
    copy_dir(Path::new(BUNDLE), &root.join("generators/chip"))?;
    Ok(root)
}

fn copy_dir(from: &Path, to: &Path) -> TestResult {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        let target = to.join(path.file_name().ok_or("no file name")?);
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn session(root: &Path) -> TestResult<Session> {
    let mut session = Session::fixture(Engine::default());
    session.set_shipped(ShippedLibrary::at(root)?);
    Ok(session)
}

fn call(session: &mut Session, command: &str, mut params: Value) -> Value {
    if let Some(map) = params.as_object_mut()
        && !map.contains_key("expected_revision")
        && !matches!(command, "library.generators" | "project.save")
    {
        map.insert(
            "expected_revision".into(),
            json!(session.engine().revision()),
        );
    }
    session.dispatch(json!({"command": command, "params": params}))
}

/// A project with a chip resistor (component) and a chip capacitor
/// package made from the shipped module, saved at `path`.
fn generated_project(dir: &Path) -> TestResult<(Session, PathBuf)> {
    let root = library(dir)?;
    let mut session = session(&root)?;
    let made = call(
        &mut session,
        "library.generate_part",
        json!({"generator": "chip", "scope": "component", "params": {"part": "resistor"}}),
    );
    assert_eq!(made["ok"], true, "{made}");
    let report = &made["result"]["outcome"]["report"];
    assert_eq!(report["created"], true, "{report}");
    assert_eq!(report["revisions"].as_array().map(Vec::len), Some(5));
    let package = call(
        &mut session,
        "library.generate_part",
        json!({"generator": "chip", "scope": "package", "params": {"part": "capacitor", "length": 2_000_000, "width": 1_250_000, "height": 1_250_000, "termination": 500_000}}),
    );
    assert_eq!(package["ok"], true, "{package}");
    let path = dir.join("generated.sparksmith");
    let saved = call(
        &mut session,
        "project.save",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(saved["ok"], true, "{saved}");
    Ok((session, path))
}

#[test]
fn a_generated_part_reopens_offline_identically() -> TestResult {
    let temp = tempfile::tempdir()?;
    let (made, path) = generated_project(temp.path())?;
    let library = &made.engine().project().library;
    assert_eq!(library.generated.len(), 2);
    let modules = library
        .assets
        .iter()
        .filter(|a| a.media_type == MODULE_MEDIA_TYPE)
        .count();
    assert_eq!(modules, 1, "both parts share one embedded module");

    // The generator folder is gone: the project reopens from its own copy.
    std::fs::remove_dir_all(temp.path().join("library/generators"))?;
    let mut reopened = session(&temp.path().join("library"))?;
    let opened = call(
        &mut reopened,
        "project.open",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(reopened.engine().to_file(), made.engine().to_file());

    let listed = call(&mut reopened, "library.generators", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
    let parts = listed["result"]["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(parts.len(), 2);
    assert!(
        parts.iter().all(|p| p["state"] == "not_shipped"),
        "{listed}"
    );
    assert_eq!(listed["result"]["shipped"], json!([]));
    Ok(())
}

#[test]
fn the_same_part_twice_is_found_not_made_again() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut session = session(&root)?;
    let params = json!({"generator": "chip", "scope": "component", "preset": "resistor-0603"});
    let first = call(&mut session, "library.generate_part", params.clone());
    assert_eq!(first["ok"], true, "{first}");
    let revision = session.engine().revision();
    let again = call(&mut session, "library.generate_part", params);
    assert_eq!(again["ok"], true, "{again}");
    let report = &again["result"]["outcome"]["report"];
    assert_eq!(report["created"], false, "{report}");
    assert_eq!(
        report["key"], first["result"]["outcome"]["report"]["key"],
        "the part key is module hash and params hash"
    );
    assert_eq!(session.engine().revision(), revision, "nothing changed");
    let listed = call(&mut session, "library.generators", json!({}));
    assert_eq!(listed["result"]["parts"][0]["state"], "current", "{listed}");
    assert_eq!(listed["result"]["shipped"][0]["name"], "chip", "{listed}");
    // The host knows where its library is; the reply, which reaches a
    // model and its transcript, never names the host's path.
    let located = session.generators_report()?.shipped_library;
    let text = listed.to_string();
    assert!(!text.contains("shipped_library"), "{text}");
    for folder in [
        located.ok_or("the host's library")?,
        root.display().to_string(),
    ] {
        assert!(!text.contains(&folder), "{folder} in {text}");
    }
    Ok(())
}

#[test]
fn refusals_change_nothing() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut session = session(&root)?;
    let before = session.engine().to_file();
    let revision = session.engine().revision();
    for params in [
        // The module's own refusal: terminations longer than the body.
        json!({"generator": "chip", "scope": "component", "params": {"length": 600_000, "termination": 400_000}}),
        // A parameter the module does not have.
        json!({"generator": "chip", "scope": "component", "params": {"colour": "red"}}),
        // No such generator.
        json!({"generator": "nonesuch", "scope": "component"}),
        // An unknown preset.
        json!({"generator": "chip", "scope": "package", "preset": "9999"}),
    ] {
        let reply = call(&mut session, "library.generate_part", params);
        assert_eq!(reply["ok"], false, "{reply}");
    }
    let bad = call(
        &mut session,
        "library.generate_part",
        json!({"generator": "chip", "scope": "component", "params": {"length": 600_000, "termination": 400_000}}),
    );
    assert!(bad["error"]["rule"] == "param.inconsistent", "{bad}");
    // Inline bytes that do not hash to the hash named with them.
    let chip = std::fs::read(CHIP)?;
    let forged = call(
        &mut session,
        "project.apply",
        json!({"operation": {"editor": "Library", "operation": {"command": "library.generate", "params": {
            "module": {"hash": "00".repeat(32), "data": base64(&chip)},
            "scope": "component",
        }}}}),
    );
    assert_eq!(forged["ok"], false, "{forged}");
    assert_eq!(session.engine().revision(), revision);
    assert_eq!(session.engine().to_file(), before);
    Ok(())
}

/// Rewrite the container at `path`, passing each entry's name and bytes
/// through `edit`.
fn rewrite(path: &Path, edit: impl Fn(&str, Vec<u8>) -> TestResult<Vec<u8>>) -> TestResult {
    let mut entries = Vec::new();
    {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let name = entry.name()?.into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            entries.push((name.clone(), edit(&name, bytes)?));
        }
    }
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path)?);
    for (name, bytes) in entries {
        writer.start_file(name, zip::write::SimpleFileOptions::default())?;
        writer.write_all(&bytes)?;
    }
    writer.finish()?;
    Ok(())
}

/// A session holding another project, its file and revision, to show a
/// refused open leaves it as it was.
fn open_refused(root: &Path, path: &Path) -> TestResult<String> {
    let mut other = session(root)?;
    let made = call(
        &mut other,
        "library.generate_part",
        json!({"generator": "chip", "scope": "package", "params": {"part": "inductor"}}),
    );
    assert_eq!(made["ok"], true, "{made}");
    let before = other.engine().to_file();
    let revision = other.engine().revision();
    let opened = call(
        &mut other,
        "project.open",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(opened["ok"], false, "{opened}");
    assert_eq!(
        other.engine().to_file(),
        before,
        "the open document is unchanged"
    );
    assert_eq!(other.engine().revision(), revision);
    Ok(format!(
        "{}: {}",
        opened["error"]["rule"].as_str().unwrap_or(""),
        opened["error"]["message"].as_str().unwrap_or("")
    ))
}

/// The two 64-digit hex hashes a message names.
fn hashes(message: &str) -> Vec<String> {
    message
        .split("sha256 ")
        .skip(1)
        .map(|rest| rest.chars().take_while(char::is_ascii_hexdigit).collect())
        .filter(|h: &String| h.len() == 64)
        .collect()
}

#[test]
fn a_changed_embedded_module_is_refused_naming_both_hashes() -> TestResult {
    let temp = tempfile::tempdir()?;
    let (made, path) = generated_project(temp.path())?;
    let module = made
        .engine()
        .project()
        .library
        .generated
        .first()
        .map(|g| g.from.module)
        .ok_or("no generated part")?;
    let entry = format!("assets/sha256/{module}");
    rewrite(&path, |name, mut bytes| {
        if name == entry
            && let Some(last) = bytes.last_mut()
        {
            *last ^= 0x01;
        }
        Ok(bytes)
    })?;
    let changed = sparksmith_core::library::bundle::asset_hash(&{
        let mut chip = std::fs::read(CHIP)?;
        if let Some(last) = chip.last_mut() {
            *last ^= 0x01;
        }
        chip
    });
    let message = open_refused(&temp.path().join("library"), &path)?;
    assert!(message.starts_with("generated.module_changed"), "{message}");
    assert!(message.contains("generator module"), "{message}");
    assert_eq!(
        hashes(&message),
        vec![module.to_string(), changed.to_string()],
        "{message}"
    );
    Ok(())
}

#[test]
fn a_changed_stored_part_is_refused_naming_both_hashes() -> TestResult {
    let temp = tempfile::tempdir()?;
    let (made, path) = generated_project(temp.path())?;
    let footprint = made
        .engine()
        .project()
        .library
        .generated
        .first()
        .and_then(|g| g.expansion.get(3).copied())
        .ok_or("no generated footprint")?;
    rewrite(&path, |name, bytes| {
        if name != "project.json" {
            return Ok(bytes);
        }
        let mut document: Value = serde_json::from_slice(&bytes)?;
        let footprints = document["project"]["library"]["footprints"]
            .as_array_mut()
            .ok_or("no footprints")?;
        let stored = footprints
            .iter_mut()
            .find(|f| f["id"] == json!(footprint.id) && f["revision"] == json!(footprint.revision))
            .ok_or("generated footprint not stored")?;
        // Move pad 1 by 10 µm: still a valid footprint, no longer the
        // generator's.
        let x = &mut stored["definition"]["pads"][0]["position"]["x_nm"];
        *x = json!(x.as_i64().ok_or("no pad position")? - 10_000);
        Ok(serde_json::to_vec(&document)?)
    })?;
    let message = open_refused(&temp.path().join("library"), &path)?;
    assert!(
        message.starts_with("generated.expansion_changed"),
        "{message}"
    );
    let named = hashes(&message);
    assert_eq!(named.len(), 2, "{message}");
    assert_ne!(named[0], named[1]);
    Ok(())
}

#[test]
fn a_part_made_again_previews_moving_its_placed_instances() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut session = session(&root)?;
    let made = call(
        &mut session,
        "library.generate_part",
        json!({"generator": "chip", "scope": "component", "preset": "resistor-0603"}),
    );
    assert_eq!(made["ok"], true, "{made}");
    let revisions = made["result"]["outcome"]["report"]["revisions"].clone();
    let component = revisions[0].clone();
    let placed = call(
        &mut session,
        "project.apply",
        json!({"operation": {"editor": "Instance", "operation": {"command": "component.place",
            "params": {"component": {"id": component["id"], "revision": component["revision"]},
                "designator": "R1", "properties": [], "representation": revisions[1]["id"],
                "unit": 1, "position": [60.0, 0.0],
                "physical": {"binding": revisions[4]["id"], "footprint": revisions[3]["id"]}}}}}),
    );
    assert_eq!(placed["ok"], true, "{placed}");
    let instance = placed["result"]["outcome"]["instance"].clone();
    // Made again as the next revision (here with another length, named
    // explicitly; a newer generator version does the same by itself).
    let again = call(
        &mut session,
        "library.generate_part",
        json!({"generator": "chip", "scope": "component", "preset": "resistor-0603",
            "params": {"length": 1_700_000}, "replaces": component}),
    );
    assert_eq!(again["ok"], true, "{again}");
    let report = &again["result"]["outcome"]["report"];
    assert_eq!(report["replaced"], component, "{report}");
    assert_eq!(report["root"]["id"], component["id"]);
    assert_eq!(report["root"]["revision"], 2);
    // The instance still pins revision 1; the reply previews the move.
    let pinned = session.engine().project().instances[0].component.revision;
    assert_eq!(pinned.0, 1);
    let update = &again["result"]["update"];
    assert_eq!(update["revision"], session.engine().revision(), "{update}");
    assert_eq!(update["change"]["params"]["instances"], json!([instance]));
    assert_eq!(update["blockers"], json!([]), "{update}");
    let applied = call(
        &mut session,
        "project.apply_update",
        json!({"change": update["change"], "remove_conflicting_tracks": false}),
    );
    assert_eq!(applied["ok"], true, "{applied}");
    let moved = session.engine().project().instances[0].component.revision;
    assert_eq!(moved.0, 2);
    Ok(())
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// E2E run 12: an agent's `library.search` reads its words as a generic
/// part too, as the Place palette does: first the operation that makes it,
/// then, once made, the component and the value to place it with.
#[test]
fn a_search_lists_the_generic_part_its_words_name_to_make_then_to_place() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut session = session(&root)?;
    let search = |session: &mut Session| -> TestResult<Value> {
        let found = session.dispatch(json!({"command": "project.query", "params":
            {"query": "library.search", "params": {"query": "resistor 0603 10k"}}}));
        assert_eq!(found["ok"], true, "{found}");
        Ok(found["result"]["generic"][0].clone())
    };
    let hint = search(&mut session)?;
    assert_eq!(hint["make"]["op"], "library.generate_part", "{hint}");
    assert_eq!(hint["standing"], "Not yet in this project", "{hint}");
    assert_eq!(
        hint["values"],
        json!([{"field": "Resistance", "text": "10 kΩ"}]),
        "{hint}"
    );
    let mut params = hint["make"]["params"].clone();
    params["expected_revision"] = json!(session.engine().revision());
    let made = session.dispatch(json!({"command": "library.generate_part", "params": params}));
    assert_eq!(made["ok"], true, "{made}");
    let hint = search(&mut session)?;
    assert!(hint["component"]["id"].is_number(), "{hint}");
    assert!(hint.get("make").is_none(), "{hint}");
    assert!(hint["physical"]["footprint"].is_number(), "{hint}");
    // The resistor family's part selects Resistance: the value goes on
    // the placed instance.
    let properties = hint["properties"].as_array().cloned().unwrap_or_default();
    assert_eq!(properties.len(), 1, "{hint}");
    assert_eq!(
        properties[0]["input"],
        json!({"input": "text", "value": "10 kΩ"}),
        "{hint}"
    );
    assert!(hint.get("note").is_none(), "{hint}");
    // Words that name no generic part add nothing.
    let found = session.dispatch(json!({"command": "project.query", "params":
        {"query": "library.search", "params": {"query": "TLV75733PDBVR"}}}));
    assert!(found["result"].get("generic").is_none(), "{found}");
    Ok(())
}

/// A generic resistor made from the shipped module selects its family's
/// fields, so each placed part holds its own value: R1 and R2 at 4.7 kΩ and
/// R3 at 10 kΩ are two BOM lines and show their values on the sheet.
/// `instance.set_properties` changes R3 to 4.7 kΩ as one undo step (one
/// BOM line), and undo takes it back. The project reopens as saved.
#[test]
fn a_generic_resistors_value_is_set_shown_and_grouped_in_the_bom() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut session = session(&root)?;
    let search = |session: &mut Session, words: &str| -> TestResult<Value> {
        let found = session.dispatch(json!({"command": "project.query", "params":
            {"query": "library.search", "params": {"query": words}}}));
        assert_eq!(found["ok"], true, "{found}");
        Ok(found["result"]["generic"][0].clone())
    };
    let hint = search(&mut session, "resistor 0603 4.7k")?;
    let mut params = hint["make"]["params"].clone();
    params["expected_revision"] = json!(session.engine().revision());
    let made = session.dispatch(json!({"command": "library.generate_part", "params": params}));
    assert_eq!(made["ok"], true, "{made}");
    let revisions = made["result"]["outcome"]["report"]["revisions"].clone();
    let place = |session: &mut Session, words: &str, designator: &str| -> TestResult<u64> {
        let hint = search(session, words)?;
        let placed = call(
            session,
            "project.apply",
            json!({"operation": {"editor": "Instance", "operation": {"command": "component.place",
                "params": {"component": hint["component"], "designator": designator,
                    "properties": hint["properties"], "representation": revisions[1]["id"],
                    "unit": 1, "position": [60.0, 0.0], "physical": hint["physical"]}}}}),
        );
        assert_eq!(placed["ok"], true, "{placed}");
        placed["result"]["outcome"]["instance"]
            .as_u64()
            .ok_or_else(|| format!("no instance in {placed}").into())
    };
    place(&mut session, "resistor 0603 4.7k", "R1")?;
    place(&mut session, "4k7 0603", "R2")?;
    let r3 = place(&mut session, "10k 0603", "R3")?;
    let lines = |session: &Session| -> TestResult<Vec<(Vec<String>, Vec<String>)>> {
        let bom = sparksmith_core::completeness::bom(session.engine().project())?;
        Ok(bom
            .lines
            .iter()
            .map(|l| {
                (
                    l.designators.clone(),
                    l.values.iter().map(|v| v.text.clone()).collect(),
                )
            })
            .collect())
    };
    let two = vec![
        (
            vec!["R1".to_owned(), "R2".to_owned()],
            vec!["4.7 kΩ".to_owned()],
        ),
        (vec!["R3".to_owned()], vec!["10 kΩ".to_owned()]),
    ];
    assert_eq!(lines(&session)?, two);
    let csv = sparksmith_core::fabrication::assembly::bom_csv(session.engine().project())?;
    assert!(csv.contains("\"R1 R2\"") || csv.contains("R1 R2"), "{csv}");
    assert!(csv.contains("4.7 kΩ") && csv.contains("10 kΩ"), "{csv}");
    // The sheet shows each value under the designator.
    let project = session.engine().project();
    let r1 = project
        .instances
        .iter()
        .find(|i| i.designator == "R1")
        .ok_or("no R1")?;
    let occurrence = project
        .occurrences
        .iter()
        .find(|o| o.instance == r1.id)
        .ok_or("R1 is not on the sheet")?;
    let drawn = sparksmith_core::schematic::placed_unit(project, occurrence.id)?;
    let texts: Vec<&str> = drawn
        .texts
        .iter()
        .chain(&drawn.fields)
        .map(|t| t.text.as_str())
        .collect();
    assert!(
        texts.contains(&"R1") && texts.contains(&"4.7 kΩ"),
        "{texts:?}"
    );
    // One edit, one undo step.
    let field = search(&mut session, "10k 0603")?["properties"][0]["field"].clone();
    let set = call(
        &mut session,
        "project.apply",
        json!({"operation": {"editor": "Instance", "operation": {"command": "instance.set_properties",
            "params": {"instance": r3, "edits": [{"field": field,
                "input": {"input": "text", "value": "4.7k"}}]}}}}),
    );
    assert_eq!(set["ok"], true, "{set}");
    assert_eq!(
        lines(&session)?,
        vec![(
            vec!["R1".to_owned(), "R2".to_owned(), "R3".to_owned()],
            vec!["4.7 kΩ".to_owned()]
        )]
    );
    let undone = call(&mut session, "project.undo", json!({}));
    assert_eq!(undone["ok"], true, "{undone}");
    assert_eq!(lines(&session)?, two);
    // Saved and reopened: the stored fields pass the generator check.
    let path = temp.path().join("values.sparksmith");
    let saved = call(
        &mut session,
        "project.save",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(saved["ok"], true, "{saved}");
    let mut reopened = self::session(&root)?;
    let opened = call(
        &mut reopened,
        "project.open",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(lines(&reopened)?, two);
    Ok(())
}

/// F2: a generic family's part is named by the family's label and found
/// as the family reads: by a value each placed part sets ("4.7k 0603"), by
/// `gen:` (generator, family or preset), in the project and, published to
/// a library folder with its recipe, in the folder too.
#[test]
fn a_generated_generic_part_is_found_by_value_and_generator() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut session = session(&root)?;
    let made = call(
        &mut session,
        "library.generate_part",
        json!({"generator": "chip", "scope": "component", "preset": "resistor-0603"}),
    );
    assert_eq!(made["ok"], true, "{made}");
    let component = made["result"]["outcome"]["report"]["root"].clone();
    let names = |session: &mut Session, text: &str| -> TestResult<Vec<String>> {
        let found = session.dispatch(json!({"command": "project.query", "params": {
            "query": "library.search", "params": {"query": text, "limit": 50}}}));
        assert_eq!(found["ok"], true, "{text}: {found}");
        Ok(found["result"]["result"]["results"]
            .as_array()
            .ok_or("results")?
            .iter()
            .filter_map(|r| r["entry"]["name"].as_str().map(str::to_owned))
            .collect())
    };
    for text in ["4.7k 0603", "10k", "gen:resistor", "gen:chip", "src:project gen:chip resistor"] {
        assert!(
            names(&mut session, text)?.contains(&"Chip resistor 0603".to_owned()),
            "{text} finds the part by its family label"
        );
    }
    assert!(names(&mut session, "gen:header")?.is_empty());

    // Published to a folder: the record carries the recipe, and the
    // folder's entry is found the same way.
    let folder = temp.path().join("generic-library");
    let created = session.dispatch(json!({"command": "library.source.create", "params": {
        "path": folder.display().to_string(), "name": "Generic"}}));
    assert_eq!(created["ok"], true, "{created}");
    let library = created["result"]["created"]["library"].clone();
    let published = call(
        &mut session,
        "library.publish_to_folder",
        json!({"library": library, "root": component}),
    );
    assert_eq!(published["ok"], true, "{published}");
    let mut recipes = 0;
    for entry in walk(&folder.join("lineages"))? {
        let record: Value = serde_json::from_str(&std::fs::read_to_string(&entry)?)?;
        if record["kind"] == "component" {
            assert_eq!(record["generated"]["generator"], "chip", "{record}");
            assert_eq!(record["generated"]["family"], "resistor", "{record}");
            assert!(record["generated"]["preset"].is_string(), "{record}");
            recipes += 1;
        }
    }
    assert_eq!(recipes, 1);
    for text in ["src:library 4.7k 0603", "src:library gen:resistor"] {
        assert!(
            names(&mut session, text)?.contains(&"Chip resistor 0603".to_owned()),
            "{text} finds the folder's part"
        );
    }
    Ok(())
}

/// Every file below `dir`.
fn walk(dir: &Path) -> TestResult<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            out.extend(walk(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

/// A second part from the same parameters under another name is its own
/// component on the shared package, its symbol and binding named after it;
/// the same name again finds the existing part and changes nothing.
#[test]
fn a_second_name_on_the_same_package_makes_its_own_component() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut session = session(&root)?;
    let params = |name: &str| {
        json!({"generator": "chip", "scope": "component", "preset": "resistor-0603", "name": name})
    };
    let first = call(&mut session, "library.generate_part", params("ExampleCo EX-R1"));
    assert_eq!(first["ok"], true, "{first}");
    let first_root = first["result"]["outcome"]["report"]["root"].clone();
    let revision = session.engine().revision();
    let again = call(&mut session, "library.generate_part", params("ExampleCo EX-R1"));
    assert_eq!(again["result"]["outcome"]["report"]["created"], false, "{again}");
    assert_eq!(session.engine().revision(), revision, "nothing changed");
    let second = call(&mut session, "library.generate_part", params("ExampleCo EX-R2"));
    assert_eq!(second["ok"], true, "{second}");
    let report = &second["result"]["outcome"]["report"];
    assert_eq!(report["created"], true, "{report}");
    assert_ne!(report["root"]["id"], first_root["id"], "{report}");
    assert_eq!(session.engine().revision(), revision + 1, "one undo step");
    let library = &session.engine().project().library;
    let names: Vec<String> = library
        .components
        .iter()
        .map(|c| c.meta.name.clone())
        .chain(library.symbols.iter().map(|s| s.meta.name.clone()))
        .chain(library.bindings.iter().map(|b| b.meta.name.clone()))
        .collect();
    for name in ["ExampleCo EX-R2", "ExampleCo EX-R2 symbol", "ExampleCo EX-R2 binding"] {
        assert!(names.iter().any(|n| n == name), "{name} in {names:?}");
    }
    let packages = |c: &str| {
        library
            .components
            .iter()
            .find(|x| x.meta.name == c)
            .and_then(|x| x.definition.implementations.first().copied())
            .and_then(|b| library.binding(b))
            .map(|b| b.definition.package)
    };
    assert_eq!(packages("ExampleCo EX-R1"), packages("ExampleCo EX-R2"));
    assert!(packages("ExampleCo EX-R1").is_some());
    Ok(())
}

/// The generators' marking rules end to end: generic parts made from the
/// shipped modules through the catalog's words, placed with their values,
/// mark what the rules say (`docs/generator-plugins.md`, "Markings as
/// generator data"). Every expected code is derived by hand from IEC 60062
/// and the EIA-96 table.
#[test]
fn generated_parts_mark_their_values_by_rule() -> TestResult {
    use sparksmith_core::library::marking::LaidElement;
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let mut session = session(&root)?;
    let search = |session: &mut Session, words: &str| -> TestResult<Value> {
        let found = session.dispatch(json!({"command": "project.query", "params":
            {"query": "library.search", "params": {"query": words}}}));
        assert_eq!(found["ok"], true, "{found}");
        Ok(found["result"]["generic"][0].clone())
    };
    // (words, designator, tolerance, texts of the marking and its decals,
    // marks drawn); `None`: no marking at all.
    type Case<'a> = (
        &'a str,
        &'a str,
        Option<&'a str>,
        Option<Vec<&'a str>>,
        usize,
    );
    let cases: [Case<'_>; 11] = [
        ("resistor 0805 10k", "R1", Some("1%"), Some(vec!["1002"]), 0),
        ("resistor 0805 10k", "R2", Some("5%"), Some(vec!["103"]), 0),
        ("resistor 0805 10k", "R3", None, Some(vec!["103"]), 0),
        ("resistor 1206 4.7", "R4", Some("5%"), Some(vec!["4R7"]), 0),
        (
            "resistor 1206 49.9",
            "R5",
            Some("1%"),
            Some(vec!["49R9"]),
            0,
        ),
        ("resistor 0603 10k", "R6", Some("1%"), Some(vec!["01C"]), 0),
        ("resistor 0603 4.7k", "R7", Some("1%"), Some(vec!["472"]), 0),
        ("resistor 0402 10k", "R8", Some("1%"), None, 0),
        ("led 0603 red", "D1", None, Some(vec![]), 1),
        (
            "electrolytic 6.3x7.7 100u 16V",
            "C1",
            None,
            Some(vec!["100", "16V"]),
            0,
        ),
        (
            "radial electrolytic 8x11.5 470u 25V",
            "C2",
            None,
            Some(vec!["470", "25V"]),
            1,
        ),
    ];
    for (words, designator, tolerance, texts, marks) in cases {
        let hint = search(&mut session, words)?;
        let component = if hint.get("make").is_some() {
            let mut params = hint["make"]["params"].clone();
            params["expected_revision"] = json!(session.engine().revision());
            let made =
                session.dispatch(json!({"command": "library.generate_part", "params": params}));
            assert_eq!(made["ok"], true, "{words}: {made}");
            search(&mut session, words)?
        } else {
            hint
        };
        let representation = session
            .engine()
            .project()
            .library
            .component(serde_json::from_value(component["component"].clone())?)
            .and_then(|c| c.definition.representations.first().map(|r| r.id))
            .ok_or("no symbol")?;
        let placed = call(
            &mut session,
            "project.apply",
            json!({"operation": {"editor": "Instance", "operation": {"command": "component.place",
                "params": {"component": component["component"], "designator": designator,
                    "properties": component["properties"], "representation": representation,
                    "unit": 1, "position": [0.0, 0.0], "physical": component["physical"]}}}}),
        );
        assert_eq!(placed["ok"], true, "{words}: {placed}");
        let instance = placed["result"]["outcome"]["instance"]
            .as_u64()
            .ok_or("no instance")?;
        if let Some(tolerance) = tolerance {
            let field = sparksmith_core::library::standards::get()?
                .fields
                .local(&session.engine().project().library, "Tolerance")
                .ok_or("no Tolerance field")?;
            let set = call(
                &mut session,
                "project.apply",
                json!({"operation": {"editor": "Instance", "operation": {"command": "instance.set_properties",
                    "params": {"instance": instance, "edits": [{"field": field.id,
                        "input": {"input": "text", "value": tolerance}}]}}}}),
            );
            assert_eq!(set["ok"], true, "{words}: {set}");
        }
        let layout =
            sparksmith_core::commands::instance_marking(session.engine().project(), instance)?;
        let Some(texts) = texts else {
            assert!(
                layout
                    .as_ref()
                    .is_none_or(|l| l.all().all(|l| l.elements.is_empty())),
                "{words}: {layout:?}"
            );
            continue;
        };
        let layout = layout.ok_or_else(|| format!("{words}: not marked"))?;
        let drawn: Vec<&str> = layout
            .all()
            .flat_map(|l| &l.elements)
            .filter_map(|e| match e {
                LaidElement::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(drawn, texts, "{words}: {:?}", layout.findings);
        let drawn_marks = layout
            .all()
            .flat_map(|l| &l.elements)
            .filter(|e| {
                matches!(
                    e,
                    LaidElement::Polarity { .. } | LaidElement::Artwork { .. }
                )
            })
            .count();
        assert_eq!(drawn_marks, marks, "{words}: {layout:?}");
    }
    Ok(())
}

/// The shipped module generator's row lists are grouped as rows, as its
/// source says; the wasm32 build once cut the group to "sing".
#[test]
fn the_shipped_module_groups_its_row_lists_as_rows() -> TestResult {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let mut session = session(&root)?;
    let listed = call(
        &mut session,
        "library.generators",
        json!({"generator": "module"}),
    );
    let params = listed["result"]["shipped"][0]["description"]["params"]
        .as_array()
        .ok_or("params")?;
    let rows: Vec<&Value> = params
        .iter()
        .filter(|p| p["key"].as_str().is_some_and(|k| k.starts_with("row_")))
        .collect();
    assert!(!rows.is_empty(), "{listed}");
    assert!(rows.iter().all(|p| p["group"] == "rows"), "{rows:?}");
    Ok(())
}

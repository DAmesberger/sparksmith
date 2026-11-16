//! The datasheet-to-component workflow as an external agent runs it: the
//! `datasheet_to_component` prompt, then only MCP verb calls, scripted here
//! in place of a model (no network, no model). The agent's steps end at
//! `library.review_check`; publishing stays behind the import review gate
//! until the person has confirmed the rows.
use super::prompts;
use super::scenario::{EXM48, Fallible};
use super::testing::Tools;
use crate::session::Session;
use crate::workflow::{HostCatalog, Loaded, METHOD_FILE, load};
use crate::{Grant, Mode};
use serde_json::{Value, json};

/// The prompt of the shipped datasheet workflow.
const DATASHEET_PART: &str = "datasheet_to_component";

/// The shipped library's workflows that can run here.
pub(super) fn shipped() -> Vec<Loaded> {
    let folder =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/assist/workflows");
    load::available(&load::list(Some(&folder), None, &HostCatalog))
}

/// The shipped datasheet workflow's method, as on disk.
fn method() -> Fallible<String> {
    Ok(std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../library/assist/workflows/datasheet-to-component")
            .join(METHOD_FILE),
    )?)
}

/// A call that must succeed, named by its step.
fn ok(step: &str, (failed, payload): (bool, Value)) -> Fallible<Value> {
    if failed {
        return Err(format!("{step}: {payload}").into());
    }
    Ok(payload)
}

/// A query's report.
fn query(tools: &Tools, step: &str, name: &str, params: Value) -> Fallible<Value> {
    Ok(ok(step, tools.query(name, params))?["result"].clone())
}

/// One library operation through `apply`: its outcome.
fn apply(tools: &Tools, step: &str, op: &str, params: Value) -> Fallible<Value> {
    let reply = ok(step, tools.apply(op, params))?;
    Ok(reply["outcomes"][0].clone())
}

/// What a JSON client that reads numbers as doubles (JavaScript, Claude
/// Code's transport) sends back of `value`: every integer above 2^53
/// loses its last digits.
fn as_doubles(value: &Value) -> Value {
    match value {
        Value::Number(n) => match n.as_u64() {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            Some(u) => json!(u as f64 as u64),
            None => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(as_doubles).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), as_doubles(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Preview `document` onto `target` and apply exactly that plan, its base
/// copied as such a client copies it.
fn import(tools: &Tools, step: &str, target: Value, document: &Value) -> Fallible<Value> {
    let plan = query(
        tools,
        step,
        "library.pinout_preview",
        json!({"target": target, "document": document}),
    )?;
    apply(
        tools,
        step,
        "library.pinout_apply",
        as_doubles(
            &json!({"target": target, "document": document, "base": plan["base"],
            "accept": {"accept": "all"}}),
        ),
    )
}

/// The report an agent writes from the EXM-48 fixture's datasheet.
fn report(open_questions: Value) -> Value {
    json!({"report": {
        "part": "EXM-48",
        "summary": "48-pin QFN MCU: pins, functions and rules from Table 4; package and footprint from the no-lead generator's QFN-48 7x7 0.5 EP 5.3 preset.",
        "sources": [{"name": "DS", "title": "EXM-48 datasheet", "revision": "rev 3"}],
        "values": [
            {"item": "pin count", "value": "48 + EP", "source": {"document": "DS", "page": 11, "location": "Table 4"}, "confidence": "high"},
            {"item": "pitch", "value": "0.5 mm", "source": {"document": "DS", "page": 40, "location": "Figure 12"}, "confidence": "high"},
            {"item": "exposed pad", "value": "5.3 mm", "source": {"document": "DS", "page": 40, "location": "Figure 12"}, "confidence": "medium", "note": "read from an ambiguous dimension line"},
        ],
        "open_questions": open_questions,
    }})
}

fn draft_of(tools: &Tools, id: u64) -> Fallible<Value> {
    let snapshot = ok(
        "inspect",
        tools.query("project.inspect", json!({"verbose": true})),
    )?;
    snapshot["drafts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["draft"]["id"] == id)
        .cloned()
        .ok_or_else(|| format!("draft {id}").into())
}

#[test]
fn the_prompt_is_listed_and_returns_the_workflow() -> Fallible<()> {
    let workflows = shipped();
    let listed = serde_json::to_value(prompts::list(&workflows))?;
    let prompt = listed
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == DATASHEET_PART)
        .ok_or_else(|| format!("{DATASHEET_PART} listed: {listed}"))?;
    let required: Vec<&str> = prompt["arguments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["required"] == true)
        .filter_map(|a| a["name"].as_str())
        .collect();
    // Generated from the workflow's inputs, in the order its input step
    // asks for them, titled and described by its data.
    assert_eq!(required, ["datasheets", "part"]);
    let all: Vec<&str> = prompt["arguments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["name"].as_str())
        .collect();
    assert_eq!(all, ["datasheets", "part", "pages", "notes"]);
    let workflow = &workflows.first().ok_or("a workflow")?.workflow;
    assert_eq!(prompt["title"], workflow.title.as_str());
    assert_eq!(prompt["description"], workflow.summary.as_str());
    assert!(
        prompt["arguments"][0]["description"]
            .as_str()
            .is_some_and(|d| d.starts_with("Datasheets. File paths or URLs (application/pdf)")),
        "{prompt}"
    );

    let arguments = |value: Value| match value {
        Value::Object(map) => Some(map),
        _ => None,
    };
    let got = serde_json::to_value(prompts::get(
        &workflows,
        DATASHEET_PART,
        arguments(json!({"part": "EXM-48",
            "datasheets": "exm48.pdf\nhttps://example.com/exm48-errata.pdf", "notes": "QFN-48 only"}))
        .as_ref(),
    )?)?;
    let message = &got["messages"][0];
    assert_eq!(message["role"], "user", "{got}");
    let text = message["content"]["text"].as_str().unwrap_or_default();
    // The method first, word for word, then the task.
    let (workflow, task) = text.split_once("## Task\n").ok_or("a task section")?;
    assert_eq!(workflow.trim_end(), method()?.trim_end());
    assert!(task.contains("Part name: EXM-48") && task.contains("- exm48.pdf"));
    assert_eq!(got["description"], "Datasheet → component: EXM-48");
    assert!(
        task.contains("- https://example.com/exm48-errata.pdf") && task.contains("QFN-48 only")
    );
    for step in [
        "No guessing",
        "library.pinout_schema",
        "library.pinout_preview",
        "library.pinout_apply",
        "library.generators",
        "library.generate_part",
        "library.draft_closure",
        "component.check_drafts",
        "library.review_check",
        "Verification checklist",
    ] {
        assert!(workflow.contains(step), "{step}");
    }

    for (name, bad) in [
        ("other", json!({"part": "X", "datasheets": "a.pdf"})),
        (DATASHEET_PART, json!({"part": "X"})),
        (DATASHEET_PART, json!({"part": 3, "datasheets": "a.pdf"})),
        (
            DATASHEET_PART,
            json!({"part": "X", "datasheets": "a.pdf", "colour": "red"}),
        ),
        (
            DATASHEET_PART,
            json!({"part": "X".repeat(65), "datasheets": "a.pdf"}),
        ),
    ] {
        assert!(
            prompts::get(&workflows, name, arguments(bad.clone()).as_ref()).is_err(),
            "{name} {bad}"
        );
    }
    Ok(())
}

/// One file is the source of both: the prompt serves the workflow's
/// method.md, read when the server starts, and the skill points there.
#[test]
fn the_method_file_and_the_prompt_are_one_source() -> Fallible<()> {
    let on_disk = method()?;
    assert!(on_disk.starts_with("# Create a component from datasheets"));
    let skill = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../skills/sparksmith-datasheet-part/SKILL.md"
    ))?;
    assert!(skill.starts_with("---\nname: sparksmith-datasheet-part\ndescription: "));
    assert!(skill.contains("library/assist/workflows/datasheet-to-component/method.md"));
    let mut arguments = rmcp::model::JsonObject::new();
    arguments.insert("part".into(), json!("P"));
    arguments.insert("datasheets".into(), json!("a.pdf"));
    let got = serde_json::to_value(prompts::get(&shipped(), DATASHEET_PART, Some(&arguments))?)?;
    let text = got["messages"][0]["content"]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(text.starts_with(on_disk.trim_end()));
    Ok(())
}

/// The whole workflow over MCP with no model: schema, a generator found and
/// its part made, the working set drafted, the pinout previewed and applied
/// onto it, checks, the review check refusing an incomplete report and
/// accepting the fixed one, and publishing refused by the review gate until
/// the person has confirmed the rows.
#[test]
fn a_scripted_session_completes_the_workflow_through_the_review_gate() -> Fallible<()> {
    // Publishing is Ask-class: allow-all stands in for the person's Allow,
    // so the test reaches the review gate that must still refuse it.
    let mut session = Session::new(sparksmith_core::Engine::profiled()?);
    session.set_shipped(sparksmith_io::shipped::ShippedLibrary::at(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")),
    )?);
    let tools = Tools::new(
        session,
        Grant::new(Mode::AllowAll, Vec::<std::path::PathBuf>::new()),
    )?;
    let schema = ok("schema", tools.query("library.pinout_schema", json!({})))?;
    assert_eq!(schema["format"], 1, "{schema}");

    // Step 3: the generator whose families cover a QFN, and its preset
    // for the drawing's package, from the compact listing; then that
    // generator's parameters by its name.
    let generators = ok(
        "generators",
        tools.query("library.generators", json!({"family": "no_lead"})),
    )?;
    let no_lead = generators["shipped"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|g| {
            g["families"]
                .as_array()
                .is_some_and(|f| f.iter().any(|f| f == "no_lead"))
        })
        .ok_or_else(|| format!("a no-lead generator: {generators}"))?;
    let preset = no_lead["presets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| {
            p["name"]
                .as_str()
                .is_some_and(|n| n.contains("7×7 0.5 EP 5.3"))
        })
        .ok_or_else(|| format!("the QFN-48 preset: {no_lead}"))?;
    let described = tools.ok(super::tools::DESCRIBE, json!({"name": no_lead["name"]}))?;
    assert!(
        described["generator"]["description"]["params"]
            .as_array()
            .is_some_and(|p| !p.is_empty()),
        "{described}"
    );
    assert_eq!(
        described["call"]["arguments"]["operations"][0]["op"], "library.generate_part",
        "{described}"
    );
    // Step 4: make the part, draft its working set.
    let made = ok(
        "generate",
        tools.apply(
            "library.generate_part",
            json!({"generator": no_lead["name"], "scope": "component",
                "preset": preset["key"], "name": "EXM-48"}),
        ),
    )?;
    let root = &made["outcome"]["report"]["root"];
    assert_eq!(root["kind"], "component", "{made}");
    let drafted = apply(
        &tools,
        "draft closure",
        "library.draft_closure",
        json!({"component": {"id": root["id"], "revision": root["revision"]}}),
    )?;
    let drafts: Vec<u64> = drafted["ids"]
        .as_array()
        .ok_or_else(|| format!("the drafted set: {drafted}"))?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    let of_kind = |kind: &str| {
        drafts
            .iter()
            .copied()
            .find(|&id| {
                draft_of(&tools, id).is_ok_and(|d| d["draft"]["definition"]["kind"] == kind)
            })
            .ok_or_else(|| format!("the {kind} draft"))
    };
    let (component, symbol, package, footprint, binding) = (
        of_kind("component")?,
        of_kind("symbol")?,
        of_kind("package")?,
        of_kind("footprint")?,
        of_kind("binding")?,
    );
    // The review: 48 terminals and the exposed pad, numbered EP.
    let pads = draft_of(&tools, footprint)?["draft"]["definition"]["body"]["pads"].clone();
    let numbers: Vec<&str> = pads
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["number"].as_str())
        .collect();
    assert_eq!(numbers.len(), 49, "{pads}");
    assert_eq!(numbers.first(), Some(&"1"));
    assert!(numbers.contains(&"EP"), "{numbers:?}");

    // Step 5. The datasheet calls the exposed pad 49; the generator EP: the
    // row takes the generator's number, and its note says so. The agent
    // forgot one row's source.
    let mut complete: Value = serde_json::from_str(EXM48)?;
    let ep = complete["pins"]
        .as_array_mut()
        .and_then(|pins| pins.iter_mut().find(|p| p["row"] == "r49"))
        .ok_or("row r49")?;
    ep["number"]["QFN-48"] = json!("EP");
    ep["note"] = json!(
        "Exposed pad, pin 49 in the datasheet (EP in the footprint); internally connected to VSS."
    );
    let mut document = complete.clone();
    let r2 = document["pins"]
        .as_array_mut()
        .and_then(|pins| pins.iter_mut().find(|p| p["row"] == "r2"))
        .ok_or("row r2")?;
    let name = r2["name"].clone();
    r2["source"] = Value::Null;
    let target = json!({"target": "draft", "id": component});
    let plan = query(
        &tools,
        "preview",
        "library.pinout_preview",
        json!({"target": target, "document": document}),
    )?;
    // Every row kept a generated terminal by number: the compact plan's
    // counts say so, and it lists no change one by one.
    let counts = plan["change_counts"].as_object().ok_or("change counts")?;
    assert!(
        counts
            .keys()
            .all(|k| !k.starts_with("terminal ") || k == "terminal changed"),
        "{plan}"
    );
    assert!(plan.get("changes").is_none(), "{plan}");
    // The review items by reason, with every row: the uncertain one.
    let uncertain = plan["review_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|g| g["reason"] == "uncertain")
        .ok_or_else(|| format!("the uncertain row: {plan}"))?;
    assert_eq!(uncertain["rows"], json!(["r20"]), "{plan}");
    // The base is text, which a client that reads numbers as doubles keeps.
    assert!(plan["base"].is_string(), "{plan}");
    import(&tools, "import", target.clone(), &document)?;

    // An early review check: no open question for the uncertain row, and
    // the imported pin without a source page, named.
    let document_now = |tools: &Tools| tools.with_session(|s| s.engine().document().clone());
    let before = document_now(&tools);
    let checked = ok(
        "review check",
        tools.query("library.review_check", report(json!([]))),
    )?;
    assert_eq!(document_now(&tools), before, "a read");
    assert_eq!(checked["accepted"], false, "{checked}");
    let problems = checked["problems"].to_string();
    assert!(problems.contains("uncertain"), "{problems}");
    let pin = name.as_str().unwrap_or_default();
    assert!(
        problems.contains("source page") && problems.contains(pin),
        "{problems}"
    );

    // The agent restores the source and asks about the uncertain row.
    import(&tools, "re-import", target, &complete)?;
    query(&tools, "check drafts", "component.check_drafts", json!({}))?;

    let checked = ok(
        "review check again",
        tools.query(
            "library.review_check",
            report(json!([
                "Row r20 (PB4): is AF1 SPI1_MISO? The scan is unreadable on page 14."
            ])),
        ),
    )?;
    assert_eq!(checked["accepted"], true, "{checked}");
    assert_eq!(checked["pins"]["uncertain"], 1, "{checked}");
    assert_eq!(checked["pins"]["unreviewed"], 49, "{checked}");
    let markdown = checked["markdown"].as_str().unwrap_or_default();
    assert!(markdown.contains("| exposed pad | 5.3 mm | DS p.40 Figure 12 | Medium |"));
    assert!(markdown.contains("- [ ] Row r20") && markdown.contains("not published"));
    assert!(!markdown.contains('$'));

    // Publishing is refused while a row is uncertain, and changes nothing.
    let set = json!({"drafts": [component, symbol, package, footprint, binding]});
    let before = tools.revision();
    let (failed, refusal) = tools.act("library.publish", set.clone());
    assert!(failed && refusal["code"] == "conflict", "{refusal}");
    assert!(
        refusal.to_string().contains("flagged uncertain"),
        "{refusal}"
    );
    assert_eq!(tools.revision(), before);

    // The person checks every row in the Pins tab and confirms them (their
    // step, in the window): the review state goes, and they publish.
    let draft = draft_of(&tools, component)?;
    let mut body = draft["draft"]["definition"]["body"].clone();
    body["review"] = Value::Null;
    apply(
        &tools,
        "the person confirms",
        "library.edit",
        json!({"edits": [{"id": component, "meta": draft["draft"]["meta"],
            "definition": {"kind": "component", "body": body}}]}),
    )?;
    let published = ok("the person publishes", tools.act("library.publish", set))?;
    assert!(
        published["outcome"]["revisions"]
            .as_array()
            .is_some_and(|r| r.len() == 5),
        "{published}"
    );
    Ok(())
}

/// The check is a read: a server that asks for every edit still runs it.
#[test]
fn the_review_check_passes_the_strictest_gate() -> Fallible<()> {
    let policy = Grant::new(Mode::AskEveryTime, Vec::<std::path::PathBuf>::new());
    let tools = Tools::new(Session::default(), policy)?;
    let (failed, checked) = tools.query("library.review_check", report(json!([])));
    assert!(!failed, "{checked}");
    assert_eq!(checked["accepted"], false, "no drafts yet: {checked}");
    Ok(())
}

/// The skill's other branch, for a package no generator covers: the
/// pinout imported as new drafts, a footprint draft of the package made
/// and built from the land pattern's pad table, and listed in the binding,
/// all through the verbs.
#[test]
fn without_a_generator_the_part_is_authored_by_commands() -> Fallible<()> {
    let tools = Tools::new(
        Session::new(sparksmith_core::Engine::profiled()?),
        Grant::default(),
    )?;
    let mut document: Value = serde_json::from_str(EXM48)?;
    // Four terminals stand in for a part no family draws.
    document["variants"][0]["terminal_count"] = json!(4);
    if let Some(pins) = document["pins"].as_array_mut() {
        pins.retain(|p| ["r1", "r2", "r3", "r4"].contains(&p["row"].as_str().unwrap_or("")));
    }
    document["rules"] = json!([]);
    document["peripherals"] = json!([]);
    for pin in document["pins"].as_array_mut().into_iter().flatten() {
        let gpio = json!([{"label": "gpio_in", "direction": "input", "signal": "digital"}]);
        pin["functions"] = gpio;
        pin["default"] = json!("gpio_in");
    }
    let applied = import(
        &tools,
        "import",
        json!({"target": "new", "name": "ODD-4"}),
        &document,
    )?;
    let drafts: Vec<u64> = applied["report"]["drafts"]
        .as_array()
        .ok_or_else(|| format!("the import's drafts: {applied}"))?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    let binding = drafts
        .iter()
        .copied()
        .find(|&id| {
            draft_of(&tools, id).is_ok_and(|d| d["draft"]["definition"]["kind"] == "binding")
        })
        .ok_or("the binding draft")?;
    let bound = draft_of(&tools, binding)?;
    let package = bound["draft"]["definition"]["body"]["package"].clone();
    // The package's body from the drawing, then a footprint draft of it.
    let package_draft = draft_of(&tools, package["id"].as_u64().ok_or("package id")?)?;
    let mut sized = package_draft["draft"]["definition"]["body"].clone();
    sized["envelope_nm"] = json!([6_000_000, 4_000_000, 1_000_000]);
    apply(
        &tools,
        "envelope",
        "library.edit",
        json!({"edits": [{"id": package["id"], "meta": package_draft["draft"]["meta"],
            "definition": {"kind": "package", "body": sized}}]}),
    )?;
    let made = apply(
        &tools,
        "new footprint",
        "library.new_draft",
        json!({"meta": {"name": "ODD-4 footprint", "description": "", "provenance": ""},
            "definition": {"kind": "footprint", "body": {
                "package": package, "next_local": 1, "source": null, "pads": [], "graphics": [],
                "polarity": null, "datum": {"offset": {"x_nm": 0, "y_nm": 0}, "rotation_mdeg": 0},
                "defaults": {"mask_expansion_nm": 50000, "paste_reduction_nm": 0}}}}),
    )?;
    let footprint = made["id"]
        .as_u64()
        .or_else(|| made["report"]["id"].as_u64())
        .ok_or_else(|| format!("the footprint draft: {made}"))?;
    // Its pads from the land pattern, landing terminals by number.
    let pads: Vec<Value> = (0..4)
        .map(|k| json!({"number": (k + 1).to_string(), "at": [f64::from(k) * 1.27 - 1.905, 0.0], "size": [0.6, 1.2]}))
        .collect();
    let built = apply(
        &tools,
        "pads",
        "library.footprint_from_pads",
        json!({"draft": footprint, "pads": pads, "assign_by_number": true}),
    )?;
    assert_eq!(built["report"]["unlanded"], json!([]), "{built}");
    // Listed in the binding.
    let mut body = bound["draft"]["definition"]["body"].clone();
    let footprint_ref = json!({"id": footprint, "revision": package["revision"]});
    body["footprints"] = json!([footprint_ref]);
    apply(
        &tools,
        "list the footprint",
        "library.edit",
        json!({"edits": [{"id": binding, "meta": bound["draft"]["meta"],
            "definition": {"kind": "binding", "body": body}}]}),
    )?;
    let checked = query(&tools, "check drafts", "component.check_drafts", json!({}))?;
    let text = checked.to_string();
    assert!(!text.contains("binding.no_footprint"), "{text}");
    Ok(())
}

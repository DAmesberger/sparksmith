//! The datasheet-to-component workflow as an external agent runs it: the
//! `datasheet_to_component` prompt, then only MCP verb calls, scripted here
//! in place of a model (no network, no model). The agent's steps end at
//! `library.review_check`; publishing stays behind the import review gate
//! until the person has confirmed the rows.
use super::prompts::{self, DATASHEET_PART, DATASHEET_PART_SKILL, skill_body};
use super::scenario::{EXM48, Fallible};
use super::testing::Tools;
use crate::session::Session;
use crate::{Grant, Mode};
use serde_json::{Value, json};

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

/// Preview `document` onto `target` and apply exactly that plan.
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
        json!({"target": target, "document": document, "base": plan["base"],
            "accept": {"accept": "all"}}),
    )
}

/// The report an agent writes from the EXM-48 fixture's datasheet.
fn report(open_questions: Value) -> Value {
    json!({"report": {
        "part": "EXM-48",
        "summary": "48-pin QFN MCU: pins, functions and rules from Table 4; footprint from the QFN-48 7x7 0.5 EP 5.3 row.",
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
    let listed = serde_json::to_value(prompts::list())?;
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
    assert_eq!(required, ["part", "datasheets"]);

    let arguments = |value: Value| match value {
        Value::Object(map) => Some(map),
        _ => None,
    };
    let got = serde_json::to_value(prompts::get(
        DATASHEET_PART,
        arguments(json!({"part": "EXM-48",
            "datasheets": "exm48.pdf\nhttps://example.com/exm48-errata.pdf", "notes": "QFN-48 only"}))
        .as_ref(),
    )?)?;
    let message = &got["messages"][0];
    assert_eq!(message["role"], "user", "{got}");
    let text = message["content"]["text"].as_str().unwrap_or_default();
    // The workflow first, word for word, then the task.
    let (workflow, task) = text.split_once("## Task\n").ok_or("a task section")?;
    assert_eq!(
        workflow.trim_end(),
        skill_body(DATASHEET_PART_SKILL).trim_end()
    );
    assert!(task.contains("\"EXM-48\"") && task.contains("- exm48.pdf"));
    assert!(
        task.contains("- https://example.com/exm48-errata.pdf") && task.contains("QFN-48 only")
    );
    for step in [
        "No guessing",
        "library.pinout_schema",
        "library.pinout_preview",
        "library.pinout_apply",
        "library.package_plan",
        "library.package_from_template",
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
    ] {
        assert!(
            prompts::get(name, arguments(bad.clone()).as_ref()).is_err(),
            "{name} {bad}"
        );
    }
    Ok(())
}

/// One file is the source of both: the prompt serves the skill file in the
/// repository, read now, without its front matter.
#[test]
fn the_skill_file_and_the_prompt_are_one_source() -> Fallible<()> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../skills/sparksmith-datasheet-part/SKILL.md"
    );
    let on_disk = std::fs::read_to_string(path)?;
    assert_eq!(on_disk, DATASHEET_PART_SKILL);
    assert!(on_disk.starts_with("---\nname: sparksmith-datasheet-part\ndescription: "));
    let body = skill_body(&on_disk);
    assert!(body.starts_with("# Create a component from datasheets"));
    assert!(!body.contains("\n---\n") && !body.contains("name: sparksmith"));
    let mut arguments = rmcp::model::JsonObject::new();
    arguments.insert("part".into(), json!("P"));
    arguments.insert("datasheets".into(), json!("a.pdf"));
    let got = serde_json::to_value(prompts::get(DATASHEET_PART, Some(&arguments))?)?;
    let text = got["messages"][0]["content"]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(text.starts_with(body));
    Ok(())
}

/// The whole workflow over MCP with no model: schema, preview, apply,
/// footprint, checks, the review check refusing an incomplete report and
/// accepting the fixed one, and publishing refused by the review gate until
/// the person has confirmed the rows.
#[test]
fn a_scripted_session_completes_the_workflow_through_the_review_gate() -> Fallible<()> {
    // Publishing is Ask-class: allow-all stands in for the person's Allow,
    // so the test reaches the review gate that must still refuse it. The
    // board has a fab profile, whose margins the estimated lands take.
    let tools = Tools::new(
        Session::new(sparksmith_core::Engine::profiled()?),
        Grant::new(Mode::AllowAll, Vec::<std::path::PathBuf>::new()),
    )?;
    let schema = ok("schema", tools.query("library.pinout_schema", json!({})))?;
    assert_eq!(schema["format"], 1, "{schema}");

    // The agent forgot one row's source.
    let complete: Value = serde_json::from_str(EXM48)?;
    let mut document = complete.clone();
    let r2 = document["pins"]
        .as_array_mut()
        .and_then(|pins| pins.iter_mut().find(|p| p["row"] == "r2"))
        .ok_or("row r2")?;
    let name = r2["name"].clone();
    r2["source"] = Value::Null;
    let applied = import(
        &tools,
        "import",
        json!({"target": "new", "name": "EXM-48"}),
        &document,
    )?;
    let drafts: Vec<u64> = applied["report"]["drafts"]
        .as_array()
        .ok_or_else(|| format!("the import's drafts: {applied}"))?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    let component = drafts.first().copied().ok_or("the component draft")?;
    let symbol = drafts.get(1).copied().ok_or("the symbol draft")?;

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

    // The agent restores the source (before the footprint, which renumbers
    // pad 49 as EP) and asks about the uncertain row.
    import(
        &tools,
        "re-import",
        json!({"target": "draft", "id": component}),
        &complete,
    )?;
    // The footprint from a starter row; 49 is the form's EP.
    let binding = drafts
        .iter()
        .copied()
        .find(|&id| {
            draft_of(&tools, id).is_ok_and(|d| d["draft"]["definition"]["kind"] == "binding")
        })
        .ok_or("the binding draft")?;
    let package = draft_of(&tools, binding)?["draft"]["definition"]["body"]["package"]["id"]
        .as_u64()
        .ok_or("the package draft")?;
    let rows = query(
        &tools,
        "templates",
        "library.templates",
        json!({"query": "qfn pins:48 pitch:0.5 body:7x7"}),
    )?;
    let starter = rows["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|h| h["starter"].as_str().filter(|n| n.contains("EP 5.3")))
        .ok_or_else(|| format!("a starter row: {rows}"))?
        .to_owned();
    let request = json!({"draft": package, "row": {"source": "starter", "name": starter},
        "renumber": [{"from": "49", "to": "EP"}]});
    let plan = query(
        &tools,
        "package plan",
        "library.package_plan",
        json!({"request": request}),
    )?;
    assert_eq!(plan["footprint_created"], true, "{plan}");
    let built = apply(
        &tools,
        "package from template",
        "library.package_from_template",
        request,
    )?;
    let footprint = built["plan"]["footprint"]
        .as_u64()
        .ok_or_else(|| format!("the footprint: {built}"))?;
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
    body["designator_prefix"] = json!("U");
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

//! The workflow contract: the shipped datasheet workflow passes its load
//! check, and every rule refuses a broken file naming the field.
use super::control::{Control, ControlCard};
use super::load::{self, Catalog, HostCatalog, Source};
use super::*;
use serde_json::json;
use std::path::{Path, PathBuf};

type Fallible = std::result::Result<(), Box<dyn std::error::Error>>;

const DATASHEET: &str = "datasheet-to-component";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn shipped_folder() -> PathBuf {
    repo().join("library/assist/workflows")
}

fn shipped_text() -> Result<String, std::io::Error> {
    std::fs::read_to_string(shipped_folder().join(DATASHEET).join(WORKFLOW_FILE))
}

/// The shipped workflow's method text.
fn method_text() -> Result<String, std::io::Error> {
    std::fs::read_to_string(shipped_folder().join(DATASHEET).join(METHOD_FILE))
}

fn shipped() -> Result<Workflow, Box<dyn std::error::Error>> {
    Ok(Workflow::parse(&shipped_text()?, DATASHEET)?)
}

/// The shipped workflow with `edit` applied to its JSON, then checked.
fn refused(edit: impl FnOnce(&mut Value)) -> Result<String, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(&shipped_text()?)?;
    edit(&mut value);
    match Workflow::parse(&value.to_string(), "edited") {
        Ok(_) => Err("the edited workflow passed its check".into()),
        Err(why) => Ok(why),
    }
}

fn step_mut<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    value["steps"]
        .as_array_mut()?
        .iter_mut()
        .find(|s| s["id"] == id)
}

#[test]
fn the_shipped_datasheet_workflow_passes_its_check() -> Fallible {
    let workflow = shipped()?;
    assert_eq!(workflow.id, DATASHEET);
    assert_eq!(workflow.version(), Some((1, 2)));
    assert_eq!(workflow.harness_profile.as_deref(), Some("datasheet"));
    assert_eq!(
        workflow.offer_when,
        vec![Signal::Attachment("application/pdf".into())]
    );
    let kinds: Vec<StepKind> = workflow.steps.iter().map(|s| s.kind).collect();
    assert_eq!(kinds.first(), Some(&StepKind::Input));
    assert_eq!(kinds.last(), Some(&StepKind::Result));
    // Three judgement steps; the rest runs without a model.
    assert_eq!(kinds.iter().filter(|k| **k == StepKind::Model).count(), 3);
    // Its model steps' guides are headings of the method text.
    workflow
        .check_method(&method_text()?)
        .map_err(|p| format!("{p:?}"))?;
    // Every input gets a card from the closed set.
    for (name, input) in &workflow.inputs {
        let card = ControlCard::for_input(name, input, None, Vec::new());
        assert_eq!(card.control(), input.control(), "{name}");
    }
    assert_eq!(
        workflow.input("datasheets").map(Input::control),
        Some(Control::Datasheet)
    );
    Ok(())
}

#[test]
fn harness_profiles_are_named_at_the_workflow_or_model_step() -> Fallible {
    assert!(refused(|w| w["harness_profile"] = json!("../name"))?.contains("/harness_profile"));
    let step = shipped()?
        .steps
        .iter()
        .position(|s| s.kind == StepKind::Model)
        .ok_or("model step")?;
    let at = format!("/steps/{step}/harness_profile");
    assert!(
        refused(|w| w["steps"][step]["harness_profile"] = json!("invalid/name"))?.contains(&at)
    );
    assert!(
        refused(|w| w["steps"][0]["harness_profile"] = json!("compact"))?
            .contains("/steps/0/harness_profile")
    );
    let mut value: Value = serde_json::from_str(&shipped_text()?)?;
    value["harness_profile"] = json!("custom");
    value["steps"][step]["harness_profile"] = json!("compact");
    let workflow = Workflow::parse(&value.to_string(), "workflow.json")?;
    assert_eq!(workflow.harness_profile.as_deref(), Some("custom"));
    assert_eq!(
        workflow.steps[step].harness_profile.as_deref(),
        Some("compact")
    );
    Ok(())
}

#[test]
fn the_shipped_workflow_is_available_here() -> Fallible {
    // Its folder loads (its method beside it) and this host meets its
    // needs. The result card's view.open is left off a host without it,
    // not a reason.
    let listed = load::list(Some(&shipped_folder()), None, &HostCatalog);
    let ours = listed
        .iter()
        .find(|l| l.id == DATASHEET)
        .ok_or("the shipped workflow is listed")?;
    assert_eq!(ours.why_not, Vec::<String>::new());
    assert!(ours.available());
    Ok(())
}

/// The first occurrence of each of `names` in `text`, in order: `None` when
/// one is missing or comes before the one listed before it.
fn in_order(text: &str, names: &[&str]) -> Result<(), String> {
    let mut last = 0;
    for name in names {
        let quoted = format!("`{name}`");
        let Some(at) = text.find(&quoted) else {
            return Err(format!("{name} is not in the procedure"));
        };
        if at < last {
            return Err(format!("{name} comes before the step listed before it"));
        }
        last = at;
    }
    Ok(())
}

/// The method an external client follows and the steps the runner makes
/// cannot drift apart silently: every entry a tool step or a confirm gate
/// calls appears in method.md's procedure, in the same order.
#[test]
fn the_steps_entries_appear_in_the_method_in_the_same_order() -> Fallible {
    let workflow = shipped()?;
    let mut calls: Vec<&str> = Vec::new();
    for step in &workflow.steps {
        let call = match step.kind {
            StepKind::Tool => step.call.as_deref(),
            StepKind::Review => step.then.as_ref().map(|t| t.call.as_str()),
            _ => None,
        };
        if let Some(call) = call
            && !calls.contains(&call)
        {
            calls.push(call);
        }
    }
    assert!(calls.len() >= 7, "{calls:?}");
    let method = method_text()?;
    let procedure = method_section(&method, "procedure").ok_or("a procedure section")?;
    in_order(procedure, &calls)?;
    // The test notices a step moved.
    calls.swap(1, 2);
    assert!(in_order(procedure, &calls).is_err());
    Ok(())
}

#[test]
fn the_shipped_index_stays_under_one_kib() -> Fallible {
    let workflow = shipped()?;
    let index = index(&[&workflow]);
    assert!(index.len() < 1024, "{} bytes: {index}", index.len());
    assert!(index.starts_with("datasheet-to-component v1.2: "));
    Ok(())
}

/// A host with the shipped workflow's modules and entries, and `view`.
struct Fake;

impl Catalog for Fake {
    fn module_version(&self, id: &str) -> Option<(u16, u16)> {
        match id {
            "library" => Some((4, 0)),
            "generators" => Some((2, 0)),
            "assist" => Some((1, 4)),
            _ => None,
        }
    }

    fn entry(&self, name: &str) -> Option<(String, Class)> {
        match name {
            "assist.document_text" => Some(("assist".into(), Class::Read)),
            "view.open" => Some(("view".into(), Class::Show)),
            other => HostCatalog.entry(other),
        }
    }
}

#[test]
fn availability_names_versions_entries_and_reach() -> Fallible {
    let mut workflow = shipped()?;
    assert_eq!(workflow.availability(&Fake), Vec::<String>::new());
    workflow.needs[0].major = 2;
    workflow.reach.insert("library".into(), Class::Read);
    let why = workflow.availability(&Fake);
    assert!(
        why.contains(&"needs library 2.0; this host has library 4.0".to_owned()),
        "{why:?}"
    );
    assert!(
        why.contains(&"library.draft_closure is edit; reach allows library up to read".to_owned()),
        "{why:?}"
    );
    workflow.reach.remove("generators");
    let why = workflow.availability(&Fake);
    assert!(
        why.contains(
            &"library.generate_part is in the generators module, which reach leaves out".to_owned()
        ),
        "{why:?}"
    );
    Ok(())
}

fn copy_folder(to: &Path, id: &str, workflow: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = to.join(id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(WORKFLOW_FILE), workflow)?;
    std::fs::write(dir.join(METHOD_FILE), method_text()?)?;
    Ok(dir)
}

#[test]
fn a_folder_loads_with_its_digest_and_a_user_workflow_replaces_a_shipped_one() -> Fallible {
    let temp = tempfile::tempdir()?;
    let (ours, theirs) = (temp.path().join("shipped"), temp.path().join("user"));
    let text = shipped_text()?;
    let dir = copy_folder(&ours, DATASHEET, &text)?;
    let loaded = load::load_folder(&dir, Source::Shipped)?;
    assert_eq!(
        loaded.digest,
        load::digest(text.as_bytes(), method_text()?.as_bytes())
    );
    assert!(loaded.digest.starts_with("sha256:") && loaded.digest.len() == 71);

    // A changed method changes the digest a recording names.
    std::fs::write(dir.join(METHOD_FILE), method_text()? + "\nMore.\n")?;
    assert_ne!(
        load::load_folder(&dir, Source::Shipped)?.digest,
        loaded.digest
    );

    // The user's copy, version 1.3, takes the shipped one's place; a
    // broken folder is listed with the reason.
    copy_folder(&theirs, DATASHEET, &text.replace("\"1.2\"", "\"1.3\""))?;
    copy_folder(&theirs, "broken", "{\"format\": 2}")?;
    let listed = load::list(Some(&ours), Some(&theirs), &Fake);
    let ids: Vec<&str> = listed.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["broken", DATASHEET]);
    assert!(!listed[0].available());
    assert!(
        listed[0].why_not[0].contains("workflow.json"),
        "{:?}",
        listed[0].why_not
    );
    let user = &listed[1];
    assert_eq!(user.source, Source::User);
    assert!(user.available(), "{:?}", user.why_not);
    assert_eq!(
        user.loaded.as_ref().map(|l| l.workflow.version.as_str()),
        Some("1.3")
    );
    Ok(())
}

#[test]
fn a_folder_is_refused_when_its_name_method_or_headings_do_not_fit() -> Fallible {
    let temp = tempfile::tempdir()?;
    let text = shipped_text()?;
    let dir = copy_folder(temp.path(), "other-name", &text)?;
    let why = load::load_folder(&dir, Source::User)
        .err()
        .unwrap_or_default();
    assert!(
        why.contains("the folder must be named datasheet-to-component"),
        "{why}"
    );

    let dir = copy_folder(temp.path(), DATASHEET, &text)?;
    std::fs::write(dir.join(METHOD_FILE), "# Method\n\n## Procedure\n")?;
    let why = load::load_folder(&dir, Source::User)
        .err()
        .unwrap_or_default();
    assert!(
        why.contains("/steps/11/method: method.md has no heading #the-review-report"),
        "{why}"
    );

    std::fs::remove_file(dir.join(METHOD_FILE))?;
    let why = load::load_folder(&dir, Source::User)
        .err()
        .unwrap_or_default();
    assert!(why.contains("method.md"), "{why}");
    Ok(())
}

#[test]
fn unknown_fields_and_controls_are_refused_by_name() -> Fallible {
    let why = refused(|v| v["colour"] = json!("red"))?;
    assert!(why.contains("unknown field `colour`"), "{why}");
    let why = refused(|v| {
        if let Some(step) = step_mut(v, "pick") {
            step["control"] = json!("slider");
        }
    })?;
    assert!(
        why.contains("unknown variant `slider`") && why.contains("page_ranges"),
        "{why}"
    );
    let why = refused(|v| v["inputs"]["part"]["placeholder"] = json!("TLV"))?;
    assert!(why.contains("unknown field `placeholder`"), "{why}");
    Ok(())
}

/// An edit of a workflow's JSON.
type Edit = Box<dyn FnOnce(&mut Value)>;

#[test]
fn each_rule_names_its_field() -> Fallible {
    let cases: Vec<(Edit, &str)> = vec![
        (
            Box::new(|v| v["format"] = json!(2)),
            "/format: format 2 is not supported",
        ),
        (
            Box::new(|v| v["version"] = json!("1")),
            "/version: \"1\" is not major.minor",
        ),
        (
            Box::new(|v| v["intents"] = json!(["one"])),
            "/intents: 1 given",
        ),
        (
            Box::new(|v| v["reach"]["settings"] = json!("never")),
            "/reach/settings: never is out of every agent's reach",
        ),
        (
            Box::new(|v| v["needs"][0]["major"] = json!(0)),
            "/needs/0/major: modules start at 1.0",
        ),
        (
            Box::new(|v| {
                v["budget"] = json!({"requests": 1, "output_tokens": 1, "minutes": 0});
            }),
            "/budget/minutes: from 1 to 1440",
        ),
        (
            Box::new(|v| {
                v["budget"] = json!({"requests": 0, "output_tokens": 1, "minutes": 1});
            }),
            "/budget/requests: from 1 to 1000",
        ),
        (
            Box::new(|v| {
                v["budget"] =
                    json!({"requests": 1, "output_tokens": 9, "total_tokens": 8, "minutes": 1});
            }),
            "/budget/total_tokens: from output_tokens",
        ),
        (
            Box::new(|v| v["budget"] = json!({"requests": 1, "output_tokens": 1})),
            "minutes",
        ),
        (
            Box::new(|v| {
                if let Some(step) = step_mut(v, "plan") {
                    step["requests"] = json!(1001);
                }
            }),
            "/requests: from 1 to 1000",
        ),
        (
            Box::new(|v| {
                v["budget"] = json!({"requests": 5, "output_tokens": 1, "minutes": 1});
                if let Some(step) = step_mut(v, "plan") {
                    step["requests"] = json!(6);
                }
            }),
            "/requests: from 1 to 5",
        ),
        (
            Box::new(|v| v["offer_when"] = json!([{"attachment": "pdf"}])),
            "/offer_when/0",
        ),
        (
            Box::new(|v| v["inputs"]["part"]["control"] = json!("datasheet")),
            "/inputs/part/control: datasheet does not show a text input",
        ),
        (
            Box::new(|v| v["inputs"]["part"]["accept"] = json!(["text/plain"])),
            "/inputs/part/accept: is for files inputs, not text",
        ),
        (
            Box::new(|v| v["inputs"]["part"]["default"] = json!("x".repeat(65))),
            "/inputs/part/default: 65 bytes; at most 64",
        ),
        (
            Box::new(|v| {
                if let Some(o) = v["inputs"]["part"].as_object_mut() {
                    o.remove("max");
                }
            }),
            "/inputs/part/max: a text input states its most bytes",
        ),
        (
            Box::new(|v| {
                v["inputs"]["pages"] =
                    json!({"type": "page_ranges", "title": "Pages", "of": "part"});
                if let Some(s) = step_mut(v, "ask") {
                    s["inputs"] = json!(["datasheets", "part", "pages", "notes"]);
                }
            }),
            "/inputs/pages/of: \"part\" is not a files input",
        ),
        (
            Box::new(|v| v["inputs"]["extra"] = json!({"type": "url", "title": "Link"})),
            "/inputs/extra: no input step asks for it",
        ),
        (
            Box::new(|v| v["outputs"]["unused"] = json!({"fields": {"a": {"type": "text"}}})),
            "/outputs/unused: no model step answers with it",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "read") {
                    s["goal"] = json!("Read.");
                }
            }),
            "/steps/1/goal: is for model steps, not tool",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "gens") {
                    s["params"] = json!({"x": "/steps/make/outcome"});
                }
            }),
            "/steps/2/params/x: \"make\" is not an earlier step",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "gens") {
                    s["params"] = json!({"x": "/item"});
                }
            }),
            "/steps/2/params/x: /item is only for a step with each",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "gens") {
                    s["params"] = json!({"x": "/input/part"});
                }
            }),
            "/steps/2/params/x: \"/input/part\" is not a pointer",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "make") {
                    s["params"]["generator"] = json!("/steps/plan/maker");
                }
            }),
            "/steps/5/params/generator: step plan's answer has no field \"maker\"",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "plan") {
                    s["check"] = json!("ask");
                }
            }),
            "/steps/3/check: \"ask\" is not a later tool step",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "plan") {
                    s["method"] = json!("guide.md#procedure");
                }
            }),
            "/steps/3/method",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "plan") {
                    s["output"] = json!("plan");
                }
            }),
            "/steps/3/output: no output \"plan\" in outputs",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "make") {
                    s["call"] = json!("Generate Part");
                }
            }),
            "/steps/5/call: \"Generate Part\" is not an entry name",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "import") {
                    s["control"] = json!("choice");
                }
            }),
            "/steps/9/control: a confirm gate shows impact",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "pick") {
                    s["control"] = json!("meter");
                }
            }),
            "/steps/4/control: meter takes no input",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "pick") {
                    s["change"] = json!("gens");
                }
            }),
            "/steps/4/change: \"gens\" is not an earlier model step",
        ),
        (
            Box::new(|v| {
                if let Some(s) = step_mut(v, "ask") {
                    s["inputs"] = json!(["part", "part", "datasheets", "notes"]);
                }
            }),
            "/steps/0/inputs/1: part is asked for by an earlier step",
        ),
        (
            Box::new(|v| {
                if let Some(list) = v["steps"].as_array_mut() {
                    list.pop();
                }
            }),
            "/steps: end with a result step",
        ),
        (
            Box::new(|v| {
                if let Some(list) = v["steps"].as_array_mut() {
                    list.swap(0, 1);
                }
            }),
            "/steps/0/each: input datasheets is not asked for before this step",
        ),
    ];
    for (edit, expected) in cases {
        let why = refused(edit)?;
        assert!(why.contains(expected), "expected {expected:?} in {why}");
    }
    Ok(())
}

#[test]
fn ten_controls_each_with_its_inputs() -> Fallible {
    let names: Vec<&str> = Control::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(
        names,
        [
            "text",
            "number",
            "choice",
            "datasheet",
            "url",
            "page_ranges",
            "impact",
            "progress",
            "meter",
            "result"
        ]
    );
    for control in Control::ALL {
        assert_eq!(serde_json::to_value(control)?, json!(control.name()));
    }
    // Each input type has a default control that shows it; only the first
    // six take input.
    for ty in [
        InputType::Text,
        InputType::Length,
        InputType::Angle,
        InputType::Number,
        InputType::Integer,
        InputType::Enum,
        InputType::EnumFrom,
        InputType::Files,
        InputType::Url,
        InputType::PageRanges,
    ] {
        let control = Control::for_input(ty);
        assert!(
            control.accepts(ty) && control.takes_input(),
            "{}",
            ty.name()
        );
    }
    assert_eq!(Control::ALL.iter().filter(|c| c.takes_input()).count(), 6);
    Ok(())
}

#[test]
fn inputs_check_their_values() -> Fallible {
    let workflow = shipped()?;
    let input = |name: &str| workflow.input(name).ok_or("no input");
    let files = input("datasheets")?;
    assert!(files.check_value(&json!(["a.pdf"])).is_ok());
    assert!(files.check_value(&json!([])).is_err());
    assert!(
        files
            .check_value(&json!(["1", "2", "3", "4", "5"]))
            .is_err()
    );
    let pages: Input = serde_json::from_value(
        json!({"type": "page_ranges", "title": "Pages", "of": "datasheets"}),
    )?;
    assert!(pages.check_value(&json!("1-5, 31, 32")).is_ok());
    assert!(pages.check_value(&json!("5-1")).is_err());
    assert_eq!(parse_page_ranges("3, 31-32")?, vec![(3, 3), (31, 32)]);
    let length: Input = serde_json::from_value(
        json!({"type": "length", "title": "Pitch", "min": 100000, "max": 2540000}),
    )?;
    assert!(length.check_value(&json!(500000)).is_ok());
    assert!(length.check_value(&json!(0.5)).is_err());
    assert!(length.check_value(&json!(3000000)).is_err());
    Ok(())
}

#[test]
fn a_step_answer_is_checked_against_its_output() -> Fallible {
    let workflow = shipped()?;
    let plan = workflow
        .outputs
        .get("package_plan")
        .ok_or("no package_plan")?;
    assert!(
        plan.check_value(&json!({"generator": "tabbed", "params": {}, "why": "SOT-23-5", "pages": "ds.pdf: drawing 31"}))
            .is_ok()
    );
    let problems = plan
        .check_value(&json!({"generator": 3, "params": {}, "colour": "red"}))
        .err()
        .unwrap_or_default();
    let text: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert_eq!(
        text,
        [
            "/generator: give text",
            "/pages: missing",
            "/why: missing",
            "/colour: not a field of this answer; its fields: generator, pages, params, preset, why",
        ]
    );
    Ok(())
}

#[test]
fn the_format_has_a_schema_and_entries_have_theirs() -> Fallible {
    let schema = schema();
    assert!(schema["properties"]["steps"].is_object(), "{schema}");
    for entry in [
        serde_json::to_value(schemars::schema_for!(entries::WorkflowsParams))?,
        serde_json::to_value(schemars::schema_for!(entries::OfferParams))?,
        serde_json::to_value(schemars::schema_for!(entries::StepResultParams))?,
    ] {
        assert_eq!(entry["additionalProperties"], json!(false), "{entry}");
    }
    let offer: entries::OfferParams = serde_json::from_value(json!({
        "workflow": DATASHEET, "inputs": {"part": "TLV75733PDBVR"}, "why": "A datasheet is attached."
    }))?;
    assert_eq!(offer.inputs.len(), 1);
    Ok(())
}

#[test]
fn heading_anchors_follow_github() {
    let anchors = heading_anchors(
        "# The review report\n```\n# not a heading\n```\n## Find a generator (step 3)\n#hashtag\n",
    );
    let anchors: Vec<&str> = anchors.iter().map(String::as_str).collect();
    assert_eq!(anchors, ["find-a-generator-step-3", "the-review-report"]);
}

/// `offer_when` matches structured signals only: a dropped PDF offers the
/// datasheet workflow and fills its files input; another type, a
/// workspace it does not name, or an issue offers nothing.
#[test]
fn signals_offer_the_workflows_that_name_them() -> Fallible {
    let listed = load::list(Some(&shipped_folder()), None, &HostCatalog);
    let pdf = Signal::Attachment("application/pdf".into());
    let offered = offer::offered_on(&listed, &pdf);
    let ids: Vec<&str> = offered.iter().map(|l| l.workflow.id.as_str()).collect();
    assert_eq!(ids, [DATASHEET]);
    assert!(shipped()?.offered_on(&Signal::Attachment("Application/PDF".into())));
    for other in [
        Signal::Attachment("image/png".into()),
        Signal::Workspace("pcb".into()),
        Signal::Issue("pinout.unreviewed".into()),
    ] {
        assert!(offer::offered_on(&listed, &other).is_empty(), "{other:?}");
    }
    assert_eq!(
        shipped()?.input_for_attachment("application/pdf"),
        Some("datasheets")
    );
    assert_eq!(shipped()?.input_for_attachment("image/png"), None);
    assert_eq!(offer::media_type(b"%PDF-1.7\n"), Some("application/pdf"));
    assert_eq!(offer::media_type(b"<html>"), None);
    Ok(())
}

/// Step results read as a person reads them, never as JSON.
#[test]
fn results_read_in_words() {
    use control::readable;
    let drafts = json!({"ids": [2, 3], "drafts": [
        {"id": 2, "kind": "component", "name": "TLV75733PDBVR"},
        {"id": 3, "kind": "symbol", "name": "TLV75733PDBVR"}]});
    assert_eq!(readable(&drafts), "TLV75733PDBVR: component, symbol");
    assert_eq!(
        readable(&json!({"accepted": true, "markdown": "# R"})),
        "Accepted"
    );
    assert_eq!(
        readable(&json!({"report": {}, "result": {"findings": [
            {"message": "a", "blocks": true}, {"message": "b", "blocks": false}]}})),
        "2 findings, 1 blocking"
    );
    assert_eq!(
        readable(
            &json!([{"document": "tlv757p.pdf", "text": "=== page 1 ===\na\n=== page 2 ===\nb"}])
        ),
        "tlv757p.pdf · 2 pages"
    );
    assert_eq!(
        readable(&json!({"generator": "tabbed", "why": "SOT-23-5, TI lands"})),
        "SOT-23-5, TI lands"
    );
    assert_eq!(
        readable(
            &json!({"changes": {"added": {"library.components": [1], "library.symbols": [2]}}, "outcome": {}})
        ),
        "Added: components, symbols"
    );
    assert_eq!(
        readable(&json!({"outcome": "drafts_created", "ids": [1, 2]})),
        "Drafts created: 2"
    );
    assert_eq!(
        control::counted_label(4, "function added"),
        "4 functions added"
    );
    assert_eq!(
        control::counted_label(1, "terminal changed"),
        "1 terminal changed"
    );
    let findings = control::findings_of(
        &json!({"result": {"findings": [{"message": "Pin 3 unreviewed", "blocking": true}]}}),
    );
    assert_eq!(findings.len(), 1);
    assert!(findings[0].blocking);
}

/// A conversation's attachments fill the start inputs that take their
/// type, in order and within the input's bounds, without a model; what was
/// given stays.
#[test]
fn attachments_fill_the_files_inputs() -> Fallible {
    let workflow = shipped()?;
    assert_eq!(workflow.start_inputs(), ["datasheets", "part", "notes"]);
    let attached = |names: &[&str]| -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| {
                let media = if n.ends_with(".pdf") {
                    "application/pdf"
                } else {
                    "image/png"
                };
                ((*n).to_owned(), media.to_owned())
            })
            .collect()
    };
    let mut inputs = serde_json::Map::new();
    workflow.fill_files(&mut inputs, &attached(&["a.png", "a.pdf", "b.pdf"]));
    assert_eq!(inputs["datasheets"], json!(["a.pdf", "b.pdf"]));
    // At most `max` (4) are taken.
    let mut inputs = serde_json::Map::new();
    workflow.fill_files(
        &mut inputs,
        &attached(&["1.pdf", "2.pdf", "3.pdf", "4.pdf", "5.pdf"]),
    );
    assert_eq!(
        inputs["datasheets"],
        json!(["1.pdf", "2.pdf", "3.pdf", "4.pdf"])
    );
    // Nothing of a type it takes: it stays unfilled.
    let mut inputs = serde_json::Map::new();
    workflow.fill_files(&mut inputs, &attached(&["a.png"]));
    assert!(inputs.is_empty());
    // A given value stays.
    let mut inputs = serde_json::Map::new();
    inputs.insert("datasheets".into(), json!(["given.pdf"]));
    workflow.fill_files(&mut inputs, &attached(&["a.pdf"]));
    assert_eq!(inputs["datasheets"], json!(["given.pdf"]));
    Ok(())
}

/// A shipped workflow limits nothing: no budget, and no request cap on a
/// step. One a person writes may declare both.
#[test]
fn no_shipped_workflow_declares_a_limit() -> Fallible {
    let listed = load::list(Some(&shipped_folder()), None, &HostCatalog);
    assert!(!listed.is_empty());
    for workflow in load::available(&listed) {
        let workflow = workflow.workflow;
        assert_eq!(workflow.budget, None, "{}", workflow.id);
        for step in &workflow.steps {
            assert_eq!(step.requests, None, "{} {}", workflow.id, step.id);
        }
    }
    let mut own: Value = serde_json::from_str(&shipped_text()?)?;
    own["budget"] = json!({"requests": 40, "output_tokens": 200_000, "minutes": 45});
    if let Some(step) = step_mut(&mut own, "plan") {
        step["requests"] = json!(40);
    }
    let own = Workflow::parse(&own.to_string(), "own")?;
    assert_eq!(own.budget.map(|b| b.requests), Some(40));
    Ok(())
}

/// An offer is checked like a run's inputs: the card names what the run
/// will still ask for, in the order its input steps ask.
#[test]
fn an_offer_names_the_inputs_still_missing() -> Fallible {
    let listed = load::list(Some(&shipped_folder()), None, &HostCatalog);
    let none = Offer::check(&listed, DATASHEET, serde_json::Map::new(), "")?;
    assert_eq!(none.missing, ["datasheets", "part"]);
    let mut inputs = serde_json::Map::new();
    inputs.insert("datasheets".into(), json!(["tlv757p.pdf"]));
    let one = Offer::check(&listed, DATASHEET, inputs, "A PDF was dropped")?;
    assert_eq!(one.missing, ["part"]);
    assert_eq!(one.version, "1.2");
    let long = "x".repeat(401);
    let Err(why) = Offer::check(&listed, DATASHEET, serde_json::Map::new(), &long) else {
        return Err("a long why was taken".into());
    };
    assert!(why.contains("why"), "{why}");
    Ok(())
}

/// The body workflow passes its check, is available here, offers itself on
/// no signal (a dropped PDF stays the datasheet workflow's) and its method
/// lists its steps' entries in their order.
#[test]
fn the_shipped_body_workflow_passes_its_check_and_follows_its_method() -> Fallible {
    const BODY: &str = "datasheet-to-body";
    let folder = shipped_folder().join(BODY);
    let workflow = Workflow::parse(&std::fs::read_to_string(folder.join(WORKFLOW_FILE))?, BODY)?;
    let method = std::fs::read_to_string(folder.join(METHOD_FILE))?;
    workflow
        .check_method(&method)
        .map_err(|p| format!("{p:?}"))?;
    assert!(workflow.offer_when.is_empty());
    assert_eq!(workflow.harness_profile.as_deref(), Some("compact"));
    let listed = load::list(Some(&shipped_folder()), None, &HostCatalog);
    let ours = listed.iter().find(|l| l.id == BODY).ok_or("listed")?;
    assert_eq!(ours.why_not, Vec::<String>::new());
    let mut calls: Vec<&str> = Vec::new();
    for step in &workflow.steps {
        let call = match step.kind {
            StepKind::Tool => step.call.as_deref(),
            StepKind::Review => step.then.as_ref().map(|t| t.call.as_str()),
            _ => None,
        };
        if let Some(call) = call
            && !calls.contains(&call)
        {
            calls.push(call);
        }
    }
    assert_eq!(
        calls,
        [
            "assist.document_text",
            "library.body_fit",
            "library.generators",
            "library.make_body",
            "library.review_check"
        ]
    );
    let procedure = method_section(&method, "procedure").ok_or("a procedure section")?;
    in_order(procedure, &calls)?;
    Ok(())
}

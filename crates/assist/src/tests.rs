//! The tool loop against recorded responses of both backends (no network): an original
//! synthetic radio module, EXR-22, authored end to end through the six
//! verbs, the run's reach profile, and the caps, cancellation and refusal
//! paths.
use super::*;
use crate::documents::Delivery;
use crate::model::anthropic::AnthropicBackend;
use crate::model::openai::OpenAiBackend;
use crate::model::wire::ScriptedWire;
use crate::model::{ModelBackend, ModelProfile};
use crate::profile::ReachProfile;
use crate::verbs::{self, Agent};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// A session on a board with the test fixture fab profile: the estimated
/// lands a run builds take their margins from the board's profile.
fn profiled_session() -> Session {
    Session::new(sparksmith_core::Engine::profiled().unwrap_or_default())
}

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// EXR-22: 8 + 6 + 8 castellated pads at 1.27 mm and a 2 × 2 ground array
/// (terminal 23). Invented for this test.
fn pinout() -> Value {
    let mut pins = Vec::new();
    for n in 1..=22u32 {
        let (name, functions, default) = match n {
            1 | 22 => (
                format!("GND{n}"),
                json!([{"label": "gnd", "direction": "power_input", "signal": "power"}]),
                "gnd",
            ),
            2 => (
                "VCC".to_owned(),
                json!([{"label": "vcc", "direction": "power_input", "signal": "power"}]),
                "vcc",
            ),
            _ => {
                let g = n - 3;
                let mut f = vec![
                    json!({"label": "gpio_in", "direction": "input", "signal": "digital"}),
                    json!({"label": "gpio_out", "direction": "output", "signal": "digital", "drive": "push_pull"}),
                ];
                if g == 0 {
                    f.push(json!({"label": "uart0_tx", "direction": "output", "signal": "digital", "drive": "push_pull", "peripheral": "UART0", "role": "tx", "mux": "F1"}));
                }
                (format!("IO{g}"), json!(f), "gpio_in")
            }
        };
        pins.push(
            json!({"row": format!("r{n}"), "number": {"MOD": n.to_string()}, "name": name,
            "class": "electrical", "functions": functions, "default": default,
            "source": {"page": 3, "table": "Table 1"}}),
        );
    }
    pins.push(
        json!({"row": "r23", "number": {"MOD": "23"}, "name": "EPAD", "class": "electrical",
        "functions": [{"label": "gnd", "direction": "power_input", "signal": "power"}],
        "default": "gnd", "source": {"page": 9, "table": "Figure 4"}}),
    );
    json!({
        "format": 1,
        "device": {"name": "EXR-22 radio module", "reference": {"title": "EXR-22 datasheet (fictitious)", "document_revision": "1"}},
        "variants": [{"name": "MOD", "terminal_count": 23}],
        "peripherals": [{"label": "UART0", "kind": "uart", "roles": [{"label": "tx", "signal": "TX", "required_when_used": true}]}],
        "pins": pins,
        "rules": [{"label": "io0_strap", "script": "if active(\"IO0\", \"uart0_tx\") { warn(\"IO0 selects the boot mode at reset\", [\"IO0\"]) } else { ok() }", "source": {"page": 5}}],
    })
}

fn module_values() -> Value {
    json!({"family": "module", "width": 14.0, "length": 20.0, "height": 2.8, "pcb_thickness": 0.8,
        "antenna_length": 5.0, "can_size": [12.0, 12.4], "can_centre": [0.0, -2.2],
        "pitch": 1.27, "pad_size": [1.0, 0.8],
        "rows": [
            {"side": "left", "count": 8, "first": [-6.5, 2.2]},
            {"side": "bottom", "count": 6, "first": [-3.175, -9.5]},
            {"side": "right", "count": 8, "first": [6.5, 2.2 - 7.0 * 1.27]},
        ],
        "castellation": 0.5, "land_width": 0.8, "land_extend": [0.4, 0.0],
        "ground": {"rows": 2, "cols": 2, "pitch": [2.0, 2.0], "size": [1.2, 1.2], "centre": [0.0, -3.0], "number": "23"},
        "keepout": {"beyond": [0.0, 0.0, 0.0]}})
}

fn tool_turn(id: &str, name: &str, input: Value) -> Value {
    json!({"id": format!("msg_{id}"), "type": "message", "role": "assistant", "model": "claude-opus-5-5",
        "content": [{"type": "text", "text": format!("Calling {name}.")},
                    {"type": "tool_use", "id": id, "name": name, "input": input}],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 500, "output_tokens": 1_000, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 50_000}})
}

/// The shipped datasheet run's reach profile.
fn reach() -> Fallible<ReachProfile> {
    Ok(crate::AssistLibrary::locate()?.reach)
}

/// The verb `name` with `input` on `session`, as a run under `reach` calls
/// it: the transport envelope.
fn verb_as(session: &mut Session, reach: &ReachProfile, name: &str, input: Value) -> Value {
    let grant = reach.grant();
    Agent {
        session,
        grant: &grant,
        turn: None,
    }
    .verb(name, input)
}

/// The verb's result under the shipped reach profile, or why it failed.
fn verb(session: &mut Session, name: &str, input: Value) -> Fallible<Value> {
    let reply = verb_as(session, &reach()?, name, input);
    if reply["ok"] == true {
        Ok(reply["result"].clone())
    } else {
        Err(format!("{name}: {reply}").into())
    }
}

/// A recorded call of a tool of before the verbs, as a replay runs it: read
/// as its verb call ([`crate::legacy`]), then through the verbs.
fn legacy_call(session: &mut Session, tool: &str, input: &Value) -> Fallible<Value> {
    let (name, arguments) =
        crate::legacy::translate(session, tool, input).ok_or("an old tool")??;
    verb(session, name, arguments)
}

fn query(entry: &str, params: Value) -> Value {
    json!({"name": entry, "params": params})
}

fn apply(revision: u64, entry: &str, params: Value) -> Value {
    json!({"expected_revision": revision, "operations": [{"op": entry, "params": params}]})
}

/// The recorded responses of a complete run on an empty project.
fn script() -> Fallible<Vec<Value>> {
    script_on(profiled_session())
}

/// The recorded responses of a complete run through the verbs, made
/// against a shadow session (a copy of the project the run will work on)
/// so the inputs carry the real revisions, plan base and draft IDs: the
/// method of the shipped prompt, the same as an MCP client's.
fn script_on(mut shadow: Session) -> Fallible<Vec<Value>> {
    shadow.ensure_generators();
    let doc = pinout();
    let mut out = vec![tool_turn(
        "t1",
        "query",
        query("library.pinout_schema", json!({})),
    )];
    // The document in two chunks: header, peripherals and the first rows;
    // then the remaining rows and the rules.
    let pins = doc["pins"].as_array().ok_or("pins")?;
    let (first, rest) = pins.split_at(12);
    let chunks = [
        json!({"format": doc["format"], "device": doc["device"], "variants": doc["variants"],
            "peripherals": doc["peripherals"], "pins": first}),
        json!({"pins": rest, "rules": doc["rules"]}),
    ];
    for (k, chunk) in chunks.into_iter().enumerate() {
        let input = query("assist.pinout_edit", chunk);
        verb(&mut shadow, "query", input.clone())?;
        out.push(tool_turn(&format!("t2{k}"), "query", input));
    }
    // The module generator makes the part; its drafts are the working set.
    let generate = apply(
        shadow.engine().revision(),
        "library.generate_part",
        json!({"generator": "module", "scope": "component", "name": "EXR-22",
            "params": crate::legacy::module_params(&module_values())}),
    );
    let made = verb(&mut shadow, "apply", generate.clone())?;
    out.push(tool_turn("t3", "apply", generate));
    let root = &made["outcome"]["report"]["root"];
    let closure = apply(
        shadow.engine().revision(),
        "library.draft_closure",
        json!({"component": {"id": root["id"], "revision": root["revision"]}}),
    );
    let before: Vec<_> = shadow
        .engine()
        .drafts()
        .items
        .iter()
        .map(|d| d.id)
        .collect();
    verb(&mut shadow, "apply", closure.clone())?;
    out.push(tool_turn("t4", "apply", closure));
    let component = shadow
        .engine()
        .drafts()
        .items
        .iter()
        .filter(|d| !before.contains(&d.id))
        .find(|d| {
            matches!(
                d.definition,
                sparksmith_core::library::Definition::Component(_)
            )
        })
        .map(|d| json!(d.id))
        .ok_or("a component draft")?;
    let preview = query(
        "assist.pinout_preview",
        json!({"target": {"target": "draft", "id": component}}),
    );
    verb(&mut shadow, "query", preview.clone())?;
    out.push(tool_turn("t5", "query", preview));
    let import = apply(shadow.engine().revision(), "assist.pinout_apply", json!({}));
    verb(&mut shadow, "apply", import.clone())?;
    out.push(tool_turn("t6", "apply", import));
    out.push(tool_turn(
        "t7",
        "apply",
        apply(
            shadow.engine().revision(),
            "library.set_marking",
            json!({"draft": component,
                "marking": {"items": [{"item": "text", "template": "EXR-22"}]}}),
        ),
    ));
    out.push(tool_turn(
        "t8",
        "query",
        query("component.check_drafts", json!({})),
    ));
    out.push(tool_turn(
        "t9",
        "query",
        query(
            "library.review_check",
            json!({"report": {
        "part": "EXR-22", "summary": "Synthetic module.",
        "sources": [{"name": "DS", "title": "EXR-22 datasheet", "revision": "1"}],
        "values": [{"item": "pitch", "value": "1.27 mm", "source": {"document": "DS", "page": 9, "location": "Figure 4"}, "confidence": "high"}],
        "open_questions": ["Ground array optional?"]}}),
        ),
    ));
    Ok(out)
}

fn datasheet(dir: &tempfile::TempDir) -> Fallible<Datasheet> {
    let path = dir.path().join("exr22.pdf");
    std::fs::write(
        &path,
        b"%PDF-1.4\n1 0 obj << /Type /Pages >> endobj\n2 0 obj << /Type /Page >> endobj\n3 0 obj <</Type/Page>> endobj\n",
    )?;
    Ok(Datasheet::File(path))
}

fn goal(sheet: Datasheet) -> Goal {
    Goal {
        name: "EXR-22".into(),
        datasheets: vec![sheet],
        notes: String::new(),
    }
}

/// The Messages backend answering `responses` (raw API JSON) in order, with
/// a token count for the first request.
fn anthropic(responses: Vec<Value>) -> Fallible<AnthropicBackend<ScriptedWire>> {
    let mut wire = ScriptedWire::new(responses);
    wire.count = Some(60_000);
    Ok(AnthropicBackend::new(shipped_config()?.profile, wire))
}

/// The shipped default run: the default model on the Messages API with its
/// run profile and the shipped base prompt.
fn shipped_config() -> Fallible<RunConfig> {
    Ok(RunConfig::shipped_default(&crate::AssistLibrary::locate()?)?)
}

/// The bodies the Messages backend posted.
fn bodies(backend: &AnthropicBackend<ScriptedWire>) -> Vec<&Value> {
    backend.wire.posts.iter().map(|(_, _, b, _)| b).collect()
}

fn run_with(
    config: &RunConfig,
    backend: &mut dyn ModelBackend,
    sheet: Datasheet,
    session: &mut Session,
) -> Fallible<(Outcome, Vec<Event>)> {
    let mut log = Vec::new();
    let outcome = run(
        &goal(sheet),
        config,
        backend,
        session,
        &mut |e| log.push(e),
        &AtomicBool::new(false),
    )?;
    Ok((outcome, log))
}

#[test]
fn a_recorded_run_authors_the_module_through_the_verbs_and_submits_its_review() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut backend = anthropic(script()?)?;
    let mut session = profiled_session();
    let (outcome, log) = run_with(
        &shipped_config()?,
        &mut backend,
        datasheet(&dir)?,
        &mut session,
    )?;
    assert_eq!(outcome.stop, Stop::Submitted, "{log:?}");
    assert!(
        log.iter().all(|e| !matches!(
            e,
            Event::Call {
                state: CallState::Refused { .. } | CallState::NotRun { .. },
                ..
            }
        )),
        "every tool call succeeded: {log:?}"
    );
    let report = outcome.report.ok_or("a report")?;
    assert_eq!(report.part, "EXR-22");
    // Ten responses at 500 in, 1000 out and 50 000 cache reads each.
    assert_eq!(outcome.facts.output_tokens, 10 * 1000);
    assert_eq!(outcome.facts.cache_read_tokens, 10 * 50_000);
    assert_eq!(outcome.facts.requests, 10);
    assert_eq!(outcome.facts.request_log.len(), 10);
    assert_eq!(
        outcome.facts.request_log[1].tools,
        vec!["query assist.pinout_edit".to_owned()]
    );
    assert_eq!(
        outcome.facts.request_log[6].tools,
        vec!["apply assist.pinout_apply".to_owned()]
    );
    assert_eq!(outcome.facts.request_log[1].stop_reason, "tool_calls");
    assert_eq!(outcome.facts.documents[0].delivery, Delivery::NativePdf);
    // Each change is a step of the run log.
    assert_eq!(outcome.facts.steps.len(), 4, "{:?}", outcome.facts.steps);
    // The review lists every pin by package number and the rule's checks.
    assert_eq!(outcome.facts.pins.len(), 23);
    assert_eq!(outcome.facts.pins[0].number, "1");
    assert_eq!(outcome.facts.pins[0].name, "GND1");
    assert_eq!(outcome.facts.rules.len(), 1);
    assert!(outcome.facts.rules[0].failures.is_empty());

    // The drafts: component, symbol, package, footprint and binding.
    let drafts = &session.engine().drafts().items;
    let footprint = drafts
        .iter()
        .find_map(|d| match &d.definition {
            sparksmith_core::library::Definition::Footprint(f) => Some(f),
            _ => None,
        })
        .ok_or("a footprint draft")?;
    assert_eq!(
        footprint.pads.len(),
        22 + 4,
        "edge lands and the ground array"
    );
    assert_eq!(footprint.keepouts.len(), 1, "the antenna keep-out");
    let pin1 = footprint
        .pads
        .iter()
        .find(|p| p.number == "1")
        .ok_or("pad 1")?;
    // The 1.4 mm land reaches 0.4 beyond the −x edge at −7: outer end −7.4,
    // centre −7.4 + 0.7.
    assert!(
        (pin1.position()[0] - (-7.4 + 0.7)).abs() < 1e-9,
        "{:?}",
        pin1.position
    );
    let component = drafts
        .iter()
        .find_map(|d| match &d.definition {
            sparksmith_core::library::Definition::Component(c) => Some(c),
            _ => None,
        })
        .ok_or("a component draft")?;
    assert!(component.marking.is_some(), "the can marking");
    assert!(
        component
            .review
            .as_ref()
            .is_some_and(|r| r.unreviewed.len() == 23),
        "every imported row waits for review"
    );
    // Nothing the person reviews was published: the library holds only the
    // generator's own revision, without the pinout or the marking.
    let published = &session.engine().project().library.components;
    assert_eq!(published.len(), 1);
    assert!(published.iter().all(|c| c.definition.marking.is_none()));

    // The first request: datasheet cached, untrusted, the six verbs,
    // adaptive thinking, fallback; no key anywhere in a request.
    let bodies = bodies(&backend);
    let first = bodies.first().ok_or("a request")?;
    let doc = &first["messages"][0]["content"][0];
    assert_eq!(doc["type"], "document");
    assert_eq!(doc["cache_control"]["type"], "ephemeral");
    assert!(
        doc["context"]
            .as_str()
            .is_some_and(|c| c.contains("untrusted"))
    );
    assert_eq!(first["thinking"]["type"], "adaptive");
    assert_eq!(first["fallbacks"], "default");
    assert_eq!(first["max_tokens"], 32_000);
    let names: Vec<&str> = first["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(names, verbs::NAMES);
    // Each later request carries the previous turn's assistant content
    // unchanged and one user message of tool results.
    let last = bodies.last().ok_or("a request")?;
    assert_eq!(last["messages"].as_array().map(Vec::len), Some(1 + 2 * 9));
    for pair in bodies.windows(2) {
        let (a, b) = (
            pair[0]["messages"].as_array().ok_or("messages")?,
            pair[1]["messages"].as_array().ok_or("messages")?,
        );
        assert_eq!(&b[..a.len()], &a[..], "append-only");
    }
    assert_eq!(
        last["messages"][3]["content"],
        script()?[1]["content"],
        "assistant content goes back as it came"
    );
    // The model never sent the document again to apply it.
    let import = bodies
        .iter()
        .flat_map(|r| r["messages"].as_array().cloned().unwrap_or_default())
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .find(|b| b["input"]["operations"][0]["op"] == "assist.pinout_apply")
        .ok_or("the apply call")?;
    assert!(
        import["input"]["operations"][0]["params"]
            .get("document")
            .is_none()
    );
    Ok(())
}

#[test]
fn a_cancelled_run_sends_nothing_and_a_refusal_stops() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut backend = anthropic(vec![])?;
    let outcome = run(
        &goal(datasheet(&dir)?),
        &shipped_config()?,
        &mut backend,
        &mut profiled_session(),
        &mut |_| {},
        &AtomicBool::new(true),
    )?;
    assert_eq!(outcome.stop, Stop::Cancelled);
    assert!(backend.wire.posts.is_empty());

    let refusal = json!({"id": "m", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
        "content": [], "stop_reason": "refusal", "stop_details": {"type": "refusal", "category": "cyber"},
        "usage": {"input_tokens": 10, "output_tokens": 0}});
    let mut backend = anthropic(vec![refusal])?;
    let (outcome, _) = run_with(
        &shipped_config()?,
        &mut backend,
        datasheet(&dir)?,
        &mut profiled_session(),
    )?;
    assert_eq!(outcome.stop, Stop::Refused("cyber".into()));
    Ok(())
}

#[test]
fn a_tool_call_cut_off_at_max_tokens_is_not_run() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut cut = tool_turn(
        "t1",
        "apply",
        apply(0, "library.set_marking", json!({"draft": 1})),
    );
    cut["stop_reason"] = json!("max_tokens");
    let mut backend = anthropic(vec![cut])?;
    let mut session = profiled_session();
    let before = session.engine().revision();
    let (outcome, _) = run_with(
        &shipped_config()?,
        &mut backend,
        datasheet(&dir)?,
        &mut session,
    )?;
    // The recording ends after the cut-off turn.
    assert!(matches!(outcome.stop, Stop::Failed(_)));
    assert_eq!(session.engine().revision(), before);
    let second = bodies(&backend)
        .get(1)
        .copied()
        .ok_or("the follow-up request")?;
    let result = &second["messages"][2]["content"][0];
    assert_eq!(result["is_error"], true);
    // The request after a cut-off has room to finish.
    assert_eq!(second["max_tokens"], 64_000);
    Ok(())
}

#[test]
fn pages_are_counted_for_the_estimate() -> Fallible {
    let dir = tempfile::tempdir()?;
    let Datasheet::File(path) = datasheet(&dir)? else {
        return Err("a file".into());
    };
    assert_eq!(
        pdf_pages(&std::fs::read(&path)?),
        2,
        "two pages, not the page tree"
    );
    let prepared = documents::prepare(
        &[Datasheet::File(path)],
        &shipped_config()?.profile.capabilities,
    )?;
    assert_eq!(prepared.tokens, 2 * TOKENS_PER_PAGE);
    Ok(())
}

// The surface: the run's tools are MCP's, and its reach profile holds.

#[test]
fn the_run_s_tool_definitions_are_mcp_s_bytes() -> Fallible {
    let served = sparksmith_automation::mcp::tools::tools();
    let offered = verbs::tools();
    assert_eq!(served.len(), offered.len());
    for (served, offered) in served.iter().zip(&offered) {
        // MCP's tools/list entry and the run's tool, each as its wire
        // writes it: the same name, description and input schema bytes.
        let mcp = serde_json::to_value(served)?;
        assert_eq!(mcp["name"].as_str(), Some(offered.name.as_str()));
        assert_eq!(
            serde_json::to_string(&mcp["description"])?,
            serde_json::to_string(&offered.description)?
        );
        assert_eq!(
            serde_json::to_string(&mcp["inputSchema"])?,
            serde_json::to_string(&offered.input_schema)?
        );
    }
    // And the Messages body carries them unchanged.
    let body = crate::model::anthropic::request_body(
        &shipped_config()?.profile,
        &crate::model::ModelRequest {
            system: "s",
            tools: &offered,
            messages: &[],
            max_output_tokens: 1,
            timeout: std::time::Duration::from_secs(1),
        },
    );
    for (sent, served) in body["tools"].as_array().into_iter().flatten().zip(&served) {
        let mcp = serde_json::to_value(served)?;
        assert_eq!(
            serde_json::to_string(&sent["input_schema"])?,
            serde_json::to_string(&mcp["inputSchema"])?
        );
        assert_eq!(sent["description"], mcp["description"]);
    }
    Ok(())
}

#[test]
fn the_datasheet_run_cannot_reach_publish_files_or_settings() -> Fallible {
    use sparksmith_core::permission::Reach;
    let reach = reach()?;
    let grant = reach.grant();
    // As data: no files module, no settings module, the library below Ask.
    assert!(!grant.modules.contains_key("files"));
    assert!(!grant.modules.contains_key("settings"));
    assert!(
        grant
            .modules
            .get("library")
            .is_some_and(|c| *c < sparksmith_core::Class::Ask)
    );
    assert!(reach.reach.is_empty());
    // As calls: each is refused, and nothing changes.
    let dir = tempfile::tempdir()?;
    let target = dir.path().join("part.sparksmith");
    let mut session = profiled_session();
    let before = session.engine().revision();
    for (tool, input, code) in [
        (
            "act",
            json!({"name": "library.publish", "params": {"drafts": []}, "expected_revision": before}),
            "gate.module",
        ),
        (
            "act",
            json!({"name": "project.save", "params": {"path": target.to_string_lossy()}}),
            "gate.module",
        ),
        (
            "act",
            json!({"name": "project.open", "params": {"path": target.to_string_lossy()}}),
            "gate.module",
        ),
        (
            // A path makes any call Ask, above every module's ceiling here.
            "act",
            json!({"name": "library.check_part_file", "params": {"path": target.to_string_lossy()}}),
            "gate.module",
        ),
        (
            "act",
            json!({"name": "library.publish_to_folder",
                "params": {"library": "0123456789abcdef0123456789abcdef", "root": {"kind": "component", "id": 1, "revision": 1}}, "expected_revision": before}),
            "gate.reach",
        ),
        ("query", query("library.sources", json!({})), "gate.reach"),
        ("query", query("library.rescan", json!({})), "gate.reach"),
    ] {
        let reply = verb_as(&mut session, &reach, tool, input.clone());
        assert_eq!(reply["ok"], false, "{input}: {reply}");
        let got = reply["error"]["code"].as_str().unwrap_or_default();
        // A call the catalog refuses for its shape never reaches the gate;
        // these are well formed, so the profile or the gate decides.
        assert_eq!(got, code, "{input}: {reply}");
    }
    assert!(!target.exists());
    assert_eq!(session.engine().revision(), before);
    // Over the whole catalog: no entry that publishes or touches files or
    // settings is reachable, and find lists none of them.
    let catalog = sparksmith_automation::session::metadata::entries();
    let mut reachable = 0;
    for entry in catalog.iter().filter(|e| e.kind.is_some()) {
        let declared = sparksmith_automation::session::metadata::declared_reach(&entry.name);
        let outside = declared.contains(Reach::Files)
            || declared.contains(Reach::Settings)
            || entry.module == "files"
            || entry.module == "settings"
            || (entry.name.contains("publish") && entry.class >= sparksmith_core::Class::Ask);
        if agent_reaches(&grant, entry) {
            reachable += 1;
            assert!(!outside, "{} is reachable", entry.name);
        }
    }
    assert!(
        reachable > 20,
        "the run still reaches its work: {reachable}"
    );
    let found = verb_as(
        &mut session,
        &reach,
        "find",
        json!({"text": "publish save open folder settings", "limit": 50}),
    );
    let hits = found["result"]["hits"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for hit in &hits {
        let name = hit["name"].as_str().unwrap_or_default();
        let entry = sparksmith_automation::session::metadata::find(name).ok_or("an entry")?;
        assert!(agent_reaches(&grant, entry), "{name}");
        assert!(
            name != "library.publish" && name != "project.save",
            "{name}"
        );
    }
    Ok(())
}

/// Whether a run under `grant` reaches `entry`, as `find` filters.
fn agent_reaches(
    grant: &sparksmith_automation::Grant,
    entry: &sparksmith_automation::session::metadata::EntryMeta,
) -> bool {
    use sparksmith_automation::mcp::desk::Calls;
    let mut session = Session::default();
    Agent {
        session: &mut session,
        grant,
        turn: None,
    }
    .reaches(entry)
}

#[test]
fn a_gated_tool_call_comes_back_as_a_tool_result_and_the_run_goes_on() -> Fallible {
    // Under a profile that only reads the library, applying the pinout is
    // beyond it; the gate refuses, and the model reads why.
    let dir = tempfile::tempdir()?;
    let mut session = profiled_session();
    let revision = session.engine().revision();
    // The whole document in one call: 23 rows, under the 40-row limit.
    let script = vec![
        tool_turn("t1", "query", query("assist.pinout_edit", pinout())),
        tool_turn(
            "t2",
            "query",
            query(
                "assist.pinout_preview",
                json!({"target": {"target": "new", "name": "EXR-22"}}),
            ),
        ),
        tool_turn(
            "t3",
            "apply",
            apply(revision, "assist.pinout_apply", json!({})),
        ),
        tool_turn(
            "t4",
            "query",
            query(
                "library.review_check",
                json!({"report": {
            "part": "EXR-22", "summary": "Not applied: the gate refused it.",
            "sources": [{"name": "DS", "title": "EXR-22 datasheet", "revision": "1"}],
            "values": [], "open_questions": ["Apply the pinout by hand."]}}),
            ),
        ),
    ];
    let mut backend = anthropic(script)?;
    let mut config = shipped_config()?;
    config
        .reach
        .modules
        .insert("library".into(), sparksmith_core::Class::Read);
    let (outcome, log) = run_with(&config, &mut backend, datasheet(&dir)?, &mut session)?;
    // The run went on to its report, which the review check refuses: with
    // nothing applied there is no component draft. The script ends there.
    assert_eq!(
        outcome.stop,
        Stop::Failed("HTTP: the recording has no more responses".into()),
        "{log:?}"
    );
    assert_eq!(
        outcome.facts.request_log[2].tools,
        vec!["apply assist.pinout_apply!"]
    );
    assert_eq!(
        outcome.facts.request_log[3].tools,
        vec!["query library.review_check!"]
    );
    assert!(
        session.engine().drafts().items.is_empty(),
        "nothing applied"
    );
    let bodies = bodies(&backend);
    let refused = &bodies[3]["messages"][6]["content"][0];
    assert_eq!(refused["type"], "tool_result");
    assert_eq!(refused["is_error"], true);
    let text = refused["content"].as_str().unwrap_or_default();
    assert!(
        text.contains("gate.module") && text.contains("library module only up to read"),
        "{text}"
    );
    let report = &bodies[4]["messages"][8]["content"][0];
    assert_eq!(report["is_error"], true);
    let text = report["content"].as_str().unwrap_or_default();
    assert!(text.contains("no component draft"), "{text}");
    Ok(())
}

#[test]
fn the_pinout_applies_without_a_base_or_with_it_as_text() -> Fallible {
    // A local model could not copy a 19-digit base: it wrote it as a float.
    // The base is optional, text is read, and a float is named as the cause.
    for (base, applies) in [(None, true), (Some("text"), true), (Some("float"), false)] {
        let mut session = profiled_session();
        verb(&mut session, "query", query("assist.pinout_edit", pinout()))?;
        let plan = verb(
            &mut session,
            "query",
            query(
                "assist.pinout_preview",
                json!({"target": {"target": "new", "name": "EXR-22"}}),
            ),
        )?;
        // The base is written as text, which a double cannot round.
        let exact = plan["base"].as_str().ok_or("a base")?.parse::<u64>()?;
        let params = match base {
            None => json!({}),
            Some("text") => json!({"base": exact.to_string()}),
            _ => json!({"base": exact as f64}),
        };
        let revision = session.engine().revision();
        let reply = verb_as(
            &mut session,
            &reach()?,
            "apply",
            apply(revision, "assist.pinout_apply", params),
        );
        assert_eq!(reply["ok"] == true, applies, "{base:?}: {reply}");
        if !applies && exact > (1u64 << 53) {
            assert!(reply.to_string().contains("Leave base out"), "{reply}");
        }
    }
    Ok(())
}

#[test]
fn non_pdf_files_and_unnamed_models_are_refused() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("x.pdf");
    std::fs::write(&path, b"not a pdf")?;
    let caps = shipped_config()?.profile.capabilities;
    assert!(documents::prepare(&[Datasheet::File(path)], &caps).is_err());
    assert!(documents::prepare(&[Datasheet::Url("http://x/y.pdf".into())], &caps).is_err());
    let config = RunConfig {
        profile: ModelProfile::anthropic(" "),
        ..shipped_config()?
    };
    let result = run(
        &goal(datasheet(&dir)?),
        &config,
        &mut anthropic(vec![])?,
        &mut profiled_session(),
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert!(matches!(result, Err(Error::Config(_))));
    Ok(())
}

// The same run on an OpenAI-compatible endpoint: a local model, its
// reasoning, a malformed call, extracted text and a rendered page.

/// A chat completion carrying one tool call, as Ollama sends it.
fn chat_call(id: &str, name: &str, arguments: &str) -> Value {
    json!({"id": "chatcmpl-1", "object": "chat.completion", "model": "local-27b",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": "", "reasoning": format!("I should call {name}."),
            "tool_calls": [{"id": id, "type": "function",
                "function": {"name": name, "arguments": arguments}}]}}],
        "usage": {"prompt_tokens": 20_000, "completion_tokens": 800}})
}

/// The Messages script, as chat completions.
fn chat_script(responses: &[Value]) -> Vec<Value> {
    responses
        .iter()
        .flat_map(|r| r["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == "tool_use")
        .map(|b| {
            chat_call(
                b["id"].as_str().unwrap_or_default(),
                b["name"].as_str().unwrap_or_default(),
                &b["input"].to_string(),
            )
        })
        .collect()
}

fn local_config(context: u64) -> Fallible<RunConfig> {
    let mut profile = ModelProfile::openai_compatible("local-27b", context);
    profile.capabilities.vision = true;
    Ok(RunConfig {
        profile,
        ..shipped_config()?
    })
}

fn openai(responses: Vec<Value>, config: &RunConfig) -> OpenAiBackend<ScriptedWire> {
    OpenAiBackend::new(config.profile.clone(), ScriptedWire::new(responses))
}

/// A text datasheet: a pin table on page 1 and a drawing on page 2.
fn text_datasheet(dir: &tempfile::TempDir) -> Fallible<Datasheet> {
    let path = dir.path().join("exr22.pdf");
    let table = format!(
        "EXR-22 PIN TABLE {}",
        "PIN 1 GND PIN 2 VCC PIN 3 IO0 ".repeat(12)
    );
    std::fs::write(&path, documents::tests::text_pdf(&[&table, ""]))?;
    Ok(Datasheet::File(path))
}

#[test]
fn a_local_model_authors_the_module_from_text_and_a_page_view() -> Fallible {
    let dir = tempfile::tempdir()?;
    let config = local_config(131_072)?;
    let mut replies = vec![
        // A look at the drawing page first, then one malformed call that the
        // model sends again.
        chat_call(
            "v1",
            "query",
            r#"{"name": "assist.document_page", "params": {"document": "exr22.pdf", "page": 2}}"#,
        ),
        chat_call("bad", "query", r#"{"oops": "#),
    ];
    replies.extend(chat_script(&script()?));
    let mut backend = openai(replies, &config);
    let mut session = profiled_session();
    let (outcome, log) = run_with(&config, &mut backend, text_datasheet(&dir)?, &mut session)?;
    assert_eq!(outcome.stop, Stop::Submitted, "{log:?}");
    assert_eq!(outcome.report.ok_or("a report")?.part, "EXR-22");
    let f = &outcome.facts;
    assert_eq!(f.requests, 12);
    assert_eq!(f.request_log[0].tools, vec!["query assist.document_page"]);
    assert_eq!(f.request_log[1].tools, vec!["query!"]);
    assert_eq!(f.output_tokens, 12 * 800);
    assert_eq!(f.documents[0].delivery, Delivery::TextAndPageViews);
    assert_eq!(f.documents[0].pages_viewed, vec![2]);
    assert_eq!(f.pins.len(), 23);
    assert!(f.provider.contains("OpenAI"));
    let md = report::markdown(None, f);
    assert!(
        md.contains("pages rendered as images when the model asked"),
        "{md}"
    );
    assert!(md.contains("it viewed pages [2]"), "{md}");
    assert!(!md.contains("spend") && !md.contains('$'), "{md}");

    let posts = &backend.wire.posts;
    let first = &posts[0].2;
    assert_eq!(posts[0].0, "/chat/completions");
    let opening = first["messages"][1]["content"]
        .as_array()
        .ok_or("the first user message")?;
    let datasheet_text = opening[0]["text"].as_str().unwrap_or_default();
    assert!(datasheet_text.contains("=== page 1 ===") && datasheet_text.contains("PIN 2 VCC"));
    let instructions = opening[1]["text"].as_str().unwrap_or_default();
    assert!(
        instructions.contains("assist.document_page"),
        "{instructions}"
    );
    let tools: Vec<&str> = first["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();
    assert_eq!(tools, verbs::NAMES);
    // The output limit is the model's, near what its context allows.
    assert_eq!(first["max_tokens"], 32_000);
    // The page went back as an image after its tool message.
    let second = posts[1].2["messages"].as_array().ok_or("messages")?;
    let n = second.len();
    assert_eq!(second[n - 2]["role"], "tool");
    assert!(
        second[n - 1]["content"][1]["image_url"]["url"]
            .as_str()
            .is_some_and(|u| u.starts_with("data:image/png;base64,"))
    );
    // The malformed call was answered with an error and never run; its
    // arguments went back as an empty object.
    let third = posts[2].2["messages"].as_array().ok_or("messages")?;
    let tool_reply = third.last().ok_or("a tool reply")?;
    assert_eq!(tool_reply["role"], "tool");
    assert!(
        tool_reply["content"].as_str().is_some_and(|c| c
            .starts_with("Error: This call was not run")
            && c.contains("not valid JSON")),
        "{tool_reply}"
    );
    assert_eq!(
        third[third.len() - 2]["tool_calls"][0]["function"]["arguments"],
        "{}"
    );
    // Reasoning never goes back to the server, and the history only grows.
    assert!(
        !posts
            .iter()
            .any(|p| p.2.to_string().contains("I should call"))
    );
    for pair in posts.windows(2) {
        let (a, b) = (
            pair[0].2["messages"].as_array().ok_or("messages")?,
            pair[1].2["messages"].as_array().ok_or("messages")?,
        );
        assert_eq!(&b[..a.len()], &a[..]);
    }
    Ok(())
}

fn cut_reply() -> Value {
    json!({"model": "local-27b", "choices": [{"finish_reason": "length", "message": {
        "role": "assistant", "content": "", "reasoning": "Thinking about pin 1...",
        "tool_calls": [{"id": "c", "type": "function",
            "function": {"name": "query", "arguments": "{\"name\": \"assist.pinout_edit\", \"params\": {\"pins\": [{\"row\""}}]}}],
        "usage": {"prompt_tokens": 12_000, "completion_tokens": 8_192}})
}

#[test]
fn replies_cut_off_at_the_limit_are_reported_never_used() -> Fallible {
    let dir = tempfile::tempdir()?;
    // A small context: 16 384 output tokens a request, 24 576 after a cut.
    let config = local_config(32_768)?;
    let mut backend = openai(vec![cut_reply(), cut_reply(), cut_reply()], &config);
    let mut session = profiled_session();
    let before = session.engine().revision();
    let (outcome, log) = run_with(&config, &mut backend, text_datasheet(&dir)?, &mut session)?;
    let Stop::CutOff(why) = &outcome.stop else {
        return Err(format!("{:?}", outcome.stop).into());
    };
    assert!(why.contains("3 replies in a row"), "{why}");
    assert!(outcome.report.is_none());
    assert_eq!(session.engine().revision(), before, "nothing ran");
    assert!(
        log.iter()
            .filter(|e| matches!(
                e,
                Event::Call {
                    state: CallState::NotRun { .. },
                    ..
                }
            ))
            .count()
            == 2,
        "the first two cut-off calls were answered with errors; the third ended the run"
    );
    assert_eq!(outcome.facts.request_log[0].stop_reason, "cut_off");
    // Never more than the context leaves: the limit after a cut is clamped
    // to what fits beside the prompt.
    let limits: Vec<u64> = backend
        .wire
        .posts
        .iter()
        .map(|p| p.2["max_tokens"].as_u64().unwrap_or(0))
        .collect();
    assert_eq!(limits[0], 16_384, "{limits:?}");
    assert!(limits[1] > limits[0], "more room after a cut: {limits:?}");
    // 12 000 prompt tokens and more: never the full 24 576 beside them.
    assert!(
        limits[1..].iter().all(|l| *l + 12_000 <= 32_768),
        "{limits:?}"
    );
    Ok(())
}

#[test]
fn a_prompt_too_large_for_the_context_says_why_before_sending() -> Fallible {
    let dir = tempfile::tempdir()?;
    let config = local_config(8_000)?;
    let mut backend = openai(vec![], &config);
    let result = run_with(
        &config,
        &mut backend,
        text_datasheet(&dir)?,
        &mut profiled_session(),
    );
    let Err(error) = result else {
        return Err("expected a refusal".into());
    };
    let text = error.to_string();
    assert!(text.contains("does not fit the 8000 tokens"), "{text}");
    assert!(backend.wire.posts.is_empty());
    Ok(())
}

#[test]
fn token_and_time_caps_bind_a_local_model() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut config = local_config(131_072)?;
    // The first request alone could take 32 000 output tokens.
    config.budget.max_total_tokens = 30_000;
    let mut backend = openai(chat_script(&script()?), &config);
    let (outcome, _) = run_with(
        &config,
        &mut backend,
        text_datasheet(&dir)?,
        &mut profiled_session(),
    )?;
    assert!(
        matches!(&outcome.stop, Stop::Budget(why) if why.contains("30000 tokens")),
        "{:?}",
        outcome.stop
    );
    assert!(backend.wire.posts.is_empty());

    let mut config = local_config(131_072)?;
    config.budget.max_duration = std::time::Duration::ZERO;
    let mut backend = openai(chat_script(&script()?), &config);
    let (outcome, _) = run_with(
        &config,
        &mut backend,
        text_datasheet(&dir)?,
        &mut profiled_session(),
    )?;
    assert!(
        matches!(&outcome.stop, Stop::Budget(why) if why.contains("seconds")),
        "{:?}",
        outcome.stop
    );
    // Each request waits no longer than the run has left.
    let mut config = local_config(131_072)?;
    config.budget.max_duration = std::time::Duration::from_secs(600);
    let mut backend = openai(chat_script(&script()?), &config);
    run_with(
        &config,
        &mut backend,
        text_datasheet(&dir)?,
        &mut profiled_session(),
    )?;
    assert!(
        backend
            .wire
            .posts
            .iter()
            .all(|p| p.3 <= std::time::Duration::from_secs(600))
    );
    Ok(())
}

#[test]
fn a_model_declared_without_tools_says_why() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut config = local_config(131_072)?;
    config.profile.capabilities.tool_calling = false;
    let result = run_with(
        &config,
        &mut openai(vec![], &config),
        text_datasheet(&dir)?,
        &mut profiled_session(),
    );
    assert!(
        matches!(&result, Err(e) if e.to_string().contains("not possible with this model") && e.to_string().contains("tool calling")),
        "{:?}",
        result.err()
    );
    Ok(())
}

#[test]
fn a_shrinking_prompt_count_warns_of_truncation() -> Fallible {
    let dir = tempfile::tempdir()?;
    let config = local_config(131_072)?;
    let mut replies = chat_script(&script()?);
    // The server suddenly counts far fewer prompt tokens.
    replies[2]["usage"]["prompt_tokens"] = json!(2_000);
    let mut backend = openai(replies, &config);
    let (outcome, log) = run_with(
        &config,
        &mut backend,
        text_datasheet(&dir)?,
        &mut profiled_session(),
    )?;
    assert_eq!(
        outcome.facts.warnings.len(),
        1,
        "{:?}",
        outcome.facts.warnings
    );
    assert!(outcome.facts.warnings[0].contains("Request 3"));
    assert!(log.iter().any(|e| matches!(e, Event::Warning(_))));
    let md = report::markdown(None, &outcome.facts);
    assert!(md.contains("**Warning:** Request 3"), "{md}");
    Ok(())
}

mod board;
mod flash_next;
mod local_run;
mod profiles;
mod run1;
mod streaming;
mod workflow_run;
mod wroom3_binding;

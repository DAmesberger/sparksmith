//! Slice D3 (GP7) of the part-generation plan: the recorded WROOM-3 and
//! TLV757 runs, made the way the datasheet skill now says (find a
//! generator, give the drawing's values, generate, draft the working set,
//! import the pinout onto it), equal the parts the recordings build today
//! with the package templates, under the plan's parity bars.
//!
//! Today's part: every recorded tool call run through the runner's own
//! tools on a fresh project, as a replay runs them. The generator flow:
//! the session requests the MCP verbs carry (`library.generate_part`,
//! `library.draft_closure`, `library.pinout_preview`, `library.pinout_apply`,
//! `library.set_marking`), with the values the recording gave its package
//! request turned into the generator's parameters as the skill says, the
//! pinout document the recording applied and the marking it set.
//!
//! Compared: every draft but the package's form and model source equal,
//! local ids aside; pads and drawings to the nanometre; the 3D model by the
//! bar of `part_file_tests.rs` (body count, bounds within 1 µm, marking
//! surfaces, every lead body bound to the same terminal). Accepted, as in
//! every phase C slice: the generator stores no footprint `polarity`
//! record (GP1; the pin-1 dot itself is drawn), and draws the courtyard as
//! a four-point polygon where the template drew the same rectangle. The
//! package of a generated part is generic with a part-file model where
//! the template's carried its form; that is the point of the change.
use super::*;
use crate::model::{ModelReply, Part};
use sparksmith_core::library::{Definition, PackageDefinition};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn example(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(path)
}

fn shipped() -> Fallible<sparksmith_io::shipped::ShippedLibrary> {
    Ok(sparksmith_io::shipped::ShippedLibrary::at(&PathBuf::from(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"),
    ))?)
}

fn session() -> Fallible<Session> {
    let mut session = profiled_session();
    session.set_shipped(shipped()?);
    Ok(session)
}

/// The recorded tool calls, in order, without the ones the runner never
/// ran (malformed input).
fn calls(path: &str) -> Fallible<Vec<(String, Value)>> {
    let text = std::fs::read_to_string(example(path))?;
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let reply: ModelReply = serde_json::from_str(line)?;
        for part in reply.content {
            if let Part::ToolCall {
                name,
                input,
                malformed: None,
                ..
            } = part
            {
                out.push((name, input));
            }
        }
    }
    Ok(out)
}

/// What a recording built and gave.
struct Recorded {
    session: Session,
    /// The pinout document it applied.
    document: Value,
    /// The values of its last package request.
    values: Value,
    /// The markings it set.
    markings: Vec<Value>,
}

/// Today's part: the recording's tool calls run through the runner's tools.
fn today(path: &str) -> Fallible<Recorded> {
    let mut session = session()?;
    let mut workspace = tools::Workspace::default();
    let (mut values, mut markings) = (Value::Null, Vec::new());
    for (name, input) in calls(path)? {
        if name == "submit_review" {
            continue;
        }
        let outcome = tools::execute(
            &mut session,
            &Grant::default(),
            &mut workspace,
            &name,
            &input,
        );
        if !matches!(outcome, ToolOutcome::Reply { error: false, .. }) {
            continue;
        }
        match name.as_str() {
            "library_package_from_template" => values = input["request"]["values"].clone(),
            "library_set_marking" => markings.push(input["marking"].clone()),
            _ => {}
        }
    }
    let document = workspace.applied.ok_or("the recording applied a pinout")?;
    Ok(Recorded {
        session,
        document,
        values,
        markings,
    })
}

/// One request through the gate, as the verbs send it; its result.
fn call(session: &mut Session, request: Value) -> Fallible<Value> {
    let response = tools::send(session, &Grant::default(), request.clone());
    if response["ok"] != true {
        return Err(format!("{request}: {}", response["error"]).into());
    }
    let result = &response["result"];
    Ok(result.get("result").unwrap_or(result).clone())
}

fn library_op(session: &Session, command: &str, params: Value) -> Value {
    json!({"command": "project.apply", "params": {
        "expected_revision": session.engine().revision(),
        "operation": {"editor": "Library", "operation": {"command": command, "params": params}},
    }})
}

fn draft_of(session: &Session, kind: &str) -> Fallible<u64> {
    session
        .engine()
        .drafts()
        .items
        .iter()
        .find(|d| serde_json::to_value(d.definition.kind()).is_ok_and(|k| k == kind))
        .map(|d| d.id.0)
        .ok_or_else(|| format!("a {kind} draft").into())
}

/// The skill's generator flow (steps 3–6) on a fresh project.
fn generated(generator: &str, params: Value, name: &str, recorded: &Recorded) -> Fallible<Session> {
    let mut session = session()?;
    let revision = session.engine().revision();
    let made = call(
        &mut session,
        json!({"command": "library.generate_part", "params": {
            "expected_revision": revision,
            "generator": generator, "scope": "component", "params": params, "name": name}}),
    )?;
    let root = &made["outcome"]["report"]["root"];
    assert_eq!(root["kind"], "component", "{made}");
    let request = library_op(
        &session,
        "library.draft_closure",
        json!({"component": {"id": root["id"], "revision": root["revision"]}}),
    );
    call(&mut session, request)?;
    let component = draft_of(&session, "component")?;
    let target = json!({"target": "draft", "id": component});
    let plan = call(
        &mut session,
        json!({"command": "project.query", "params": {"query": "library.pinout_preview",
            "params": {"target": target, "document": recorded.document}}}),
    )?;
    // Rows keep the generated terminals by number: none added or removed.
    for change in plan["changes"].as_array().into_iter().flatten() {
        assert!(
            change["object"] != "terminal" || change["kind"] == "changed",
            "a terminal added or removed: {change}"
        );
    }
    let request = library_op(
        &session,
        "library.pinout_apply",
        json!({"target": target, "document": recorded.document, "base": plan["base"],
            "accept": {"accept": "all"}}),
    );
    call(&mut session, request)?;
    for marking in &recorded.markings {
        let revision = session.engine().revision();
        call(
            &mut session,
            json!({"command": "library.set_marking", "params": {
                "expected_revision": revision, "draft": component, "marking": marking}}),
        )?;
    }
    Ok(session)
}

/// Millimetres from a recorded value as integer nanometres.
fn nm(v: &Value) -> i64 {
    (v.as_f64().unwrap_or(f64::NAN) * 1e6).round() as i64
}

fn nm_pair(v: &Value) -> [i64; 2] {
    [nm(&v[0]), nm(&v[1])]
}

/// The module form's values (the custom module entry of
/// `library.package_from_template`) as the module generator's parameters,
/// as the skill's Module packages section names them.
fn module_params(v: &Value) -> Value {
    let rows = v["rows"].as_array().cloned().unwrap_or_default();
    let singles = v["singles"].as_array().cloned().unwrap_or_default();
    let list = |items: &[Value], f: &dyn Fn(&Value) -> Value| -> Value {
        Value::Array(items.iter().map(f).collect())
    };
    let [can_width, can_length] = nm_pair(&v["can_size"]);
    let [can_x, can_y] = nm_pair(&v["can_centre"]);
    let [pad_length, pad_width] = nm_pair(&v["pad_size"]);
    let [land_outer, land_inner] = nm_pair(&v["land_extend"]);
    let mut p = json!({
        "width": nm(&v["width"]), "length": nm(&v["length"]), "height": nm(&v["height"]),
        "board_thickness": nm(&v["pcb_thickness"]), "antenna_length": nm(&v["antenna_length"]),
        "can_width": can_width, "can_length": can_length, "can_x": can_x, "can_y": can_y,
        "pitch": nm(&v["pitch"]), "pad_length": pad_length, "pad_width": pad_width,
        "castellation": nm(&v["castellation"]), "land_width": nm(&v["land_width"]),
        "land_outer": land_outer, "land_inner": land_inner,
        "row_sides": list(&rows, &|r| r["side"].clone()),
        "row_counts": list(&rows, &|r| r["count"].clone()),
        "row_x": list(&rows, &|r| json!(nm(&r["first"][0]))),
        "row_y": list(&rows, &|r| json!(nm(&r["first"][1]))),
        "ground": v["ground"].is_object(),
        "single_numbers": list(&singles, &|s| s["number"].clone()),
        "single_x": list(&singles, &|s| json!(nm(&s["centre"][0]))),
        "single_y": list(&singles, &|s| json!(nm(&s["centre"][1]))),
        "single_length": list(&singles, &|s| json!(nm(&s["size"][0]))),
        "single_width": list(&singles, &|s| json!(nm(&s["size"][1]))),
        "keepout": v["keepout"].is_object(),
    });
    let ground = &v["ground"];
    if ground.is_object() {
        let [pitch_x, pitch_y] = nm_pair(&ground["pitch"]);
        let [size_x, size_y] = nm_pair(&ground["size"]);
        let [x, y] = nm_pair(&ground["centre"]);
        let turn = (ground["rotation_deg"].as_f64().unwrap_or(0.0) * 1000.0).round() as i64;
        for (key, value) in [
            ("ground_rows", ground["rows"].clone()),
            ("ground_columns", ground["cols"].clone()),
            ("ground_pitch_x", json!(pitch_x)),
            ("ground_pitch_y", json!(pitch_y)),
            ("ground_size_x", json!(size_x)),
            ("ground_size_y", json!(size_y)),
            ("ground_x", json!(x)),
            ("ground_y", json!(y)),
            ("ground_rotation", json!(turn)),
            ("ground_number", ground["number"].clone()),
        ] {
            p[key] = value;
        }
    }
    if v["keepout"].is_object() {
        let beyond = &v["keepout"]["beyond"];
        p["keepout_left"] = json!(nm(&beyond[0]));
        p["keepout_right"] = json!(nm(&beyond[1]));
        p["keepout_top"] = json!(nm(&beyond[2]));
    }
    p
}

/// The tabbed form's values as the tabbed generator's parameters; the
/// datasheet's land pattern as `lands: explicit`.
fn tabbed_params(v: &Value) -> Value {
    let (leads, step) = (&v["leads_per_side"], &v["lead_step"]);
    let mut p = json!({
        "leads_s": leads[0], "leads_n": leads[1],
        "step_s": step[0].as_u64().unwrap_or(1), "step_n": step[1].as_u64().unwrap_or(1),
        "pitch": nm(&v["pitch"]), "body_length": nm(&v["body_length"]),
        "body_width": nm(&v["body_width"]), "body_height": nm(&v["body_height"]),
        "standoff": nm(&v["standoff"]), "span": nm(&v["span"]),
        "lead_width": nm(&v["lead_width"]), "lead_thickness": nm(&v["lead_thickness"]),
        "foot_length": nm(&v["foot_length"]), "tab": "none",
    });
    if let Some(lands) = v.get("lands").filter(|l| l.is_object()) {
        let [width, length] = nm_pair(&lands["size"]);
        p["lands"] = json!("explicit");
        p["land_width"] = json!(width);
        p["land_length"] = json!(length);
        p["land_span"] = json!(nm(&lands["span"]));
    }
    p
}

/// The one draft of `kind`, as JSON.
fn body(session: &Session, kind: &str) -> Fallible<Value> {
    let id = draft_of(session, kind)?;
    let draft = session
        .engine()
        .drafts()
        .items
        .iter()
        .find(|d| d.id.0 == id)
        .ok_or("the draft")?;
    Ok(serde_json::to_value(&draft.definition)?["body"].clone())
}

/// Local ids (terminals, functions, resources and roles, pads, drawings)
/// renamed in order of first appearance, and references to other
/// definitions dropped: two drafts built the same way compare equal
/// whatever ids their lineages took.
fn canonical(value: &Value) -> Value {
    fn walk(value: &Value, key: &str, ids: &mut BTreeMap<u64, u64>) -> Value {
        const LOCAL: &[&str] = &[
            "id",
            "design_default",
            "resource",
            "role",
            "unreviewed",
            "terminal",
            "markers",
            "pad",
        ];
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(k, _)| {
                        !matches!(
                            k.as_str(),
                            "component"
                                | "package"
                                | "representations"
                                | "implementations"
                                | "next_local"
                        )
                    })
                    .map(|(k, v)| (k.clone(), walk(v, k, ids)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(|v| walk(v, key, ids)).collect()),
            Value::Number(n) if LOCAL.contains(&key) => {
                let next = ids.len() as u64 + 1;
                json!(*ids.entry(n.as_u64().unwrap_or(u64::MAX)).or_insert(next))
            }
            other => other.clone(),
        }
    }
    walk(value, "", &mut BTreeMap::new())
}

/// Every number under `key` named by `names` (a reference into another
/// definition's ids: a component terminal, a package terminal).
fn resolve(value: &mut Value, key: &str, names: &BTreeMap<u64, String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == key
                    && let Some(name) = v.as_u64().and_then(|n| names.get(&n))
                {
                    *v = json!(name);
                } else {
                    resolve(v, key, names);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| resolve(v, key, names)),
        _ => {}
    }
}

/// The names references resolve to: each component terminal by its place
/// in the component, each package terminal by its number.
fn names(session: &Session) -> Fallible<(BTreeMap<u64, String>, BTreeMap<u64, String>)> {
    let terminals = body(session, "component")?["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(k, t)| Some((t["id"].as_u64()?, format!("terminal {k}"))))
        .collect();
    let numbers = body(session, "package")?["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some((
                t["id"].as_u64()?,
                format!("pad {}", t["designation"].as_str()?),
            ))
        })
        .collect();
    Ok((terminals, numbers))
}

/// A draft with its references into other drafts resolved, then
/// [`canonical`].
fn resolved(session: &Session, kind: &str) -> Fallible<Value> {
    let (terminals, numbers) = names(session)?;
    let mut value = body(session, kind)?;
    match kind {
        "symbol" => resolve(&mut value, "terminal", &terminals),
        "binding" => {
            resolve(&mut value, "terminal", &terminals);
            resolve(&mut value, "package_terminal", &numbers);
        }
        "footprint" => resolve(&mut value, "terminal", &numbers),
        _ => {}
    }
    Ok(canonical(&value))
}

/// A rectangle drawn as the four-point polygon the generator draws.
fn as_polygon(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(rect) = map.get("rect").filter(|r| r.get("a").is_some()) {
                let (a, b) = (&rect["a"], &rect["b"]);
                let point = |x: &Value, y: &Value| json!({"x_nm": x, "y_nm": y});
                let points = json!([
                    point(&a["x_nm"], &a["y_nm"]),
                    point(&b["x_nm"], &a["y_nm"]),
                    point(&b["x_nm"], &b["y_nm"]),
                    point(&a["x_nm"], &b["y_nm"]),
                ]);
                map.remove("rect");
                map.insert("polygon".into(), json!({"points": points}));
            }
            map.values_mut().for_each(as_polygon);
        }
        Value::Array(items) => items.iter_mut().for_each(as_polygon),
        _ => {}
    }
}

/// A footprint for comparison: its drawings in one order, rectangles as
/// polygons, without the polarity record.
fn footprint(session: &Session) -> Fallible<Value> {
    let mut f = resolved(session, "footprint")?;
    let polarity = f
        .as_object_mut()
        .and_then(|m| m.remove("polarity"))
        .unwrap_or(Value::Null);
    if let Some(graphics) = f["graphics"].as_array_mut() {
        for g in graphics.iter_mut() {
            if let Some(m) = g.as_object_mut() {
                m.remove("id");
            }
            as_polygon(g);
        }
        graphics.sort_by_key(|g| g.to_string());
    }
    f["had_polarity"] = json!(!polarity.is_null());
    Ok(f)
}

/// The 3D model as the bar compares it: body count, bounds, marking
/// surfaces and the terminal (by number) each lead body is bound to.
struct Model {
    bodies: u32,
    bounds_nm: [[i64; 3]; 2],
    surfaces: Vec<(String, Value)>,
    leads: Vec<String>,
}

fn model(session: &Session) -> Fallible<Model> {
    let draft = draft_of(session, "package")?;
    let package: PackageDefinition = session
        .engine()
        .drafts()
        .items
        .iter()
        .find(|d| d.id.0 == draft)
        .and_then(|d| match &d.definition {
            Definition::Package(p) => Some(p.clone()),
            _ => None,
        })
        .ok_or("the package draft")?;
    let number = |id: u64| -> String {
        package
            .terminals
            .iter()
            .find(|t| u64::from(t.id.0) == id)
            .map(|t| t.designation.clone())
            .unwrap_or_default()
    };
    let json = serde_json::to_value(&package.model)?;
    let derived = if json["source"] == "generated" {
        // The template's model: the family's generator through the bridge.
        let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
        let (part, bodies) = sparksmith_solidcraft::models::family::evaluate(&package)?;
        let mut d =
            serde_json::to_value(sparksmith_solidcraft::models::part_file::derived(&part)?)?;
        d["leads"] = Value::Array(
            bodies
                .iter()
                .filter_map(|b| b.terminal)
                .map(|t| json!({"terminal": number(u64::from(t))}))
                .collect(),
        );
        d
    } else {
        json["derived"].clone()
    };
    let mut leads: Vec<String> = derived["leads"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l["terminal"].as_str().map(str::to_owned))
        .collect();
    leads.sort();
    Ok(Model {
        bodies: u32::try_from(derived["bodies"].as_u64().unwrap_or(0))?,
        bounds_nm: serde_json::from_value(derived["bounds_nm"].clone())?,
        surfaces: derived["surfaces"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                (
                    s["name"].as_str().unwrap_or_default().to_owned(),
                    s["geometry"].clone(),
                )
            })
            .collect(),
        leads,
    })
}

/// Both geometries within 1 µm (the marking-surface bar is 1e-9 mm, which
/// integer nanometres hold exactly; directions within 1e-9).
fn near(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| near(v, w)))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| near(v, w))
        }
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap_or(f64::NAN) - y.as_f64().unwrap_or(f64::NAN)).abs() <= 1e-9
        }
        _ => a == b,
    }
}

/// The parity bars between today's drafts and the generated ones.
fn assert_parity(today: &Session, new: &Session) -> Fallible {
    // Symbol and binding: pins, names, numbers and drawings equal.
    for kind in ["component", "symbol", "binding"] {
        assert_eq!(resolved(today, kind)?, resolved(new, kind)?, "{kind}");
    }
    // Package: terminals, envelope and placement equal; the form and the
    // model source are what changed.
    let (old, now) = (body(today, "package")?, body(new, "package")?);
    for key in ["terminals", "envelope_nm", "placement"] {
        assert_eq!(canonical(&old[key]), canonical(&now[key]), "package {key}");
    }
    assert_eq!(now["form"]["family"], "generic");
    assert_eq!(now["model"]["source"], "part_file");
    // Footprint: pads, drawings, keep-outs, mask and paste rules to the
    // nanometre; the polarity record is the accepted difference.
    let (mut old, mut now) = (footprint(today)?, footprint(new)?);
    assert_eq!(old["had_polarity"], true);
    assert_eq!(now["had_polarity"], false);
    old["had_polarity"] = Value::Null;
    now["had_polarity"] = Value::Null;
    assert_eq!(old, now, "footprint");
    // 3D.
    let (old, now) = (model(today)?, model(new)?);
    assert_eq!(old.bodies, now.bodies, "bodies");
    for (a, b) in old
        .bounds_nm
        .iter()
        .flatten()
        .zip(now.bounds_nm.iter().flatten())
    {
        assert!(
            (a - b).abs() <= 1_000,
            "bounds {:?} {:?}",
            old.bounds_nm,
            now.bounds_nm
        );
    }
    assert_eq!(old.surfaces.len(), now.surfaces.len(), "surfaces");
    for ((name_a, a), (name_b, b)) in old.surfaces.iter().zip(&now.surfaces) {
        assert_eq!(name_a, name_b);
        assert!(near(a, b), "surface {name_a}: {a} {b}");
    }
    assert!(!now.leads.is_empty());
    assert_eq!(old.leads, now.leads, "lead bodies' terminals");
    Ok(())
}

#[test]
fn the_wroom3_reference_run_through_the_module_generator_is_todays_part() -> Fallible {
    let recorded = today("esp32-s31-wroom-3/wroom3.replay.jsonl")?;
    let new = generated(
        "module",
        module_params(&recorded.values),
        "ESP32-S31-WROOM-3",
        &recorded,
    )?;
    assert_parity(&recorded.session, &new)
}

#[test]
fn the_flash_next_tlv757_run_through_the_tabbed_generator_is_todays_part() -> Fallible {
    // The run gave TI's DBV0005A land pattern (0.6 × 1.1 mm, rows 2.6 mm
    // apart): the generator takes it as `lands: explicit`.
    let recorded = today("qwen3.8-flash-next/tlv757.recording.jsonl")?;
    let params = tabbed_params(&recorded.values);
    assert_eq!(params["lands"], "explicit");
    let new = generated("tabbed", params, "TLV757", &recorded)?;
    assert_parity(&recorded.session, &new)
}

#[test]
fn the_local_tlv757_run_through_the_tabbed_generator_is_todays_part() -> Fallible {
    // This run left the lands to the estimate.
    let recorded = today("tlv75733-local/tlv757.recording.jsonl")?;
    let new = generated(
        "tabbed",
        tabbed_params(&recorded.values),
        "TLV757",
        &recorded,
    )?;
    assert_parity(&recorded.session, &new)
}

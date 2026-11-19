//! The generators' marking rules as drawn: generic parts the shipped
//! modules make, placed on the demo board with their values, in the PCB 3D
//! view's model (the board built, each instance's marking laid out from its
//! fields and drawn as decals by `BoardMarkings`), close up.
//! `SPARKSMITH_PICTURES=dir cargo test -p sparksmith write_marking_rule_pictures -- --ignored`.
use super::*;
use serde_json::{Value, json};

/// One part: picture name, catalog words or a generator preset, the
/// tolerance to set, the manufacturer part number its Part gives, and the
/// close-up distance (mm).
struct Case {
    name: &'static str,
    words: Option<&'static str>,
    preset: Option<(&'static str, &'static str)>,
    tolerance: Option<&'static str>,
    mpn: Option<&'static str>,
    distance: f64,
}

const CASES: [Case; 9] = [
    Case {
        name: "resistor-0805-10k-1pct",
        words: Some("resistor 0805 10k"),
        preset: None,
        tolerance: Some("1%"),
        mpn: None,
        distance: 6.0,
    },
    Case {
        name: "resistor-0603-4k7",
        words: Some("resistor 0603 4.7k"),
        preset: None,
        tolerance: Some("5%"),
        mpn: None,
        distance: 5.0,
    },
    Case {
        name: "resistor-1206-4r7",
        words: Some("resistor 1206 4.7"),
        preset: None,
        tolerance: None,
        mpn: None,
        distance: 8.0,
    },
    Case {
        name: "led-0603",
        words: Some("led 0603 red"),
        preset: None,
        tolerance: None,
        mpn: None,
        distance: 5.0,
    },
    Case {
        name: "diode-sma",
        words: None,
        preset: Some(("molded", "molded-sma")),
        tolerance: None,
        mpn: Some("SS34"),
        distance: 12.0,
    },
    Case {
        name: "sot-23",
        words: None,
        preset: Some(("tabbed", "tabbed-sot-23-3")),
        tolerance: None,
        mpn: Some("BC847"),
        distance: 8.0,
    },
    Case {
        name: "soic-8",
        words: None,
        preset: Some(("gull-wing", "gull-wing-soic-8-3.9")),
        tolerance: None,
        mpn: Some("LM358"),
        distance: 13.0,
    },
    Case {
        name: "electrolytic-6.3x7.7",
        words: Some("electrolytic 6.3x7.7 100u 16V"),
        preset: None,
        tolerance: None,
        mpn: None,
        distance: 18.0,
    },
    Case {
        name: "radial-8x11.5",
        words: Some("radial electrolytic 8x11.5 470u 25V"),
        preset: None,
        tolerance: None,
        mpn: None,
        distance: 40.0,
    },
];

fn ok(reply: Value, what: &str) -> Result<Value, Box<dyn std::error::Error>> {
    if reply["ok"] == true {
        Ok(reply)
    } else {
        Err(format!("{what}: {reply}").into())
    }
}

fn apply(
    session: &mut sparksmith_automation::Session,
    editor: &str,
    operation: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let revision = session.engine().revision();
    ok(
        session.dispatch(json!({"command": "project.apply", "params": {
            "operation": {"editor": editor, "operation": operation},
            "expected_revision": revision}})),
        editor,
    )
}

/// Make, place on the board at `at` and give its values: the instance id.
fn place(
    session: &mut sparksmith_automation::Session,
    case: &Case,
    at: [f64; 2],
) -> Result<u64, Box<dyn std::error::Error>> {
    let search = |session: &mut sparksmith_automation::Session,
                  words: &str|
     -> Result<Value, Box<dyn std::error::Error>> {
        let found = ok(
            session.dispatch(json!({"command": "project.query", "params":
                {"query": "library.search", "params": {"query": words}}})),
            words,
        )?;
        Ok(found["result"]["generic"][0].clone())
    };
    let revision = session.engine().revision();
    let (component, properties, physical) = match (case.words, case.preset) {
        (Some(words), _) => {
            let hint = search(session, words)?;
            let hint = if hint.get("make").is_some() {
                let mut params = hint["make"]["params"].clone();
                params["expected_revision"] = json!(revision);
                ok(
                    session.dispatch(json!({"command": "library.generate_part", "params": params})),
                    words,
                )?;
                search(session, words)?
            } else {
                hint
            };
            (
                hint["component"].clone(),
                hint["properties"].clone(),
                hint["physical"].clone(),
            )
        }
        (None, Some((generator, preset))) => {
            let made = ok(
                session.dispatch(json!({"command": "library.generate_part", "params": {
                    "generator": generator, "scope": "component", "preset": preset,
                    "params": {}, "expected_revision": revision}})),
                preset,
            )?;
            let refs = &made["result"]["outcome"]["report"]["revisions"];
            let binding = refs
                .as_array()
                .and_then(|r| r.iter().find(|r| r["kind"] == "binding"))
                .cloned()
                .ok_or("no binding")?;
            let footprint = refs
                .as_array()
                .and_then(|r| r.iter().find(|r| r["kind"] == "footprint"))
                .cloned()
                .ok_or("no footprint")?;
            (
                json!({"id": refs[0]["id"], "revision": refs[0]["revision"]}),
                json!([]),
                json!({"binding": binding["id"],
                       "footprint": footprint["id"]}),
            )
        }
        _ => return Err("a case names words or a preset".into()),
    };
    let library = &session.engine().project().library;
    let reference = serde_json::from_value(component.clone())
        .map_err(|e| format!("{}: no component ({e}): {component}", case.name))?;
    let symbol = library
        .component(reference)
        .and_then(|c| c.definition.representations.first().map(|r| r.id))
        .ok_or("no symbol")?;
    let placed = apply(
        session,
        "Instance",
        json!({"command": "component.place", "params": {"component": component,
            "designator": case.name, "properties": properties, "representation": symbol,
            "unit": 1, "position": [at[0], at[1] + 200.0], "physical": physical}}),
    )?;
    let instance = placed["result"]["outcome"]["instance"]
        .as_u64()
        .ok_or("no instance")?;
    apply(
        session,
        "Layout",
        json!({"command": "layout.place", "params": {"instance": instance,
            "position": at, "rotation_deg": 0.0, "layer": "Top"}}),
    )?;
    if let Some(tolerance) = case.tolerance {
        let field = sparksmith_core::library::standards::get()?
            .fields
            .local(&session.engine().project().library, "Tolerance")
            .ok_or("no Tolerance field")?;
        apply(
            session,
            "Instance",
            json!({"command": "instance.set_properties", "params": {"instance": instance,
                "edits": [{"field": field.id, "input": {"input": "text", "value": tolerance}}]}}),
        )?;
    }
    Ok(instance)
}

#[test]
#[ignore = "writes marking rule pictures"]
fn write_marking_rule_pictures() -> Result<(), Box<dyn std::error::Error>> {
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let Some(dir) = std::env::var_os("SPARKSMITH_PICTURES") else {
        return Err("set SPARKSMITH_PICTURES".into());
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    let Some(mut gpu) = Gpu::new() else {
        return Err("no GPU adapter".into());
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let mut session = sparksmith_automation::Session::new(sparksmith_core::Engine::demo()?);
    session.set_shipped(sparksmith_io::shipped::ShippedLibrary::at(&root)?);
    // In a row above the demo board's parts, 15 mm apart.
    let origin = {
        let p = session.engine().project();
        let xs: Vec<f64> = p.placements.iter().map(|p| p.position[0]).collect();
        let ys: Vec<f64> = p.placements.iter().map(|p| p.position[1]).collect();
        [
            xs.iter().copied().fold(f64::INFINITY, f64::min),
            ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        ]
    };
    let mut placed = Vec::new();
    for (k, case) in CASES.iter().enumerate() {
        let at = [origin[0] + 15.0 * k as f64, origin[1] + 15.0];
        let instance = place(&mut session, case, at)?;
        placed.push((case, instance, at));
    }
    let mut project = session.engine().project().clone();
    project.fabrication = Fabrication::default();
    // A Part gives the ICs and the diode their part numbers (made here as
    // a published Part revision would be; the marking reads its MPN).
    for (case, instance, _) in &placed {
        let Some(mpn) = case.mpn else { continue };
        let id = sparksmith_core::ids::DefinitionId(project.allocate()?);
        let target = project
            .instances
            .iter()
            .find(|i| i.id == *instance)
            .ok_or("no instance")?;
        let component = target.component;
        let binding = project
            .library
            .component(component)
            .and_then(|c| c.definition.implementations.first().copied())
            .ok_or("no binding")?;
        project
            .library
            .parts
            .push(sparksmith_core::library::Published {
                id,
                revision: sparksmith_core::ids::Revision::FIRST,
                meta: sparksmith_core::library::DefinitionMeta {
                    name: mpn.into(),
                    ..Default::default()
                },
                definition: sparksmith_core::library::PartDefinition {
                    mpn: mpn.into(),
                    manufacturer: "Example".into(),
                    component,
                    binding,
                    ..Default::default()
                },
            });
        if let Some(i) = project.instances.iter_mut().find(|i| i.id == *instance) {
            i.part = Some(sparksmith_core::ids::PartRef::new(
                id,
                sparksmith_core::ids::Revision::FIRST,
            ));
        }
    }
    let model = sparksmith_solidcraft::project::build(&project)?;
    let mut geometry = model.geometry.clone();
    let decals = sparksmith_solidcraft::marking::board::BoardMarkings::default().decals(
        &project,
        &model.geometry,
        &model.sources,
    );
    eprintln!("{} decal meshes", decals.len());
    geometry.extend(decals);
    let mut v = Viewport::detached();
    v.set_model(crate::model_scene::build(geometry, &model.stack)?);
    v.fit_to(4.0 / 3.0);
    let (w, h) = (1280, 960);
    for (case, instance, at) in &placed {
        let laid = sparksmith_core::commands::instance_marking(&project, *instance)?;
        let texts: Vec<String> = laid
            .iter()
            .flat_map(|l| l.all())
            .flat_map(|l| &l.elements)
            .filter_map(|e| match e {
                sparksmith_core::library::marking::LaidElement::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        eprintln!("{}: {texts:?}", case.name);
        // A sleeve is seen from round about, at its middle's height.
        let views: &[(&str, f64, f64)] = if case.name.starts_with("radial") {
            &[
                ("top", 0.0, 89.0),
                ("side-0", 0.0, 15.0),
                ("side-90", 90.0, 15.0),
                ("side-180", 180.0, 15.0),
                ("side-270", 270.0, 15.0),
            ]
        } else {
            &[("top", 0.0, 89.0), ("angle", 30.0, 45.0)]
        };
        let height = if case.name.starts_with("radial") {
            6.0
        } else {
            1.0
        };
        for (view, yaw, pitch) in views.iter().copied() {
            aim(
                &mut v,
                Vec3::new(at[0], at[1], height),
                case.distance,
                yaw,
                pitch,
            );
            let rgba = gpu.draw(&mut v, w, h, 2.0).ok_or("no picture")?;
            let png = sparksmith_solidcraft::viewport::png(w as usize, h as usize, rgba)
                .ok_or("png encoding failed")?;
            std::fs::write(dir.join(format!("{}-{view}.png", case.name)), png)?;
        }
    }
    Ok(())
}

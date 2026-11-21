//! Fiducials through the session: the `fiducial` generator's part, placed
//! board-only, held to the JLCPCB profile's assembly rules (each value
//! sourced in `library/profiles/jlcpcb-2-layer.json`): too few fiducials on
//! a side with surface-mount parts, a symmetric set, silkscreen and another
//! part's courtyard in a clear area, and the clear area's copper keep-out
//! on the fiducial's own side only.
use super::super::Session;
use serde_json::{Value, json};
use sparksmith_core::Engine;
use sparksmith_core::Finding;
use sparksmith_core::layout::fiducial::{SILK, SYMMETRIC, TOO_FEW};
use sparksmith_core::model::CopperLayer;
use sparksmith_core::model::fill::KeepoutLayers;
use sparksmith_io::shipped::ShippedLibrary;
use std::path::Path;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The shipped library of the repository (every generator's bundle).
const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library");

fn call(session: &mut Session, command: &str, mut params: Value) -> TestResult<Value> {
    if let Some(map) = params.as_object_mut() {
        map.insert(
            "expected_revision".into(),
            json!(session.engine().revision()),
        );
    }
    let reply = session.dispatch(json!({"command": command, "params": params}));
    if reply["ok"] != true {
        return Err(format!("{command}: {reply}").into());
    }
    Ok(reply["result"].clone())
}

fn apply(session: &mut Session, editor: &str, command: &str, params: Value) -> TestResult<Value> {
    call(
        session,
        "project.apply",
        json!({"operation": {"editor": editor, "operation": {"command": command, "params": params}}}),
    )
}

/// The part a generator preset makes: its component, symbol, footprint
/// and binding revisions as `component.place` names them.
fn make(session: &mut Session, generator: &str, preset: &str, params: Value) -> TestResult<Value> {
    let made = call(
        session,
        "library.generate_part",
        json!({"generator": generator, "scope": "component", "preset": preset, "params": params}),
    )?;
    Ok(made["outcome"]["report"]["revisions"].clone())
}

/// A board-only fiducial placed at `at` on `layer`.
fn fiducial(session: &mut Session, part: &Value, at: [f64; 2], layer: &str) -> TestResult<u64> {
    let placed = apply(
        session,
        "Instance",
        "component.place_board_only",
        json!({"component": {"id": part[0]["id"], "revision": part[0]["revision"]},
            "representation": part[1]["id"],
            "physical": {"binding": part[4]["id"], "footprint": part[3]["id"]}}),
    )?;
    let instance = placed["outcome"]["instance"]
        .as_u64()
        .ok_or("no instance")?;
    place(session, instance, at, layer)?;
    Ok(instance)
}

fn place(session: &mut Session, instance: u64, at: [f64; 2], layer: &str) -> TestResult {
    let placed = session
        .engine()
        .project()
        .placements
        .iter()
        .any(|p| p.instance == instance);
    apply(
        session,
        "Layout",
        if placed {
            "layout.move"
        } else {
            "layout.place"
        },
        json!({"instance": instance, "position": at, "rotation_deg": 0.0, "layer": layer}),
    )?;
    Ok(())
}

fn findings(session: &Session, code: &str) -> Vec<Finding> {
    sparksmith_core::layout::checks(session.engine().project(), &Default::default())
        .into_iter()
        .filter(|f| f.code == code)
        .collect()
}

fn messages(session: &Session, code: &str) -> Vec<String> {
    findings(session, code)
        .into_iter()
        .map(|f| f.message)
        .collect()
}

fn session() -> TestResult<Session> {
    let mut session = Session::fixture(Engine::default());
    session.set_shipped(ShippedLibrary::at(Path::new(SHIPPED))?);
    let file: Value = serde_json::from_str(&std::fs::read_to_string(
        Path::new(SHIPPED).join("profiles/jlcpcb-2-layer.json"),
    )?)?;
    apply(
        &mut session,
        "Rules",
        "rules.set_fab_profile",
        json!({"profile": file["fab_profile"]}),
    )?;
    Ok(session)
}

#[test]
fn a_side_with_surface_parts_needs_three_fiducials_not_symmetric() -> TestResult {
    let mut session = session()?;
    let b = session.engine().project().board().bounds();
    let c = [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0];
    let fid = make(&mut session, "fiducial", "fiducial-global-1mm", json!({}))?;
    let resistor = make(&mut session, "chip", "resistor-0603", json!({}))?;
    let placed = apply(
        &mut session,
        "Instance",
        "component.place",
        json!({"component": {"id": resistor[0]["id"], "revision": resistor[0]["revision"]},
            "designator": "R1", "properties": [], "representation": resistor[1]["id"],
            "unit": 1, "position": [60.0, 0.0],
            "physical": {"binding": resistor[4]["id"], "footprint": resistor[3]["id"]}}),
    )?;
    let r1 = placed["outcome"]["instance"].as_u64().ok_or("no R1")?;
    place(&mut session, r1, c, "Top")?;
    // R1 is on the top only: the top needs three fiducials, the bottom none.
    assert_eq!(
        messages(&session, TOO_FEW),
        vec![
            "The top side has 0 fiducials for its surface-mount parts; needs 3 (JLCPCB 2-layer: min fiducials per assembled side)"
                .to_string()
        ]
    );
    // Two fiducials diagonally across the centre: too few, and symmetric.
    let a = fiducial(&mut session, &fid, [c[0] - 20.0, c[1] - 15.0], "Top")?;
    let d = fiducial(&mut session, &fid, [c[0] + 20.0, c[1] + 15.0], "Top")?;
    assert_eq!(
        messages(&session, TOO_FEW),
        vec![
            "The top side has 2 fiducials for its surface-mount parts; needs 3 (JLCPCB 2-layer: min fiducials per assembled side)"
                .to_string()
        ]
    );
    let symmetric = findings(&session, SYMMETRIC);
    assert_eq!(symmetric.len(), 1, "{symmetric:?}");
    assert!(
        symmetric[0]
            .message
            .starts_with("The top side's fiducials are symmetric about the board's centre"),
        "{}",
        symmetric[0].message
    );
    assert_eq!(symmetric[0].objects.len(), 2);
    // JLCPCB's absolute minimum is two: only the symmetry remains.
    apply(
        &mut session,
        "Rules",
        "rules.set_fab_options",
        json!({"options": {"limits": "minimum"}}),
    )?;
    assert!(messages(&session, TOO_FEW).is_empty());
    assert_eq!(findings(&session, SYMMETRIC).len(), 1);
    apply(
        &mut session,
        "Rules",
        "rules.set_fab_options",
        json!({"options": {"limits": "recommended"}}),
    )?;
    // A third in the L: three, and no longer symmetric.
    fiducial(&mut session, &fid, [c[0] - 20.0, c[1] + 15.0], "Top")?;
    assert!(messages(&session, TOO_FEW).is_empty());
    assert!(findings(&session, SYMMETRIC).is_empty());
    // A bottom fiducial does not count for the top. Moving R1 to the
    // bottom moves the need with it: the bottom has 1 of 3.
    place(&mut session, r1, c, "Bottom")?;
    fiducial(&mut session, &fid, [c[0] + 20.0, c[1] - 15.0], "Bottom")?;
    assert_eq!(
        messages(&session, TOO_FEW),
        vec![
            "The bottom side has 1 fiducial for its surface-mount parts; needs 3 (JLCPCB 2-layer: min fiducials per assembled side)"
                .to_string()
        ]
    );
    let _ = (a, d);
    Ok(())
}

#[test]
fn silkscreen_parts_and_copper_keep_out_of_the_clear_area() -> TestResult {
    let mut session = session()?;
    let b = session.engine().project().board().bounds();
    let c = [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0];
    let fid = make(&mut session, "fiducial", "fiducial-global-1mm", json!({}))?;
    let resistor = make(&mut session, "chip", "resistor-0603", json!({}))?;
    let f1 = fiducial(&mut session, &fid, c, "Top")?;
    // The fiducial's clear area is a keep-out on its own side only, so a
    // plane or track on the far side may pass under it.
    let project = session.engine().project();
    let kept: Vec<KeepoutLayers> = sparksmith_core::layout::placed_keepouts(project)
        .into_iter()
        .filter(|k| k.instance == f1)
        .map(|k| k.keepout.layers)
        .collect();
    assert_eq!(kept, vec![KeepoutLayers::Only(vec![CopperLayer::Top])]);
    // Its own drawings are clean.
    assert!(findings(&session, SILK).is_empty());
    assert!(findings(&session, "board.courtyard_overlap").is_empty());
    // A resistor laid over it: its silkscreen and its courtyard are in the
    // clear area.
    let placed = apply(
        &mut session,
        "Instance",
        "component.place",
        json!({"component": {"id": resistor[0]["id"], "revision": resistor[0]["revision"]},
            "designator": "R1", "properties": [], "representation": resistor[1]["id"],
            "unit": 1, "position": [60.0, 0.0],
            "physical": {"binding": resistor[4]["id"], "footprint": resistor[3]["id"]}}),
    )?;
    let r1 = placed["outcome"]["instance"].as_u64().ok_or("no R1")?;
    place(&mut session, r1, c, "Top")?;
    assert_eq!(
        messages(&session, SILK),
        vec!["Silkscreen of R1 lies in fiducial FID1's clear area on the top side".to_string()]
    );
    assert_eq!(findings(&session, "board.courtyard_overlap").len(), 1);
    // On the other side of the board neither applies.
    place(&mut session, r1, c, "Bottom")?;
    assert!(findings(&session, SILK).is_empty());
    assert!(findings(&session, "board.courtyard_overlap").is_empty());
    Ok(())
}

/// A fiducial on both sides: a dot on each outer layer, each with its clear
/// area on its own layer; placed on the bottom, the near dot is the
/// bottom's.
#[test]
fn a_fiducial_on_both_sides_keeps_both_outer_layers_clear() -> TestResult {
    let mut session = session()?;
    let fid = make(
        &mut session,
        "fiducial",
        "fiducial-global-1mm-both",
        json!({}),
    )?;
    let f = fiducial(&mut session, &fid, [10.0, 10.0], "Bottom")?;
    let project = session.engine().project();
    // No body of its own and no part file packed: the board draws the land.
    assert!(project.library.packages.iter().all(|p| matches!(
        p.definition.model,
        sparksmith_core::library::PackageModel::NoBody
    )));
    assert!(project.library.models.is_empty());
    // Copper only: nothing to order or place, so neither the BOM nor
    // pick-and-place lists it.
    let instance = project.instance(f).ok_or("no fiducial")?;
    let library = &project.library;
    assert!(sparksmith_core::library::mechanical::copper_only(
        library,
        instance.component
    ));
    assert!(!sparksmith_core::library::mechanical::in_bom(
        library,
        instance.component
    ));
    assert!(!sparksmith_core::library::mechanical::in_pick_and_place(
        library,
        instance.component
    ));
    let kept: Vec<KeepoutLayers> = sparksmith_core::layout::placed_keepouts(project)
        .into_iter()
        .filter(|k| k.instance == f)
        .map(|k| k.keepout.layers)
        .collect();
    assert_eq!(
        kept,
        vec![
            KeepoutLayers::Only(vec![CopperLayer::Bottom]),
            KeepoutLayers::Only(vec![CopperLayer::Top]),
        ]
    );
    let dots: Vec<bool> = sparksmith_core::layout::fiducial::placed_fiducials(project)
        .into_iter()
        .map(|d| d.layer == sparksmith_core::model::Layer::Top)
        .collect();
    assert_eq!(dots, vec![false, true]);
    Ok(())
}

/// A track across a fiducial is copper in its clear area, and it blocks the
/// production check, whether the fiducial carries its own keep-out (the
/// generator's: `keepout.track`) or not (a footprint made elsewhere, whose
/// clear area is its mask opening: `board.fiducial_copper`). The fiducial
/// is found from the physical pads the fabrication films draw, so an
/// unnetted fiducial is checked as the Gerber writes it.
#[test]
fn a_track_across_a_fiducial_blocks_the_production_check() -> TestResult {
    use sparksmith_core::fabrication::preflight;
    use sparksmith_core::model::{Net, Track};
    let mut session = session()?;
    let fid = make(&mut session, "fiducial", "fiducial-global-1mm", json!({}))?;
    let f = fiducial(&mut session, &fid, [20.0, 20.0], "Top")?;
    let mut project = session.engine().project().clone();
    let net = project.allocate_net()?;
    project.nets.push(Net {
        id: net,
        name: "VCC".into(),
        terminals: Vec::new(),
    });
    let track = project.allocate()?;
    project.tracks.push(Track {
        id: track,
        net,
        start: [15.0, 20.0],
        end: [25.0, 20.0],
        width: 0.25,
        layer: CopperLayer::Top,
        locked: false,
    });
    let blocking = |project: &sparksmith_core::model::Project, code: &str| -> TestResult<usize> {
        let report = preflight::report(
            project,
            &session.engine().document().drafts,
            None,
            &Default::default(),
        )?;
        Ok(report
            .items
            .iter()
            .filter(|i| i.code == code && i.blocks())
            .count())
    };
    assert_eq!(blocking(&project, "keepout.track")?, 1);
    assert_eq!(blocking(&project, "board.fiducial_copper")?, 0);
    // The same fiducial without its keep-out: its mask opening is the clear
    // area, and the track still blocks.
    for footprint in project.library.footprints.iter_mut() {
        if footprint
            .definition
            .pads
            .iter()
            .any(|p| p.kind == sparksmith_core::library::PadKind::Fiducial)
        {
            footprint.definition.keepouts.clear();
        }
    }
    let copper: Vec<String> = sparksmith_core::layout::checks(&project, &Default::default())
        .into_iter()
        .filter(|x| x.code == "board.fiducial_copper")
        .map(|x| x.message)
        .collect();
    assert_eq!(
        copper,
        vec!["A track on VCC crosses fiducial FID1's clear area on the top side".to_string()]
    );
    assert_eq!(blocking(&project, "keepout.track")?, 0);
    assert_eq!(blocking(&project, "board.fiducial_copper")?, 1);
    // On the bottom the track passes under it.
    if let Some(t) = project.tracks.iter_mut().find(|t| t.id == track) {
        t.layer = CopperLayer::Bottom;
    }
    assert_eq!(blocking(&project, "board.fiducial_copper")?, 0);
    let _ = f;
    Ok(())
}

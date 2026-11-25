//! Flipping and turning parts that dimensions hold, on the ESP32-S31 dev
//! board: J1's shell face lies on the board's bottom edge, and U3, U2, R2
//! and others are held at offsets from J1. Flipped or turned in place, J1's
//! face would meet the edge from outside, with the parts held to it; that
//! is refused. Turned with the flip it keeps its place. The report names
//! every connection that is open afterwards.
use sparksmith_automation::session::hosted;
use sparksmith_core::layout::arrange::PartMove;
use sparksmith_core::layout::dimensions;
use sparksmith_core::layout::{self, LayoutCommand};
use sparksmith_core::model::{Layer, Placement};
use sparksmith_core::{Engine, Operation};
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn example() -> Fallible<Engine> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut engine = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    engine.refill_stale()?;
    Ok(engine)
}

fn placed(engine: &Engine, designator: &str) -> Fallible<Placement> {
    let id = engine
        .project()
        .instance_by_designator(designator)
        .ok_or(designator.to_owned())?
        .id;
    Ok(engine.project().placement_of(id).cloned().ok_or("placed")?)
}

fn posed(p: &Placement, turn: f64, layer: Layer) -> Operation {
    Operation::Layout(LayoutCommand::Move {
        instance: p.instance,
        position: p.position,
        rotation_deg: (p.rotation_deg + turn).rem_euclid(360.0),
        layer,
    })
}

/// `operation` is refused, naming J1 and saying why, and changes nothing.
fn refused(engine: &mut Engine, operation: Operation, word: &str) -> Fallible {
    let before = engine.to_file();
    let error = engine
        .apply_hosted(engine.revision(), operation, hosted())
        .err()
        .ok_or(format!("{word}: accepted"))?
        .to_string();
    assert!(
        error.contains("J1") && error.contains(word) && error.contains("the board"),
        "{error}"
    );
    assert!(engine.to_file() == before, "{word}");
    Ok(())
}

/// The nets of the open connections, one name each, as the airwires list
/// them once the stale fills are refilled.
fn open(engine: &mut Engine) -> Fallible<Vec<String>> {
    engine.refill_stale()?;
    let mut names: Vec<String> = layout::airwires(engine.project())
        .iter()
        .map(|(a, _)| engine.project().net_name(a.net).to_owned())
        .collect();
    names.sort();
    Ok(names)
}

#[test]
fn an_edge_held_part_flipped_or_turned_in_place_is_refused() -> Fallible {
    let mut engine = example()?;
    let j1 = placed(&engine, "J1")?;
    refused(&mut engine, posed(&j1, 0.0, Layer::Bottom), "flipped")?;
    refused(&mut engine, posed(&j1, 180.0, Layer::Top), "turned")?;
    refused(&mut engine, posed(&j1, 90.0, Layer::Top), "turned")?;
    // In a group with a part nothing holds to J1.
    let u1 = placed(&engine, "U1")?;
    let flip = |p: &Placement| PartMove {
        instance: p.instance,
        position: p.position,
        rotation_deg: p.rotation_deg,
        layer: Layer::Bottom,
    };
    refused(
        &mut engine,
        Operation::Layout(LayoutCommand::MoveParts {
            moves: vec![flip(&u1), flip(&j1)],
            copper: None,
        }),
        "flipped",
    )
}

#[test]
fn a_held_part_flipped_keeps_its_place_and_every_open_connection_is_named() -> Fallible {
    let mut engine = example()?;
    assert_eq!(open(&mut engine)?, Vec::<String>::new());
    let evaluator = hosted().board.ok_or("no evaluator")?;
    for (name, turn) in [("J1", 180.0), ("R2", 0.0)] {
        let part = placed(&engine, name)?;
        let pose =
            dimensions::flipped(engine.project(), part.instance, evaluator).ok_or("no flip")?;
        let turned = (pose.rotation_deg - part.rotation_deg).rem_euclid(360.0);
        assert!((turned - turn).abs() < 1e-9, "{name}: {turned}");
        let before = engine.to_file();
        let poses: Vec<Placement> = engine.project().placements.to_vec();
        engine.apply_hosted(
            engine.revision(),
            posed(&part, turn, Layer::Bottom),
            hosted(),
        )?;
        // The part is where it was, and so is every other one.
        for old in &poses {
            let now = engine
                .project()
                .placement_of(old.instance)
                .ok_or("placement")?;
            let moved =
                (now.position[0] - old.position[0]).hypot(now.position[1] - old.position[1]);
            assert!(moved < 1e-6, "{name}: {} moved {moved}", old.instance);
        }
        let mut named = engine.rerouted().unrouted.clone();
        named.sort();
        let line = engine.rerouted().line().unwrap_or_default();
        println!("{name}: {line}");
        assert!(!named.is_empty(), "{name}: its tracks stay on the top");
        assert_eq!(named, open(&mut engine)?, "{name}");
        engine.undo()?;
        assert!(engine.to_file() == before, "{name}: one undo step");
    }
    Ok(())
}

//! Flipping and turning parts that dimensions hold, on the ESP32-S31 dev
//! board: J1's shell face lies on the board's bottom edge, and U3, U2, R2
//! and others are held at offsets from J1. Flipped or turned in place, J1's
//! face meets the edge from outside, with the parts held to it; that is
//! made, said and listed. Turned with the flip it keeps its place. The
//! report names every connection that is open afterwards.
use sparksmith_automation::session::hosted;
use sparksmith_core::fabrication::FabricationCommand;
use sparksmith_core::layout::arrange::PartMove;
use sparksmith_core::layout::dimensions;
use sparksmith_core::layout::{self, LayoutCommand};
use sparksmith_core::model::{Layer, Placement};
use sparksmith_core::{Engine, Operation, Query, QueryResult};
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

/// The `board.outside_outline` findings of the parts' pads.
fn beyond(engine: &Engine) -> Vec<String> {
    layout::checks(engine.project(), engine.check_settings())
        .into_iter()
        .filter(|f| f.code == "board.outside_outline" && f.message.contains(" pad "))
        .map(|f| f.message)
        .collect()
}

/// `operation` is made as one undo step, said at once (J1, the board and
/// the edge its dimension holds it on), and listed for J1's pads.
fn over_the_edge(engine: &mut Engine, operation: Operation, what: &str) -> Fallible {
    let before = engine.to_file();
    engine
        .apply_hosted(engine.revision(), operation, hosted())
        .map_err(|e| format!("{what}: {e}"))?;
    let said = engine.rerouted().over_edge.clone();
    println!("{what}: {said:?} {:?}", engine.rerouted().line());
    let [line] = said.as_slice() else {
        return Err(format!("{what}: {said:?}").into());
    };
    assert!(
        line.starts_with("J1 ") && line.contains("beyond the board: ") && line.contains("edge"),
        "{what}: {line}"
    );
    let found = beyond(engine);
    assert!(
        found.iter().any(|m| m.contains("J1 pad")),
        "{what}: {found:?}"
    );
    engine.undo()?;
    assert!(engine.to_file() == before, "{what}: one undo step");
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
fn an_edge_held_part_flipped_in_place_is_made_said_and_listed() -> Fallible {
    let mut engine = example()?;
    assert_eq!(beyond(&engine), Vec::<String>::new());
    let j1 = placed(&engine, "J1")?;
    over_the_edge(&mut engine, posed(&j1, 0.0, Layer::Bottom), "flipped")?;
    // In a group with a part held to the module, by its origin.
    let sw1 = placed(&engine, "SW1")?;
    let flip = |p: &Placement| PartMove {
        instance: p.instance,
        position: p.position,
        rotation_deg: p.rotation_deg,
        layer: Layer::Bottom,
    };
    over_the_edge(
        &mut engine,
        Operation::Layout(LayoutCommand::MoveParts {
            moves: vec![flip(&sw1), flip(&j1)],
            copper: None,
        }),
        "flipped with SW1",
    )
}

/// A turn in place has the same solve; what it may be refused for is the
/// copper, not the part.
#[test]
fn an_edge_held_part_turned_in_place_is_not_refused_for_the_part() -> Fallible {
    for turn in [90.0, 180.0] {
        let mut engine = example()?;
        let j1 = placed(&engine, "J1")?;
        let before = engine.to_file();
        match engine.apply_hosted(engine.revision(), posed(&j1, turn, Layer::Top), hosted()) {
            Ok(_) => {
                println!("{turn}: {:?}", engine.rerouted());
                assert!(!engine.rerouted().over_edge.is_empty(), "{turn}");
                engine.undo()?;
            }
            Err(error) => {
                println!("{turn}: {error}");
                let text = error.to_string().to_lowercase();
                assert!(text.contains("track") || text.contains("via"), "{text}");
            }
        }
        assert!(engine.to_file() == before, "{turn}");
    }
    Ok(())
}

/// The findings of a part beyond the outline may be waived with a reason.
#[test]
fn a_part_beyond_the_outline_may_be_waived() -> Fallible {
    let mut engine = example()?;
    let j1 = placed(&engine, "J1")?;
    engine.apply_hosted(engine.revision(), posed(&j1, 0.0, Layer::Bottom), hosted())?;
    engine.refill_stale()?;
    let items = |engine: &Engine| -> Fallible<Vec<(String, bool)>> {
        let QueryResult::Preflight(report) = engine.query(Query::Preflight { profile: None })?
        else {
            return Err("not a preflight report".into());
        };
        Ok(report
            .items
            .iter()
            .filter(|i| i.code == "board.outside_outline" && i.title.contains("J1 pad"))
            .map(|i| (i.key.clone(), i.blocks()))
            .collect())
    };
    let found = items(&engine)?;
    assert!(!found.is_empty() && found.iter().all(|(_, blocks)| *blocks));
    engine.apply(
        engine.revision(),
        Operation::Fabrication(FabricationCommand::Waive {
            keys: found.into_iter().map(|(key, _)| key).collect(),
            reason: "hangs over on purpose".into(),
            profile: None,
        }),
    )?;
    assert!(items(&engine)?.iter().all(|(_, blocks)| !*blocks));
    Ok(())
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

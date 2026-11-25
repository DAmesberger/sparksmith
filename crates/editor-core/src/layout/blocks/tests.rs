//! The `block.*` commands through the engine: one undo step each,
//! refusals that name what is in the way and change nothing, copper carried
//! or named, and blocks that follow the design's parts.
use super::*;
use crate::instances::blocks::{BlockCommand as B, block_of};
use crate::layout::arrange::{AlignTo, PartMove};
use crate::layout::{LayoutCommand as L, RouteRequest};
use crate::model::Block;
use crate::schematic::{ClipObject, SchematicCommand};
use crate::tests::support::{by_designator, demo_fixture, pt};
use crate::updates::Change;
use crate::{Engine, Operation, Outcome, Query, QueryResult, UnitId, format, instances};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

impl From<L> for Operation {
    fn from(command: L) -> Self {
        Self::Layout(command)
    }
}

impl From<B> for Operation {
    fn from(command: B) -> Self {
        Self::Block(command)
    }
}

fn run(engine: &mut Engine, command: impl Into<Operation>) -> Fallible<Outcome> {
    Ok(engine.apply(engine.revision(), command.into())?)
}

/// The refusal's words; nothing may have changed.
fn refused(engine: &mut Engine, command: impl Into<Operation>) -> Fallible<String> {
    let (before, revision) = (engine.project().clone(), engine.revision());
    let error = engine
        .apply(revision, command.into())
        .err()
        .ok_or("taken")?;
    assert_eq!(engine.project(), &before);
    assert_eq!(engine.revision(), revision);
    Ok(error.to_string())
}

fn mv(instance: Id, position: Point) -> L {
    L::Move {
        instance,
        position,
        rotation_deg: 0.0,
        layer: Layer::Top,
    }
}

/// R1 and R2 of the demo at (10, 10) and (25, 10), a track between them.
fn pair() -> Fallible<(Engine, Id, Id)> {
    let mut engine = Engine::demo()?;
    let (r1, r2) = (by_designator(&engine, "R1")?, by_designator(&engine, "R2")?);
    run(&mut engine, mv(r1, [10.0, 10.0]))?;
    run(&mut engine, mv(r2, [25.0, 10.0]))?;
    let net = engine
        .project()
        .net_of(crate::Endpoint {
            instance: r1,
            terminal: crate::TerminalId(2),
        })
        .map(|n| n.id)
        .ok_or("no net")?;
    run(
        &mut engine,
        L::Route {
            request: RouteRequest {
                net,
                start: [11.2, 10.0],
                end: [23.8, 10.0],
                width: 0.3,
                layer: Layer::Top.into(),
            },
        },
    )?;
    Ok((engine, r1, r2))
}

/// A third resistor on the schematic, not on the board.
fn third(engine: &mut Engine) -> Fallible<Id> {
    let f = demo_fixture(engine)?;
    let outcome = engine.apply(
        engine.revision(),
        Operation::Instance(crate::InstanceCommand::Place(instances::NewInstance {
            sheet: None,
            component: f.r1(),
            designator: Some("R3".into()),
            properties: vec![],
            representation: Some(f.symbol),
            unit: Some(UnitId(1)),
            position: Some(pt([40.0, 0.0])),
            physical: Some(f.physical()),
        })),
    )?;
    let Outcome::InstancePlaced { instance, .. } = outcome else {
        return Err("R3 was not placed".into());
    };
    Ok(instance)
}

fn block(engine: &mut Engine, parts: &[Id], name: Option<&str>) -> Fallible<Id> {
    let outcome = run(
        engine,
        B::Add {
            parts: parts.to_vec(),
            name: name.map(str::to_owned),
        },
    )?;
    match outcome {
        Outcome::BlockAdded { block } => Ok(block),
        other => Err(format!("{other:?}").into()),
    }
}

fn move_block(id: Id, by: Point, turn_deg: f64, flip: bool) -> B {
    BlockMove { by, turn_deg, flip }.command(id)
}

fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

fn at(engine: &Engine, id: Id) -> Fallible<Placement> {
    Ok(engine
        .project()
        .placement_of(id)
        .ok_or("not placed")?
        .clone())
}

#[test]
fn a_block_is_made_named_changed_and_deleted_in_one_step_each() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    let r3 = third(&mut engine)?;
    let start = engine.project().clone();
    let id = block(&mut engine, &[r2, r1], None)?;
    let made = engine.project().block(id).ok_or("no block")?.clone();
    assert_eq!(
        made,
        Block {
            id,
            name: "Block 1".into(),
            members: vec![r1.min(r2), r1.max(r2)],
            locked: false,
        }
    );
    assert_eq!(engine.project().placements, start.placements);
    engine.undo()?;
    assert_eq!(engine.project(), &start, "one undo step");
    engine.redo()?;
    run(
        &mut engine,
        B::Rename {
            id,
            name: " Converter ".into(),
        },
    )?;
    assert_eq!(engine.project().block(id).ok_or("gone")?.name, "Converter");
    // Its own name in another case is no other block's.
    run(
        &mut engine,
        B::Rename {
            id,
            name: "CONVERTER".into(),
        },
    )?;
    let why = refused(
        &mut engine,
        B::Add {
            parts: vec![r3, r1],
            name: None,
        },
    )?;
    assert!(why.contains("R1 is in the block CONVERTER"), "{why}");
    let why = refused(
        &mut engine,
        B::Add {
            parts: vec![r3],
            name: Some("converter".into()),
        },
    )?;
    assert!(why.contains("already a block CONVERTER"), "{why}");
    for (parts, name, word) in [
        (vec![], None, "at least one part"),
        (vec![r3, r3], None, "R3 is named twice"),
        (vec![99_999], None, "99999"),
        (vec![r3], Some("  "), "needs a name"),
        (vec![r3], Some("a\nb"), "needs a name"),
    ] {
        let name = name.map(str::to_owned);
        let why = refused(&mut engine, B::Add { parts, name })?;
        assert!(why.contains(word), "{why}");
    }
    run(
        &mut engine,
        B::Members {
            id,
            add: vec![r3],
            remove: vec![r2],
        },
    )?;
    assert_eq!(
        engine.project().block(id).ok_or("gone")?.members,
        vec![r1, r3]
    );
    let why = refused(
        &mut engine,
        B::Members {
            id,
            add: vec![],
            remove: vec![r2],
        },
    )?;
    assert!(why.contains("R2 is not in the block CONVERTER"), "{why}");
    let why = refused(
        &mut engine,
        B::Members {
            id,
            add: vec![],
            remove: vec![r1, r3],
        },
    )?;
    assert!(why.contains("would have no part left"), "{why}");
    let before = engine.project().clone();
    run(&mut engine, B::Delete { ids: vec![id] })?;
    assert!(engine.project().blocks.is_empty());
    assert_eq!(engine.project().placements, before.placements);
    assert_eq!(engine.project().tracks, before.tracks);
    engine.undo()?;
    assert_eq!(engine.project(), &before);
    Ok(())
}

/// A new block takes the first free number; a saved and reopened design has
/// its blocks, and one without writes none.
#[test]
fn blocks_are_saved_and_reopened() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    assert!(!String::from_utf8_lossy(&format::encode(&engine.to_file())?).contains("\"blocks\""));
    let first = block(&mut engine, &[r1], None)?;
    block(&mut engine, &[r2], None)?;
    run(&mut engine, B::Delete { ids: vec![first] })?;
    let again = block(&mut engine, &[r1], None)?;
    assert_ne!(again, first, "an identity is not used twice");
    let names: Vec<&str> = engine
        .project()
        .blocks
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(names, ["Block 2", "Block 1"]);
    run(
        &mut engine,
        B::BoardLock {
            ids: vec![again],
            locked: true,
        },
    )?;
    let bytes = format::encode(&engine.to_file())?;
    let opened = Engine::from_file(format::decode(&bytes)?)?;
    assert_eq!(opened.project().blocks, engine.project().blocks);
    assert_eq!(format::encode(&opened.to_file())?, bytes);
    Ok(())
}

/// A file whose blocks break the rules is refused when it is read.
#[test]
fn a_file_with_a_wrong_block_is_refused() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    let id = block(&mut engine, &[r1, r2], None)?;
    let wrong = |change: &dyn Fn(&mut Vec<Block>)| -> bool {
        let mut file = engine.to_file();
        change(&mut file.project.blocks);
        Engine::from_file(file).is_err()
    };
    assert!(!wrong(&|_| ()));
    assert!(wrong(&|b| b[0].members.push(99_999)));
    assert!(wrong(&|b| b[0].members.clear()));
    assert!(wrong(&|b| b[0].members.push(r1)));
    assert!(wrong(&|b| b[0].name = " ".into()));
    assert!(wrong(&|b| b[0].id = r1));
    assert!(wrong(&|b| {
        let mut twin = b[0].clone();
        twin.name = "Other".into();
        twin.id = id + 1;
        b.push(twin);
    }));
    Ok(())
}

/// The border is the convex hull of the courtyards of the parts on the
/// board: it holds every point of them, each corner is one, and no margin
/// is added. A part without a courtyard counts with its outline.
#[test]
fn the_border_is_the_hull_of_the_parts() -> Fallible {
    let mut engine = Engine::showcase()?;
    let parts: Vec<Id> = engine
        .project()
        .placements
        .iter()
        .map(|p| p.instance)
        .take(4)
        .collect();
    let id = block(&mut engine, &parts, None)?;
    let project = engine.project();
    let mut points: Vec<Point> = super::super::placed_courtyards(project)
        .iter()
        .filter(|c| parts.contains(&c.instance))
        .flat_map(|c| c.shape.flatten(CHORD_MM))
        .collect();
    let with: BTreeSet<Id> = super::super::placed_courtyards(project)
        .iter()
        .map(|c| c.instance)
        .collect();
    for part in parts.iter().filter(|p| !with.contains(p)) {
        let placement = project.placement_of(*part).ok_or("not placed")?;
        points.extend(super::super::part_envelope(project, placement).ok_or("no outline")?);
    }
    let found = borders(project);
    let [(of, ring)] = &found[..] else {
        return Err("one border".into());
    };
    assert_eq!(*of, id);
    assert!(ring.len() >= 3 && ring.iter().all(|c| points.contains(c)));
    for (k, a) in ring.iter().enumerate() {
        let b = ring[(k + 1) % ring.len()];
        for p in &points {
            let side = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            assert!(side >= -1e-9, "{p:?} is outside {a:?}–{b:?}");
        }
    }
    let QueryResult::Blocks(rows) = engine.query(Query::Blocks {})? else {
        return Err("no rows".into());
    };
    assert_eq!(rows[0].board_border, *ring);
    assert_eq!(rows[0].parts.len(), 4);
    assert!(rows[0].parts.iter().all(|p| p.board_side.is_some()));
    Ok(())
}

/// One step moves every part and the copper between them; a whole turn and
/// two flips are where they began.
#[test]
fn a_block_moves_turns_and_flips_as_one() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    let id = block(&mut engine, &[r1, r2], None)?;
    let before = engine.project().clone();
    run(&mut engine, move_block(id, [2.0, 3.0], 0.0, false))?;
    assert!(near(at(&engine, r1)?.position, [12.0, 13.0]));
    assert!(near(at(&engine, r2)?.position, [27.0, 13.0]));
    let track = engine.project().tracks.first().ok_or("no track")?;
    assert!(near(track.start, [13.2, 13.0]) && near(track.end, [25.8, 13.0]));
    assert!(engine.rerouted().is_empty());
    engine.undo()?;
    assert_eq!(engine.project(), &before, "one undo step");

    // A quarter turn takes the track between the parts round with them.
    run(&mut engine, move_block(id, [0.0, 0.0], 90.0, false))?;
    let (a, b) = (at(&engine, r1)?, at(&engine, r2)?);
    assert!(near(a.position, [17.5, 2.5]) && near(b.position, [17.5, 17.5]));
    assert_eq!((a.rotation_deg, b.rotation_deg), (90.0, 90.0));
    let track = engine.project().tracks.first().ok_or("no track")?;
    assert!(
        near(track.start, [17.5, 3.7]) && near(track.end, [17.5, 16.3]),
        "{track:?}"
    );
    assert!(engine.rerouted().is_empty(), "{:?}", engine.rerouted());
    for _ in 0..3 {
        run(&mut engine, move_block(id, [0.0, 0.0], 90.0, false))?;
    }
    assert!(near(at(&engine, r1)?.position, [10.0, 10.0]));
    assert_eq!(at(&engine, r1)?.rotation_deg, 0.0);

    // Over to the other side, left to right: the connection whose track
    // stays on the top is named.
    run(&mut engine, move_block(id, [0.0, 0.0], 0.0, true))?;
    let (a, b) = (at(&engine, r1)?, at(&engine, r2)?);
    assert!(near(a.position, [25.0, 10.0]) && near(b.position, [10.0, 10.0]));
    assert_eq!((a.layer, b.layer), (Layer::Bottom, Layer::Bottom));
    assert_eq!(
        engine.rerouted().unrouted.len(),
        1,
        "{:?}",
        engine.rerouted()
    );
    run(&mut engine, move_block(id, [0.0, 0.0], 0.0, true))?;
    let a = at(&engine, r1)?;
    assert!(near(a.position, [10.0, 10.0]) && a.layer == Layer::Top);
    assert_eq!(a.rotation_deg, 0.0);
    Ok(())
}

/// What a flip does to a part is a mirror of the board, left to right: every
/// pad lands where its mirror image is.
#[test]
fn a_flipped_block_is_its_mirror_image() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    run(
        &mut engine,
        L::Move {
            instance: r2,
            position: [25.0, 14.0],
            rotation_deg: 30.0,
            layer: Layer::Bottom,
        },
    )?;
    let id = block(&mut engine, &[r1, r2], None)?;
    let pads = |engine: &Engine| -> Vec<(Id, crate::PadId, Point)> {
        placed_pads(engine.project())
            .iter()
            .filter(|p| [r1, r2].contains(&p.instance))
            .map(|p| (p.instance, p.pad.id, p.transform.apply([0.4, 0.3])))
            .collect()
    };
    let was = pads(&engine);
    let m = middle(engine.project(), engine.project().block(id).ok_or("gone")?);
    run(&mut engine, move_block(id, [0.0, 0.0], 0.0, true))?;
    let now = pads(&engine);
    assert_eq!(was.len(), now.len());
    for (a, b) in was.iter().zip(&now) {
        assert_eq!((a.0, a.1), (b.0, b.1));
        assert!(near(b.2, [2.0 * m[0] - a.2[0], a.2[1]]), "{a:?} {b:?}");
    }
    Ok(())
}

/// Nothing moves a locked block or one part of it by hand; a locked part
/// refuses the block's move; each is named.
#[test]
fn locks_refuse_and_are_named() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    let r3 = third(&mut engine)?;
    run(
        &mut engine,
        L::Place {
            instance: r3,
            position: [40.0, 20.0],
            rotation_deg: 0.0,
            layer: Layer::Top,
        },
    )?;
    let id = block(&mut engine, &[r1, r2], Some("Converter"))?;
    run(
        &mut engine,
        B::BoardLock {
            ids: vec![id],
            locked: true,
        },
    )?;
    assert!(!at(&engine, r1)?.locked, "the parts' own locks stay");
    let words = "R1 is in the locked block Converter; unlock the block to";
    let one = PartMove {
        instance: r1,
        position: [11.0, 10.0],
        rotation_deg: 0.0,
        layer: Layer::Top,
    };
    for (command, verb) in [
        (mv(r1, [11.0, 10.0]), "move"),
        (
            L::MoveParts {
                moves: vec![one],
                copper: None,
            },
            "move",
        ),
        (
            L::Align {
                instances: vec![r1, r3],
                to: AlignTo::Right,
            },
            "align",
        ),
    ] {
        let why = refused(&mut engine, command)?;
        assert!(why.contains(&format!("{words} {verb} it")), "{why}");
    }
    let error = super::super::move_refusal(engine.project(), r1, None).ok_or("may move")?;
    assert!(error.to_string().contains(words));
    for (command, verb) in [
        (move_block(id, [1.0, 0.0], 0.0, false), "move"),
        (move_block(id, [0.0, 0.0], 90.0, false), "rotate"),
        (move_block(id, [0.0, 0.0], 0.0, true), "flip"),
    ] {
        let why = refused(&mut engine, command)?;
        let wanted = format!("The block Converter is locked; unlock it to {verb} it");
        assert!(why.contains(&wanted), "{why}");
    }
    // A part outside moves, and so does one the block lets go.
    run(&mut engine, mv(r3, [41.0, 20.0]))?;
    run(
        &mut engine,
        B::Members {
            id,
            add: vec![],
            remove: vec![r1],
        },
    )?;
    run(&mut engine, mv(r1, [11.0, 10.0]))?;
    run(
        &mut engine,
        B::BoardLock {
            ids: vec![id],
            locked: false,
        },
    )?;
    run(
        &mut engine,
        L::SetPlacementLock {
            instance: r2,
            locked: true,
        },
    )?;
    let why = refused(&mut engine, move_block(id, [1.0, 0.0], 0.0, false))?;
    assert!(why.contains("R2 is locked"), "{why}");
    Ok(())
}

/// A part a block's move takes over the board's edge is said at once, also
/// when it is turned or flipped there; one that was beyond it already, or a
/// block that stays on the board, says nothing.
#[test]
fn parts_taken_over_the_edge_are_named() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    // Without copper that would have to stay on the board.
    let tracks = engine.project().tracks.iter().map(|t| t.id).collect();
    run(
        &mut engine,
        L::DeleteObjects {
            objects: crate::layout::several::Objects {
                tracks,
                ..Default::default()
            },
        },
    )?;
    let id = block(&mut engine, &[r1, r2], None)?;
    run(&mut engine, move_block(id, [2.0, 0.0], 0.0, false))?;
    assert!(engine.rerouted().over_edge.is_empty());
    run(&mut engine, move_block(id, [-12.0, 0.0], 0.0, false))?;
    assert_eq!(engine.rerouted().over_edge, ["R1 extends beyond the board"]);
    run(&mut engine, move_block(id, [0.0, 1.0], 0.0, false))?;
    assert!(engine.rerouted().over_edge.is_empty(), "it was there");
    engine.undo()?;
    engine.undo()?;
    run(&mut engine, move_block(id, [-40.0, 0.0], 0.0, false))?;
    assert_eq!(
        engine.rerouted().over_edge,
        ["R1 and 1 more extend beyond the board"]
    );
    Ok(())
}

/// A part that is not on the board yet stays in its block; the move takes
/// the others, and a block with none on the board says so.
#[test]
fn a_part_not_on_the_board_stays_where_it_is() -> Fallible {
    let (mut engine, r1, _) = pair()?;
    let r3 = third(&mut engine)?;
    let id = block(&mut engine, &[r1, r3], None)?;
    run(&mut engine, move_block(id, [1.0, 0.0], 0.0, false))?;
    assert!(near(at(&engine, r1)?.position, [11.0, 10.0]));
    assert!(engine.project().placement_of(r3).is_none());
    run(
        &mut engine,
        B::Members {
            id,
            add: vec![],
            remove: vec![r1],
        },
    )?;
    let why = refused(&mut engine, move_block(id, [1.0, 0.0], 0.0, false))?;
    assert!(
        why.contains("No part of the block Block 1 is on the board"),
        "{why}"
    );
    assert!(borders(engine.project()).is_empty());
    Ok(())
}

/// A removed part leaves its block and the last one takes the block along;
/// undo brings both back. A pasted copy is in no block.
#[test]
fn a_block_follows_the_designs_parts() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    let r3 = third(&mut engine)?;
    let id = block(&mut engine, &[r1, r3], None)?;
    let before = engine.project().clone();
    let remove = |engine: &mut Engine, part: Id| -> Fallible {
        let change = Change::RemoveObjects {
            objects: vec![ClipObject::Instance { id: part }],
        };
        engine.apply_update(engine.revision(), change, false)?;
        Ok(())
    };
    remove(&mut engine, r3)?;
    assert_eq!(engine.project().block(id).ok_or("gone")?.members, vec![r1]);
    engine.undo()?;
    assert_eq!(engine.project().blocks, before.blocks);
    run(
        &mut engine,
        B::Members {
            id,
            add: vec![],
            remove: vec![r1],
        },
    )?;
    remove(&mut engine, r3)?;
    assert!(engine.project().blocks.is_empty());
    engine.undo()?;
    engine.undo()?;
    assert_eq!(engine.project().blocks, before.blocks);

    let QueryResult::SheetClip(clip) = engine.query(Query::SheetCopy {
        sheet: None,
        objects: vec![
            ClipObject::Instance { id: r1 },
            ClipObject::Instance { id: r2 },
        ],
        portable: false,
    })?
    else {
        return Err("no clip".into());
    };
    let outcome = engine.apply(
        engine.revision(),
        Operation::Schematic(SchematicCommand::Paste {
            sheet: None,
            clip,
            offset: pt([0.0, -100.0]),
        }),
    )?;
    let Outcome::Pasted { report } = outcome else {
        return Err("not pasted".into());
    };
    for symbol in report.symbols {
        assert!(block_of(engine.project(), symbol.instance).is_none());
    }
    assert_eq!(engine.project().blocks, before.blocks);
    Ok(())
}

/// Several blocks are deleted or locked in one step, and never their parts;
/// naming one twice or none is refused, and a command that changes nothing
/// takes no step.
#[test]
fn several_blocks_are_deleted_and_locked_at_once() -> Fallible {
    let (mut engine, r1, r2) = pair()?;
    let (a, b) = (
        block(&mut engine, &[r1], None)?,
        block(&mut engine, &[r2], None)?,
    );
    let lock = |locked| B::BoardLock {
        ids: vec![a, b],
        locked,
    };
    run(&mut engine, lock(true))?;
    assert!(engine.project().blocks.iter().all(|b| b.locked));
    assert!(!at(&engine, r1)?.locked);
    let revision = engine.revision();
    run(&mut engine, lock(true))?;
    assert_eq!(engine.revision(), revision, "nothing changed");
    run(&mut engine, lock(false))?;
    for (ids, word) in [
        (vec![], "at least one block"),
        (vec![a, a], "named twice"),
        (vec![99_999], "unknown block"),
    ] {
        let why = refused(&mut engine, B::Delete { ids })?;
        assert!(why.contains(word), "{why}");
    }
    let before = engine.project().clone();
    run(&mut engine, B::Delete { ids: vec![a, b] })?;
    assert!(engine.project().blocks.is_empty());
    assert_eq!(engine.project().placements, before.placements);
    assert_eq!(engine.project().tracks, before.tracks);
    engine.undo()?;
    assert_eq!(engine.project(), &before);
    Ok(())
}

/// What a flip leaves behind is named with its net and counted as the checks
/// count it: one entry for each connection the copper graph newly has open.
/// Undo puts everything back.
#[test]
fn a_flip_names_what_it_leaves_as_the_checks_count_it() -> Fallible {
    let mut engine = Engine::showcase()?;
    let parts: Vec<Id> = engine
        .project()
        .placements
        .iter()
        .filter(|p| !p.locked)
        .map(|p| p.instance)
        .take(3)
        .collect();
    let id = block(&mut engine, &parts, None)?;
    let before = engine.project().clone();
    let open = |engine: &Engine| -> BTreeMap<Id, usize> {
        let mut out = BTreeMap::new();
        let graph = crate::layout::CopperGraph::of(engine.project());
        for (a, _) in graph.airwires() {
            *out.entry(a.net).or_default() += 1;
        }
        out
    };
    let was = open(&engine);
    run(&mut engine, move_block(id, [0.0, 0.0], 0.0, true))?;
    let said = engine.rerouted().unrouted.clone();
    assert!(!said.is_empty(), "the flip opened nothing");
    let poured: BTreeSet<Id> = before.fills.iter().filter_map(|f| f.net).collect();
    for (net, now) in open(&engine) {
        // A poured net is counted as its refill will join it.
        if poured.contains(&net) {
            continue;
        }
        let opened = now.saturating_sub(was.get(&net).copied().unwrap_or(0));
        let name = before.net_name(net);
        let named = said.iter().filter(|n| *n == name).count();
        assert_eq!(named, opened, "{name}: {said:?}");
    }
    let line = engine.rerouted().line().ok_or("nothing said")?;
    for name in &said {
        assert!(line.contains(name.as_str()), "{name} is not in {line:?}");
    }
    engine.undo()?;
    assert_eq!(engine.project(), &before);
    Ok(())
}

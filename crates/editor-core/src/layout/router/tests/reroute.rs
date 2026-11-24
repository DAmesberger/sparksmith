//! A move whose dragged track the router cannot re-route: the edit still
//! applies, the track goes and its connection is left unrouted (an
//! airwire), never a rule violation; one undo restores the board exactly.
use super::*;
use crate::layout::tests::{SHARED, ids};
use crate::layout::{CopperGraph, checks};
use crate::library::{Outline, Segment};
use crate::model::Layer;
use crate::model::fill::{KeepoutLayers, KeepoutRules, KeepoutSpec};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Outline {
    Outline {
        start: [x0, y0],
        segments: vec![
            Segment::Line { to: [x1, y0] },
            Segment::Line { to: [x1, y1] },
            Segment::Line { to: [x0, y1] },
            Segment::Line { to: [x0, y0] },
        ],
    }
}

fn mv(instance: Id, position: [f64; 2]) -> Operation {
    Operation::Layout(LayoutCommand::Move {
        instance,
        position,
        rotation_deg: 0.0,
        layer: Layer::Top,
    })
}

#[test]
fn a_dragged_track_with_no_way_through_is_left_unrouted() -> Fallible {
    let mut engine = Engine::demo()?;
    engine.apply(engine.revision(), Operation::Layout(LayoutCommand::Sync))?;
    let (r1, r2) = ids()?;
    for (instance, position) in [(r1, [10.0, 10.0]), (r2, [25.0, 10.0])] {
        engine.apply(engine.revision(), mv(instance, position))?;
    }
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::Route {
            request: RouteRequest {
                net: SHARED,
                start: [11.2, 10.0],
                end: [23.8, 10.0],
                width: 0.3,
                layer: Top,
            },
        }),
    )?;
    // A closed box of track keepouts round where R2 goes: no route can
    // reach its pads.
    for wall in [
        rectangle(21.0, 16.0, 29.0, 16.5),
        rectangle(21.0, 23.5, 29.0, 24.0),
        rectangle(21.0, 16.0, 21.5, 24.0),
        rectangle(28.5, 16.0, 29.0, 24.0),
    ] {
        engine.apply(
            engine.revision(),
            Operation::Fill(layout::fill::FillCommand::AddKeepout {
                keepout: Box::new(KeepoutSpec {
                    name: String::new(),
                    outline: wall,
                    layers: KeepoutLayers::All,
                    forbid: KeepoutRules {
                        fills: true,
                        tracks: true,
                        vias: true,
                        pads: false,
                    },
                }),
            }),
        )?;
    }
    let before = engine.project().clone();
    let errors = |p: &Project| {
        checks(p, &Default::default())
            .into_iter()
            .filter(|f| f.blocks.is_some() || f.code == "keepout.track")
            .map(|f| f.code.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(errors(&before), Vec::<String>::new());
    assert_eq!(CopperGraph::of(&before).airwires().len(), 0);

    engine.apply(engine.revision(), mv(r2, [25.0, 20.0]))?;
    // The move applied; the track that could not follow is gone.
    assert_eq!(
        engine.project().placement_of(r2).map(|p| p.position),
        Some([25.0, 20.0])
    );
    assert!(engine.project().tracks.is_empty());
    assert_eq!(errors(engine.project()), Vec::<String>::new());
    assert_eq!(CopperGraph::of(engine.project()).airwires().len(), 1);
    let report = engine.rerouted();
    assert!(report.rerouted.is_empty());
    assert_eq!(report.unrouted, vec![before.net_name(SHARED).to_owned()]);
    assert_eq!(
        report.line().as_deref(),
        Some(format!("1 connection left unrouted ({})", before.net_name(SHARED)).as_str())
    );

    engine.undo()?;
    assert_eq!(engine.project(), &before, "one undo step");
    Ok(())
}

/// R1 at (10, 10) and R2 at (25, 10), nothing routed.
fn parts() -> Fallible<(Engine, Id)> {
    let mut engine = Engine::demo()?;
    engine.apply(engine.revision(), Operation::Layout(LayoutCommand::Sync))?;
    let (r1, r2) = ids()?;
    for (instance, position) in [(r1, [10.0, 10.0]), (r2, [25.0, 10.0])] {
        engine.apply(engine.revision(), mv(instance, position))?;
    }
    Ok((engine, r2))
}

fn do_(engine: &mut Engine, command: LayoutCommand) -> Fallible {
    engine.apply(engine.revision(), Operation::Layout(command))?;
    Ok(())
}

/// Where R2 goes for its left pad's edge to be 0.1 mm from the copper of
/// a 0.6 mm via at `at` (the rule is 0.2 mm).
fn beside(engine: &Engine, r2: Id, at: Point) -> Fallible<Point> {
    let pad = layout::pads(engine.project())
        .into_iter()
        .filter(|p| p.instance == r2)
        .min_by(|a, b| a.position[0].total_cmp(&b.position[0]))
        .ok_or("no pad")?;
    let part = engine.project().placement_of(r2).ok_or("r2")?.position;
    let centre = at[0] + 0.3 + 0.1 + pad.size[0] / 2.0;
    Ok([
        part[0] + centre - pad.position[0],
        part[1] + at[1] - pad.position[1],
    ])
}

/// R2 dropped 0.1 mm from via `id` of another net: the via is as it was,
/// nothing is reported re-routed or lost, and the gap is a finding.
fn crowd(engine: &mut Engine, r2: Id, id: Id) -> Fallible {
    let before = engine.project().clone();
    let at = before.via(id).ok_or("via")?.position;
    let to = beside(engine, r2, at)?;
    engine.apply(engine.revision(), mv(r2, to))?;
    let after = engine.project();
    assert_eq!(after.placement_of(r2).map(|p| p.position), Some(to));
    assert_eq!(after.vias, before.vias, "every via stays");
    assert_eq!(after.tracks, before.tracks, "every track stays");
    assert!(engine.rerouted().is_empty(), "{:?}", engine.rerouted());
    assert!(
        violations(after).len() > violations(&before).len(),
        "too close is a finding"
    );
    engine.undo()?;
    assert_eq!(engine.project(), &before);
    Ok(())
}

#[test]
fn a_part_dropped_beside_another_nets_lone_via_keeps_it() -> Fallible {
    for locked in [false, true] {
        let (mut engine, r2) = parts()?;
        let id = add_via(&mut engine, [30.0, 25.0])?;
        do_(&mut engine, LayoutCommand::SetViaLock { id, locked })?;
        crowd(&mut engine, r2, id)?;
    }
    Ok(())
}

fn add_via(engine: &mut Engine, at: Point) -> Fallible<Id> {
    do_(
        engine,
        LayoutCommand::AddVia {
            via: ViaRequest::fixture(A, at),
        },
    )?;
    Ok(engine.project().vias.last().map(|v| v.id).ok_or("no via")?)
}

#[test]
fn a_part_dropped_beside_a_nets_only_layer_change_keeps_it() -> Fallible {
    let (mut engine, r2) = parts()?;
    let id = add_via(&mut engine, [30.0, 25.0])?;
    for (layer, end) in [(Top, [20.0, 25.0]), (CopperLayer::Bottom, [30.0, 35.0])] {
        do_(
            &mut engine,
            LayoutCommand::Route {
                request: RouteRequest {
                    net: A,
                    start: [30.0, 25.0],
                    end,
                    width: 0.3,
                    layer,
                },
            },
        )?;
    }
    crowd(&mut engine, r2, id)
}

#[test]
fn a_part_dropped_beside_a_stitching_via_keeps_the_pours_joined() -> Fallible {
    use crate::layout::connectivity::ElementKind;
    use crate::layout::fill::{FillCommand, RefillTarget};
    use crate::model::fill::{FillRegion, FillSettings, FillSpec};
    let (mut engine, r2) = parts()?;
    let id = add_via(&mut engine, [30.0, 25.0])?;
    for layer in [Top, CopperLayer::Bottom] {
        engine.apply(
            engine.revision(),
            Operation::Fill(FillCommand::Add {
                fill: Box::new(FillSpec {
                    region: FillRegion::Drawn,
                    name: String::new(),
                    net: Some(A),
                    layer,
                    outline: rectangle(22.0, 20.0, 38.0, 32.0),
                    priority: 0,
                    settings: FillSettings::default(),
                }),
            }),
        )?;
    }
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Refill {
            target: RefillTarget::All,
        }),
    )?;
    // Both pours are one island, through the via alone.
    let graph = CopperGraph::of(engine.project());
    let pours: Vec<Option<usize>> = graph
        .elements()
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e.kind, ElementKind::Fill { .. }))
        .map(|(k, _)| graph.island_of(graph.vertices_of(k).start))
        .collect();
    assert!(pours.len() == 2 && pours[0] == pours[1], "{pours:?}");
    crowd(&mut engine, r2, id)?;

    // Selected and dragged with the part onto another net's locked via:
    // the stitch moves with the drag and is still there.
    do_(
        &mut engine,
        LayoutCommand::AddVia {
            via: ViaRequest::fixture(B, [36.0, 25.0]),
        },
    )?;
    let wall = engine.project().vias.last().map(|v| v.id).ok_or("wall")?;
    do_(
        &mut engine,
        LayoutCommand::SetViaLock {
            id: wall,
            locked: true,
        },
    )?;
    let part = engine.project().placement_of(r2).ok_or("r2")?.position;
    do_(
        &mut engine,
        LayoutCommand::MoveParts {
            moves: vec![layout::arrange::PartMove {
                instance: r2,
                position: [part[0] + 5.3, part[1]],
                rotation_deg: 0.0,
                layer: Layer::Top,
            }],
            copper: Some(layout::arrange::CopperMove {
                tracks: Vec::new(),
                vias: vec![id],
                by: [5.3, 0.0],
            }),
        },
    )?;
    assert_eq!(
        engine.project().via(id).map(|v| v.position),
        Some([35.3, 25.0])
    );
    Ok(())
}

/// Another net's stub in the way: nothing can be routed in its place, so
/// it stays and is a finding.
#[test]
fn a_part_dropped_on_another_nets_stub_keeps_it() -> Fallible {
    let (mut engine, r2) = parts()?;
    do_(
        &mut engine,
        LayoutCommand::Route {
            request: RouteRequest {
                net: A,
                start: [30.0, 25.0],
                end: [30.0, 30.0],
                width: 0.3,
                layer: Top,
            },
        },
    )?;
    let before = engine.project().clone();
    let to = beside(&engine, r2, [30.0, 27.0])?;
    engine.apply(engine.revision(), mv(r2, [to[0] - 0.2, to[1]]))?;
    assert_eq!(engine.project().tracks, before.tracks);
    assert!(engine.rerouted().is_empty(), "{:?}", engine.rerouted());
    assert!(!violations(engine.project()).is_empty());
    Ok(())
}

/// A carried via that joined the part to a plane cannot be routed again:
/// the report says the net was left unrouted, not that it was re-routed.
#[test]
fn a_lost_plane_via_of_the_moved_part_is_reported_unrouted() -> Fallible {
    use crate::layout::fill::{FillCommand, RefillTarget};
    use crate::model::fill::{FillRegion, FillSettings, FillSpec};
    let (mut engine, r2) = parts()?;
    let pad = layout::pads(engine.project())
        .into_iter()
        .filter(|p| p.instance == r2)
        .max_by(|a, b| a.position[0].total_cmp(&b.position[0]))
        .ok_or("no pad")?;
    let at = [pad.position[0] + 2.0, pad.position[1]];
    do_(
        &mut engine,
        LayoutCommand::AddVia {
            via: ViaRequest::fixture(pad.net, at),
        },
    )?;
    let id = engine.project().vias.last().map(|v| v.id).ok_or("via")?;
    do_(
        &mut engine,
        LayoutCommand::Route {
            request: RouteRequest {
                net: pad.net,
                start: pad.position,
                end: at,
                width: 0.3,
                layer: Top,
            },
        },
    )?;
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Add {
            fill: Box::new(FillSpec {
                region: FillRegion::Drawn,
                name: String::new(),
                net: Some(pad.net),
                layer: CopperLayer::Bottom,
                outline: rectangle(15.0, 5.0, 40.0, 35.0),
                priority: 0,
                settings: FillSettings::default(),
            }),
        }),
    )?;
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Refill {
            target: RefillTarget::All,
        }),
    )?;
    // A locked via of another net where the carried one lands.
    let wall = add_via(&mut engine, [at[0], 25.2])?;
    do_(
        &mut engine,
        LayoutCommand::SetViaLock {
            id: wall,
            locked: true,
        },
    )?;
    engine.apply(engine.revision(), mv(r2, [25.0, 25.0]))?;
    let report = engine.rerouted();
    let lost = engine.project().via(id).is_none();
    assert!(lost && report.rerouted.is_empty(), "{report:?}");
    assert_eq!(
        report.unrouted,
        vec![engine.project().net_name(pad.net).to_owned()]
    );
    Ok(())
}

fn locked_wall(engine: &mut Engine, start: Point, end: Point) -> Fallible {
    do_(
        engine,
        LayoutCommand::Route {
            request: RouteRequest {
                net: A,
                start,
                end,
                width: 0.3,
                layer: Top,
            },
        },
    )?;
    let id = engine.project().tracks.last().map(|t| t.id).ok_or("wall")?;
    do_(engine, LayoutCommand::SetTrackLock { id, locked: true })
}

fn pad_of(engine: &Engine, part: Id, right: bool) -> Fallible<layout::WorldPad> {
    let x = |p: &layout::WorldPad| if right { -p.position[0] } else { p.position[0] };
    layout::pads(engine.project())
        .into_iter()
        .filter(|p| p.instance == part)
        .min_by(|a, b| x(a).total_cmp(&x(b)))
        .ok_or("no pad".into())
}

/// A stub the moved part carries onto locked copper: nothing could be
/// routed in its place, so it stays and is a finding, not "re-routed".
#[test]
fn a_carried_stub_in_the_way_stays() -> Fallible {
    let (mut engine, r2) = parts()?;
    let pad = pad_of(&engine, r2, true)?;
    let [x, y] = pad.position;
    do_(
        &mut engine,
        LayoutCommand::Route {
            request: RouteRequest {
                net: pad.net,
                start: [x, y],
                end: [x + 3.0, y],
                width: 0.3,
                layer: Top,
            },
        },
    )?;
    let stub = engine.project().tracks.last().map(|t| t.id).ok_or("stub")?;
    locked_wall(&mut engine, [x + 2.0, 22.0], [x + 2.0, 28.0])?;
    engine.apply(engine.revision(), mv(r2, [25.0, 25.0]))?;
    assert!(engine.rerouted().is_empty(), "{:?}", engine.rerouted());
    let at = engine
        .project()
        .tracks
        .iter()
        .find(|t| t.id == stub)
        .map(|t| t.end)
        .ok_or("the stub is gone")?;
    assert!(close(at, [x + 3.0, y + 15.0]), "{at:?}");
    assert!(!violations(engine.project()).is_empty());
    Ok(())
}

/// A second way round between two parts that move together, carried onto
/// locked copper, is laid again: before, its middle was removed as if
/// re-routed and its ends left hanging.
#[test]
fn a_carried_second_way_round_is_laid_again() -> Fallible {
    let (mut engine, r2) = parts()?;
    let (r1, _) = ids()?;
    let (a, b) = ([11.2, 10.0], [23.8, 10.0]);
    for (start, end) in [
        (a, b),
        (a, [11.2, 14.0]),
        ([11.2, 14.0], [23.8, 14.0]),
        ([23.8, 14.0], b),
    ] {
        do_(
            &mut engine,
            LayoutCommand::Route {
                request: RouteRequest {
                    net: SHARED,
                    start,
                    end,
                    width: 0.3,
                    layer: Top,
                },
            },
        )?;
    }
    locked_wall(&mut engine, [17.0, 27.5], [17.0, 33.0])?;
    let to = |instance, x| layout::arrange::PartMove {
        instance,
        position: [x, 25.0],
        rotation_deg: 0.0,
        layer: Layer::Top,
    };
    do_(
        &mut engine,
        LayoutCommand::MoveParts {
            moves: vec![to(r1, 10.0), to(r2, 25.0)],
            copper: None,
        },
    )?;
    let after = engine.project();
    let graph = CopperGraph::of(after);
    let stubs: Vec<_> = graph
        .stubs()
        .into_iter()
        .filter(|s| s.net == SHARED)
        .collect();
    assert!(stubs.is_empty(), "{stubs:?}");
    assert_eq!(
        engine.rerouted().rerouted,
        vec![after.net_name(SHARED).to_owned()]
    );
    assert_eq!(violations(after), Vec::<String>::new());
    Ok(())
}

/// A via carried out of the drawn pour it stood in is named as unrouted:
/// no refill joins it there again.
#[test]
fn a_via_carried_out_of_its_pour_is_named() -> Fallible {
    let (mut engine, r2) = parts()?;
    let pad = pad_of(&engine, r2, true)?;
    do_(
        &mut engine,
        LayoutCommand::AddVia {
            via: ViaRequest::fixture(pad.net, pad.position),
        },
    )?;
    engine.apply(
        engine.revision(),
        Operation::Fill(layout::fill::FillCommand::Add {
            fill: Box::new(crate::model::fill::FillSpec {
                region: crate::model::fill::FillRegion::Drawn,
                name: String::new(),
                net: Some(pad.net),
                layer: CopperLayer::Bottom,
                outline: rectangle(20.0, 5.0, 32.0, 15.0),
                priority: 0,
                settings: crate::model::fill::FillSettings::default(),
            }),
        }),
    )?;
    engine.apply(engine.revision(), mv(r2, [25.0, 12.0]))?;
    assert!(engine.rerouted().is_empty(), "still in it");
    engine.apply(engine.revision(), mv(r2, [45.0, 25.0]))?;
    assert_eq!(
        engine.rerouted().unrouted,
        vec![engine.project().net_name(pad.net).to_owned()]
    );
    Ok(())
}

/// A shove moves a via within the pour it stands in, never out of it.
#[test]
fn a_shoved_via_stays_in_its_pour() -> Fallible {
    let mut p = board()?;
    let v = via(&mut p, A, [30.0, 20.2])?;
    layout::fill::apply(
        &mut p,
        layout::fill::FillCommand::Add {
            fill: Box::new(crate::model::fill::FillSpec {
                region: crate::model::fill::FillRegion::Drawn,
                name: String::new(),
                net: Some(A),
                layer: CopperLayer::Bottom,
                outline: rectangle(28.0, 18.0, 32.0, 20.22),
                priority: 0,
                settings: crate::model::fill::FillSettings::default(),
            }),
        },
    )?;
    let plan = run(
        &p,
        &request(B, [25.0, 19.6], [35.0, 19.6], RouteMode::Shove),
    );
    commit(&mut p, &plan)?;
    let y = p.via(v).ok_or("via")?.position[1];
    assert!((18.0..=20.22).contains(&y), "shoved out to {y}");
    Ok(())
}

fn pour(engine: &mut Engine, net: Id, layer: CopperLayer, outline: Outline) -> Fallible {
    engine.apply(
        engine.revision(),
        Operation::Fill(layout::fill::FillCommand::Add {
            fill: Box::new(crate::model::fill::FillSpec {
                region: crate::model::fill::FillRegion::Drawn,
                name: String::new(),
                net: Some(net),
                layer,
                outline,
                priority: 0,
                settings: crate::model::fill::FillSettings::default(),
            }),
        }),
    )?;
    Ok(())
}

/// A carried piece that meets three places, two of them joined elsewhere
/// too, is laid again to all three: before, one leg of the second way
/// round was dropped and left hanging.
#[test]
fn a_carried_piece_of_three_ends_keeps_every_leg() -> Fallible {
    let (mut engine, r2) = parts()?;
    let (r1, _) = ids()?;
    let (a, b) = ([11.2, 10.0], [23.8, 10.0]);
    let mut legs = Vec::new();
    for (start, end) in [
        (a, b),
        (a, [11.2, 14.0]),
        ([11.2, 14.0], [23.8, 14.0]),
        ([23.8, 14.0], b),
        ([17.5, 14.0], [17.5, 16.0]),
    ] {
        do_(
            &mut engine,
            LayoutCommand::Route {
                request: RouteRequest {
                    net: SHARED,
                    start,
                    end,
                    width: 0.3,
                    layer: Top,
                },
            },
        )?;
        legs.push(engine.project().tracks.last().map(|t| t.id).ok_or("leg")?);
    }
    locked_wall(&mut engine, [14.0, 27.5], [14.0, 33.0])?;
    let to = |instance, x| layout::arrange::PartMove {
        instance,
        position: [x, 25.0],
        rotation_deg: 0.0,
        layer: Layer::Top,
    };
    do_(
        &mut engine,
        LayoutCommand::MoveParts {
            moves: vec![to(r1, 10.0), to(r2, 25.0)],
            copper: None,
        },
    )?;
    let after = engine.project();
    // Only the stub that hung before hangs now, and it is still there.
    let hanging: Vec<Id> = CopperGraph::of(after)
        .stubs()
        .into_iter()
        .filter(|s| s.net == SHARED)
        .flat_map(|s| s.tracks)
        .collect();
    assert_eq!(hanging, [legs[4]]);
    assert!(engine.rerouted().unrouted.is_empty());
    assert_eq!(violations(after), Vec::<String>::new());
    Ok(())
}

/// A track carried out of the drawn pour it ended in is named as unrouted.
#[test]
fn a_track_carried_out_of_its_pour_is_named() -> Fallible {
    let (mut engine, r2) = parts()?;
    let pad = pad_of(&engine, r2, true)?;
    let [x, y] = pad.position;
    do_(
        &mut engine,
        LayoutCommand::Route {
            request: RouteRequest {
                net: pad.net,
                start: [x, y],
                end: [x + 3.0, y],
                width: 0.3,
                layer: Top,
            },
        },
    )?;
    pour(
        &mut engine,
        pad.net,
        Top,
        rectangle(x + 2.0, y - 2.0, x + 6.0, y + 2.0),
    )?;
    engine.apply(engine.revision(), mv(r2, [25.5, 10.5]))?;
    assert!(engine.rerouted().is_empty(), "still in it");
    engine.apply(engine.revision(), mv(r2, [45.0, 25.0]))?;
    assert_eq!(
        engine.rerouted().unrouted,
        vec![engine.project().net_name(pad.net).to_owned()]
    );
    Ok(())
}

/// A shove that would push a chain's middle out of the pour it crosses is
/// not made.
#[test]
fn a_shoved_chain_stays_in_the_pour_its_middle_crosses() -> Fallible {
    let mut p = board()?;
    track(&mut p, A, [25.0, 20.0], [35.0, 20.0])?;
    layout::fill::apply(
        &mut p,
        layout::fill::FillCommand::Add {
            fill: Box::new(crate::model::fill::FillSpec {
                region: crate::model::fill::FillRegion::Drawn,
                name: String::new(),
                net: Some(A),
                layer: Top,
                outline: rectangle(29.8, 19.8, 30.2, 20.2),
                priority: 0,
                settings: crate::model::fill::FillSettings::default(),
            }),
        },
    )?;
    let plan = run(
        &p,
        &request(B, [30.0, 14.0], [30.0, 19.95], RouteMode::Shove),
    );
    commit(&mut p, &plan)?;
    let lines: Vec<(Point, Point)> = p
        .tracks
        .iter()
        .filter(|t| t.net == A)
        .map(|t| (t.start, t.end))
        .collect();
    let met = crate::layout::connectivity::pours_met(&p, A, Top, &lines);
    assert_eq!(met.len(), 1, "{lines:?}");
    Ok(())
}

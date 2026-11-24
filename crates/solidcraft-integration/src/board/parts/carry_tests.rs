//! A solved part carries the copper attached to its pads
//! (`sparksmith_core::layout::dimensions::carry`): a board-parameter edit
//! that moves a dimensioned part keeps its tracks connected, in one undo
//! step.
#![allow(clippy::unwrap_used)]

use super::super::tests::{Fallible, apply, close, created_curves_of, edit, set, sketched};
use sparksmith_core::geometry::Point;
use sparksmith_core::ids::Id;
use sparksmith_core::layout::connectivity::CopperGraph;
use sparksmith_core::layout::dimensions::{DimensionCommand, carry};
use sparksmith_core::layout::outline::BoardEdit;
use sparksmith_core::layout::{self, LayoutCommand, RouteEditSet, RouteRequest, WorldPad};
use sparksmith_core::library::footprint_source::{DimKind, DimValue, Expr, SketchEdit, SketchRef};
use sparksmith_core::model::board::{BoardShape, ConstraintTarget};
use sparksmith_core::model::dimension::{BoardRef, FeatureRef, PartDimensionSpec, PartRelation};
use sparksmith_core::{CopperLayer, Engine, Operation, Project, Via, ViaSpan};

fn pad(project: &Project, instance: Id, n: usize) -> Fallible<WorldPad> {
    layout::pads(project)
        .into_iter()
        .filter(|p| p.instance == instance)
        .nth(n)
        .ok_or_else(|| "pad".into())
}

fn board_codes(project: &Project) -> Vec<String> {
    let mut codes: Vec<String> = layout::checks(project, &Default::default())
        .into_iter()
        .map(|f| f.code.to_string())
        .collect();
    codes.sort();
    codes
}

fn unrouted(project: &Project) -> usize {
    layout::checks(project, &Default::default())
        .iter()
        .filter(|f| f.code == "board.unrouted")
        .count()
}

/// The demo's two resistors, the second held at half the board's width
/// from its left edge, joined by two tracks with a corner between them.
fn held_and_routed() -> Fallible<(Engine, Id)> {
    let mut engine = sketched()?;
    let p = &engine.project().placements;
    let (r1, r2) = (
        p.first().ok_or("no part")?.instance,
        p.get(1).ok_or("no second part")?.instance,
    );
    let BoardShape::Sketch(sketch) = &engine.project().board_shape else {
        return Err("not a sketch".into());
    };
    let left = sketch
        .edges
        .iter()
        .find(|e| e.name.0 == "left")
        .map(|e| e.curve.clone())
        .ok_or("no left edge")?;
    apply(
        &mut engine,
        Operation::Dimension(DimensionCommand::Add {
            dimension: PartDimensionSpec {
                kind: PartRelation::Dim(DimKind::Distance),
                refs: vec![
                    BoardRef::Part(FeatureRef::Origin {
                        target: ConstraintTarget::Placement(r2),
                    }),
                    BoardRef::Board(SketchRef::Curve(left)),
                ],
                value: Some(DimValue::Expr(Expr("width / 2".into()))),
                reference: false,
            },
        }),
    )?;
    let (a, b) = (pad(engine.project(), r1, 1)?, pad(engine.project(), r2, 0)?);
    assert_eq!(a.net, b.net);
    let corner = [20.0, 14.0];
    let track = |start: Point, end: Point| RouteRequest {
        net: a.net,
        start,
        end,
        width: 0.25,
        layer: CopperLayer::Top,
    };
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::ApplyRoute {
            edits: RouteEditSet {
                add: vec![track(a.position, corner), track(corner, b.position)],
                ..Default::default()
            },
        }),
    )?;
    assert_eq!(unrouted(engine.project()), 0);
    Ok((engine, r2))
}

/// The VM's J1 case: a part held at `width / 2` with two routed tracks.
/// Widening the board moves the part; the track on its pad slides with it
/// and the first track stretches along its own line to meet it, so both
/// keep their directions: no unconnected item, no new finding, and undo
/// restores the board exactly.
#[test]
fn a_part_held_at_half_the_width_keeps_its_tracks_when_the_width_changes() -> Fallible<()> {
    let (mut engine, r2) = held_and_routed()?;
    let codes = board_codes(engine.project());
    let saved = engine.to_file();
    let tracks = engine.project().tracks.clone();
    edit(&mut engine, vec![set("width", "80 mm")])?;
    let project = engine.project();
    let moved = pad(project, r2, 0)?;
    assert!((project.placement_of(r2).ok_or("placement")?.position[0] - 40.0).abs() < 1e-6);
    let first = project.tracks.first().ok_or("track")?;
    let second = project.tracks.get(1).ok_or("track")?;
    let (was_first, was_second) = (
        tracks.first().ok_or("track")?,
        tracks.get(1).ok_or("track")?,
    );
    let way = |t: &sparksmith_core::Track| {
        let (dx, dy) = (t.end[0] - t.start[0], t.end[1] - t.start[1]);
        let length = dx.hypot(dy);
        [dx / length, dy / length]
    };
    let same_way = |a: [f64; 2], b: [f64; 2]| close([a[0] - b[0], a[1] - b[1]], [0.0, 0.0]);
    // The first track keeps its start and direction; the corner moves
    // along it, and the second track meets it there, slid with the pad.
    assert!(close(first.start, was_first.start), "{first:?}");
    assert!(same_way(way(first), way(was_first)), "{first:?}");
    assert!(close(second.start, first.end), "{second:?}");
    assert!(same_way(way(second), way(was_second)), "{second:?}");
    assert!(close(second.end, moved.position), "{:?}", second.end);
    assert_eq!(unrouted(project), 0);
    assert_eq!(board_codes(project), codes);
    engine.undo()?;
    assert_eq!(engine.to_file(), saved);
    Ok(())
}

/// Two parts that slide two ways in one edit, one with the board's width
/// and one with its height: the route between them goes with the shorter
/// slide and then keeps every segment's direction as the other part slides
/// the rest of the way, as it does between a moved and an unmoved part.
#[test]
fn a_route_between_parts_that_slide_two_ways_keeps_its_directions() -> Fallible<()> {
    let (mut engine, r2) = held_and_routed()?;
    let r1 = engine
        .project()
        .placements
        .first()
        .ok_or("no part")?
        .instance;
    let BoardShape::Sketch(sketch) = &engine.project().board_shape else {
        return Err("not a sketch".into());
    };
    let top = sketch
        .edges
        .iter()
        .find(|e| e.name.0 == "top")
        .map(|e| e.curve.clone())
        .ok_or("no top edge")?;
    let [w, h] = engine.project().board_size;
    let below = h - engine
        .project()
        .placement_of(r1)
        .ok_or("placement")?
        .position[1];
    apply(
        &mut engine,
        Operation::Dimension(DimensionCommand::Add {
            dimension: PartDimensionSpec {
                kind: PartRelation::Dim(DimKind::Distance),
                refs: vec![
                    BoardRef::Part(FeatureRef::Origin {
                        target: ConstraintTarget::Placement(r1),
                    }),
                    BoardRef::Board(SketchRef::Curve(top)),
                ],
                value: Some(DimValue::Fixed(below)),
                reference: false,
            },
        }),
    )?;
    let codes = board_codes(engine.project());
    let before = engine.project().clone();
    edit(
        &mut engine,
        vec![
            set("width", &format!("{} mm", w + 10.0)),
            set("height", &format!("{} mm", h + 4.0)),
        ],
    )?;
    let project = engine.project();
    let slid = |part: Id| -> Fallible<Point> {
        let (was, now) = (
            before.placement_of(part).ok_or("placement")?.position,
            project.placement_of(part).ok_or("placement")?.position,
        );
        Ok([now[0] - was[0], now[1] - was[1]])
    };
    assert!(close(slid(r1)?, [0.0, 4.0]), "{:?}", slid(r1)?);
    assert!(close(slid(r2)?, [5.0, 0.0]), "{:?}", slid(r2)?);
    let way = |t: &sparksmith_core::Track| {
        let (dx, dy) = (t.end[0] - t.start[0], t.end[1] - t.start[1]);
        let length = dx.hypot(dy);
        [dx / length, dy / length]
    };
    assert_eq!(project.tracks.len(), before.tracks.len());
    for (now, was) in project.tracks.iter().zip(before.tracks.iter()) {
        assert!(close(way(now), way(was)), "{now:?} was {was:?}");
    }
    let (first, second) = (
        project.tracks.first().ok_or("track")?,
        project.tracks.get(1).ok_or("track")?,
    );
    assert!(
        close(first.start, pad(project, r1, 1)?.position),
        "{first:?}"
    );
    assert!(close(second.start, first.end), "{second:?}");
    assert!(
        close(second.end, pad(project, r2, 0)?.position),
        "{second:?}"
    );
    assert_eq!(unrouted(project), 0);
    assert_eq!(board_codes(project), codes);
    Ok(())
}

/// A board made narrower than its copper stands is no refusal when the
/// copper follows: the part held at half the width carries its tracks back
/// onto the narrower board, and the copper is checked against the new size
/// where the carry left it.
#[test]
fn a_narrower_board_takes_the_copper_its_held_part_carries() -> Fallible<()> {
    let (mut engine, r2) = held_and_routed()?;
    let width = engine.project().board_size[0];
    let tracks = engine.project().tracks.clone();
    let reach = |p: &Project| {
        p.tracks
            .iter()
            .flat_map(|t| [t.start[0], t.end[0]])
            .fold(0.0, f64::max)
    };
    edit(
        &mut engine,
        vec![set("width", &format!("{} mm", 3.0 * width))],
    )?;
    assert!(
        reach(engine.project()) > width,
        "{}",
        reach(engine.project())
    );
    edit(&mut engine, vec![set("width", &format!("{width} mm"))])?;
    let project = engine.project();
    assert_eq!(project.board_size[0], width);
    let at = project.placement_of(r2).ok_or("placement")?.position;
    assert!((at[0] - width / 2.0).abs() < 1e-6, "{at:?}");
    assert!(reach(project) < width, "{}", reach(project));
    assert_eq!(unrouted(project), 0);
    for (now, was) in project.tracks.iter().zip(tracks.iter()) {
        assert!(
            close(now.start, was.start) && close(now.end, was.end),
            "{now:?}"
        );
    }
    Ok(())
}

/// A turned and moved part carries a via on its pad and the track ends on
/// both: each lands where the part's frame takes it. A locked track stays.
#[test]
fn a_turned_part_carries_its_via_and_track_ends_rigidly() -> Fallible<()> {
    let (engine, r2) = held_and_routed()?;
    let before = engine.project().clone();
    let pad_at = pad(&before, r2, 0)?.position;
    let net = pad(&before, r2, 0)?.net;
    let mut with_via = before.clone();
    let id = with_via.allocate()?;
    with_via.vias.push(Via {
        id,
        net,
        position: pad_at,
        drill: 0.3,
        diameter: 0.6,
        span: ViaSpan::Through,
        tented: true,
        locked: false,
    });
    let mut after = with_via.clone();
    let p = after
        .placements
        .iter_mut()
        .find(|p| p.instance == r2)
        .ok_or("placement")?;
    let origin = p.position;
    p.position = [origin[0] + 3.0, origin[1] + 1.0];
    p.rotation_deg += 90.0;
    carry(
        &with_via,
        &mut after,
        &CopperGraph::of(&with_via),
        &[r2],
        None,
    )?;
    let landed = pad(&after, r2, 0)?.position;
    let via = after.via(id).ok_or("via")?;
    assert!(close(via.position, landed), "{:?} {landed:?}", via.position);
    let second = after.tracks.get(1).ok_or("track")?;
    assert!(close(second.end, landed), "{:?}", second.end);
    assert_eq!(after.tracks.first(), with_via.tracks.first());

    // Locked, the track stays where it was; the via still goes.
    let mut locked = with_via.clone();
    if let Some(t) = locked.tracks.get_mut(1) {
        t.locked = true;
    }
    let mut after = locked.clone();
    if let Some(p) = after.placements.iter_mut().find(|p| p.instance == r2) {
        p.position = [origin[0] + 3.0, origin[1] + 1.0];
    }
    carry(&locked, &mut after, &CopperGraph::of(&locked), &[r2], None)?;
    assert_eq!(after.tracks.get(1), locked.tracks.get(1));
    assert!(close(
        after.via(id).ok_or("via")?.position,
        [pad_at[0] + 3.0, pad_at[1] + 1.0]
    ));
    Ok(())
}

/// A locked routed segment cannot follow its dimensioned pad. The board
/// edit must refuse, including when the router reports no failed route
/// because the locked copper was not eligible for rerouting.
#[test]
fn a_board_edit_that_detaches_locked_copper_is_atomic() -> Fallible<()> {
    let (mut engine, _) = held_and_routed()?;
    let track = engine.project().tracks.last().ok_or("track")?.id;
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::SetTrackLock {
            id: track,
            locked: true,
        }),
    )?;
    let before = engine.to_file();
    let revision = engine.revision();
    let error = edit(&mut engine, vec![set("width", "80 mm")])
        .err()
        .ok_or("lost connection committed")?;
    assert!(
        error
            .to_string()
            .to_lowercase()
            .contains("cannot keep connected copper"),
        "{error}"
    );
    assert_eq!(engine.to_file(), before);
    assert_eq!(engine.revision(), revision);
    // The refused edit left the previous lock operation at the top of undo.
    engine.undo()?;
    assert!(
        !engine
            .project()
            .tracks
            .iter()
            .find(|t| t.id == track)
            .ok_or("track")?
            .locked
    );
    engine.redo()?;
    assert_eq!(engine.to_file(), before);
    Ok(())
}

#[test]
fn a_board_edit_carries_a_via_or_refuses_when_it_is_locked() -> Fallible<()> {
    let (mut engine, r2) = held_and_routed()?;
    let old_pad = pad(engine.project(), r2, 0)?;
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::AddVia {
            via: layout::ViaRequest::fixture(old_pad.net, old_pad.position),
        }),
    )?;
    let via = engine.project().vias.last().ok_or("via")?.id;
    let before = engine.to_file();
    edit(&mut engine, vec![set("width", "80 mm")])?;
    assert!(close(
        engine.project().via(via).ok_or("via")?.position,
        pad(engine.project(), r2, 0)?.position
    ));
    assert_eq!(unrouted(engine.project()), 0);
    engine.undo()?;
    assert_eq!(engine.to_file(), before);
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::SetViaLock {
            id: via,
            locked: true,
        }),
    )?;
    let before = engine.to_file();
    let revision = engine.revision();
    let error = edit(&mut engine, vec![set("width", "80 mm")])
        .err()
        .ok_or("detached locked via committed")?;
    assert!(
        error
            .to_string()
            .to_lowercase()
            .contains("cannot keep connected copper"),
        "{error}"
    );
    assert_eq!(engine.to_file(), before);
    assert_eq!(engine.revision(), revision);
    Ok(())
}

#[test]
fn the_layout_board_size_alias_also_refuses_lost_copper() -> Fallible<()> {
    let (mut engine, _) = held_and_routed()?;
    let size = engine.project().board_size;
    apply(
        &mut engine,
        Operation::Board(sparksmith_core::layout::outline::BoardCommand::SetRectangle { size }),
    )?;
    let track = engine.project().tracks.last().ok_or("track")?.id;
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::SetTrackLock {
            id: track,
            locked: true,
        }),
    )?;
    let before = engine.to_file();
    let revision = engine.revision();
    let error = apply(
        &mut engine,
        Operation::Layout(LayoutCommand::SetBoardSize {
            size: [80.0, size[1]],
        }),
    )
    .err()
    .ok_or("lost connection committed by layout.set_board_size")?;
    assert!(
        error
            .to_string()
            .to_lowercase()
            .contains("cannot keep connected copper"),
        "{error}"
    );
    assert_eq!(engine.to_file(), before);
    assert_eq!(engine.revision(), revision);
    Ok(())
}

/// Draw a circle off the board: two regions are no board, so its solve is no
/// longer current.
fn invalidate(engine: &mut Engine) -> Fallible<BoardEdit> {
    let circle = created_curves_of(edit(
        engine,
        vec![BoardEdit::Sketch {
            edit: SketchEdit::AddCircle {
                center: [150.0, 20.0],
                radius: 2.0,
                construction: false,
            },
        }],
    )?);
    assert!(engine.project().board_shape.invalid().is_some());
    Ok(BoardEdit::Sketch {
        edit: SketchEdit::Delete {
            ids: circle.into_iter().map(SketchRef::Curve).collect(),
        },
    })
}

/// While the outline is invalid a changed parameter moves no part, track or
/// via; they stay with the stored shape. The edit that makes it valid moves
/// them as the parameter's own edit does on a valid board, in one undo step.
#[test]
fn a_held_part_and_its_copper_wait_for_a_valid_outline() -> Fallible<()> {
    let (mut engine, r2) = held_and_routed()?;
    let on_pad = pad(engine.project(), r2, 0)?;
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::AddVia {
            via: layout::ViaRequest::fixture(on_pad.net, on_pad.position),
        }),
    )?;
    let start = engine.project().clone();
    let mut direct = Engine::from_file(engine.to_file())?;
    edit(&mut direct, vec![set("width", "80 mm")])?;
    let same = |a: &Project, b: &Project| {
        assert_eq!(a.board_size, b.board_size);
        assert_eq!(a.placements, b.placements);
        assert_eq!(a.tracks, b.tracks);
        assert_eq!(a.vias, b.vias);
    };

    let fix = invalidate(&mut engine)?;
    edit(&mut engine, vec![set("width", "80 mm")])?;
    same(engine.project(), &start);
    let waiting = engine.to_file();

    edit(&mut engine, vec![fix])?;
    let (now, want) = (engine.project(), direct.project());
    assert!(now.board_shape.invalid().is_none());
    let at = now.placement_of(r2).ok_or("placement")?.position;
    assert!((at[0] - 40.0).abs() < 1e-6, "{at:?}");
    let pose = |p: &Project| p.placement_of(r2).map(|p| p.position);
    assert!(close(at, pose(want).ok_or("direct")?));
    assert_eq!(now.tracks.len(), want.tracks.len());
    for (a, b) in now.tracks.iter().zip(want.tracks.iter()) {
        assert!(
            close(a.start, b.start) && close(a.end, b.end),
            "{a:?} {b:?}"
        );
    }
    let vias = |p: &Project| p.vias.iter().map(|v| v.position).collect::<Vec<_>>();
    let (a, b) = (vias(now), vias(want));
    assert!(a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| close(*a, *b)));
    assert_eq!(unrouted(now), 0);

    engine.undo()?;
    assert_eq!(engine.to_file(), waiting);
    engine.undo()?;
    engine.undo()?;
    same(engine.project(), &start);
    assert_eq!(engine.project().board_shape, start.board_shape);
    Ok(())
}

/// While the outline is invalid a held part's dimension and the part itself
/// are not changed, in plain words; a dimension can be deleted, and a part
/// nothing holds moves.
#[test]
fn a_dimension_edit_waits_for_a_valid_outline() -> Fallible<()> {
    let (mut engine, r2) = held_and_routed()?;
    invalidate(&mut engine)?;
    let (revision, before) = (engine.revision(), engine.to_file());
    let mut dimension = engine
        .project()
        .part_dimensions
        .iter()
        .next()
        .ok_or("a dimension")?
        .clone();
    let id = dimension.id;
    dimension.value = Some(DimValue::Expr(Expr("width / 3".into())));
    let at = engine
        .project()
        .placement_of(r2)
        .ok_or("placement")?
        .clone();
    let moved = || {
        Operation::Layout(LayoutCommand::Move {
            instance: at.instance,
            position: [at.position[0] + 1.0, at.position[1]],
            rotation_deg: at.rotation_deg,
            layer: at.layer,
        })
    };
    for operation in [
        Operation::Dimension(DimensionCommand::Edit { dimension }),
        moved(),
    ] {
        let error = apply(&mut engine, operation).err().ok_or("accepted")?;
        assert_eq!(error.rule(), Some("layout.dimension_outline"));
        assert!(error.to_string().contains("not valid"), "{error}");
        assert_eq!(engine.revision(), revision);
        assert_eq!(engine.to_file(), before);
    }
    apply(
        &mut engine,
        Operation::Dimension(DimensionCommand::Delete { id }),
    )?;
    apply(&mut engine, moved())?;
    Ok(())
}

/// A parameter the held parts cannot be solved with is refused while the
/// outline is invalid as it is on a valid board, changing nothing.
#[test]
fn an_unsolvable_parameter_is_refused_while_the_outline_is_invalid() -> Fallible<()> {
    let (mut engine, _) = held_and_routed()?;
    let mut dimension = engine
        .project()
        .part_dimensions
        .iter()
        .next()
        .ok_or("a dimension")?
        .clone();
    // The same place on the 60 mm board; below 30 mm it has no length.
    dimension.value = Some(DimValue::Expr(Expr("width - 30 mm".into())));
    apply(
        &mut engine,
        Operation::Dimension(DimensionCommand::Edit { dimension }),
    )?;
    invalidate(&mut engine)?;
    let (revision, before) = (engine.revision(), engine.to_file());
    let error = edit(&mut engine, vec![set("width", "28 mm")])
        .err()
        .ok_or("accepted")?
        .to_string();
    assert!(error.contains("out of range"), "{error}");
    assert_eq!(engine.revision(), revision);
    assert_eq!(engine.to_file(), before);
    Ok(())
}

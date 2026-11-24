//! The physical inventory against what is checked and made: a small
//! generic board with one deliberate violation of each kind the checks
//! once let through (a fiducial across a net, a plated washer between two
//! nets, a fill whose stored copper shorts a track, a track under its
//! minimum width, a pad drill under the minimum and a track through a
//! keepout). Every expected position is derived by hand from the fixture
//! coordinates below.
use super::*;
use crate::diagnostics::{Error, Finding, Result};
use crate::geometry::Point;
use crate::layout::fill::{FillCommand, inputs_key};
use crate::layout::{LayoutCommand, RouteEditSet, RouteRequest};
use crate::library::{Hole as PadHoleSpec, Outline, Pad, PadKind, PadShape, PadSide, Segment};
use crate::model::board::{BoardShape, BoardSketch, SolvedBoard, SolvedMountingHole};
use crate::model::fill::{FillSpec, FilledCopper, FilledRegion, KeepoutSpec};
use crate::model::{CopperLayer, Track};
use crate::rules::{RuleKey, RuleValue, RulesCommand};
use crate::tests::support::{by_designator, nm, pt};
use crate::{Engine, Operation};

/// The minimum track width and drill the fixture sets, mm.
pub(crate) const TRACK_MIN_MM: f64 = 0.2;
pub(crate) const DRILL_MIN_MM: f64 = 0.2;

/// The deliberate violations of [`gate_board`], by what they are.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Gate {
    pub r1: Id,
    pub fiducial: PadId,
    /// The track across R1's fiducial (net A).
    pub over_fiducial: Id,
    /// The tracks of net A and net B touching the washer.
    pub washer_a: Id,
    pub washer_b: Id,
    /// The plated mounting hole.
    pub hole: BoardFeatureId,
    /// The 0.1 mm track on net B.
    pub thin: Id,
    /// R1's pad 1, drilled 0.08 mm.
    pub drilled: PadId,
    /// The track through the keepout (net A).
    pub through_keepout: Id,
    pub keepout: Id,
    /// The net B fill and the net A bottom track its stored copper covers.
    pub fill: Id,
    pub under_fill: Id,
    pub net_a: Id,
    pub net_b: Id,
}

pub(crate) fn rect(min: Point, max: Point) -> Outline {
    Outline {
        start: min,
        segments: vec![
            Segment::Line {
                to: [max[0], min[1]],
            },
            Segment::Line { to: max },
            Segment::Line {
                to: [min[0], max[1]],
            },
            Segment::Line { to: min },
        ],
    }
}

fn track(id: Id, net: Id, layer: CopperLayer, start: Point, end: Point, width: f64) -> Track {
    Track {
        id,
        net,
        start,
        end,
        width,
        layer,
        locked: false,
    }
}

/// The parts of the gate board chosen by `with`: the demo board (R1 at
/// (8, 8), R2 at (16, 8), 60 × 40 mm) whose shared footprint gains a
/// 1 mm fiducial at (0, 3) (no terminal, no net) and whose pad 1 becomes a
/// plated through pad drilled 0.08 mm; a solved sketch of the board with
/// a 3 mm plated mounting hole and a 6 mm washer at (40, 20); and the
/// tracks, keepout and fill below. Tracks are written into the document
/// as an imported file would carry them: commands now refuse them.
pub(crate) fn gate_board_with(with: &[&str]) -> Result<(Engine, Gate)> {
    let want = |what: &str| with.is_empty() || with.contains(&what);
    let demo = Engine::demo()?;
    let r1 = by_designator(&demo, "R1")?;
    let mut file = demo.to_file();
    let project = &mut file.project;
    let net_a = project.nets.first().map(|n| n.id);
    let net_b = project.nets.get(1).map(|n| n.id);
    let (Some(net_a), Some(net_b)) = (net_a, net_b) else {
        return Err(Error::NotFound("two demo nets".into()));
    };
    let footprint = &mut project
        .library
        .footprints
        .first_mut()
        .ok_or_else(|| Error::NotFound("the demo footprint".into()))?
        .definition;
    let fiducial = PadId(footprint.next_local);
    footprint.next_local += 1;
    let drilled = footprint
        .pads
        .first()
        .map(|p| p.id)
        .ok_or_else(|| Error::NotFound("pad 1".into()))?;
    if want("fiducial") {
        footprint.pads.push(Pad {
            id: fiducial,
            kind: PadKind::Fiducial,
            terminal: None,
            number: String::new(),
            position: pt([0.0, 3.0]),
            rotation_mdeg: Default::default(),
            side: PadSide::Top,
            shape: PadShape::Circle {
                diameter_nm: nm(1.0),
            },
            hole: None,
            mask: Default::default(),
            paste: Default::default(),
            fiducial_role: None,
            vias: Vec::new(),
        });
    }
    if want("drill")
        && let Some(pad) = footprint.pads.first_mut()
    {
        pad.kind = PadKind::PlatedThrough;
        pad.hole = Some(PadHoleSpec::round_mm(0.08, true)?);
    }
    let mut solved = SolvedBoard::rectangle(project.board_size);
    let hole = BoardFeatureId(1);
    if want("washer") {
        solved.holes.push(SolvedMountingHole {
            id: hole,
            at: [40.0, 20.0],
            drill: 3.0,
            plated: true,
            pad_diameter: Some(6.0),
        });
    }
    project.board_shape = BoardShape::Sketch(Box::new(BoardSketch {
        evaluator: Default::default(),
        sketch: Default::default(),
        parameters: Vec::new(),
        cutouts: Vec::new(),
        holes: Vec::new(),
        edges: Vec::new(),
        next_feature: 2,
        solve: Default::default(),
        solved,
        current: true,
        outline_fit: 0.01,
        fault: None,
        idle: Vec::new(),
    }));
    for (key, mm) in [
        (RuleKey::TrackWidthMin, TRACK_MIN_MM),
        (RuleKey::DrillMin, DRILL_MIN_MM),
    ] {
        crate::rules::apply(
            &mut project.rules,
            RulesCommand::SetOverride {
                key,
                value: RuleValue::Length(mm),
            },
        )?;
    }
    let top = CopperLayer::Top;
    let bottom = CopperLayer::Bottom;
    let mut ids = [0; 7];
    for id in &mut ids {
        *id = project.allocate()?;
    }
    let [
        over_fiducial,
        washer_a,
        washer_b,
        thin,
        through_keepout,
        under_fill,
        keepout,
    ] = ids;
    // R1's fiducial is at (8, 11): the track runs from 2 mm left of it to
    // 2 mm right, through its centre.
    if want("fiducial") {
        project.tracks.push(track(
            over_fiducial,
            net_a,
            top,
            [6.0, 11.0],
            [10.0, 11.0],
            0.25,
        ));
    }
    // 2.5 mm above and below the hole's centre: each track's copper comes
    // within 2.375 mm, inside the 3 mm washer, while 0.875 mm clear of
    // the 1.5 mm drill.
    if want("washer") {
        project.tracks.push(track(
            washer_a,
            net_a,
            top,
            [35.0, 22.5],
            [45.0, 22.5],
            0.25,
        ));
        project.tracks.push(track(
            washer_b,
            net_b,
            top,
            [35.0, 17.5],
            [45.0, 17.5],
            0.25,
        ));
    }
    if want("thin") {
        project
            .tracks
            .push(track(thin, net_b, top, [4.0, 34.0], [14.0, 34.0], 0.1));
    }
    if want("keepout") {
        project.keepouts.push(crate::model::Keepout::from_spec(
            keepout,
            KeepoutSpec {
                name: "Antenna".into(),
                outline: rect([20.0, 25.0], [30.0, 35.0]),
                layers: Default::default(),
                forbid: Default::default(),
            },
        ));
        project.tracks.push(track(
            through_keepout,
            net_a,
            top,
            [18.0, 30.0],
            [32.0, 30.0],
            0.25,
        ));
    }
    if want("fill") {
        project.tracks.push(track(
            under_fill,
            net_a,
            bottom,
            [44.0, 34.0],
            [56.0, 34.0],
            0.25,
        ));
    }
    let mut engine = Engine::from_file(file)?;
    let mut fill = 0;
    if want("fill") {
        // A fill on net B whose stored copper a worker computed wrongly:
        // its rectangle covers the net A track at y = 34, under the
        // current inputs key.
        let outcome = engine.apply(
            engine.revision(),
            Operation::Fill(FillCommand::Add {
                fill: Box::new(FillSpec {
                    name: "Return".into(),
                    net: Some(net_b),
                    layer: bottom,
                    region: Default::default(),
                    outline: rect([45.0, 30.0], [55.0, 38.0]),
                    priority: 0,
                    settings: Default::default(),
                }),
            }),
        )?;
        let crate::Outcome::FillAdded { fill: id } = outcome else {
            return Err(Error::Invalid(format!("fill.add gave {outcome:?}")));
        };
        fill = id;
        let key = inputs_key(engine.project(), fill)?;
        engine.apply(
            engine.revision(),
            Operation::Fill(FillCommand::Store {
                fill,
                copper: Box::new(FilledCopper {
                    key,
                    regions: vec![FilledRegion {
                        outer: vec![[46.0, 31.0], [54.0, 31.0], [54.0, 37.0], [46.0, 37.0]],
                        holes: Vec::new(),
                    }],
                    removed_islands: 0,
                    removed_spokes: Vec::new(),
                }),
            }),
        )?;
    }
    Ok((
        engine,
        Gate {
            r1,
            fiducial,
            over_fiducial,
            washer_a,
            washer_b,
            hole,
            thin,
            drilled,
            through_keepout,
            keepout,
            fill,
            under_fill,
            net_a,
            net_b,
        },
    ))
}

/// Every deliberate violation of the gate board.
pub(crate) fn gate_board() -> Result<(Engine, Gate)> {
    gate_board_with(&[])
}

fn findings(engine: &Engine, code: &str) -> Vec<Finding> {
    crate::layout::checks(engine.project(), &Default::default())
        .into_iter()
        .chain(crate::layout::fill::findings(engine.project()))
        .filter(|f| f.code == code)
        .collect()
}

fn names(findings: &[Finding], object: &ObjectRef) -> bool {
    findings.iter().any(|f| f.objects.contains(object))
}

#[test]
fn every_pad_with_copper_is_a_land_with_or_without_a_net() -> Result<()> {
    let (engine, gate) = gate_board_with(&["fiducial", "washer"])?;
    let lands = lands(engine.project());
    let fiducial = lands
        .iter()
        .find(|l| {
            l.owner
                == LandOwner::Pad {
                    instance: gate.r1,
                    pad: gate.fiducial,
                }
        })
        .ok_or_else(|| Error::NotFound("R1's fiducial is a land".into()))?;
    assert_eq!(
        fiducial.conductor,
        Conductor::Alone {
            instance: Some(gate.r1)
        }
    );
    assert!(fiducial.on(CopperLayer::Top) && !fiducial.on(CopperLayer::Bottom));
    let washer = lands
        .iter()
        .find(|l| l.owner == LandOwner::Washer { hole: gate.hole })
        .ok_or_else(|| Error::NotFound("the washer is a land".into()))?;
    assert!(washer.through && washer.net().is_none());
    // The copper graph's pads stay the netted ones: neither is joined.
    assert!(
        !crate::layout::pads(engine.project())
            .iter()
            .any(|p| p.pad == gate.fiducial)
    );
    Ok(())
}

/// Finding 1: an unnetted fiducial under a track was invisible to the
/// clearance check and the router but plotted.
#[test]
fn a_track_across_an_unnetted_fiducial_is_a_clearance_finding() -> Result<()> {
    let (engine, gate) = gate_board_with(&["fiducial"])?;
    let found = findings(&engine, "board.clearance");
    let pad = ObjectRef::Pad {
        instance: gate.r1,
        pad: gate.fiducial,
    };
    let track = ObjectRef::Track {
        id: gate.over_fiducial,
    };
    assert!(
        found
            .iter()
            .any(|f| f.objects.contains(&pad) && f.objects.contains(&track)),
        "{found:?}"
    );
    Ok(())
}

#[test]
fn the_router_refuses_a_track_across_a_fiducial() -> Result<()> {
    let (mut engine, gate) = gate_board_with(&["fiducial"])?;
    // R2's fiducial is at (16, 11).
    let before = engine.project().clone();
    let refused = engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::ApplyRoute {
            edits: RouteEditSet {
                add: vec![RouteRequest {
                    net: gate.net_a,
                    start: [14.0, 11.0],
                    end: [18.0, 11.0],
                    width: 0.25,
                    layer: CopperLayer::Top,
                }],
                ..Default::default()
            },
        }),
    );
    let Err(error) = refused else {
        return Err(Error::Invalid(
            "the track across R2's fiducial was accepted".into(),
        ));
    };
    assert!(error.to_string().contains("R2 pad"), "{error}");
    assert_eq!(engine.project(), &before);
    Ok(())
}

/// Finding 2: the plated washer is copper the checks see.
#[test]
fn tracks_of_two_nets_on_a_plated_washer_are_clearance_findings() -> Result<()> {
    let (engine, gate) = gate_board_with(&["washer"])?;
    let found = findings(&engine, "board.clearance");
    for id in [gate.washer_a, gate.washer_b] {
        let track = ObjectRef::Track { id };
        let hit = found
            .iter()
            .find(|f| {
                f.objects.contains(&track)
                    && f.objects.contains(&ObjectRef::BoardFeature {
                        feature: gate.hole.0,
                    })
            })
            .ok_or_else(|| Error::Invalid(format!("no washer finding for {id}: {found:?}")))?;
        assert!(
            hit.message.contains("plated mounting hole at (40, 20) mm"),
            "{}",
            hit.message
        );
    }
    // The drill itself is 0.875 mm clear of both: no hole finding.
    assert!(findings(&engine, "board.hole_clearance").is_empty());
    Ok(())
}

/// Finding 3: an existing track under the effective minimum width.
#[test]
fn an_existing_track_under_the_minimum_width_is_found() -> Result<()> {
    let (engine, gate) = gate_board_with(&["thin"])?;
    let found = findings(&engine, "board.track_width");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].objects, vec![ObjectRef::Track { id: gate.thin }]);
    assert!(
        found[0].message.contains("0.1 mm") && found[0].message.contains("needs 0.2 mm"),
        "{}",
        found[0].message
    );
    Ok(())
}

/// Finding 3, through a net class and a profile change: tracks that met
/// the old minimum are found under the new one.
#[test]
fn raising_the_minimum_by_class_or_profile_finds_existing_tracks() -> Result<()> {
    let (mut engine, gate) = gate_board_with(&["washer"])?;
    assert!(findings(&engine, "board.track_width").is_empty());
    // A class on net A with a 0.3 mm minimum: its 0.25 mm track is under.
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::AddNetClass {
            class: crate::rules::NetClassSpec {
                name: "Wide".into(),
                track_width_min: Some(0.3),
                ..Default::default()
            },
        }),
    )?;
    let class = engine
        .project()
        .rules
        .net_classes
        .last()
        .map(|c| c.id)
        .ok_or_else(|| Error::NotFound("the class".into()))?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::AssignNets {
            nets: vec![gate.net_a],
            class: Some(class),
        }),
    )?;
    let found = findings(&engine, "board.track_width");
    assert!(
        names(&found, &ObjectRef::Track { id: gate.washer_a })
            && !names(&found, &ObjectRef::Track { id: gate.washer_b }),
        "{found:?}"
    );
    assert!(found.iter().all(|f| f.message.contains("net class Wide")));
    // A profile whose minimum is 0.3 mm: both are under.
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::SetOverride {
            key: RuleKey::TrackWidthMin,
            value: RuleValue::Length(0.3),
        }),
    )?;
    let found = findings(&engine, "board.track_width");
    assert!(
        names(&found, &ObjectRef::Track { id: gate.washer_b }),
        "{found:?}"
    );
    Ok(())
}

/// Finding 14: a plated pad drill under the minimum.
#[test]
fn a_pad_drill_under_the_minimum_is_found() -> Result<()> {
    let (engine, gate) = gate_board_with(&["drill"])?;
    let found = findings(&engine, "board.drill_size");
    let pad = ObjectRef::Pad {
        instance: gate.r1,
        pad: gate.drilled,
    };
    let hit = found
        .iter()
        .find(|f| f.objects == vec![pad])
        .ok_or_else(|| Error::Invalid(format!("{found:?}")))?;
    assert!(
        hit.message.contains("0.08 mm") && hit.message.contains("needs 0.2 mm"),
        "{}",
        hit.message
    );
    Ok(())
}

/// Finding 14: every kind of hole keeps the minimum drill.
#[test]
fn every_hole_kind_keeps_the_minimum_drill() -> Result<()> {
    let (engine, _) = gate_board_with(&["washer"])?;
    let mut file = engine.to_file();
    let project = &mut file.project;
    let footprint = &mut project
        .library
        .footprints
        .first_mut()
        .ok_or_else(|| Error::NotFound("footprint".into()))?
        .definition;
    let next = footprint.next_local;
    footprint.next_local += 3;
    // An unplated pad hole, a slot and a pad-owned via, each 0.1 mm.
    let mut npth = footprint
        .pads
        .first()
        .cloned()
        .ok_or_else(|| Error::NotFound("pad".into()))?;
    npth.id = PadId(next);
    npth.kind = PadKind::NonPlatedHole;
    npth.terminal = None;
    npth.position = pt([0.0, -4.0]);
    npth.hole = Some(PadHoleSpec::round_mm(0.1, false)?);
    let mut slot = npth.clone();
    slot.id = PadId(next + 1);
    slot.position = pt([0.0, -6.0]);
    slot.hole = Some(PadHoleSpec {
        shape: crate::library::HoleShape::slot_mm(0.1, 0.8, 0.0)?,
        plated: false,
        offset: Default::default(),
    });
    footprint.pads.push(npth);
    footprint.pads.push(slot);
    if let Some(pad) = footprint.pads.get_mut(1) {
        pad.vias.push(crate::library::PadVia {
            id: crate::library::ApertureId(next + 2),
            at: pt([0.0, 0.0]),
            drill_nm: nm(0.1),
        });
    }
    if let BoardShape::Sketch(sketch) = &mut project.board_shape {
        for hole in &mut sketch.solved.holes {
            hole.drill = 0.1;
            hole.pad_diameter = Some(0.6);
        }
    }
    let engine = Engine::from_file(file)?;
    let found = findings(&engine, "board.drill_size");
    let text: Vec<&str> = found.iter().map(|f| f.message.as_str()).collect();
    // Two placed parts: an unplated hole, a slot and a pad via each, and
    // the mounting hole.
    assert_eq!(found.len(), 7, "{text:#?}");
    assert!(text.iter().any(|t| t.contains("slot 0.1 mm")), "{text:#?}");
    assert!(
        text.iter().any(|t| t.contains("via-in-pad hole")),
        "{text:#?}"
    );
    assert!(
        text.iter().any(|t| t.contains("plated mounting hole")),
        "{text:#?}"
    );
    Ok(())
}

/// The router's keepouts: a transaction cannot commit a track through
/// one, and the refusal leaves the document unchanged.
#[test]
fn a_route_through_a_keepout_is_refused_at_commit() -> Result<()> {
    let (mut engine, gate) = gate_board_with(&["keepout"])?;
    let before = engine.project().clone();
    let refused = engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::ApplyRoute {
            edits: RouteEditSet {
                add: vec![RouteRequest {
                    net: gate.net_b,
                    start: [19.0, 27.0],
                    end: [31.0, 27.0],
                    width: 0.25,
                    layer: CopperLayer::Top,
                }],
                ..Default::default()
            },
        }),
    );
    let Err(error) = refused else {
        return Err(Error::Invalid(
            "the track through the keepout was accepted".into(),
        ));
    };
    assert!(error.to_string().contains("keepout Antenna"), "{error}");
    assert_eq!(engine.project(), &before);
    // The one written into the file is found by the checks.
    let found = findings(&engine, "keepout.track");
    assert!(
        found.iter().any(|f| f.objects.contains(&ObjectRef::Track {
            id: gate.through_keepout
        }) && f.objects.contains(&ObjectRef::Keepout { id: gate.keepout })),
        "{found:?}"
    );
    Ok(())
}

/// Finding 13: a well-formed but wrong stored fill, under the current
/// key, is checked against foreign copper.
#[test]
fn stored_fill_copper_over_another_net_is_a_clearance_finding() -> Result<()> {
    let (engine, gate) = gate_board_with(&["fill"])?;
    let found = findings(&engine, "fill.overlap");
    let fill = ObjectRef::Fill { id: gate.fill };
    let track = ObjectRef::Track {
        id: gate.under_fill,
    };
    assert!(
        found
            .iter()
            .any(|f| f.objects.contains(&fill) && f.objects.contains(&track)),
        "{found:?}"
    );
    assert!(findings(&engine, "fill.stale").is_empty());
    Ok(())
}

/// A refilled fill keeps its clearances, so its stored copper raises
/// nothing.
#[test]
fn engine_computed_fill_copper_raises_no_clearance_finding() -> Result<()> {
    let (mut engine, _) = gate_board_with(&["fill"])?;
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Refill {
            target: crate::layout::fill::RefillTarget::All,
        }),
    )?;
    let found = findings(&engine, "fill.clearance");
    assert!(found.is_empty(), "{found:?}");
    let found = findings(&engine, "fill.overlap");
    assert!(found.is_empty(), "{found:?}");
    let showcase = Engine::showcase()?;
    let found = findings(&showcase, "fill.clearance");
    assert!(found.is_empty(), "{found:?}");
    let found = findings(&showcase, "fill.overlap");
    assert!(found.is_empty(), "{found:?}");
    Ok(())
}

#[test]
fn an_unqualified_mark_on_a_package_is_local() -> crate::Result<()> {
    let (engine, gate) = gate_board()?;
    let project = engine.project();
    let marks = crate::layout::fiducial::placed_fiducials(project);
    let embedded: Vec<_> = marks.iter().filter(|f| f.instance == gate.r1).collect();
    assert_eq!(embedded.len(), 1);
    assert_eq!(embedded[0].role, crate::library::FiducialRole::Local);
    let films = crate::fabrication::artwork::layers(
        project,
        &Default::default(),
        &Default::default(),
        "marks",
    )?;
    assert!(films.iter().any(|f| {
        f.name.ends_with("top.gbr")
            && std::str::from_utf8(&f.bytes)
                .is_ok_and(|s| s.contains("%TA.AperFunction,FiducialPad,Local*%"))
    }));
    Ok(())
}

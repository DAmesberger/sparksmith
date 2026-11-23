//! The release gate's physical acceptance cases (re-review of 22 November
//! 2026, "stop the manufacturing false passes"): stored fill copper short
//! of its gap or off the board, every pair of drilled feature owners and
//! every hole against the board edge, and one waiver per hole and per
//! board side. Each case asserts the objects named and the measured gap,
//! hand-derived from the fixture's coordinates.
use crate::diagnostics::ObjectRef;
use crate::diagnostics::{Error, Finding, Result};
use crate::fabrication::preflight::{self, PreflightReport};
use crate::fabrication::{FabricationCommand, ProductionSnapshot};
use crate::ids::{Id, PadId};
use crate::layout::fill::{FillCommand, inputs_key};
use crate::layout::physical::tests::{gate_board_with, rect};
use crate::library::{Hole as PadHoleSpec, Pad, PadKind, PadShape, PadSide, PadVia};
use crate::library::{Outline, Segment};
use crate::model::Project;
use crate::model::board::{BoardFeatureId, CutoutKind};
use crate::model::board::{BoardShape, SolvedCutout, SolvedMountingHole};
use crate::model::fill::{FillSpec, FilledCopper, FilledRegion};
use crate::model::{CopperLayer, Track, Via, ViaSpan};
use crate::rules::{CopperObject, RuleKey, RuleValue, RulesCommand};
use crate::tests::support::{nm, pt};
use crate::{Engine, Operation};

fn all(engine: &Engine) -> Vec<Finding> {
    crate::layout::checks(engine.project(), &Default::default())
}

fn coded<'a>(findings: &'a [Finding], code: &str) -> Vec<&'a Finding> {
    findings.iter().filter(|f| f.code == code).collect()
}

fn report(engine: &Engine) -> Result<PreflightReport> {
    preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )
}

/// Store `outer` as the gate board's net B fill copper under its current
/// key: a well-formed region a defective worker could return.
fn store(engine: &mut Engine, fill: Id, outer: Vec<[f64; 2]>) -> Result<()> {
    let key = inputs_key(engine.project(), fill)?;
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Store {
            fill,
            copper: Box::new(FilledCopper {
                key,
                regions: vec![FilledRegion {
                    outer,
                    holes: Vec::new(),
                }],
                removed_islands: 0,
                removed_spokes: Vec::new(),
            }),
        }),
    )?;
    Ok(())
}

fn set(engine: &mut Engine, key: RuleKey, mm: f64) -> Result<()> {
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::SetOverride {
            key,
            value: RuleValue::Length(mm),
        }),
    )?;
    Ok(())
}

/// The gate board's net B fill on the bottom layer, its rectangle
/// (45..55, 30..38) mm, over the net A track from (44, 34) to (56, 34),
/// 0.25 mm wide: the track's copper ends at y = 34.125. The fill to track
/// clearance is set to 0.2 mm. A stored rectangle whose lower edge is at
/// y = 34.125 + g leaves a gap of g.
#[test]
fn stored_fill_copper_short_of_its_gap_names_the_fill_the_track_and_the_gap() -> Result<()> {
    let (mut engine, gate) = gate_board_with(&["fill"])?;
    set(
        &mut engine,
        RuleKey::Clearance {
            a: CopperObject::Fill,
            b: CopperObject::Track,
        },
        0.2,
    )?;
    let fill = ObjectRef::Fill { id: gate.fill };
    let track = ObjectRef::Track {
        id: gate.under_fill,
    };
    // (gap, expected words): the review's 0.025 mm, just under the rule,
    // contact, and overlap.
    for (gap, words) in [
        (0.025, Some("is 0.025 mm from a track on")),
        (0.175, Some("is 0.175 mm from a track on")),
        (0.0, Some("Overlap:")),
        (-0.1, Some("Overlap:")),
        // At the rule and beyond it: no finding.
        (0.2, None),
        (0.25, None),
    ] {
        let y = 34.125 + gap;
        store(
            &mut engine,
            gate.fill,
            vec![[46.0, y], [54.0, y], [54.0, 37.0], [46.0, 37.0]],
        )?;
        let found = crate::layout::fill::findings(engine.project());
        let hits: Vec<&Finding> = found
            .iter()
            .filter(|f| {
                f.code == "fill.clearance"
                    && f.objects.contains(&fill)
                    && f.objects.contains(&track)
            })
            .collect();
        match words {
            Some(words) => {
                let [hit] = hits.as_slice() else {
                    return Err(Error::Invalid(format!("gap {gap}: {found:#?}")));
                };
                assert!(hit.message.contains(words), "gap {gap}: {}", hit.message);
                if gap > 0.0 {
                    assert!(hit.message.contains("needs 0.2 mm"), "{}", hit.message);
                }
                // It blocks the release.
                let pf = report(&engine)?;
                assert!(
                    pf.items.iter().any(|i| i.code == "fill.clearance"
                        && i.objects.contains(&track)
                        && i.blocks()),
                    "gap {gap}: {:#?}",
                    pf.items
                );
            }
            None => assert!(hits.is_empty(), "gap {gap}: {hits:#?}"),
        }
    }
    Ok(())
}

/// Copper wholly off the board (the gate board is 60 × 40 mm), copper
/// crossing its edge, and copper on the board but closer to the edge than
/// the 0.3 mm fill-to-edge rule, by 0.1 mm.
#[test]
fn stored_fill_copper_off_or_near_the_board_edge_is_found() -> Result<()> {
    let (mut engine, gate) = gate_board_with(&["fill"])?;
    set(
        &mut engine,
        RuleKey::Clearance {
            a: CopperObject::Fill,
            b: CopperObject::BoardEdge,
        },
        0.3,
    )?;
    let fill = ObjectRef::Fill { id: gate.fill };
    for (outer, words) in [
        (
            vec![[80.0, 45.0], [82.0, 45.0], [82.0, 47.0], [80.0, 47.0]],
            "outside the board outline at (80, 45) mm",
        ),
        (
            vec![[56.0, 36.0], [61.0, 36.0], [61.0, 37.0], [56.0, 37.0]],
            "outside the board outline at (60, 36) mm",
        ),
        (
            vec![[56.0, 36.0], [59.8, 36.0], [59.8, 37.0], [56.0, 37.0]],
            "is 0.2 mm from the board edge; needs 0.3 mm",
        ),
    ] {
        store(&mut engine, gate.fill, outer.clone())?;
        let found = crate::layout::fill::findings(engine.project());
        let hits: Vec<&Finding> = coded(&found, crate::layout::fill::BOARD_EDGE)
            .into_iter()
            .filter(|f| f.objects == vec![fill, ObjectRef::Board])
            .collect();
        let [hit] = hits.as_slice() else {
            return Err(Error::Invalid(format!("{outer:?}: {found:#?}")));
        };
        assert!(hit.message.contains(words), "{}", hit.message);
        let pf = report(&engine)?;
        assert!(
            pf.items
                .iter()
                .any(|i| i.code == crate::layout::fill::BOARD_EDGE && i.blocks()),
            "{:#?}",
            pf.items
        );
    }
    // 0.3 mm in: clear.
    store(
        &mut engine,
        gate.fill,
        vec![[56.0, 36.0], [59.7, 36.0], [59.7, 37.0], [56.0, 37.0]],
    )?;
    let found = crate::layout::fill::findings(engine.project());
    assert!(
        coded(&found, crate::layout::fill::BOARD_EDGE).is_empty(),
        "{found:#?}"
    );
    Ok(())
}

/// The review's release case: on the 44 × 28 mm showcase a 1 × 1 mm
/// island at (90, 80) added to a fill's stored copper reached the copper
/// Gerber with no blocker. It now blocks the release, naming the fill and
/// the board.
#[test]
fn the_showcase_rejects_stored_copper_at_90_80() -> Result<()> {
    let mut engine = Engine::showcase()?;
    let board = engine.project().board().bounds();
    assert_eq!((board.max[0], board.max[1]), (44.0, 28.0));
    let before = report(&engine)?;
    let fill = engine
        .project()
        .fills
        .first()
        .ok_or_else(|| Error::NotFound("a showcase fill".into()))?;
    let id = fill.id;
    let mut copper = fill
        .computed
        .clone()
        .ok_or_else(|| Error::NotFound("its copper".into()))?;
    copper.regions.push(FilledRegion {
        outer: vec![[90.0, 80.0], [91.0, 80.0], [91.0, 81.0], [90.0, 81.0]],
        holes: Vec::new(),
    });
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Store {
            fill: id,
            copper: Box::new(copper),
        }),
    )?;
    let after = report(&engine)?;
    let item = after
        .items
        .iter()
        .find(|i| i.code == crate::layout::fill::BOARD_EDGE)
        .ok_or_else(|| Error::Invalid(format!("{:#?}", after.items)))?;
    assert_eq!(item.objects, vec![ObjectRef::Fill { id }, ObjectRef::Board]);
    assert!(item.title.contains("(90, 80) mm"), "{}", item.title);
    assert!(item.blocks());
    assert_eq!(after.blocking, before.blocking + 1);
    Ok(())
}

/// Two other-net fills on one layer: overlapping, and fresh with a gap
/// under the fill-to-fill rule.
#[test]
fn two_fills_of_other_nets_keep_the_fill_to_fill_gap() -> Result<()> {
    let (mut engine, gate) = gate_board_with(&["fill"])?;
    set(
        &mut engine,
        RuleKey::Clearance {
            a: CopperObject::Fill,
            b: CopperObject::Fill,
        },
        0.3,
    )?;
    let outcome = engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Add {
            fill: Box::new(FillSpec {
                name: "Other".into(),
                net: Some(gate.net_a),
                layer: CopperLayer::Bottom,
                region: Default::default(),
                outline: rect([45.0, 20.0], [55.0, 29.0]),
                priority: 0,
                settings: Default::default(),
            }),
        }),
    )?;
    let crate::Outcome::FillAdded { fill: other } = outcome else {
        return Err(Error::Invalid(format!("{outcome:?}")));
    };
    // The net B fill clear of the track, its lower edge at y = 35.
    store(
        &mut engine,
        gate.fill,
        vec![[46.0, 35.0], [54.0, 35.0], [54.0, 37.0], [46.0, 37.0]],
    )?;
    let pair = |found: &[Finding]| -> Vec<Finding> {
        found
            .iter()
            .filter(|f| {
                f.code == "fill.clearance"
                    && f.objects.contains(&ObjectRef::Fill { id: gate.fill })
                    && f.objects.contains(&ObjectRef::Fill { id: other })
            })
            .cloned()
            .collect()
    };
    for (top, words) in [
        (35.2, Some("Overlap:")),
        (34.9, Some("is 0.1 mm from fill Other; needs 0.3 mm")),
        (34.7, None),
    ] {
        store(
            &mut engine,
            other,
            vec![[46.0, 21.0], [54.0, 21.0], [54.0, top], [46.0, top]],
        )?;
        let found = pair(&crate::layout::fill::findings(engine.project()));
        match words {
            Some(words) => {
                let [hit] = found.as_slice() else {
                    return Err(Error::Invalid(format!("{top}: {found:#?}")));
                };
                assert!(hit.message.contains(words), "{}", hit.message);
            }
            None => assert!(found.is_empty(), "{found:#?}"),
        }
    }
    Ok(())
}

/// The audit measures every object; past the rows it lists, one more
/// finding counts the rest, and it may not be waived.
#[test]
fn the_fill_audit_counts_every_violation_beyond_its_rows() -> Result<()> {
    let (engine, gate) = gate_board_with(&["fill"])?;
    let mut file = engine.to_file();
    let project = &mut file.project;
    // 300 net A tracks under the fill, 0.02 mm apart.
    for k in 0..300 {
        let id = project.allocate()?;
        let x = 46.5 + 0.02 * f64::from(k);
        project.tracks.push(Track {
            id,
            net: gate.net_a,
            start: [x, 32.0],
            end: [x, 33.0],
            width: 0.1,
            layer: CopperLayer::Bottom,
            locked: false,
        });
    }
    let engine = Engine::from_file(file)?;
    let found = crate::layout::fill::findings(engine.project());
    let rows = coded(&found, "fill.clearance");
    assert_eq!(rows.len(), crate::layout::fill::MAX_FILL_ROWS);
    let [more] = coded(&found, crate::layout::fill::CLEARANCE_UNLISTED)[..] else {
        return Err(Error::Invalid(format!("{found:#?}")));
    };
    // 301 tracks overlap: the 300 and the gate's own track under it.
    assert!(
        more.message.contains("45 more copper violations"),
        "{}",
        more.message
    );
    assert!(!preflight::can_waive(
        crate::layout::fill::CLEARANCE_UNLISTED
    ));
    Ok(())
}

/// A drilled feature owner of the pair cases.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// A board mounting hole, 1 mm.
    Mount { plated: bool },
    /// A plated board slot, 1 mm wide, its centre line 1 mm long along x.
    BoardSlot,
    /// A footprint pad's own hole, 1 mm (plated in a 1.6 mm land, or a
    /// bare unplated hole).
    PadHole { plated: bool },
    /// A via-in-pad hole of 1 mm in a 1.6 mm surface pad.
    PadVia,
    /// A board via, 1 mm drill, 1.6 mm pad.
    Via,
}

const KINDS: [Kind; 7] = [
    Kind::Mount { plated: false },
    Kind::Mount { plated: true },
    Kind::BoardSlot,
    Kind::PadHole { plated: true },
    Kind::PadHole { plated: false },
    Kind::PadVia,
    Kind::Via,
];

/// How far from its centre point a feature's wall reaches along +x and
/// −x: a slot's centre line runs ±0.5 mm, its wall 0.5 mm beyond.
fn half_x(kind: Kind) -> f64 {
    match kind {
        Kind::BoardSlot => 1.0,
        _ => 0.5,
    }
}

/// The holes rules of the pair cases: 0.25 mm between any two hole walls,
/// 0.3 mm from any hole wall to an edge.
fn hole_rules(project: &mut Project) -> Result<()> {
    let holes = [
        CopperObject::ViaHole,
        CopperObject::PthHole,
        CopperObject::NpthHole,
    ];
    for a in holes {
        for b in holes {
            crate::rules::apply(
                &mut project.rules,
                RulesCommand::SetOverride {
                    key: RuleKey::Clearance { a, b }.normalized(),
                    value: RuleValue::Length(0.25),
                },
            )?;
        }
        crate::rules::apply(
            &mut project.rules,
            RulesCommand::SetOverride {
                key: RuleKey::Clearance {
                    a,
                    b: CopperObject::BoardEdge,
                }
                .normalized(),
                value: RuleValue::Length(0.3),
            },
        )?;
    }
    Ok(())
}

/// The demo board's R1 sits at (8, 8) mm unturned, so a footprint pad at
/// local (x − 8, y − 8) lands at board (x, y). R2 (at (16, 8)) carries the
/// same pads 8 mm further right, clear of every case.
fn r1_local(at: [f64; 2]) -> [f64; 2] {
    [at[0] - 8.0, at[1] - 8.0]
}

/// The gate board with no deliberate violation, the hole rules, and one
/// feature of each `(kind, centre)`, returning each feature's object. The
/// project is checked as a file would carry it, unvalidated: a board slot
/// here has no sketch curves, and a via may cross the edge.
fn holes_board(features: &[(Kind, [f64; 2])]) -> Result<(Project, Vec<ObjectRef>)> {
    let (engine, gate) = gate_board_with(&["none"])?;
    let mut file = engine.to_file();
    let project = &mut file.project;
    hole_rules(project)?;
    let mut objects = Vec::new();
    let mut feature = 10u32;
    for (kind, at) in features {
        let [x, y] = *at;
        match *kind {
            Kind::Mount { plated } => {
                let BoardShape::Sketch(sketch) = &mut project.board_shape else {
                    return Err(Error::Invalid("the gate board is sketched".into()));
                };
                feature += 1;
                let id = BoardFeatureId(feature);
                sketch.solved.holes.push(SolvedMountingHole {
                    id,
                    at: *at,
                    drill: 1.0,
                    plated,
                    pad_diameter: None,
                });
                sketch.next_feature = feature + 1;
                objects.push(ObjectRef::BoardFeature { feature: id.0 });
            }
            Kind::BoardSlot => {
                let BoardShape::Sketch(sketch) = &mut project.board_shape else {
                    return Err(Error::Invalid("the gate board is sketched".into()));
                };
                feature += 1;
                let id = BoardFeatureId(feature);
                sketch.solved.cutouts.push(SolvedCutout {
                    id: Some(id),
                    kind: CutoutKind::Slot { plated: true },
                    outline: Outline {
                        start: [x - 0.5, y - 0.5],
                        segments: vec![
                            Segment::Line {
                                to: [x + 0.5, y - 0.5],
                            },
                            Segment::Arc {
                                to: [x + 0.5, y + 0.5],
                                bulge: 1.0,
                            },
                            Segment::Line {
                                to: [x - 0.5, y + 0.5],
                            },
                            Segment::Arc {
                                to: [x - 0.5, y - 0.5],
                                bulge: 1.0,
                            },
                        ],
                    },
                    curves: Vec::new(),
                });
                sketch.next_feature = feature + 1;
                objects.push(ObjectRef::BoardFeature { feature: id.0 });
            }
            Kind::PadHole { .. } | Kind::PadVia => {
                let footprint = &mut project
                    .library
                    .footprints
                    .first_mut()
                    .ok_or_else(|| Error::NotFound("the demo footprint".into()))?
                    .definition;
                let id = PadId(footprint.next_local);
                footprint.next_local += 1;
                let (pad_kind, shape, hole, vias) = match *kind {
                    Kind::PadHole { plated: true } => (
                        PadKind::Mechanical,
                        1.6,
                        Some(PadHoleSpec::round_mm(1.0, true)?),
                        Vec::new(),
                    ),
                    Kind::PadHole { plated: false } => (
                        PadKind::NonPlatedHole,
                        1.0,
                        Some(PadHoleSpec::round_mm(1.0, false)?),
                        Vec::new(),
                    ),
                    _ => {
                        let aperture = crate::library::ApertureId(footprint.next_local);
                        footprint.next_local += 1;
                        (
                            PadKind::Mechanical,
                            1.6,
                            None,
                            vec![PadVia {
                                id: aperture,
                                at: pt([0.0, 0.0]),
                                drill_nm: nm(1.0),
                            }],
                        )
                    }
                };
                footprint.pads.push(Pad {
                    id,
                    kind: pad_kind,
                    terminal: None,
                    number: String::new(),
                    position: pt(r1_local(*at)),
                    rotation_mdeg: Default::default(),
                    side: PadSide::Top,
                    shape: PadShape::Circle {
                        diameter_nm: nm(shape),
                    },
                    hole,
                    mask: Default::default(),
                    paste: Default::default(),
                    vias,
                });
                objects.push(match *kind {
                    Kind::PadVia => ObjectRef::PadVia {
                        instance: gate.r1,
                        pad: id,
                        index: 0,
                    },
                    _ => ObjectRef::Pad {
                        instance: gate.r1,
                        pad: id,
                    },
                });
            }
            Kind::Via => {
                let id = project.allocate()?;
                project.vias.push(Via {
                    id,
                    net: gate.net_a,
                    position: *at,
                    drill: 1.0,
                    diameter: 1.6,
                    span: ViaSpan::Through,
                    tented: true,
                    locked: false,
                });
                objects.push(ObjectRef::Via { id });
            }
        }
    }
    Ok((file.project, objects))
}

fn checked(project: &Project) -> Vec<Finding> {
    crate::layout::checks(project, &Default::default())
}

/// Every code a hole-to-hole spacing finding has (hole to copper is
/// `board.hole_clearance` and `board.pad_hole_clearance`).
const SPACING: [&str; 3] = [
    "board.via_hole_spacing",
    "board.pad_hole_spacing",
    crate::layout::via::HOLE_SPACING,
];

/// Whether an edge finding is about a hole's wall, not its copper.
fn hole_edge(f: &Finding) -> bool {
    f.code == "board.edge_clearance" && (f.message.contains("hole") || f.message.contains("drill"))
}

/// Every ordered pair of drilled feature owners (the first made first in
/// its collection, then the other way round), walls 0.025 mm apart under a
/// 0.25 mm rule, is one spacing finding naming both and the gap; 0.25 mm
/// apart, none. The review found two mounting holes 0.025 mm apart passing.
#[test]
fn every_pair_of_drilled_feature_owners_keeps_the_hole_spacing() -> Result<()> {
    let mut missed = Vec::new();
    for a in KINDS {
        for b in KINDS {
            for (gap, expect) in [(0.025, true), (0.25, false)] {
                let left = [20.0, 20.0];
                let right = [20.0 + half_x(a) + gap + half_x(b), 20.0];
                let (project, objects) = holes_board(&[(a, left), (b, right)])?;
                let found = checked(&project);
                let hits: Vec<&Finding> = found
                    .iter()
                    .filter(|f| SPACING.contains(&f.code.as_str()))
                    .filter(|f| objects.iter().all(|o| f.objects.contains(o)))
                    .collect();
                let ok = if expect {
                    hits.len() == 1
                        && hits.iter().all(|f| {
                            f.message.contains("0.025 mm") && f.message.contains("0.25 mm")
                        })
                } else {
                    hits.is_empty()
                };
                if !ok {
                    missed.push(format!("{a:?} then {b:?} at {gap} mm: {hits:#?}"));
                }
            }
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
    Ok(())
}

/// Every hole owner against the board edge: a hole crossing the edge (its
/// centre 0.1 mm inside, so 0.4 mm of it outside) and one whose wall is
/// 0.1 mm inside, under a 0.3 mm rule. The review found a 2 mm component
/// NPTH reaching 0.9 mm outside the board with no finding. A plated board
/// slot is an edge of the board shape itself (the outline check's); a via
/// crossing the edge is refused by the document (and its copper is the
/// copper check's).
#[test]
fn every_hole_owner_keeps_the_board_edge_clearance() -> Result<()> {
    let mut missed = Vec::new();
    for kind in KINDS {
        if kind == Kind::BoardSlot {
            continue;
        }
        let cases: &[(f64, &str)] = if kind == Kind::Via {
            &[(0.6, "0.1 mm from the board edge")]
        } else {
            &[
                (0.1, "extends beyond the board edge"),
                (0.6, "0.1 mm from the board edge"),
            ]
        };
        for (x, words) in cases {
            let (project, objects) = holes_board(&[(kind, [*x, 20.0])])?;
            let found = checked(&project);
            let hits = found
                .iter()
                .filter(|f| hole_edge(f) && f.objects == objects && f.message.contains(words))
                .count();
            if hits != 1 {
                missed.push(format!("{kind:?} at x = {x}: {found:#?}"));
            }
        }
        // 0.3 mm inside: clear.
        let (project, objects) = holes_board(&[(kind, [0.8, 20.0])])?;
        let found = checked(&project);
        if found.iter().any(|f| hole_edge(f) && f.objects == objects) {
            missed.push(format!("{kind:?} 0.3 mm in: {found:#?}"));
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
    Ok(())
}

fn drill_rows(pf: &PreflightReport) -> Vec<&crate::fabrication::PreflightItem> {
    pf.items
        .iter()
        .filter(|i| i.code == "board.drill_size")
        .collect()
}

/// Two 0.08 mm mounting holes are two release rows, keyed by each hole,
/// each stating what it needs of the fab profile. A drill under the
/// profile's minimum is never waived (founder decision, 23 November 2026):
/// the waiver is refused and both keep blocking; a third added later
/// blocks too.
#[test]
fn an_undersized_hole_is_never_waived_and_each_is_its_own_row() -> Result<()> {
    let (engine, _) = gate_board_with(&["none"])?;
    let mut file = engine.to_file();
    // The profile's own minimum drill (0.1 mm), not the gate board's
    // override: only a profile limit is never waived.
    crate::rules::apply(
        &mut file.project.rules,
        RulesCommand::ClearOverride {
            key: RuleKey::DrillMin,
        },
    )?;
    let BoardShape::Sketch(sketch) = &mut file.project.board_shape else {
        return Err(Error::Invalid("the gate board is sketched".into()));
    };
    for (id, x) in [(1, 32.0), (2, 48.0)] {
        sketch.solved.holes.push(SolvedMountingHole {
            id: BoardFeatureId(id),
            at: [x, 21.0],
            drill: 0.08,
            plated: false,
            pad_diameter: None,
        });
    }
    sketch.next_feature = 3;
    let mut engine = Engine::from_file(file)?;
    let pf = report(&engine)?;
    let rows = drill_rows(&pf);
    let objects: Vec<_> = rows.iter().map(|r| r.objects.clone()).collect();
    assert_eq!(
        objects,
        vec![
            vec![ObjectRef::BoardFeature { feature: 1 }],
            vec![ObjectRef::BoardFeature { feature: 2 }],
        ]
    );
    assert!(rows.iter().all(|r| r.blocks()));
    let first = rows
        .first()
        .map(|r| r.key.clone())
        .ok_or_else(|| Error::NotFound("a row".into()))?;
    assert!(rows.iter().all(|r| r.needs.is_some() && !r.offers_waiver()));
    let refused = engine.apply(
        engine.revision(),
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![first],
            reason: "the fabricator drills it".into(),
        }),
    );
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.rule() == Some("fabrication.not_waivable")),
        "{refused:?}"
    );
    let pf = report(&engine)?;
    let rows = drill_rows(&pf);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.blocks()), "{rows:?}");
    // A third undersized hole, added after the waiver, blocks.
    let mut file = engine.to_file();
    let BoardShape::Sketch(sketch) = &mut file.project.board_shape else {
        return Err(Error::Invalid("the gate board is sketched".into()));
    };
    sketch.solved.holes.push(SolvedMountingHole {
        id: BoardFeatureId(3),
        at: [40.0, 30.0],
        drill: 0.08,
        plated: false,
        pad_diameter: None,
    });
    sketch.next_feature = 4;
    let engine = Engine::from_file(file)?;
    let pf = report(&engine)?;
    let rows = drill_rows(&pf);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().filter(|r| r.blocks()).count(), 3);
    assert!(
        !preflight::check(
            &ProductionSnapshot::resolve(engine.project(), None)?,
            engine.drafts(),
            engine.check_settings()
        )?
        .ready()
    );
    Ok(())
}

/// Two undersized via-in-pad holes of one pad are two rows too.
#[test]
fn two_undersized_holes_of_one_pad_are_two_findings() -> Result<()> {
    let (engine, gate) = gate_board_with(&["none"])?;
    let mut file = engine.to_file();
    let footprint = &mut file
        .project
        .library
        .footprints
        .first_mut()
        .ok_or_else(|| Error::NotFound("the demo footprint".into()))?
        .definition;
    let id = PadId(footprint.next_local);
    let (a, b) = (
        crate::library::ApertureId(footprint.next_local + 1),
        crate::library::ApertureId(footprint.next_local + 2),
    );
    footprint.next_local += 3;
    footprint.pads.push(Pad {
        id,
        kind: PadKind::Mechanical,
        terminal: None,
        number: String::new(),
        position: pt([0.0, 12.0]),
        rotation_mdeg: Default::default(),
        side: PadSide::Top,
        shape: PadShape::Circle {
            diameter_nm: nm(2.0),
        },
        hole: None,
        mask: Default::default(),
        paste: Default::default(),
        vias: vec![
            PadVia {
                id: a,
                at: pt([-0.5, 0.0]),
                drill_nm: nm(0.08),
            },
            PadVia {
                id: b,
                at: pt([0.5, 0.0]),
                drill_nm: nm(0.08),
            },
        ],
    });
    let engine = Engine::from_file(file)?;
    let pf = report(&engine)?;
    let mine: Vec<_> = drill_rows(&pf)
        .into_iter()
        .filter(|r| {
            r.objects.iter().any(|o| {
                matches!(o, ObjectRef::PadVia { instance, pad, .. } if *instance == gate.r1 && *pad == id)
            })
        })
        .map(|r| r.objects.clone())
        .collect();
    assert_eq!(
        mine,
        vec![
            vec![ObjectRef::PadVia {
                instance: gate.r1,
                pad: id,
                index: 0
            }],
            vec![ObjectRef::PadVia {
                instance: gate.r1,
                pad: id,
                index: 1
            }],
        ]
    );
    Ok(())
}

/// The showcase assembles both sides and has too few fiducials on each:
/// two findings, one per side, and a waiver of the top side's leaves the
/// bottom side's in Issues and unwaived in the release.
#[test]
fn each_board_side_has_its_own_fiducial_count_finding_and_waiver() -> Result<()> {
    let mut engine = Engine::showcase()?;
    // An assembler asking for three fiducials a side.
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::SetOverride {
            key: RuleKey::FiducialCountMin,
            value: RuleValue::Count(3),
        }),
    )?;
    let sides: Vec<Vec<ObjectRef>> = coded(&all(&engine), crate::layout::fiducial::TOO_FEW)
        .iter()
        .map(|f| f.objects.clone())
        .collect();
    assert_eq!(
        sides,
        vec![
            vec![ObjectRef::BoardSide { top: true }],
            vec![ObjectRef::BoardSide { top: false }],
        ]
    );
    let pf = report(&engine)?;
    let rows: Vec<_> = pf
        .items
        .iter()
        .filter(|i| i.code == crate::layout::fiducial::TOO_FEW)
        .collect();
    assert_eq!(rows.len(), 2, "{rows:#?}");
    let top = rows
        .iter()
        .find(|r| r.objects == vec![ObjectRef::BoardSide { top: true }])
        .map(|r| r.key.clone())
        .ok_or_else(|| Error::NotFound("the top row".into()))?;
    engine.apply(
        engine.revision(),
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![top],
            reason: "Top assembly waiver".into(),
        }),
    )?;
    let pf = report(&engine)?;
    let bottom = pf
        .items
        .iter()
        .find(|i| {
            i.code == crate::layout::fiducial::TOO_FEW
                && i.objects == vec![ObjectRef::BoardSide { top: false }]
        })
        .ok_or_else(|| Error::NotFound("the bottom row".into()))?;
    assert!(bottom.waiver.is_none());
    let issues = crate::issues::report(
        engine.project(),
        engine.drafts(),
        &crate::issues::IssueScope::All,
        engine.check_settings(),
    )?;
    assert_eq!(
        issues
            .rows
            .iter()
            .filter(|r| r.finding.code == crate::layout::fiducial::TOO_FEW)
            .count(),
        1,
        "the waived top side leaves the bottom side's row"
    );
    Ok(())
}

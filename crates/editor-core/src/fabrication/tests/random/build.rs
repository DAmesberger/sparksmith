//! A case made into a board: its library, parts, nets and rules through the
//! public commands, its shape as a solved sketch, all of it opened from a
//! file, so it is a board the document's own validation accepts.
use super::Fallible;
use super::outline;
use super::plan::{Atom, BOARD, CELLS, Case, Cut, Form, NETS, Opening, PITCH, Print, TIE};
use crate::geometry::Point;
use crate::ids::{BridgeId, GraphicId, Id, PackageTerminalId, PadId, TerminalId, TieId};
use crate::layout::LayoutCommand;
use crate::library::PackageModel;
use crate::library::tie::{Bridge, TieForm, TieSpec};
use crate::library::{
    ApertureId, CornerMask, Definition, DrawingLayer, Fill, FootprintGraphic, Hole, HoleShape,
    Item, Justify, MaskRule, Outline, Pad, PadKind, PadShape, PadSide, PadVia, PasteRule,
    PasteWindow, Rotation, Segment, ShapeNm, Side, Text, TextContent,
};
use crate::model::board::{BoardFeatureId, CutoutKind, SolvedBoard, SolvedCutout};
use crate::model::board::{BoardShape, SolvedMountingHole};
use crate::model::{LabelKind, Project, Stackup};
use crate::rules::signal::DiffPairSpec;
use crate::rules::{
    CopperObject, RuleKey, RuleProfile, RuleValue, RulesCommand, fixtures, netclass::NetClassSpec,
};
use crate::schematic::{SchematicCommand, WireTarget};
use crate::tests::support::{self, Spec, ep, mdeg, nm, passive, pt};
use crate::units::ArcDirection;
use crate::{Engine, Operation, format};

/// The prototype profile, filling holes of 0.2 to 0.4 mm, told by size.
/// Test numbers, not a fabricator's.
fn profile() -> Fallible<RuleProfile> {
    let mut value: serde_json::Value = serde_json::from_str(fixtures::PROTOTYPE_JSON)?;
    let p = &mut value["fab_profile"];
    p["id"] = "test-random".into();
    p["name"] = "Test random".into();
    p["drill_rules"] = serde_json::json!([{
        "id": "through", "kind": "mechanical", "layers": {"span": "board"},
        "fills": [
            {"fill": "none"},
            {"fill": "capped", "not_stated": false, "source": "Test fixture.",
             "order": {"told_by_size": true}}
        ]
    }]);
    for (key, mm) in [("capped_drill_min", 0.2), ("capped_drill_max", 0.4)] {
        p["rules"][key] = serde_json::json!({"value": mm, "source": "Test fixture."});
    }
    Ok(fixtures::parse(&value.to_string())?)
}

fn apply(engine: &mut Engine, operation: Operation) -> Fallible {
    engine.apply(engine.revision(), operation)?;
    Ok(())
}

/// The name of net `n`, if it is joined to anything.
pub(super) fn net_name(n: usize) -> String {
    // Not `N<id>`, which a net without a label is called.
    format!("RAIL{n}")
}

pub(super) fn net(project: &Project, n: usize) -> Option<Id> {
    let name = net_name(n % NETS);
    project.nets.iter().find(|x| x.name == name).map(|x| x.id)
}

/// The middle of a spot of a footprint.
pub(super) fn spot(k: usize) -> Point {
    [
        ((k % 3) as f64 - 1.0) * PITCH,
        ((k / 3) as f64 - 1.0) * PITCH,
    ]
}

fn local(k: usize) -> Fallible<u32> {
    Ok(u32::try_from(k)? + 1)
}

/// A D: two sides, and an arc that bulges out.
fn dee() -> Outline {
    Outline {
        start: [-0.6, -0.5],
        segments: vec![
            Segment::Line { to: [0.3, -0.5] },
            Segment::Arc {
                to: [0.3, 0.5],
                bulge: 0.6,
            },
            Segment::Line { to: [-0.6, 0.5] },
            Segment::Line { to: [-0.6, -0.5] },
        ],
    }
}

fn path(points: &[Point]) -> Outline {
    let start = points.first().copied().unwrap_or_default();
    Outline {
        start,
        segments: points
            .iter()
            .skip(1)
            .chain([&start])
            .map(|p| Segment::Line { to: *p })
            .collect(),
    }
}

/// A pad of a form, at a spot: at most 1.1 mm from the spot's middle.
fn pad(k: usize, form: Form, turn: f64, mask: Opening, paste: Opening) -> Fallible<Pad> {
    let size = |w: f64, h: f64| pt([w, h]);
    let round_rect = |corners: CornerMask| PadShape::RoundRect {
        size: size(1.5, 1.1),
        radius_ratio: 0.3,
        max_radius_nm: Some(nm(0.28)),
        corners,
    };
    let shape = match form {
        Form::Rect | Form::Drilled | Form::FarSide => PadShape::Rect {
            size: size(1.6, 1.1),
        },
        Form::Windows => PadShape::Rect {
            size: size(1.6, 1.4),
        },
        Form::Round | Form::Fiducial => PadShape::Circle {
            diameter_nm: nm(1.0),
        },
        Form::RoundRect => round_rect(CornerMask::ALL),
        Form::OneSided => round_rect(CornerMask {
            top_left: true,
            top_right: false,
            bottom_right: false,
            bottom_left: true,
        }),
        Form::Oblong => PadShape::Oblong {
            size: size(1.8, 0.9),
        },
        Form::Dee => PadShape::Custom { outline: dee() },
        Form::Ell => PadShape::Custom {
            outline: path(&[
                [-0.7, -0.6],
                [0.7, -0.6],
                [0.7, 0.0],
                [0.0, 0.0],
                [0.0, 0.6],
                [-0.7, 0.6],
            ]),
        },
        Form::Through | Form::ThroughOff => PadShape::Circle {
            diameter_nm: nm(1.7),
        },
        Form::ThroughSquare => PadShape::Rect {
            size: size(1.5, 1.5),
        },
        Form::ThroughSlot => PadShape::Oblong {
            size: size(2.0, 1.4),
        },
        Form::Bare => PadShape::Circle {
            diameter_nm: nm(1.3),
        },
    };
    let hole = match form {
        Form::Through | Form::ThroughSquare => Some(Hole::round_mm(0.8, true)?),
        Form::ThroughOff => Some(Hole {
            shape: HoleShape::round_mm(0.7)?,
            plated: true,
            offset: pt([0.2, -0.1]),
        }),
        Form::ThroughSlot => Some(Hole {
            shape: HoleShape::slot_mm(0.6, 1.2, 0.0)?,
            plated: true,
            offset: pt([0.0, 0.0]),
        }),
        Form::Bare => Some(Hole::round_mm(1.3, false)?),
        _ => None,
    };
    let kind = match form {
        Form::Through | Form::ThroughSquare | Form::ThroughOff | Form::ThroughSlot => {
            PadKind::PlatedThrough
        }
        Form::Bare => PadKind::NonPlatedHole,
        Form::Fiducial => PadKind::Fiducial,
        Form::Windows | Form::Drilled => PadKind::Mechanical,
        _ => PadKind::Smd,
    };
    let paste = match (form, paste) {
        (Form::Windows, _) => PasteRule::Windows(vec![
            PasteWindow {
                id: ApertureId(1),
                shape: ShapeNm::Rect {
                    a: pt([-0.7, -0.6]),
                    b: pt([-0.1, 0.6]),
                    radius_nm: nm(0.15),
                },
            },
            PasteWindow {
                id: ApertureId(2),
                shape: ShapeNm::Circle {
                    center: pt([0.4, 0.0]),
                    radius_nm: nm(0.3),
                },
            },
        ]),
        (Form::Fiducial | Form::Bare, _) => PasteRule::Inherit,
        (_, Opening::Board) => PasteRule::Inherit,
        (_, Opening::Closed) => PasteRule::None,
        // Through pads are not pasted by a reduction.
        _ if kind == PadKind::PlatedThrough => PasteRule::Inherit,
        (_, Opening::Own) => PasteRule::Reduction(nm(0.07)),
    };
    let id = local(k)?;
    Ok(Pad {
        id: PadId(id),
        kind,
        terminal: form.lands().then_some(PackageTerminalId(id)),
        number: id.to_string(),
        position: pt(spot(k)),
        rotation_mdeg: shape.canonical_rotation(mdeg(turn)),
        side: match form {
            Form::FarSide => PadSide::Bottom,
            _ => PadSide::Top,
        },
        shape,
        hole,
        mask: match mask {
            Opening::Board => MaskRule::Inherit,
            Opening::Own => MaskRule::Expansion(nm(0.12)),
            Opening::Closed => MaskRule::Tented,
        },
        paste,
        fiducial_role: None,
        vias: match form {
            Form::Drilled => vec![PadVia {
                id: ApertureId(1),
                at: pt([0.3, 0.1]),
                drill_nm: nm(0.3),
            }],
            _ => Vec::new(),
        },
    })
}

fn print(k: usize, at: usize, what: Print, far: bool) -> Fallible<FootprintGraphic> {
    let [x, y] = spot(at);
    let stroke = |shape: ShapeNm, fill: Fill| Item::Shape {
        shape,
        width_nm: nm(0.15),
        fill,
    };
    let text = |content: TextContent| {
        Item::Text(Text {
            content,
            at: pt([x, y]),
            height_nm: nm(1.0),
            rotation: Rotation::R0,
            justify: Justify::default(),
            visible: true,
            line_spacing: 1.5,
        })
    };
    let item = match what {
        Print::Line => stroke(
            ShapeNm::Line {
                a: pt([x - 1.0, y + 1.1]),
                b: pt([x + 1.0, y + 1.2]),
            },
            Fill::None,
        ),
        Print::Arc => stroke(
            ShapeNm::Arc {
                start: pt([x + 0.6, y]),
                end: pt([x, y + 0.6]),
                centre: pt([x, y]),
                direction: ArcDirection::CounterClockwise,
            },
            Fill::None,
        ),
        Print::Circle => stroke(
            ShapeNm::Circle {
                center: pt([x, y]),
                radius_nm: nm(0.4),
            },
            Fill::None,
        ),
        Print::Solid => stroke(
            ShapeNm::Polygon {
                points: [[x, y + 1.0], [x + 0.6, y + 1.0], [x, y + 1.5]]
                    .map(pt)
                    .to_vec(),
                radii_nm: Vec::new(),
            },
            Fill::Solid,
        ),
        Print::Text => text(TextContent::Literal("L7".into())),
        Print::Designator => text(TextContent::Designator),
    };
    Ok(FootprintGraphic {
        id: GraphicId(20 + u32::try_from(k)?),
        layer: DrawingLayer::Silkscreen,
        side: if far { Side::Opposite } else { Side::Component },
        item,
    })
}

/// Where a symbol's pin of spot `k` is: three a side, as on paper.
fn pin(k: usize) -> Point {
    let row = [-2.5, 0.0, 2.5][k % 3];
    match k / 3 {
        0 => [-5.0, row],
        1 => [5.0, row],
        _ => [row, 5.0],
    }
}

/// Publish part `part` and place its symbol. Its pads are the case's; a
/// part without one that lands a terminal gets one in the middle.
fn part(engine: &mut Engine, case: &Case, part: usize, datum: bool) -> Fallible<Id> {
    let (mut pads, mut prints) = (Vec::new(), Vec::new());
    for (_, atom) in &case.atoms {
        match atom {
            Atom::Pad {
                part: p,
                spot,
                form,
                turn,
                mask,
                paste,
            } if *p == part => pads.push((*spot, pad(*spot, *form, *turn, *mask, *paste)?)),
            Atom::Print {
                part: p,
                spot,
                print: what,
                far,
            } if *p == part => prints.push(print(prints.len(), *spot, *what, *far)?),
            _ => {}
        }
    }
    let tied = case
        .atoms
        .iter()
        .any(|(_, a)| matches!(a, Atom::Tie { part: p } if *p == part));
    if tied {
        pads.retain(|(k, _)| !TIE.contains(k));
        for k in TIE {
            pads.push((k, pad(k, Form::Rect, 0.0, Opening::Board, Opening::Board)?));
        }
    }
    if !pads.iter().any(|(_, p)| p.terminal.is_some()) {
        pads.retain(|(k, _)| *k != 4);
        pads.push((4, pad(4, Form::Rect, 0.0, Opening::Board, Opening::Board)?));
    }
    let landed: Vec<u32> = pads
        .iter()
        .filter_map(|(_, p)| p.terminal.map(|t| t.0))
        .collect();
    let names: Vec<String> = landed.iter().map(|t| format!("P{t}")).collect();
    let numbers: Vec<String> = landed.iter().map(u32::to_string).collect();
    let name = format!("Random {part}");
    let spec = Spec {
        name: &name,
        terminals: landed
            .iter()
            .zip(&names)
            .map(|(t, n)| (*t, n.as_str(), passive()))
            .collect(),
        units: vec![(
            1,
            "A",
            landed.iter().map(|t| (*t, pin(*t as usize - 1))).collect(),
        )],
        package: landed
            .iter()
            .zip(&numbers)
            .map(|(t, n)| (*t, n.as_str()))
            .collect(),
        mapping: landed.iter().map(|t| (*t, *t)).collect(),
        physical: true,
    };
    let fixture = support::draft(engine, &spec)?;
    support::rewrite(engine, fixture.footprint, |d| {
        if let Definition::Footprint(f) = d {
            f.pads = pads.into_iter().map(|(_, p)| p).collect();
            f.graphics = prints;
            f.next_local = 60;
            if datum {
                f.datum.offset = pt([0.4, -0.3]);
                f.datum.rotation_mdeg = mdeg(30.0);
            }
        }
    })?;
    if tied {
        tie(engine, &fixture)?;
    }
    support::publish(engine, fixture.all())?;
    let (instance, _) = support::place(
        engine,
        &fixture,
        &format!("U{}", part + 1),
        1,
        [part as f64 * 60.0, 0.0],
    )?;
    for (_, atom) in &case.atoms {
        let Atom::Join { part: p, spot, net } = atom else {
            continue;
        };
        let terminal = local(*spot)?;
        if *p != part || !landed.contains(&terminal) {
            continue;
        }
        let at = crate::schematic::pin_position(engine.project(), ep(instance, terminal))
            .ok_or("a pin")?;
        apply(
            engine,
            Operation::Schematic(SchematicCommand::PlaceLabel {
                sheet: None,
                name: net_name(*net),
                kind: LabelKind::Local,
                position: pt(at),
                rotation: Rotation::R0,
                attach: Some(WireTarget::Terminal(ep(instance, terminal))),
            }),
        )?;
    }
    Ok(instance)
}

/// The part a copper tie: a bridge between its two tied pads, nothing to
/// buy or to place.
fn tie(engine: &mut Engine, fixture: &support::Fixture) -> Fallible {
    let [a, b] = TIE.map(|k| k as u32 + 1);
    support::rewrite(engine, fixture.component, |d| {
        if let Definition::Component(c) = d {
            c.ties = vec![TieSpec {
                id: TieId(1),
                form: TieForm::Copper,
                ends: vec![TerminalId(a), TerminalId(b)],
            }];
            c.assembly = TieForm::Copper.assembly();
        }
    })?;
    support::rewrite(engine, fixture.package, |d| {
        if let Definition::Package(p) = d {
            p.model = PackageModel::NoBody;
        }
    })?;
    support::rewrite(engine, fixture.footprint, |d| {
        if let Definition::Footprint(f) = d {
            let ([x0, y], [x1, _]) = (spot(TIE[0]), spot(TIE[1]));
            f.bridges = vec![Bridge {
                id: BridgeId(59),
                side: PadSide::Top,
                pads: vec![PadId(a), PadId(b)],
                outline: path(&[[x0, y - 0.1], [x1, y - 0.1], [x1, y + 0.1], [x0, y + 0.1]]),
            }];
        }
    })?;
    Ok(())
}

fn ring(at: Point, r: f64) -> Outline {
    Outline {
        start: [at[0] + r, at[1]],
        segments: [[at[0] - r, at[1]], [at[0] + r, at[1]]]
            .into_iter()
            .map(|to| Segment::Arc { to, bulge: 1.0 })
            .collect(),
    }
}

/// An upright slot.
fn slot(at: Point, length: f64, width: f64) -> Outline {
    let (h, r) = ((length - width) / 2.0, width / 2.0);
    Outline {
        start: [at[0] + r, at[1] - h],
        segments: vec![
            Segment::Line {
                to: [at[0] + r, at[1] + h],
            },
            Segment::Arc {
                to: [at[0] - r, at[1] + h],
                bulge: 1.0,
            },
            Segment::Line {
                to: [at[0] - r, at[1] - h],
            },
            Segment::Arc {
                to: [at[0] + r, at[1] - h],
                bulge: 1.0,
            },
        ],
    }
}

fn cuts(case: &Case) -> Vec<Cut> {
    case.atoms
        .iter()
        .filter_map(|(_, a)| match a {
            Atom::Cut(cut) => Some(*cut),
            _ => None,
        })
        .collect()
}

/// The board as its cuts make it; `None` is the plain rectangle.
fn solved(case: &Case) -> Option<SolvedBoard> {
    let cuts = cuts(case);
    if cuts.is_empty() {
        return None;
    }
    let has = |c: Cut| cuts.contains(&c);
    let [w, h] = BOARD;
    let r = if has(Cut::Corners) { 2.0 } else { 0.0 };
    // The right side's ends, when it bows: the bow reaches the full width.
    let right = if has(Cut::Bow) { w - 2.0 } else { w };
    let quarter = (std::f64::consts::PI / 8.0).tan();
    let mut segments = Vec::new();
    let corner = |segments: &mut Vec<Segment>, to: Point| {
        if r > 0.0 {
            segments.push(Segment::Arc { to, bulge: quarter });
        }
    };
    if has(Cut::Notch) {
        for to in [[12.0, 0.0], [12.0, 1.5], [16.0, 1.5], [16.0, 0.0]] {
            segments.push(Segment::Line { to });
        }
    }
    segments.push(Segment::Line {
        to: [right - r, 0.0],
    });
    corner(&mut segments, [right, r]);
    segments.push(match has(Cut::Bow) {
        true => Segment::Arc {
            to: [right, h - r],
            bulge: 2.0 / ((h - 2.0 * r) / 2.0),
        },
        false => Segment::Line { to: [right, h - r] },
    });
    corner(&mut segments, [right - r, h]);
    if has(Cut::Bay) {
        segments.push(Segment::Line { to: [22.0, h] });
        segments.push(Segment::Arc {
            to: [16.0, h],
            bulge: -0.5,
        });
    }
    segments.push(Segment::Line { to: [r, h] });
    corner(&mut segments, [0.0, h - r]);
    segments.push(Segment::Line { to: [0.0, r] });
    corner(&mut segments, [r, 0.0]);
    let mut board = SolvedBoard::rectangle(BOARD);
    board.edges = Vec::new();
    board.outer = Outline {
        start: [r, 0.0],
        segments,
    };
    let cutout = |id: u32, kind: CutoutKind, outline: Outline| SolvedCutout {
        id: Some(BoardFeatureId(id)),
        kind,
        curves: outline::curves(id, &outline),
        outline,
    };
    if has(Cut::Window) {
        board
            .cutouts
            .push(cutout(1, CutoutKind::Cutout, ring([13.25, 13.0], 1.2)));
    }
    if has(Cut::Slot) {
        board.cutouts.push(cutout(
            2,
            CutoutKind::Slot { plated: false },
            slot([2.0, 13.0], 4.0, 1.0),
        ));
    }
    if has(Cut::PlatedSlot) {
        board.cutouts.push(cutout(
            3,
            CutoutKind::Slot { plated: true },
            slot([24.75, 13.0], 3.0, 1.0),
        ));
    }
    if has(Cut::PlatedHole) {
        board.holes.push(SolvedMountingHole {
            id: BoardFeatureId(4),
            at: [2.4, 2.4],
            drill: 1.6,
            plated: true,
            pad_diameter: Some(2.6),
        });
    }
    if has(Cut::BareHole) {
        board.holes.push(SolvedMountingHole {
            id: BoardFeatureId(5),
            at: [2.4, h - 2.4],
            drill: 1.7,
            plated: false,
            pad_diameter: None,
        });
    }
    Some(board)
}

/// The board of `case` before its copper and edits, with nothing to undo.
pub(super) fn built(case: &Case) -> Fallible<Engine> {
    let mut engine = Engine::profiled()?;
    apply(
        &mut engine,
        Operation::Rules(RulesCommand::set_profile(profile()?, false)),
    )?;
    apply(
        &mut engine,
        Operation::Layout(LayoutCommand::SetStackup(Stackup::preset(case.layers)?)),
    )?;
    let mut parts = Vec::new();
    for (_, atom) in &case.atoms {
        if let Atom::Part {
            part: k,
            turn,
            side,
            datum,
        } = atom
        {
            parts.push((*k, *turn, *side, part(&mut engine, case, *k, *datum)?));
        }
    }
    let mut file = engine.to_file();
    let project = &mut file.project;
    project.name = "random".into();
    project.board_size = BOARD;
    if let Some(board) = solved(case) {
        project.board_size = board.bounds().max;
        let fit = crate::layout::outline::outline_fit(project);
        project.board_shape = BoardShape::Sketch(Box::new(outline::sketch(board, fit)?));
    }
    let mut engine = Engine::from_file(format::decode(&format::encode(&file)?)?)?;
    for (k, turn, side, instance) in parts {
        apply(
            &mut engine,
            Operation::Layout(LayoutCommand::Place {
                instance,
                position: CELLS[k % CELLS.len()],
                rotation_deg: turn,
                layer: side,
            }),
        )?;
    }
    for (_, atom) in &case.atoms {
        match atom {
            Atom::Rule { object, mm } => apply(&mut engine, rule(*object, *mm))?,
            Atom::Class {
                net: n,
                clearance,
                fill,
            } => {
                if let Some(id) = net(engine.project(), *n) {
                    class(&mut engine, id, *clearance, *fill)?;
                }
            }
            Atom::Pair { nets: [a, b] } => {
                let ids = (net(engine.project(), *a), net(engine.project(), *b));
                if let (Some(positive), Some(negative)) = ids {
                    apply(
                        &mut engine,
                        Operation::Rules(RulesCommand::AddDiffPair {
                            pair: DiffPairSpec {
                                name: "Pair".into(),
                                positive,
                                negative,
                                target: None,
                            },
                        }),
                    )?;
                }
            }
            _ => {}
        }
    }
    engine.clear_history();
    Ok(engine)
}

/// The override of the clearance fills keep to `object`.
pub(super) fn rule(object: CopperObject, mm: f64) -> Operation {
    Operation::Rules(RulesCommand::SetOverride {
        key: RuleKey::Clearance {
            a: CopperObject::Fill,
            b: object,
        }
        .normalized(),
        value: RuleValue::Length(mm),
    })
}

fn class(engine: &mut Engine, net: Id, clearance: f64, fill: Option<f64>) -> Fallible {
    let outcome = engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::AddNetClass {
            class: NetClassSpec {
                name: format!("Class {net}"),
                clearance: Some(clearance),
                fill_clearance: fill,
                ..NetClassSpec::default()
            },
        }),
    )?;
    let crate::Outcome::NetClassAdded { class } = outcome else {
        return Err(format!("unexpected {outcome:?}").into());
    };
    apply(
        engine,
        Operation::Rules(RulesCommand::AssignNets {
            nets: vec![net],
            class: Some(class),
        }),
    )
}

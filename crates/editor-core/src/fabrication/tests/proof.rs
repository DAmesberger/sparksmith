//! The written files against the design, by checks that share no code with
//! what wrote them: [`check`] reads every Gerber and Excellon file of a set
//! back and holds each layer's image to the point oracle
//! ([`super::oracle`]), each hole, slot, cutout and object to a two-way
//! inventory of the document, and the fill copper's measured spacing to the
//! rules' own numbers. The seeded faults damage the output one way each:
//! a fault the check does not report is a hole in the check.
use super::Fallible;
use super::oracle::{self, Image};
use super::{excellon_reader, reader};
use crate::fabrication::{self, FabricationOptions, ProductionSnapshot};
use crate::geometry::Point;
use crate::ids::PadId;
use crate::library::footprint_source::{EvaluatorRef, SketchBlob, SolveSummary};
use crate::library::{
    ApertureId, CornerMask, Hole, HoleShape, MaskRule, Outline, Pad, PadKind, PadShape, PadSide,
    PadVia, PasteRule, PasteWindow, Segment, ShapeNm,
};
use crate::model::board::{
    BoardFeatureId, BoardShape, BoardSketch, CutoutKind, SolvedBoard, SolvedCutout,
    SolvedMountingHole,
};
use crate::model::fill::{FilledCopper, FilledRegion};
use crate::model::{CopperLayer, Fill, Layer, Project, Track, Via, ViaSpan};
use crate::rules::{CopperObject, RuleKey, RuleOverride, RuleValue};
use crate::tests::support::{mdeg, nm, pt};
use crate::{Engine, Operation};
use sparksmith_fabcheck::spacing::{self, Against, Spacing};
use sparksmith_fabcheck::{drills, outline};

/// How far fill copper may measure short of a rule: the fill chord its
/// computation flattens curves at and the reader's chords, mm.
pub(super) const SPACING_SLACK_MM: f64 = 0.002;

pub(super) fn lines(points: &[Point]) -> Outline {
    Outline {
        start: points[0],
        segments: points[1..]
            .iter()
            .chain(&points[..1])
            .map(|p| Segment::Line { to: *p })
            .collect(),
    }
}

/// Two 1 mm squares joined by a bar 0.1 mm wide and 0.6 mm long: shrunk by
/// more than 0.05 mm it is two squares.
fn dumbbell() -> Outline {
    lines(&[
        [-1.3, -0.5],
        [-0.3, -0.5],
        [-0.3, -0.05],
        [0.3, -0.05],
        [0.3, -0.5],
        [1.3, -0.5],
        [1.3, 0.5],
        [0.3, 0.5],
        [0.3, 0.05],
        [-0.3, 0.05],
        [-0.3, 0.5],
        [-1.3, 0.5],
    ])
}

/// A 3 by 2 mm block with a 1 mm square cavity that opens to its top by a
/// gap 0.06 mm wide: grown by more than 0.03 mm the cavity is a hole.
fn cavity() -> Outline {
    lines(&[
        [-1.5, -1.0],
        [1.5, -1.0],
        [1.5, 1.0],
        [0.03, 1.0],
        [0.03, 0.5],
        [0.5, 0.5],
        [0.5, -0.5],
        [-0.5, -0.5],
        [-0.5, 0.5],
        [-0.03, 0.5],
        [-0.03, 1.0],
        [-1.5, 1.0],
    ])
}

fn pad(id: u32, kind: PadKind, at: Point, shape: PadShape) -> Pad {
    Pad {
        id: PadId(id),
        kind,
        terminal: None,
        number: String::new(),
        position: pt(at),
        rotation_mdeg: Default::default(),
        side: PadSide::Top,
        shape,
        hole: None,
        mask: Default::default(),
        paste: Default::default(),
        fiducial_role: None,
        vias: Vec::new(),
    }
}

fn slot_outline(a: Point, b: Point, width: f64) -> Outline {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = d[0].hypot(d[1]);
    let n = [-d[1] / l * width / 2.0, d[0] / l * width / 2.0];
    Outline {
        start: [a[0] - n[0], a[1] - n[1]],
        segments: vec![
            Segment::Line {
                to: [b[0] - n[0], b[1] - n[1]],
            },
            Segment::Arc {
                to: [b[0] + n[0], b[1] + n[1]],
                bulge: 1.0,
            },
            Segment::Line {
                to: [a[0] + n[0], a[1] + n[1]],
            },
            Segment::Arc {
                to: [a[0] - n[0], a[1] - n[1]],
                bulge: 1.0,
            },
        ],
    }
}

/// A slot whose ends bulge inwards: 2 mm half turns, centre line from 1 to
/// 4 mm, the material it removes ending at x = 4 mm.
fn concave(x: f64, y: f64) -> Outline {
    Outline {
        start: [x, y - 1.0],
        segments: vec![
            Segment::Line {
                to: [x + 4.0, y - 1.0],
            },
            Segment::Arc {
                to: [x + 4.0, y + 1.0],
                bulge: -1.0,
            },
            Segment::Line { to: [x, y + 1.0] },
            Segment::Arc {
                to: [x, y - 1.0],
                bulge: -1.0,
            },
        ],
    }
}

pub(super) fn sketch(solved: SolvedBoard) -> BoardShape {
    let evaluator = EvaluatorRef {
        engine: "test".into(),
        rev: "0".into(),
    };
    let roles = solved
        .cutouts
        .iter()
        .filter_map(|c| {
            let id = c.id?;
            Some(crate::model::board::CutoutRole {
                id,
                kind: c.kind,
                curves: vec![crate::library::footprint_source::SketchCurveId(format!(
                    "c{}",
                    id.0
                ))],
            })
        })
        .collect();
    BoardShape::Sketch(Box::new(BoardSketch {
        evaluator: evaluator.clone(),
        sketch: SketchBlob("{}".into()),
        parameters: Vec::new(),
        cutouts: roles,
        holes: Vec::new(),
        edges: Vec::new(),
        next_feature: 20,
        solve: SolveSummary {
            evaluator,
            status: Default::default(),
            dof: 0,
            result_hash: String::new(),
        },
        solved,
        current: true,
        outline_fit: 0.01,
        fault: None,
        idle: Vec::new(),
    }))
}

fn cutout(id: u32, kind: CutoutKind, outline: Outline) -> SolvedCutout {
    SolvedCutout {
        id: Some(BoardFeatureId(id)),
        kind,
        outline,
        curves: Vec::new(),
    }
}

/// The clearances the board is drawn under, mm, by what fill copper meets:
/// every one different, so one rule read for another shows.
const RULES: [(CopperObject, f64); 8] = [
    (CopperObject::Track, 0.21),
    (CopperObject::Pad, 0.23),
    (CopperObject::Via, 0.22),
    (CopperObject::Fill, 0.25),
    (CopperObject::BoardEdge, 0.35),
    (CopperObject::ViaHole, 0.45),
    (CopperObject::PthHole, 0.38),
    (CopperObject::NpthHole, 0.31),
];

fn clearance(against: Against) -> f64 {
    let object = match against {
        Against::Fill => CopperObject::Fill,
        Against::Track => CopperObject::Track,
        Against::Via => CopperObject::Via,
        Against::Pad => CopperObject::Pad,
        Against::ViaHole => CopperObject::ViaHole,
        Against::PlatedHole => CopperObject::PthHole,
        Against::UnplatedHole => CopperObject::NpthHole,
    };
    RULES
        .iter()
        .find(|(o, _)| *o == object)
        .map_or(0.0, |(_, mm)| *mm)
}

fn rules(project: &mut Project) {
    for (b, mm) in RULES {
        let key = RuleKey::Clearance {
            a: CopperObject::Fill,
            b,
        }
        .normalized();
        project.rules.overrides.retain(|o| o.key != key);
        project.rules.overrides.push(RuleOverride {
            key,
            value: RuleValue::Length(mm),
        });
    }
}

pub(super) fn refill(p: &mut Project) -> Fallible<()> {
    for (id, copper) in crate::layout::fill::refill(p, crate::layout::fill::RefillTarget::All)? {
        if let Some(fill) = p.fills.iter_mut().find(|f| f.id == id) {
            fill.computed = Some(copper);
        }
    }
    Ok(())
}

fn fill(p: &mut Project, net: u64, layer: CopperLayer) -> Fallible<u64> {
    let id = p.allocate()?;
    p.fills.push(Fill {
        id,
        name: format!("Pour {id}"),
        net: Some(net),
        layer,
        region: Default::default(),
        outline: lines(&[[1.0, 1.0], [59.0, 1.0], [59.0, 39.0], [1.0, 39.0]]),
        priority: 0,
        settings: Default::default(),
        locked: false,
        computed: None,
    });
    Ok(id)
}

/// A 60 by 40 mm board holding one of everything the files can hold: every
/// pad shape on a part turned on each side, paste that splits, a mask
/// opening that closes, slots and holes of every kind, pours with islands
/// and holes on both layers.
pub(super) fn board() -> Fallible<Project> {
    let mut p = Engine::demo()?.project().clone();
    let footprint = &mut p
        .library
        .footprints
        .first_mut()
        .ok_or("the demo footprint")?
        .definition;
    let mut d_shape = pad(
        50,
        PadKind::Mechanical,
        [0.0, 3.0],
        PadShape::RoundRect {
            size: pt([2.0, 1.0]),
            radius_ratio: 0.25,
            max_radius_nm: None,
            corners: CornerMask {
                top_left: true,
                top_right: false,
                bottom_right: false,
                bottom_left: true,
            },
        },
    );
    d_shape.rotation_mdeg = mdeg(20.0);
    d_shape.mask = MaskRule::Expansion(nm(0.1));
    let mut split = pad(
        51,
        PadKind::Mechanical,
        [0.0, -4.0],
        PadShape::Custom {
            outline: dumbbell(),
        },
    );
    split.paste = PasteRule::Reduction(nm(0.08));
    let mut closes = pad(
        52,
        PadKind::Mechanical,
        [5.0, -4.0],
        PadShape::Custom { outline: cavity() },
    );
    closes.mask = MaskRule::Expansion(nm(0.05));
    closes.rotation_mdeg = mdeg(15.0);
    let mut slotted = pad(
        53,
        PadKind::PlatedThrough,
        [5.0, 3.0],
        PadShape::Oblong {
            size: pt([1.8, 3.0]),
        },
    );
    slotted.rotation_mdeg = mdeg(30.0);
    slotted.hole = Some(Hole {
        shape: HoleShape::slot_mm(0.8, 1.8, 90.0)?,
        plated: true,
        offset: pt([0.0, 0.1]),
    });
    let mut bare = pad(
        54,
        PadKind::NonPlatedHole,
        [-5.0, 3.0],
        PadShape::Circle {
            diameter_nm: nm(1.2),
        },
    );
    bare.hole = Some(Hole::round_mm(1.2, false)?);
    let mut via_in_pad = pad(
        55,
        PadKind::Mechanical,
        [-5.0, -4.0],
        PadShape::Rect {
            size: pt([1.6, 1.2]),
        },
    );
    via_in_pad.vias = vec![PadVia {
        id: ApertureId(1),
        at: pt([0.3, 0.2]),
        drill_nm: nm(0.3),
    }];
    let mut mark = pad(
        56,
        PadKind::Fiducial,
        [0.0, 6.0],
        PadShape::Circle {
            diameter_nm: nm(1.0),
        },
    );
    mark.mask = MaskRule::Expansion(nm(0.5));
    let mut windows = pad(
        57,
        PadKind::Mechanical,
        [0.0, -7.0],
        PadShape::Rect {
            size: pt([3.0, 2.0]),
        },
    );
    windows.paste = PasteRule::Windows(vec![
        PasteWindow {
            id: ApertureId(1),
            shape: ShapeNm::Rect {
                a: pt([-1.3, -0.8]),
                b: pt([-0.1, 0.8]),
                radius_nm: nm(0.2),
            },
        },
        PasteWindow {
            id: ApertureId(2),
            shape: ShapeNm::Circle {
                center: pt([0.8, 0.0]),
                radius_nm: nm(0.5),
            },
        },
    ]);
    footprint.pads.extend([
        d_shape, split, closes, slotted, bare, via_in_pad, mark, windows,
    ]);
    for (placement, (position, rotation, layer)) in p.placements.iter_mut().zip([
        ([15.0, 15.0], 0.0, Layer::Top),
        ([42.0, 20.0], 37.0, Layer::Bottom),
    ]) {
        placement.position = position;
        placement.rotation_deg = rotation;
        placement.layer = layer;
    }
    let mut solved = SolvedBoard::rectangle([60.0, 40.0]);
    let round = (std::f64::consts::PI / 8.0).tan();
    solved.outer = Outline {
        start: [2.0, 0.0],
        segments: vec![
            Segment::Line { to: [60.0, 0.0] },
            Segment::Line { to: [60.0, 40.0] },
            Segment::Line { to: [0.0, 40.0] },
            Segment::Line { to: [0.0, 2.0] },
            Segment::Arc {
                to: [2.0, 0.0],
                bulge: round,
            },
        ],
    };
    let circle = Outline {
        start: [32.0, 32.0],
        segments: vec![
            Segment::Arc {
                to: [28.0, 32.0],
                bulge: 1.0,
            },
            Segment::Arc {
                to: [32.0, 32.0],
                bulge: 1.0,
            },
        ],
    };
    solved.cutouts = vec![
        cutout(1, CutoutKind::Cutout, circle),
        cutout(
            2,
            CutoutKind::Slot { plated: false },
            slot_outline([6.0, 30.0], [10.0, 34.0], 1.0),
        ),
        cutout(
            3,
            CutoutKind::Slot { plated: true },
            slot_outline([48.0, 33.0], [52.0, 35.0], 1.2),
        ),
        // Not a slot, whatever its kind says: milled as it is drawn.
        cutout(4, CutoutKind::Slot { plated: false }, concave(26.0, 6.0)),
        cutout(5, CutoutKind::Cutout, concave(34.0, 34.0)),
    ];
    solved.holes = vec![
        SolvedMountingHole {
            id: BoardFeatureId(6),
            at: [55.0, 5.0],
            drill: 2.2,
            plated: true,
            pad_diameter: Some(4.0),
        },
        SolvedMountingHole {
            id: BoardFeatureId(7),
            at: [5.0, 5.0],
            drill: 1.6,
            plated: false,
            pad_diameter: None,
        },
    ];
    p.board_shape = sketch(solved);
    rules(&mut p);
    let nets: Vec<u64> = p.nets.iter().map(|n| n.id).collect();
    let [a, b, ..] = nets[..] else {
        return Err("two demo nets".into());
    };
    for (net, at, tented) in [
        (a, [26.0, 14.0], true),
        (b, [24.0, 22.0], true),
        (b, [33.0, 24.0], false),
        (a, [34.0, 11.0], false),
    ] {
        let id = p.allocate()?;
        p.vias.push(Via {
            id,
            net,
            position: at,
            drill: 0.3,
            diameter: 0.6,
            span: ViaSpan::Through,
            tented,
            locked: false,
        });
    }
    for (net, layer, start, end) in [
        (a, CopperLayer::Top, [26.0, 14.0], [34.0, 11.0]),
        (b, CopperLayer::Top, [24.0, 22.0], [33.0, 24.0]),
        (b, CopperLayer::Top, [24.0, 22.0], [20.0, 30.0]),
        (a, CopperLayer::Bottom, [26.0, 14.0], [22.0, 30.0]),
        (b, CopperLayer::Bottom, [33.0, 24.0], [28.0, 16.0]),
    ] {
        let id = p.allocate()?;
        p.tracks.push(Track {
            id,
            net,
            start,
            end,
            width: 0.25,
            layer,
            locked: false,
        });
    }
    fill(&mut p, a, CopperLayer::Top)?;
    fill(&mut p, b, CopperLayer::Bottom)?;
    refill(&mut p)?;
    Ok(p)
}

/// A written set: file names and texts (the CSV, PDF and job files left
/// out).
pub(super) type Files = Vec<(String, String)>;

pub(super) fn written(project: &Project) -> Fallible<Files> {
    let set = fabrication::generate(
        &ProductionSnapshot::resolve(project, None)?,
        &FabricationOptions {
            base_name: Some("proof".into()),
            ..FabricationOptions::default()
        },
        &Default::default(),
    )?;
    set.files
        .into_iter()
        .filter(|f| f.name.ends_with(".gbr") || f.name.ends_with(".drl"))
        .map(|f| Ok((f.name, String::from_utf8(f.bytes)?)))
        .collect()
}

pub(super) fn file<'a>(files: &'a Files, suffix: &str) -> Fallible<&'a str> {
    files
        .iter()
        .find(|(name, _)| name == &format!("proof-{suffix}"))
        .map(|(_, text)| text.as_str())
        .ok_or_else(|| format!("no {suffix}").into())
}

pub(super) fn layer_name(project: &Project, layer: CopperLayer) -> Fallible<(String, String)> {
    let n = project
        .stackup
        .layers()
        .iter()
        .position(|l| *l == layer)
        .ok_or("a layer")?
        + 1;
    Ok(match layer {
        CopperLayer::Top => (format!("copper-l{n}-top.gbr"), format!("Copper,L{n},Top")),
        CopperLayer::Bottom => (
            format!("copper-l{n}-bottom.gbr"),
            format!("Copper,L{n},Bot"),
        ),
        CopperLayer::Inner(k) => (
            format!("copper-l{n}-inner{k}.gbr"),
            format!("Copper,L{n},Inr"),
        ),
    })
}

/// Every spacing the files show under its rule.
fn short(project: &Project, files: &Files) -> Fallible<Vec<Spacing>> {
    let mut layers = Vec::new();
    for layer in project.stackup.layers() {
        layers.push(reader::parse(file(files, &layer_name(project, layer)?.0)?)?);
    }
    let mut holes = Vec::new();
    for (suffix, plated) in [("plated.drl", true), ("non-plated.drl", false)] {
        if let Ok(text) = file(files, suffix) {
            holes.push((excellon_reader::parse(text)?, plated));
        }
    }
    let layers: Vec<_> = layers.iter().collect();
    let holes: Vec<_> = holes.iter().map(|(f, p)| (f, *p)).collect();
    Ok(spacing::fill_spacings(&layers, &holes, 1.5)?
        .into_iter()
        .filter(|s| s.gap_mm < clearance(s.against) - SPACING_SLACK_MM)
        .collect())
}

/// What is wrong with `files` as the production of `project`, each with the
/// check that says so.
fn check(project: &Project, files: &Files) -> Fallible<Vec<String>> {
    check_changed(project, files, &Vec::new())
}

/// [`check`], taking the images of the files `passed` holds unchanged, and
/// the spacing when every copper and drill file is, as checked.
fn check_changed(project: &Project, files: &Files, passed: &Files) -> Fallible<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let same = |suffix: &str| {
        let name = format!("proof-{suffix}");
        let at = |f: &'_ Files| f.iter().find(|x| x.0 == name).map(|x| x.1.clone());
        at(passed).is_some() && at(passed) == at(files)
    };
    let mut report = |check: &str, text: String| out.push(format!("{check}: {text}"));
    // The files there must be, by function, and no others.
    let mut expected: Vec<(String, String)> = Vec::new();
    for layer in project.stackup.layers() {
        expected.push(layer_name(project, layer)?);
    }
    for (side, word, code) in [(Layer::Top, "top", "Top"), (Layer::Bottom, "bottom", "Bot")] {
        expected.push((format!("mask-{word}.gbr"), format!("Soldermask,{code}")));
        if !oracle::paste(project, side)?.is_empty() {
            expected.push((format!("paste-{word}.gbr"), format!("Paste,{code}")));
        }
        if files
            .iter()
            .any(|f| f.0 == format!("proof-legend-{word}.gbr"))
        {
            expected.push((format!("legend-{word}.gbr"), format!("Legend,{code}")));
        }
    }
    expected.push(("profile.gbr".into(), "Profile,NP".into()));
    expected.push(("drill-map.gbr".into(), "Drillmap".into()));
    let layers = project.stackup.copper.len();
    let holes = oracle::holes(project)?;
    for (plated, suffix, function) in [
        (true, "plated.drl", format!("Plated,1,{layers},PTH")),
        (
            false,
            "non-plated.drl",
            format!("NonPlated,1,{layers},NPTH"),
        ),
    ] {
        if holes.iter().any(|h| h.plated == plated) {
            expected.push((suffix.into(), function));
        }
    }
    for (name, _) in files {
        if !expected.iter().any(|(e, _)| *name == format!("proof-{e}")) {
            report("files", format!("{name} is not a file of the board"));
        }
    }
    for (name, function) in &expected {
        let text = match file(files, name) {
            Ok(text) => text,
            Err(e) => {
                report("files", e.to_string());
                continue;
            }
        };
        let stated = if name.ends_with(".drl") {
            excellon_reader::parse(text)?
                .attribute(".FileFunction")
                .or(excellon_reader::parse(text)?.attribute("FileFunction"))
        } else {
            reader::parse(text)?
                .file_attribute(".FileFunction")
                .map(<[String]>::to_vec)
        };
        if stated.as_ref().map(|s| s.join(",")).as_ref() != Some(function) {
            report(
                "files",
                format!("{name} says it is {stated:?}, not {function}"),
            );
        }
    }
    let b = project.board().bounds();
    let window = [
        [b.min[0] - 1.0, b.min[1] - 1.0],
        [b.max[0] + 1.0, b.max[1] + 1.0],
    ];
    let image = |name: &str| file(files, name).and_then(Image::read);
    let mut layer = |name: String, design: Vec<oracle::Prim>, step: f64| {
        // A missing file is reported above.
        if !same(&name)
            && let Ok(image) = image(&name)
        {
            let differs = oracle::differences(&image, &design, window, step);
            if let Some(first) = differs.first() {
                report(
                    "image",
                    format!("{name}: {first}, and {} more", differs.len() - 1),
                );
            }
        }
    };
    for side in [Layer::Top, Layer::Bottom] {
        let word = if side == Layer::Top { "top" } else { "bottom" };
        layer(
            format!("mask-{word}.gbr"),
            oracle::mask(project, side),
            0.05,
        );
        layer(
            format!("paste-{word}.gbr"),
            oracle::paste(project, side)?,
            0.05,
        );
    }
    for copper in project.stackup.layers() {
        layer(
            layer_name(project, copper)?.0,
            oracle::copper(project, copper),
            0.2,
        );
    }
    // Every object of a copper layer once: pads, vias, tracks, washers.
    for copper in project.stackup.layers() {
        let name = layer_name(project, copper)?.0;
        let Ok(text) = file(files, &name) else {
            continue;
        };
        let parsed = reader::parse(text)?;
        let of = |f: &str| {
            parsed
                .objects
                .iter()
                .filter(|o| {
                    !matches!(o.kind, reader::Kind::Region { .. })
                        && o.aperture_attributes
                            .get(".AperFunction")
                            .and_then(|v| v.first())
                            .is_some_and(|v| v == f)
                })
                .count()
        };
        let pads = parsed
            .objects
            .iter()
            .filter(|o| o.object_attributes.contains_key(".C"))
            .count();
        let washers = project
            .board()
            .holes
            .iter()
            .filter(|h| h.plated && h.pad_diameter.is_some())
            .count();
        for (what, written, design) in [
            (
                "pads",
                pads,
                oracle::pads(project)
                    .iter()
                    .filter(|p| oracle::has_copper(p, copper))
                    .count(),
            ),
            ("vias", of("ViaPad"), project.vias.len()),
            (
                "tracks",
                of("Conductor"),
                project.tracks.iter().filter(|t| t.layer == copper).count(),
            ),
            ("washers", of("WasherPad"), washers),
        ] {
            if written != design {
                report(
                    "objects",
                    format!("{name}: {written} {what}, {design} in the design"),
                );
            }
        }
    }
    // Every contour of the profile, both ways.
    if let Ok(text) = file(files, "profile.gbr") {
        match outline::contours(&reader::parse(text)?) {
            Ok(drawn) => {
                let diff = outline::compare(&drawn, &oracle::profile(project), 0.0005);
                if !diff.passes() {
                    report(
                        "profile",
                        format!(
                            "{} contours not in the design, {} of the design not drawn",
                            diff.only_a.len(),
                            diff.only_b.len()
                        ),
                    );
                }
            }
            Err(e) => report("profile", e.to_string()),
        }
    }
    // Every hole and slot once, of its size and plating, and nothing else.
    for (plated, suffix) in [(true, "plated.drl"), (false, "non-plated.drl")] {
        let written = match file(files, suffix) {
            Ok(text) => drills::hits(&excellon_reader::parse(text)?, |_| true)?,
            Err(_) => Vec::new(),
        };
        let design: Vec<drills::DrillHit> = holes
            .iter()
            .filter(|h| h.plated == plated)
            .map(|h| drills::DrillHit {
                feature: if h.from == h.to {
                    drills::Feature::Round { at: h.from }
                } else {
                    drills::Feature::Slot {
                        from: h.from,
                        to: h.to,
                    }
                },
                diameter: h.width,
            })
            .collect();
        let diff = drills::pair(&written, &design, 0.0008);
        if !diff.passes() {
            report(
                "holes",
                format!(
                    "{suffix}: not in the design {:?}, not drilled {:?}",
                    diff.unmatched_a, diff.unmatched_b
                ),
            );
        }
    }
    let mut measured = vec!["plated.drl".to_owned(), "non-plated.drl".to_owned()];
    for copper in project.stackup.layers() {
        measured.push(layer_name(project, copper)?.0);
    }
    let unmeasured = !measured.iter().all(|name| same(name));
    let found = if unmeasured {
        short(project, files)
    } else {
        Ok(Vec::new())
    };
    match found {
        Ok(found) => {
            for s in found.iter().take(3) {
                report("spacing", format!("{s:?}, needs {}", clearance(s.against)));
            }
        }
        Err(e) => report("spacing", e.to_string()),
    }
    Ok(out)
}

#[test]
fn everything_the_files_hold_is_the_design() -> Fallible<()> {
    let project = board()?;
    let files = written(&project)?;
    let found = check(&project, &files)?;
    assert!(found.is_empty(), "{found:#?}");
    // The drawing holds the same holes and cutouts: the plated slot as a
    // routed hole, every other cutout as a contour.
    let drawing = fabrication::drawing::data(
        &ProductionSnapshot::resolve(&project, None)?,
        &Default::default(),
        &Default::default(),
    )?;
    let design = oracle::holes(&project)?;
    assert_eq!(drawing.holes.len(), design.len());
    for h in &design {
        let (w, p) = (h.width, h.plated);
        assert!(
            drawing.holes.iter().any(|d| d.plated == p
                && (d.feature.diameter() - w).abs() < 1e-6
                && match d.feature {
                    crate::layout::physical::HoleFeature::Round { at, .. } =>
                        near(at, h.from) && near(at, h.to),
                    crate::layout::physical::HoleFeature::Slot { from, to, .. } =>
                        near(from, h.from) && near(to, h.to) || near(from, h.to) && near(to, h.from),
                }),
            "{h:?} is not drawn"
        );
    }
    assert_eq!(drawing.cutouts.len(), 4);
    // The check measured something: fill copper meets every kind at its
    // rule, within a chord and the fill's minimum width.
    let all = {
        let mut layers = Vec::new();
        for layer in project.stackup.layers() {
            layers.push(reader::parse(file(
                &files,
                &layer_name(&project, layer)?.0,
            )?)?);
        }
        let plated = excellon_reader::parse(file(&files, "plated.drl")?)?;
        let bare = excellon_reader::parse(file(&files, "non-plated.drl")?)?;
        spacing::fill_spacings(
            &layers.iter().collect::<Vec<_>>(),
            &[(&plated, true), (&bare, false)],
            1.5,
        )?
    };
    for against in [
        Against::Track,
        Against::Pad,
        Against::ViaHole,
        Against::PlatedHole,
        Against::UnplatedHole,
    ] {
        let least = all
            .iter()
            .filter(|s| s.against == against)
            .map(|s| s.gap_mm)
            .fold(f64::INFINITY, f64::min);
        assert!(
            least < clearance(against) + 0.02 && least > clearance(against) - SPACING_SLACK_MM,
            "{against:?}: {least} mm, rule {}",
            clearance(against)
        );
    }
    Ok(())
}

pub(super) fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6
}

/// Fill copper by first principles: well clear of everything it must keep
/// from it is there, and closer than a rule it is not.
#[test]
fn pours_lie_where_the_rules_let_them_and_nowhere_else() -> Fallible<()> {
    let project = board()?;
    let files = written(&project)?;
    let outer = oracle::Area::of_outline(&project.board().outer);
    let cutouts: Vec<(oracle::Area, f64)> = project
        .board()
        .cutouts
        .iter()
        .map(|c| {
            let rule = if c.kind == (CutoutKind::Slot { plated: true }) {
                0.38
            } else {
                0.35
            };
            (oracle::Area::of_outline(&c.outline), rule)
        })
        .collect();
    let a = project.nets.first().ok_or("a net")?.id;
    let b = project.nets.get(1).ok_or("a net")?.id;
    for (layer, net) in [(CopperLayer::Top, a), (CopperLayer::Bottom, b)] {
        let image = Image::read(file(&files, &layer_name(&project, layer)?.0)?)?;
        // Everything the pour keeps away from and by how much.
        let mut keep: Vec<(oracle::Prim, f64)> = Vec::new();
        let connected = |pad: &oracle::DesignPad| pad.pad.terminal.is_some();
        for pad in oracle::pads(&project) {
            if oracle::has_copper(&pad, layer) {
                keep.push((
                    oracle::Prim::new(oracle::land(&pad.pad.shape), pad.frame, 0.0, &pad.name),
                    // A pad of a net may be joined: judged apart from it.
                    if connected(&pad) { -1.0 } else { 0.23 },
                ));
            }
        }
        for t in project.tracks.iter().filter(|t| t.layer == layer) {
            let d = [t.end[0] - t.start[0], t.end[1] - t.start[1]];
            keep.push((
                still(oracle::Area::capsule([0.0; 2], d, t.width / 2.0), t.start),
                if t.net == net { -1.0 } else { 0.21 },
            ));
        }
        for v in &project.vias {
            let own = v.net == net;
            keep.push((
                still(oracle::Area::disc([0.0; 2], v.diameter / 2.0), v.position),
                if own { -1.0 } else { 0.22 },
            ));
            keep.push((
                still(oracle::Area::disc([0.0; 2], v.drill / 2.0), v.position),
                if own { -1.0 } else { 0.45 },
            ));
        }
        for h in oracle::holes(&project)?.iter().filter(|h| !h.via) {
            let d = [h.to[0] - h.from[0], h.to[1] - h.from[1]];
            keep.push((
                still(oracle::Area::capsule([0.0; 2], d, h.width / 2.0), h.from),
                if h.plated { 0.38 } else { 0.31 },
            ));
        }
        for h in project.board().holes.iter().filter(|h| h.plated) {
            let r = h.pad_diameter.unwrap_or(h.drill) / 2.0;
            keep.push((still(oracle::Area::disc([0.0; 2], r), h.at), 0.23));
        }
        let objects = oracle::copper_but_fills(&project, layer);
        let (mut there, mut away) = (0, 0);
        for i in 0..=600 {
            for j in 0..=400 {
                let p = [f64::from(i) * 0.1, f64::from(j) * 0.1];
                // On other copper the image says nothing of the pour.
                if objects.iter().any(|o| o.excess(p) < 0.005) {
                    continue;
                }
                let edge = -outer.signed(p);
                let slack = keep
                    .iter()
                    .filter(|(_, rule)| *rule >= 0.0)
                    .map(|(prim, rule)| prim.excess(p) - rule)
                    .chain(cutouts.iter().map(|(c, rule)| c.signed(p) - rule))
                    .chain([
                        edge - 0.35,
                        p[0] - 1.0,
                        59.0 - p[0],
                        p[1] - 1.0,
                        39.0 - p[1],
                    ])
                    .fold(f64::INFINITY, f64::min);
                let own = keep
                    .iter()
                    .filter(|(_, rule)| *rule < 0.0)
                    .map(|(prim, _)| prim.excess(p))
                    .fold(f64::INFINITY, f64::min);
                if slack < -0.002 {
                    assert!(
                        !image.dark(p),
                        "{layer:?}: pour at {p:?}, {slack} mm inside a rule"
                    );
                    away += 1;
                } else if slack > 0.5 && own > 0.8 {
                    assert!(
                        image.dark(p),
                        "{layer:?}: no pour at {p:?}, {slack} mm clear"
                    );
                    there += 1;
                }
            }
        }
        assert!(there > 50_000 && away > 20_000, "{there} {away}");
    }
    Ok(())
}

fn still(area: oracle::Area, at: Point) -> oracle::Prim {
    oracle::Prim::new(area, oracle::Frame::still(at), 0.0, "")
}

#[test]
fn a_paste_reduction_that_splits_a_pad_prints_every_island() -> Fallible<()> {
    let project = board()?;
    let files = written(&project)?;
    let image = Image::read(file(&files, "paste-top.gbr")?)?;
    // R1 at (15, 15): the dumbbell is centred at (15, 11), its squares at
    // x = 14.2 and 15.8, each 0.84 mm across once shrunk by 0.08 mm. Beside the bar the corners
    // round it: at y = 11 the right one starts where hypot(x - 15.3, 0.05)
    // is 0.08, at x = 15.3624.
    for (p, dark) in [
        ([14.2, 11.0], true),
        ([15.8, 11.0], true),
        ([15.0, 11.0], false),
        ([13.79, 10.59], true),
        ([13.77, 10.57], false),
        ([16.21, 11.41], true),
        ([16.23, 11.43], false),
        ([15.35, 11.0], false),
        ([15.37, 11.0], true),
    ] {
        assert_eq!(image.dark(p), dark, "{p:?}");
    }
    // Two islands, each the square less 0.08 mm a side and the corner the
    // bar leaves: between its two round corners of 0.08 mm, 0.05 mm either
    // side of the middle.
    let (r, half): (f64, f64) = (0.08, 0.05);
    let under = (half * (r * r - half * half).sqrt() + r * r * (half / r).asin()) / 2.0;
    let area = 0.84 * 0.84 + 2.0 * (r * half - under);
    let film = reader::parse(file(&files, "paste-top.gbr")?)?;
    let islands: Vec<f64> = sparksmith_fabcheck::polygons::render(&film, |_| true)?
        .iter()
        .filter(|s| {
            s.iter().flatten().all(|p| {
                let (x, y) = (f64::from(p.x) / 1e6, f64::from(p.y) / 1e6);
                (13.6..16.4).contains(&x) && (10.4..11.6).contains(&y)
            })
        })
        .map(|s| sparksmith_fabcheck::polygons::area_mm2(std::slice::from_ref(s)))
        .collect();
    assert_eq!(islands.len(), 2, "{islands:?}");
    assert!(
        islands.iter().all(|a| (a - area).abs() < 1e-4),
        "{islands:?}, {area}"
    );
    Ok(())
}

#[test]
fn a_mask_opening_that_closes_round_a_cavity_keeps_the_mask_in_it() -> Fallible<()> {
    let mut project = board()?;
    for pad in project
        .library
        .footprints
        .iter_mut()
        .flat_map(|f| &mut f.definition.pads)
        .filter(|p| p.id == PadId(52))
    {
        pad.rotation_mdeg = Default::default();
    }
    // A second opening inside the cavity's: a via of 0.4 mm there must stay
    // open whatever the hole round it does.
    let net = project.nets.first().ok_or("a net")?.id;
    let id = project.allocate()?;
    project.vias.push(Via {
        id,
        net,
        position: [20.0, 11.0],
        drill: 0.1,
        diameter: 0.2,
        span: ViaSpan::Through,
        tented: false,
        locked: false,
    });
    let files = written(&project)?;
    let image = Image::read(file(&files, "mask-top.gbr")?)?;
    // R1's cavity is centred at (20, 11): grown by 0.05 mm it is a hole
    // 0.9 mm square; its gap (0.06 mm) is closed.
    for (p, dark) in [
        ([20.0, 11.0], true),
        ([20.4, 11.0], false),
        ([20.44, 11.4], false),
        ([20.46, 11.0], true),
        ([20.0, 11.7], true),
        ([19.56, 10.56], false),
        ([19.54, 10.54], true),
        ([21.54, 11.0], true),
        ([21.56, 11.0], false),
    ] {
        assert_eq!(image.dark(p), dark, "{p:?}");
    }
    Ok(())
}

/// The review's probe: a cutout whose ends bulge inwards ends at x = 4 mm;
/// as a slot of its arcs' centres it would reach x = 5 mm.
#[test]
fn only_a_true_slot_or_circle_is_a_slot_or_circle() {
    use crate::layout::physical::{HoleFeature, stadium};
    let tilted = slot_outline([1.0, 2.0], [4.0, 6.0], 1.5);
    assert!(
        matches!(stadium(&tilted), Some(HoleFeature::Slot { from, to, width })
            if (width - 1.5).abs() < 1e-9
                && (from[0] - 4.0).abs().min((from[0] - 1.0).abs()) < 1e-9
                && (to[0] - 4.0).abs().min((to[0] - 1.0).abs()) < 1e-9),
        "{:?}",
        stadium(&tilted)
    );
    assert_eq!(stadium(&concave(0.0, 0.0)), None);
    let outline = |start, segments: Vec<Segment>| Outline { start, segments };
    let line = |to| Segment::Line { to };
    let arc = |to, bulge| Segment::Arc { to, bulge };
    let bad = [
        // One end bulges inwards.
        (
            "half concave",
            outline(
                [0.0, -1.0],
                vec![
                    line([4.0, -1.0]),
                    arc([4.0, 1.0], 1.0),
                    line([0.0, 1.0]),
                    arc([0.0, -1.0], -1.0),
                ],
            ),
        ),
        // Both half turns on one side: the lines cross nothing but run
        // from arc to arc askew.
        (
            "not parallel",
            outline(
                [0.0, -1.0],
                vec![
                    line([4.0, -1.5]),
                    arc([4.0, 0.5], 1.0),
                    line([0.0, 1.0]),
                    arc([0.0, -1.0], 1.0),
                ],
            ),
        ),
        // The two lines side by side, the two arcs side by side.
        (
            "arcs together",
            outline(
                [0.0, 0.0],
                vec![
                    arc([2.0, 0.0], 1.0),
                    arc([4.0, 0.0], 1.0),
                    line([2.0, 5.0]),
                    line([0.0, 0.0]),
                ],
            ),
        ),
        (
            "open",
            outline(
                [0.0, -1.0],
                vec![
                    line([4.0, -1.0]),
                    arc([4.0, 1.0], 1.0),
                    line([0.0, 1.0]),
                    arc([0.0, -0.999], 1.0),
                ],
            ),
        ),
        // A near half turn (its ends 2 µm off tangent).
        (
            "not tangent",
            outline(
                [0.0, -1.0],
                vec![
                    line([4.0, -1.0]),
                    arc([4.002, 1.0], 1.0),
                    line([0.0, 1.0]),
                    arc([0.0, -1.0], 1.0),
                ],
            ),
        ),
        // Two half turns that go out and come back the same way.
        (
            "no area",
            outline(
                [1.0, 0.0],
                vec![arc([-1.0, 0.0], 1.0), arc([1.0, 0.0], -1.0)],
            ),
        ),
        // A lens of two flat arcs.
        (
            "lens",
            outline(
                [1.0, 0.0],
                vec![arc([-1.0, 0.0], 0.5), arc([1.0, 0.0], 0.5)],
            ),
        ),
        // A half disc.
        (
            "half disc",
            outline([1.0, 0.0], vec![arc([-1.0, 0.0], 1.0), line([1.0, 0.0])]),
        ),
        // A dented disc: two half turns on one diameter's ends, one dented
        // (a crescent's outline).
        (
            "crescent",
            outline(
                [1.0, 0.0],
                vec![arc([-1.0, 0.0], 1.0), arc([1.0, 0.0], -0.9)],
            ),
        ),
        (
            "rectangle",
            lines(&[[0.0, 0.0], [4.0, 0.0], [4.0, 1.0], [0.0, 1.0]]),
        ),
    ];
    for (what, o) in bad {
        assert_eq!(stadium(&o), None, "{what}");
    }
    // The same slot drawn clockwise, and starting at an arc.
    let cw = outline(
        [0.0, 1.0],
        vec![
            line([4.0, 1.0]),
            arc([4.0, -1.0], -1.0),
            line([0.0, -1.0]),
            arc([0.0, 1.0], -1.0),
        ],
    );
    assert!(matches!(stadium(&cw), Some(HoleFeature::Slot { .. })));
    let from_arc = outline(
        [4.0, -1.0],
        vec![
            arc([4.0, 1.0], 1.0),
            line([0.0, 1.0]),
            arc([0.0, -1.0], 1.0),
            line([4.0, -1.0]),
        ],
    );
    assert!(matches!(stadium(&from_arc), Some(HoleFeature::Slot { .. })));
    let round = outline(
        [1.0, 0.0],
        vec![arc([-1.0, 0.0], 1.0), arc([1.0, 0.0], 1.0)],
    );
    assert!(
        matches!(stadium(&round), Some(HoleFeature::Round { at, diameter })
            if at[0].abs() < 1e-9 && at[1].abs() < 1e-9 && (diameter - 2.0).abs() < 1e-9)
    );
}

/// No way, a waiver included, writes a plated slot the drill file cannot
/// route: as a profile outline it would be unplated.
#[test]
fn a_plated_slot_that_cannot_be_routed_is_never_written() -> Fallible<()> {
    // Both ends inwards; one inwards and one outwards (the audit's probe,
    // which a counting classifier drills as a slot); a plain 2 by 1 mm
    // rectangle (the audit's other probe).
    let mut one_inwards = concave(26.0, 20.0);
    if let Some(Segment::Arc { bulge, .. }) = one_inwards.segments.get_mut(3) {
        *bulge = 1.0;
    }
    for outline in [
        concave(26.0, 20.0),
        one_inwards,
        lines(&[[26.0, 20.0], [28.0, 20.0], [28.0, 21.0], [26.0, 21.0]]),
    ] {
        unroutable(outline)?;
    }
    Ok(())
}

fn unroutable(outline: Outline) -> Fallible<()> {
    let mut engine = Engine::demo()?;
    let mut file = engine.to_file();
    let mut solved = SolvedBoard::rectangle(file.project.board_size);
    solved.cutouts = vec![cutout(9, CutoutKind::Slot { plated: true }, outline)];
    file.project.board_shape = sketch(solved);
    engine = Engine::from_file(file)?;
    let preflight = |engine: &Engine| {
        fabrication::preflight::report(
            engine.project(),
            engine.drafts(),
            None,
            engine.check_settings(),
        )
    };
    let report = preflight(&engine)?;
    let item = report
        .items
        .iter()
        .find(|i| i.code == "fabrication.slot_unroutable.9")
        .ok_or_else(|| format!("{:#?}", report.items))?
        .clone();
    assert!(item.blocks() && !item.offers_waiver());
    let refused = engine.apply(
        engine.revision(),
        Operation::Fabrication(fabrication::FabricationCommand::Waive {
            keys: vec![item.key.clone()],
            reason: "accepted".into(),
        }),
    );
    assert!(
        refused.is_err() && format!("{refused:?}").contains("fabrication.not_waivable"),
        "{refused:?}"
    );
    // A waiver a file holds is ignored.
    let mut file = engine.to_file();
    file.project
        .production
        .waivers
        .push(crate::model::production::Waiver {
            key: item.key.clone(),
            reason: "accepted".into(),
        });
    let engine = Engine::from_file(file)?;
    let report = preflight(&engine)?;
    assert!(report.items.iter().any(|i| i.key == item.key && i.blocks()));
    // And every writer refuses the board.
    let project = engine.project();
    let options = FabricationOptions::default();
    let snapshot = ProductionSnapshot::resolve(project, None)?;
    let refusal = fabrication::generate(&snapshot, &options, &Default::default());
    assert!(
        refusal.is_err() && format!("{refusal:?}").contains("fabrication.slot_unroutable"),
        "{refusal:?}"
    );
    let identity = Default::default();
    assert!(fabrication::artwork::layers(project, &identity, &options, "x").is_err());
    assert!(fabrication::excellon::files(project, &identity, &options, "x").is_err());
    // Alone, too: the drawing has nowhere to show it.
    assert!(fabrication::drawing::data(&snapshot, &identity, &Default::default()).is_err());
    for (gerber, drill, drawings) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let options = FabricationOptions {
            outputs: fabrication::Outputs {
                gerber,
                drill,
                drawings,
                assembly: false,
            },
            ..FabricationOptions::default()
        };
        assert!(fabrication::generate(&snapshot, &options, &Default::default()).is_err());
    }
    Ok(())
}

/// The audit's probe on the showcase: the fill-to-via-hole rule raised to
/// 1 mm, both stale findings waived through the command and saved. The
/// export stays blocked by the copper that would be made; a refill makes
/// the gap, read back from the files.
#[test]
fn waiving_the_showcases_stale_fills_does_not_clear_a_raised_drill_rule() -> Fallible<()> {
    use crate::layout::fill::{FillCommand, RefillTarget};
    let mut engine = Engine::showcase()?;
    let blocking = |engine: &Engine| -> Fallible<Vec<(String, String)>> {
        let report = fabrication::preflight::report(
            engine.project(),
            engine.drafts(),
            None,
            engine.check_settings(),
        )?;
        Ok(report
            .items
            .iter()
            .filter(|i| i.blocks())
            .map(|i| (i.code.clone(), i.key.clone()))
            .collect())
    };
    // The least gap between a pour and a drill of another net's via.
    let least = |engine: &Engine| -> Fallible<f64> {
        let set = fabrication::generate(
            &ProductionSnapshot::resolve(engine.project(), None)?,
            &FabricationOptions::default(),
            &Default::default(),
        )?;
        let text = |part: &str| -> Fallible<Vec<String>> {
            set.files
                .iter()
                .filter(|f| f.name.contains(part))
                .map(|f| Ok(String::from_utf8(f.bytes.clone())?))
                .collect()
        };
        let layers = text("-copper-")?
            .iter()
            .map(|t| reader::parse(t))
            .collect::<Fallible<Vec<_>>>()?;
        let plated = text("-plated.drl")?;
        let plated = plated
            .iter()
            .filter(|t| t.contains("TF.FileFunction,Plated"))
            .map(|t| excellon_reader::parse(t))
            .collect::<Fallible<Vec<_>>>()?;
        Ok(spacing::fill_spacings(
            &layers.iter().collect::<Vec<_>>(),
            &plated.iter().map(|f| (f, true)).collect::<Vec<_>>(),
            2.0,
        )?
        .into_iter()
        .filter(|s| s.against == Against::ViaHole)
        .map(|s| s.gap_mm)
        .fold(f64::INFINITY, f64::min))
    };
    let before = blocking(&engine)?;
    engine.apply(
        engine.revision(),
        Operation::Rules(crate::rules::RulesCommand::SetOverride {
            key: RuleKey::Clearance {
                a: CopperObject::Fill,
                b: CopperObject::ViaHole,
            }
            .normalized(),
            value: RuleValue::Length(1.0),
        }),
    )?;
    let stale: Vec<String> = blocking(&engine)?
        .into_iter()
        .filter(|(code, _)| code == "fill.stale")
        .map(|(_, key)| key)
        .collect();
    assert_eq!(stale.len(), 2);
    engine.apply(
        engine.revision(),
        Operation::Fabrication(fabrication::FabricationCommand::Waive {
            keys: stale.clone(),
            reason: "accepted".into(),
        }),
    )?;
    // Held by the document, as a saved file holds it.
    assert_eq!(engine.project().production.waivers.len(), 2);
    let short = blocking(&engine)?;
    assert!(
        short.iter().any(|(code, _)| code == "fill.clearance"),
        "{short:?}"
    );
    let gap = least(&engine)?;
    assert!((gap - 0.35).abs() < 0.01, "{gap}");
    for key in stale {
        engine.apply(
            engine.revision(),
            Operation::Fabrication(fabrication::FabricationCommand::Unwaive { key }),
        )?;
    }
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Refill {
            target: RefillTarget::All,
        }),
    )?;
    let gap = least(&engine)?;
    assert!(gap > 1.0 - SPACING_SLACK_MM, "{gap}");
    let after = blocking(&engine)?;
    assert!(
        !after.iter().any(|(code, _)| code.starts_with("fill.")),
        "{after:?} {before:?}"
    );
    Ok(())
}

/// Copper kept as it was when a fill's own clearance, or a pad's, is
/// raised: each is a rule the retained copper is held to.
#[test]
fn retained_copper_keeps_the_fills_own_and_a_pads_own_clearance() -> Fallible<()> {
    use crate::diagnostics::ObjectRef;
    let mut project = board()?;
    let id = project.fills.first().ok_or("a fill")?.id;
    let found = |p: &Project| -> Vec<String> {
        crate::layout::fill::findings(p)
            .into_iter()
            .filter(|f| f.code == "fill.clearance" && f.objects.contains(&ObjectRef::Fill { id }))
            .map(|f| f.message)
            .collect()
    };
    // The stored copper under a current key, so only the audit can tell.
    let keep = |p: &mut Project| -> Fallible<()> {
        let key = crate::layout::fill::inputs_key(p, id)?;
        for f in p.fills.iter_mut().filter(|f| f.id == id) {
            if let Some(c) = &mut f.computed {
                c.key.clone_from(&key);
            }
        }
        Ok(())
    };
    assert!(found(&project).is_empty(), "{:?}", found(&project));
    let clean = project.clone();
    if let Some(f) = project.fills.iter_mut().find(|f| f.id == id) {
        f.settings.clearance = Some(0.6);
    }
    keep(&mut project)?;
    let own = found(&project);
    assert!(
        own.iter()
            .any(|m| m.contains("needs 0.6 mm (the fill: clearance)")),
        "{own:?}"
    );
    let mut project = clean;
    let instance = project.placements.first().ok_or("R1")?.instance;
    let Some(placement) = project.placements.first_mut() else {
        return Err("R1".into());
    };
    placement.fill_override.clearance = Some(nm(0.7));
    keep(&mut project)?;
    let own = found(&project);
    assert!(
        own.iter()
            .any(|m| m.contains("needs 0.7 mm (the pad's fill override: clearance)")),
        "{own:?} {instance}"
    );
    Ok(())
}

/// One pad's own fill clearance, the rules' fill clearance, and a pad's
/// farther than any clearance rule reaches: each holds retained copper,
/// and only where it applies.
#[test]
fn retained_copper_keeps_one_pads_and_the_rules_fill_clearance() -> Fallible<()> {
    use crate::diagnostics::ObjectRef;
    use crate::model::fill::{PadFillOverride, PadFillOverrideEntry};
    let clean = board()?;
    let id = clean.fills.first().ok_or("a fill")?.id;
    let r1 = clean.placements.first().ok_or("R1")?.instance;
    let found = |p: &mut Project| -> Fallible<Vec<(ObjectRef, String)>> {
        let key = crate::layout::fill::inputs_key(p, id)?;
        for f in p.fills.iter_mut().filter(|f| f.id == id) {
            if let Some(c) = &mut f.computed {
                c.key.clone_from(&key);
            }
        }
        Ok(crate::layout::fill::findings(p)
            .into_iter()
            .filter(|f| f.code == "fill.clearance")
            .filter_map(|f| match f.objects.as_slice() {
                [ObjectRef::Fill { id: f_id }, other] if *f_id == id => Some((*other, f.message)),
                _ => None,
            })
            .collect())
    };
    let set = |p: &mut Project, pad: u32, mm: f64| -> Fallible<()> {
        let placement = p.placements.first_mut().ok_or("R1")?;
        placement.pad_fill_overrides = vec![PadFillOverrideEntry {
            pad: PadId(pad),
            settings: PadFillOverride {
                clearance: Some(nm(mm)),
                ..PadFillOverride::default()
            },
        }];
        Ok(())
    };
    // R1's D-shaped pad alone.
    let d_shape = ObjectRef::Pad {
        instance: r1,
        pad: PadId(50),
    };
    let mut project = clean.clone();
    set(&mut project, 50, 0.7)?;
    let own = found(&mut project)?;
    assert!(
        matches!(own.as_slice(), [(o, m)] if *o == d_shape
            && m.contains("needs 0.7 mm (the pad's fill override: clearance)")),
        "{own:?}"
    );
    // The rules' fill clearance, over every copper pair it exceeds and no
    // hole: the holes' own rules stay.
    let mut project = clean.clone();
    project
        .rules
        .overrides
        .retain(|o| o.key != RuleKey::FillClearance);
    project.rules.overrides.push(RuleOverride {
        key: RuleKey::FillClearance,
        value: RuleValue::Length(0.5),
    });
    let own = found(&mut project)?;
    assert!(own.iter().any(|(o, _)| *o == d_shape), "{own:?}");
    assert!(
        own.iter()
            .all(|(_, m)| m.contains("needs 0.5 mm (") && m.contains("fill clearance")),
        "{own:?}"
    );
    // Copper 2.5 mm from R1's fiducial (a 1 mm disc at (15, 21)), which no
    // rule of at most 0.45 mm looks for.
    let mark = ObjectRef::Pad {
        instance: r1,
        pad: PadId(56),
    };
    for (mm, reported) in [(3.0, true), (2.4, false)] {
        let mut project = clean.clone();
        project.tracks.clear();
        project.vias.clear();
        set(&mut project, 56, mm)?;
        for c in project
            .fills
            .iter_mut()
            .filter(|f| f.id == id)
            .filter_map(|f| f.computed.as_mut())
        {
            c.regions = vec![FilledRegion {
                outer: vec![[18.0, 20.9], [19.0, 20.9], [19.0, 21.1], [18.0, 21.1]],
                holes: Vec::new(),
            }];
        }
        let own = found(&mut project)?;
        assert_eq!(own.iter().any(|(o, _)| *o == mark), reported, "{own:?}");
        assert!(
            own.iter().all(|(o, m)| *o == mark && m.contains("is 2.5")),
            "{own:?}"
        );
    }
    Ok(())
}

/// The review's five steps: a person waives a pour's staleness and the gap
/// to one via (0.35 mm where a raised drill rule asks for 1 mm), then the
/// via is moved into the pour. The short is another thing than the gap that
/// was accepted, so it must block.
#[test]
#[ignore = "waits for the founder's waiver decision"]
fn accepting_a_gap_never_accepts_a_later_short() -> Fallible<()> {
    use crate::diagnostics::ObjectRef;
    use crate::layout::LayoutCommand;
    use crate::model::production::{Waiver, waiver_key};
    let mut engine = Engine::showcase()?;
    // 1. The drill rule is raised: the pours are stale.
    engine.apply(
        engine.revision(),
        Operation::Rules(crate::rules::RulesCommand::SetOverride {
            key: RuleKey::Clearance {
                a: CopperObject::Fill,
                b: CopperObject::ViaHole,
            }
            .normalized(),
            value: RuleValue::Length(1.0),
        }),
    )?;
    // 2. Held by the document, whatever the command accepts: each pour's
    // staleness, and its gap to every via.
    let mut file = engine.to_file();
    let project = &mut file.project;
    let mut keys = Vec::new();
    for fill in &project.fills {
        let me = ObjectRef::Fill { id: fill.id };
        keys.push(waiver_key("fill.stale", &[me]));
        for via in &project.vias {
            keys.push(waiver_key(
                "fill.clearance",
                &[me, ObjectRef::Via { id: via.id }],
            ));
        }
    }
    keys.sort();
    keys.dedup();
    project.production.waivers = keys
        .into_iter()
        .map(|key| Waiver {
            key,
            reason: "accepted".into(),
        })
        .collect();
    let mut engine = Engine::from_file(file)?;
    // 3. A via of another net goes where the pour is solid: 0.8 mm from its
    // edge, and 1.5 mm from all other copper, holes and the board's edge.
    let project = engine.project().clone();
    let holes = oracle::holes(&project)?;
    let outer = oracle::Area::of_outline(&project.board().outer);
    let copper: Vec<oracle::Prim> = project
        .stackup
        .layers()
        .into_iter()
        .flat_map(|layer| oracle::copper_but_fills(&project, layer))
        .collect();
    let free = |p: Point, moved: Point| {
        -outer.signed(p) > 1.5
            && copper
                .iter()
                .all(|c| near(c.frame.to_board([0.0; 2]), moved) || c.excess(p) > 1.5)
            && holes.iter().all(|h| {
                near(h.from, moved)
                    || oracle::Area::capsule(h.from, h.to, h.width / 2.0).signed(p) > 1.5
            })
    };
    let bounds = project.board().bounds();
    let mut target = None;
    'search: for fill in project.fills.iter().filter(|f| f.net.is_some()) {
        let Some(via) = project
            .vias
            .iter()
            .find(|v| !v.locked && Some(v.net) != fill.net)
        else {
            continue;
        };
        let mut solid = Vec::new();
        for region in fill.computed.iter().flat_map(|c| &c.regions) {
            let mut area = oracle::Area::of_ring(&region.outer);
            for hole in &region.holes {
                area.edges.extend(oracle::Area::of_ring(hole).edges);
            }
            solid.push(area.filed());
        }
        // Solid all round, 0.8 mm out.
        let poured = |p: Point| {
            solid.iter().any(|a| {
                a.inside(p)
                    && (0..16).all(|k| {
                        let (s, c) = (f64::from(k) * std::f64::consts::TAU / 16.0).sin_cos();
                        a.inside([p[0] + 0.8 * c, p[1] + 0.8 * s])
                    })
            })
        };
        for i in 0..400 {
            for j in 0..400 {
                let p = [
                    bounds.min[0] + 0.25 * f64::from(i),
                    bounds.min[1] + 0.25 * f64::from(j),
                ];
                if p[0] > bounds.max[0] || p[1] > bounds.max[1] {
                    break;
                }
                if poured(p) && free(p, via.position) {
                    target = Some((fill.id, via.id, p));
                    break 'search;
                }
            }
        }
    }
    let (fill, via, position) = target.ok_or("a place in a pour for a via")?;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::MoveVia { id: via, position }),
    )?;
    // 4. and 5. The files would short the two nets; the export must not be
    // free to write them.
    let report = fabrication::preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?;
    let names = [ObjectRef::Fill { id: fill }, ObjectRef::Via { id: via }];
    let short: Vec<_> = report
        .items
        .iter()
        .filter(|i| names.iter().all(|n| i.objects.contains(n)))
        .map(|i| (&i.code, &i.title, i.blocks()))
        .collect();
    assert!(short.iter().any(|(_, _, blocks)| *blocks), "{short:?}");
    Ok(())
}

/// The review's probe: the fill-to-via-hole minimum raised to 1 mm after
/// the pour was computed, and its staleness waived. The stored copper is
/// what would be made, so its gap to the drill blocks the export, naming
/// the via; a drill rule no ring covers is measured on fresh copper too.
#[test]
fn a_via_drill_closer_to_a_pour_than_its_rule_blocks() -> Fallible<()> {
    use crate::diagnostics::ObjectRef;
    use crate::model::production::{Waiver, waiver_key};
    let mut project = board()?;
    let net = project.nets.get(1).ok_or("a net")?.id;
    let via = project
        .vias
        .iter()
        .find(|v| v.net == net)
        .ok_or("a via")?
        .id;
    let fill = project.fills.first().ok_or("a fill")?.id;
    let names = |project: &Project| -> Vec<String> {
        crate::layout::fill::findings(project)
            .into_iter()
            .filter(|f| {
                f.code == "fill.clearance"
                    && f.objects == vec![ObjectRef::Fill { id: fill }, ObjectRef::Via { id: via }]
            })
            .map(|f| f.message)
            .collect()
    };
    assert!(names(&project).is_empty(), "{:?}", names(&project));
    let key = RuleKey::Clearance {
        a: CopperObject::Fill,
        b: CopperObject::ViaHole,
    }
    .normalized();
    for o in project.rules.overrides.iter_mut().filter(|o| o.key == key) {
        o.value = RuleValue::Length(1.0);
    }
    // Stale: its refill makes the gap, so only overlap is listed.
    assert!(names(&project).is_empty(), "{:?}", names(&project));
    let stale = waiver_key("fill.stale", &[ObjectRef::Fill { id: fill }]);
    project.production.waivers.push(Waiver {
        key: stale,
        reason: "accepted".into(),
    });
    let found = names(&project);
    let [message] = found.as_slice() else {
        return Err(format!("{found:?}").into());
    };
    assert!(
        message.contains("from the drill of a via on") && message.contains("needs 1 mm"),
        "{message}"
    );
    // The files agree: the drill is 0.45 mm from the pour.
    let files = written(&project)?;
    let top = reader::parse(file(&files, "copper-l1-top.gbr")?)?;
    let plated = excellon_reader::parse(file(&files, "plated.drl")?)?;
    let at = project.via(via).ok_or("the via")?.position;
    let gap = spacing::fill_spacings(&[&top], &[(&plated, true)], 1.5)?
        .into_iter()
        .find(|s| s.against == Against::ViaHole && s.at == at)
        .ok_or("a spacing")?
        .gap_mm;
    assert!((gap - 0.45).abs() < 0.01, "{gap}");
    // Fresh copper whose key is current but whose regions are not what the
    // rules make: the drill is measured there too.
    project.production.waivers.clear();
    let current = crate::layout::fill::inputs_key(&project, fill)?;
    for f in project.fills.iter_mut().filter(|f| f.id == fill) {
        if let Some(c) = &mut f.computed {
            c.key = current.clone();
        }
    }
    assert_eq!(names(&project).len(), 1);
    Ok(())
}

/// Two pours that overlap (a violation a person may waive): each keeps its
/// copper in the other's holes, so the files show the design.
#[test]
fn overlapping_pours_erase_nothing_of_each_other() -> Fallible<()> {
    let mut project = board()?;
    let b = project.nets.get(1).ok_or("a net")?.id;
    fill(&mut project, b, CopperLayer::Top)?;
    let ring = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let mut regions = [
        // The larger: 20 by 10 mm, solid.
        FilledRegion {
            outer: ring(20.0, 20.0, 40.0, 30.0),
            holes: Vec::new(),
        },
        // The smaller lies across its edge, with a hole over both.
        FilledRegion {
            outer: ring(38.0, 22.0, 44.0, 28.0),
            holes: vec![ring(39.0, 23.0, 43.0, 27.0)],
        },
    ]
    .into_iter();
    for f in project
        .fills
        .iter_mut()
        .filter(|f| f.layer == CopperLayer::Top)
    {
        f.computed = Some(FilledCopper {
            key: String::new(),
            regions: regions.next().into_iter().collect(),
            removed_islands: 0,
            removed_spokes: Vec::new(),
        });
    }
    let files = written(&project)?;
    let image = Image::read(file(&files, "copper-l1-top.gbr")?)?;
    assert!(image.dark([39.5, 25.0]), "erased in the other's hole");
    assert!(!image.dark([42.0, 25.0]), "the hole itself");
    assert!(image.dark([38.5, 22.5]) && image.dark([43.5, 25.0]));
    Ok(())
}

/// A fill's regions that overlap, or a hole outside its ring, are refused
/// by `fill.store`; held by a document all the same, they are plotted as
/// the copper they say: each outer ring less its own holes, together.
#[test]
fn a_fills_own_overlapping_regions_erase_nothing() -> Fallible<()> {
    use crate::layout::fill::{FillCommand, inputs_key};
    let ring = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let region = |outer, holes| FilledRegion { outer, holes };
    let overlapping = vec![
        region(ring(20.0, 20.0, 30.0, 30.0), Vec::new()),
        region(
            ring(20.5, 20.5, 29.5, 29.5),
            vec![ring(23.0, 23.0, 26.0, 26.0)],
        ),
    ];
    let stray = vec![region(
        ring(20.0, 20.0, 25.0, 25.0),
        vec![ring(27.0, 20.0, 29.0, 22.0)],
    )];
    let crossing = vec![region(
        ring(20.0, 20.0, 30.0, 30.0),
        vec![ring(21.0, 21.0, 24.0, 24.0), ring(23.0, 23.0, 26.0, 26.0)],
    )];
    let across = vec![region(
        ring(20.0, 20.0, 25.0, 25.0),
        vec![ring(24.0, 21.0, 27.0, 23.0)],
    )];
    let sound = vec![
        region(
            ring(20.0, 20.0, 30.0, 30.0),
            vec![ring(22.0, 22.0, 28.0, 28.0)],
        ),
        // An island in the other's hole.
        region(ring(23.0, 23.0, 27.0, 27.0), Vec::new()),
    ];
    let mut engine = Engine::showcase()?;
    let id = engine.project().fills.first().ok_or("a fill")?.id;
    let store = |engine: &mut Engine, regions: &Vec<FilledRegion>| -> Fallible<bool> {
        let copper = FilledCopper {
            key: inputs_key(engine.project(), id)?,
            regions: regions.clone(),
            removed_islands: 0,
            removed_spokes: Vec::new(),
        };
        let done = engine.apply(
            engine.revision(),
            Operation::Fill(FillCommand::Store {
                fill: id,
                copper: Box::new(copper),
            }),
        );
        Ok(done.is_ok())
    };
    let before = engine.project().clone();
    for regions in [&overlapping, &stray, &crossing, &across] {
        assert!(!store(&mut engine, regions)?, "{regions:?}");
        assert_eq!(engine.project(), &before);
    }
    assert!(store(&mut engine, &sound)?);
    // Every computed fill is sound.
    for project in [before, board()?] {
        for fill in &project.fills {
            let regions = fill.computed.as_ref().ok_or("copper")?.regions.as_slice();
            assert!(crate::layout::fill::sound(regions), "{}", fill.name);
        }
    }
    // Plotted: what each says is copper, and nothing else.
    for (regions, dark, clear) in [
        (
            overlapping,
            vec![[24.5, 24.5], [20.2, 25.0], [29.8, 29.8]],
            vec![[19.9, 25.0], [30.1, 25.0]],
        ),
        (stray, vec![[22.0, 22.0]], vec![[28.0, 21.0], [26.0, 21.0]]),
        (
            crossing,
            vec![[20.5, 20.5], [28.0, 28.0]],
            vec![[22.0, 22.0], [23.5, 23.5], [25.0, 25.0]],
        ),
        (
            across,
            vec![[22.0, 22.0], [24.5, 24.0]],
            vec![[24.5, 22.0], [26.0, 22.0]],
        ),
    ] {
        let mut project = board()?;
        // Nothing but the pour near these points.
        project.tracks.clear();
        project.vias.clear();
        for f in project
            .fills
            .iter_mut()
            .filter(|f| f.layer == CopperLayer::Top)
        {
            f.computed = Some(FilledCopper {
                key: String::new(),
                regions: regions.clone(),
                removed_islands: 0,
                removed_spokes: Vec::new(),
            });
        }
        let files = written(&project)?;
        let image = Image::read(file(&files, "copper-l1-top.gbr")?)?;
        // The fixture's own lands may lie on a point: it is then no test.
        let lands = oracle::copper_but_fills(&project, CopperLayer::Top);
        for (p, want) in dark
            .iter()
            .map(|p| (*p, true))
            .chain(clear.iter().map(|p| (*p, false)))
        {
            assert!(
                lands.iter().all(|l| l.excess(p) > 0.0),
                "{p:?} is on a land"
            );
            assert_eq!(image.dark(p), want, "{p:?} of {regions:?}");
        }
    }
    Ok(())
}

/// An arc shorter than the format's step would be read as a full circle.
#[test]
fn an_arc_shorter_than_the_format_draws_no_circle() -> Fallible<()> {
    let mut project = board()?;
    let mut solved = SolvedBoard::rectangle([60.0, 40.0]);
    // A corner blended by a 0.2 nm arc of 5 mm radius.
    let tiny = 2e-7;
    solved.outer = Outline {
        start: [0.0, 0.0],
        segments: vec![
            Segment::Line { to: [60.0, 0.0] },
            Segment::Arc {
                to: [60.0, tiny],
                bulge: tiny / 20.0,
            },
            Segment::Line { to: [60.0, 40.0] },
            Segment::Line { to: [0.0, 40.0] },
            Segment::Line { to: [0.0, 0.0] },
        ],
    };
    project.board_shape = sketch(solved);
    let files = written(&project)?;
    let parsed = reader::parse(file(&files, "profile.gbr")?)?;
    assert!(
        !parsed
            .objects
            .iter()
            .any(|o| matches!(o.kind, reader::Kind::Arc { .. })),
        "{:?}",
        parsed.objects
    );
    Ok(())
}

/// The whole list of `key` lines of a file's text, with their numbers.
pub(super) fn line_numbers(text: &str, matches: impl Fn(&str) -> bool) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| matches(l))
        .map(|(k, _)| k)
        .collect()
}

pub(super) fn edit(files: &mut Files, suffix: &str, change: impl Fn(Vec<String>) -> Vec<String>) {
    for (name, text) in files {
        if *name == format!("proof-{suffix}") {
            let lines: Vec<String> = text.lines().map(str::to_owned).collect();
            *text = change(lines).join("\n") + "\n";
        }
    }
}

/// `line` with its `axis` word changed by `by` (mm as written: nm in
/// Gerber, decimals in Excellon).
pub(super) fn shifted(line: &str, axis: char, by: f64, decimals: bool) -> String {
    let Some(at) = line.find(axis) else {
        return line.to_owned();
    };
    let rest = &line[at + 1..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.'))
        .unwrap_or(rest.len());
    let value: f64 = rest[..end].parse().unwrap_or(0.0);
    let value = if decimals {
        format!("{:.3}", value + by)
    } else {
        format!("{}", (value + by * 1e6).round() as i64)
    };
    format!("{}{axis}{value}{}", &line[..at], &rest[end..])
}

/// `line`'s `axis` word mirrored about `2·about` (mm).
pub(super) fn mirrored(line: &str, axis: char, about: f64, decimals: bool) -> String {
    let Some(at) = line.find(axis) else {
        return line.to_owned();
    };
    let rest = &line[at + 1..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.'))
        .unwrap_or(rest.len());
    let value: f64 = rest[..end].parse().unwrap_or(0.0);
    let value = if decimals {
        format!("{:.3}", 2.0 * about - value)
    } else {
        format!("{}", (2e6 * about - value).round() as i64)
    };
    format!("{}{axis}{value}{}", &line[..at], &rest[end..])
}

pub(super) fn without(lines: Vec<String>, gone: std::ops::RangeInclusive<usize>) -> Vec<String> {
    lines
        .into_iter()
        .enumerate()
        .filter(|(k, _)| !gone.contains(k))
        .map(|(_, l)| l)
        .collect()
}

/// The regions (`G36` to `G37`) of a Gerber file, as line ranges.
pub(super) fn regions(lines: &[String]) -> Vec<std::ops::RangeInclusive<usize>> {
    let mut out = Vec::new();
    let mut from = None;
    for (k, l) in lines.iter().enumerate() {
        match l.as_str() {
            "G36*" => from = Some(k),
            "G37*" => out.extend(from.take().map(|f| f..=k)),
            _ => {}
        }
    }
    out
}

/// Without the region at lines `gone`. The modes a region sets hold after
/// it: they stay, so the rest of the file reads as it did.
pub(super) fn without_region(
    lines: Vec<String>,
    gone: std::ops::RangeInclusive<usize>,
) -> Vec<String> {
    lines
        .into_iter()
        .enumerate()
        .filter(|(k, l)| !gone.contains(k) || l.starts_with("G0"))
        .map(|(_, l)| l)
        .collect()
}

/// Each kind of output fault, seeded in the files, with the check that must
/// report it. One the check passes is a hole in the check.
#[test]
fn every_seeded_fault_is_reported() -> Fallible<()> {
    type Fault = (&'static str, &'static str, Box<dyn Fn(&mut Files)>);
    let project = board()?;
    let clean = written(&project)?;
    assert!(check(&project, &clean)?.is_empty());
    let drop_region = |suffix: &'static str, k: usize| {
        move |f: &mut Files| {
            edit(f, suffix, |l| {
                let r = regions(&l);
                match r.get(k).cloned() {
                    Some(r) => without_region(l, r),
                    None => l,
                }
            });
        }
    };
    // A tool's first hit, and slots, of a drill file.
    let hits = |l: &[String]| line_numbers(&l.join("\n"), |s| s.starts_with('X'));
    let slots = |l: &[String]| line_numbers(&l.join("\n"), |s| s.contains("G85"));
    let circle_tool = |l: &str| l.starts_with("%ADD") && l.contains("C,");
    let on_hit = move |suffix: &'static str, change: fn(&str) -> String| {
        move |f: &mut Files| {
            edit(f, suffix, |mut l| {
                if let Some(k) = hits(&l).first().copied() {
                    l[k] = change(&l[k]);
                }
                l
            });
        }
    };
    let on_slot = move |change: fn(&str) -> String| {
        move |f: &mut Files| {
            edit(f, "plated.drl", |mut l| {
                if let Some(k) = slots(&l).first().copied() {
                    l[k] = change(&l[k]);
                }
                l
            });
        }
    };
    let replace = |suffix: &'static str, from: &'static str, to: &'static str| {
        move |f: &mut Files| {
            edit(f, suffix, |l| {
                l.join("\n")
                    .replacen(from, to, 1)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            });
        }
    };
    let drop_flash = |suffix: &'static str| {
        move |f: &mut Files| {
            edit(f, suffix, |l| {
                let k = line_numbers(&l.join("\n"), |s| s.ends_with("D03*"));
                match k.first().copied() {
                    Some(k) => without(l, k..=k),
                    None => l,
                }
            });
        }
    };
    // The last circular aperture a file defines, 40 µm smaller.
    let shrink_aperture = move |suffix: &'static str| {
        move |f: &mut Files| {
            edit(f, suffix, |l| {
                let mut done = false;
                let mut out: Vec<String> = l
                    .into_iter()
                    .rev()
                    .map(|s| {
                        if done || !circle_tool(&s) {
                            return s;
                        }
                        done = true;
                        match s.split_once("C,") {
                            Some((a, d)) => {
                                let d: f64 = d.trim_end_matches("*%").parse().unwrap_or(1.0);
                                format!("{a}C,{:.6}*%", d - 0.04)
                            }
                            None => s,
                        }
                    })
                    .collect();
                out.reverse();
                out
            });
        }
    };
    let mirror = |suffix: &'static str| {
        move |f: &mut Files| {
            let excellon = suffix.ends_with(".drl");
            edit(f, suffix, |l| {
                l.into_iter()
                    .map(|s| {
                        if excellon {
                            return match s.split_once("G85") {
                                Some((a, b)) => format!(
                                    "{}G85{}",
                                    mirrored(a, 'X', 30.0, true),
                                    mirrored(b, 'X', 30.0, true)
                                ),
                                None if s.starts_with('X') => mirrored(&s, 'X', 30.0, true),
                                None => s,
                            };
                        }
                        match s.as_str() {
                            "G02*" => "G03*".into(),
                            "G03*" => "G02*".into(),
                            _ if s.starts_with('X') || s.starts_with('Y') => {
                                mirrored(&mirrored(&s, 'X', 30.0, false), 'I', 0.0, false)
                            }
                            _ => s,
                        }
                    })
                    .collect()
            });
        }
    };
    let swap = |a: &'static str, b: &'static str| {
        move |f: &mut Files| {
            let at = |f: &Files, s: &str| f.iter().position(|x| x.0 == format!("proof-{s}"));
            if let (Some(i), Some(j)) = (at(f, a), at(f, b)) {
                let t = f[i].1.clone();
                f[i].1 = f[j].1.clone();
                f[j].1 = t;
            }
        }
    };
    let remove =
        |suffix: &'static str| move |f: &mut Files| f.retain(|x| x.0 != format!("proof-{suffix}"));
    const SQUARE: &str = "X44000000Y4000000D02*\nG01*\nX45000000D01*\nY5000000D01*\nX44000000D01*\nY4000000D01*\nM02*";
    let mut faults: Vec<Fault> = vec![
        // Paste.
        (
            "a paste island missing",
            "image",
            Box::new(drop_region("paste-top.gbr", 0)),
        ),
        (
            "the other paste island missing",
            "image",
            Box::new(drop_region("paste-top.gbr", 1)),
        ),
        (
            "a paste pad missing",
            "image",
            Box::new(drop_flash("paste-top.gbr")),
        ),
        (
            "a paste aperture 40 µm small",
            "image",
            Box::new(shrink_aperture("paste-bottom.gbr")),
        ),
        (
            "the paste layer missing",
            "files",
            Box::new(remove("paste-top.gbr")),
        ),
        // Mask.
        (
            "a mask opening missing",
            "image",
            Box::new(drop_flash("mask-bottom.gbr")),
        ),
        (
            "a mask opening region missing",
            "image",
            Box::new(drop_region("mask-top.gbr", 1)),
        ),
        (
            "a mask opening 40 µm small",
            "image",
            Box::new(shrink_aperture("mask-top.gbr")),
        ),
        (
            "the mask layer missing",
            "files",
            Box::new(remove("mask-bottom.gbr")),
        ),
        // Copper.
        (
            "a via flash missing",
            "objects",
            Box::new(drop_flash("copper-l1-top.gbr")),
        ),
        (
            "a conductor 40 µm thin",
            "image",
            Box::new(shrink_aperture("copper-l1-top.gbr")),
        ),
        (
            "a pour's hole filled",
            "image",
            Box::new(replace("copper-l1-top.gbr", "%LPC*%", "%LPD*%")),
        ),
        (
            "a pour missing",
            "image",
            Box::new(drop_region("copper-l2-bottom.gbr", 0)),
        ),
        (
            "a copper layer missing",
            "files",
            Box::new(remove("copper-l2-bottom.gbr")),
        ),
        (
            "a track missing",
            "objects",
            Box::new(|f: &mut Files| {
                edit(f, "copper-l1-top.gbr", |l| {
                    // The last track: the last move and draw before a flash.
                    let k = line_numbers(&l.join("\n"), |s| s.ends_with("D02*"));
                    let flash = line_numbers(&l.join("\n"), |s| s.ends_with("D03*"));
                    match k
                        .iter()
                        .rev()
                        .find(|k| flash.first().is_some_and(|f| *k < f))
                    {
                        Some(k) => without(l, *k..=*k + 1),
                        None => l,
                    }
                });
            }),
        ),
        (
            "a via moved 10 µm",
            "image",
            Box::new(|f: &mut Files| {
                edit(f, "copper-l1-top.gbr", |mut l| {
                    if let Some(k) = line_numbers(&l.join("\n"), |s| s.ends_with("D03*")).first() {
                        l[*k] = shifted(&l[*k], 'X', 0.01, false);
                    }
                    l
                });
            }),
        ),
        // Profile.
        (
            "an extra cutout",
            "profile",
            Box::new(replace("profile.gbr", "M02*", SQUARE)),
        ),
        (
            "the profile missing",
            "files",
            Box::new(remove("profile.gbr")),
        ),
        (
            "a cutout missing",
            "profile",
            Box::new(|f: &mut Files| {
                edit(f, "profile.gbr", |l| {
                    let k = line_numbers(&l.join("\n"), |s| s.ends_with("D02*"));
                    match (k.get(1), k.get(2)) {
                        (Some(a), Some(b)) => without(l, *a..=*b - 1),
                        _ => l,
                    }
                });
            }),
        ),
        (
            "an edge moved 2 µm",
            "profile",
            Box::new(replace("profile.gbr", "X60000000", "X60002000")),
        ),
        (
            "a concave end turned outwards",
            "profile",
            Box::new(replace("profile.gbr", "G02*", "G03*")),
        ),
        // Holes.
        (
            "a hole moved 5 µm",
            "holes",
            Box::new(on_hit("plated.drl", |s| shifted(s, 'X', 0.005, true))),
        ),
        (
            "a bare hole moved 5 µm",
            "holes",
            Box::new(on_hit("non-plated.drl", |s| shifted(s, 'Y', 0.005, true))),
        ),
        (
            "a hole missing",
            "holes",
            Box::new(on_hit("plated.drl", |_| String::new())),
        ),
        (
            "a hole drilled twice",
            "holes",
            Box::new(on_hit("plated.drl", |s| format!("{s}\n{s}"))),
        ),
        (
            "an extra hole",
            "holes",
            Box::new(on_hit("non-plated.drl", |s| format!("{s}\nX44.000Y4.000"))),
        ),
        (
            "a slot 0.1 mm too long",
            "holes",
            Box::new(on_slot(|s| match s.split_once("G85") {
                Some((a, b)) => format!("{a}G85{}", shifted(b, 'X', 0.1, true)),
                None => s.to_owned(),
            })),
        ),
        (
            "a slot drilled as a hole",
            "holes",
            Box::new(on_slot(|s| s.split("G85").next().unwrap_or(s).to_owned())),
        ),
        (
            "a slot missing",
            "holes",
            Box::new(on_slot(|_| String::new())),
        ),
        (
            "a tool 0.05 mm too large",
            "holes",
            Box::new(|f: &mut Files| {
                edit(f, "plated.drl", |l| {
                    l.into_iter()
                        .map(|s| s.replace("C0.300", "C0.350"))
                        .collect()
                });
            }),
        ),
        (
            "plated holes written as unplated",
            "holes",
            Box::new(swap("plated.drl", "non-plated.drl")),
        ),
        (
            "a plated slot milled on the profile, unplated",
            "profile",
            Box::new(|f: &mut Files| {
                on_slot_gone(f);
                edit(f, "profile.gbr", |l| {
                    l.join("\n")
                        .replacen("M02*", SQUARE, 1)
                        .lines()
                        .map(str::to_owned)
                        .collect()
                });
            }),
        ),
        (
            "the drill file missing",
            "files",
            Box::new(remove("plated.drl")),
        ),
        // Whole layers.
        (
            "mirrored copper",
            "image",
            Box::new(mirror("copper-l2-bottom.gbr")),
        ),
        ("mirrored mask", "image", Box::new(mirror("mask-top.gbr"))),
        (
            "mirrored paste",
            "image",
            Box::new(mirror("paste-bottom.gbr")),
        ),
        (
            "mirrored profile",
            "profile",
            Box::new(mirror("profile.gbr")),
        ),
        ("mirrored drills", "holes", Box::new(mirror("plated.drl"))),
        (
            "top and bottom copper swapped",
            "files",
            Box::new(swap("copper-l1-top.gbr", "copper-l2-bottom.gbr")),
        ),
        (
            "top and bottom mask swapped",
            "image",
            Box::new(swap("mask-top.gbr", "mask-bottom.gbr")),
        ),
        (
            "a file too many",
            "files",
            Box::new(|f: &mut Files| {
                if let Some(x) = f.first().cloned() {
                    f.push(("proof-copper-l3-inner1.gbr".into(), x.1));
                }
            }),
        ),
    ];
    // A plated slot of the board in the unplated file.
    faults.push((
        "one plated hole written as unplated",
        "holes",
        Box::new(move |f: &mut Files| {
            let mut moved = None;
            edit_take(f, "plated.drl", &mut moved);
            if let Some(line) = moved {
                edit(f, "non-plated.drl", move |mut l| {
                    if let Some(k) = line_numbers(&l.join("\n"), |s| s.starts_with('X')).first() {
                        l.insert(*k, line.clone());
                    }
                    l
                });
            }
        }),
    ));
    let mut missed = Vec::new();
    let mut unmade = Vec::new();
    for (what, by, seed) in &faults {
        let mut files = clean.clone();
        seed(&mut files);
        if files == clean {
            unmade.push(*what);
            continue;
        }
        // A file the reader refuses tests nothing of the check it names.
        let found = match check_changed(&project, &files, &clean) {
            Ok(found) => found,
            Err(e) => vec![format!("refused: {e}")],
        };
        if !found.iter().any(|f| f.starts_with(&format!("{by}:"))) {
            missed.push(format!("{what}: not by {by}; found {found:#?}"));
        }
    }
    assert!(unmade.is_empty(), "seeded nothing: {unmade:?}");
    assert!(missed.is_empty(), "{missed:#?}");
    eprintln!("{} faults seeded, each reported", faults.len());
    // A pour drawn under looser rules, held to the board's own: its image
    // is what was stored, and its spacing is short of every rule.
    let mut loose = project.clone();
    for o in &mut loose.rules.overrides {
        if let (RuleKey::Clearance { .. }, RuleValue::Length(mm)) = (o.key, &mut o.value) {
            *mm *= 0.8;
        }
    }
    refill(&mut loose)?;
    let found = short(&project, &written(&loose)?)?;
    for against in [
        Against::Track,
        Against::Pad,
        Against::ViaHole,
        Against::PlatedHole,
        Against::UnplatedHole,
    ] {
        assert!(
            found.iter().any(|s| s.against == against),
            "{against:?}: {found:#?}"
        );
    }
    Ok(())
}

fn on_slot_gone(f: &mut Files) {
    edit(f, "plated.drl", |l| {
        let k = line_numbers(&l.join("\n"), |s| s.contains("G85"));
        match k.first().copied() {
            Some(k) => without(l, k..=k),
            None => l,
        }
    });
}

/// Takes the first round hit of a drill file out of it.
fn edit_take(f: &mut Files, suffix: &str, taken: &mut Option<String>) {
    for (name, text) in f {
        if *name == format!("proof-{suffix}") {
            let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
            if let Some(k) = lines
                .iter()
                .position(|s| s.starts_with('X') && !s.contains("G85"))
            {
                *taken = Some(lines.remove(k));
            }
            *text = lines.join("\n") + "\n";
        }
    }
}

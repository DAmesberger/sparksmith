//! KiCad footprints and pads in Sparksmith's frame.
//!
//! **Axes.** KiCad's y grows downward and ours upward: a board point
//! becomes `(x − ox, −(y − oy))` mm about the origin `o` (the outline's
//! lower-left corner, else the auxiliary origin, else the file origin;
//! `Board::origin`), and a footprint- or pad-local vector
//! `(x, y)` becomes `(x, −y)`. Angles keep their value: both count
//! counter-clockwise as seen on screen.
//!
//! **Frames.** A pad's `at` is in its footprint's frame (before the
//! footprint's turn), and its angle is its orientation on the board, the
//! footprint's included. A back-side footprint's pads and primitives are
//! stored as seen from the top (already mirrored). So a pad's board pose
//! `W` is known without any mirroring, and the definition pad is
//! `P⁻¹ ∘ W`, where `P` is our placement (a reflection across x on the
//! bottom, then the footprint's turn): the reflection that lands in the
//! definition is baked into the pad-local geometry (custom outlines,
//! hole offsets), which is why a definition is computed per placement and
//! then shared when equal. These are hypotheses confirmed by `kicad-cli`
//! plots (`tests/oracle.rs`), not by documentation.
use crate::board::{Board, Footprint, KPad, PadShapeK, PadType, PointNm, Primitive};
use crate::report::{Case, Report};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use sparksmith_core::Layer;
use sparksmith_core::geometry::{Bounds, Point, Transform2};
use sparksmith_core::library::{
    ApertureId, CornerMask, Hole, HoleShape, MAX_OUTLINE_SEGMENTS, MaskRule, Outline, Pad, PadKind,
    PadShape, PadSide, PasteRule, PasteWindow, Segment, ShapeNm,
};
use sparksmith_core::units::{Angle, Nm};
use sparksmith_core::{PackageTerminalId, PadId};

/// Largest distance of a custom pad's chord from its round primitive, mm.
pub const CHORD_MM: f64 = 0.000_25;
/// Most segments one round primitive becomes.
const MAX_CIRCLE_SEGMENTS: usize = 256;

fn mm(nm: i64) -> f64 {
    nm as f64 / 1e6
}

/// Millimetres rounded to whole nanometres, so equal definitions computed
/// from different placements compare equal.
fn snap(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

fn snap_point(p: Point) -> Point {
    [snap(p[0]), snap(p[1])]
}

/// Millimetres in whole nanometres; beyond the coordinate bound the pad
/// is refused.
fn nm(v: f64) -> Result<Nm, Case> {
    Nm::from_mm_f64_rounded(v).map_err(|_| Case::PadOutOfRange)
}

/// A point in whole nanometres (see [`nm`]).
fn nm_point(p: Point) -> Result<sparksmith_core::units::PointNm, Case> {
    sparksmith_core::units::PointNm::from_mm_f64_rounded(p).map_err(|_| Case::PadOutOfRange)
}

/// A KiCad length (whole nanometres already) within the coordinate bound.
fn nm_exact(v: i64) -> Result<Nm, Case> {
    Nm::new(v).map_err(|_| Case::PadOutOfRange)
}

/// An angle in [0, 360) to a billionth of a degree.
fn snap_angle(deg: f64) -> f64 {
    let r = ((deg.rem_euclid(360.0)) * 1e9).round() / 1e9;
    if r >= 360.0 { 0.0 } else { r }
}

/// A KiCad board point in our board frame, mm (about [`Board::origin`]).
pub fn board_point(board: &Board, p: PointNm) -> Point {
    let o = board.origin;
    [mm(p[0] - o[0]), -mm(p[1] - o[1])]
}

/// A KiCad local vector in our orientation, mm.
fn local(p: PointNm) -> Point {
    [mm(p[0]), -mm(p[1])]
}

/// Our placement of a footprint.
pub fn placement(board: &Board, fp: &Footprint) -> (Point, f64, Layer) {
    (
        board_point(board, fp.at),
        snap_angle(fp.angle),
        if fp.back { Layer::Bottom } else { Layer::Top },
    )
}

fn transform(board: &Board, fp: &Footprint) -> Transform2 {
    let (translate, rotation_deg, layer) = placement(board, fp);
    Transform2 {
        translate,
        rotation_deg,
        mirror: layer == Layer::Bottom,
    }
}

/// A pad of a definition, without its id and terminal, and a short class
/// name for difference reports (no names or values from the file).
#[derive(Clone, Debug, PartialEq)]
pub struct DefPad {
    pub pad: Pad,
    pub class: String,
    /// The pad's box on the board, mm.
    pub world: Bounds,
    /// Round primitives of a custom pad on the board (centre, radius, mm),
    /// with the radii of their mask and paste openings: where KiCad's
    /// chords may differ from ours.
    pub arcs: Vec<(Point, f64)>,
}

/// How a pad maps: into a definition pad, or refused with its case.
pub enum Mapped {
    Pad(Box<DefPad>),
    /// A pad with paste and no copper: a paste window for the pad under it.
    PasteOnly(Box<DefPad>),
    Refused(Case),
}

/// The pad-local geometry map: KiCad's local `(x, y)` to the definition
/// pad's frame. With the placement's reflection baked in (a back-side
/// footprint) it is the identity; otherwise y flips.
#[derive(Clone, Copy)]
struct Local {
    flip: bool,
}

impl Local {
    fn point(self, p: Point) -> Point {
        // `p` is already in our orientation (y up); the definition frame
        // of a mirrored pad reflects it back.
        if self.flip { [p[0], -p[1]] } else { p }
    }
    /// A bulge drawn in our orientation, in the definition frame.
    fn bulge(self, b: f64) -> f64 {
        if self.flip { -b } else { b }
    }
}

/// Translate one pad of a placed footprint.
pub fn pad(board: &Board, fp: &Footprint, kpad: &KPad, report: &mut Report) -> Mapped {
    pad_or_refusal(board, fp, kpad, report).unwrap_or_else(Mapped::Refused)
}

/// [`pad`], with a refusal as the error.
fn pad_or_refusal(
    board: &Board,
    fp: &Footprint,
    kpad: &KPad,
    report: &mut Report,
) -> Result<Mapped, Case> {
    if !kpad.unknown.is_empty() {
        for path in &kpad.unknown {
            report.add_detail(Case::PadUnknownToken, &fp.reference, &kpad.number, path);
        }
        return Err(Case::PadUnknownToken);
    }
    let place = transform(board, fp);
    let (c, s) = sparksmith_core::geometry::cos_sin(place.rotation_deg);
    // The pad's `at` is its hole; a drill `offset` moves the copper shape
    // from the hole, in the pad's frame (oracle). Our pad origin is the
    // shape's centre, with the hole offset back from it.
    let p = local(kpad.at);
    let anchor = [
        place.translate[0] + c * p[0] - s * p[1],
        place.translate[1] + s * p[0] + c * p[1],
    ];
    let shift = local(kpad.drill.map(|d| d.offset).unwrap_or_default());
    let (pc, ps) = sparksmith_core::geometry::cos_sin(snap_angle(kpad.angle));
    let center = [
        anchor[0] + pc * shift[0] - ps * shift[1],
        anchor[1] + ps * shift[0] + pc * shift[1],
    ];
    let world = Transform2 {
        translate: center,
        rotation_deg: snap_angle(kpad.angle),
        mirror: false,
    };
    let def = world.then(&place.inverse());
    let map = Local { flip: def.mirror };
    let front = kpad.on("F.Cu");
    let back = kpad.on("B.Cu");
    let side = if fp.back { "B" } else { "F" };
    let size = [mm(kpad.size[0]), mm(kpad.size[1])];
    let (kind, pad_side) = match kpad.kind {
        PadType::NpThroughHole => (PadKind::NonPlatedHole, PadSide::Top),
        PadType::ThroughHole => (PadKind::PlatedThrough, PadSide::Top),
        PadType::Smd | PadType::Connect => {
            let (on_component, on_other) = if fp.back {
                (back, front)
            } else {
                (front, back)
            };
            let (paste_component, paste_other) = if fp.back {
                (kpad.on("B.Paste"), kpad.on("F.Paste"))
            } else {
                (kpad.on("F.Paste"), kpad.on("B.Paste"))
            };
            let pad_side = match (on_component, on_other) {
                (true, true) => return Err(Case::PadOnBothSides),
                (false, false) => match (paste_component, paste_other) {
                    (true, false) => PadSide::Top,
                    (false, true) => PadSide::Bottom,
                    _ => return Err(Case::PadWithoutCopper),
                },
                (true, false) => PadSide::Top,
                (false, true) => PadSide::Bottom,
            };
            let kind = if kpad.fiducial {
                PadKind::Fiducial
            } else if kpad.number.trim().is_empty() {
                PadKind::Mechanical
            } else {
                PadKind::Smd
            };
            (kind, pad_side)
        }
    };
    // The copper side this pad's mask and paste are on, as KiCad names it.
    let copper_side = match (pad_side, side) {
        (PadSide::Top, s) => s,
        (PadSide::Bottom, "F") => "B",
        (PadSide::Bottom, _) => "F",
    };
    let shape = pad_shape(kpad, size, map, report, fp)?;
    let hole = match kpad.drill {
        Some(d) => {
            let back = local(d.offset);
            let offset = map.point([-back[0], -back[1]]);
            let (w, h) = (nm_exact(d.size[0])?, nm_exact(d.size[1])?);
            let shape = if d.oval && w != h {
                HoleShape::Slot {
                    width_nm: w.min(h),
                    length_nm: w.max(h),
                    angle_mdeg: if w >= h {
                        Angle::ZERO
                    } else {
                        Angle::normalized(90_000)
                    },
                }
            } else {
                HoleShape::Round { diameter_nm: w }
            };
            Some(Hole {
                shape,
                plated: kpad.kind == PadType::ThroughHole,
                offset: nm_point(offset)?,
            })
        }
        None => None,
    };
    let mask = mask_rule(board, fp, kpad, copper_side, report)?;
    let paste = paste_rule(board, fp, kpad, copper_side, size, &shape, report)?;
    if matches!(&shape, PadShape::Custom { .. })
        && (matches!(mask, MaskRule::Expansion(e) if e != Nm::ZERO)
            || matches!(paste, PasteRule::Reduction(r) if r != Nm::ZERO))
    {
        report.add(Case::CustomPadOffset);
    }
    let mut pad = Pad {
        id: PadId(0),
        kind,
        terminal: None,
        number: kpad.number.clone(),
        position: nm_point(def.translate)?,
        rotation_mdeg: Angle::from_degrees_f64_rounded(def.rotation_deg)
            .map_err(|_| Case::PadOutOfRange)?,
        side: pad_side,
        shape,
        hole,
        mask,
        paste,
        vias: Vec::new(),
    };
    // A non-plated hole needs a land at least as large as the hole.
    if let (PadKind::NonPlatedHole, Some(h)) = (pad.kind, &pad.hole)
        && let HoleShape::Round { diameter_nm } = h.shape
        && let PadShape::Circle { diameter_nm: land } = &mut pad.shape
        && *land < diameter_nm
    {
        *land = diameter_nm;
    }
    if pad.fold_rotation().is_err() {
        return Err(Case::CustomPadNotOneOutline);
    }
    let class = class_of(kpad, fp);
    let ring = pad.shape.flatten(0.001);
    let t = Transform2 {
        translate: pad.position(),
        rotation_deg: pad.rotation_deg(),
        mirror: false,
    }
    .then(&place);
    let world_box = Bounds::of(ring.iter().map(|p| t.apply(*p))).unwrap_or(Bounds::point(center));
    let grow = match pad.mask {
        MaskRule::Expansion(e) => e.to_mm_f64(),
        _ => 0.0,
    };
    let shrink = match pad.paste {
        PasteRule::Reduction(r) => r.to_mm_f64(),
        _ => 0.0,
    };
    let mut arcs = Vec::new();
    if let PadShape::Custom { outline } = &pad.shape {
        // A custom pad's grown or shrunk opening is rounded at its
        // vertices (a true offset); KiCad draws those joins as chords.
        for v in std::iter::once(outline.start).chain(outline.segments.iter().map(Segment::to)) {
            let at = t.apply(v);
            for reach in [grow.abs(), shrink.abs()] {
                if reach > 0.0 {
                    arcs.push((at, reach));
                }
            }
        }
    }
    if kpad.chamfer_ratio > 0.0 && kpad.shape == PadShapeK::RoundRect && kpad.rratio > 0.0 {
        // A chamfered round rect's rounded corners: KiCad draws them as
        // chords.
        let short = size[0].min(size[1]);
        let r = kpad.rratio.clamp(0.0, 0.5) * short;
        let (x, y) = (size[0] / 2.0 - r, size[1] / 2.0 - r);
        for c in [[-x, -y], [x, -y], [x, y], [-x, y]] {
            let at = world.apply(c);
            for radius in [r, r + grow, r - shrink] {
                arcs.push((at, radius));
            }
        }
    }
    if kpad.shape == PadShapeK::Custom {
        // Round primitives, and the round joins and ends of stroked ones.
        let mut round: Vec<(Point, f64)> = Vec::new();
        for primitive in &kpad.primitives {
            match primitive {
                Primitive::Circle {
                    center,
                    radius,
                    width,
                    ..
                } => {
                    round.push((local(*center), radius / 1e6 + mm(*width) / 2.0));
                }
                Primitive::Poly { points, width, .. } if *width > 0 => {
                    round.extend(points.iter().map(|p| (local(*p), mm(*width) / 2.0)));
                }
                Primitive::Rect {
                    start, end, width, ..
                } if *width > 0 => {
                    let (a, b) = (local(*start), local(*end));
                    for c in [a, [b[0], a[1]], b, [a[0], b[1]]] {
                        round.push((c, mm(*width) / 2.0));
                    }
                }
                Primitive::Line { start, end, width } => {
                    round.push((local(*start), mm(*width) / 2.0));
                    round.push((local(*end), mm(*width) / 2.0));
                }
                _ => {}
            }
        }
        if kpad.anchor_circle {
            round.push(([0.0, 0.0], size[0] / 2.0));
        }
        for (c, r) in round {
            let at = world.apply(c);
            for radius in [r, r + grow, r - shrink] {
                arcs.push((at, radius));
            }
        }
    }
    let def = Box::new(DefPad {
        pad,
        class,
        world: world_box,
        arcs,
    });
    Ok(if kpad.kind == PadType::Smd && !front && !back {
        Mapped::PasteOnly(def)
    } else {
        Mapped::Pad(def)
    })
}

/// A class name: type, shape, side and turn, never a name or value.
fn class_of(kpad: &KPad, fp: &Footprint) -> String {
    let kind = match kpad.kind {
        PadType::Smd => "smd",
        PadType::ThroughHole => "pth",
        PadType::NpThroughHole => "npth",
        PadType::Connect => "connect",
    };
    let shape = match kpad.shape {
        PadShapeK::Rect => "rect",
        PadShapeK::RoundRect => "roundrect",
        PadShapeK::Circle => "circle",
        PadShapeK::Oval => "oval",
        PadShapeK::Trapezoid => "trapezoid",
        PadShapeK::Custom => "custom",
    };
    let turn = kpad.angle.rem_euclid(360.0);
    let turn = if (turn / 90.0 - (turn / 90.0).round()).abs() < 1e-9 {
        format!("{}", (turn / 90.0).round() as i64 * 90)
    } else {
        "free".to_owned()
    };
    let chamfer = if kpad.chamfer_ratio > 0.0 && kpad.shape == PadShapeK::RoundRect {
        " chamfer"
    } else {
        ""
    };
    format!(
        "{kind} {shape}{chamfer} {} rot{turn}{}",
        if fp.back { "back" } else { "front" },
        if kpad.drill.is_some_and(|d| d.offset != [0, 0]) {
            " offset-drill"
        } else {
            ""
        }
    )
}

/// The mask margin a pad takes: its own, else its footprint's, else the
/// board's (a stored 0 counts as not set).
fn mask_margin(board: &Board, fp: &Footprint, kpad: &KPad) -> i64 {
    kpad.margins
        .mask
        .filter(|v| *v != 0)
        .or(fp.margins.mask.filter(|v| *v != 0))
        .unwrap_or(board.setup.mask_margin)
}

fn mask_rule(
    board: &Board,
    fp: &Footprint,
    kpad: &KPad,
    copper_side: &str,
    report: &mut Report,
) -> Result<MaskRule, Case> {
    let margin = nm_exact(mask_margin(board, fp, kpad))?;
    Ok(match kpad.kind {
        PadType::ThroughHole | PadType::NpThroughHole => {
            match (kpad.on("F.Mask"), kpad.on("B.Mask")) {
                (false, false) => MaskRule::Tented,
                (true, true) => MaskRule::Expansion(margin),
                _ => {
                    report.add_detail(Case::OneSidedThroughMask, &fp.reference, &kpad.number, "");
                    MaskRule::Expansion(margin)
                }
            }
        }
        PadType::Smd | PadType::Connect => {
            if kpad.on(&format!("{copper_side}.Mask")) {
                MaskRule::Expansion(margin)
            } else {
                MaskRule::Tented
            }
        }
    })
}

fn paste_rule(
    board: &Board,
    fp: &Footprint,
    kpad: &KPad,
    copper_side: &str,
    size: Point,
    shape: &PadShape,
    report: &mut Report,
) -> Result<PasteRule, Case> {
    if !kpad.on(&format!("{copper_side}.Paste")) {
        return Ok(PasteRule::None);
    }
    let margin = kpad
        .margins
        .paste
        .filter(|v| *v != 0)
        .or(fp.margins.paste.filter(|v| *v != 0))
        .unwrap_or(board.setup.paste_margin);
    let ratio = kpad
        .margins
        .paste_ratio
        .filter(|v| *v != 0.0)
        .or(fp.margins.paste_ratio.filter(|v| *v != 0.0))
        .unwrap_or(board.setup.paste_ratio);
    let short = size[0].min(size[1]);
    if ratio != 0.0 && (size[0] - size[1]).abs() > 1e-9 {
        // KiCad takes a relative margin per axis (oracle): on a pad that is
        // not square the opening is no offset of the land, so it is a paste
        // window of the land's shape at the opening's size.
        let grow = [mm(margin) + ratio * size[0], mm(margin) + ratio * size[1]];
        if let Some(window) = ratio_window(shape, size, grow)? {
            report.add(Case::PasteRatioWindow);
            return Ok(PasteRule::Windows(vec![window]));
        }
        report.add_detail(Case::PasteRatio, &fp.reference, &kpad.number, "");
    }
    Ok(PasteRule::Reduction(nm(-(mm(margin) + ratio * short))?))
}

/// The paste window of a box-like land (`size`, centred on the pad
/// origin) grown by `grow` per axis: a rectangle, a round rect whose
/// radius KiCad rebuilds from the pad's ratio on the new size, or a
/// stadium. `None` for other shapes; an opening that vanishes is refused.
fn ratio_window(shape: &PadShape, size: Point, grow: Point) -> Result<Option<PasteWindow>, Case> {
    let (w, h) = (size[0] + 2.0 * grow[0], size[1] + 2.0 * grow[1]);
    if !(w > 0.0 && h > 0.0) {
        return Err(Case::PadOutOfRange);
    }
    let radius = match shape {
        PadShape::Rect { .. } => 0.0,
        PadShape::RoundRect {
            radius_ratio,
            corners,
            max_radius_nm: None,
            ..
        } if *corners == CornerMask::ALL => radius_ratio * w.min(h),
        PadShape::Oblong { .. } => w.min(h) / 2.0,
        _ => return Ok(None),
    };
    let (x, y) = (w / 2.0, h / 2.0);
    Ok(Some(PasteWindow {
        id: ApertureId(1),
        shape: ShapeNm::Rect {
            a: nm_point([-x, -y])?,
            b: nm_point([x, y])?,
            radius_nm: nm(radius)?,
        },
    }))
}

/// The copper outline in the definition pad frame.
fn pad_shape(
    kpad: &KPad,
    size: Point,
    map: Local,
    report: &mut Report,
    fp: &Footprint,
) -> Result<PadShape, Case> {
    let chamfered = kpad.chamfer_ratio > 0.0
        && (kpad.chamfer.top_left
            || kpad.chamfer.top_right
            || kpad.chamfer.bottom_left
            || kpad.chamfer.bottom_right);
    let exact =
        sparksmith_core::units::PointNm::new(nm_exact(kpad.size[0])?, nm_exact(kpad.size[1])?);
    Ok(match kpad.shape {
        // KiCad writes chamfers on round-rect pads (a ratio of 0 for
        // sharp corners).
        PadShapeK::RoundRect if chamfered => PadShape::Custom {
            outline: chamfered_outline(kpad, size, map),
        },
        PadShapeK::Rect => PadShape::Rect { size: exact },
        PadShapeK::RoundRect => PadShape::RoundRect {
            size: exact,
            radius_ratio: kpad.rratio.clamp(0.0, 0.5),
            max_radius_nm: None,
            corners: CornerMask::ALL,
        },
        PadShapeK::Circle => PadShape::Circle {
            diameter_nm: exact.x_nm,
        },
        PadShapeK::Oval if kpad.size[0] == kpad.size[1] => PadShape::Circle {
            diameter_nm: exact.x_nm,
        },
        PadShapeK::Oval => PadShape::Oblong { size: exact },
        PadShapeK::Trapezoid => {
            report.add_detail(Case::PadShapeUnsupported, &fp.reference, &kpad.number, "");
            return Err(Case::PadShapeUnsupported);
        }
        PadShapeK::Custom => {
            if kpad.unsupported_primitives > 0 {
                report.add_detail(Case::CustomPadPrimitive, &fp.reference, &kpad.number, "");
                return Err(Case::CustomPadPrimitive);
            }
            let (ring, chords) = custom_ring(kpad, size)?;
            if chords {
                report.add(Case::CustomPadChords);
            }
            let ring: Vec<Point> = ring.iter().map(|p| map.point(*p)).collect();
            if ring.len() > MAX_OUTLINE_SEGMENTS {
                report.add_detail(
                    Case::CustomPadNotOneOutline,
                    &fp.reference,
                    &kpad.number,
                    "",
                );
                return Err(Case::CustomPadNotOneOutline);
            }
            PadShape::Custom {
                outline: polygon_outline(&ring),
            }
        }
    })
}

/// A closed outline of straight segments, counter-clockwise.
fn polygon_outline(ring: &[Point]) -> Outline {
    let mut ring: Vec<Point> = ring.iter().map(|p| snap_point(*p)).collect();
    let area: f64 = (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if area < 0.0 {
        ring.reverse();
    }
    let start = ring.first().copied().unwrap_or_default();
    let mut segments: Vec<Segment> = ring
        .iter()
        .skip(1)
        .map(|p| Segment::Line { to: *p })
        .collect();
    segments.push(Segment::Line { to: start });
    Outline { start, segments }
}

/// A rectangle with KiCad's chamfered corners cut and its other corners
/// rounded by the round-rect ratio (in our orientation, then mapped).
fn chamfered_outline(kpad: &KPad, size: Point, map: Local) -> Outline {
    let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
    let short = size[0].min(size[1]);
    let c = kpad.chamfer_ratio.clamp(0.0, 0.5) * short;
    let r = if kpad.shape == PadShapeK::RoundRect {
        kpad.rratio.clamp(0.0, 0.5) * short
    } else {
        0.0
    };
    // Corners counter-clockwise in our orientation (y up): KiCad's "top"
    // is −y in its frame, +y in ours.
    let corners = [
        ([-hx, -hy], kpad.chamfer.bottom_left, [1.0, 0.0], [0.0, 1.0]),
        (
            [hx, -hy],
            kpad.chamfer.bottom_right,
            [0.0, 1.0],
            [-1.0, 0.0],
        ),
        ([hx, hy], kpad.chamfer.top_right, [-1.0, 0.0], [0.0, -1.0]),
        ([-hx, hy], kpad.chamfer.top_left, [0.0, -1.0], [1.0, 0.0]),
    ];
    // Each corner: from a point on the incoming edge to one on the
    // outgoing edge, cut straight (chamfer) or by a quarter arc.
    let quarter = (std::f64::consts::PI / 8.0).tan();
    let mut points: Vec<(Point, f64)> = Vec::new();
    for (corner, cut, out_dir, in_dir) in corners {
        let d = if cut { c } else { r };
        if d <= 0.0 {
            points.push((corner, 0.0));
            continue;
        }
        let enter = [corner[0] + in_dir[0] * d, corner[1] + in_dir[1] * d];
        let leave = [corner[0] + out_dir[0] * d, corner[1] + out_dir[1] * d];
        points.push((enter, if cut { 0.0 } else { quarter }));
        points.push((leave, 0.0));
    }
    let mapped: Vec<(Point, f64)> = points
        .iter()
        .map(|(p, b)| (snap_point(map.point(*p)), map.bulge(*b)))
        .collect();
    let (start, _) = mapped.first().copied().unwrap_or_default();
    let mut segments = Vec::new();
    for k in 0..mapped.len() {
        let (_, bulge) = mapped[k];
        let to = mapped[(k + 1) % mapped.len()].0;
        segments.push(if bulge != 0.0 {
            Segment::Arc { to, bulge }
        } else {
            Segment::Line { to }
        });
    }
    let mut outline = Outline { start, segments };
    if map.flip {
        // A reflection turns the ring clockwise: reverse it.
        outline = reversed(&outline);
    }
    outline
}

/// The same outline run the other way.
fn reversed(outline: &Outline) -> Outline {
    let mut points = vec![outline.start];
    points.extend(outline.segments.iter().map(Segment::to));
    let n = outline.segments.len();
    let mut segments = Vec::with_capacity(n);
    for k in (0..n).rev() {
        let to = points[k];
        segments.push(match outline.segments[k] {
            Segment::Line { .. } => Segment::Line { to },
            Segment::Arc { bulge, .. } => Segment::Arc { to, bulge: -bulge },
        });
    }
    Outline {
        start: points[n],
        segments,
    }
}

/// A circle as a ring within [`CHORD_MM`] (vertices on the circle).
fn circle(c: Point, r: f64) -> Vec<Point> {
    let step = if r > CHORD_MM {
        2.0 * (1.0 - CHORD_MM / r).acos()
    } else {
        std::f64::consts::FRAC_PI_2
    };
    let n = ((std::f64::consts::TAU / step).ceil() as usize).clamp(8, MAX_CIRCLE_SEGMENTS);
    (0..n)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

type Rings = Vec<Vec<Point>>;

/// `rings` (one region: an outer ring and its holes) grown by `by` mm with
/// round joins whose chords stay within [`CHORD_MM`].
fn grown(mut rings: Rings, by: f64) -> Vec<Rings> {
    if by <= 0.0 {
        return vec![rings];
    }
    // The offset reads a counter-clockwise ring as an outer boundary: a
    // ring flipped from KiCad's y-down frame runs clockwise.
    if let Some(outer) = rings.first_mut() {
        let area: f64 = (0..outer.len())
            .map(|i| {
                let (a, b) = (outer[i], outer[(i + 1) % outer.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        if area < 0.0 {
            outer.reverse();
        }
    }
    let step = 2.0 * (1.0 - (CHORD_MM / by).min(1.0)).acos().max(0.01);
    let style = OutlineStyle::new(by).line_join(LineJoin::Round(step));
    vec![rings].outline(&style)
}

/// Every point within `r` of the segment `a`–`b`.
fn capsule(a: Point, b: Point, r: f64) -> Vec<Point> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0].hypot(d[1]);
    if len < 1e-12 {
        return circle(a, r);
    }
    let heading = d[1].atan2(d[0]);
    let half = circle([0.0, 0.0], r).len() / 2;
    let cap = |c: Point, from: f64| {
        (0..=half).map(move |k| {
            let t = from + std::f64::consts::PI * k as f64 / half as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
    };
    let quarter = std::f64::consts::FRAC_PI_2;
    cap(b, heading - quarter)
        .chain(cap(a, heading + quarter))
        .collect()
}

/// A custom pad's anchor and primitives united into one ring (our
/// orientation, pad frame), and whether a round primitive became chords.
fn custom_ring(kpad: &KPad, size: Point) -> Result<(Vec<Point>, bool), Case> {
    let mut parts: Vec<Rings> = Vec::new();
    let mut chords = false;
    if kpad.anchor_circle {
        parts.push(vec![circle([0.0, 0.0], size[0] / 2.0)]);
        chords = true;
    } else {
        let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
        parts.push(vec![vec![[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]]]);
    }
    let (union, curved) = primitive_union(parts, &kpad.primitives);
    chords |= curved;
    match union.as_slice() {
        [only] if only.len() == 1 => Ok((only[0].clone(), chords)),
        _ => Err(Case::CustomPadNotOneOutline),
    }
}

fn primitive_union(mut parts: Vec<Rings>, primitives: &[Primitive]) -> (Vec<Rings>, bool) {
    let mut chords = false;
    for primitive in primitives {
        match primitive {
            Primitive::Arc {
                start,
                mid,
                end,
                width,
            } => {
                let (a, m, b) = (local(*start), local(*mid), local(*end));
                let Some((centre, radius)) = crate::shape::circle_through(a, m, b) else {
                    return (Vec::new(), true);
                };
                let half = mm(*width) / 2.0;
                if half <= 0.0 {
                    return (Vec::new(), true);
                }
                let angle = |p: Point| (p[1] - centre[1]).atan2(p[0] - centre[0]).to_degrees();
                let from = angle(a);
                let to = (angle(b) - from).rem_euclid(360.);
                let sweep = if (angle(m) - from).rem_euclid(360.) <= to {
                    to
                } else {
                    to - 360.
                };
                // The copper is an annular sector and two round endcaps.
                let arc = |radius| {
                    sparksmith_core::geometry::Shape::Arc {
                        center: centre,
                        radius,
                        start_deg: from,
                        sweep_deg: sweep,
                    }
                    .flatten(CHORD_MM)
                };
                let mut sector = arc(radius + half);
                if radius > half {
                    sector.extend(arc(radius - half).into_iter().rev());
                } else {
                    sector.push(centre);
                }
                parts.extend([vec![sector], vec![circle(a, half)], vec![circle(b, half)]]);
                chords = true;
            }
            Primitive::Poly {
                points,
                width,
                fill,
            } => {
                let ring: Vec<Point> = points.iter().map(|p| local(*p)).collect();
                if *width > 0 {
                    chords = true;
                }
                if *fill {
                    parts.extend(grown(vec![ring], mm(*width) / 2.0));
                } else if *width > 0 {
                    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                        parts.push(vec![capsule(*a, *b, mm(*width) / 2.)]);
                    }
                }
            }
            Primitive::Circle {
                center,
                radius,
                width,
                fill,
            } => {
                chords = true;
                let c = local(*center);
                let r = radius / 1e6;
                let w = mm(*width) / 2.0;
                if *fill {
                    parts.push(vec![circle(c, r + w)]);
                } else {
                    let mut inner = circle(c, (r - w).max(0.0));
                    inner.reverse();
                    parts.push(vec![circle(c, r + w), inner]);
                }
            }
            Primitive::Rect {
                start,
                end,
                width,
                fill,
            } => {
                let (a, b) = (local(*start), local(*end));
                let ring = vec![a, [b[0], a[1]], b, [a[0], b[1]]];
                let w = mm(*width) / 2.0;
                if w > 0.0 {
                    chords = true;
                }
                if *fill {
                    parts.extend(grown(vec![ring], w));
                } else {
                    for side in 0..4 {
                        parts.push(vec![capsule(ring[side], ring[(side + 1) % 4], w)]);
                    }
                }
            }
            Primitive::Line { start, end, width } => {
                chords = true;
                parts.push(vec![capsule(local(*start), local(*end), mm(*width) / 2.0)]);
            }
        }
    }
    // Each part counter-clockwise, united under the non-zero rule.
    let mut subject: Rings = Vec::new();
    for part in parts {
        let cleaned: Vec<Rings> = part.overlay(
            &Vec::<Vec<Point>>::new(),
            OverlayRule::Subject,
            FillRule::EvenOdd,
        );
        subject.extend(cleaned.into_iter().flatten());
    }
    let union: Vec<Rings> = subject.overlay(
        &Vec::<Vec<Point>>::new(),
        OverlayRule::Subject,
        FillRule::NonZero,
    );
    (union, chords)
}

/// The package terminal a pad lands, for the pads that need one.
pub fn needs_terminal(pad: &Pad) -> bool {
    matches!(pad.kind, PadKind::Smd | PadKind::PlatedThrough)
}

/// Package terminal ids are 1-based.
pub fn terminal_id(k: usize) -> PackageTerminalId {
    PackageTerminalId(u32::try_from(k + 1).unwrap_or(u32::MAX))
}

/// Add a paste-only pad's opening as a paste window of the pad under its
/// centre on the same side (keeping that pad's own paste opening as a
/// window). Returns false when no pad lies under it.
pub fn add_paste_window(pads: &mut [Pad], window: &Pad) -> bool {
    let reduction = match window.paste {
        PasteRule::Reduction(r) => r.to_mm_f64(),
        _ => 0.0,
    };
    let Some(opening) = window.shape.grown(-reduction, CHORD_MM) else {
        return false;
    };
    let frame = |p: &Pad| Transform2 {
        translate: p.position(),
        rotation_deg: p.rotation_deg(),
        mirror: false,
    };
    let Some(target) = pads.iter_mut().find(|p| {
        p.side == window.side
            && p.kind != PadKind::NonPlatedHole
            && sparksmith_core::geometry::polygon_contains(
                &p.shape.flatten(0.001),
                frame(p).inverse().apply(window.position()),
            )
    }) else {
        return false;
    };
    let into_target = frame(window).then(&frame(target).inverse());
    let mut windows = match &target.paste {
        PasteRule::Windows(w) => w.clone(),
        PasteRule::Reduction(r) => target
            .shape
            .grown(-r.to_mm_f64(), CHORD_MM)
            .and_then(|s| ShapeNm::from_mm_rounded(&s).ok())
            .map(|shape| PasteWindow {
                id: ApertureId(1),
                shape,
            })
            .into_iter()
            .collect(),
        PasteRule::None | PasteRule::Inherit => Vec::new(),
    };
    let Ok(shape) = ShapeNm::from_mm_rounded(&opening.transformed(&into_target)) else {
        return false;
    };
    let next = windows.iter().map(|w| w.id.0).max().unwrap_or(0) + 1;
    windows.push(PasteWindow {
        id: ApertureId(next),
        shape,
    });
    target.paste = PasteRule::Windows(windows);
    true
}

/// Most growth (mm, either way) a footprint polygon may give its pad's
/// land and still count as that pad's opening.
const MAX_OPENING_GROWTH_MM: f64 = 1.0;
/// How much of an opening's area may differ from its pad's land grown
/// evenly: KiCad's drawn polygons keep sharp corners where an even growth
/// rounds them (a few thousandths of a square millimetre per corner).
const OPENING_AREA_SLACK: f64 = 0.02;

/// The footprint's mask and paste polygons ([`crate::board::Opening`])
/// that are one of its pads' land grown evenly, taken as that pad's mask
/// expansion or paste rule (where the pad has no opening of its own on
/// that layer, or the same one). Returns how many were taken; the others
/// stay refused drawings.
pub fn take_openings(board: &Board, fp: &Footprint, pads: &mut [Pad]) -> usize {
    let place = transform(board, fp);
    let (c, s) = sparksmith_core::geometry::cos_sin(place.rotation_deg);
    let on_board = |p: PointNm| {
        let q = local(p);
        [
            place.translate[0] + c * q[0] - s * q[1],
            place.translate[1] + s * q[0] + c * q[1],
        ]
    };
    let component = if fp.back { "B" } else { "F" };
    let mut taken = 0;
    for opening in &fp.openings {
        let Some((side, what)) = opening.layer.split_once('.') else {
            continue;
        };
        let ring: Vec<Point> = opening.points.iter().map(|p| on_board(*p)).collect();
        let open_area = sparksmith_core::geometry::signed_area2(&ring).abs() / 2.0;
        let matches: Vec<(usize, f64)> = pads
            .iter()
            .enumerate()
            .filter(|(_, pad)| {
                let own = match pad.side {
                    PadSide::Top => component,
                    PadSide::Bottom if component == "F" => "B",
                    PadSide::Bottom => "F",
                };
                match pad.kind {
                    PadKind::NonPlatedHole => false,
                    PadKind::PlatedThrough => true,
                    _ => own == side,
                }
            })
            .filter_map(|(i, pad)| Some((i, even_growth(pad, &place, &ring, open_area)?)))
            .collect();
        let [(index, growth)] = matches.as_slice() else {
            continue;
        };
        let (Some(pad), Ok(by), Ok(reduction)) =
            (pads.get_mut(*index), nm(snap(*growth)), nm(-snap(*growth)))
        else {
            continue;
        };
        let took = match what {
            "Mask" => match pad.mask {
                MaskRule::Tented => {
                    pad.mask = MaskRule::Expansion(by);
                    true
                }
                MaskRule::Expansion(e) => e == by,
                MaskRule::Inherit => false,
            },
            "Paste" => match &pad.paste {
                PasteRule::None => {
                    pad.paste = PasteRule::Reduction(reduction);
                    true
                }
                PasteRule::Reduction(r) => *r == reduction,
                _ => false,
            },
            _ => false,
        };
        if took {
            taken += 1;
        }
    }
    taken
}

/// How far `ring` (board mm) grows `pad`'s land evenly on every side, if
/// it does: the growth whose land has the ring's area, accepted when the
/// grown land and the ring differ by at most [`OPENING_AREA_SLACK`] of it.
fn even_growth(pad: &Pad, place: &Transform2, ring: &[Point], open_area: f64) -> Option<f64> {
    let to_board = pad.frame().then(place);
    let land: Vec<Point> = pad
        .shape
        .outline(CHORD_MM)
        .transformed(&to_board)
        .flatten(CHORD_MM);
    if land.len() < 3 || !boxes_overlap(&land, ring) {
        return None;
    }
    let area = sparksmith_core::geometry::signed_area2(&land).abs() / 2.0;
    let perimeter: f64 = (0..land.len())
        .map(|i| sparksmith_core::geometry::distance(land[i], land[(i + 1) % land.len()]))
        .sum();
    // An even growth d of a convex land with sharp corners (as drawn)
    // adds its perimeter times d and d² times the sum of tan(θ/2) over its
    // corners' turns θ (π for a round land, 4 for a box).
    let n = land.len();
    let turn = |i: usize| {
        let (a, b, c) = (land[(i + n - 1) % n], land[i], land[(i + 1) % n]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1])
    };
    let sense = sparksmith_core::geometry::signed_area2(&land).signum();
    let corners: f64 = (0..n)
        .map(|i| sense * turn(i))
        .filter(|t| *t > 0.0)
        .map(|t| (t / 2.0).tan())
        .sum();
    let k = corners.max(f64::EPSILON);
    let disc = perimeter * perimeter + 4.0 * k * (open_area - area);
    if disc < 0.0 {
        return None;
    }
    let growth = (disc.sqrt() - perimeter) / (2.0 * k);
    if !growth.is_finite() || growth.abs() > MAX_OPENING_GROWTH_MM {
        return None;
    }
    let grown: Vec<Point> = pad
        .shape
        .grown(growth, CHORD_MM)?
        .transformed(&to_board)
        .flatten(CHORD_MM);
    let xor: Vec<Rings> =
        vec![grown].overlay(&vec![ring.to_vec()], OverlayRule::Xor, FillRule::EvenOdd);
    // Holes counted as area too: that only makes the test stricter.
    let differ: f64 = xor
        .iter()
        .flatten()
        .map(|r| sparksmith_core::geometry::signed_area2(r).abs() / 2.0)
        .sum();
    (differ <= OPENING_AREA_SLACK * open_area).then_some(growth)
}

/// Whether two rings' boxes overlap.
fn boxes_overlap(a: &[Point], b: &[Point]) -> bool {
    let bounds = |r: &[Point]| {
        r.iter().fold(
            ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
            |(lo, hi), p| {
                (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                )
            },
        )
    };
    let ((alo, ahi), (blo, bhi)) = (bounds(a), bounds(b));
    alo[0] <= bhi[0] && blo[0] <= ahi[0] && alo[1] <= bhi[1] && blo[1] <= ahi[1]
}

/// The groups on a single source footprint and the copper regions they own.
/// Pad identities determine ownership through the component's pin mapping.
#[derive(Clone, Debug)]
pub(crate) struct ImportedTies {
    pub groups: Vec<sparksmith_core::library::tie::TieSpec>,
    pub bridges: Vec<sparksmith_core::library::tie::Bridge>,
}

/// Translate a marked footprint only if each group is connected by its own
/// original copper drawings. An invalid declaration keeps its pads for review.
pub(crate) fn copper_ties(fp: &Footprint, pads: &[Pad]) -> Result<ImportedTies, &'static str> {
    use sparksmith_core::ids::{BridgeId, TieId};
    use sparksmith_core::library::tie::{Bridge, TieForm, TieSpec};
    use std::collections::{BTreeMap, BTreeSet};
    if fp.unsupported_tie_copper {
        return Err("This tie has unsupported copper drawings.");
    }
    if fp.tie_copper.is_empty() {
        return Err("The footprint has no copper drawing joining its declared lands.");
    }
    let numbers: BTreeMap<_, _> = pads
        .iter()
        .filter_map(|p| Some((p.number.as_str(), p.terminal?)))
        .collect();
    // The older net_tie attribute has no groups and declares one tie.
    let declared: Vec<Vec<&str>> = if fp.tie_groups.is_empty() {
        vec![numbers.keys().copied().collect()]
    } else {
        fp.tie_groups
            .iter()
            .map(|g| g.iter().map(String::as_str).collect())
            .collect()
    };
    let mut group_of = BTreeMap::new();
    let mut groups = Vec::new();
    for names in declared {
        let mut ends = BTreeSet::new();
        for name in names {
            let terminal = numbers
                .get(name)
                .filter(|_| !name.is_empty())
                .ok_or("A declared tie group names a missing or unnumbered land.")?;
            if !ends.insert(sparksmith_core::TerminalId(terminal.0)) {
                return Err("A tie group repeats one of its pad numbers.");
            }
        }
        if !(2..=8).contains(&ends.len()) {
            return Err("Each tie group needs two to eight distinct terminals.");
        }
        // Deterministic even when the source reorders its quoted groups.
        let id = TieId(ends.first().ok_or("A tie group has no terminals.")?.0);
        for end in &ends {
            if group_of.insert(PackageTerminalId(end.0), id).is_some() {
                return Err("Two tie groups claim the same terminal.");
            }
        }
        groups.push(TieSpec {
            id,
            form: TieForm::Copper,
            ends: ends.into_iter().collect(),
        });
    }
    groups.sort_by_key(|g| g.id);
    let mut group_lands = BTreeMap::<TieId, Vec<Vec<PadId>>>::new();
    let mut bridges = Vec::new();
    for layer in ["F.Cu", "B.Cu"] {
        let shapes = fp
            .tie_copper
            .iter()
            .filter(|(l, _)| l == layer)
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>();
        if shapes.is_empty() {
            continue;
        }
        let side = if (layer == "F.Cu") != fp.back {
            PadSide::Top
        } else {
            PadSide::Bottom
        };
        let regions = primitive_union(vec![], &shapes).0;
        if regions.is_empty() {
            return Err("The footprint has an invalid or empty copper drawing on a tie layer.");
        }
        for region in regions {
            let [ring] = region.as_slice() else {
                return Err("A bridge drawing contains a hole that its outline cannot represent.");
            };
            if ring.len() > MAX_OUTLINE_SEGMENTS {
                return Err("A bridge drawing exceeds the supported outline size.");
            }
            let outline = polygon_outline(
                &ring
                    .iter()
                    .map(|p| Local { flip: fp.back }.point(*p))
                    .collect::<Vec<_>>(),
            );
            let mut lands = Vec::new();
            let mut owner = None;
            for pad in pads {
                if sparksmith_core::library::tie::bridge_touches_pad(&outline, side, pad) {
                    let group = pad
                        .terminal
                        .and_then(|end| group_of.get(&end).copied())
                        .ok_or("Bridge copper touches a land outside every declared group.")?;
                    if owner.replace(group).is_some_and(|old| old != group) {
                        return Err("Bridge copper touches lands from separate tie groups.");
                    }
                    lands.push(pad.id);
                }
            }
            if lands.len() < 2 {
                return Err("A bridge drawing does not reach two lands in its group.");
            }
            let group = owner.ok_or("A bridge drawing has no declared lands.")?;
            group_lands.entry(group).or_default().push(lands.clone());
            bridges.push(Bridge {
                id: BridgeId(
                    u32::try_from(pads.len() + bridges.len() + 1)
                        .map_err(|_| "Too many bridge drawings.")?,
                ),
                outline,
                side,
                pads: lands,
            });
        }
    }
    for group in &groups {
        // A group may contain several drawings that meet on a shared land,
        // including the two sides of a plated pad. A second isolated drawing
        // cannot stand in for copper between those drawings.
        let mut pieces = group_lands.remove(&group.id).unwrap_or_default();
        let mut joined: BTreeSet<_> = pieces
            .pop()
            .ok_or("A declared tie group has no bridge drawing.")?
            .into_iter()
            .collect();
        while let Some(i) = pieces
            .iter()
            .position(|p| p.iter().any(|id| joined.contains(id)))
        {
            joined.extend(pieces.swap_remove(i));
        }
        if !pieces.is_empty()
            || !group.ends.iter().all(|end| {
                pads.iter()
                    .any(|p| p.terminal == Some(PackageTerminalId(end.0)) && joined.contains(&p.id))
            })
        {
            return Err("The bridge drawings do not connect every end of a declared tie group.");
        }
    }
    Ok(ImportedTies { groups, bridges })
}

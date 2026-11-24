//! What must be made, decided point by point from the document's own
//! primitives: exact distances to lines and arcs, with no polygon library,
//! no flattened curve and none of the production geometry (`grown`,
//! `offset_pieces`, `stadium`, the placement transform, the fill
//! computation). A pad's land is its outline's lines and arcs in the pad
//! frame; growing it by `by` is every point within `by` of it, shrinking it
//! every point inside by at least that; a point of the board is carried
//! back into the pad frame by the placement written out below. The written
//! Gerber image is read the same way ([`Image`]): the last object covering
//! a point decides.
use super::Fallible;
use super::reader::{self, GerberFile, Kind, Path, Prim as MacroPrim, Template};
use crate::geometry::Point;
use crate::library::{
    FootprintDefinition, HoleShape, MaskRule, Outline, Pad, PadKind, PadShape, PadSide, PasteRule,
    Segment, ShapeNm,
};
use crate::model::board::CutoutKind;
use crate::model::{CopperLayer, Layer, Placement, Project};

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}

fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn len(a: Point) -> f64 {
    a[0].hypot(a[1])
}

pub(crate) fn turned(p: Point, deg: f64) -> Point {
    let (s, c) = deg.to_radians().sin_cos();
    [c * p[0] - s * p[1], s * p[0] + c * p[1]]
}

/// A line, or an arc of signed sweep (radians, counter-clockwise positive).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Edge {
    Line {
        a: Point,
        b: Point,
    },
    Arc {
        a: Point,
        b: Point,
        c: Point,
        sweep: f64,
    },
}

impl Edge {
    /// `bulge` is the tangent of a quarter of the sweep.
    fn of_bulge(a: Point, b: Point, bulge: f64) -> Self {
        let chord = sub(b, a);
        // The apex lies a sagitta right of the chord for a positive bulge;
        // the centre is on the line through it, a radius back.
        let half = len(chord) / 2.0;
        let sagitta = bulge.abs() * half;
        let radius = (half * half + sagitta * sagitta) / (2.0 * sagitta);
        let right = [chord[1] / (2.0 * half), -chord[0] / (2.0 * half)];
        let side = bulge.signum();
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let back = (sagitta - radius) * side;
        Self::Arc {
            a,
            b,
            c: [mid[0] + right[0] * back, mid[1] + right[1] * back],
            sweep: 4.0 * bulge.atan(),
        }
    }

    fn arc(a: Point, b: Point, c: Point, clockwise: bool) -> Self {
        let at = |p: Point| (p[1] - c[1]).atan2(p[0] - c[0]);
        let turn = if clockwise {
            at(a) - at(b)
        } else {
            at(b) - at(a)
        };
        let turn = turn.rem_euclid(std::f64::consts::TAU);
        let turn = if len(sub(a, b)) < 1e-9 {
            std::f64::consts::TAU
        } else {
            turn
        };
        Self::Arc {
            a,
            b,
            c,
            sweep: if clockwise { -turn } else { turn },
        }
    }

    fn ends(&self) -> (Point, Point) {
        match *self {
            Self::Line { a, b } | Self::Arc { a, b, .. } => (a, b),
        }
    }

    fn distance(&self, p: Point) -> f64 {
        match *self {
            Self::Line { a, b } => {
                let d = sub(b, a);
                let l2 = d[0] * d[0] + d[1] * d[1];
                let t = if l2 > 0.0 {
                    (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                len(sub(p, add(a, [d[0] * t, d[1] * t])))
            }
            Self::Arc { a, b, c, sweep } => {
                let at = |q: Point| (q[1] - c[1]).atan2(q[0] - c[0]);
                let turn = (if sweep < 0.0 {
                    at(a) - at(p)
                } else {
                    at(p) - at(a)
                })
                .rem_euclid(std::f64::consts::TAU);
                if turn <= sweep.abs() {
                    (len(sub(p, c)) - len(sub(a, c))).abs()
                } else {
                    len(sub(p, a)).min(len(sub(p, b)))
                }
            }
        }
    }

    /// Whether a ray to +x from `p` meets the edge's chord.
    fn chord_crossed(&self, p: Point) -> bool {
        let (a, b) = self.ends();
        (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
    }

    /// Whether `p` lies between an arc and its chord.
    fn in_segment(&self, p: Point) -> bool {
        let Self::Arc { a, b, c, sweep } = *self else {
            return false;
        };
        if len(sub(p, c)) >= len(sub(a, c)) {
            return false;
        }
        if len(sub(a, b)) < 1e-9 {
            return true;
        }
        cross(sub(b, a), sub(p, a)) * sweep.signum() < 0.0
    }

    fn length(&self) -> f64 {
        match *self {
            Self::Line { a, b } => len(sub(b, a)),
            Self::Arc { a, c, sweep, .. } => len(sub(a, c)) * sweep.abs(),
        }
    }

    /// The point a share `t` along, and the direction to its right.
    fn along(&self, t: f64) -> (Point, Point) {
        match *self {
            Self::Line { a, b } => {
                let d = sub(b, a);
                let l = len(d).max(1e-12);
                (add(a, [d[0] * t, d[1] * t]), [d[1] / l, -d[0] / l])
            }
            Self::Arc { a, c, sweep, .. } => {
                let r = len(sub(a, c));
                let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
                let (s, k) = (a0 + sweep * t).sin_cos();
                let out = sweep.signum();
                ([c[0] + r * k, c[1] + r * s], [out * k, out * s])
            }
        }
    }

    /// Points along the edge with the direction to its right.
    fn samples(&self, step: f64) -> Vec<(Point, Point)> {
        let n = (self.length() / step).ceil().clamp(1.0, 400.0) as usize;
        (0..=n).map(|k| self.along(k as f64 / n as f64)).collect()
    }
}

/// An area bounded by closed loops of edges (even-odd).
#[derive(Clone, Debug, Default)]
pub(crate) struct Area {
    pub edges: Vec<Edge>,
    /// Of an area of many lines, the lines near each row.
    rows: Option<Rows>,
}

/// Height of a row, mm: distances are told exactly up to it.
const ROW_MM: f64 = 0.25;

#[derive(Clone, Debug)]
struct Rows {
    first: i64,
    rows: Vec<Vec<Edge>>,
}

impl Area {
    fn new(edges: Vec<Edge>) -> Self {
        Self { edges, rows: None }
    }

    /// With its lines filed by row, when it has many and no arc: distances
    /// past [`ROW_MM`] then read as that much.
    fn filed(mut self) -> Self {
        if self.edges.len() < 64 || self.edges.iter().any(|e| matches!(e, Edge::Arc { .. })) {
            return self;
        }
        let row = |y: f64| (y / ROW_MM).floor() as i64;
        let span = |e: &Edge| {
            let (a, b) = e.ends();
            (
                row(a[1].min(b[1]) - ROW_MM - 1e-6),
                row(a[1].max(b[1]) + ROW_MM + 1e-6),
            )
        };
        let first = self.edges.iter().map(|e| span(e).0).min().unwrap_or(0);
        let last = self.edges.iter().map(|e| span(e).1).max().unwrap_or(0);
        let mut rows = vec![Vec::new(); (last - first + 1) as usize];
        for e in &self.edges {
            let (a, b) = span(e);
            for r in a..=b {
                rows[(r - first) as usize].push(*e);
            }
        }
        self.rows = Some(Rows { first, rows });
        self
    }

    /// The edges that tell about `p`.
    fn near(&self, p: Point) -> &[Edge] {
        match &self.rows {
            None => &self.edges,
            Some(r) => {
                let k = (p[1] / ROW_MM).floor() as i64 - r.first;
                usize::try_from(k)
                    .ok()
                    .and_then(|k| r.rows.get(k))
                    .map_or(&[], Vec::as_slice)
            }
        }
    }

    pub(crate) fn of_outline(outline: &Outline) -> Self {
        let mut edges = Vec::new();
        let mut a = outline.start;
        for s in &outline.segments {
            let b = s.to();
            match *s {
                _ if len(sub(a, b)) < 1e-12 => {}
                Segment::Arc { bulge, .. } if bulge.abs() > 1e-12 => {
                    edges.push(Edge::of_bulge(a, b, bulge));
                }
                _ => edges.push(Edge::Line { a, b }),
            }
            a = b;
        }
        Self::new(edges)
    }

    pub(crate) fn of_ring(ring: &[Point]) -> Self {
        Self::new(
            (0..ring.len())
                .map(|k| Edge::Line {
                    a: ring[k],
                    b: ring[(k + 1) % ring.len()],
                })
                .collect(),
        )
    }

    pub(crate) fn disc(c: Point, r: f64) -> Self {
        let (a, b) = ([c[0] + r, c[1]], [c[0] - r, c[1]]);
        Self::new(vec![Edge::of_bulge(a, b, 1.0), Edge::of_bulge(b, a, 1.0)])
    }

    /// A box about the origin, its corners (counter-clockwise from the
    /// lower left) rounded by `radii`.
    pub(crate) fn boxed(size: Point, radii: [f64; 4]) -> Self {
        let (x, y) = (size[0] / 2.0, size[1] / 2.0);
        Self::rounded(&[[-x, -y], [x, -y], [x, y], [-x, y]], &radii)
    }

    /// A polygon whose corners are rounded by fillets of `radii`.
    pub(crate) fn rounded(points: &[Point], radii: &[f64]) -> Self {
        let n = points.len();
        let unit = |d: Point| [d[0] / len(d), d[1] / len(d)];
        // Each corner's tangent points, and the fillet's bulge.
        let corners: Vec<(Point, Point, f64)> = (0..n)
            .map(|k| {
                let (p, r) = (points[k], radii.get(k).copied().unwrap_or(0.0));
                let u = unit(sub(p, points[(k + n - 1) % n]));
                let v = unit(sub(points[(k + 1) % n], p));
                let turn = cross(u, v).atan2(u[0] * v[0] + u[1] * v[1]);
                let t = r * (turn / 2.0).tan().abs();
                (
                    sub(p, [u[0] * t, u[1] * t]),
                    add(p, [v[0] * t, v[1] * t]),
                    (turn / 4.0).tan(),
                )
            })
            .collect();
        let mut edges = Vec::new();
        for k in 0..n {
            let (a, b, bulge) = corners[k];
            if len(sub(a, b)) > 1e-12 {
                edges.push(Edge::of_bulge(a, b, bulge));
            }
            let next = corners[(k + 1) % n].0;
            if len(sub(b, next)) > 1e-12 {
                edges.push(Edge::Line { a: b, b: next });
            }
        }
        Self::new(edges)
    }

    /// Every point within `r` of the segment.
    pub(crate) fn capsule(a: Point, b: Point, r: f64) -> Self {
        let d = sub(b, a);
        let l = len(d);
        if l < 1e-12 {
            return Self::disc(a, r);
        }
        let n = [-d[1] / l * r, d[0] / l * r];
        let (p, q) = (sub(a, n), sub(b, n));
        let (s, t) = (add(b, n), add(a, n));
        Self::new(vec![
            Edge::Line { a: p, b: q },
            Edge::of_bulge(q, s, 1.0),
            Edge::Line { a: s, b: t },
            Edge::of_bulge(t, p, 1.0),
        ])
    }

    pub(crate) fn inside(&self, p: Point) -> bool {
        // Off any line the edges' ends lie on, by far less than is judged:
        // a point on a chord is on neither side of it.
        let p = add(p, [1.3e-10, 0.7e-10]);
        self.near(p)
            .iter()
            .fold(false, |odd, e| odd ^ e.chord_crossed(p) ^ e.in_segment(p))
    }

    /// The distance of `p` from the boundary, negative inside.
    pub(crate) fn signed(&self, p: Point) -> f64 {
        let most = if self.rows.is_some() {
            ROW_MM
        } else {
            f64::INFINITY
        };
        let d = self
            .near(p)
            .iter()
            .map(|e| e.distance(p))
            .fold(most, f64::min);
        if self.inside(p) { -d } else { d }
    }

    /// How far the area reaches from the origin.
    fn reach(&self) -> f64 {
        self.edges
            .iter()
            .map(|e| match *e {
                Edge::Line { a, b } => len(a).max(len(b)),
                Edge::Arc { a, c, .. } => len(c) + len(sub(a, c)),
            })
            .fold(0.0, f64::max)
    }
}

/// The land of a pad in its own frame.
pub(crate) fn land(shape: &PadShape) -> Area {
    match shape {
        PadShape::Rect { size } => Area::boxed(size.to_mm_f64(), [0.0; 4]),
        PadShape::RoundRect {
            size,
            radius_ratio,
            max_radius_nm,
            corners,
        } => {
            let size = size.to_mm_f64();
            let short = size[0].min(size[1]);
            let mut r = (radius_ratio * short).min(short / 2.0);
            if let Some(most) = max_radius_nm {
                r = r.min(most.to_mm_f64());
            }
            let pick = |on: bool| if on { r } else { 0.0 };
            Area::boxed(
                size,
                [
                    pick(corners.bottom_left),
                    pick(corners.bottom_right),
                    pick(corners.top_right),
                    pick(corners.top_left),
                ],
            )
        }
        PadShape::Circle { diameter_nm } => Area::disc([0.0; 2], diameter_nm.to_mm_f64() / 2.0),
        PadShape::Oblong { size } => {
            let size = size.to_mm_f64();
            Area::boxed(size, [size[0].min(size[1]) / 2.0; 4])
        }
        PadShape::Custom { outline } => Area::of_outline(outline),
    }
}

/// One closed graphic shape, as a paste window draws it.
fn window(shape: &ShapeNm) -> Fallible<Area> {
    Ok(match shape {
        ShapeNm::Circle { center, radius_nm } => {
            Area::disc(center.to_mm_f64(), radius_nm.to_mm_f64())
        }
        ShapeNm::Rect { a, b, radius_nm } => {
            let (a, b) = (a.to_mm_f64(), b.to_mm_f64());
            let lo = [a[0].min(b[0]), a[1].min(b[1])];
            let hi = [a[0].max(b[0]), a[1].max(b[1])];
            Area::rounded(
                &[lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
                &[radius_nm.to_mm_f64(); 4],
            )
        }
        ShapeNm::Polygon { points, radii_nm } => {
            let points: Vec<Point> = points.iter().map(|p| p.to_mm_f64()).collect();
            let radii: Vec<f64> = radii_nm.iter().map(|r| r.to_mm_f64()).collect();
            Area::rounded(&points, &radii)
        }
        other => return Err(format!("the oracle does not draw {other:?}").into()),
    })
}

/// Where one pad is: its frame on the board.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    position: Point,
    rotation: f64,
    bottom: bool,
    datum: Point,
    datum_turn: f64,
    pad: Point,
    pad_turn: f64,
}

impl Frame {
    /// A part is placed by its footprint's datum: its datum goes to the
    /// placement position; seen from the top, a part on the bottom is
    /// turned over about the x axis, then every part turned by its
    /// rotation.
    fn of(placement: &Placement, footprint: &FootprintDefinition, pad: &Pad) -> Self {
        Self {
            position: placement.position,
            rotation: placement.rotation_deg,
            bottom: placement.layer == Layer::Bottom,
            datum: footprint.datum.offset.to_mm_f64(),
            datum_turn: footprint.datum.rotation_mdeg.to_degrees_f64(),
            pad: pad.position.to_mm_f64(),
            pad_turn: pad.rotation_mdeg.to_degrees_f64(),
        }
    }

    pub(crate) fn still(at: Point) -> Self {
        Self {
            position: at,
            rotation: 0.0,
            bottom: false,
            datum: [0.0; 2],
            datum_turn: 0.0,
            pad: [0.0; 2],
            pad_turn: 0.0,
        }
    }

    pub(crate) fn to_board(self, p: Point) -> Point {
        let p = add(turned(p, self.pad_turn), self.pad);
        let p = turned(sub(p, self.datum), -self.datum_turn);
        let p = if self.bottom { [p[0], -p[1]] } else { p };
        add(turned(p, self.rotation), self.position)
    }

    fn to_pad(self, p: Point) -> Point {
        let p = turned(sub(p, self.position), -self.rotation);
        let p = if self.bottom { [p[0], -p[1]] } else { p };
        let p = add(turned(p, self.datum_turn), self.datum);
        turned(sub(p, self.pad), -self.pad_turn)
    }
}

/// Material: an area in a frame, grown by `by`.
#[derive(Clone, Debug)]
pub(crate) struct Prim {
    pub area: Area,
    pub frame: Frame,
    pub by: f64,
    /// How far from the frame's origin it may reach, with a margin.
    pub reach: f64,
    /// How it is told in a message.
    pub what: String,
}

impl Prim {
    pub(crate) fn new(area: Area, frame: Frame, by: f64, what: impl Into<String>) -> Self {
        let reach = area.reach() + by.max(0.0) + 0.01;
        Self {
            area,
            frame,
            by,
            reach,
            what: what.into(),
        }
    }

    /// How far `p` is from the material's edge, at least: negative within.
    pub(crate) fn excess(&self, p: Point) -> f64 {
        let far = len(sub(p, self.frame.to_board([0.0; 2]))) - self.reach;
        // Far off, at least that much.
        if far > 1.0 {
            return far;
        }
        self.area.signed(self.frame.to_pad(p)) - self.by
    }

    pub(crate) fn bounds(&self) -> [Point; 2] {
        let c = self.frame.to_board([0.0; 2]);
        [
            [c[0] - self.reach, c[1] - self.reach],
            [c[0] + self.reach, c[1] + self.reach],
        ]
    }

    /// Board points just inside and just outside the material's edge.
    fn probes(&self, step: f64, out: &mut Vec<Point>) {
        const OFF: [f64; 3] = [0.003, 0.015, 0.06];
        for edge in &self.area.edges {
            let samples = edge.samples(step);
            for (at, right) in &samples {
                for off in OFF {
                    for side in [1.0, -1.0] {
                        for d in [self.by + off, self.by - off] {
                            let d = d * side;
                            out.push(
                                self.frame
                                    .to_board([at[0] + right[0] * d, at[1] + right[1] * d]),
                            );
                        }
                    }
                }
            }
            // Round the corner the edge starts at.
            if let Some((corner, _)) = samples.first() {
                for k in 0..16 {
                    let (s, c) = (f64::from(k) * std::f64::consts::TAU / 16.0).sin_cos();
                    for off in OFF {
                        for d in [self.by.abs() + off, self.by.abs() - off] {
                            out.push(self.frame.to_board([corner[0] + c * d, corner[1] + s * d]));
                        }
                    }
                }
            }
        }
    }
}

/// One pad of the design where it is placed.
pub(crate) struct DesignPad {
    pub pad: Pad,
    pub frame: Frame,
    pub layer: Layer,
    pub name: String,
}

/// Every pad of every placed part, from the document.
pub(crate) fn pads(project: &Project) -> Vec<DesignPad> {
    let mut out = Vec::new();
    for placement in &project.placements {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        let Some(physical) = crate::instances::resolve(&project.library, instance)
            .ok()
            .and_then(|r| r.physical)
        else {
            continue;
        };
        let footprint = &physical.footprint.definition;
        for pad in &footprint.pads {
            let over = pad.side == PadSide::Bottom;
            out.push(DesignPad {
                pad: pad.clone(),
                frame: Frame::of(placement, footprint, pad),
                layer: match (placement.layer, over) {
                    (Layer::Top, false) | (Layer::Bottom, true) => Layer::Top,
                    _ => Layer::Bottom,
                },
                name: format!("{} pad {}", instance.designator, pad.id),
            });
        }
    }
    out
}

/// Whether a pad has copper on `layer`.
pub(crate) fn has_copper(pad: &DesignPad, layer: CopperLayer) -> bool {
    match pad.pad.kind {
        PadKind::PlatedThrough => true,
        PadKind::NonPlatedHole => false,
        PadKind::Smd | PadKind::Fiducial | PadKind::Mechanical => {
            layer
                == match pad.layer {
                    Layer::Top => CopperLayer::Top,
                    Layer::Bottom => CopperLayer::Bottom,
                }
        }
    }
}

/// The copper of `layer`.
pub(crate) fn copper(project: &Project, layer: CopperLayer) -> Vec<Prim> {
    let mut out = Vec::new();
    for fill in project.fills.iter().filter(|f| f.layer == layer) {
        for region in fill.computed.iter().flat_map(|c| &c.regions) {
            let mut area = Area::of_ring(&region.outer);
            for hole in &region.holes {
                area.edges.extend(Area::of_ring(hole).edges);
            }
            let area = area.filed();
            out.push(Prim::new(area, Frame::still([0.0; 2]), 0.0, "a fill"));
        }
    }
    out.extend(copper_but_fills(project, layer));
    out
}

/// The copper of `layer` that is not fill.
pub(crate) fn copper_but_fills(project: &Project, layer: CopperLayer) -> Vec<Prim> {
    let mut out = Vec::new();
    for pad in pads(project).iter().filter(|p| has_copper(p, layer)) {
        out.push(Prim::new(land(&pad.pad.shape), pad.frame, 0.0, &pad.name));
    }
    for t in project.tracks.iter().filter(|t| t.layer == layer) {
        out.push(Prim::new(
            Area::capsule([0.0; 2], sub(t.end, t.start), t.width / 2.0),
            Frame::still(t.start),
            0.0,
            "a track",
        ));
    }
    for v in &project.vias {
        out.push(Prim::new(
            Area::disc([0.0; 2], v.diameter / 2.0),
            Frame::still(v.position),
            0.0,
            "a via",
        ));
    }
    for h in &project.board().holes {
        if let (true, Some(d)) = (h.plated, h.pad_diameter) {
            out.push(Prim::new(
                Area::disc([0.0; 2], d / 2.0),
                Frame::still(h.at),
                0.0,
                "a washer",
            ));
        }
    }
    out
}

/// The solder mask openings of `side`: a pad's own expansion, else the
/// board rule's; a tented pad and a tented via open nothing; a through pad
/// and a bare hole open both sides.
pub(crate) fn mask(project: &Project, side: Layer) -> Vec<Prim> {
    let rule = project.rules.effective().mask.expansion;
    let mut out = Vec::new();
    for pad in pads(project) {
        let here = matches!(
            pad.pad.kind,
            PadKind::PlatedThrough | PadKind::NonPlatedHole
        ) || pad.layer == side;
        let by = match pad.pad.mask {
            MaskRule::Inherit => rule,
            MaskRule::Tented => continue,
            MaskRule::Expansion(e) => e.to_mm_f64(),
        };
        if here {
            out.push(Prim::new(land(&pad.pad.shape), pad.frame, by, &pad.name));
        }
    }
    for v in project.vias.iter().filter(|v| !v.tented) {
        out.push(Prim::new(
            Area::disc([0.0; 2], v.diameter / 2.0),
            Frame::still(v.position),
            rule,
            "an open via",
        ));
    }
    for h in &project.board().holes {
        let size = match (h.plated, h.pad_diameter) {
            (true, Some(d)) => d,
            _ => h.drill,
        };
        out.push(Prim::new(
            Area::disc([0.0; 2], size / 2.0),
            Frame::still(h.at),
            rule,
            "a mounting hole",
        ));
    }
    out
}

/// The paste of `side`: a surface pad without a hole shrunk by the board's
/// reduction, or by its own, or its explicit windows.
pub(crate) fn paste(project: &Project, side: Layer) -> Fallible<Vec<Prim>> {
    let rule = project.rules.effective().paste.reduction;
    let mut out = Vec::new();
    for pad in pads(project).into_iter().filter(|p| p.layer == side) {
        let shrunk = |by: f64| Prim::new(land(&pad.pad.shape), pad.frame, -by, &pad.name);
        match &pad.pad.paste {
            PasteRule::Inherit => {
                if matches!(pad.pad.kind, PadKind::Smd | PadKind::Mechanical)
                    && pad.pad.hole.is_none()
                {
                    out.push(shrunk(rule));
                }
            }
            PasteRule::None => {}
            PasteRule::Reduction(r) => out.push(shrunk(r.to_mm_f64())),
            PasteRule::Windows(windows) => {
                for w in windows {
                    out.push(Prim::new(window(&w.shape)?, pad.frame, 0.0, &pad.name));
                }
            }
        }
    }
    Ok(out)
}

/// One drilled or routed hole of the design.
#[derive(Clone, Debug)]
pub(crate) struct DesignHole {
    /// The ends of its centre line (equal for a round hole).
    pub from: Point,
    pub to: Point,
    pub width: f64,
    pub plated: bool,
    pub via: bool,
    pub what: String,
}

/// The straight slot or circle an outline drawn as one is, by its two half
/// turns: their centres and width. The caller's fixtures draw nothing else.
fn drawn_slot(outline: &Outline) -> Fallible<(Point, Point, f64)> {
    let arcs: Vec<(Point, f64)> = Area::of_outline(outline)
        .edges
        .iter()
        .filter_map(|e| match *e {
            Edge::Arc { a, c, .. } => Some((c, len(sub(a, c)))),
            Edge::Line { .. } => None,
        })
        .collect();
    match arcs.as_slice() {
        [(a, r), (b, _)] => Ok((*a, *b, 2.0 * r)),
        _ => Err("a plated slot of the fixture has two ends".into()),
    }
}

/// Every hole the drill files must hold, from the document.
pub(crate) fn holes(project: &Project) -> Fallible<Vec<DesignHole>> {
    let mut out = Vec::new();
    for v in &project.vias {
        out.push(DesignHole {
            from: v.position,
            to: v.position,
            width: v.drill,
            plated: true,
            via: true,
            what: "a via".into(),
        });
    }
    for pad in pads(project) {
        if let Some(hole) = &pad.pad.hole {
            let offset = hole.offset.to_mm_f64();
            let (half, width) = match hole.shape {
                HoleShape::Round { diameter_nm } => ([0.0; 2], diameter_nm.to_mm_f64()),
                HoleShape::Slot {
                    width_nm,
                    length_nm,
                    angle_mdeg,
                } => {
                    let width = width_nm.to_mm_f64();
                    let half = (length_nm.to_mm_f64() - width) / 2.0;
                    (turned([half, 0.0], angle_mdeg.to_degrees_f64()), width)
                }
            };
            out.push(DesignHole {
                from: pad.frame.to_board(sub(offset, half)),
                to: pad.frame.to_board(add(offset, half)),
                width,
                plated: hole.plated,
                via: false,
                what: format!("the hole of {}", pad.name),
            });
        }
        for via in &pad.pad.vias {
            let at = pad.frame.to_board(via.at.to_mm_f64());
            out.push(DesignHole {
                from: at,
                to: at,
                width: via.drill_nm.to_mm_f64(),
                plated: true,
                via: false,
                what: format!("a via of {}", pad.name),
            });
        }
    }
    let board = project.board();
    for h in &board.holes {
        out.push(DesignHole {
            from: h.at,
            to: h.at,
            width: h.drill,
            plated: h.plated,
            via: false,
            what: "a mounting hole".into(),
        });
    }
    for c in &board.cutouts {
        if c.kind == (CutoutKind::Slot { plated: true }) {
            let (from, to, width) = drawn_slot(&c.outline)?;
            out.push(DesignHole {
                from,
                to,
                width,
                plated: true,
                via: false,
                what: "a plated board slot".into(),
            });
        }
    }
    Ok(out)
}

/// The contours the profile must draw: the outline and every cutout and
/// slot that is not plated.
pub(crate) fn profile(project: &Project) -> Vec<sparksmith_fabcheck::outline::Contour> {
    use sparksmith_fabcheck::outline::{Contour, Edge as Drawn};
    let board = project.board();
    std::iter::once(&board.outer)
        .chain(
            board
                .cutouts
                .iter()
                .filter(|c| c.kind != (CutoutKind::Slot { plated: true }))
                .map(|c| &c.outline),
        )
        .map(|o| Contour {
            edges: Area::of_outline(o)
                .edges
                .into_iter()
                .map(|e| match e {
                    Edge::Line { a, b } => Drawn::Line { from: a, to: b },
                    Edge::Arc { a, b, c, sweep } => Drawn::Arc {
                        from: a,
                        to: b,
                        center: c,
                        sweep,
                    },
                })
                .collect(),
        })
        .collect()
}

/// A written Gerber layer read at points: whether the last object covering
/// a point is dark.
pub(crate) struct Image {
    objects: Vec<(Cover, [Point; 2], bool)>,
}

enum Cover {
    Area(Area),
    /// A flashed macro: primitives in order, exposing or not, about a point.
    Macro(Vec<(bool, Area)>),
    /// A drawn arc of half width.
    Stroke(Edge, f64),
}

impl Cover {
    fn at(&self, p: Point) -> bool {
        match self {
            Self::Area(a) => a.signed(p) <= 0.0,
            Self::Macro(parts) => {
                parts.iter().fold(
                    false,
                    |on, (expose, a)| {
                        if a.signed(p) <= 0.0 { *expose } else { on }
                    },
                )
            }
            Self::Stroke(e, half) => e.distance(p) <= *half,
        }
    }
}

fn moved(area: Area, by: Point) -> Area {
    Area::new(
        area.edges
            .into_iter()
            .map(|e| match e {
                Edge::Line { a, b } => Edge::Line {
                    a: add(a, by),
                    b: add(b, by),
                },
                Edge::Arc { a, b, c, sweep } => Edge::Arc {
                    a: add(a, by),
                    b: add(b, by),
                    c: add(c, by),
                    sweep,
                },
            })
            .collect(),
    )
}

fn box_at(center: Point, w: f64, h: f64, rotation: f64) -> Area {
    let (x, y) = (w / 2.0, h / 2.0);
    let ring: Vec<Point> = [[-x, -y], [x, -y], [x, y], [-x, y]]
        .iter()
        .map(|p| turned(add(center, *p), rotation))
        .collect();
    Area::of_ring(&ring)
}

fn flashed(template: &Template, at: Point) -> Fallible<Cover> {
    Ok(match template {
        Template::Circle { diameter, .. } => Cover::Area(Area::disc(at, diameter / 2.0)),
        Template::Rect { w, h, .. } => Cover::Area(moved(box_at([0.0; 2], *w, *h, 0.0), at)),
        Template::Obround { w, h, .. } => {
            let half = (w - h).abs() / 2.0;
            let d = if w >= h { [half, 0.0] } else { [0.0, half] };
            Cover::Area(Area::capsule(sub(at, d), add(at, d), w.min(*h) / 2.0))
        }
        Template::Macro { primitives, .. } => {
            let mut parts = Vec::new();
            for p in primitives {
                let (on, area) = match p {
                    MacroPrim::Circle {
                        on,
                        diameter,
                        center,
                        rotation,
                    } => (*on, Area::disc(turned(*center, *rotation), diameter / 2.0)),
                    MacroPrim::CenterLine {
                        on,
                        width,
                        height,
                        center,
                        rotation,
                    } => (*on, box_at(*center, *width, *height, *rotation)),
                    other => return Err(format!("the oracle does not read {other:?}").into()),
                };
                parts.push((on, moved(area, at)));
            }
            Cover::Macro(parts)
        }
        Template::Polygon { .. } => return Err("a polygon aperture".into()),
    })
}

impl Image {
    pub(crate) fn read(source: &str) -> Fallible<Self> {
        Self::of(&reader::parse(source)?)
    }

    pub(crate) fn of(file: &GerberFile) -> Fallible<Self> {
        let mut objects = Vec::new();
        for object in &file.objects {
            let template = || -> Fallible<&Template> {
                let id = object.aperture.ok_or("an object without an aperture")?;
                Ok(&file.apertures.get(&id).ok_or("an aperture")?.template)
            };
            let round = || -> Fallible<f64> {
                match template()? {
                    Template::Circle { diameter, .. } => Ok(diameter / 2.0),
                    other => Err(format!("a draw with {other:?}").into()),
                }
            };
            let cover = match &object.kind {
                Kind::Flash { at } => flashed(template()?, *at)?,
                Kind::Draw { from, to } => Cover::Area(Area::capsule(*from, *to, round()?)),
                Kind::Arc {
                    from,
                    to,
                    center,
                    clockwise,
                } => Cover::Stroke(Edge::arc(*from, *to, *center, *clockwise), round()?),
                Kind::Region { contour } => {
                    let mut edges = Vec::new();
                    let mut a = contour.start;
                    for s in &contour.segments {
                        let (b, edge) = match *s {
                            Path::Line { to } => (to, Edge::Line { a, b: to }),
                            Path::Arc {
                                to,
                                center,
                                clockwise,
                            } => (to, Edge::arc(a, to, center, clockwise)),
                        };
                        edges.push(edge);
                        a = b;
                    }
                    Cover::Area(Area::new(edges).filed())
                }
            };
            let (edges, grow): (Vec<Edge>, f64) = match &cover {
                Cover::Area(a) => (a.edges.clone(), 0.0),
                Cover::Macro(parts) => (
                    parts.iter().flat_map(|(_, a)| a.edges.clone()).collect(),
                    0.0,
                ),
                Cover::Stroke(e, half) => (vec![*e], *half),
            };
            let mut b = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
            for e in edges {
                let points = match e {
                    Edge::Line { a, b } => vec![a, b],
                    Edge::Arc { a, c, .. } => {
                        let r = len(sub(a, c));
                        vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] + r]]
                    }
                };
                for p in points {
                    for k in 0..2 {
                        b[0][k] = b[0][k].min(p[k] - grow);
                        b[1][k] = b[1][k].max(p[k] + grow);
                    }
                }
            }
            objects.push((cover, b, object.dark));
        }
        Ok(Self { objects })
    }

    pub(crate) fn dark(&self, p: Point) -> bool {
        self.objects
            .iter()
            .rev()
            .find(|(cover, b, _)| (0..2).all(|k| b[0][k] <= p[k] && p[k] <= b[1][k]) && cover.at(p))
            .is_some_and(|(_, _, dark)| *dark)
    }
}

/// How near an edge the files' 1 nm coordinates, the writer's 1 µm chords
/// and the sampled arithmetic may tell a point either way, mm.
pub(crate) const BAND_MM: f64 = 0.0021;

/// Where a layer's written image and the design's material differ: points
/// on a grid over `window` and beside every edge, as messages. Points
/// within [`BAND_MM`] of an edge are not judged.
pub(crate) fn differences(
    image: &Image,
    design: &[Prim],
    window: [Point; 2],
    step: f64,
) -> Vec<String> {
    let mut points = Vec::new();
    for prim in design {
        prim.probes(step, &mut points);
    }
    // Beside every edge written, too: material the design does not have.
    for (cover, b, _) in &image.objects {
        let mid = [(b[0][0] + b[1][0]) / 2.0, (b[0][1] + b[1][1]) / 2.0];
        points.push(mid);
        if let Cover::Area(area) = cover {
            for e in &area.edges {
                for (at, right) in e.samples(step) {
                    for off in [0.003, 0.02] {
                        points.push([at[0] - right[0] * off, at[1] - right[1] * off]);
                        points.push([at[0] + right[0] * off, at[1] + right[1] * off]);
                    }
                }
            }
        }
    }
    let grid = 0.137;
    let (nx, ny) = (
        ((window[1][0] - window[0][0]) / grid) as usize,
        ((window[1][1] - window[0][1]) / grid) as usize,
    );
    for i in 0..nx {
        for j in 0..ny {
            points.push([
                window[0][0] + i as f64 * grid,
                window[0][1] + j as f64 * grid,
            ]);
        }
    }
    let mut out = Vec::new();
    for p in points {
        // The nearest edge decides whether the point is judged; the most
        // within decides what it must be.
        let mut within = f64::INFINITY;
        let mut near = false;
        let mut what = "nothing";
        for prim in design {
            let e = prim.excess(p);
            near |= e.abs() < BAND_MM;
            if e < within {
                within = e;
                what = &prim.what;
            }
        }
        if near {
            continue;
        }
        let want = within < 0.0;
        if image.dark(p) != want {
            out.push(format!(
                "({:.4}, {:.4}): {} is {}, must {}be made",
                p[0],
                p[1],
                what,
                if want {
                    "missing"
                } else {
                    "near; extra material"
                },
                if want { "" } else { "not " }
            ));
        }
    }
    out
}

mod tests {
    use super::*;

    /// The oracle itself against hand-derived areas and distances.
    #[test]
    fn areas_tell_exact_distances() {
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} {b}");
        let r = Area::boxed([2.0, 1.0], [0.25, 0.0, 0.0, 0.25]);
        close(r.signed([0.0, 0.0]), -0.5);
        close(r.signed([1.5, 0.5]), 0.5);
        close(r.signed([1.3, 0.9]), 0.5);
        // The rounded lower left corner: from its centre (-0.75, -0.25).
        close(r.signed([-1.05, -0.65]), 0.25);
        close(r.signed([-0.75, -0.25]), -0.25);
        let d = Area::disc([1.0, 2.0], 0.5);
        close(d.signed([1.0, 2.0]), -0.5);
        close(d.signed([1.0, 3.0]), 0.5);
        let c = Area::capsule([0.0, 0.0], [3.0, 4.0], 0.5);
        close(c.signed([1.5, 2.0]), -0.5);
        close(c.signed([6.0, 8.0]), 4.5);
        // A major arc: a 270 degree turn about the origin, closed by its
        // chord.
        let bulge = (std::f64::consts::PI * 1.5 / 4.0).tan();
        let arc = Area::new(vec![
            Edge::of_bulge([1.0, 0.0], [0.0, -1.0], bulge),
            Edge::Line {
                a: [0.0, -1.0],
                b: [1.0, 0.0],
            },
        ]);
        assert!(arc.inside([-0.9, 0.0]) && arc.inside([0.0, 0.0]));
        assert!(!arc.inside([0.6, -0.6]));
        close(arc.signed([0.0, 2.0]), 1.0);
    }

    /// A placement on the bottom, worked by hand: the datum at (1, 0) goes
    /// to (10, 20); the point 1 mm right and 1 mm up of it is turned over
    /// (1, -1) and turned a quarter (1, 1).
    #[test]
    fn a_bottom_placement_is_turned_over_and_then_turned() {
        let f = Frame {
            position: [10.0, 20.0],
            rotation: 90.0,
            bottom: true,
            datum: [1.0, 0.0],
            datum_turn: 0.0,
            pad: [2.0, 1.0],
            pad_turn: 0.0,
        };
        let p = f.to_board([0.0, 0.0]);
        assert!(
            (p[0] - 11.0).abs() < 1e-9 && (p[1] - 21.0).abs() < 1e-9,
            "{p:?}"
        );
        let q = f.to_pad([11.3, 21.2]);
        let back = f.to_board(q);
        assert!((back[0] - 11.3).abs() < 1e-9 && (back[1] - 21.2).abs() < 1e-9);
    }
}

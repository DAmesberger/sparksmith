//! What is on the board, worked out again from the document: every piece of
//! copper, opening and hole as plain polygons of many corners, with crossing
//! numbers and distances between segments. Nothing of the production
//! geometry, the copper graph, the polygon library or the point oracle takes
//! part. Shared with them: the document itself, its stored fill regions, the
//! footprint a part resolves to (`instances::resolve`), and the board's mask
//! expansion and paste reduction (the effective rules).
use super::Fallible;
use crate::geometry::Point;
use crate::ids::{Id, PadId, TerminalId};
use crate::library::{
    FootprintDefinition, HoleShape, MaskRule, Outline, Pad, PadKind, PadShape, PadSide, PasteRule,
    Segment, ShapeNm,
};
use crate::model::board::CutoutKind;
use crate::model::fill::FilledRegion;
use crate::model::{CopperLayer, Layer, Placement, Project};
use crate::rules::CopperObject;
use crate::{Endpoint, ObjectRef};
use sparksmith_fabcheck::raster::Bitmap;
use std::collections::BTreeMap;

/// How far a chord may lie inside its arc, mm.
const CHORD: f64 = 0.0005;

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// How far `p` is from the segment `a`–`b`.
fn from_segment(p: Point, a: Point, b: Point) -> f64 {
    let d = sub(b, a);
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}

/// Whether two segments cross, each ending on both sides of the other.
fn crosses((a, b): (Point, Point), (c, d): (Point, Point)) -> bool {
    let side = |p: Point, q: Point, r: Point| cross(sub(q, p), sub(r, p));
    let (s, t) = (side(a, b, c), side(a, b, d));
    let (u, v) = (side(c, d, a), side(c, d, b));
    s * t < -1e-18 && u * v < -1e-18
}

/// Closed rings, filled even-odd.
#[derive(Clone, Debug, Default)]
pub(super) struct Shape {
    pub rings: Vec<Vec<Point>>,
    min: Point,
    max: Point,
}

impl Shape {
    pub(super) fn new(rings: Vec<Vec<Point>>) -> Self {
        let points = || rings.iter().flatten();
        let fold =
            |k: usize, f: fn(f64, f64) -> f64, from: f64| points().map(|p| p[k]).fold(from, f);
        let low = [0, 1].map(|k| fold(k, f64::min, f64::INFINITY));
        let high = [0, 1].map(|k| fold(k, f64::max, f64::NEG_INFINITY));
        Self {
            min: low,
            max: high,
            rings,
        }
    }

    pub(super) fn of_outline(outline: &Outline) -> Self {
        Self::new(vec![outline_ring(outline)])
    }

    pub(super) fn of_region(region: &FilledRegion) -> Self {
        let mut rings = vec![region.outer.clone()];
        rings.extend(region.holes.iter().cloned());
        Self::new(rings)
    }

    fn segments(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        self.rings
            .iter()
            .flat_map(|r| (0..r.len()).map(move |k| (r[k], r[(k + 1) % r.len()])))
    }

    pub(super) fn inside(&self, p: Point) -> bool {
        self.segments().fold(false, |odd, (a, b)| {
            let over = (a[1] > p[1]) != (b[1] > p[1])
                && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            odd ^ over
        })
    }

    /// How far `p` is from the edge, whichever side it is on.
    pub(super) fn edge(&self, p: Point) -> f64 {
        self.segments()
            .map(|(a, b)| from_segment(p, a, b))
            .fold(f64::INFINITY, f64::min)
    }

    fn far(&self, other: &Self, most: f64) -> bool {
        (0..2).any(|k| self.min[k] > other.max[k] + most || other.min[k] > self.max[k] + most)
    }

    /// The segments within `by` of a box.
    fn near(&self, min: Point, max: Point, by: f64) -> Vec<(Point, Point)> {
        self.segments()
            .filter(|(a, b)| {
                (0..2).all(|k| a[k].min(b[k]) <= max[k] + by && a[k].max(b[k]) >= min[k] - by)
            })
            .collect()
    }

    /// The least distance between the edges of two shapes, whether one lies
    /// in the other or not; infinite past `most`.
    pub(super) fn edges_apart(&self, other: &Self, most: f64) -> f64 {
        if self.far(other, most) {
            return f64::INFINITY;
        }
        let (small, large) = (self.near(other.min, other.max, most), other);
        let mut least = f64::INFINITY;
        for (c, d) in large.near(self.min, self.max, most) {
            for (a, b) in &small {
                let wide = (0..2).any(|k| {
                    a[k].min(b[k]) > c[k].max(d[k]) + most || c[k].min(d[k]) > a[k].max(b[k]) + most
                });
                if wide {
                    continue;
                }
                if crosses((*a, *b), (c, d)) {
                    return 0.0;
                }
                for gap in [
                    from_segment(*a, c, d),
                    from_segment(*b, c, d),
                    from_segment(c, *a, *b),
                    from_segment(d, *a, *b),
                ] {
                    least = least.min(gap);
                }
            }
        }
        if least <= most { least } else { f64::INFINITY }
    }

    /// Whether the areas overlap: edges that cross, or one in the other.
    /// Shapes that only touch do not.
    pub(super) fn overlaps(&self, other: &Self) -> bool {
        if self.far(other, 0.0) {
            return false;
        }
        let within = |a: &Self, b: &Self| {
            a.rings
                .iter()
                .filter_map(|r| r.first())
                .any(|p| b.inside(*p) && b.edge(*p) > 1e-6)
        };
        if within(self, other) || within(other, self) {
            return true;
        }
        let mine = self.near(other.min, other.max, 0.0);
        let theirs = other.near(self.min, self.max, 0.0);
        mine.iter().any(|s| theirs.iter().any(|t| crosses(*s, *t)))
    }

    /// The gap between two shapes: 0 when they overlap, infinite past `most`.
    pub(super) fn gap(&self, other: &Self, most: f64) -> f64 {
        if self.far(other, most) {
            return f64::INFINITY;
        }
        if self.overlaps(other) {
            return 0.0;
        }
        self.edges_apart(other, most)
    }

    /// How deep a corner of either lies in the other, mm.
    pub(super) fn sunk(&self, other: &Self) -> f64 {
        if self.far(other, 0.0) {
            return 0.0;
        }
        let deep = |a: &Self, b: &Self| {
            a.rings
                .iter()
                .flatten()
                .filter(|p| b.inside(**p))
                .map(|p| b.edge(*p))
                .fold(0.0, f64::max)
        };
        deep(self, other).max(deep(other, self))
    }

    /// Dark on `bitmap`: the shape grown (or, below zero, shrunk) by `by`.
    pub(super) fn paint(&self, by: f64, bitmap: &mut Bitmap) {
        if self.rings.iter().all(Vec::is_empty) {
            return;
        }
        // Where each row of pixels crosses the edge.
        let (low, pixel) = (bitmap.min[1], bitmap.pixel);
        let mut rows: Vec<Vec<f64>> = vec![Vec::new(); bitmap.height];
        for (a, b) in self.segments() {
            let row = |y: f64| ((y - low) / pixel - 0.5).clamp(0.0, bitmap.height as f64);
            let first = row(a[1].min(b[1])).floor() as usize;
            let last = (row(a[1].max(b[1])).ceil() as usize + 1).min(bitmap.height);
            for (j, xs) in rows.iter_mut().enumerate().take(last).skip(first) {
                let y = low + (j as f64 + 0.5) * pixel;
                if (a[1] > y) != (b[1] > y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        for xs in &mut rows {
            xs.sort_by(f64::total_cmp);
        }
        let inside = |p: Point| {
            rows.get(((p[1] - low) / pixel) as usize)
                .is_some_and(|xs| xs.partition_point(|x| *x <= p[0]) % 2 == 1)
        };
        let grow = by.max(0.0) + pixel;
        bitmap.paint(
            [self.min[0] - grow, self.min[1] - grow],
            [self.max[0] + grow, self.max[1] + grow],
            |p| {
                let within = inside(p);
                match by {
                    b if b > 0.0 => within || self.edge(p) <= b,
                    b if b < 0.0 => within && self.edge(p) >= -b,
                    _ => within,
                }
            },
        );
    }
}

/// Points along an arc about `c` from `a`, turning `sweep`, without `a`.
fn around(c: Point, a: Point, sweep: f64) -> Vec<Point> {
    let r = (a[0] - c[0]).hypot(a[1] - c[1]);
    let from = (a[1] - c[1]).atan2(a[0] - c[0]);
    let step = 2.0 * (1.0 - (CHORD / r.max(2.0 * CHORD))).acos();
    let n = (sweep.abs() / step).ceil().max(1.0);
    (1..=n as usize)
        .map(|k| {
            let t = from + sweep * k as f64 / n;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

fn outline_ring(outline: &Outline) -> Vec<Point> {
    let mut ring = vec![outline.start];
    let mut a = outline.start;
    for s in &outline.segments {
        let b = s.to();
        match *s {
            Segment::Arc { bulge, .. } if bulge != 0.0 => {
                // The centre is on the chord's left for a turn to the left
                // of less than half.
                let k = (1.0 - bulge * bulge) / (4.0 * bulge);
                let d = sub(b, a);
                let c = [
                    (a[0] + b[0]) / 2.0 - d[1] * k,
                    (a[1] + b[1]) / 2.0 + d[0] * k,
                ];
                ring.extend(around(c, a, 4.0 * bulge.atan()));
            }
            _ => ring.push(b),
        }
        a = b;
    }
    // The last point is the first.
    if ring.len() > 1
        && let (Some(first), Some(last)) = (ring.first(), ring.last())
        && (first[0] - last[0]).hypot(first[1] - last[1]) < 1e-9
    {
        ring.pop();
    }
    ring
}

fn disc(c: Point, r: f64) -> Vec<Point> {
    around(c, [c[0] + r, c[1]], std::f64::consts::TAU)
}

/// Every point within `r` of the segment `a`–`b`.
fn capsule(a: Point, b: Point, r: f64) -> Vec<Point> {
    let d = sub(b, a);
    let l = d[0].hypot(d[1]);
    if l < 1e-12 {
        return disc(a, r);
    }
    let n = [-d[1] / l * r, d[0] / l * r];
    let mut ring = vec![sub(a, n), sub(b, n)];
    ring.extend(around(b, sub(b, n), std::f64::consts::PI));
    ring.push(add(a, n));
    ring.extend(around(a, add(a, n), std::f64::consts::PI));
    ring
}

/// A box about the origin, its corners rounded by `r` where `round` says
/// (lower left, lower right, upper right, upper left).
fn boxed([w, h]: Point, r: f64, round: [bool; 4]) -> Vec<Point> {
    let (x, y) = (w / 2.0, h / 2.0);
    let corners = [[-x, -y], [x, -y], [x, y], [-x, y]];
    // Along the sides: in and out of each corner.
    let along = [[0.0, -1.0], [1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]];
    let mut ring = Vec::new();
    for (k, c) in corners.into_iter().enumerate() {
        let (into, out) = (along[k], along[(k + 1) % 4]);
        if r > 1e-12 && round[k] {
            let start = [c[0] - into[0] * r, c[1] - into[1] * r];
            let centre = [start[0] + out[0] * r, start[1] + out[1] * r];
            ring.push(start);
            ring.extend(around(centre, start, std::f64::consts::FRAC_PI_2));
        } else {
            ring.push(c);
        }
    }
    ring
}

/// The land of a pad in its own frame.
fn land(shape: &PadShape) -> Vec<Point> {
    match shape {
        PadShape::Rect { size } => boxed(size.to_mm_f64(), 0.0, [false; 4]),
        PadShape::RoundRect {
            size,
            radius_ratio,
            max_radius_nm,
            corners,
        } => {
            let size = size.to_mm_f64();
            let most = max_radius_nm.map_or(f64::INFINITY, |r| r.to_mm_f64());
            boxed(
                size,
                (radius_ratio * size[0].min(size[1])).min(most),
                [
                    corners.bottom_left,
                    corners.bottom_right,
                    corners.top_right,
                    corners.top_left,
                ],
            )
        }
        PadShape::Circle { diameter_nm } => disc([0.0; 2], diameter_nm.to_mm_f64() / 2.0),
        PadShape::Oblong { size } => {
            let size = size.to_mm_f64();
            boxed(size, size[0].min(size[1]) / 2.0, [true; 4])
        }
        PadShape::Custom { outline } => outline_ring(outline),
    }
}

/// A turn, perhaps a reflection, and a shift: `[xx, xy, yx, yy, x, y]`.
#[derive(Clone, Copy, Debug)]
struct Affine([f64; 6]);

impl Affine {
    fn turn(deg: f64) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Self([c, -s, s, c, 0.0, 0.0])
    }

    fn shift(by: Point) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, by[0], by[1]])
    }

    /// Over about the x axis.
    const OVER: Self = Self([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);

    /// `self`, then `next`.
    fn then(self, next: Self) -> Self {
        let (a, b) = (self.0, next.0);
        let p = next.of([a[4], a[5]]);
        Self([
            b[0] * a[0] + b[1] * a[2],
            b[0] * a[1] + b[1] * a[3],
            b[2] * a[0] + b[3] * a[2],
            b[2] * a[1] + b[3] * a[3],
            p[0],
            p[1],
        ])
    }

    fn of(&self, p: Point) -> Point {
        let m = self.0;
        [
            m[0] * p[0] + m[1] * p[1] + m[4],
            m[2] * p[0] + m[3] * p[1] + m[5],
        ]
    }

    fn all(&self, ring: &[Point]) -> Vec<Point> {
        ring.iter().map(|p| self.of(*p)).collect()
    }

    /// From a pad's own frame to the board: its place in the footprint, the
    /// footprint by its datum, over when the part is on the bottom, turned
    /// and put where the part is.
    fn of_pad(placement: &Placement, footprint: &FootprintDefinition, pad: &Pad) -> Self {
        Self::turn(pad.rotation_mdeg.to_degrees_f64())
            .then(Self::shift(pad.position.to_mm_f64()))
            .then(Self::of_part(placement, footprint))
    }

    /// From a footprint's own frame to the board.
    fn of_part(placement: &Placement, footprint: &FootprintDefinition) -> Self {
        let datum = footprint.datum.offset.to_mm_f64();
        let own = Self::shift([-datum[0], -datum[1]])
            .then(Self::turn(-footprint.datum.rotation_mdeg.to_degrees_f64()));
        let own = match placement.layer {
            Layer::Top => own,
            Layer::Bottom => own.then(Self::OVER),
        };
        own.then(Self::turn(placement.rotation_deg))
            .then(Self::shift(placement.position))
    }
}

/// One pad of a placed part.
pub(super) struct PadOn {
    pub instance: Id,
    pub part: String,
    pub pad: Pad,
    pub net: Option<Id>,
    /// The outer layer of a pad of one side.
    pub side: Layer,
    pub at: Point,
    pub land: Shape,
    to_board: Affine,
}

impl PadOn {
    pub(super) fn object(&self) -> ObjectRef {
        ObjectRef::Pad {
            instance: self.instance,
            pad: self.pad.id,
        }
    }

    pub(super) fn on(&self, layer: CopperLayer) -> bool {
        match self.pad.kind {
            PadKind::PlatedThrough => true,
            PadKind::NonPlatedHole => false,
            _ => {
                layer
                    == match self.side {
                        Layer::Top => CopperLayer::Top,
                        Layer::Bottom => CopperLayer::Bottom,
                    }
            }
        }
    }
}

/// Every pad of the board. The boards' components have one terminal of the
/// same number for every package terminal (`build::part`), so a pad's net is
/// the one its number's terminal is on.
pub(super) fn pads(project: &Project) -> Vec<PadOn> {
    let mut nets: BTreeMap<(Id, u32), Id> = BTreeMap::new();
    for net in project.nets.iter() {
        for Endpoint {
            instance,
            terminal: TerminalId(t),
        } in &net.terminals
        {
            nets.insert((*instance, *t), net.id);
        }
    }
    let mut out = Vec::new();
    for placement in project.placements.iter() {
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
            let to_board = Affine::of_pad(placement, footprint, pad);
            let over = pad.side == PadSide::Bottom;
            out.push(PadOn {
                instance: instance.id,
                part: instance.designator.clone(),
                pad: pad.clone(),
                net: pad
                    .terminal
                    .and_then(|t| nets.get(&(instance.id, t.0)).copied()),
                side: match (placement.layer, over) {
                    (Layer::Top, false) | (Layer::Bottom, true) => Layer::Top,
                    _ => Layer::Bottom,
                },
                at: to_board.of([0.0; 2]),
                land: Shape::new(vec![to_board.all(&land(&pad.shape))]),
                to_board,
            });
        }
    }
    out
}

/// One piece of copper.
pub(super) struct Copper {
    pub kind: CopperObject,
    /// `None` is the board's own: a washer.
    pub object: Option<ObjectRef>,
    pub net: Option<Id>,
    pub layers: Vec<CopperLayer>,
    pub shape: Shape,
    /// Of a fill: which.
    pub fill: Option<Id>,
}

impl Copper {
    pub(super) fn on(&self, layer: CopperLayer) -> bool {
        self.layers.contains(&layer)
    }
}

/// The copper a tie draws between its lands: it is of no net, and of each.
pub(super) struct BridgeOn {
    pub instance: Id,
    pub nets: Vec<Id>,
    pub layer: CopperLayer,
    pub shape: Shape,
}

/// Every placed tie's bridges. The boards' ties are copper alone
/// (`build::tie`).
pub(super) fn bridges(project: &Project) -> Vec<BridgeOn> {
    let pads = pads(project);
    let mut out = Vec::new();
    for placement in project.placements.iter() {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        let physical = crate::instances::resolve(&project.library, instance)
            .ok()
            .and_then(|r| r.physical);
        let Some(physical) = physical.filter(|_| !instance.ties.is_empty()) else {
            continue;
        };
        let footprint = &physical.footprint.definition;
        for bridge in &footprint.bridges {
            let top = (bridge.side == PadSide::Top) == (placement.layer == Layer::Top);
            let to_board = Affine::of_part(placement, footprint);
            out.push(BridgeOn {
                instance: instance.id,
                nets: pads
                    .iter()
                    .filter(|p| p.instance == instance.id && bridge.pads.contains(&p.pad.id))
                    .filter_map(|p| p.net)
                    .collect(),
                layer: match top {
                    true => CopperLayer::Top,
                    false => CopperLayer::Bottom,
                },
                shape: Shape::new(vec![to_board.all(&outline_ring(&bridge.outline))]),
            });
        }
    }
    out
}

/// Every pad, track, via and washer.
pub(super) fn copper(project: &Project) -> Vec<Copper> {
    let stack = project.stackup.layers();
    let mut out: Vec<Copper> = Vec::new();
    for pad in pads(project) {
        let layers: Vec<CopperLayer> = stack.iter().copied().filter(|l| pad.on(*l)).collect();
        if !layers.is_empty() {
            out.push(Copper {
                kind: CopperObject::Pad,
                object: Some(pad.object()),
                net: pad.net,
                layers,
                shape: pad.land,
                fill: None,
            });
        }
    }
    for t in project.tracks.iter() {
        out.push(Copper {
            kind: CopperObject::Track,
            object: Some(ObjectRef::Track { id: t.id }),
            net: Some(t.net),
            layers: vec![t.layer],
            shape: Shape::new(vec![capsule(t.start, t.end, t.width / 2.0)]),
            fill: None,
        });
    }
    for v in project.vias.iter() {
        out.push(Copper {
            kind: CopperObject::Via,
            object: Some(ObjectRef::Via { id: v.id }),
            net: Some(v.net),
            layers: stack.clone(),
            shape: Shape::new(vec![disc(v.position, v.diameter / 2.0)]),
            fill: None,
        });
    }
    for h in &project.board().holes {
        if let (true, Some(d)) = (h.plated, h.pad_diameter) {
            out.push(Copper {
                kind: CopperObject::Pad,
                object: None,
                net: None,
                layers: stack.clone(),
                shape: Shape::new(vec![disc(h.at, d / 2.0)]),
                fill: None,
            });
        }
    }
    out
}

/// Every region of every fill, as it is stored.
pub(super) fn fills(project: &Project) -> Vec<Copper> {
    let mut out = Vec::new();
    for fill in project.fills.iter() {
        for region in fill.computed.iter().flat_map(|c| &c.regions) {
            out.push(Copper {
                kind: CopperObject::Fill,
                object: Some(ObjectRef::Fill { id: fill.id }),
                net: fill.net,
                layers: vec![fill.layer],
                shape: Shape::of_region(region),
                fill: Some(fill.id),
            });
        }
    }
    out
}

/// One drilled or routed hole.
#[derive(Clone, Debug)]
pub(super) struct HoleOn {
    pub from: Point,
    pub to: Point,
    pub width: f64,
    pub plated: bool,
    pub kind: CopperObject,
    pub net: Option<Id>,
}

impl HoleOn {
    pub(super) fn shape(&self) -> Shape {
        Shape::new(vec![capsule(self.from, self.to, self.width / 2.0)])
    }
}

/// Every hole of the drill files. The boards' plated slots are upright with
/// round ends (`build::slot`).
pub(super) fn holes(project: &Project) -> Fallible<Vec<HoleOn>> {
    let mut out = Vec::new();
    for v in project.vias.iter() {
        out.push(HoleOn {
            from: v.position,
            to: v.position,
            width: v.drill,
            plated: true,
            kind: CopperObject::ViaHole,
            net: Some(v.net),
        });
    }
    let kind = |plated: bool| match plated {
        true => CopperObject::PthHole,
        false => CopperObject::NpthHole,
    };
    for pad in pads(project) {
        if let Some(hole) = &pad.pad.hole {
            let at = hole.offset.to_mm_f64();
            let (half, width) = match hole.shape {
                HoleShape::Round { diameter_nm } => ([0.0; 2], diameter_nm.to_mm_f64()),
                HoleShape::Slot {
                    width_nm,
                    length_nm,
                    angle_mdeg,
                } => {
                    let (w, l) = (width_nm.to_mm_f64(), length_nm.to_mm_f64());
                    let along = Affine::turn(angle_mdeg.to_degrees_f64());
                    (along.of([(l - w) / 2.0, 0.0]), w)
                }
            };
            out.push(HoleOn {
                from: pad.to_board.of(sub(at, half)),
                to: pad.to_board.of(add(at, half)),
                width,
                plated: hole.plated,
                kind: kind(hole.plated),
                net: pad.net,
            });
        }
        for via in &pad.pad.vias {
            let at = pad.to_board.of(via.at.to_mm_f64());
            out.push(HoleOn {
                from: at,
                to: at,
                width: via.drill_nm.to_mm_f64(),
                plated: true,
                kind: CopperObject::PthHole,
                net: pad.net,
            });
        }
    }
    let board = project.board();
    for h in &board.holes {
        out.push(HoleOn {
            from: h.at,
            to: h.at,
            width: h.drill,
            plated: h.plated,
            kind: kind(h.plated),
            net: None,
        });
    }
    for cut in &board.cutouts {
        if cut.kind != (CutoutKind::Slot { plated: true }) {
            continue;
        }
        let shape = Shape::of_outline(&cut.outline);
        let width = shape.max[0] - shape.min[0];
        let x = (shape.max[0] + shape.min[0]) / 2.0;
        let (low, high) = (shape.min[1] + width / 2.0, shape.max[1] - width / 2.0);
        if high < low {
            return Err("a plated slot that is not upright".into());
        }
        out.push(HoleOn {
            from: [x, low],
            to: [x, high],
            width,
            plated: true,
            kind: CopperObject::PthHole,
            net: None,
        });
    }
    Ok(out)
}

/// The mask's openings on `side`, each with how far it grows.
pub(super) fn mask(project: &Project, side: Layer) -> Vec<(Shape, f64)> {
    let rule = project.rules.effective().mask.expansion;
    let mut out = Vec::new();
    for pad in pads(project) {
        let through = matches!(
            pad.pad.kind,
            PadKind::PlatedThrough | PadKind::NonPlatedHole
        );
        let by = match pad.pad.mask {
            MaskRule::Inherit => rule,
            MaskRule::Tented => continue,
            MaskRule::Expansion(e) => e.to_mm_f64(),
        };
        if through || pad.side == side {
            out.push((pad.land, by));
        }
    }
    for v in project.vias.iter().filter(|v| !v.tented) {
        out.push((Shape::new(vec![disc(v.position, v.diameter / 2.0)]), rule));
    }
    for h in &project.board().holes {
        let d = match (h.plated, h.pad_diameter) {
            (true, Some(d)) => d,
            _ => h.drill,
        };
        out.push((Shape::new(vec![disc(h.at, d / 2.0)]), rule));
    }
    out
}

/// The paste on `side`, each opening with how far it grows (below zero: it
/// is smaller than its land).
pub(super) fn paste(project: &Project, side: Layer) -> Fallible<Vec<(Shape, f64)>> {
    let rule = project.rules.effective().paste.reduction;
    let mut out = Vec::new();
    // A tie of copper alone is not soldered.
    let soldered = |p: &PadOn| {
        project
            .instance(p.instance)
            .is_none_or(|i| i.ties.is_empty())
    };
    for pad in pads(project)
        .into_iter()
        .filter(|p| p.side == side && soldered(p))
    {
        let by = match &pad.pad.paste {
            PasteRule::None => continue,
            PasteRule::Inherit => {
                let pasted = matches!(pad.pad.kind, PadKind::Smd | PadKind::Mechanical);
                if !pasted || pad.pad.hole.is_some() {
                    continue;
                }
                rule
            }
            PasteRule::Reduction(r) => r.to_mm_f64(),
            PasteRule::Windows(windows) => {
                for window in windows {
                    let ring = match &window.shape {
                        ShapeNm::Circle { center, radius_nm } => {
                            disc(center.to_mm_f64(), radius_nm.to_mm_f64())
                        }
                        ShapeNm::Rect { a, b, radius_nm } => {
                            let (a, b) = (a.to_mm_f64(), b.to_mm_f64());
                            let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                            let size = [(b[0] - a[0]).abs(), (b[1] - a[1]).abs()];
                            let ring = boxed(size, radius_nm.to_mm_f64(), [true; 4]);
                            Affine::shift(middle).all(&ring)
                        }
                        other => return Err(format!("a paste window {other:?}").into()),
                    };
                    out.push((Shape::new(vec![pad.to_board.all(&ring)]), 0.0));
                }
                continue;
            }
        };
        out.push((pad.land, -by));
    }
    Ok(out)
}

/// The pads of the copper graph by the pieces of copper that join them:
/// each net's pads, with the number of the piece each is on. Two pieces
/// join when they are of one net, on one layer, and no further apart than
/// `reach`; with none given, only where they overlap.
pub(super) fn joined(
    pieces: &[Copper],
    reach: Option<f64>,
) -> BTreeMap<Id, BTreeMap<(Id, PadId), usize>> {
    let groups = grouped(pieces, reach);
    let mut out: BTreeMap<Id, BTreeMap<(Id, PadId), usize>> = BTreeMap::new();
    for (piece, at) in pieces.iter().zip(groups) {
        if let (Some(net), Some(ObjectRef::Pad { instance, pad })) = (piece.net, piece.object) {
            out.entry(net).or_default().insert((instance, pad), at);
        }
    }
    out
}

/// Each piece's group: those it joins ([`joined`]), directly or through
/// others, by the least number among them.
pub(super) fn grouped(pieces: &[Copper], reach: Option<f64>) -> Vec<usize> {
    let mut root: Vec<usize> = (0..pieces.len()).collect();
    fn top(root: &mut [usize], mut k: usize) -> usize {
        while root[k] != k {
            root[k] = root[root[k]];
            k = root[k];
        }
        k
    }
    for i in 0..pieces.len() {
        for j in 0..i {
            let (a, b) = (&pieces[i], &pieces[j]);
            if a.net.is_none() || a.net != b.net || !a.layers.iter().any(|l| b.on(*l)) {
                continue;
            }
            let meets = match reach {
                Some(reach) => a.shape.gap(&b.shape, reach) <= reach,
                None => a.shape.overlaps(&b.shape),
            };
            if meets {
                let (x, y) = (top(&mut root, i), top(&mut root, j));
                root[x.max(y)] = x.min(y);
            }
        }
    }
    (0..pieces.len()).map(|k| top(&mut root, k)).collect()
}

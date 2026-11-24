//! Spacing measured in the files: the least distance between the copper of
//! a layer's regions (fill copper) and everything else that is made, per
//! net, read from the Gerber and Excellon files alone. No design rule and
//! no code of the program that wrote them takes part; the polygon images
//! are [`crate::polygons`]', the distances are exact between their edges.
use crate::excellon::{DrillFile, Hit};
use crate::gerber::{GerberFile, Kind, Object, Template};
use crate::polygons::{self, Ring, Shapes};
use crate::{Fallible, Point};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::BTreeMap;

/// What fill copper is measured against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Against {
    Fill,
    Track,
    Via,
    Pad,
    /// A drilled via hole, another plated hole, an unplated hole.
    ViaHole,
    PlatedHole,
    UnplatedHole,
}

/// One spacing: fill copper of `net` on `layer` and an object.
#[derive(Clone, Debug, PartialEq)]
pub struct Spacing {
    /// The layer's file function, `Copper,L2,Inr`.
    pub layer: String,
    pub net: String,
    pub against: Against,
    /// The object's net (empty for none).
    pub other: String,
    /// The gap, mm: 0 when they overlap.
    pub gap_mm: f64,
    /// Where the object is.
    pub at: Point,
}

struct Item {
    against: Against,
    net: Option<String>,
    shapes: Shapes,
    at: Point,
}

fn net_of(object: &Object) -> Option<String> {
    object
        .object_attributes
        .get(".N")
        .map(|v| v.join(","))
        .filter(|n| !n.is_empty())
}

fn function(object: &Object) -> &str {
    object
        .aperture_attributes
        .get(".AperFunction")
        .and_then(|v| v.first())
        .map_or("", String::as_str)
}

/// Whether an object is fill copper: a region of no component.
fn is_fill(object: &Object) -> bool {
    matches!(object.kind, Kind::Region { .. }) && !object.object_attributes.contains_key(".C")
}

/// A point of the object: a flash's centre, a draw's start.
fn anchor(object: &Object) -> Point {
    match &object.kind {
        Kind::Flash { at } => *at,
        Kind::Draw { from, .. } | Kind::Arc { from, .. } => *from,
        Kind::Region { contour } => contour.start,
    }
}

/// The edges of shapes in a grid of square cells.
struct Edges {
    cell: f64,
    cells: BTreeMap<(i64, i64), Vec<(Point, Point)>>,
}

fn mm(ring: &Ring) -> Vec<Point> {
    ring.iter()
        .map(|p| [f64::from(p.x) / 1e6, f64::from(p.y) / 1e6])
        .collect()
}

fn edges(shapes: &Shapes) -> impl Iterator<Item = (Point, Point)> + '_ {
    shapes.iter().flatten().flat_map(|r| {
        let points = mm(r);
        let n = points.len();
        (0..n)
            .map(move |k| (points[k], points[(k + 1) % n]))
            .collect::<Vec<_>>()
    })
}

impl Edges {
    fn of(shapes: &Shapes, cell: f64) -> Self {
        let mut cells: BTreeMap<(i64, i64), Vec<(Point, Point)>> = BTreeMap::new();
        for (a, b) in edges(shapes) {
            for key in Self::keys(cell, a, b, 0.0) {
                cells.entry(key).or_default().push((a, b));
            }
        }
        Self { cell, cells }
    }

    fn keys(cell: f64, a: Point, b: Point, by: f64) -> impl Iterator<Item = (i64, i64)> {
        let lo = |k: usize| ((a[k].min(b[k]) - by) / cell).floor() as i64;
        let hi = |k: usize| ((a[k].max(b[k]) + by) / cell).floor() as i64;
        let (x0, x1, y0, y1) = (lo(0), hi(0), lo(1), hi(1));
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    fn near(&self, a: Point, b: Point, by: f64) -> impl Iterator<Item = &(Point, Point)> {
        Self::keys(self.cell, a, b, by).flat_map(|k| self.cells.get(&k).into_iter().flatten())
    }
}

fn point_segment(p: Point, a: Point, b: Point) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - d[0] * t).hypot(p[1] - a[1] - d[1] * t)
}

/// The least distance between two segments that do not cross (shapes whose
/// areas overlap are told apart first).
fn segments(a: Point, b: Point, c: Point, d: Point) -> f64 {
    point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b))
}

/// The least distance of `item` from the fill `image`, when under `limit`.
fn gap(image: &Shapes, index: &Edges, item: &Shapes, limit: f64) -> Option<f64> {
    if !polygons::boolean(image, item, OverlayRule::Intersect).is_empty() {
        return Some(0.0);
    }
    let least = edges(item)
        .flat_map(|(a, b)| {
            index
                .near(a, b, limit)
                .map(move |(c, d)| segments(a, b, *c, *d))
        })
        .fold(f64::INFINITY, f64::min);
    (least < limit).then_some(least)
}

/// The box round shapes, mm.
fn bounds(shapes: &Shapes) -> Option<[Point; 2]> {
    let mut out: Option<[Point; 2]> = None;
    for (p, _) in edges(shapes) {
        let b = out.get_or_insert([p, p]);
        for k in 0..2 {
            b[0][k] = b[0][k].min(p[k]);
            b[1][k] = b[1][k].max(p[k]);
        }
    }
    out
}

fn meet(a: Option<[Point; 2]>, b: Option<[Point; 2]>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (0..2).all(|k| a[0][k] <= b[1][k] && b[0][k] <= a[1][k]),
        _ => false,
    }
}

/// A hole's image.
fn hole(hit: &Hit, diameter: f64) -> Fallible<Shapes> {
    let (from, to) = match *hit {
        Hit::Round { at, .. } => (at, at),
        Hit::Slot { from, to, .. } => (from, to),
    };
    let disc = |p| {
        polygons::flash(
            &Template::Circle {
                diameter,
                hole: None,
            },
            p,
        )
    };
    if from == to {
        return disc(from);
    }
    // The slot: its two ends and the straight part between.
    let d = [to[0] - from[0], to[1] - from[1]];
    let len = d[0].hypot(d[1]);
    let n = [-d[1] / len * diameter / 2.0, d[0] / len * diameter / 2.0];
    let bar: Ring = [
        [from[0] - n[0], from[1] - n[1]],
        [to[0] - n[0], to[1] - n[1]],
        [to[0] + n[0], to[1] + n[1]],
        [from[0] + n[0], from[1] + n[1]],
    ]
    .iter()
    .map(|p| polygons::nm(*p))
    .collect::<Fallible<_>>()?;
    let mut parts = disc(from)?;
    parts.extend(disc(to)?);
    parts.push(vec![bar]);
    Ok(polygons::boolean(&parts, &[], OverlayRule::Union))
}

/// Every spacing under `limit_mm` between fill copper and what is made of
/// other nets: other fills, tracks, vias and pads of its layer, and the
/// drilled holes of `drills` (each file and whether it is plated). A plated
/// hole's net is that of the land it is drilled in; copper and holes of no
/// net are measured from every fill. `layers` are the copper layers.
pub fn fill_spacings(
    layers: &[&GerberFile],
    drills: &[(&DrillFile, bool)],
    limit_mm: f64,
) -> Fallible<Vec<Spacing>> {
    let mut out = Vec::new();
    // Each hole's net, from any layer's land round its centre.
    let mut holes: Vec<Item> = Vec::new();
    for (file, plated) in drills {
        for hit in &file.hits {
            let tool = hit.tool();
            let diameter = *file
                .tools
                .get(&tool)
                .ok_or_else(|| format!("spacing: tool T{tool} is not defined"))?;
            let via = file
                .tool_functions
                .get(&tool)
                .is_some_and(|f| f.iter().any(|v| v == "ViaDrill"));
            let at = match *hit {
                Hit::Round { at, .. } => at,
                Hit::Slot { from, to, .. } => [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0],
            };
            holes.push(Item {
                against: match (*plated, via) {
                    (false, _) => Against::UnplatedHole,
                    (true, true) => Against::ViaHole,
                    (true, false) => Against::PlatedHole,
                },
                net: None,
                shapes: hole(hit, diameter)?,
                at,
            });
        }
    }
    let mut lands: Vec<(Option<[Point; 2]>, Option<String>, Shapes)> = Vec::new();
    for file in layers {
        for object in &file.objects {
            let land = matches!(object.kind, Kind::Flash { .. } | Kind::Region { .. });
            if land && object.dark && !is_fill(object) {
                let shapes = polygons::object_shapes(file, object)?;
                lands.push((bounds(&shapes), net_of(object), shapes));
            }
        }
    }
    for hole in holes
        .iter_mut()
        .filter(|h| h.against != Against::UnplatedHole)
    {
        let around = bounds(&hole.shapes);
        hole.net = lands
            .iter()
            .filter(|(b, net, _)| net.is_some() && meet(*b, around))
            .find(|(_, _, land)| {
                !polygons::boolean(land, &hole.shapes, OverlayRule::Intersect).is_empty()
            })
            .and_then(|(_, net, _)| net.clone());
    }
    for file in layers {
        let layer = file
            .file_attribute(".FileFunction")
            .map(|f| f.join(","))
            .unwrap_or_default();
        let mut items: Vec<Item> = Vec::new();
        let mut nets: Vec<Option<String>> = Vec::new();
        for object in &file.objects {
            if is_fill(object) {
                let net = net_of(object);
                if object.dark && !nets.contains(&net) {
                    nets.push(net);
                }
                continue;
            }
            let against = match function(object) {
                "ViaPad" => Against::Via,
                "Conductor" => Against::Track,
                "EtchedComponent" => Against::Track,
                _ => Against::Pad,
            };
            items.push(Item {
                against,
                net: net_of(object),
                shapes: polygons::object_shapes(file, object)?,
                at: anchor(object),
            });
        }
        let mut fills: Vec<(Option<String>, Shapes)> = Vec::new();
        for net in nets {
            let image = polygons::render(file, |o| is_fill(o) && net_of(o) == net)?;
            fills.push((net, image));
        }
        for (k, (net, image)) in fills.iter().enumerate() {
            let index = Edges::of(image, 0.5);
            let mut measure = |item: &Item| -> Fallible<()> {
                // Copper of one net is one conductor; an unplated hole and
                // copper without a net are nobody's.
                if net.is_some() && item.net == *net {
                    return Ok(());
                }
                if let Some(gap_mm) = gap(image, &index, &item.shapes, limit_mm) {
                    out.push(Spacing {
                        layer: layer.clone(),
                        net: net.clone().unwrap_or_default(),
                        against: item.against,
                        other: item.net.clone().unwrap_or_default(),
                        gap_mm,
                        at: item.at,
                    });
                }
                Ok(())
            };
            for item in items.iter().chain(&holes) {
                measure(item)?;
            }
            for (other, shapes) in fills.iter().skip(k + 1) {
                let at = shapes
                    .first()
                    .and_then(|s| s.first())
                    .and_then(|r| mm(r).first().copied())
                    .unwrap_or_default();
                measure(&Item {
                    against: Against::Fill,
                    net: other.clone(),
                    shapes: shapes.clone(),
                    at,
                })?;
            }
        }
    }
    Ok(out)
}

/// The least gap, mm, between a layer's fill copper and the board's edges
/// (the profile's contours): negative where copper lies outside the
/// outline or in a cutout, `None` without fill copper. Distances are exact
/// from the copper's corners to the contours; from the contours to the
/// copper they are taken at points 0.05 mm apart.
pub fn edge_gap(layer: &GerberFile, profile: &[crate::outline::Contour]) -> Fallible<Option<f64>> {
    let image = polygons::render(layer, is_fill)?;
    if image.is_empty() || profile.is_empty() {
        return Ok(None);
    }
    // The contours as chords, to tell inside from outside.
    let rings: Vec<Vec<Point>> = profile
        .iter()
        .map(|c| {
            c.edges
                .iter()
                .flat_map(|e| {
                    let n = match e {
                        crate::outline::Edge::Line { .. } => 1,
                        crate::outline::Edge::Arc { .. } => {
                            (e.length() / 0.01).ceil().clamp(8.0, 4096.0) as usize
                        }
                    };
                    (0..n).map(move |k| e.at(k as f64 / n as f64))
                })
                .collect()
        })
        .collect();
    let within = |p: Point, rings: &[Vec<Point>]| {
        rings.iter().fold(false, |odd, r| {
            (0..r.len()).fold(odd, |odd, k| {
                let (a, b) = (r[k], r[(k + 1) % r.len()]);
                odd ^ ((a[1] > p[1]) != (b[1] > p[1])
                    && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]))
            })
        })
    };
    let corners: Vec<Point> = image.iter().flatten().flat_map(mm).collect();
    let mut least = f64::INFINITY;
    for p in &corners {
        let d = profile
            .iter()
            .map(|c| c.distance(*p))
            .fold(f64::INFINITY, f64::min);
        least = least.min(if within(*p, &rings) { d } else { -d });
    }
    let index = Edges::of(&image, 0.5);
    let copper: Vec<Vec<Point>> = image.iter().flatten().map(mm).collect();
    for edge in profile.iter().flat_map(|c| &c.edges) {
        let n = (edge.length() / 0.05).ceil().clamp(1.0, 8192.0) as usize;
        for k in 0..n {
            let p = edge.at(k as f64 / n as f64);
            let d = index
                .near(p, p, 1.0)
                .map(|(a, b)| point_segment(p, *a, *b))
                .fold(f64::INFINITY, f64::min);
            // A contour inside the copper: none of its corners says so.
            least = least.min(if within(p, &copper) { -d.min(1.0) } else { d });
        }
    }
    Ok(Some(least))
}

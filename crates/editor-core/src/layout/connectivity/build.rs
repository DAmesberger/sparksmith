//! Building the graph: elements, the one joining rule, joints with
//! stations, vertices, edges and islands.
use super::{
    CopperGraph, Dsu, Edge, EdgeKind, Element, ElementKind, Group, JOIN_TOLERANCE_MM, Vertex,
};
use crate::geometry::{
    Bounds, Point, polygon_contains, segment_distance, segments_cross, segments_gap,
};
use crate::ids::Id;
use crate::layout::WorldPad;
use crate::layout::check_pair_budget;
use crate::layout::copper::{CURVE_SLACK_MM, CopperRing};
use crate::layout::fill::{self, RegionIndex};
use crate::layout::grid::BoundsGrid;
use crate::model::fill::FilledRegion;
use crate::model::{Project, Track, Via};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The farthest apart two outlines' bounds may be and still join.
pub(super) const JOIN_REACH_MM: f64 = 2.0 * CURVE_SLACK_MM + 2.0 * JOIN_TOLERANCE_MM;
/// Most joints per element; past it the graph is partial.
pub(super) const MAX_JOINTS_PER_ELEMENT: usize = 4;

/// Whether two outlines join: their copper overlaps or touches within
/// [`JOIN_TOLERANCE_MM`] plus the curve slack of either. The one joining rule.
pub(super) fn joins(a: &CopperRing, b: &CopperRing) -> bool {
    let slack = a.slack + b.slack;
    a.gap(b, JOIN_TOLERANCE_MM + slack + JOIN_TOLERANCE_MM) <= JOIN_TOLERANCE_MM + slack
}

/// One meeting of two elements, with the station on each (0 off tracks).
#[derive(Clone, Copy, Debug)]
struct Joint {
    a: usize,
    station_a: f64,
    b: usize,
    station_b: f64,
}

/// The geometry a joint's stations are read from.
#[derive(Clone, Copy)]
enum Geo<'a> {
    /// A track's centre line, half-width and length.
    Track {
        start: Point,
        end: Point,
        radius: f64,
        length: f64,
    },
    /// A land and its centre (a pad's terminal point, a via's centre).
    Land { centre: Point, ring: &'a CopperRing },
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// The station of the point of segment `a`–`b` (of `length`) nearest `p`.
fn station(p: Point, a: Point, b: Point, length: f64) -> f64 {
    let ab = sub(b, a);
    let len2 = dot(ab, ab);
    if len2 <= 0.0 {
        return 0.0;
    }
    (dot(sub(p, a), ab) / len2).clamp(0.0, 1.0) * length
}

fn at(a: Point, b: Point, length: f64, s: f64) -> Point {
    if length <= 0.0 {
        return a;
    }
    let t = s / length;
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

pub(super) fn nearest_contact(a: Point, b: Point, c: Point, d: Point) -> Point {
    let ab = (b[0] - a[0]).hypot(b[1] - a[1]);
    let cd = (d[0] - c[0]).hypot(d[1] - c[1]);
    let (s, t) = nearest(a, b, ab, c, d, cd);
    let first = at(a, b, ab, s);
    let second = at(c, d, cd, t);
    [(first[0] + second[0]) / 2.0, (first[1] + second[1]) / 2.0]
}

/// A point on the common copper of two outlines already known to meet.
pub(super) fn contact_point(a: &CopperRing, b: &CopperRing) -> Option<Point> {
    for (item, other) in [(a, b), (b, a)] {
        let inside = |p: Point| match other.spine {
            Some((c, d, r)) => segment_distance(p, c, d) <= r,
            None => polygon_contains(&other.ring, p),
        };
        if let Some((c, d, _)) = item.spine {
            if let Some(p) = [c, d].into_iter().find(|p| inside(*p)) {
                return Some(p);
            }
        } else if let Some(p) = item.ring.iter().copied().find(|p| inside(*p)) {
            return Some(p);
        }
    }
    let edges = |ring: &CopperRing| -> Vec<(Point, Point, f64)> {
        match ring.spine {
            Some(spine) => vec![spine],
            None => ring_edges(&ring.ring).map(|(p, q)| (p, q, 0.)).collect(),
        }
    };
    let right = edges(b);
    for (p, q, r) in edges(a) {
        for &(c, d, r2) in &right {
            if segments_gap(p, q, c, d) > r + r2 + JOIN_TOLERANCE_MM + a.slack + b.slack {
                continue;
            }
            let l = (q[0] - p[0]).hypot(q[1] - p[1]);
            let n = (d[0] - c[0]).hypot(d[1] - c[1]);
            let (s, t) = nearest(p, q, l, c, d, n);
            let first = at(p, q, l, s);
            let second = at(c, d, n, t);
            let f = if r + r2 > 0.0 { r / (r + r2) } else { 0.5 };
            return Some([
                first[0] + f * (second[0] - first[0]),
                first[1] + f * (second[1] - first[1]),
            ]);
        }
    }
    None
}

pub(super) fn fill_contact_point(index: &RegionIndex, ring: &CopperRing) -> Option<Point> {
    if let Some((p, q, _)) = ring.spine {
        if let Some(at) = [p, q].into_iter().find(|p| index.region_at(*p).is_some()) {
            return Some(at);
        }
    } else if let Some(p) = ring.ring.iter().find(|p| index.region_at(**p).is_some()) {
        return Some(*p);
    }
    let bounds = ring.bounds?.inflated(JOIN_REACH_MM);
    for (_, c, d) in index.edges_near(bounds.min, bounds.max) {
        let edge = CopperRing::capsule(c, d, 0.);
        if let Some(p) = contact_point(ring, &edge) {
            return Some(p);
        }
    }
    None
}
/// The nearest stations of segments `a`–`b` and `c`–`d`: where they
/// cross, or the closest pair of points.
fn nearest(a: Point, b: Point, ab: f64, c: Point, d: Point, cd: f64) -> (f64, f64) {
    if segments_cross(a, b, c, d) {
        let denominator = cross(sub(b, a), sub(d, c));
        if denominator != 0.0 {
            let t = (cross(sub(c, a), sub(d, c)) / denominator).clamp(0.0, 1.0);
            let s = t * ab;
            return (s, station(at(a, b, ab, s), c, d, cd));
        }
    }
    let candidates = [
        (segment_distance(a, c, d), 0.0, station(a, c, d, cd)),
        (segment_distance(b, c, d), ab, station(b, c, d, cd)),
        (segment_distance(c, a, b), station(c, a, b, ab), 0.0),
        (segment_distance(d, a, b), station(d, a, b, ab), cd),
    ];
    let best = candidates
        .into_iter()
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .unwrap_or((0.0, 0.0, 0.0));
    (best.1, best.2)
}

/// How a track end meets another track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// Not a track end on a track's copper only: a land, a crossing, or an
    /// end on the other track's centre line.
    Exact,
    /// The first (`A`) or second (`B`) element's end lies on the other
    /// track's copper but off its centre line.
    SoftA,
    SoftB,
}

/// Where two joined elements meet: their stations, and for tracks meeting
/// only by soft ends, the nearest stations to fall back on.
struct Meet {
    at: Vec<(f64, f64, End)>,
    fallback: Option<(f64, f64)>,
}

/// The stations where two joined elements meet. A track end whose copper
/// meets the other element joins there; otherwise the track joins at its
/// station nearest the other copper (a crossing, or a land's centre).
///
/// A track end that lies on another track's copper, but not on its centre
/// line, is a soft joint: it is dropped when that end is joined exactly
/// elsewhere ([`CopperGraph::scan`]), so that a short leg's far end does not
/// cut the leg short through the copper of the track before it.
fn stations(x: Geo<'_>, y: Geo<'_>) -> Meet {
    let exact = |at: Vec<(f64, f64)>| Meet {
        at: at.into_iter().map(|(s, t)| (s, t, End::Exact)).collect(),
        fallback: None,
    };
    match (x, y) {
        (Geo::Land { .. }, Geo::Land { .. }) => exact(vec![(0.0, 0.0)]),
        (Geo::Track { .. }, Geo::Land { .. }) => exact(track_land(x, y)),
        (Geo::Land { .. }, Geo::Track { .. }) => {
            exact(track_land(y, x).into_iter().map(|(s, t)| (t, s)).collect())
        }
        (
            Geo::Track {
                start: a,
                end: b,
                radius: r,
                length: ab,
            },
            Geo::Track {
                start: c,
                end: d,
                radius: q,
                length: cd,
            },
        ) => {
            let reach = r + q + JOIN_TOLERANCE_MM;
            let kind = |gap: f64, soft: End| {
                if gap <= JOIN_TOLERANCE_MM {
                    End::Exact
                } else {
                    soft
                }
            };
            let mut at = Vec::new();
            for (end, s) in [(a, 0.0), (b, ab)] {
                let gap = segment_distance(end, c, d);
                if gap <= reach {
                    at.push((s, station(end, c, d, cd), kind(gap, End::SoftA)));
                }
            }
            for (end, s) in [(c, 0.0), (d, cd)] {
                let gap = segment_distance(end, a, b);
                if gap <= reach {
                    at.push((station(end, a, b, ab), s, kind(gap, End::SoftB)));
                }
            }
            let nearest = nearest(a, b, ab, c, d, cd);
            if at.is_empty() {
                return exact(vec![nearest]);
            }
            let fallback = at.iter().all(|j| j.2 != End::Exact).then_some(nearest);
            Meet { at, fallback }
        }
    }
}

fn track_land(track: Geo<'_>, land: Geo<'_>) -> Vec<(f64, f64)> {
    let (
        Geo::Track {
            start,
            end,
            radius,
            length,
        },
        Geo::Land { centre, ring },
    ) = (track, land)
    else {
        return Vec::new();
    };
    let mut out: Vec<(f64, f64)> = [(start, 0.0), (end, length)]
        .into_iter()
        .filter(|(p, _)| joins(&CopperRing::capsule(*p, *p, radius), ring))
        .map(|(_, s)| (s, 0.0))
        .collect();
    if out.is_empty() {
        out.push((station(centre, start, end, length), 0.0));
    }
    out
}

/// The copper of a track: its centre line grown by its half-width.
pub(crate) fn track_copper(track: &Track) -> CopperRing {
    CopperRing::capsule(track.start, track.end, track.width / 2.0)
}

/// The copper of one via land: a disc of the via's diameter.
pub(crate) fn via_copper(via: &Via) -> CopperRing {
    CopperRing::capsule(via.position, via.position, via.diameter / 2.0)
}

/// The islands of an indexed fill that `ring` joins: the one joining rule
/// ([`joins`]) for fill copper, whose stored polygon is the fabricated
/// copper (no slack). The caller has checked the layer and the net.
pub(crate) fn islands_joined(index: &RegionIndex, ring: &CopperRing) -> Vec<u32> {
    let Some(bounds) = ring.bounds else {
        return Vec::new();
    };
    let reach = JOIN_TOLERANCE_MM + ring.slack;
    let lo = [bounds.min[0] - reach, bounds.min[1] - reach];
    let hi = [bounds.max[0] + reach, bounds.max[1] + reach];
    let mut out = Vec::new();
    match ring.spine {
        Some((a, b, radius)) => {
            // Inside an island, or within reach of one of its edges.
            out.extend(index.region_at(a));
            out.extend(
                index
                    .edges_near(lo, hi)
                    .filter(|(_, c, d)| segments_gap(a, b, *c, *d) <= radius + reach)
                    .map(|(r, _, _)| r),
            );
        }
        None => {
            // The outline inside an island, or an edge within reach of one.
            out.extend(ring.ring.first().and_then(|p| index.region_at(*p)));
            for (p, q) in ring_edges(&ring.ring) {
                let lo = [p[0].min(q[0]) - reach, p[1].min(q[1]) - reach];
                let hi = [p[0].max(q[0]) + reach, p[1].max(q[1]) + reach];
                out.extend(
                    index
                        .edges_near(lo, hi)
                        .filter(|(r, c, d)| !out.contains(r) && segments_gap(p, q, *c, *d) <= reach)
                        .map(|(r, _, _)| r)
                        .collect::<Vec<_>>(),
                );
            }
            // An island inside the outline: its edges lie within its bounds.
            let inside: Vec<u32> = index
                .edges_near(lo, hi)
                .filter(|(r, c, _)| !out.contains(r) && polygon_contains(&ring.ring, *c))
                .map(|(r, _, _)| r)
                .collect();
            out.extend(inside);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The islands of an indexed fill that `region`, an island of another fill,
/// joins by an edge or by lying in one: the one joining rule for two
/// polygons with holes, both the fabricated copper (no slack). An indexed
/// island that lies in `region` meets none of its edges: the caller asks
/// `region`'s own index. The caller has checked the layer and the net.
fn islands_meeting(index: &RegionIndex, region: &FilledRegion) -> BTreeSet<u32> {
    let reach = JOIN_TOLERANCE_MM;
    let mut out: BTreeSet<u32> = region
        .outer
        .first()
        .and_then(|p| index.region_at(*p))
        .into_iter()
        .collect();
    for (p, q) in std::iter::once(&region.outer)
        .chain(&region.holes)
        .flat_map(|ring| ring_edges(ring))
    {
        let lo = [p[0].min(q[0]) - reach, p[1].min(q[1]) - reach];
        let hi = [p[0].max(q[0]) + reach, p[1].max(q[1]) + reach];
        for (r, c, d) in index.edges_near(lo, hi) {
            if !out.contains(&r) && segments_gap(p, q, c, d) <= reach {
                out.insert(r);
            }
        }
    }
    out
}

fn ring_edges(ring: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| (*a, *b))
}

/// The islands of two fresh fills of one net and layer that join: (of `f`,
/// whose index is given; of `other`).
fn pour_joints(
    f: &fill::Fresh<'_>,
    index: &RegionIndex,
    other: &fill::Fresh<'_>,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut met = BTreeSet::new();
    for (r, region) in (0u32..).zip(&other.copper.regions) {
        for island in islands_meeting(index, region) {
            met.insert(island);
            out.push((island, r));
        }
    }
    // An island that meets no edge of the other's and holds none of it lies
    // in one of them, or clear of them all.
    let mut theirs = None;
    for (island, region) in (0u32..).zip(&f.copper.regions) {
        if met.contains(&island) {
            continue;
        }
        let theirs = theirs.get_or_insert_with(|| RegionIndex::filled(&other.copper.regions));
        out.extend(
            region
                .outer
                .first()
                .and_then(|p| theirs.region_at(*p))
                .map(|r| (island, r)),
        );
    }
    out
}

/// Where a track (`start`–`end`, half-width `radius`) joined to fill
/// island `island` meets it: each end whose copper reaches the island,
/// else the station nearest the island's edges.
fn fill_stations(
    index: &RegionIndex,
    island: u32,
    start: Point,
    end: Point,
    radius: f64,
    length: f64,
) -> Vec<f64> {
    let reach = radius + JOIN_TOLERANCE_MM;
    let near = |a: Point, b: Point| {
        let lo = [a[0].min(b[0]) - reach, a[1].min(b[1]) - reach];
        let hi = [a[0].max(b[0]) + reach, a[1].max(b[1]) + reach];
        index
            .edges_near(lo, hi)
            .filter(move |(r, _, _)| *r == island)
    };
    let mut out: Vec<f64> = [(start, 0.0), (end, length)]
        .into_iter()
        .filter(|(p, _)| {
            index.region_at(*p) == Some(island)
                || near(*p, *p).any(|(_, c, d)| segment_distance(*p, c, d) <= reach)
        })
        .map(|(_, s)| s)
        .collect();
    if out.is_empty() {
        let mut best = (f64::INFINITY, 0.0);
        for (_, c, d) in near(start, end) {
            let cd = (d[0] - c[0]).hypot(d[1] - c[1]);
            let (s, t) = nearest(start, end, length, c, d, cd);
            let p = at(start, end, length, s);
            let q = at(c, d, cd, t);
            let gap = (p[0] - q[0]).hypot(p[1] - q[1]);
            if gap < best.0 {
                best = (gap, s);
            }
        }
        out.push(best.1);
    }
    out
}

type PadBarrels = BTreeMap<(Id, crate::ids::PadId), Vec<(Point, CopperRing)>>;

/// The barrel wall of each pad-owned via, including its finished bore.
fn thermal_barrels(project: &Project) -> PadBarrels {
    let mut barrels = BTreeMap::new();
    if !project
        .library
        .footprints
        .iter()
        .any(|f| f.definition.pads.iter().any(|p| !p.vias.is_empty()))
    {
        return barrels;
    }
    let Ok(plating) = project.stackup.plating() else {
        return barrels;
    };
    let wall = plating.wall_nm.to_mm_f64();
    for placed in crate::layout::placed_pads(project) {
        let transform = placed.pad_transform();
        for via in &placed.pad.vias {
            let centre = transform.apply(via.at());
            let bore = via.drill() / 2.0;
            let outline = |radius| crate::geometry::Shape::Circle {
                center: centre,
                radius,
            };
            let mut outer = CopperRing::of_shape(&outline(bore + wall), true);
            let inner = outline(bore).flatten(CURVE_SLACK_MM);
            // Traverse the two loops with opposite winding. The coincident
            // radial edges close the annulus without adding copper in the bore.
            if let (Some(&start), Some(&end)) = (outer.ring.first(), inner.first()) {
                outer.ring.push(start);
                outer.ring.push(end);
                outer.ring.extend(inner.into_iter().rev());
                barrels
                    .entry((placed.instance, placed.pad.id))
                    .or_insert_with(Vec::new)
                    .push((centre, outer));
            }
        }
    }
    barrels
}

impl CopperGraph {
    /// Build the graph of `project`'s copper. The engine caches it per
    /// revision ([`crate::Engine::copper_graph`]).
    pub fn of(project: &Project) -> Self {
        Self::with_pads(project, crate::layout::pads(project))
    }

    /// The graph of `project`'s copper with `pads` as its connected pads,
    /// each on the net it carries. Update PCB builds the updated board with
    /// every pad on the net its copper had, to ask which pads that copper
    /// would join after the update.
    pub fn with_pads(project: &Project, pads: Vec<WorldPad>) -> Self {
        let mut pads = pads;
        pads.extend(super::ties::sense_destinations(project, &pads));
        Self::build(project, pads, None)
    }

    /// The graph of the copper of `nets` alone: their tracks, vias, pads
    /// and fresh fills, joined by the one rule. Different nets never join,
    /// so its joints, islands, lengths and paths are those of the whole
    /// board's graph on these nets, as long as neither stopped at a limit
    /// ([`Self::complete`]); its element indices are its own. A drag that
    /// checks a differential pair per pointer move builds this instead of
    /// the whole board's graph.
    pub fn of_nets(project: &Project, nets: &[Id]) -> Self {
        let mut all = crate::layout::pads(project);
        all.extend(super::ties::sense_destinations(project, &all));
        let pads = all.into_iter().filter(|p| nets.contains(&p.net)).collect();
        Self::build(project, pads, Some(nets))
    }

    /// The graph of `project`'s copper on `nets` (every net: `None`), with
    /// `pads` as its connected pads.
    fn build(project: &Project, pads: Vec<WorldPad>, nets: Option<&[Id]>) -> Self {
        let wanted = |net: &Id| nets.is_none_or(|n| n.contains(net));
        let layers = project.stackup.layers();
        let tracks: Vec<Track> = project
            .tracks
            .iter()
            .filter(|t| wanted(&t.net))
            .cloned()
            .collect();
        let vias: Vec<Via> = project
            .vias
            .iter()
            .filter(|v| wanted(&v.net))
            .cloned()
            .collect();
        let mut elements = Vec::new();
        let mut shapes = Vec::new();
        let element = |kind, net, layer| Element {
            kind,
            net,
            layer,
            first: 0,
            count: 0,
            barrel_centre: None,
        };
        let mut lands = Vec::with_capacity(tracks.len() + vias.len() + pads.len());
        for (index, track) in tracks.iter().enumerate() {
            lands.push(elements.len()..elements.len() + 1);
            shapes.push(track_copper(track));
            elements.push(element(
                ElementKind::Track { index },
                track.net,
                track.layer,
            ));
        }
        let mut via_lands = Vec::with_capacity(vias.len());
        for (index, via) in vias.iter().enumerate() {
            let first = elements.len();
            shapes.push(via_copper(via));
            for layer in layers.iter().filter(|l| via.on(**l, &layers)) {
                elements.push(element(ElementKind::Via { index }, via.net, *layer));
            }
            via_lands.push(first..elements.len());
            lands.push(first..elements.len());
        }
        // Retain the primary outline index of every pad. Barrel outlines
        // follow them, but their land elements are grouped with the owner.
        let thermal = thermal_barrels(project);
        let mut pad_lands = Vec::with_capacity(pads.len());
        let mut barrel_shapes = Vec::new();
        for (index, pad) in pads.iter().enumerate() {
            let first = elements.len();
            shapes.push(CopperRing::of_pad(pad));
            for layer in layers.iter().filter(|l| pad.on(**l)) {
                elements.push(element(ElementKind::Pad { index }, pad.net, *layer));
            }
            lands.push(first..elements.len());
            if let Some(barrels) = thermal.get(&(pad.instance, pad.pad)) {
                for (centre, shape) in barrels {
                    let start = elements.len();
                    for &layer in &layers {
                        let mut land = element(ElementKind::Pad { index }, pad.net, layer);
                        land.barrel_centre = Some(*centre);
                        elements.push(land);
                    }
                    barrel_shapes.push((shape.clone(), start..elements.len()));
                }
            }
            pad_lands.push(first..elements.len());
        }
        for (ring, span) in barrel_shapes {
            shapes.push(ring);
            lands.push(span);
        }
        // One element per island of each fresh fill; stale copper joins
        // nothing until it is refilled.
        let fills = fill::fresh_on(project, nets);
        let first_fill = elements.len();
        for f in &fills {
            for region in 0..f.copper.regions.len() {
                elements.push(element(
                    ElementKind::Fill {
                        fill: f.id,
                        region: u32::try_from(region).unwrap_or(u32::MAX),
                    },
                    f.net,
                    f.layer,
                ));
            }
        }
        let mid_planes = layers
            .iter()
            .filter_map(|l| {
                let (lo, hi) = project.stackup.copper_span(*l)?;
                Some((*l, 0.5 * (lo + hi)))
            })
            .collect();
        let mut graph = Self {
            layers,
            mid_planes,
            pads,
            tracks,
            vias,
            elements,
            shapes,
            lands,
            via_lands,
            pad_lands,
            ..Self::default()
        };
        let mut joints = graph.join();
        joints.extend(graph.fill_joints(&fills, first_fill));
        graph.connect(joints);
        if nets.is_none() {
            graph.record_ties(project, &fills);
        }
        graph
    }

    /// The station geometry of `element` with outline `ring`.
    fn geo<'a>(&'a self, element: &Element, ring: &'a CopperRing) -> Option<Geo<'a>> {
        Some(match element.kind {
            ElementKind::Track { index } => {
                let t = self.tracks.get(index)?;
                Geo::Track {
                    start: t.start,
                    end: t.end,
                    radius: t.width / 2.0,
                    length: (t.end[0] - t.start[0]).hypot(t.end[1] - t.start[1]),
                }
            }
            ElementKind::Via { index } => Geo::Land {
                centre: self.vias.get(index)?.position,
                ring,
            },
            ElementKind::Pad { index } => Geo::Land {
                centre: element
                    .barrel_centre
                    .unwrap_or(self.pads.get(index)?.position),
                ring,
            },
            ElementKind::Fill { .. } => return None,
        })
    }

    /// File the outlines by net, and find every joint between them. Each
    /// outline is tested once, whatever the layers its lands are on: two
    /// outlines that join meet on every layer both have a land on.
    fn join(&mut self) -> Vec<Joint> {
        let mut members: BTreeMap<Id, Vec<usize>> = BTreeMap::new();
        for (shape, lands) in self.lands.iter().enumerate() {
            if let Some(e) = self.elements.get(lands.start) {
                members.entry(e.net).or_default().push(shape);
            }
        }
        for (net, members) in members {
            let grid = BoundsGrid::new(
                members
                    .iter()
                    .map(|s| self.shapes.get(*s).and_then(|r| r.bounds)),
                JOIN_REACH_MM,
            );
            self.groups.insert(net, Group { grid, members });
        }
        let (joints, complete, scanned) = self.scan();
        self.partial = !complete;
        self.scanned = scanned;
        joints
    }

    /// The land elements of outlines `a` and `b` on the layers both reach.
    pub(super) fn common_lands(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        let (Some(x), Some(y)) = (self.lands.get(a), self.lands.get(b)) else {
            return Vec::new();
        };
        let layer = |e: usize| self.elements.get(e).map(|e| e.layer);
        x.clone()
            .flat_map(|ea| {
                y.clone()
                    .filter(move |eb| layer(ea).is_some() && layer(ea) == layer(*eb))
                    .map(move |eb| (ea, eb))
            })
            .collect()
    }

    /// Every joint between filed outlines, and whether the scan finished
    /// within its limits. A soft joint (see [`stations`]) is kept only when
    /// its track end is joined exactly to nothing. Two tracks left with no
    /// joint, and not joined through other copper, meet at their nearest
    /// stations instead, so dropping a soft joint never splits an island.
    fn scan(&self) -> (Vec<Joint>, bool, BTreeMap<Id, (usize, usize)>) {
        let limit = self.elements.len().saturating_mul(MAX_JOINTS_PER_ELEMENT);
        let mut joints = Vec::new();
        // (pair, joint, the soft end's element and station)
        let mut soft: Vec<(usize, Joint, usize, f64)> = Vec::new();
        let mut fallbacks: Vec<(usize, Joint)> = Vec::new();
        let mut near = Vec::new();
        let mut pairs = 0usize;
        let budget = check_pair_budget(self.shapes.len());
        let mut scanned = BTreeMap::new();
        for (net, group) in &self.groups {
            let (pairs_before, joints_before) = (pairs, joints.len());
            for (local, &a) in group.members.iter().enumerate() {
                let Some(ring_a) = self.shapes.get(a) else {
                    continue;
                };
                let Some(bounds) = ring_a.bounds else {
                    continue;
                };
                group.grid.near(&bounds, JOIN_REACH_MM, &mut near);
                for &b in near
                    .iter()
                    .filter(|j| **j > local)
                    .filter_map(|j| group.members.get(*j))
                {
                    pairs += 1;
                    if pairs > budget || joints.len() > limit {
                        return (joints, false, BTreeMap::new());
                    }
                    let Some(ring_b) = self.shapes.get(b) else {
                        continue;
                    };
                    let common = self.common_lands(a, b);
                    let Some(&(ea, eb)) = common.first() else {
                        continue;
                    };
                    if !joins(ring_a, ring_b) {
                        continue;
                    }
                    let (Some(x), Some(y)) = (
                        self.elements.get(ea).and_then(|e| self.geo(e, ring_a)),
                        self.elements.get(eb).and_then(|e| self.geo(e, ring_b)),
                    ) else {
                        continue;
                    };
                    let meet = stations(x, y);
                    for (ea, eb) in common {
                        let joint = |sa: f64, sb: f64| Joint {
                            a: ea,
                            station_a: sa,
                            b: eb,
                            station_b: sb,
                        };
                        for &(sa, sb, end) in &meet.at {
                            match end {
                                End::Exact => joints.push(joint(sa, sb)),
                                End::SoftA => soft.push((pairs, joint(sa, sb), ea, sa)),
                                End::SoftB => soft.push((pairs, joint(sa, sb), eb, sb)),
                            }
                        }
                        if let Some((sa, sb)) = meet.fallback {
                            fallbacks.push((pairs, joint(sa, sb)));
                        }
                    }
                }
            }
            scanned.insert(*net, (pairs - pairs_before, joints.len() - joints_before));
        }
        if !soft.is_empty() {
            let mut exact: HashMap<usize, Vec<f64>> = HashMap::new();
            for j in &joints {
                exact.entry(j.a).or_default().push(j.station_a);
                exact.entry(j.b).or_default().push(j.station_b);
            }
            let anchored = |element: usize, s: f64| {
                exact
                    .get(&element)
                    .is_some_and(|list| list.iter().any(|x| (x - s).abs() <= JOIN_TOLERANCE_MM))
            };
            let mut kept: HashSet<usize> = HashSet::new();
            for (pair, joint, element, s) in soft {
                if !anchored(element, s) {
                    joints.push(joint);
                    kept.insert(pair);
                }
            }
            let mut dsu = Dsu::new(self.elements.len());
            for lands in &self.lands {
                for e in lands.clone().skip(1) {
                    dsu.union(lands.start, e);
                }
            }
            for j in &joints {
                dsu.union(j.a, j.b);
            }
            for (pair, joint) in fallbacks {
                if !kept.contains(&pair) && dsu.find(joint.a) != dsu.find(joint.b) {
                    dsu.union(joint.a, joint.b);
                    joints.push(joint);
                }
            }
        }
        (joints, true, scanned)
    }

    /// The joints of each fresh fill's islands with the copper of its net
    /// on its layer, the islands of other fills among it, by the one
    /// joining rule, with the stations where tracks meet an island.
    fn fill_joints(&self, fills: &[fill::Fresh<'_>], first_fill: usize) -> Vec<Joint> {
        let mut out = Vec::new();
        let mut near = Vec::new();
        let mut first = first_fill;
        let bounds_of = |f: &fill::Fresh<'_>| {
            Bounds::of(
                f.copper
                    .regions
                    .iter()
                    .flat_map(|r| r.outer.iter().copied()),
            )
        };
        for (k, f) in fills.iter().enumerate() {
            let fill_at = first;
            first += f.copper.regions.len();
            let Some(bounds) = bounds_of(f) else {
                continue;
            };
            let index = RegionIndex::filled(&f.copper.regions);
            // Each pair of fills once, from the earlier.
            let mut other_at = first;
            for other in fills.iter().skip(k + 1) {
                let at = other_at;
                other_at += other.copper.regions.len();
                let apart = || {
                    bounds_of(other).is_none_or(|b| !b.inflated(JOIN_REACH_MM).intersects(&bounds))
                };
                if other.net != f.net || other.layer != f.layer || apart() {
                    continue;
                }
                out.extend(
                    pour_joints(f, &index, other)
                        .into_iter()
                        .map(|(island, region)| Joint {
                            a: fill_at + island as usize,
                            station_a: 0.0,
                            b: at + region as usize,
                            station_b: 0.0,
                        }),
                );
            }
            let Some(group) = self.groups.get(&f.net) else {
                continue;
            };
            group.grid.near(&bounds, JOIN_REACH_MM, &mut near);
            for &outline in near.iter().filter_map(|j| group.members.get(*j)) {
                let (Some(lands), Some(ring)) = (self.lands.get(outline), self.shapes.get(outline))
                else {
                    continue;
                };
                let Some(land) = lands
                    .clone()
                    .find(|e| self.elements.get(*e).is_some_and(|e| e.layer == f.layer))
                else {
                    continue;
                };
                let track = match self.elements.get(land).map(|e| e.kind) {
                    Some(ElementKind::Track { index }) => self.tracks.get(index),
                    _ => None,
                };
                for island in islands_joined(&index, ring) {
                    let stations = match track {
                        Some(t) => fill_stations(
                            &index,
                            island,
                            t.start,
                            t.end,
                            t.width / 2.0,
                            (t.end[0] - t.start[0]).hypot(t.end[1] - t.start[1]),
                        ),
                        None => vec![0.0],
                    };
                    let b = fill_at + island as usize;
                    out.extend(stations.into_iter().map(|s| Joint {
                        a: land,
                        station_a: s,
                        b,
                        station_b: 0.0,
                    }));
                }
            }
        }
        out
    }

    /// Lay out the vertices (track stations), the edges and the islands.
    fn connect(&mut self, joints: Vec<Joint>) {
        // Stations per track element.
        let mut stations: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
        for (i, e) in self.elements.iter().enumerate() {
            if let ElementKind::Track { index } = e.kind
                && let Some(t) = self.tracks.get(index)
            {
                let length = (t.end[0] - t.start[0]).hypot(t.end[1] - t.start[1]);
                stations.insert(i, vec![0.0, length]);
            }
        }
        for j in &joints {
            for (element, s) in [(j.a, j.station_a), (j.b, j.station_b)] {
                if let Some(list) = stations.get_mut(&element) {
                    list.push(s);
                }
            }
        }
        for list in stations.values_mut() {
            // The track's end is its last station, exactly: a station merged
            // into it must not shorten the track's stretches.
            let length = list.get(1).copied().unwrap_or(0.0);
            list.sort_by(f64::total_cmp);
            list.dedup_by(|later, kept| (*later - *kept).abs() <= JOIN_TOLERANCE_MM);
            if list.len() > 1
                && let Some(last) = list.last_mut()
            {
                *last = length;
            }
        }
        let mut vertices = Vec::new();
        for (i, e) in self.elements.iter_mut().enumerate() {
            e.first = vertices.len();
            match stations.get(&i) {
                Some(list) => vertices.extend(list.iter().map(|&station| Vertex {
                    element: i,
                    station,
                })),
                None => vertices.push(Vertex {
                    element: i,
                    station: 0.0,
                }),
            }
            e.count = vertices.len() - e.first;
        }
        self.vertices = vertices;
        let mut edges = Vec::new();
        for e in &self.elements {
            for v in e.first + 1..e.first + e.count {
                edges.push(Edge {
                    a: v - 1,
                    b: v,
                    kind: EdgeKind::Stretch,
                });
            }
        }
        // Only the lands of one physical outline have a vertical edge.
        // The pad and each barrel meet on their common layer through scan().
        for lands in &self.lands {
            for i in lands.clone().skip(1) {
                let (Some(a), Some(b)) = (self.elements.get(i - 1), self.elements.get(i)) else {
                    continue;
                };
                let kind = match b.kind {
                    ElementKind::Via { .. } => EdgeKind::Barrel,
                    ElementKind::Pad { .. } => EdgeKind::Plated,
                    _ => continue,
                };
                edges.push(Edge {
                    a: a.first,
                    b: b.first,
                    kind,
                });
            }
        }
        let mut joined: Vec<(usize, usize)> = joints
            .iter()
            .filter_map(|j| {
                let a = self.vertex_at(j.a, j.station_a)?;
                let b = self.vertex_at(j.b, j.station_b)?;
                (a != b).then_some((a.min(b), a.max(b)))
            })
            .collect();
        joined.sort_unstable();
        joined.dedup();
        edges.extend(joined.into_iter().map(|(a, b)| Edge {
            a,
            b,
            kind: EdgeKind::Joint,
        }));
        let n = self.vertices.len();
        let mut degree = vec![0usize; n + 1];
        for e in &edges {
            for v in [e.a, e.b] {
                if let Some(d) = degree.get_mut(v + 1) {
                    *d += 1;
                }
            }
        }
        for v in 0..n {
            let before = degree.get(v).copied().unwrap_or(0);
            if let Some(d) = degree.get_mut(v + 1) {
                *d += before;
            }
        }
        let mut fill = degree.clone();
        let mut adjacent = vec![0usize; edges.len() * 2];
        for e in &edges {
            for (from, to) in [(e.a, e.b), (e.b, e.a)] {
                if let Some(slot) = fill.get_mut(from)
                    && let Some(cell) = adjacent.get_mut(*slot)
                {
                    *cell = to;
                    *slot += 1;
                }
            }
        }
        let mut dsu = Dsu::new(n);
        for e in &edges {
            dsu.union(e.a, e.b);
        }
        let mut number = vec![usize::MAX; n];
        let mut islands = 0usize;
        let mut island = Vec::with_capacity(n);
        for v in 0..n {
            let root = dsu.find(v);
            let Some(slot) = number.get_mut(root) else {
                continue;
            };
            if *slot == usize::MAX {
                *slot = islands;
                islands += 1;
            }
            island.push(*slot);
        }
        self.islands = islands;
        self.island = island;
        self.offsets = degree;
        self.adjacent = adjacent;
        self.edges = edges;
    }

    /// The vertex of `element` at `station` (the nearest of a track's).
    fn vertex_at(&self, element: usize, station: f64) -> Option<usize> {
        let e = self.elements.get(element)?;
        let list = self.vertices.get(e.first..e.first + e.count)?;
        let at = list.partition_point(|v| v.station < station);
        let best = [at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten()
            .filter(|k| *k < list.len())
            .min_by(|x, y| {
                let dx = list
                    .get(*x)
                    .map_or(f64::INFINITY, |v| (v.station - station).abs());
                let dy = list
                    .get(*y)
                    .map_or(f64::INFINITY, |v| (v.station - station).abs());
                dx.total_cmp(&dy)
            })?;
        Some(e.first + best)
    }
}

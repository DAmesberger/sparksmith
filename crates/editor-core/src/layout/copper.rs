//! Board copper as exact outlines: every connected pad is its core pad
//! outline ([`WorldPad::outline`], built on `library/footprint/geometry.rs`)
//! and every track its round-capped stadium ([`super::track_shape`]).
//! Clearance, edge and routing checks measure these rings; which copper is
//! joined is the copper graph's alone ([`super::connectivity`]).
//!
//! Curved edges are flattened within [`CURVE_SLACK_MM`]. Each ring records
//! that slack, so the checks stay exact for straight-sided copper. On
//! curved edges, touching is accepted within the slack, and clearance is
//! reported conservatively: a curved gap may be flagged up to the slack
//! before it is truly under the rule, and is never missed. A track's round
//! caps are exact instead: its gaps are measured from its centre line less
//! its half-width.
use crate::diagnostics::FindingCode;
use super::board_edge::{BoardEdges, EdgeVerdict};
use super::grid::BoundsGrid;
use super::limits::{BoardRules, mm, mm_against};
use super::physical::{Conductor, Land};
use super::{CLEARANCE_MM, EPS, WorldPad, check_pair_budget, sync_preview};
use crate::diagnostics::{Finding, ObjectRef};
use crate::geometry::{
    Bounds, Point, Shape, polygons_overlap, polyline_polygon_distance, segments_gap,
};
use crate::ids::Id;
use crate::library::{PAD_ARC_TOLERANCE_MM, PadShape, Segment};
use crate::model::{CopperLayer, Project, Track, Via};
use crate::rules::CopperObject;

/// The chord tolerance curved copper edges are flattened at, in mm, and so
/// the most a measured gap on a curved edge differs from the exact one.
pub const CURVE_SLACK_MM: f64 = PAD_ARC_TOLERANCE_MM;

/// The farthest apart two outlines' bounds may be for any board check to
/// compare them under `limits`: the largest clearance the project's rules
/// and net classes resolve to (copper, drill to copper, hole to hole) plus
/// both outlines' curve slack. Shapes farther apart are never compared,
/// which [`CopperRing::gap`] would reject by their bounds anyway.
pub(super) fn check_reach(limits: &BoardRules<'_>) -> f64 {
    limits.max_clearance() + SLACK_REACH_MM
}

/// How far two outlines' measured gaps may stray from the exact ones: both
/// outlines' curve slack, and rounding.
const SLACK_REACH_MM: f64 = 2.0 * CURVE_SLACK_MM + 1e-6;

/// One copper outline in board coordinates: a closed ring, the exact
/// bounds of the shape it approximates, and how far the ring may stray from
/// that shape.
#[derive(Clone, Debug, PartialEq)]
pub struct CopperRing {
    pub ring: Vec<Point>,
    /// Exact bounds of the true outline (curves included), or `None` for a
    /// degenerate outline, which has no copper.
    pub bounds: Option<Bounds>,
    /// 0 for straight edges; [`CURVE_SLACK_MM`] when the ring flattens a
    /// curve.
    pub slack: f64,
    /// A track's centre line and half-width: its copper is exactly the
    /// points within that distance of the segment, so gaps to it are
    /// measured from the segment, not from the flattened ring (whose
    /// slack is then 0).
    pub spine: Option<(Point, Point, f64)>,
}

impl CopperRing {
    /// A board outline, flattened within [`CURVE_SLACK_MM`]. `curved` says
    /// whether it (or the pad shape it came from) has curved edges.
    pub fn of_shape(shape: &Shape, curved: bool) -> Self {
        let ring = shape.flatten(CURVE_SLACK_MM);
        let bounds = (ring.len() >= 3).then(|| shape.bounds(0.0));
        Self {
            ring,
            bounds,
            slack: if curved { CURVE_SLACK_MM } else { 0.0 },
            spine: None,
        }
    }

    /// A placed pad's copper outline.
    pub fn of_pad(pad: &WorldPad) -> Self {
        Self::of_shape(&pad.outline(), curved(&pad.shape))
    }

    /// A placed pad's copper outline, the same as [`Self::of_pad`] gives
    /// for its connected pad.
    pub fn of_placed_pad(pad: &super::PlacedPad) -> Self {
        Self::of_shape(&pad.outline(), curved(&pad.pad.shape))
    }

    /// A via's annular pad, a circle of its diameter.
    pub fn of_via(via: &Via) -> Self {
        Self::of_circle(via.position, via.diameter)
    }

    /// A via's drilled hole, a circle of its drill diameter.
    pub fn of_drill(via: &Via) -> Self {
        Self::of_circle(via.position, via.drill)
    }

    /// A circle of `diameter` at `center` (a drilled hole, a via's pad).
    /// Its gaps are exact, measured from the centre less the radius (a
    /// disc is a track of no length), so two drills exactly at their rule
    /// pass.
    pub fn of_circle(center: Point, diameter: f64) -> Self {
        let radius = diameter / 2.0;
        Self {
            slack: 0.0,
            spine: Some((center, center, radius)),
            ..Self::of_shape(&Shape::Circle { center, radius }, true)
        }
    }

    /// A drilled hole's outline: a round hole exactly ([`Self::of_circle`]),
    /// a slot flattened.
    pub fn of_hole(shape: &Shape) -> Self {
        match *shape {
            Shape::Circle { center, radius } => Self::of_circle(center, 2.0 * radius),
            _ => Self::of_shape(shape, true),
        }
    }

    /// A track's stadium, round end caps included. Its gaps are exact:
    /// measured from the centre line less the half-width.
    pub fn of_track(track: &Track) -> Self {
        Self::of_stadium(track.start, track.end, track.width)
    }

    /// The points within `width / 2` of `a`–`b` (a track, a slot), its
    /// gaps exact from the centre line.
    pub fn of_stadium(a: Point, b: Point, width: f64) -> Self {
        Self {
            slack: 0.0,
            spine: Some((a, b, width / 2.0)),
            ..Self::of_shape(&super::stadium_shape(a, b, width), true)
        }
    }

    /// The points within `radius` of segment `a`–`b` (a disc when they
    /// coincide), measured exactly from its spine. It carries no flattened
    /// ring, so it is only compared with other outlines through
    /// [`Self::gap`], which reads the spine.
    pub(super) fn capsule(a: Point, b: Point, radius: f64) -> Self {
        Self {
            ring: Vec::new(),
            bounds: Some(Bounds {
                min: [a[0].min(b[0]) - radius, a[1].min(b[1]) - radius],
                max: [a[0].max(b[0]) + radius, a[1].max(b[1]) + radius],
            }),
            slack: 0.0,
            spine: Some((a, b, radius)),
        }
    }

    pub(super) fn translated(&self, delta: Point) -> Self {
        let shift = |p: Point| [p[0] + delta[0], p[1] + delta[1]];
        Self {
            ring: self.ring.iter().map(|p| shift(*p)).collect(),
            bounds: self.bounds.map(|b| Bounds {
                min: shift(b.min),
                max: shift(b.max),
            }),
            slack: self.slack,
            spine: self.spine.map(|(a, b, r)| (shift(a), shift(b), r)),
        }
    }

    /// The measured boundary gap to `other` (0 when they overlap or touch),
    /// or some value of at least `limit` once their bounds alone prove the
    /// gap is that large. Infinite when either has no copper.
    pub fn gap(&self, other: &CopperRing, limit: f64) -> f64 {
        let (Some(a), Some(b)) = (self.bounds, other.bounds) else {
            return f64::INFINITY;
        };
        let dx = (a.min[0] - b.max[0]).max(b.min[0] - a.max[0]).max(0.0);
        let dy = (a.min[1] - b.max[1]).max(b.min[1] - a.max[1]).max(0.0);
        let lower = dx.hypot(dy);
        if lower >= limit {
            return lower;
        }
        match (self.spine, other.spine) {
            (Some((a, b, r)), Some((c, d, s))) => {
                return (segments_gap(a, b, c, d) - r - s).max(0.0);
            }
            (Some((a, b, r)), None) => {
                return (polyline_polygon_distance(&[a, b], &other.ring) - r).max(0.0);
            }
            (None, Some((c, d, s))) => {
                return (polyline_polygon_distance(&[c, d], &self.ring) - s).max(0.0);
            }
            (None, None) => {}
        }
        if polygons_overlap(&self.ring, &other.ring) {
            return 0.0;
        }
        let mut closed = self.ring.clone();
        if let Some(first) = self.ring.first() {
            closed.push(*first);
        }
        polyline_polygon_distance(&closed, &other.ring)
    }

    /// Whether the two outlines are closer than the default profile's
    /// copper clearance (an exact boundary passes; curved edges are judged
    /// conservatively). The board checks take the project's rules through
    /// [`Self::closer_than`] instead.
    pub fn violates_clearance(&self, other: &CopperRing) -> bool {
        self.closer_than(other, CLEARANCE_MM)
    }

    /// Whether the two outlines are closer than `limit` mm (an exact
    /// boundary passes; curved edges are judged conservatively).
    pub fn closer_than(&self, other: &CopperRing, limit: f64) -> bool {
        let slack = self.slack + other.slack;
        self.gap(other, limit + slack) - slack < limit - EPS
    }

    /// Whether the whole outline lies on the board, by its exact bounds.
    pub fn inside(&self, board: Point) -> bool {
        self.bounds.is_none_or(|b| {
            b.min[0] >= -EPS
                && b.min[1] >= -EPS
                && b.max[0] <= board[0] + EPS
                && b.max[1] <= board[1] + EPS
        })
    }
}

/// Whether a pad shape's outline has curved edges.
fn curved(shape: &PadShape) -> bool {
    match shape {
        PadShape::Rect { .. } => false,
        PadShape::RoundRect { .. } => shape.corner_radius() > 0.0,
        PadShape::Circle { .. } | PadShape::Oblong { .. } => true,
        PadShape::Custom { outline } => outline
            .segments
            .iter()
            .any(|s| matches!(s, Segment::Arc { bulge, .. } if bulge.abs() > 1e-12)),
    }
}

/// The board's lands ([`super::physical::lands`]: every pad with copper,
/// netted or not, and every plated mounting hole's washer), its vias and
/// its tracks with their copper outlines, built once per document state
/// for the clearance, hole and edge checks, which all read these outlines
/// rather than each flattening its own. A next state's copper is made from
/// the last one's ([`Self::next`]): an unchanged track or via keeps its
/// outline.
#[derive(Clone, Debug, Default)]
pub struct BoardCopper {
    lands: std::sync::Arc<Vec<Land>>,
    vias: Vec<Via>,
    via_rings: Vec<std::sync::Arc<CopperRing>>,
    drill_rings: Vec<std::sync::Arc<CopperRing>>,
    tracks: Vec<Track>,
    track_rings: Vec<std::sync::Arc<CopperRing>>,
}

impl PartialEq for BoardCopper {
    fn eq(&self, other: &Self) -> bool {
        self.lands == other.lands && self.vias == other.vias && self.tracks == other.tracks
    }
}

/// The outlines of `items`, each `make` of its item, reusing the outline
/// of an equal item of `last` (with its outlines `rings`) when `last` has
/// one with that identity. Collections stay in identity order but for
/// undone deletions, so the lookup is a binary search with a scan when it
/// misses.
fn rings_of<T: PartialEq>(
    items: &[T],
    last: Option<(&[T], &[std::sync::Arc<CopperRing>])>,
    id: impl Fn(&T) -> Id,
    make: impl Fn(&T) -> CopperRing,
) -> Vec<std::sync::Arc<CopperRing>> {
    items
        .iter()
        .map(|item| {
            let kept = last.and_then(|(old, rings)| {
                let at = match old.binary_search_by_key(&id(item), &id) {
                    Ok(k) => Some(k),
                    Err(_) => old.iter().position(|o| id(o) == id(item)),
                }?;
                (old.get(at)? == item).then(|| rings.get(at).cloned())?
            });
            kept.unwrap_or_else(|| std::sync::Arc::new(make(item)))
        })
        .collect()
}

/// One copper shape the checks compare: a land, a via or a track.
struct Copper<'a> {
    kind: CopperObject,
    conductor: Conductor,
    layer: CopperLayer,
    through: bool,
    ring: &'a CopperRing,
    object: ObjectRef,
    /// A land's name; tracks and vias are named by their net.
    name: Option<&'a str>,
}

impl Copper<'_> {
    fn net(&self) -> Option<Id> {
        self.conductor.net()
    }
}

impl BoardCopper {
    /// Every land, via and track of `project` with its exact outline.
    pub fn of(project: &Project) -> Self {
        Self::build(
            project,
            None,
            std::sync::Arc::new(super::physical::lands(project)),
        )
    }

    /// The copper of `project`, a next state of the one `self` was made
    /// of: tracks and vias equal to one of `self` keep its outlines. With
    /// `same_lands` the caller has established that nothing the lands
    /// are made of changed, and `self`'s lands are kept.
    pub fn next(&self, project: &Project, same_lands: bool) -> Self {
        let lands = if same_lands {
            self.lands.clone()
        } else {
            std::sync::Arc::new(super::physical::lands(project))
        };
        Self::build(project, Some(self), lands)
    }

    fn build(project: &Project, last: Option<&Self>, lands: std::sync::Arc<Vec<Land>>) -> Self {
        let via_id = |v: &Via| v.id;
        let vias = |rings: fn(&Self) -> &[std::sync::Arc<CopperRing>]| {
            last.map(|l| (l.vias.as_slice(), rings(l)))
        };
        Self {
            via_rings: rings_of(
                &project.vias,
                vias(|l| &l.via_rings),
                via_id,
                CopperRing::of_via,
            ),
            drill_rings: rings_of(
                &project.vias,
                vias(|l| &l.drill_rings),
                via_id,
                CopperRing::of_drill,
            ),
            track_rings: rings_of(
                &project.tracks,
                last.map(|l| (l.tracks.as_slice(), l.track_rings.as_slice())),
                |t: &Track| t.id,
                CopperRing::of_track,
            ),
            lands,
            vias: project.vias.to_vec(),
            tracks: project.tracks.to_vec(),
        }
    }

    /// The board's vias, in document order.
    pub fn vias(&self) -> &[Via] {
        &self.vias
    }

    /// The annular-pad outline of the via at `index` in [`Self::vias`].
    pub fn via_ring(&self, index: usize) -> Option<&CopperRing> {
        self.via_rings.get(index).map(|r| &**r)
    }

    /// The drilled-hole outline of the via at `index` in [`Self::vias`].
    pub fn drill_ring(&self, index: usize) -> Option<&CopperRing> {
        self.drill_rings.get(index).map(|r| &**r)
    }

    /// The board's tracks, in document order.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The outline of each of [`Self::tracks`], in the same order.
    pub fn track_rings(&self) -> impl ExactSizeIterator<Item = &CopperRing> + Clone {
        self.track_rings.iter().map(|r| &**r)
    }

    /// The lands: pads in placement then pad order, then washers.
    pub fn lands(&self) -> &[Land] {
        &self.lands
    }

    fn shapes(&self) -> Vec<Copper<'_>> {
        let lands = self.lands.iter().map(|land| Copper {
            kind: CopperObject::Pad,
            conductor: land.conductor,
            layer: land.layer.into(),
            through: land.through,
            ring: &land.ring,
            object: land.object(),
            name: Some(land.name.as_str()),
        });
        // A through via has copper on every layer, as a plated barrel pad.
        let vias = self
            .vias
            .iter()
            .zip(self.via_rings.iter().map(|r| &**r))
            .map(|(v, ring)| Copper {
                kind: CopperObject::Via,
                conductor: Conductor::Net(v.net),
                layer: CopperLayer::Top,
                through: true,
                ring,
                object: ObjectRef::Via { id: v.id },
                name: None,
            });
        let tracks = self
            .tracks
            .iter()
            .zip(self.track_rings())
            .map(|(t, ring)| Copper {
                kind: CopperObject::Track,
                conductor: Conductor::Net(t.net),
                layer: t.layer,
                through: false,
                ring,
                object: ObjectRef::Track { id: t.id },
                name: None,
            });
        lands.chain(vias).chain(tracks).collect()
    }

    /// Edge and clearance findings from these outlines, and whether the
    /// check stopped at its pair limit (its last finding then says so).
    pub(super) fn clearance(&self, project: &Project) -> (Vec<Finding>, bool) {
        let cx = ClearanceCx::new(self, project);
        let grid = BoundsGrid::new(cx.shapes.iter().map(|s| s.ring.bounds), cx.reach);
        let budget = check_pair_budget(cx.shapes.len());
        let mut issues = Vec::new();
        let mut near = Vec::new();
        let mut pairs = 0usize;
        for i in 0..cx.shapes.len() {
            cx.edge(i, &mut issues);
            let Some(bounds) = cx.shapes.get(i).and_then(|a| a.ring.bounds) else {
                continue;
            };
            grid.near(&bounds, cx.reach, &mut near);
            match cx.pairs(i, &near, budget.saturating_sub(pairs), &mut issues) {
                Ok(n) => pairs += n,
                Err(()) => {
                    issues.push(limit_finding());
                    return (issues, true);
                }
            }
        }
        (issues, false)
    }

    /// [`Self::clearance`] as each shape's findings, made from `last`,
    /// the items of an earlier state of this board, and `changes`, what
    /// changed since: a shape is checked again when it changed or a
    /// changed shape (in either state) is within the check's reach of it;
    /// every other shape keeps its findings. `None` when the check would
    /// stop at its pair limit, which only the full check reports.
    pub fn clearance_items(
        &self,
        project: &Project,
        last: Option<(&Items, &CopperChanges)>,
    ) -> Option<Items> {
        let cx = ClearanceCx::new(self, project);
        let keys = self.shape_keys();
        let bounds: Vec<Option<Bounds>> = cx.shapes.iter().map(|s| s.ring.bounds).collect();
        let grid = last
            .is_none()
            .then(|| BoundsGrid::new(bounds.iter().copied(), cx.reach));
        let mut near = Vec::new();
        let make = |i: usize, near: &mut Vec<usize>| {
            let mut out = Vec::new();
            cx.edge(i, &mut out);
            let Some(b) = bounds.get(i).copied().flatten() else {
                return Some((out, 0));
            };
            match &grid {
                Some(grid) => grid.near(&b, cx.reach, near),
                None => scan(&bounds, &b, cx.reach, near),
            }
            let pairs = cx.pairs(i, near, usize::MAX, &mut out).ok()?;
            Some((out, pairs))
        };
        let again = |i: usize, key: &ItemKey| {
            last.is_some_and(|(_, changes)| changes.again(key, bounds.get(i), cx.reach))
        };
        let last = last.map(|(items, _)| (items, &again as &dyn Fn(usize, &ItemKey) -> bool));
        let items = Items::made(&keys, last, |i| make(i, &mut near))?;
        (items.pairs() <= check_pair_budget(cx.shapes.len())).then_some(items)
    }

    /// The identity of each shape the clearance check compares, in its
    /// order: lands by index, then vias and tracks.
    fn shape_keys(&self) -> Vec<ItemKey> {
        (0..self.lands.len())
            .map(ItemKey::Land)
            .chain(self.vias.iter().map(|v| ItemKey::Via(v.id)))
            .chain(self.tracks.iter().map(|t| ItemKey::Track(t.id)))
            .collect()
    }

    /// What changed from `last`, an earlier state's copper with the same
    /// lands, to this: the tracks and vias added or changed, and the
    /// bounds and nets of every track and via added, changed or removed,
    /// in either state.
    pub fn changes_from(&self, last: &BoardCopper) -> CopperChanges {
        let mut changes = CopperChanges::default();
        // The copper graph reads neither lock, a via's tenting nor its
        // drill: an edit of those alone joins and splits nothing.
        diff_items(
            &self.tracks,
            &self.track_rings,
            &last.tracks,
            &last.track_rings,
            |t| t.id,
            |t| t.net,
            |old, new| {
                let mut old = old.clone();
                old.locked = new.locked;
                old == *new
            },
            ItemKey::Track,
            &mut changes,
        );
        diff_items(
            &self.vias,
            &self.via_rings,
            &last.vias,
            &last.via_rings,
            |v| v.id,
            |v| v.net,
            |old, new| {
                let mut old = old.clone();
                old.locked = new.locked;
                old.tented = new.tented;
                old.drill = new.drill;
                old == *new
            },
            ItemKey::Via,
            &mut changes,
        );
        changes
    }
}

/// Which tracks and vias changed between two states of one board's copper
/// ([`BoardCopper::changes_from`]).
#[derive(Clone, Debug, Default)]
pub struct CopperChanges {
    /// The tracks and vias added or changed: their checks run again.
    pub keys: std::collections::HashSet<ItemKey>,
    /// The copper bounds, in either state, of every track and via added,
    /// changed or removed: an unchanged shape near one is checked again.
    pub bounds: Vec<Bounds>,
    /// The nets, in either state, of the tracks and vias added, removed or
    /// changed in what the copper graph reads: the nets whose connectivity
    /// can change.
    pub nets: std::collections::BTreeSet<Id>,
}

impl CopperChanges {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.bounds.is_empty()
    }

    /// Whether the item `key` (with `bounds`) of a pairwise check must be
    /// made again: it changed, or a changed shape is within `reach`.
    pub fn again(&self, key: &ItemKey, bounds: Option<&Option<Bounds>>, reach: f64) -> bool {
        self.keys.contains(key)
            || bounds
                .copied()
                .flatten()
                .is_some_and(|b| self.near(&b, reach))
    }

    /// Whether an unchanged shape with `bounds` is within `reach` of a
    /// changed one, so its pairwise checks run again.
    pub fn near(&self, bounds: &Bounds, reach: f64) -> bool {
        self.bounds
            .iter()
            .any(|c| super::grid::within(bounds, c, reach))
    }
}

/// Record in `changes` the items of `new` (with outlines `rings`) that
/// differ from `old`'s, and the items of `old` that are gone. `joins_alike`
/// says whether two states of an item are alike to the copper graph.
#[allow(clippy::too_many_arguments)]
fn diff_items<T: PartialEq>(
    new: &[T],
    rings: &[std::sync::Arc<CopperRing>],
    old: &[T],
    old_rings: &[std::sync::Arc<CopperRing>],
    id: impl Fn(&T) -> Id,
    net: impl Fn(&T) -> Id,
    joins_alike: impl Fn(&T, &T) -> bool,
    key: impl Fn(Id) -> ItemKey,
    changes: &mut CopperChanges,
) {
    let at = |items: &[T], wanted: Id| match items.binary_search_by_key(&wanted, &id) {
        Ok(k) => Some(k),
        Err(_) => items.iter().position(|o| id(o) == wanted),
    };
    let mut kept = 0usize;
    for (item, ring) in new.iter().zip(rings) {
        match at(old, id(item)).and_then(|k| Some((old.get(k)?, old_rings.get(k)?))) {
            Some((before, _)) if before == item => kept += 1,
            Some((before, old_ring)) => {
                changes.keys.insert(key(id(item)));
                changes
                    .bounds
                    .extend(ring.bounds.into_iter().chain(old_ring.bounds));
                if !joins_alike(before, item) {
                    changes.nets.extend([net(item), net(before)]);
                }
                kept += 1;
            }
            None => {
                changes.keys.insert(key(id(item)));
                changes.bounds.extend(ring.bounds);
                changes.nets.insert(net(item));
            }
        }
    }
    if kept == old.len() {
        return;
    }
    // Some of `old` are gone.
    let present: std::collections::HashSet<Id> = new.iter().map(&id).collect();
    for (item, ring) in old.iter().zip(old_rings) {
        if !present.contains(&id(item)) {
            changes.bounds.extend(ring.bounds);
            changes.nets.insert(net(item));
        }
    }
}

/// The identity of one item a pairwise board check keeps findings for
/// ([`Items`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKey {
    /// A land, by index (lands are kept only while unchanged).
    Land(usize),
    Via(Id),
    Track(Id),
    /// A pad hole, by its index among the pad holes.
    PadHole(usize),
    /// A board mounting hole, by index.
    Mount(usize),
    /// The pad hole check's annular rings, which read no track or via.
    Rings,
}

/// Whether item `i` (with its key) of a check must be made again.
pub type Again<'a> = &'a dyn Fn(usize, &ItemKey) -> bool;

/// The findings and the number of pairs compared of each item of one
/// pairwise board check, in the check's order: what a state keeps so the
/// next state checks again only the items a change reaches.
#[derive(Clone, Debug, Default)]
pub struct Items {
    items: Vec<(ItemKey, Vec<Finding>, usize)>,
}

impl Items {
    /// The items of `keys` (in check order): each made by `make` when
    /// there is no `last`, or when `last`'s test says it must be made
    /// again; otherwise `last`'s findings for it. `None` when `make` gives
    /// none (a check that must stop).
    pub(super) fn made(
        keys: &[ItemKey],
        last: Option<(&Items, Again<'_>)>,
        mut make: impl FnMut(usize) -> Option<(Vec<Finding>, usize)>,
    ) -> Option<Self> {
        let mut items = Vec::with_capacity(keys.len());
        let mut cursor = 0usize;
        for (i, key) in keys.iter().enumerate() {
            let kept = last.and_then(|(old, again)| {
                if again(i, key) {
                    return None;
                }
                // Unchanged items keep their order, so a forward search
                // from the last one found finds the next.
                let at = old
                    .items
                    .get(cursor..)?
                    .iter()
                    .position(|(k, _, _)| k == key)?;
                cursor += at + 1;
                old.items.get(cursor - 1).map(|(_, f, p)| (f.clone(), *p))
            });
            let (findings, pairs) = match kept {
                Some(item) => item,
                None => make(i)?,
            };
            items.push((*key, findings, pairs));
        }
        Some(Self { items })
    }

    /// Every finding, in check order.
    pub fn findings(&self) -> impl Iterator<Item = &Finding> {
        self.items.iter().flat_map(|(_, f, _)| f)
    }

    /// The pairs compared, all items together.
    pub fn pairs(&self) -> usize {
        self.items
            .iter()
            .fold(0usize, |sum, (_, _, p)| sum.saturating_add(*p))
    }
}

/// Every shape of `bounds` (by index, ascending) that a [`BoundsGrid`] of
/// them would list near `near` within `reach`: the same filter, by a scan,
/// for the few queries of a changed board.
pub(super) fn scan(bounds: &[Option<Bounds>], near: &Bounds, reach: f64, out: &mut Vec<usize>) {
    out.clear();
    out.extend(
        bounds
            .iter()
            .enumerate()
            .filter(|(_, b)| b.is_some_and(|b| super::grid::within(near, &b, reach)))
            .map(|(k, _)| k),
    );
}

/// One state's clearance check: its shapes, rules, reach and edges.
struct ClearanceCx<'a> {
    project: &'a Project,
    shapes: Vec<Copper<'a>>,
    limits: BoardRules<'a>,
    reach: f64,
    edges: BoardEdges,
}

impl<'a> ClearanceCx<'a> {
    fn new(copper: &'a BoardCopper, project: &'a Project) -> Self {
        let limits = BoardRules::of(project);
        Self {
            project,
            shapes: copper.shapes(),
            reach: check_reach(&limits),
            limits,
            edges: BoardEdges::of(project),
        }
    }

    fn label(&self, shape: &Copper) -> String {
        match (shape.name, shape.net()) {
            (Some(name), _) => name.to_owned(),
            (None, Some(net)) => {
                format!("a {} on {}", shape.kind.label(), self.project.net_name(net))
            }
            (None, None) => String::new(),
        }
    }

    /// Shape `i` against the board's edges.
    fn edge(&self, i: usize, issues: &mut Vec<Finding>) {
        let Some(a) = self.shapes.get(i) else {
            return;
        };
        let limits = &self.limits;
        let edge = limits.clearance(a.kind, a.net(), CopperObject::BoardEdge, None);
        match self.edges.verdict(a.ring, edge) {
            EdgeVerdict::Clear => {}
            EdgeVerdict::Outside => issues.push(Finding::new(
                FindingCode::BoardOutsideOutline,
                format!("{} extends beyond board outline", self.label(a)),
                vec![a.object],
            )),
            EdgeVerdict::Inside(kind) => issues.push(Finding::new(
                FindingCode::BoardOutsideOutline,
                format!(
                    "{} extends into {}",
                    self.label(a),
                    super::board_edge::EdgeLoopKind::Cutout(kind).describe()
                ),
                vec![a.object],
            )),
            EdgeVerdict::TooClose { gap, near } => {
                let need = limits.clearance_need(a.kind, a.net(), CopperObject::BoardEdge, None);
                issues.push(Finding::new(
                    FindingCode::BoardEdgeClearance,
                    format!(
                        "{} is {} mm from {}; {}",
                        crate::library::names::capitalised(&self.label(a)),
                        mm_against(gap, need.mm),
                        near.describe(),
                        need.text()
                    ),
                    vec![a.object],
                ));
            }
        }
    }

    /// Shape `i` against each later shape of `near` (ascending): the pairs
    /// compared, or `Err` once more than `budget` would be.
    fn pairs(
        &self,
        i: usize,
        near: &[usize],
        budget: usize,
        issues: &mut Vec<Finding>,
    ) -> Result<usize, ()> {
        let Some(a) = self.shapes.get(i) else {
            return Ok(0);
        };
        let limits = &self.limits;
        let mut pairs = 0usize;
        for b in near
            .iter()
            .filter(|j| **j > i)
            .filter_map(|j| self.shapes.get(*j))
        {
            pairs += 1;
            if pairs > budget {
                return Err(());
            }
            if a.layer != b.layer && !a.through && !b.through {
                continue;
            }
            if a.conductor.same(b.conductor) {
                continue;
            }
            let (an, bn) = class_nets(a, b);
            let mut gap = limits.clearance(a.kind, an, b.kind, bn);
            // The footprint exception spares the classes' fab spacing
            // only: an isolation clearance between their classes still
            // holds between two pads of one footprint.
            let isolation = match (within_footprint(a, b), a.net(), b.net()) {
                (true, Some(x), Some(y)) => limits.isolation(x, y),
                _ => None,
            }
            .filter(|need| need.mm > gap);
            if let Some(need) = &isolation {
                gap = need.mm;
            }
            if a.ring.closer_than(b.ring, gap) {
                let need =
                    isolation.unwrap_or_else(|| limits.clearance_need(a.kind, an, b.kind, bn));
                issues.push(Finding::new(
                    FindingCode::BoardClearance,
                    format!(
                        "Clearance < {} mm: {} / {} ({})",
                        mm(need.mm),
                        self.label(a),
                        self.label(b),
                        need.cite()
                    ),
                    vec![a.object, b.object],
                ));
            }
        }
        Ok(pairs)
    }
}

/// The parts the schematic has that the board does not place.
pub(super) fn unplaced(project: &Project) -> Vec<Finding> {
    sync_preview(project)
        .into_iter()
        .map(|id| {
            let reference = project
                .instance(id)
                .map(|i| i.designator.as_str())
                .unwrap_or("component");
            Finding::new(
                FindingCode::BoardUnplaced,
                format!("{reference} is not placed on the board"),
                vec![ObjectRef::Instance { id }],
            )
        })
        .collect()
}

/// Whether `a` and `b` are two pads of one placed footprint: the
/// footprint exception. The footprint fixes their spacing (a QFN's pitch
/// cannot meet a power class's spacing to its own exposed pad), so they
/// are held to the profile's pad-to-pad clearance (and its overrides) and
/// to an isolation clearance between their classes
/// (`ClassClearance::isolation`), but not to the classes' fab spacing.
fn within_footprint(a: &Copper<'_>, b: &Copper<'_>) -> bool {
    matches!(
        (a.object, b.object),
        (ObjectRef::Pad { instance: x, .. }, ObjectRef::Pad { instance: y, .. }) if x == y
    )
}

/// The nets whose classes set the fab spacing between `a` and `b`: none
/// within one footprint ([`within_footprint`]).
fn class_nets(a: &Copper<'_>, b: &Copper<'_>) -> (Option<Id>, Option<Id>) {
    if within_footprint(a, b) {
        (None, None)
    } else {
        (a.net(), b.net())
    }
}

/// The finding that says the copper checks stopped at their pair limit, so
/// the geometry after it is unchecked. It names the board, so it locates.
pub(crate) fn limit_finding() -> Finding {
    Finding::new(
        FindingCode::BoardCheckLimit,
        "Board checks incomplete: copper past the check limit is unchecked".into(),
        vec![ObjectRef::Board],
    )
}

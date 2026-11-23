//! The plated-hole annular ring and the pad hole checks: footprint pad
//! holes (plated and unplated, slots and pad vias included) measured
//! against other nets' copper and against other holes.
//!
//! - `board.pth_annular_ring`: every plated hole of a pad, and every
//!   plated mounting hole, keeps at least the profile's `min PTH annular
//!   ring` of copper around the finished hole. A pad has one outline on
//!   every layer it has copper on, so one ring holds for each of them.
//! - `board.npth_annular_ring`: an unplated hole in a pad with copper
//!   keeps the profile's `min NPTH pad annular ring` the same way.
//! - `board.pad_hole_clearance`: a pad hole passes through every copper
//!   layer, so every track, pad and via within the hole kind's clearance
//!   (`clearance_<copper>_pth_hole` or `_npth_hole`) is reported, except
//!   copper the copper graph joins to the hole's own pad (its island).
//!   Fill copper keeps these clearances by construction ([`super::fill`]).
//! - `board.pad_hole_spacing`: a pad hole against every other hole (other
//!   pad holes, via drills and mounting holes) by the hole-kind pair rule,
//!   whatever their nets.
//!
//! Every limit is the project's ([`BoardRules`]); pairs are found through
//! a bounds grid reaching as far as the largest clearance.
use super::connectivity::CopperGraph;
use super::copper::{check_reach, limit_finding};
use super::grid::BoundsGrid;
use super::limits::{BoardRules, mm, mm_against};
use super::physical::{self, HoleOwner};
use super::{BoardCopper, CopperRing, EPS, PlacedPad, check_pair_budget, pad_labels, placed_pads};
use crate::diagnostics::FindingCode;
use crate::diagnostics::{Finding, ObjectRef};
use crate::geometry::Shape;
use crate::ids::{Id, PadId};
use crate::library::PadCopper;
use crate::library::names::capitalised;
use crate::model::Project;
use crate::rules::{CopperObject, RuleKey};
use std::collections::{BTreeMap, BTreeSet};

/// The chord tolerance a slot's outline is measured at for its annular
/// ring, mm: far below what a fab resolves, so a ring is at most this much
/// above the exact value. Round holes are measured exactly.
const RING_TOLERANCE_MM: f64 = 1e-4;

/// The least copper between a plated hole and its pad's outline, both in
/// board coordinates; 0 when the hole reaches past the pad. Exact for a
/// round hole; a slot is measured at its outline's vertices, which is
/// exact on convex pads.
pub fn annular_ring(pad: &Shape, hole: &Shape) -> f64 {
    let inside = |p| pad.distance(p, true) <= 0.0;
    if let Shape::Circle { center, radius } = *hole {
        return if inside(center) {
            (pad.distance(center, false) - radius).max(0.0)
        } else {
            0.0
        };
    }
    let ring = hole.flatten(RING_TOLERANCE_MM);
    if ring.is_empty() || !ring.iter().all(|p| inside(*p)) {
        return 0.0;
    }
    ring.iter()
        .map(|p| pad.distance(*p, false))
        .fold(f64::INFINITY, f64::min)
        .max(0.0)
}

/// One drilled hole the checks compare.
#[derive(Debug)]
struct Hole {
    ring: CopperRing,
    kind: CopperObject,
    /// The pad in `placed` it belongs to; `None` for a via or mounting hole.
    pad: Option<usize>,
    object: ObjectRef,
    /// How a message names it: "J1 pad 5", "a via on GND".
    name: String,
    /// What it is within its pad, for a message comparing two holes of one
    /// pad: "hole", "slot" or "via-in-pad hole".
    what: &'static str,
}

/// The annular ring, pad hole clearance and hole spacing findings.
pub(super) fn findings(
    copper: &BoardCopper,
    project: &Project,
    graph: &super::GraphSource<'_>,
) -> Vec<Finding> {
    let Some(cx) = PadHoleCx::new(copper, project, None) else {
        return Vec::new();
    };
    let mut issues = cx.rings();
    if cx.pad_holes == 0 {
        return issues;
    }
    let shape_grid = BoundsGrid::new(cx.shape_bounds.iter().copied(), cx.reach);
    let hole_grid = BoundsGrid::new(cx.hole_bounds.iter().copied(), cx.reach);
    let joined = Joined::of(graph.get());
    let (mut near_shapes, mut near_holes) = (Vec::new(), Vec::new());
    let mut pairs = 0usize;
    let budget = cx.budget();
    for i in 0..cx.pad_holes {
        let Some(bounds) = cx.hole_bounds.get(i).copied().flatten() else {
            continue;
        };
        shape_grid.near(&bounds, cx.reach, &mut near_shapes);
        hole_grid.near(&bounds, cx.reach, &mut near_holes);
        match cx.hole(
            i,
            &joined,
            &near_shapes,
            &near_holes,
            budget.saturating_sub(pairs),
            &mut issues,
        ) {
            Ok(n) => pairs += n,
            Err(()) => {
                issues.push(limit_finding());
                return issues;
            }
        }
    }
    issues
}

/// [`findings`] as items: the annular rings, then each pad hole's
/// findings, made from `last` when given: an earlier state's items, what
/// changed since and the nets whose islands may have changed. A pad hole
/// is checked again when a changed shape is near it or its pad is on one
/// of those nets. `graph` gives a copper graph holding at least the nets
/// it is asked for (the nets of the pads checked again, for the copper
/// joined to each). `None` when the check would stop at its pair limit.
pub fn items(
    copper: &BoardCopper,
    project: &Project,
    graph: &dyn Fn(&BTreeSet<Id>) -> Option<std::sync::Arc<CopperGraph>>,
    last: Option<(
        &super::Items,
        &PadHoleIndex,
        &super::CopperChanges,
        &BTreeSet<Id>,
    )>,
) -> Option<(super::Items, PadHoleIndex)> {
    // A change that reaches no pad hole keeps every item, without the
    // pad holes being made again.
    if let Some((old, index, changes, nets)) = last
        && !index.reached(changes, nets)
        && old.pairs() <= index.budget(copper)
    {
        return Some((old.clone(), index.clone()));
    }
    let part = last.and_then(|(_, index, _, _)| index.part.clone());
    made(
        copper,
        project,
        graph,
        last.map(|(i, _, c, n)| (i, c, n)),
        part,
    )
}

/// Where a board's pad holes are and which nets their pads carry, which a
/// change of tracks and vias alone leaves as they are: what decides
/// whether a change reaches a pad hole check.
#[derive(Clone, Debug, Default)]
pub struct PadHoleIndex {
    /// Each pad hole's bounds and its pad's net.
    holes: Vec<(Option<crate::geometry::Bounds>, Option<Id>)>,
    reach: f64,
    mounts: usize,
    /// The pad holes and what they are made of, kept for a next state.
    part: Option<std::sync::Arc<PadPart>>,
}

impl PadHoleIndex {
    /// Whether `changes`, whose reached nets are `nets`, reach a pad hole:
    /// a changed track or via near it, or its pad's island on such a net.
    fn reached(&self, changes: &super::CopperChanges, nets: &BTreeSet<Id>) -> bool {
        self.holes.iter().any(|(bounds, net)| {
            bounds.is_some_and(|b| changes.near(&b, self.reach))
                || net.is_some_and(|n| nets.contains(&n))
        })
    }

    /// The pair budget of the check on `copper` ([`PadHoleCx::budget`]).
    fn budget(&self, copper: &BoardCopper) -> usize {
        let vias = copper.vias().len();
        let shapes = copper.lands().len() + vias + copper.tracks().len();
        let holes = if self.holes.is_empty() {
            0
        } else {
            self.holes.len() + vias + self.mounts
        };
        check_pair_budget(shapes.saturating_add(holes))
    }
}

/// [`items`], making the pad holes whatever reached them.
fn made(
    copper: &BoardCopper,
    project: &Project,
    graph: &dyn Fn(&BTreeSet<Id>) -> Option<std::sync::Arc<CopperGraph>>,
    last: Option<(&super::Items, &super::CopperChanges, &BTreeSet<Id>)>,
    part: Option<std::sync::Arc<PadPart>>,
) -> Option<(super::Items, PadHoleIndex)> {
    use super::ItemKey;
    let Some(cx) = PadHoleCx::new(copper, project, part) else {
        let items = super::Items::made(&[ItemKey::Rings], None, |_| Some((Vec::new(), 0)))?;
        return Some((items, PadHoleIndex::default()));
    };
    let keys: Vec<ItemKey> = std::iter::once(ItemKey::Rings)
        .chain((0..cx.pad_holes).map(ItemKey::PadHole))
        .collect();
    // The pad holes to check again, and the nets of their pads.
    let net_of = |i: usize| {
        cx.hole_at(i)
            .and_then(|h| h.pad)
            .and_then(|k| cx.part.placed.get(k))
            .and_then(|p| p.net)
    };
    let remake: Vec<bool> = keys
        .iter()
        .enumerate()
        .map(|(k, key)| match (last, k.checked_sub(1)) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some((_, changes, nets)), Some(i)) => {
                changes.again(key, cx.hole_bounds.get(i), cx.reach)
                    || net_of(i).is_some_and(|n| nets.contains(&n))
            }
        })
        .collect();
    let nets: BTreeSet<Id> = (0..cx.pad_holes)
        .filter(|i| remake.get(i + 1).copied().unwrap_or(false))
        .filter_map(net_of)
        .collect();
    let grids = last.is_none().then(|| {
        (
            BoundsGrid::new(cx.shape_bounds.iter().copied(), cx.reach),
            BoundsGrid::new(cx.hole_bounds.iter().copied(), cx.reach),
        )
    });
    let joined = if remake.iter().skip(1).any(|r| *r) {
        Some(Joined::of(&*graph(&nets)?))
    } else {
        None
    };
    let (mut near_shapes, mut near_holes) = (Vec::new(), Vec::new());
    let again = |k: usize, _: &ItemKey| remake.get(k).copied().unwrap_or(true);
    let last = last.map(|(items, _, _)| (items, &again as &dyn Fn(usize, &ItemKey) -> bool));
    let items = super::Items::made(&keys, last, |k| {
        let Some(i) = k.checked_sub(1) else {
            return Some((cx.rings(), 0));
        };
        let mut out = Vec::new();
        let Some(b) = cx.hole_bounds.get(i).copied().flatten() else {
            return Some((out, 0));
        };
        match &grids {
            Some((shapes, holes)) => {
                shapes.near(&b, cx.reach, &mut near_shapes);
                holes.near(&b, cx.reach, &mut near_holes);
            }
            None => {
                super::copper::scan(&cx.shape_bounds, &b, cx.reach, &mut near_shapes);
                super::copper::scan(&cx.hole_bounds, &b, cx.reach, &mut near_holes);
            }
        }
        let pairs = cx
            .hole(
                i,
                joined.as_ref()?,
                &near_shapes,
                &near_holes,
                usize::MAX,
                &mut out,
            )
            .ok()?;
        Some((out, pairs))
    })?;
    let index = PadHoleIndex {
        holes: (0..cx.pad_holes)
            .map(|i| (cx.hole_bounds.get(i).copied().flatten(), net_of(i)))
            .collect(),
        reach: cx.reach,
        mounts: cx.part.mounts.len(),
        part: Some(cx.part.clone()),
    };
    (items.pairs() <= cx.budget()).then_some((items, index))
}

/// What a pad hole check reads that a change of tracks and vias alone
/// leaves as it was: the placed pads, their labels, the pad holes and the
/// mounting holes. A next state's check keeps it ([`PadHoleIndex`]).
#[derive(Debug)]
pub struct PadPart {
    placed: Vec<PlacedPad>,
    labels: BTreeMap<(Id, PadId), String>,
    pad_holes: Vec<Hole>,
    mounts: Vec<Hole>,
}

impl PadPart {
    /// `None` when the board has no pad hole and no plated mounting hole.
    fn of(project: &Project) -> Option<Self> {
        let placed = placed_pads(project);
        if placed.iter().all(|p| p.holes().is_empty())
            && !project.board().holes.iter().any(|h| h.plated)
        {
            return None;
        }
        let labels = pad_labels(project);
        // The board's holes ([`physical::holes`]), pad holes first, then
        // via drills and mounting holes (plated board slots keep the
        // profile's slot rules, not these).
        let index: BTreeMap<(Id, PadId), usize> = placed
            .iter()
            .enumerate()
            .map(|(k, p)| ((p.instance, p.pad.id), k))
            .collect();
        let all = physical::holes_of(project, &placed);
        let of = |h: &physical::Hole| {
            let pad = match h.owner {
                HoleOwner::Pad { instance, pad, .. } => index.get(&(instance, pad)).copied(),
                _ => None,
            };
            hole_of(h, pad, |k| {
                placed.get(k).map(|p| pad_name(project, &labels, p))
            })
        };
        let pad_holes: Vec<Hole> = all
            .iter()
            .filter(|h| matches!(h.owner, HoleOwner::Pad { .. }))
            .map(of)
            .collect();
        let mounts: Vec<Hole> = all
            .iter()
            .filter(|h| {
                matches!(
                    h.owner,
                    HoleOwner::MountingHole(_) | HoleOwner::BoardSlot(_)
                )
            })
            .map(of)
            .collect();
        Some(Self {
            placed,
            labels,
            pad_holes,
            mounts,
        })
    }
}

/// The check's hole for `h`, of the placed pad at `pad` (named by `name`).
fn hole_of(h: &physical::Hole, pad: Option<usize>, name: impl Fn(usize) -> Option<String>) -> Hole {
    Hole {
        ring: h.feature.ring(),
        kind: match h.owner {
            HoleOwner::Via(_) => CopperObject::ViaHole,
            _ => CopperObject::hole(h.plated),
        },
        pad,
        object: h.object(),
        name: match (pad.and_then(&name), h.owner) {
            (Some(n), _) => n,
            (None, _) => h.name.clone(),
        },
        what: h.what(),
    }
}

/// One state's pad hole checks: the placed pads, the board's holes (pad
/// holes, then via drills and mounting holes), the copper they are
/// measured against and the rules.
struct PadHoleCx<'a> {
    project: &'a Project,
    part: std::sync::Arc<PadPart>,
    via_holes: Vec<Hole>,
    limits: BoardRules<'a>,
    hole_bounds: Vec<Option<crate::geometry::Bounds>>,
    pad_holes: usize,
    shapes: Vec<super::via::Drilled<'a>>,
    shape_bounds: Vec<Option<crate::geometry::Bounds>>,
    reach: f64,
    edges: super::board_edge::BoardEdges,
}

impl<'a> PadHoleCx<'a> {
    /// `None` when the board has no pad hole and no plated mounting hole.
    /// `part`, when given, is an earlier state's, which the caller knows
    /// is this state's too.
    fn new(
        copper: &'a BoardCopper,
        project: &'a Project,
        part: Option<std::sync::Arc<PadPart>>,
    ) -> Option<Self> {
        let part = match part {
            Some(part) => part,
            None => std::sync::Arc::new(PadPart::of(project)?),
        };
        let limits = BoardRules::of(project);
        let pad_holes = part.pad_holes.len();
        let via_holes: Vec<Hole> = if pad_holes > 0 {
            copper
                .vias()
                .iter()
                .map(|v| hole_of(&physical::via_hole(project, v), None, |_| None))
                .collect()
        } else {
            Vec::new()
        };
        let shapes = super::via::drilled_copper(copper);
        let mut cx = Self {
            project,
            part,
            via_holes,
            hole_bounds: Vec::new(),
            pad_holes,
            shape_bounds: shapes.iter().map(|s| s.ring.bounds).collect(),
            shapes,
            reach: check_reach(&limits),
            limits,
            edges: super::board_edge::BoardEdges::of(project),
        };
        cx.hole_bounds = (0..cx.holes())
            .map(|i| cx.hole_at(i).and_then(|h| h.ring.bounds))
            .collect();
        Some(cx)
    }

    /// The number of holes: pad holes, then (when there are any) via
    /// drills and mounting holes.
    fn holes(&self) -> usize {
        if self.pad_holes == 0 {
            0
        } else {
            self.pad_holes + self.via_holes.len() + self.part.mounts.len()
        }
    }

    /// Hole `i` in that order.
    fn hole_at(&self, i: usize) -> Option<&Hole> {
        if let Some(h) = self.part.pad_holes.get(i) {
            return Some(h);
        }
        if self.pad_holes == 0 {
            return None;
        }
        let i = i - self.pad_holes;
        self.via_holes
            .get(i)
            .or_else(|| self.part.mounts.get(i.checked_sub(self.via_holes.len())?))
    }

    fn budget(&self) -> usize {
        check_pair_budget(self.shapes.len().saturating_add(self.holes()))
    }

    fn rings(&self) -> Vec<Finding> {
        let name = |pad: &PlacedPad| pad_name(self.project, &self.part.labels, pad);
        rings(self.project, &self.part.placed, &self.limits, &name)
    }

    /// Pad hole `i` against the copper shapes `near_shapes` (but those
    /// `joined` to its pad) and the holes `near_holes`, both ascending:
    /// the pairs compared, or `Err` once more than `budget` would be.
    fn hole(
        &self,
        i: usize,
        joined: &Joined,
        near_shapes: &[usize],
        near_holes: &[usize],
        budget: usize,
        issues: &mut Vec<Finding>,
    ) -> std::result::Result<usize, ()> {
        let (project, limits) = (self.project, &self.limits);
        let Some(hole) = self.hole_at(i) else {
            return Ok(0);
        };
        let Some(pad) = hole.pad.and_then(|k| self.part.placed.get(k)) else {
            return Ok(0);
        };
        if hole.ring.bounds.is_none() {
            return Ok(0);
        }
        // Every pad hole keeps the hole kind's board-edge clearance inside
        // the board, whether or not its pad has copper (a bare NPTH).
        if let Some((text, needs)) =
            super::via::edge_text(&self.edges, &hole.ring, limits, hole.kind)
        {
            issues.push(
                Finding::new(
                    FindingCode::BoardEdgeClearance,
                    format!("{}: {} {text}", hole.name, hole.what),
                    vec![hole.object],
                )
                .requiring(needs),
            );
        }
        let mut pairs = 0usize;
        let net = pad.net.filter(|_| hole.kind == CopperObject::PthHole);
        let own = joined.island(pad);
        let own_pad = ObjectRef::Pad {
            instance: pad.instance,
            pad: pad.pad.id,
        };
        for shape in near_shapes.iter().filter_map(|k| self.shapes.get(*k)) {
            if shape.object == own_pad || joined.holds(own, &shape.object) {
                continue;
            }
            pairs += 1;
            if pairs > budget {
                return Err(());
            }
            let need = limits.clearance(hole.kind, net, shape.kind, shape.net);
            if !hole.ring.closer_than(shape.ring, need) {
                continue;
            }
            let need = limits.clearance_need(hole.kind, net, shape.kind, shape.net);
            let what = shape.words(project);
            let gap = hole.ring.gap(shape.ring, need.mm + 1.0);
            let how = if gap <= EPS {
                format!("overlaps {what}")
            } else {
                format!("is {} mm from {what}", mm_against(gap, need.mm))
            };
            issues.push(
                Finding::new(
                    FindingCode::BoardPadHoleClearance,
                    format!("{}: hole {how}; {}", hole.name, need.text()),
                    vec![hole.object, shape.object],
                )
                .requiring(need.at_most(gap)),
            );
        }
        for other in near_holes
            .iter()
            .filter(|j| **j != i && (**j > i || **j >= self.pad_holes))
            .filter_map(|j| self.hole_at(*j))
        {
            pairs += 1;
            if pairs > budget {
                return Err(());
            }
            let spacing = limits.clearance(hole.kind, None, other.kind, None);
            if !hole.ring.closer_than(&other.ring, spacing) {
                continue;
            }
            let need = limits.clearance_need(hole.kind, None, other.kind, None);
            let gap = hole.ring.gap(&other.ring, need.mm + 1.0);
            let how = if gap <= EPS {
                "overlap".to_owned()
            } else {
                format!("are {} mm apart", mm_against(gap, need.mm))
            };
            // Two holes of one pad (its slot and a via-in-pad hole) are
            // named as that, not as the pad against itself.
            let (text, objects) = if hole.pad.is_some() && hole.pad == other.pad {
                (
                    format!(
                        "{}: its {} and {} {how}; {}",
                        hole.name,
                        hole.what,
                        other.what,
                        need.text()
                    ),
                    vec![hole.object, other.object],
                )
            } else {
                (
                    format!(
                        "{} and {}: holes {how}; {}",
                        hole.name,
                        other.name,
                        need.text()
                    ),
                    vec![hole.object, other.object],
                )
            };
            issues.push(
                Finding::new(FindingCode::BoardPadHoleSpacing, text, objects)
                    .requiring(need.at_most(gap)),
            );
        }
        Ok(pairs)
    }
}

/// How a pad hole message names a pad: "J1 pad 5".
fn pad_name(project: &Project, labels: &BTreeMap<(Id, PadId), String>, pad: &PlacedPad) -> String {
    let part = super::outline::part_name(project, pad.instance);
    let number = labels
        .get(&(pad.instance, pad.pad.id))
        .cloned()
        .unwrap_or_else(|| pad.pad.id.to_string());
    format!("{part} pad {number}")
}

/// `board.pth_annular_ring` for every plated pad hole and plated
/// mounting hole, and `board.npth_annular_ring` for every unplated hole
/// in a pad with copper.
fn rings(
    project: &Project,
    placed: &[PlacedPad],
    limits: &BoardRules<'_>,
    name: &dyn Fn(&PlacedPad) -> String,
) -> Vec<Finding> {
    let need = limits.need(RuleKey::PthAnnularRingMin);
    let npth_need = limits.need(RuleKey::NpthAnnularRingMin);
    let mut issues = Vec::new();
    let least =
        |need: &super::limits::Need| format!("needs at least {} mm ({})", mm(need.mm), need.cite());
    // A recommended limit the fabricator relaxes under another choice
    // says so, naming the choice.
    let text = |ring: f64| {
        let looser =
            super::limits::looser_choice(limits.project(), RuleKey::PthAnnularRingMin, ring)
                .map(|c| format!("; {c}"))
                .unwrap_or_default();
        format!(
            "annular ring {} mm; {}{looser}",
            mm_against(ring, need.mm),
            least(&need)
        )
    };
    for pad in placed {
        if pad.pad.kind.copper() == PadCopper::None {
            continue;
        }
        let outline = pad.outline();
        let holes = pad.holes();
        let ring_of = |plated: bool| {
            holes
                .iter()
                .filter(|h| h.plated == plated)
                .map(|h| annular_ring(&outline, &h.shape))
                .fold(f64::INFINITY, f64::min)
        };
        let object = ObjectRef::Pad {
            instance: pad.instance,
            pad: pad.pad.id,
        };
        let ring = ring_of(true);
        if ring.is_finite() && ring < need.mm - EPS {
            issues.push(
                Finding::new(
                    FindingCode::BoardPthAnnularRing,
                    format!("{}: {}", name(pad), text(ring)),
                    vec![object],
                )
                .requiring(need.at_most(ring)),
            );
        }
        let ring = ring_of(false);
        if ring.is_finite() && ring < npth_need.mm - EPS {
            issues.push(
                Finding::new(
                    FindingCode::BoardNpthAnnularRing,
                    format!(
                        "{}: annular ring {} mm round the unplated hole; {}",
                        name(pad),
                        mm_against(ring, npth_need.mm),
                        least(&npth_need)
                    ),
                    vec![object],
                )
                .requiring(npth_need.at_most(ring)),
            );
        }
    }
    for mount in project.board().holes.iter().filter(|h| h.plated) {
        let ring = mount
            .pad_diameter
            .map_or(0.0, |d| ((d - mount.drill) / 2.0).max(0.0));
        if ring < need.mm - EPS {
            issues.push(
                Finding::new(
                    FindingCode::BoardPthAnnularRing,
                    format!(
                        "{}: {}",
                        capitalised(&physical::mounting_hole_name(true, mount.at)),
                        text(ring)
                    ),
                    vec![ObjectRef::BoardFeature {
                        feature: mount.id.0,
                    }],
                )
                .requiring(need.at_most(ring)),
            );
        }
    }
    issues
}

/// The copper the copper graph joins to each connected pad: its island's
/// pads, tracks and vias.
struct Joined {
    /// The island of each connected pad.
    island: BTreeMap<(Id, PadId), usize>,
    pads: Vec<BTreeSet<(Id, PadId)>>,
    tracks: Vec<BTreeSet<Id>>,
    vias: Vec<BTreeSet<Id>>,
}

impl Joined {
    fn of(graph: &CopperGraph) -> Self {
        let key = |k: &usize| graph.pads().get(*k).map(|p| (p.instance, p.pad));
        let island = (0..graph.pads().len())
            .filter_map(|k| Some((key(&k)?, graph.pad_island(k)?)))
            .collect();
        let islands = graph.islands();
        Self {
            island,
            pads: islands
                .iter()
                .map(|i| i.pads.iter().filter_map(key).collect())
                .collect(),
            tracks: islands
                .iter()
                .map(|i| i.tracks.iter().copied().collect())
                .collect(),
            vias: islands
                .iter()
                .map(|i| i.vias.iter().copied().collect())
                .collect(),
        }
    }

    /// The island of `pad`, if the graph holds it (a connected pad).
    fn island(&self, pad: &PlacedPad) -> Option<usize> {
        self.island.get(&(pad.instance, pad.pad.id)).copied()
    }

    /// Whether `object` is copper of `island`.
    fn holds(&self, island: Option<usize>, object: &ObjectRef) -> bool {
        let Some(i) = island else {
            return false;
        };
        match *object {
            ObjectRef::Pad { instance, pad } => self
                .pads
                .get(i)
                .is_some_and(|s| s.contains(&(instance, pad))),
            ObjectRef::Track { id } => self.tracks.get(i).is_some_and(|s| s.contains(&id)),
            ObjectRef::Via { id } => self.vias.get(i).is_some_and(|s| s.contains(&id)),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;

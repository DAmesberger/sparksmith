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
use crate::diagnostics::{Finding, ObjectRef};
use crate::geometry::Shape;
use crate::ids::{Id, PadId};
use crate::library::PadCopper;
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
    graph: &CopperGraph,
) -> Vec<Finding> {
    let placed = placed_pads(project);
    if placed.iter().all(|p| p.holes().is_empty())
        && !project.board().holes.iter().any(|h| h.plated)
    {
        return Vec::new();
    }
    let limits = BoardRules::of(project);
    let labels = pad_labels(project);
    let name = |pad: &PlacedPad| {
        let part = super::outline::part_name(project, pad.instance);
        let number = labels
            .get(&(pad.instance, pad.pad.id))
            .cloned()
            .unwrap_or_else(|| pad.pad.id.to_string());
        format!("{part} pad {number}")
    };
    let mut issues = rings(project, &placed, &limits, &name);

    // The board's holes ([`physical::holes`]), pad holes first, then via
    // drills and mounting holes (plated board slots keep the profile's
    // slot rules, not these).
    let index: BTreeMap<(Id, PadId), usize> = placed
        .iter()
        .enumerate()
        .map(|(k, p)| ((p.instance, p.pad.id), k))
        .collect();
    let all = physical::holes_of(project, &placed);
    let hole_of = |h: &physical::Hole| {
        let pad = match h.owner {
            HoleOwner::Pad { instance, pad, .. } => index.get(&(instance, pad)).copied(),
            _ => None,
        };
        Hole {
            ring: h.feature.ring(),
            kind: match h.owner {
                HoleOwner::Via(_) => CopperObject::ViaHole,
                _ => CopperObject::hole(h.plated),
            },
            pad,
            object: h.object(),
            name: match (pad.and_then(|k| placed.get(k)), h.owner) {
                (Some(p), _) => name(p),
                (None, HoleOwner::MountingHole(_)) => "a mounting hole".into(),
                (None, _) => h.name.clone(),
            },
            what: h.what(),
        }
    };
    let mut holes: Vec<Hole> = all
        .iter()
        .filter(|h| matches!(h.owner, HoleOwner::Pad { .. }))
        .map(hole_of)
        .collect();
    let pad_holes = holes.len();
    if pad_holes == 0 {
        return issues;
    }
    holes.extend(
        all.iter()
            .filter(|h| matches!(h.owner, HoleOwner::Via(_) | HoleOwner::MountingHole(_)))
            .map(hole_of),
    );

    let track_rings: Vec<CopperRing> = project.tracks.iter().map(CopperRing::of_track).collect();
    let shapes = super::via::drilled_copper(copper, project, &track_rings);
    let reach = check_reach(&limits);
    let shape_grid = BoundsGrid::new(shapes.iter().map(|s| s.ring.bounds), reach);
    let hole_grid = BoundsGrid::new(holes.iter().map(|h| h.ring.bounds), reach);
    let joined = Joined::of(graph);
    let mut near = Vec::new();
    let mut pairs = 0usize;
    let budget = check_pair_budget(shapes.len().saturating_add(holes.len()));
    for (i, hole) in holes.iter().take(pad_holes).enumerate() {
        let Some(pad) = hole.pad.and_then(|k| placed.get(k)) else {
            continue;
        };
        let Some(bounds) = hole.ring.bounds else {
            continue;
        };
        let net = pad.net.filter(|_| hole.kind == CopperObject::PthHole);
        let own = joined.island(pad);
        shape_grid.near(&bounds, reach, &mut near);
        for shape in near.iter().filter_map(|k| shapes.get(*k)) {
            if shape.object == hole.object || joined.holds(own, &shape.object) {
                continue;
            }
            pairs += 1;
            if pairs > budget {
                issues.push(limit_finding());
                return issues;
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
            issues.push(Finding::new(
                "board.pad_hole_clearance",
                format!("{}: hole {how}; {}", hole.name, need.text()),
                vec![hole.object, shape.object],
            ));
        }
        hole_grid.near(&bounds, reach, &mut near);
        for other in near
            .iter()
            .filter(|j| **j != i && (**j > i || **j >= pad_holes))
            .filter_map(|j| holes.get(*j))
        {
            pairs += 1;
            if pairs > budget {
                issues.push(limit_finding());
                return issues;
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
                    vec![hole.object],
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
            issues.push(Finding::new("board.pad_hole_spacing", text, objects));
        }
    }
    issues
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
            issues.push(Finding::new(
                "board.pth_annular_ring",
                format!("{}: {}", name(pad), text(ring)),
                vec![object],
            ));
        }
        let ring = ring_of(false);
        if ring.is_finite() && ring < npth_need.mm - EPS {
            issues.push(Finding::new(
                "board.npth_annular_ring",
                format!(
                    "{}: annular ring {} mm round the unplated hole; {}",
                    name(pad),
                    mm_against(ring, npth_need.mm),
                    least(&npth_need)
                ),
                vec![object],
            ));
        }
    }
    for mount in project.board().holes.iter().filter(|h| h.plated) {
        let ring = mount
            .pad_diameter
            .map_or(0.0, |d| ((d - mount.drill) / 2.0).max(0.0));
        if ring < need.mm - EPS {
            issues.push(Finding::new(
                "board.pth_annular_ring",
                format!("A plated mounting hole: {}", text(ring)),
                vec![ObjectRef::Board],
            ));
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

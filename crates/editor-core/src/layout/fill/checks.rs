//! Fill and keepout checks, and the connectivity fills add to the board.
//!
//! Stored fill copper is plotted as it is stored, so it is audited against
//! the board ([`findings`]): a current key says which board a worker
//! computed for, not that the regions it returned are right. Fresh copper
//! keeps every clearance, the board edge and keepouts; stale copper (whose
//! inputs moved on and that has not been refilled yet) carries one
//! `fill.stale` finding and is reported only where it overlaps other
//! copper, a hole, a keepout or leaves the board. Gaps that merely miss a
//! changed clearance (a profile switch, a class edit) are what the refill
//! fixes, so they are not listed one by one.
//!
//! Only fresh copper joins board copper ([`fresh`], read by the copper
//! graph, which joins its islands by the one joining rule): an airwire
//! shows as soon as a joined pad or via moves off the pour, not after the
//! refill lands. The graph also reports islands that join nothing
//! (`fill.island_unconnected`).
use super::FILL_CHORD_MM;
use super::compute::Ctx;
use super::geom::{self, Region, RegionIndex};
use crate::diagnostics::FindingCode;
use crate::diagnostics::{Finding, ObjectRef};
use crate::geometry::{Point, polygon_contains, polygons_overlap};
use crate::ids::Id;
use crate::layout::physical::{self, HoleFeature, HoleOwner};
use crate::model::fill::FilledCopper;
use crate::model::{CopperLayer, Fill, Project};
use crate::rules::CopperObject;

/// How a fill is named in messages: its name, else its net and layer.
pub fn fill_label(project: &Project, fill: &Fill) -> String {
    if !fill.name.is_empty() {
        return format!("fill {}", fill.name);
    }
    let net = fill.net.map_or("no net", |n| project.net_name(n));
    format!("{net} fill on {}", fill.layer.name())
}

fn regions_of(fill: &Fill) -> Vec<Region> {
    fill.computed
        .iter()
        .flat_map(|c| &c.regions)
        .map(|r| {
            let mut region = vec![r.outer.clone()];
            region.extend(r.holes.iter().cloned());
            region
        })
        .collect()
}

/// A fill whose stored copper is current, with its net.
pub(crate) struct Fresh<'a> {
    pub id: Id,
    pub net: Id,
    pub layer: CopperLayer,
    pub copper: &'a FilledCopper,
}

/// Every fill with a net whose stored copper is current. Only these join
/// board copper (in the copper graph): stale copper joins nothing until it
/// is refilled.
pub(crate) fn fresh(project: &Project) -> Vec<Fresh<'_>> {
    fresh_on(project, None)
}

/// [`fresh`] for the fills of `nets` alone (every net: `None`).
pub(crate) fn fresh_on<'a>(project: &'a Project, nets: Option<&[Id]>) -> Vec<Fresh<'a>> {
    let mut out = Vec::new();
    if project.fills.iter().all(|f| f.computed.is_none()) {
        return out;
    }
    let mut ctx = Ctx::new(project);
    for fill in &project.fills {
        let (Some(net), Some(copper)) = (fill.net, &fill.computed) else {
            continue;
        };
        if nets.is_some_and(|n| !n.contains(&net)) {
            continue;
        }
        if ctx.key(fill.id).ok().as_ref() == Some(&copper.key) {
            out.push(Fresh {
                id: fill.id,
                net,
                layer: fill.layer,
                copper,
            });
        }
    }
    out
}

/// Most rows one fill's audit lists; past it one
/// [`CLEARANCE_UNLISTED`] finding counts the rest. The audit itself always
/// measures every object.
pub const MAX_FILL_ROWS: usize = 256;

/// A fill's audit found more violations than it lists ([`MAX_FILL_ROWS`]):
/// an error that may not be waived, since a waiver could not say which
/// copper it accepts.
pub const CLEARANCE_UNLISTED: &str =
    crate::diagnostics::FindingCode::FillClearanceUnlisted.as_str();

/// Stored fill copper beyond the board outline, in a cutout or slot, or
/// closer to an edge than the fill-to-board-edge clearance.
pub const BOARD_EDGE: &str = crate::diagnostics::FindingCode::FillBoardEdge.as_str();

/// Stored fill copper inside a keepout that forbids fills on its layer.
pub const KEEPOUT_FILL: &str = crate::diagnostics::FindingCode::KeepoutFill.as_str();

/// How far, mm, a measured gap may fall short of the rule before the audit
/// reports it: the chord tolerance the fill computation flattens curves at
/// ([`FILL_CHORD_MM`]; its grown obstacles' round joins lie that much
/// inside the true arcs), and 1 µm for the polygon arithmetic. A gap short
/// by more is copper the computation would not have made.
pub const AUDIT_SLACK_MM: f64 = FILL_CHORD_MM + 0.001;

/// Fill and keepout findings.
///
/// Every fill's stored copper is audited against the board it will be
/// plotted on, whatever its key says (a current key says which board a
/// worker computed for, not that the regions it returned are right):
///
/// - `fill.clearance`: other copper (any land without a net, other nets'
///   pads, vias and tracks, other fills) and every hole it does not own,
///   on its layer. A fresh fill's copper keeps the clearance rule of each
///   pair (measured, within [`AUDIT_SLACK_MM`]); stale copper is reported
///   only where it overlaps, since its refill cuts the gaps the rules now
///   ask for.
/// - [`BOARD_EDGE`]: copper outside the board outline or in a cutout or
///   slot; for fresh copper also closer to an edge than the board-edge
///   rule.
/// - [`KEEPOUT_FILL`]: copper in a keepout that forbids fills.
pub fn findings(project: &Project) -> Vec<Finding> {
    let mut out = Vec::new();
    if project.fills.is_empty()
        && project.keepouts.is_empty()
        && super::super::placed_keepouts(project).is_empty()
    {
        return out;
    }
    let mut ctx = Ctx::new(project);
    let audit = project
        .fills
        .iter()
        .any(|f| f.computed.as_ref().is_some_and(|c| !c.regions.is_empty()))
        .then(|| Audit::new(&ctx));
    for fill in &project.fills {
        let me = ObjectRef::Fill { id: fill.id };
        let label = fill_label(project, fill);
        if let Some(net) = fill.net
            && project.net(net).is_none()
        {
            out.push(Finding::new(
                FindingCode::FillNetMissing,
                format!("{label} names a net that no longer exists"),
                vec![me],
            ));
        }
        let Some(copper) = &fill.computed else {
            out.push(Finding::new(
                FindingCode::FillStale,
                format!("{label} is not filled yet"),
                vec![me],
            ));
            continue;
        };
        // Only stored copper is compared with the key of its inputs.
        let key = ctx.key(fill.id).ok();
        let stale = key.as_ref() != Some(&copper.key);
        if stale {
            out.push(Finding::new(
                FindingCode::FillStale,
                format!("{label} needs refilling: the board changed"),
                vec![me],
            ));
        }
        if copper.regions.is_empty() {
            out.push(Finding::new(
                FindingCode::FillEmpty,
                format!("{label} has no copper"),
                vec![me],
            ));
        }
        if let Some(audit) = &audit {
            audit.fill(&mut ctx, fill, &label, stale, &mut out);
        }
        if !stale {
            spokes_removed(&ctx, fill, copper, &label, &mut out);
        }
    }
    keepouts(&ctx, &mut out);
    out
}

/// Where one audited object has copper.
#[derive(Clone, Copy)]
enum On {
    /// Every copper layer: a hole, a through land.
    All,
    One(CopperLayer),
    /// The layers of via `k` of the project.
    Via(usize),
}

/// One board object stored fill copper is measured against.
struct Obstacle {
    object: ObjectRef,
    /// Its outline, covering the true shape (curves circumscribed) or
    /// flattened within the fill chord.
    ring: Vec<Point>,
    kind: CopperObject,
    net: Option<Id>,
    on: On,
    /// A plated hole, drilled out of copper of its own net.
    plated: bool,
}

/// One stored fill's copper, prepared once for the audit.
struct Stored {
    regions: Vec<Region>,
    index: RegionIndex,
    bounds: crate::geometry::Bounds,
}

impl Stored {
    fn of(fill: &Fill) -> Option<Self> {
        let regions = regions_of(fill);
        let [lo, hi] = geom::bounds(regions.iter().flatten())?;
        Some(Self {
            index: RegionIndex::new(&regions),
            bounds: crate::geometry::Bounds { min: lo, max: hi },
            regions,
        })
    }

    /// The least distance from the copper to `ring` (0 when they touch,
    /// cross or one holds the other), when it is under `limit`.
    fn gap(&self, ring: &[Point], limit: f64) -> Option<f64> {
        let n = ring.len();
        if n == 0 {
            return None;
        }
        let inside = ring.iter().any(|p| self.index.region_at(*p).is_some())
            || self
                .regions
                .iter()
                .filter_map(|r| r.first().and_then(|outer| outer.first()))
                .any(|p| polygon_contains(ring, *p));
        if inside {
            return Some(0.0);
        }
        self.distance(ring, limit)
    }

    /// The least distance from the copper's edges to `ring`'s edges (0
    /// when they cross), when it is under `limit`.
    fn distance(&self, ring: &[Point], limit: f64) -> Option<f64> {
        let n = ring.len();
        let reach = limit.max(0.0) + FILL_CHORD_MM;
        let gap = (0..n)
            .map(|i| {
                self.index
                    .segment_distance(ring[i], ring[(i + 1) % n], reach)
            })
            .fold(reach, f64::min);
        (gap < limit).then_some(gap)
    }

    /// The copper's area inside `other` (regions), mm².
    fn overlap(&self, other: &[Region]) -> f64 {
        geom::intersect(&self.regions, other)
            .iter()
            .map(geom::region_area)
            .sum()
    }

    /// How the copper meets the solid outline `ring` under a gap rule of
    /// `rule` mm: `Some(0)` where they overlap (or touch, under a rule),
    /// `Some(gap)` where the gap is short of the rule by more than
    /// [`AUDIT_SLACK_MM`], `None` where it keeps the rule. `overlap_only`
    /// (stale copper, which its refill cuts back) reports overlap alone.
    /// Touching is overlap only under a rule: with none, copper may meet
    /// the outline, so only an overlap of area counts.
    fn meets(&self, ring: &[Point], rule: f64, overlap_only: bool) -> Option<f64> {
        let limit = rule - AUDIT_SLACK_MM;
        if !overlap_only && limit > 0.0 {
            return self.gap(ring, limit);
        }
        // Cheap first: no contact at all is no overlap.
        self.gap(ring, f64::MIN_POSITIVE)?;
        let solid = geom::ccw(ring.to_vec())?;
        let n = solid.len();
        let around: f64 = (0..n)
            .map(|i| {
                let (a, b) = (solid[i], solid[(i + 1) % n]);
                (b[0] - a[0]).hypot(b[1] - a[1])
            })
            .sum();
        let rounding = OVERLAP_AREA_MM2 + around * OVERLAP_DEPTH_MM;
        (self.overlap(&[vec![solid]]) > rounding).then_some(0.0)
    }

    /// The copper's area outside the solid outline `outer`, with a vertex
    /// of it, when there is any.
    fn outside(&self, outer: &[Point]) -> Option<Point> {
        let inside = |p: &Point| polygon_contains(outer, *p);
        let all_in = self.regions.iter().flatten().flatten().all(inside);
        if all_in && self.distance(outer, f64::MIN_POSITIVE).is_none() {
            return None;
        }
        let solid = geom::ccw(outer.to_vec())?;
        let beyond = geom::difference(&self.regions, &[vec![solid]]);
        let area: f64 = beyond.iter().map(geom::region_area).sum();
        if area <= OVERLAP_AREA_MM2 {
            return None;
        }
        // Its lowest-left corner: where the stray copper starts, read the
        // same whichever way the arithmetic winds it.
        beyond
            .iter()
            .filter_map(|r| r.first())
            .flatten()
            .copied()
            .min_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])))
    }
}

/// The least area of copper that counts as overlap, mm²: below it, two
/// outlines that share an edge (a fill drawn to a keepout or the board
/// edge, as its computation leaves it) differ by the polygon arithmetic's
/// rounding only.
const OVERLAP_AREA_MM2: f64 = 1e-6;

/// How deep, mm, copper computed up to an outline may lie inside it along
/// the edge they share and still differ by the polygon arithmetic's
/// rounding only: the arithmetic snaps coordinates to its own grid
/// (measured: 0.12 nm on a 40 × 70 mm board), so the sliver's area grows
/// with the length of the shared edge, 2.6e-6 mm² along a 22 mm keepout
/// edge. No copper is 10 nm deep.
const OVERLAP_DEPTH_MM: f64 = 1e-5;

/// Every board object the stored fills are audited against, made once per
/// check run and filed in a bounds grid, with the board shape.
struct Audit<'a> {
    limits: crate::layout::limits::BoardRules<'a>,
    obstacles: Vec<Obstacle>,
    grid: super::super::grid::BoundsGrid,
    reach: f64,
    outer: Vec<Point>,
    /// Each cutout or slot's ring, with its feature.
    cutouts: Vec<(Vec<Point>, ObjectRef)>,
    stored: Vec<Option<Stored>>,
}

impl<'a> Audit<'a> {
    fn new(ctx: &Ctx<'a>) -> Self {
        let project = ctx.project;
        let mut obstacles = Vec::new();
        // Holes pass through every layer. Via drills lie inside their
        // vias' copper, which is measured instead.
        for hole in physical::holes_of(project, &ctx.pads) {
            if matches!(hole.owner, HoleOwner::Via(_)) {
                continue;
            }
            let ring = match hole.feature {
                HoleFeature::Round { at, diameter } => {
                    geom::circle(at, diameter / 2.0, FILL_CHORD_MM)
                }
                HoleFeature::Slot { from, to, width } => {
                    geom::stadium(from, to, width / 2.0, FILL_CHORD_MM)
                }
            };
            obstacles.push(Obstacle {
                object: hole.object(),
                ring,
                kind: CopperObject::hole(hole.plated),
                net: hole.net,
                on: On::All,
                plated: hole.plated,
            });
        }
        // Every land: pads with or without a net, and mounting-hole washers.
        for land in physical::lands_of(project, &ctx.pads) {
            obstacles.push(Obstacle {
                object: land.object(),
                ring: land.outline.flatten(FILL_CHORD_MM),
                kind: CopperObject::Pad,
                net: land.net(),
                on: if land.through {
                    On::All
                } else {
                    On::One(CopperLayer::from(land.layer))
                },
                plated: false,
            });
        }
        for (k, v) in project.vias.iter().enumerate() {
            obstacles.push(Obstacle {
                object: ObjectRef::Via { id: v.id },
                ring: geom::circle(v.position, v.diameter / 2.0, FILL_CHORD_MM),
                kind: CopperObject::Via,
                net: Some(v.net),
                on: On::Via(k),
                plated: false,
            });
        }
        for t in &project.tracks {
            obstacles.push(Obstacle {
                object: ObjectRef::Track { id: t.id },
                ring: geom::stadium(t.start, t.end, t.width / 2.0, FILL_CHORD_MM),
                kind: CopperObject::Track,
                net: Some(t.net),
                on: On::One(t.layer),
                plated: false,
            });
        }
        let limits = crate::layout::limits::BoardRules::of(project);
        let reach = super::super::copper::check_reach(&limits) + FILL_CHORD_MM;
        let grid = super::super::grid::BoundsGrid::new(
            obstacles
                .iter()
                .map(|o| crate::geometry::Bounds::of(o.ring.iter().copied())),
            reach,
        );
        let board = project.board();
        let cutouts = board
            .cutouts
            .iter()
            .map(|c| {
                let object = c.id.map_or(ObjectRef::Board, |id| ObjectRef::BoardFeature {
                    feature: id.0,
                });
                (c.outline.flatten(FILL_CHORD_MM), object)
            })
            .filter(|(ring, _)| ring.len() >= 3)
            .collect();
        Self {
            obstacles,
            grid,
            reach,
            outer: board.outer.flatten(FILL_CHORD_MM),
            cutouts,
            stored: project.fills.iter().map(Stored::of).collect(),
            limits,
        }
    }

    /// The audit of `fill`'s stored copper ([`findings`]).
    fn fill(
        &self,
        ctx: &mut Ctx<'_>,
        fill: &Fill,
        label: &str,
        stale: bool,
        out: &mut Vec<Finding>,
    ) {
        let project = ctx.project;
        let Some(Some(stored)) = project
            .fills
            .iter()
            .position(|f| f.id == fill.id)
            .and_then(|k| self.stored.get(k))
        else {
            return;
        };
        let me = ObjectRef::Fill { id: fill.id };
        let what = if stale {
            format!("{label} (stale)")
        } else {
            label.to_owned()
        };
        let other = |net: Option<Id>| fill.net.is_none() || net != fill.net;
        let mut rows: Vec<Finding> = Vec::new();
        let mut found = 0usize;
        let mut report = |f: Finding| {
            found += 1;
            if rows.len() < MAX_FILL_ROWS {
                rows.push(f);
            }
        };
        // How a measured gap reads: overlap, or the gap against its rule.
        let words = |gap: f64, need: &crate::layout::limits::Need, them: &str| {
            if gap <= 0.0 {
                format!("Overlap: {what} / {them}; refill it")
            } else {
                format!(
                    "{} is {} mm from {them}; {}; refill it",
                    crate::library::names::capitalised(&what),
                    crate::layout::limits::mm_against(gap, need.mm),
                    need.text()
                )
            }
        };
        let layers = project.stackup.layers();
        let mut near = Vec::new();
        self.grid.near(&stored.bounds, self.reach, &mut near);
        for o in near.iter().filter_map(|k| self.obstacles.get(*k)) {
            let on = match o.on {
                On::All => true,
                On::One(layer) => layer == fill.layer,
                On::Via(k) => project
                    .vias
                    .get(k)
                    .is_some_and(|v| v.on(fill.layer, &layers)),
            };
            // A plated hole of the fill's own net is drilled out of the
            // copper it joins; same-net copper is the fill's own.
            if !on || !other(o.net) && (o.plated || !o.kind.is_hole()) {
                continue;
            }
            let need = self
                .limits
                .clearance_need(CopperObject::Fill, fill.net, o.kind, o.net);
            let Some(gap) = stored.meets(&o.ring, need.mm, stale) else {
                continue;
            };
            report(Finding::new(
                FindingCode::FillClearance,
                words(gap, &need, &describe(project, &o.object)),
                vec![me, o.object],
            ));
        }
        // Other fills on the layer, each pair once (from the lower id).
        for (k, theirs) in project.fills.iter().enumerate() {
            if theirs.id <= fill.id
                || theirs.layer != fill.layer
                || !(other(theirs.net) || theirs.net.is_none())
            {
                continue;
            }
            let Some(Some(them)) = self.stored.get(k) else {
                continue;
            };
            if !stored.bounds.inflated(self.reach).intersects(&them.bounds) {
                continue;
            }
            let fresh_pair = !stale
                && theirs
                    .computed
                    .as_ref()
                    .is_some_and(|c| ctx.key(theirs.id).ok().as_ref() == Some(&c.key));
            let need = self.limits.clearance_need(
                CopperObject::Fill,
                fill.net,
                CopperObject::Fill,
                theirs.net,
            );
            // Overlap by area (an island in the other's hole is not in
            // it), then the gap between their edges.
            let limit = need.mm - AUDIT_SLACK_MM;
            let gap = if stored.overlap(&them.regions) > OVERLAP_AREA_MM2 {
                0.0
            } else if fresh_pair && limit > 0.0 {
                them.regions
                    .iter()
                    .flatten()
                    .filter_map(|ring| stored.distance(ring, limit))
                    .fold(f64::INFINITY, f64::min)
            } else {
                f64::INFINITY
            };
            if gap.is_finite() {
                report(Finding::new(
                    FindingCode::FillClearance,
                    words(gap, &need, &fill_label(project, theirs)),
                    vec![me, ObjectRef::Fill { id: theirs.id }],
                ));
            }
        }
        // The board shape.
        let edge =
            self.limits
                .clearance_need(CopperObject::Fill, fill.net, CopperObject::BoardEdge, None);
        let edge_limit = edge.mm - AUDIT_SLACK_MM;
        let at = |p: Point| {
            format!(
                "({}, {}) mm",
                crate::layout::limits::mm(p[0]),
                crate::layout::limits::mm(p[1])
            )
        };
        if self.outer.len() >= 3 {
            if let Some(p) = stored.outside(&self.outer) {
                report(Finding::new(
                    FindingCode::FillBoardEdge,
                    format!(
                        "{what}: copper lies outside the board outline at {}; refill it",
                        at(p)
                    ),
                    vec![me, ObjectRef::Board],
                ));
            } else if !stale
                && edge_limit > 0.0
                && let Some(gap) = stored.distance(&self.outer, edge_limit)
            {
                report(Finding::new(
                    FindingCode::FillBoardEdge,
                    format!(
                        "{what}: copper is {} mm from the board edge; {}; refill it",
                        crate::layout::limits::mm_against(gap, edge.mm),
                        edge.text()
                    ),
                    vec![me, ObjectRef::Board],
                ));
            }
        }
        for (ring, object) in &self.cutouts {
            if let Some(gap) = stored.meets(ring, edge.mm, stale) {
                report(Finding::new(
                    FindingCode::FillBoardEdge,
                    if gap <= 0.0 {
                        format!(
                            "{what}: copper lies in {}; refill it",
                            describe(project, object)
                        )
                    } else {
                        format!(
                            "{what}: copper is {} mm from {}; {}; refill it",
                            crate::layout::limits::mm_against(gap, edge.mm),
                            describe(project, object),
                            edge.text()
                        )
                    },
                    vec![me, *object],
                ));
            }
        }
        // Keepouts that forbid fills on the layer.
        for (k, owner) in &ctx.keepouts {
            if !k.on(fill.layer) || !k.forbid.fills {
                continue;
            }
            let ring = k.outline.flatten(FILL_CHORD_MM);
            if stored.meets(&ring, 0.0, true).is_some() {
                let them = match owner {
                    Some(instance) => ObjectRef::Instance { id: *instance },
                    None => ObjectRef::Keepout { id: k.id },
                };
                let name = if k.name.is_empty() {
                    "a keepout".to_owned()
                } else {
                    format!("keepout {}", k.name)
                };
                report(Finding::new(
                    FindingCode::KeepoutFill,
                    format!("{what}: copper lies in {name}; refill it"),
                    vec![me, them],
                ));
            }
        }
        let unlisted = found.saturating_sub(rows.len());
        out.extend(rows);
        if unlisted > 0 {
            out.push(Finding::new(
                FindingCode::FillClearanceUnlisted,
                format!(
                    "{}: {unlisted} more copper violations not listed; refill it",
                    crate::library::names::capitalised(&what)
                ),
                vec![me],
            ));
        }
    }
}

/// One information finding per pad or via whose thermal spokes the fresh
/// fill removed because they would end without reaching its net's copper.
fn spokes_removed(
    ctx: &Ctx<'_>,
    fill: &Fill,
    copper: &FilledCopper,
    label: &str,
    out: &mut Vec<Finding>,
) {
    let project = ctx.project;
    let net = fill.net.map_or("its net", |n| project.net_name(n));
    for removed in &copper.removed_spokes {
        let what = match removed.object {
            ObjectRef::Pad { instance, pad } => {
                let designator = project
                    .instance(instance)
                    .map_or("a part", |i| i.designator.as_str());
                match ctx
                    .pads
                    .iter()
                    .find(|p| p.instance == instance && p.pad.id == pad)
                {
                    Some(p) if !p.pad.number.is_empty() => {
                        format!("{designator} pad {}", p.pad.number)
                    }
                    _ => format!("a pad of {designator}"),
                }
            }
            ObjectRef::Via { .. } => "a via".to_owned(),
            _ => continue,
        };
        let spokes = if removed.spokes == 1 {
            "1 thermal spoke".to_owned()
        } else {
            format!("{} thermal spokes", removed.spokes)
        };
        let they = if removed.spokes == 1 { "it" } else { "they" };
        out.push(Finding::new(
            FindingCode::FillSpokesRemoved,
            format!(
                "{}: {spokes} of {what} removed; {they} would end without reaching {net} copper",
                crate::library::names::capitalised(label)
            ),
            vec![removed.object, ObjectRef::Fill { id: fill.id }],
        ));
    }
}

fn describe(project: &Project, object: &ObjectRef) -> String {
    match *object {
        ObjectRef::Track { id } => project.tracks.iter().find(|t| t.id == id).map_or_else(
            || "a track".into(),
            |t| format!("a track on {}", project.net_name(t.net)),
        ),
        ObjectRef::Via { id } => project.via(id).map_or_else(
            || "a via".into(),
            |v| format!("a via on {}", project.net_name(v.net)),
        ),
        ObjectRef::Pad { instance, .. } => format!(
            "a pad of {}",
            project
                .instance(instance)
                .map_or("component", |i| i.designator.as_str())
        ),
        ObjectRef::Board => "the board".into(),
        ObjectRef::BoardFeature { feature } => crate::layout::physical::feature_name(
            project,
            crate::model::board::BoardFeatureId(feature),
        ),
        ObjectRef::PadVia { instance, .. } => format!(
            "a via-in-pad hole of {}",
            project
                .instance(instance)
                .map_or("component", |i| i.designator.as_str())
        ),
        _ => "copper".into(),
    }
}

/// Tracks, vias and pads inside keepouts that forbid them on their layer.
fn keepouts(ctx: &Ctx<'_>, out: &mut Vec<Finding>) {
    let project = ctx.project;
    for (k, owner) in &ctx.keepouts {
        let Some(ring) = geom::ccw(k.outline.flatten(FILL_CHORD_MM)) else {
            continue;
        };
        let Some(kb) = geom::bounds([&ring]) else {
            continue;
        };
        // A footprint keep-out is reported against its part.
        let me = match owner {
            Some(instance) => ObjectRef::Instance { id: *instance },
            None => ObjectRef::Keepout { id: k.id },
        };
        let name = if k.name.is_empty() {
            "a keepout".to_owned()
        } else {
            format!("keepout {}", k.name)
        };
        let inside = |other: &[Point]| {
            geom::bounds([&other.to_vec()]).is_some_and(|b| geom::overlaps(kb, b, 0.0))
                && polygons_overlap(&ring, other)
        };
        if k.forbid.tracks {
            for t in project.tracks.iter().filter(|t| k.on(t.layer)) {
                let copper = geom::stadium(t.start, t.end, t.width / 2.0, FILL_CHORD_MM);
                if inside(&copper) {
                    out.push(Finding::new(
                        FindingCode::KeepoutTrack,
                        format!("A track on {} enters {name}", project.net_name(t.net)),
                        vec![ObjectRef::Track { id: t.id }, me],
                    ));
                }
            }
        }
        if k.forbid.vias {
            for v in &project.vias {
                let on = ctx.layers.iter().any(|l| k.on(*l) && v.on(*l, &ctx.layers));
                if on && inside(&geom::circle(v.position, v.diameter / 2.0, FILL_CHORD_MM)) {
                    out.push(Finding::new(
                        FindingCode::KeepoutVia,
                        format!("A via on {} lies in {name}", project.net_name(v.net)),
                        vec![ObjectRef::Via { id: v.id }, me],
                    ));
                }
            }
        }
        if k.forbid.pads {
            for pad in ctx.pads.iter().filter(|p| Some(p.instance) != *owner) {
                let on = ctx.layers.iter().any(|l| k.on(*l) && pad.on_copper(*l));
                if on && inside(&pad.outline().flatten(FILL_CHORD_MM)) {
                    out.push(Finding::new(
                        FindingCode::KeepoutPad,
                        format!(
                            "A pad of {} lies in {name}",
                            project
                                .instance(pad.instance)
                                .map_or("component", |i| i.designator.as_str())
                        ),
                        vec![
                            ObjectRef::Pad {
                                instance: pad.instance,
                                pad: pad.pad.id,
                            },
                            me,
                        ],
                    ));
                }
            }
        }
    }
}

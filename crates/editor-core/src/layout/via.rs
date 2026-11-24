//! Board via commands, the document rules over vias, routing through a
//! layer change, and the via manufacturing checks (drill size, annular
//! ring, aspect ratio, drill to copper, hole to hole, drill to board edge,
//! and mounting holes against vias and copper). Every limit comes from the
//! project's design rules and net classes ([`BoardRules`]); the constants
//! here are the default profile's values. Connectivity through vias lives
//! in the copper graph ([`super::connectivity`]): one land per layer the
//! via reaches, joined by its barrel.
use super::board_edge::{BoardEdges, EdgeVerdict};
use super::copper::check_reach;
use super::grid::BoundsGrid;
use super::limits::{BoardRules, mm, mm_against};
use super::physical::LandOwner;
use super::{
    BoardCopper, CopperRing, EPS, Error, Result, RouteEditSet, RouteRequest, WorldPad,
    check_pair_budget, distance, invalid,
};
use crate::diagnostics::{Finding, FindingCode, ObjectRef, Requirement};
use crate::geometry::{Bounds, Point, point_valid};
use crate::ids::Id;
use crate::library::names::capitalised;
use crate::model::board::BoardFeatureId;
use crate::model::{CopperLayer, Project, Via, ViaSpan};
use crate::rules::{CopperObject, RuleKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Largest via pad, mm: an input bound on the document, not a rule (the
/// fab profile's `min drill` and `min via annular ring` are the limits
/// vias are placed and checked against).
pub const MAX_DIAMETER_MM: f64 = 6.0;

/// A via to add: everything but its identity.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViaRequest {
    pub net: Id,
    pub position: Point,
    pub drill: f64,
    pub diameter: f64,
    pub span: ViaSpan,
    pub tented: bool,
}

impl ViaRequest {
    /// A through via on `net` at `position` of the rules' default via
    /// drill and pad, tented as `tent vias` says.
    pub fn through(rules: &crate::rules::DesignRules, net: Id, position: Point) -> Self {
        Self {
            net,
            position,
            drill: rules.holes.via_drill_default,
            diameter: rules.holes.via_diameter_default,
            span: ViaSpan::Through,
            tented: rules.mask.tent_vias,
        }
    }

    /// A test fixture, never in the product: the test prototype profile's
    /// default via (0.3 mm drill, 0.6 mm pad, tented).
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn fixture(net: Id, position: Point) -> Self {
        Self {
            net,
            position,
            drill: 0.3,
            diameter: 0.6,
            span: ViaSpan::Through,
            tented: true,
        }
    }
}

/// A route that changes layer: a track on `request.layer` to
/// `request.end`, a via there, then a track on `to_layer` to `end`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerChangeRequest {
    pub request: super::RouteRequest,
    pub via: ViaRequest,
    pub to_layer: CopperLayer,
    pub end: Point,
}

/// Add a via clear of other nets' copper; returns its identity.
pub(super) fn add(project: &mut Project, request: ViaRequest) -> Result<Id> {
    let id = insert(project, request)?;
    check_clearance(project, id)?;
    Ok(id)
}

pub(super) fn insert(project: &mut Project, request: ViaRequest) -> Result<Id> {
    let id = project.allocate()?;
    project.vias.push(via_of(id, request));
    Ok(id)
}

fn via_of(id: Id, request: ViaRequest) -> Via {
    Via {
        id,
        net: request.net,
        position: request.position,
        drill: request.drill,
        diameter: request.diameter,
        span: request.span,
        tented: request.tented,
        locked: false,
    }
}

/// Whether the document can hold the via `request` asks for, whatever the
/// rules say of it.
pub(super) fn check_held(
    project: &Project,
    request: &ViaRequest,
    nets: &BTreeSet<Id>,
) -> Result<()> {
    check_shape(project, &via_of(0, *request), nets)
}

fn unlocked(project: &mut Project, id: Id) -> Result<&mut Via> {
    let at = project.vias.iter().position(|v| v.id == id);
    unlocked_at(project, at)
}

/// The unlocked via at index `at` of the project's vias (as an id index
/// found it); `None` is an unknown via.
pub(super) fn unlocked_at(project: &mut Project, at: Option<usize>) -> Result<&mut Via> {
    let via = at
        .and_then(|k| project.vias.get_mut(k))
        .ok_or_else(|| Error::NotFound("unknown via".into()))?;
    if via.locked {
        return Err(Error::Conflict("via is locked".into()));
    }
    Ok(via)
}

pub(super) fn move_to(project: &mut Project, id: Id, position: Point) -> Result<()> {
    unlocked(project, id)?.position = position;
    check_clearance(project, id)
}

/// Route `request.request` to the via, place the via at its end and route
/// on `to_layer` to `request.end`, with one width: one transaction that
/// creates two tracks and the via. Returns the via.
pub(super) fn route_layer_change(project: &mut Project, request: LayerChangeRequest) -> Result<Id> {
    let LayerChangeRequest {
        request: first,
        via,
        to_layer,
        end,
    } = request;
    if !point_valid(via.position) || distance(via.position, first.end) > EPS {
        return invalid("the via must sit at the end of the first route");
    }
    if via.net != first.net {
        return invalid("the via must be on the routed net");
    }
    if to_layer == first.layer {
        return invalid("a layer change needs a different copper layer to route on");
    }
    for layer in [first.layer, to_layer] {
        if !project.stackup.has(layer) {
            return Err(Error::Invalid(format!(
                "the stackup has no {} copper layer",
                layer.name()
            )));
        }
    }
    let layers = project.stackup.layers();
    let probe = Via {
        id: 0,
        net: via.net,
        position: via.position,
        drill: via.drill,
        diameter: via.diameter,
        span: via.span,
        tented: via.tented,
        locked: false,
    };
    if !probe.on(first.layer, &layers) || !probe.on(to_layer, &layers) {
        return invalid("the via must join both routed layers");
    }
    let id = insert(project, via)?;
    let second = RouteRequest {
        net: first.net,
        start: first.end,
        end,
        width: first.width,
        layer: to_layer,
    };
    super::apply_routes(
        project,
        RouteEditSet {
            add: vec![first, second],
            ..Default::default()
        },
    )?;
    check_clearance(project, id)?;
    Ok(id)
}

/// Whether a via has copper on any layer a pad has copper on.
fn shares_layer(via: &Via, pad: &WorldPad, layers: &[CopperLayer]) -> bool {
    layers.iter().any(|l| pad.on(*l) && via.on(*l, layers))
}

/// Whether a via is closer than the effective clearance (rules, net
/// classes and class pairs) to another net's pad: the via side of a
/// revision impact plan.
pub fn via_pad_clearance_conflict(
    via: &Via,
    pad: &WorldPad,
    layers: &[CopperLayer],
    rules: &super::limits::BoardRules<'_>,
) -> bool {
    use crate::rules::CopperObject;
    via.net != pad.net
        && shares_layer(via, pad, layers)
        && CopperRing::of_via(via).closer_than(
            &CopperRing::of_pad(pad),
            rules.clearance(
                CopperObject::Via,
                Some(via.net),
                CopperObject::Pad,
                Some(pad.net),
            ),
        )
}

/// Refuse a via closer than the clearance rule to another net's copper on
/// a layer it joins, as routing refuses a track.
pub(super) fn check_clearance(project: &Project, id: Id) -> Result<()> {
    clearance_of(project, id, false)
}

/// [`check_clearance`]; with `every`, every object's outline is made and
/// measured, not only those whose bounds are within reach (tests).
fn clearance_of(project: &Project, id: Id, every: bool) -> Result<()> {
    let via = project
        .via(id)
        .ok_or_else(|| Error::NotFound("unknown via".into()))?;
    let layers = project.stackup.layers();
    let limits = BoardRules::of(project);
    let ring = CopperRing::of_via(via);
    let me = (CopperObject::Via, Some(via.net));
    let refuse = |kind: &str, object: CopperObject, net: Id| {
        Err(Error::Conflict(format!(
            "Too close to a {kind} on {}: {}",
            project.net_name(net),
            limits.clearance_need(me.0, me.1, object, Some(net)).short()
        )))
    };
    // Copper whose bounds are farther than any clearance (and both
    // outlines' slack) cannot be too close: its outline is not made.
    let reach = limits.max_clearance() + 1.0;
    let near = |a: Point, b: Point, r: f64| {
        every
            || ring.bounds.is_some_and(|rb| {
                let ob = Bounds {
                    min: [a[0].min(b[0]) - r, a[1].min(b[1]) - r],
                    max: [a[0].max(b[0]) + r, a[1].max(b[1]) + r],
                };
                super::grid::within(&rb, &ob, reach)
            })
    };
    for track in project
        .tracks
        .iter()
        .filter(|t| t.net != via.net && near(t.start, t.end, t.width / 2.0))
    {
        let need = limits.clearance(me.0, me.1, CopperObject::Track, Some(track.net));
        if via.on(track.layer, &layers) && ring.closer_than(&CopperRing::of_track(track), need) {
            return refuse("track", CopperObject::Track, track.net);
        }
    }
    let placed: Vec<super::PlacedPad> = super::placed_pads(project)
        .into_iter()
        .filter(|p| {
            let b = p.outline().bounds(0.0);
            near(b.min, b.max, 0.0)
        })
        .collect();
    for land in super::physical::lands_of(project, &placed)
        .iter()
        .filter(|l| l.net() != Some(via.net))
    {
        let need = limits.clearance(me.0, me.1, CopperObject::Pad, land.net());
        if layers.iter().any(|l| via.on(*l, &layers) && land.on(*l))
            && ring.closer_than(&land.ring, need)
        {
            return Err(Error::Conflict(format!(
                "Too close to {}: {}",
                land.name,
                limits
                    .clearance_need(me.0, me.1, CopperObject::Pad, land.net())
                    .short()
            )));
        }
    }
    for other in project
        .vias
        .iter()
        .filter(|v| v.net != via.net && near(v.position, v.position, v.diameter / 2.0))
    {
        let need = limits.clearance(me.0, me.1, CopperObject::Via, Some(other.net));
        if layers
            .iter()
            .any(|l| via.on(*l, &layers) && other.on(*l, &layers))
            && ring.closer_than(&CopperRing::of_via(other), need)
        {
            return refuse("via", CopperObject::Via, other.net);
        }
    }
    Ok(())
}

/// One copper shape a drilled hole is checked against.
pub(super) struct Drilled<'a> {
    pub ring: &'a CopperRing,
    pub object: ObjectRef,
    pub kind: CopperObject,
    pub net: Option<Id>,
    /// A land's name ("J1 pad 3"); tracks and vias are named by net.
    pub name: Option<&'a str>,
    /// The mounting hole whose washer this is.
    pub washer: Option<BoardFeatureId>,
}

impl Drilled<'_> {
    /// How a hole message names it: "U1 pad 1 on GND", "a track on GND",
    /// "the plated mounting hole at (5, 5) mm".
    pub fn words(&self, project: &Project) -> String {
        match (self.name, self.net) {
            (Some(name), Some(net)) => format!("{name} on {}", project.net_name(net)),
            (Some(name), None) => name.to_owned(),
            (None, Some(net)) => format!("a {} on {}", self.kind.label(), project.net_name(net)),
            (None, None) => format!("a {}", self.kind.label()),
        }
    }
}

/// Every copper shape a drilled hole is checked against, in the order
/// findings name them (lands, vias, tracks).
pub(super) fn drilled_copper(copper: &BoardCopper) -> Vec<Drilled<'_>> {
    let mut shapes = Vec::new();
    for land in copper.lands() {
        shapes.push(Drilled {
            ring: &land.ring,
            object: land.object(),
            kind: CopperObject::Pad,
            net: land.net(),
            name: Some(land.name.as_str()),
            washer: match land.owner {
                LandOwner::Washer { hole } => Some(hole),
                LandOwner::Pad { .. } => None,
            },
        });
    }
    for (k, via) in copper.vias().iter().enumerate() {
        if let Some(ring) = copper.via_ring(k) {
            shapes.push(Drilled {
                ring,
                object: ObjectRef::Via { id: via.id },
                kind: CopperObject::Via,
                net: Some(via.net),
                name: None,
                washer: None,
            });
        }
    }
    for (track, ring) in copper.tracks().iter().zip(copper.track_rings()) {
        shapes.push(Drilled {
            ring,
            object: ObjectRef::Track { id: track.id },
            kind: CopperObject::Track,
            net: Some(track.net),
            name: None,
            washer: None,
        });
    }
    shapes
}

/// The via manufacturing checks over `copper`'s vias: drill size, annular
/// ring, aspect ratio, drilled hole to another net's copper on any layer,
/// hole to hole and drill to board edge; then each mounting hole of the
/// board shape against via drills and all copper. Every limit is the
/// project's ([`BoardRules`]); pairs are found through bounds grids that
/// reach as far as the largest clearance. Copper clearance is part of
/// [`BoardCopper::clearance`]; connectivity through vias is the copper
/// graph's.
pub(super) fn findings(copper: &BoardCopper, project: &Project) -> Vec<Finding> {
    let cx = ViaCx::new(copper, project);
    let shapes = BoundsGrid::new(cx.shape_bounds.iter().copied(), cx.reach);
    let holes = BoundsGrid::new(cx.drill_bounds.iter().copied(), cx.reach);
    let budget = cx.budget();
    let mut issues = Vec::new();
    let (mut near_shapes, mut near_holes) = (Vec::new(), Vec::new());
    let mut pairs = 0usize;
    for i in 0..cx.items() {
        let found = match cx.bounds(i) {
            Some(bounds) => {
                shapes.near(&bounds, cx.reach, &mut near_shapes);
                holes.near(&bounds, cx.reach, &mut near_holes);
                cx.item(
                    i,
                    &near_shapes,
                    &near_holes,
                    budget.saturating_sub(pairs),
                    &mut issues,
                )
            }
            None => cx.item(i, &[], &[], budget.saturating_sub(pairs), &mut issues),
        };
        match found {
            Ok(n) => pairs += n,
            Err(()) => {
                issues.push(Finding::new(
                    FindingCode::BoardCheckLimit,
                    "Via check limit reached; remaining vias are unchecked".into(),
                    vec![],
                ));
                return issues;
            }
        }
    }
    issues
}

/// [`findings`] as each via's and mounting hole's findings, made from
/// `last` and what changed since (see [`BoardCopper::clearance_items`]).
/// `None` when the check would stop at its pair limit.
pub(super) fn items(
    copper: &BoardCopper,
    project: &Project,
    last: Option<(&super::Items, &super::CopperChanges)>,
) -> Option<super::Items> {
    let cx = ViaCx::new(copper, project);
    let grids = last.is_none().then(|| {
        (
            BoundsGrid::new(cx.shape_bounds.iter().copied(), cx.reach),
            BoundsGrid::new(cx.drill_bounds.iter().copied(), cx.reach),
        )
    });
    let keys: Vec<super::ItemKey> = cx
        .vias
        .iter()
        .map(|v| super::ItemKey::Via(v.id))
        .chain((0..cx.mounts.len()).map(super::ItemKey::Mount))
        .collect();
    let bounds: Vec<Option<Bounds>> = (0..cx.items()).map(|i| cx.bounds(i)).collect();
    let (mut near_shapes, mut near_holes) = (Vec::new(), Vec::new());
    let again = |i: usize, key: &super::ItemKey| {
        last.is_some_and(|(_, changes)| changes.again(key, bounds.get(i), cx.reach))
    };
    let last = last.map(|(items, _)| (items, &again as &dyn Fn(usize, &super::ItemKey) -> bool));
    let items = super::Items::made(&keys, last, |i| {
        let mut out = Vec::new();
        let Some(b) = cx.bounds(i) else {
            let pairs = cx.item(i, &[], &[], usize::MAX, &mut out).ok()?;
            return Some((out, pairs));
        };
        match &grids {
            Some((shapes, holes)) => {
                shapes.near(&b, cx.reach, &mut near_shapes);
                holes.near(&b, cx.reach, &mut near_holes);
            }
            None => {
                super::copper::scan(&cx.shape_bounds, &b, cx.reach, &mut near_shapes);
                super::copper::scan(&cx.drill_bounds, &b, cx.reach, &mut near_holes);
            }
        }
        let pairs = cx
            .item(i, &near_shapes, &near_holes, usize::MAX, &mut out)
            .ok()?;
        Some((out, pairs))
    })?;
    (items.pairs() <= cx.budget()).then_some(items)
}

/// One state's via checks: the vias and their drills, the copper they are
/// measured against, the board's mounting holes and the rules.
struct ViaCx<'a> {
    project: &'a Project,
    limits: BoardRules<'a>,
    edges: BoardEdges,
    vias: &'a [Via],
    drills: Vec<&'a CopperRing>,
    drill_bounds: Vec<Option<Bounds>>,
    shapes: Vec<Drilled<'a>>,
    shape_bounds: Vec<Option<Bounds>>,
    /// The board shape's holes ([`super::physical::board_holes`]):
    /// mounting holes, then plated board slots, each with its outline.
    mounts: Vec<(super::physical::Hole, CopperRing)>,
    reach: f64,
    tracks: usize,
    lands: usize,
}

impl<'a> ViaCx<'a> {
    fn new(copper: &'a BoardCopper, project: &'a Project) -> Self {
        let limits = BoardRules::of(project);
        let vias = copper.vias();
        let drills: Vec<&CopperRing> = (0..vias.len())
            .filter_map(|k| copper.drill_ring(k))
            .collect();
        let shapes = drilled_copper(copper);
        Self {
            project,
            edges: BoardEdges::of(project),
            vias,
            drill_bounds: drills.iter().map(|d| d.bounds).collect(),
            drills,
            shape_bounds: shapes.iter().map(|s| s.ring.bounds).collect(),
            shapes,
            mounts: super::physical::board_holes(project)
                .into_iter()
                .map(|h| {
                    let ring = h.feature.ring();
                    (h, ring)
                })
                .collect(),
            reach: check_reach(&limits),
            limits,
            tracks: copper.tracks().len(),
            lands: copper.lands().len(),
        }
    }

    fn budget(&self) -> usize {
        check_pair_budget(
            self.lands
                .saturating_add(self.vias.len())
                .saturating_add(self.tracks),
        )
    }

    /// The vias, then the mounting holes.
    fn items(&self) -> usize {
        self.vias.len() + self.mounts.len()
    }

    /// The bounds of item `i`'s hole.
    fn bounds(&self, i: usize) -> Option<Bounds> {
        match i.checked_sub(self.vias.len()) {
            None => self.drill_bounds.get(i).copied().flatten(),
            Some(m) => self.mounts.get(m).and_then(|(_, ring)| ring.bounds),
        }
    }

    /// Item `i` (a via, then a mounting hole) against the copper shapes
    /// `near_shapes` and the via drills `near_holes` (both ascending, near
    /// its hole): the pairs compared, or `Err` once more than `budget`
    /// would be.
    fn item(
        &self,
        i: usize,
        near_shapes: &[usize],
        near_holes: &[usize],
        budget: usize,
        issues: &mut Vec<Finding>,
    ) -> std::result::Result<usize, ()> {
        match i.checked_sub(self.vias.len()) {
            None => self.via(i, near_shapes, near_holes, budget, issues),
            Some(m) => self.mount(m, near_shapes, near_holes, budget, issues),
        }
    }

    fn via(
        &self,
        i: usize,
        near_shapes: &[usize],
        near_holes: &[usize],
        budget: usize,
        issues: &mut Vec<Finding>,
    ) -> std::result::Result<usize, ()> {
        let (Some(via), Some(drill)) = (self.vias.get(i), self.drills.get(i)) else {
            return Ok(0);
        };
        let (project, limits) = (self.project, &self.limits);
        let ring_min = limits.need(RuleKey::ViaAnnularRingMin);
        let aspect_max = limits.rules().holes.aspect_ratio_max;
        let thickness = project.stackup.thickness_mm();
        // Via drills keep the via hole rules; a mounting hole its own
        // kind's (plated or not, `CopperObject::hole`).
        let hole = CopperObject::ViaHole;
        let edge = limits.clearance(hole, None, CopperObject::BoardEdge, None);
        let spacing = limits.clearance(hole, None, hole, None);
        let mut pairs = 0usize;
        let mut over = || {
            pairs += 1;
            if pairs > budget { Err(()) } else { Ok(()) }
        };
        let me = ObjectRef::Via { id: via.id };
        let name = project.net_name(via.net);
        if via.annular_ring() < ring_min.mm - EPS {
            issues.push(
                Finding::new(
                    FindingCode::BoardViaAnnularRing,
                    format!(
                        "A via on {name}: annular ring {:.3} mm is under the minimum; {}",
                        via.annular_ring(),
                        ring_min.text()
                    ),
                    vec![me],
                )
                .requiring(ring_min.at_most(via.annular_ring())),
            );
        }
        if let Some(max) = aspect_max
            && via.drill > 0.0
            && thickness / via.drill > max + 1e-9
        {
            issues.push(Finding::new(
                FindingCode::BoardViaAspectRatio,
                format!(
                    "A via on {name}: aspect ratio {:.1}:1 ({} mm board, {} mm drill) is over the maximum; needs at most {}:1 ({})",
                    thickness / via.drill,
                    mm(thickness),
                    mm(via.drill),
                    mm(max),
                    super::limits::cite(limits.project(), RuleKey::AspectRatioMax)
                ),
                vec![me],
            )
            .requiring(super::limits::at_least(
                limits.project(),
                RuleKey::AspectRatioMax,
                thickness / via.drill,
            )));
        }
        if let EdgeVerdict::TooClose { gap, near } = self.edges.verdict(drill, edge) {
            let need = limits.clearance_need(hole, None, CopperObject::BoardEdge, None);
            issues.push(
                Finding::new(
                    FindingCode::BoardEdgeClearance,
                    format!(
                        "A via on {name}: drill is {} mm from {}; {}",
                        mm_against(gap, need.mm),
                        near.describe(),
                        need.text()
                    ),
                    vec![me],
                )
                .requiring(need.at_most(gap)),
            );
        }
        if drill.bounds.is_none() {
            return Ok(0);
        }
        for shape in near_shapes
            .iter()
            .filter_map(|k| self.shapes.get(*k))
            .filter(|s| s.net != Some(via.net))
        {
            over()?;
            let need = limits.clearance(hole, Some(via.net), shape.kind, shape.net);
            if drill.closer_than(shape.ring, need) {
                let need = limits.clearance_need(hole, Some(via.net), shape.kind, shape.net);
                let gap = drill.gap(shape.ring, need.mm + 1.0);
                issues.push(
                    Finding::new(
                        FindingCode::BoardViaDrillClearance,
                        format!(
                            "A via on {name}: drill is {} mm from {}; {}",
                            mm_against(gap, need.mm),
                            shape.words(project),
                            need.text()
                        ),
                        vec![me, shape.object],
                    )
                    .requiring(need.at_most(gap)),
                );
            }
        }
        for (other, other_drill) in near_holes
            .iter()
            .filter(|j| **j > i)
            .filter_map(|j| Some((self.vias.get(*j)?, self.drills.get(*j)?)))
        {
            over()?;
            if drill.closer_than(other_drill, spacing) {
                let need = limits.clearance_need(hole, None, hole, None);
                let gap = drill.gap(other_drill, need.mm + 1.0);
                issues.push(
                    Finding::new(
                        FindingCode::BoardViaHoleSpacing,
                        format!(
                            "Vias on {name} and {}: drills are {} mm apart; {}",
                            project.net_name(other.net),
                            mm_against(gap, need.mm),
                            need.text()
                        ),
                        vec![me, ObjectRef::Via { id: other.id }],
                    )
                    .requiring(need.at_most(gap)),
                );
            }
        }
        Ok(pairs)
    }

    // The board's holes have no net: every copper object, via drill and
    // other board hole is measured against them.
    fn mount(
        &self,
        m: usize,
        near_shapes: &[usize],
        near_holes: &[usize],
        budget: usize,
        issues: &mut Vec<Finding>,
    ) -> std::result::Result<usize, ()> {
        let Some((mount, ring)) = self.mounts.get(m) else {
            return Ok(0);
        };
        let (project, limits) = (self.project, &self.limits);
        let hole = CopperObject::ViaHole;
        let mut pairs = 0usize;
        let mut over = || {
            pairs += 1;
            if pairs > budget { Err(()) } else { Ok(()) }
        };
        let kind = CopperObject::hole(mount.plated);
        let me = mount.object();
        let name = capitalised(&mount.name);
        let washer = match mount.owner {
            super::physical::HoleOwner::MountingHole(id) => Some(id),
            _ => None,
        };
        // A plated board slot is itself an edge of the board shape: the
        // outline check measures it against the others.
        if washer.is_some()
            && let Some((text, needs)) = edge_text(&self.edges, ring, limits, kind)
        {
            issues.push(
                Finding::new(
                    FindingCode::BoardEdgeClearance,
                    format!("{name} {text}"),
                    vec![me],
                )
                .requiring(needs),
            );
        }
        if ring.bounds.is_none() {
            return Ok(0);
        }
        // Its own plated ring (if any) is not board copper here.
        for shape in near_shapes
            .iter()
            .filter_map(|k| self.shapes.get(*k))
            .filter(|s| washer.is_none() || s.washer != washer)
        {
            over()?;
            let need = limits.clearance(kind, None, shape.kind, shape.net);
            if ring.closer_than(shape.ring, need) {
                let need = limits.clearance_need(kind, None, shape.kind, shape.net);
                let what = shape.words(project);
                let gap = ring.gap(shape.ring, need.mm + 1.0);
                // Touching or overlapping copper is said so, not "0 mm".
                let how = if gap <= EPS {
                    format!("overlaps {what}")
                } else {
                    format!("is {} mm from {what}", mm_against(gap, need.mm))
                };
                issues.push(
                    Finding::new(
                        FindingCode::BoardHoleClearance,
                        format!("{name} {how}; {}", need.text()),
                        vec![me, shape.object],
                    )
                    .requiring(need.at_most(gap)),
                );
            }
        }
        for (via, drill) in near_holes
            .iter()
            .filter_map(|j| Some((self.vias.get(*j)?, self.drills.get(*j)?)))
        {
            over()?;
            let spacing = limits.clearance(kind, None, hole, None);
            if ring.closer_than(drill, spacing) {
                let need = limits.clearance_need(kind, None, hole, None);
                let gap = ring.gap(drill, need.mm + 1.0);
                issues.push(
                    Finding::new(
                        FindingCode::BoardHoleSpacing,
                        format!(
                            "{name} and a via on {}: drills are {} mm apart; {}",
                            project.net_name(via.net),
                            mm_against(gap, need.mm),
                            need.text()
                        ),
                        vec![me, ObjectRef::Via { id: via.id }],
                    )
                    .requiring(need.at_most(gap)),
                );
            }
        }
        // Every later board hole: each pair of the board's own holes once.
        for (other, other_ring) in self.mounts.iter().skip(m + 1) {
            let other_kind = CopperObject::hole(other.plated);
            let spacing = limits.clearance(kind, None, other_kind, None);
            let (Some(a), Some(b)) = (ring.bounds, other_ring.bounds) else {
                continue;
            };
            if !a.inflated(spacing + EPS).intersects(&b) {
                continue;
            }
            over()?;
            if ring.closer_than(other_ring, spacing) {
                let need = limits.clearance_need(kind, None, other_kind, None);
                let gap = ring.gap(other_ring, need.mm + 1.0);
                let how = if gap <= EPS {
                    "overlap".to_owned()
                } else {
                    format!("are {} mm apart", mm_against(gap, need.mm))
                };
                issues.push(
                    Finding::new(
                        FindingCode::BoardHoleSpacing,
                        format!("{name} and {}: holes {how}; {}", other.name, need.text()),
                        vec![me, other.object()],
                    )
                    .requiring(need.at_most(gap)),
                );
            }
        }
        Ok(pairs)
    }
}

/// A hole of the board shape (a mounting hole, a plated board slot)
/// closer to another of them or to a via drill than the hole-kind pair
/// rule. Pad holes against them are `board.pad_hole_spacing`.
pub const HOLE_SPACING: &str = crate::diagnostics::FindingCode::BoardHoleSpacing.as_str();

/// What a drilled or routed hole's outline does against the board shape,
/// as the end of a sentence naming it ("is 0.1 mm from the board edge;
/// needs …", "extends beyond the board edge"), with what a hole too close
/// to an edge needs of the fab profile; `None` when it keeps the hole
/// kind's board-edge clearance inside the board.
pub(super) fn edge_text(
    edges: &BoardEdges,
    ring: &CopperRing,
    limits: &BoardRules<'_>,
    kind: CopperObject,
) -> Option<(String, Option<Requirement>)> {
    let edge = limits.clearance(kind, None, CopperObject::BoardEdge, None);
    match edges.verdict(ring, edge) {
        EdgeVerdict::Clear => None,
        EdgeVerdict::Outside => Some(("extends beyond the board edge".into(), None)),
        EdgeVerdict::Inside(kind) => Some((
            format!(
                "lies in {}",
                super::board_edge::EdgeLoopKind::Cutout(kind).describe()
            ),
            None,
        )),
        EdgeVerdict::TooClose { gap, near } => {
            let need = limits.clearance_need(kind, None, CopperObject::BoardEdge, None);
            Some((
                format!(
                    "is {} mm from {}; {}",
                    mm_against(gap, need.mm),
                    near.describe(),
                    need.text()
                ),
                need.at_most(gap),
            ))
        }
    }
}

pub(super) fn delete(project: &mut Project, id: Id) -> Result<()> {
    unlocked(project, id)?;
    project.vias.retain(|v| v.id != id);
    Ok(())
}

pub(super) fn set_tented(project: &mut Project, id: Id, tented: bool) -> Result<()> {
    unlocked(project, id)?.tented = tented;
    Ok(())
}

pub(super) fn set_lock(project: &mut Project, id: Id, locked: bool) -> Result<()> {
    project
        .vias
        .iter_mut()
        .find(|v| v.id == id)
        .ok_or_else(|| Error::NotFound("unknown via".into()))?
        .locked = locked;
    Ok(())
}

/// Document rules over vias: unique identities, known nets,
/// sane geometry inside the board, through vias only for now.
pub(super) fn validate(project: &Project, nets: &BTreeSet<Id>) -> Result<()> {
    let mut ids = BTreeSet::new();
    for via in &project.vias {
        if via.id == 0 || via.id >= project.next_id || !ids.insert(via.id) {
            return invalid("invalid or duplicate via identity");
        }
        check_shape(project, via, nets)?;
    }
    Ok(())
}

/// What every stored via keeps: a net of the board, a ring round its drill,
/// its copper on the board and a through span.
fn check_shape(project: &Project, via: &Via, nets: &BTreeSet<Id>) -> Result<()> {
    if !nets.contains(&via.net) {
        return invalid("via references an unknown net");
    }
    if !point_valid(via.position)
        || !via.drill.is_finite()
        || !via.diameter.is_finite()
        || via.drill <= 0.0
        || via.diameter > MAX_DIAMETER_MM
        || via.diameter <= via.drill + EPS
    {
        return invalid(&format!(
            "via drill must be positive and smaller than its pad, at most {MAX_DIAMETER_MM} mm"
        ));
    }
    let r = via.diameter / 2.0;
    let [w, h] = project.board_size;
    let [x, y] = via.position;
    if x - r < -EPS || y - r < -EPS || x + r > w + EPS || y + r > h + EPS {
        return invalid("via copper must lie inside the board outline");
    }
    if let ViaSpan::Between { .. } = via.span {
        return Err(Error::Unsupported(
            "blind and buried vias are not supported yet; use a through via".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;

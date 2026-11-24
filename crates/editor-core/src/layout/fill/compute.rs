//! The fill computation: a pure, deterministic function of the project.
//!
//! For fill `F` on layer `L`:
//!
//! 1. The allowed area is `F`'s outline within the board outline, shrunk
//!    by the fill-to-board-edge clearance.
//! 2. Obstacles are subtracted: cutouts and slots (edge clearance), board
//!    mounting holes, keepouts that forbid fills on `L`, other-net and
//!    unconnected pads, vias and tracks on `L` grown by the effective
//!    clearance (object pair, net classes, class pair and the fill rule's
//!    own gap, whichever is largest; a class's fill clearance stands in
//!    for its clearance here), every drilled hole grown by the hole
//!    clearance, and the copper of higher-ranking fills of other nets.
//!    Same-net pads and vias get their connection style: solid (nothing
//!    cleared), thermal (a relief gap crossed by spokes) or none (cleared
//!    like another net).
//! 3. Copper narrower than the minimum width is removed (an opening: erode
//!    then grow by half the width). A spoke must cross its relief gap to
//!    copper of the net: past the gap its copper (what stays connected to
//!    its pad anywhere across its width, so a spoke a neighbour's
//!    clearance narrows still counts) has to meet the fill's copper
//!    beyond its end or beside it (along at least half its width), or
//!    another joined pad. A spoke that meets none (its gap opens onto a
//!    clearance, the fill's edge or copper too narrow to keep) joins
//!    nothing and is removed; the fill records how many each pad or via
//!    lost, which Issues reports as information (`fill.spokes_removed`).
//! 4. Hatched fills keep a border and a grid of lines.
//! 5. Islands that no copper of the fill's net touches (by the copper
//!    graph's one joining rule) are removed, and by setting small ones.
//!
//! Obstacles cover their true curves, so the fill boundary is never closer
//! than the rule: circles (vias, drills) and track ends are drawn with
//! their chords outside the true curve; outlines flattened with chords
//! inside it (pads, slots, cutouts, other fills' copper) grow by
//! [`FILL_CHORD_MM`] more.
//!
//! Fills rank by priority (higher first), then by id (lower first). A fill
//! reads the copper of higher-ranking fills, so one pass computes them in
//! rank order; fresh stored copper is reused.
use super::FILL_CHORD_MM;
use super::geom::{self, Region, RegionIndex, Ring};
use crate::diagnostics::{Error, ObjectRef, Result};
use crate::geometry::Point;
use crate::ids::Id;
use crate::layout::connectivity::{islands_joined, track_copper, via_copper};
use crate::layout::copper::CopperRing;
use crate::library::{PadKind, Segment};
use crate::model::fill::{FilledCopper, FilledRegion, Hatch, IslandRemoval, RemovedSpokes};
use crate::model::{CopperLayer, Fill, Project};
use crate::rules::{
    ClearanceSide, CopperObject, DesignRules, NetClassId, PadConnection, ThermalRelief,
};
use serde::Serialize;
use std::collections::BTreeMap;

use super::super::{PlacedPad, placed_pads};

/// Most hatch lines per direction of one fill.
const MAX_HATCH_LINES: usize = 8192;

/// What one pass over a project shares between fills: the placed pads,
/// effective rules and net classes, and the keys and copper computed so
/// far.
pub(crate) struct Ctx<'a> {
    pub(crate) project: &'a Project,
    pub(crate) rules: DesignRules,
    pub(crate) layers: Vec<CopperLayer>,
    pub(crate) pads: Vec<PlacedPad>,
    bridges: Vec<crate::layout::PlacedBridge>,
    /// Board keepouts, then placed footprint keep-outs with the instance
    /// that carries them.
    pub(crate) keepouts: Vec<(crate::model::Keepout, Option<Id>)>,
    /// Each pad's board bounds, by index into `pads`.
    pad_bounds: Vec<Option<[Point; 2]>>,
    classes: BTreeMap<Id, Option<NetClassId>>,
    /// The farthest any pad's fill override reaches (clearance, or relief
    /// gap and spoke), mm.
    override_reach: f64,
    keys: BTreeMap<Id, String>,
    copper: BTreeMap<Id, FilledCopper>,
}

/// The board objects that can change one fill's copper.
#[derive(Default)]
struct Relevant {
    bridges: Vec<usize>,
    pads: Vec<usize>,
    vias: Vec<usize>,
    tracks: Vec<usize>,
    keepouts: Vec<usize>,
    /// Higher-ranking fills on the layer whose copper is cleared (other
    /// nets), by id.
    higher: Vec<Id>,
}

fn arcs(outline: &crate::library::Outline) -> bool {
    outline
        .segments
        .iter()
        .any(|s| matches!(s, Segment::Arc { bulge, .. } if bulge.abs() > 1e-12))
}

/// Whether `a` outranks `b`: it claims overlapping area first.
pub(crate) fn outranks(a: &Fill, b: &Fill) -> bool {
    (a.priority, std::cmp::Reverse(a.id)) > (b.priority, std::cmp::Reverse(b.id))
}

fn bounds_of(points: &[Point]) -> Option<[Point; 2]> {
    let mut it = points.iter();
    let first = *it.next()?;
    let mut b = [first, first];
    for p in it {
        b[0] = [b[0][0].min(p[0]), b[0][1].min(p[1])];
        b[1] = [b[1][0].max(p[0]), b[1][1].max(p[1])];
    }
    Some(b)
}

/// The board's keepouts and every placed footprint keep-out
/// ([`super::super::placed_keepouts`]), the latter with their instance.
pub(crate) fn effective_keepouts(project: &Project) -> Vec<(crate::model::Keepout, Option<Id>)> {
    let mut out: Vec<_> = project.keepouts.iter().map(|k| (k.clone(), None)).collect();
    out.extend(
        super::super::placed_keepouts(project)
            .into_iter()
            .map(|p| (p.keepout, Some(p.instance))),
    );
    out
}

/// A fill's net (when it has stored copper), its layer and the area
/// within which a track or via can change its key ([`key_reaches`]).
pub(crate) type KeyReach = (Option<Id>, CopperLayer, Option<crate::geometry::Bounds>);

/// Each fill's net (`None` for a fill without stored copper, which is
/// never fresh whatever its key), its layer and the board area within
/// which a track or via can change its key ([`Ctx::key`], and so whether
/// its stored copper is fresh): its outline's bounds grown by its reach,
/// as [`Ctx::relevant`] finds the objects it hashes. A fill's key also
/// hashes the keys of the fills it clears on its layer, so a caller treats
/// every fill on that layer as reached once one is.
pub(crate) fn key_reaches(project: &Project) -> Vec<KeyReach> {
    if project.fills.iter().all(|f| f.computed.is_none()) {
        return Vec::new();
    }
    let ctx = Ctx::new(project);
    project
        .fills
        .iter()
        .map(|fill| {
            let reach = ctx.reach(fill);
            let bounds = bounds_of(&fill.outline.flatten(FILL_CHORD_MM)).map(|[lo, hi]| {
                crate::geometry::Bounds {
                    min: [lo[0] - reach, lo[1] - reach],
                    max: [hi[0] + reach, hi[1] + reach],
                }
            });
            (
                fill.net.filter(|_| fill.computed.is_some()),
                fill.layer,
                bounds,
            )
        })
        .collect()
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(project: &'a Project) -> Self {
        let pads = placed_pads(project);
        let pad_bounds = pads
            .iter()
            .map(|p| bounds_of(&p.outline().flatten(FILL_CHORD_MM)))
            .collect();
        let classes = project
            .nets
            .iter()
            .map(|n| (n.id, project.rules.class_of(n.id, &n.name)))
            .collect();
        let override_reach = pads
            .iter()
            .map(|p| p.fill_override.reach_mm())
            .fold(0.0, f64::max);
        Self {
            project,
            rules: project.rules.effective(),
            layers: project.stackup.layers(),
            pads,
            pad_bounds,
            bridges: crate::layout::placed_bridges(project),
            keepouts: effective_keepouts(project),
            classes,
            override_reach,
            keys: BTreeMap::new(),
            copper: BTreeMap::new(),
        }
    }

    fn class(&self, net: Option<Id>) -> Option<NetClassId> {
        net.and_then(|n| self.classes.get(&n).copied().flatten())
    }

    /// The fill's gap to another net's `object` of `class`.
    fn clearance(&self, fill: &Fill, object: CopperObject, class: Option<NetClassId>) -> f64 {
        self.clearance_over(fill, object, class, None)
    }

    /// [`Self::clearance`] with `own` (a pad's fill override) in place of
    /// the fill's own clearance.
    fn clearance_over(
        &self,
        fill: &Fill,
        object: CopperObject,
        class: Option<NetClassId>,
        own: Option<f64>,
    ) -> f64 {
        let r = &self.project.rules;
        let side = |object, class| ClearanceSide { object, class };
        let pair = r
            .clearance(
                side(CopperObject::Fill, self.class(fill.net)),
                side(object, class),
            )
            .value;
        let own = own
            .or(fill.settings.clearance)
            .unwrap_or(self.rules.fill.clearance);
        let copper = object.is_copper();
        if copper { pair.max(own) } else { pair }
    }

    fn thermal(&self, fill: &Fill) -> ThermalRelief {
        fill.settings.thermal.unwrap_or(self.rules.fill.thermal)
    }

    /// The relief of a round (`round`) or other pad or via when no part or
    /// pad override states one: the fill's own setting (one angle for all
    /// its pads), else the rule's, whose round-pad angle, when set, turns
    /// the spokes of round pads and vias.
    fn thermal_for(&self, fill: &Fill, round: bool) -> ThermalRelief {
        let mut t = self.thermal(fill);
        if fill.settings.thermal.is_none()
            && round
            && let Some(angle) = self.rules.fill.thermal_angle_round
        {
            t.angle_deg = angle;
        }
        t
    }

    /// How a same-net pad (`Some(kind)`) or via (`None`) joins the fill.
    fn connection(&self, fill: &Fill, pad: Option<PadKind>) -> PadConnection {
        let c = &fill.settings.connections;
        let own = match pad {
            Some(PadKind::PlatedThrough) => c.through_hole,
            Some(_) => c.smd,
            None => c.via,
        };
        own.or_else(|| {
            self.class(fill.net)
                .and_then(|id| self.project.rules.class(id))
                .and_then(|class| class.fill_connection)
        })
        .unwrap_or(self.rules.fill.connection)
    }

    /// An upper bound on how far any object can reach into a fill.
    fn reach(&self, fill: &Fill) -> f64 {
        let r = &self.project.rules;
        let mut most = fill.settings.clearance.unwrap_or(self.rules.fill.clearance);
        for a in CopperObject::ALL {
            for b in CopperObject::ALL {
                most = most.max(self.rules.copper.clearance.get(a, b));
            }
        }
        for c in &r.net_classes {
            most = most.max(c.clearance.unwrap_or(0.0));
            most = most.max(c.fill_clearance.unwrap_or(0.0));
        }
        for p in &r.class_clearances {
            most = most.max(p.mm);
        }
        let t = self.thermal(fill);
        most.max(t.gap + t.spoke_width).max(self.override_reach) + 2.0 * FILL_CHORD_MM + 0.01
    }

    fn fill(&self, id: Id) -> Result<&'a Fill> {
        self.project
            .fill(id)
            .ok_or_else(|| Error::NotFound(format!("fill.missing: no fill {id}")))
    }

    fn relevant(&self, fill: &Fill) -> Relevant {
        let p = self.project;
        let layer = fill.layer;
        let Some(own) = bounds_of(&fill.outline.flatten(FILL_CHORD_MM)) else {
            return Relevant::default();
        };
        let reach = self.reach(fill);
        let near = |b: Option<[Point; 2]>| b.is_some_and(|b| geom::overlaps(own, b, reach));
        let mut out = Relevant::default();
        for (i, pad) in self.pads.iter().enumerate() {
            // A bare hole clears fills on every layer.
            let holes = pad.pad.hole.is_some() || !pad.pad.vias.is_empty();
            if (pad.on_copper(layer) || holes) && near(self.pad_bounds.get(i).copied().flatten()) {
                out.pads.push(i);
            }
        }
        for (i, v) in p.vias.iter().enumerate() {
            let r = v.diameter.max(v.drill) / 2.0;
            let b = [
                [v.position[0] - r, v.position[1] - r],
                [v.position[0] + r, v.position[1] + r],
            ];
            if v.on(layer, &self.layers) && near(Some(b)) {
                out.vias.push(i);
            }
        }
        for (i, t) in p.tracks.iter().enumerate() {
            let h = t.width / 2.0;
            let b = [
                [t.start[0].min(t.end[0]) - h, t.start[1].min(t.end[1]) - h],
                [t.start[0].max(t.end[0]) + h, t.start[1].max(t.end[1]) + h],
            ];
            if t.layer == layer && near(Some(b)) {
                out.tracks.push(i);
            }
        }
        for (i, bridge) in self.bridges.iter().enumerate() {
            if bridge.layer == layer && near(bounds_of(&bridge.outline.flatten(FILL_CHORD_MM))) {
                out.bridges.push(i);
            }
        }
        for (i, (k, _)) in self.keepouts.iter().enumerate() {
            if k.on(layer) && k.forbid.fills && near(bounds_of(&k.outline.flatten(FILL_CHORD_MM))) {
                out.keepouts.push(i);
            }
        }
        for other in &p.fills {
            let same_net = fill.net.is_some() && other.net == fill.net;
            if other.id != fill.id
                && other.layer == layer
                && !same_net
                && outranks(other, fill)
                && near(bounds_of(&other.outline.flatten(FILL_CHORD_MM)))
            {
                out.higher.push(other.id);
            }
        }
        out
    }

    /// The cache key of fill `id`: a hash of the fill and of everything
    /// near it that its copper depends on. Objects out of its reach, on
    /// other layers or in fills it does not clear leave the key alone, so
    /// an edit elsewhere does not make it stale.
    pub(crate) fn key(&mut self, id: Id) -> Result<String> {
        if let Some(k) = self.keys.get(&id) {
            return Ok(k.clone());
        }
        let fill = self.fill(id)?;
        let rel = self.relevant(fill);
        let mut higher = Vec::new();
        for h in &rel.higher {
            higher.push((*h, self.key(*h)?));
        }
        let p = self.project;
        let net_name = |n: Option<Id>| n.map(|n| p.net_name(n));
        // Check-only rules (etch hazards) leave the copper and so the key.
        let rules = p.rules.without_checks();
        #[derive(Serialize)]
        struct Inputs<'b> {
            fill: (
                &'b Option<Id>,
                Option<&'b str>,
                &'b CopperLayer,
                &'b crate::library::Outline,
                u32,
                &'b crate::model::fill::FillSettings,
            ),
            rules: &'b crate::rules::ProjectRules,
            board: (&'b Point, &'b crate::model::BoardShape),
            layers: &'b [CopperLayer],
            pads: Vec<(&'b PlacedPad, Option<&'b str>)>,
            vias: Vec<&'b crate::model::Via>,
            tracks: Vec<(&'b crate::model::Track, Option<&'b str>)>,
            keepouts: Vec<&'b crate::model::Keepout>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            bridges: Vec<(&'b crate::layout::PlacedBridge, Vec<Option<NetClassId>>)>,
            higher: Vec<(Id, String)>,
        }
        let inputs = Inputs {
            bridges: rel
                .bridges
                .iter()
                .filter_map(|i| self.bridges.get(*i))
                .map(|b| (b, b.nets.iter().map(|n| self.class(Some(*n))).collect()))
                .collect(),
            fill: (
                &fill.net,
                net_name(fill.net),
                &fill.layer,
                &fill.outline,
                fill.priority,
                &fill.settings,
            ),
            rules: &rules,
            board: (&p.board_size, &p.board_shape),
            layers: &self.layers,
            pads: rel
                .pads
                .iter()
                .filter_map(|i| self.pads.get(*i))
                .map(|pad| (pad, net_name(pad.net)))
                .collect(),
            vias: rel.vias.iter().filter_map(|i| p.vias.get(*i)).collect(),
            tracks: rel
                .tracks
                .iter()
                .filter_map(|i| p.tracks.get(*i))
                .map(|t| (t, net_name(Some(t.net))))
                .collect(),
            keepouts: rel
                .keepouts
                .iter()
                .filter_map(|i| self.keepouts.get(*i).map(|(k, _)| k))
                .collect(),
            higher,
        };
        let key = super::fnv_hex(&serde_json::to_vec(&inputs)?);
        self.keys.insert(id, key.clone());
        Ok(key)
    }

    /// Use `copper` for fill `id` in this pass (a host cache hit).
    pub(crate) fn seed(&mut self, id: Id, copper: FilledCopper) {
        self.copper.insert(id, copper);
    }

    /// The copper of fill `id`: stored copper when its key is current,
    /// otherwise computed (higher-ranking fills first).
    pub(crate) fn copper(&mut self, id: Id, reuse: bool) -> Result<FilledCopper> {
        if let Some(c) = self.copper.get(&id) {
            return Ok(c.clone());
        }
        let key = self.key(id)?;
        let fill = self.fill(id)?;
        if reuse
            && let Some(stored) = &fill.computed
            && stored.key == key
        {
            self.copper.insert(id, stored.clone());
            return Ok(stored.clone());
        }
        let c = self.compute(fill, key)?;
        self.copper.insert(id, c.clone());
        Ok(c)
    }

    fn compute(&mut self, fill: &'a Fill, key: String) -> Result<FilledCopper> {
        let tol = FILL_CHORD_MM;
        let p = self.project;
        let rel = self.relevant(fill);
        let fill_net = fill.net;
        let same = |net: Option<Id>| fill_net.is_some() && net == fill_net;

        // 1. Allowed area.
        let Some(outline) = geom::ccw(fill.outline.flatten(tol)) else {
            return Ok(empty(key));
        };
        let board = p.board();
        let Some(outer) = geom::ccw(board.outer.flatten(tol)) else {
            return Ok(empty(key));
        };
        let edge = self.clearance(fill, CopperObject::BoardEdge, None);
        let shrink = edge + if arcs(&board.outer) { tol } else { 0.0 };
        let mut inside = vec![vec![outer]];
        if shrink > 0.0 {
            inside = geom::offset(&inside, -shrink, tol);
        }
        let area = geom::intersect(&[vec![outline]], &inside);
        if area.is_empty() {
            return Ok(empty(key));
        }

        // 2. Obstacles.
        let mut rings: Vec<Ring> = Vec::new();
        let mut regions: Vec<Region> = Vec::new();
        let mut reliefs: Vec<Relief> = Vec::new();
        // Plated pad and mounting holes keep the plated hole clearance,
        // unplated ones the unplated hole clearance; via drills keep the
        // via hole clearance.
        let hole_gap = |plated: bool| self.clearance(fill, CopperObject::hole(plated), None);
        let via_hole_gap = self.clearance(fill, CopperObject::ViaHole, None);
        for cut in &board.cutouts {
            // A plated slot is an edge and a plated hole: the larger rule.
            let gap = match cut.kind {
                crate::model::board::CutoutKind::Slot { plated: true } => edge.max(hole_gap(true)),
                _ => edge,
            };
            if let Some(ring) = geom::ccw(cut.outline.flatten(tol)) {
                regions.extend(geom::offset(&[vec![ring]], gap + tol, tol));
            }
        }
        for h in &board.holes {
            rings.push(geom::circle(h.at, h.drill / 2.0 + hole_gap(h.plated), tol));
            if h.plated
                && let Some(d) = h.pad_diameter
            {
                let gap = self.clearance(fill, CopperObject::Pad, None);
                rings.push(geom::circle(h.at, d / 2.0 + gap, tol));
            }
        }
        for (k, _) in rel.keepouts.iter().filter_map(|i| self.keepouts.get(*i)) {
            if let Some(ring) = geom::ccw(k.outline.flatten(tol)) {
                if arcs(&k.outline) {
                    regions.extend(geom::offset(&[vec![ring]], tol, tol));
                } else {
                    rings.push(ring);
                }
            }
        }
        for &i in &rel.pads {
            let Some(pad) = self.pads.get(i) else {
                continue;
            };
            let on = pad.on_copper(fill.layer);
            let o = &pad.fill_override;
            let connection = if same(pad.net) && on {
                o.connection
                    .unwrap_or_else(|| self.connection(fill, Some(pad.pad.kind)))
            } else {
                PadConnection::None
            };
            let round = matches!(pad.pad.shape, crate::library::PadShape::Circle { .. });
            let thermal = o.thermal(self.thermal_for(fill, round));
            let frame = pad.pad_transform();
            if on {
                match connection {
                    PadConnection::None => {
                        let class = self.class(pad.net);
                        let gap =
                            self.clearance_over(fill, CopperObject::Pad, class, o.clearance_mm())
                                + tol;
                        if let Some(shape) = pad.pad.shape.grown(gap, tol) {
                            rings.extend(geom::ccw(shape.transformed(&frame).flatten(tol)));
                        }
                    }
                    PadConnection::Solid => {}
                    PadConnection::Thermal => {
                        if let Some(shape) = pad.pad.shape.grown(thermal.gap + tol, tol)
                            && let Some(ring) = geom::ccw(shape.transformed(&frame).flatten(tol))
                        {
                            let reach = pad.pad.shape.extent();
                            let length =
                                reach[0].hypot(reach[1]) + thermal.gap + thermal.spoke_width;
                            let spokes = spokes(
                                frame.apply([0.0, 0.0]),
                                frame.rotation_deg,
                                frame.mirror,
                                length,
                                &thermal,
                            );
                            let own = geom::ccw(
                                pad.pad.shape.outline(tol).transformed(&frame).flatten(tol),
                            );
                            regions.extend(relief(&ring, &spokes));
                            reliefs.push(Relief {
                                object: ObjectRef::Pad {
                                    instance: pad.instance,
                                    pad: pad.pad.id,
                                },
                                pad: own,
                                ring,
                                spokes,
                            });
                        }
                    }
                }
            }
            for hole in pad.holes() {
                // A joined plated hole is drilled out of the fill; any
                // other hole keeps the hole clearance.
                let joined = hole.plated && on && connection != PadConnection::None;
                let gap = if joined { 0.0 } else { hole_gap(hole.plated) };
                rings.extend(grown_shape(&hole.shape, gap, tol));
            }
        }
        let thermal = self.thermal_for(fill, true);
        for &i in &rel.vias {
            let Some(v) = p.vias.get(i) else {
                continue;
            };
            let connection = if same(Some(v.net)) {
                self.connection(fill, None)
            } else {
                PadConnection::None
            };
            match connection {
                PadConnection::None => {
                    let gap = self.clearance(fill, CopperObject::Via, self.class(Some(v.net)));
                    let r = (v.diameter / 2.0 + gap).max(v.drill / 2.0 + via_hole_gap);
                    rings.push(geom::circle(v.position, r, tol));
                }
                PadConnection::Solid => {
                    rings.push(geom::circle(v.position, v.drill / 2.0, tol));
                }
                PadConnection::Thermal => {
                    let ring = geom::circle(v.position, v.diameter / 2.0 + thermal.gap, tol);
                    let length = v.diameter / 2.0 + thermal.gap + thermal.spoke_width;
                    let spokes = spokes(v.position, 0.0, false, length, &thermal);
                    regions.extend(relief(&ring, &spokes));
                    reliefs.push(Relief {
                        object: ObjectRef::Via { id: v.id },
                        pad: Some(geom::circle(v.position, v.diameter / 2.0, tol)),
                        ring,
                        spokes,
                    });
                    rings.push(geom::circle(v.position, v.drill / 2.0, tol));
                }
            }
        }
        for b in rel.bridges.iter().filter_map(|i| self.bridges.get(*i)) {
            if let Some(ring) = geom::ccw(b.outline.flatten(tol)) {
                let gap = b
                    .nets
                    .iter()
                    .map(|n| self.clearance(fill, CopperObject::Track, self.class(Some(*n))))
                    .fold(self.clearance(fill, CopperObject::Track, None), f64::max);
                regions.extend(geom::offset(
                    &[vec![ring]],
                    gap + if arcs(&b.outline) { tol } else { 0.0 },
                    tol,
                ));
            }
        }
        for t in rel.tracks.iter().filter_map(|i| p.tracks.get(*i)) {
            if same(Some(t.net)) {
                continue;
            }
            let gap = self.clearance(fill, CopperObject::Track, self.class(Some(t.net)));
            rings.push(geom::stadium(t.start, t.end, t.width / 2.0 + gap, tol));
        }
        for h in rel.higher.clone() {
            let other = self.copper(h, true)?;
            let net = self.fill(h)?.net;
            let gap = self.clearance(fill, CopperObject::Fill, self.class(net)) + tol;
            let theirs: Vec<Region> = other
                .regions
                .iter()
                .map(|r| {
                    let mut region = vec![r.outer.clone()];
                    region.extend(r.holes.iter().cloned());
                    region
                })
                .collect();
            regions.extend(geom::offset(&theirs, gap, tol));
        }
        regions.extend(rings.into_iter().map(|r| vec![r]));
        let raw = geom::difference(&area, &regions);

        // 3. Minimum width.
        let width = fill.settings.width_min.unwrap_or(self.rules.fill.width_min);
        let half = width / 2.0 - 1e-4;
        let mut copper = if half > 0.0 {
            let eroded = geom::offset(&raw, -half, tol);
            let opened = geom::offset(&eroded, half, tol);
            geom::intersect(&opened, &raw)
        } else {
            raw
        };
        // Spokes that cross their gap to no copper of the net are removed,
        // and recorded per pad or via for Issues.
        let (loose, removed_spokes) = loose_spokes(&copper, &reliefs);
        if !loose.is_empty() {
            copper = geom::difference(&copper, &loose);
        }

        // 4. Hatch.
        if let Some(h) = fill.settings.hatch {
            copper = hatch(&copper, &h, tol)?;
        }

        // 5. Islands. One is joined when copper of the fill's net on its
        // layer touches it, by the copper graph's one joining rule
        // (`connectivity::islands_joined`), which also joins the fresh
        // fill's islands on the board.
        let (unconnected, area_min) = match fill.settings.islands {
            IslandRemoval::Never => (false, 0.0),
            IslandRemoval::Unconnected => (true, self.rules.fill.island_area_min),
            IslandRemoval::BelowArea(a) => (true, a.max(self.rules.fill.island_area_min)),
        };
        let mut connected = vec![false; copper.len()];
        // A fill without a net has nothing to connect to.
        let unconnected = match fill_net {
            Some(net) if unconnected => {
                let index = RegionIndex::new(&copper);
                let mut mark = |ring: &CopperRing| {
                    for r in islands_joined(&index, ring) {
                        if let Some(flag) = connected.get_mut(r as usize) {
                            *flag = true;
                        }
                    }
                };
                for pad in rel.pads.iter().filter_map(|i| self.pads.get(*i)) {
                    if pad.net == Some(net) && pad.on_copper(fill.layer) {
                        mark(&CopperRing::of_placed_pad(pad));
                    }
                }
                for v in rel.vias.iter().filter_map(|i| p.vias.get(*i)) {
                    if v.net == net {
                        mark(&via_copper(v));
                    }
                }
                for t in rel.tracks.iter().filter_map(|i| p.tracks.get(*i)) {
                    if t.net == net {
                        mark(&track_copper(t));
                    }
                }
                true
            }
            _ => false,
        };
        let mut kept = Vec::new();
        let mut removed = 0u32;
        for (i, region) in copper.into_iter().enumerate() {
            let drop = (unconnected && !connected.get(i).copied().unwrap_or(false))
                || (area_min > 0.0 && geom::region_area(&region) < area_min);
            if drop {
                removed = removed.saturating_add(1);
                continue;
            }
            kept.push(region);
        }
        let regions = kept
            .into_iter()
            .filter_map(|mut region| {
                if region.is_empty() {
                    return None;
                }
                let outer = region.remove(0);
                Some(FilledRegion {
                    outer,
                    holes: region,
                })
            })
            .collect();
        Ok(FilledCopper {
            key,
            regions,
            removed_islands: removed,
            removed_spokes,
        })
    }
}

fn empty(key: String) -> FilledCopper {
    FilledCopper {
        key,
        regions: Vec::new(),
        removed_islands: 0,
        removed_spokes: Vec::new(),
    }
}

/// A drilled hole (circle or slot) grown by `by`, covering its true curve.
fn grown_shape(shape: &crate::geometry::Shape, by: f64, tol: f64) -> Vec<Ring> {
    use crate::geometry::Shape;
    match shape {
        Shape::Circle { center, radius } => vec![geom::circle(*center, radius + by, tol)],
        other => {
            let Some(ring) = geom::ccw(other.flatten(tol)) else {
                return Vec::new();
            };
            // Every ring of every piece: an outer ring and, clockwise, the
            // holes in it (none, for the convex holes there are).
            geom::offset(&[vec![ring]], by + 2.0 * tol, tol)
                .into_iter()
                .flatten()
                .collect()
        }
    }
}

/// The spoke directions on the board, degrees: `count` evenly from the
/// relief angle in the pad frame, turned and mirrored with the pad.
fn spoke_angles(rotation_deg: f64, mirror: bool, t: &ThermalRelief) -> Vec<f64> {
    let n = t.spoke_count.clamp(1, 8);
    let sign = if mirror { -1.0 } else { 1.0 };
    (0..n)
        .map(|k| rotation_deg + sign * (t.angle_deg + 360.0 * f64::from(k) / f64::from(n)))
        .collect()
}

/// One thermal spoke: its centre line from the pad's centre outward and
/// its bar.
struct Spoke {
    from: Point,
    to: Point,
    width: f64,
    bar: Ring,
}

/// A thermal relief: the joined pad or via, its copper outline, the
/// ring it clears (the pad grown by the relief gap) and its spokes.
struct Relief {
    object: ObjectRef,
    pad: Option<Ring>,
    ring: Ring,
    spokes: Vec<Spoke>,
}

fn spokes(
    center: Point,
    rotation_deg: f64,
    mirror: bool,
    length: f64,
    t: &ThermalRelief,
) -> Vec<Spoke> {
    spoke_angles(rotation_deg, mirror, t)
        .into_iter()
        .filter_map(|a| {
            let (s, c) = a.to_radians().sin_cos();
            let to = [center[0] + c * length, center[1] + s * length];
            Some(Spoke {
                from: center,
                to,
                width: t.spoke_width,
                bar: geom::bar(center, to, t.spoke_width)?,
            })
        })
        .collect()
}

/// Whether `p` lies inside `ring` (even-odd).
fn inside(p: Point, ring: &[Point]) -> bool {
    let mut odd = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                odd = !odd;
            }
        }
    }
    odd
}

/// The relief obstacle: the grown pad less its spokes.
fn relief(ring: &Ring, spokes: &[Spoke]) -> Vec<Region> {
    let bars: Vec<Ring> = spokes.iter().map(|s| s.bar.clone()).collect();
    geom::difference_rings(&[vec![ring.clone()]], &bars)
}

/// Samples along a spoke, per spoke width.
const SPOKE_SAMPLES_PER_WIDTH: f64 = 4.0;
/// Most samples along one spoke.
const MAX_SPOKE_SAMPLES: usize = 1024;
/// Sample lanes across a spoke's width: a neighbour's clearance may cut a
/// spoke down to part of its width, and the rest still carries copper.
const SPOKE_LANES: usize = 5;
/// Samples along which copper must lie beside a spoke for it to join
/// (half a spoke width): a spoke that only grazes a corner joins nothing.
const SPOKE_CONTACT_SAMPLES: usize = 2;

/// The spokes (less their own pad) that cross their relief gap to no
/// copper of the fill's net. A spoke's own copper is the copper within its
/// bar that stays connected to its pad, sampled along the spoke and in
/// lanes across its width, so a spoke that a neighbour's clearance narrows
/// keeps the copper beside the cut. Past its ring, that copper must meet
/// the fill's copper beyond the spoke's end or beside it (along at least
/// half its width), or another joined pad. A spoke whose gap opens onto a
/// clearance, the fill's edge or copper removed as too narrow joins
/// nothing and is dropped. Also returns how many spokes each pad or via
/// lost.
fn loose_spokes(copper: &[Region], reliefs: &[Relief]) -> (Vec<Region>, Vec<RemovedSpokes>) {
    let mut removed = Vec::new();
    if reliefs.is_empty() || copper.is_empty() {
        return (Vec::new(), removed);
    }
    let index = RegionIndex::new(copper);
    let rings: Vec<Option<[Point; 2]>> = reliefs
        .iter()
        .map(|r| geom::bounds(std::iter::once(&r.ring)))
        .collect();
    let mut loose = Vec::new();
    for (k, relief) in reliefs.iter().enumerate() {
        let mut lost = 0u32;
        for spoke in &relief.spokes {
            let Some(b) = geom::bounds(std::iter::once(&spoke.bar)) else {
                continue;
            };
            let near: Vec<&Ring> = reliefs
                .iter()
                .zip(&rings)
                .enumerate()
                .filter(|(j, (_, rb))| {
                    *j != k && rb.is_some_and(|rb| geom::overlaps(b, rb, spoke.width))
                })
                .filter_map(|(_, (r, _))| r.pad.as_ref())
                .collect();
            let len = crate::geometry::distance(spoke.from, spoke.to);
            if len <= 0.0 {
                continue;
            }
            let SpokeReach::Loose { reach: end } = spoke_reach(&index, relief, spoke, &near, len)
            else {
                continue;
            };
            let dir = [
                (spoke.to[0] - spoke.from[0]) / len,
                (spoke.to[1] - spoke.from[1]) / len,
            ];
            let Some(bar) = geom::bar(
                spoke.from,
                [spoke.from[0] + dir[0] * end, spoke.from[1] + dir[1] * end],
                spoke.width,
            ) else {
                continue;
            };
            let bar = vec![vec![bar]];
            match &relief.pad {
                Some(pad) => loose.extend(geom::difference(&bar, &[vec![pad.clone()]])),
                None => loose.extend(bar),
            }
            lost = lost.saturating_add(1);
        }
        if lost > 0 {
            removed.push(RemovedSpokes {
                object: relief.object,
                spokes: lost,
            });
        }
    }
    (loose, removed)
}

/// Whether one spoke joins the net's copper, or how far from its pad's
/// centre its loose copper runs.
enum SpokeReach {
    Joins,
    Loose { reach: f64 },
}

/// [`loose_spokes`] for one spoke of length `len`: samples along the spoke
/// and across its width, flooded from the pad through the samples that
/// hold copper. Copper the flood never reaches (beyond a cut) belongs to
/// something the spoke does not reach.
fn spoke_reach(
    index: &RegionIndex,
    relief: &Relief,
    spoke: &Spoke,
    near: &[&Ring],
    len: f64,
) -> SpokeReach {
    let dir = [
        (spoke.to[0] - spoke.from[0]) / len,
        (spoke.to[1] - spoke.from[1]) / len,
    ];
    let across = [-dir[1], dir[0]];
    let side = spoke.width / 2.0 + (spoke.width / 8.0).max(0.005);
    let n =
        ((len / spoke.width * SPOKE_SAMPLES_PER_WIDTH).ceil() as usize).clamp(1, MAX_SPOKE_SAMPLES);
    let step = len / n as f64;
    // A point `t` along the spoke and `o` across it.
    let at = |t: f64, o: f64| {
        [
            spoke.from[0] + dir[0] * t + across[0] * o,
            spoke.from[1] + dir[1] * t + across[1] * o,
        ]
    };
    // Lane `j`'s offset across the spoke, within its bar.
    let lane = |j: usize| ((j as f64 + 0.5) / SPOKE_LANES as f64 - 0.5) * spoke.width;
    let in_near = |p: Point| near.iter().any(|pad| inside(p, pad));
    let own = |p: Point| relief.pad.as_ref().is_some_and(|pad| inside(p, pad));
    let meets = |p: Point| !inside(p, &relief.ring) && (index.region_at(p).is_some() || in_near(p));
    let ringed: Vec<bool> = (0..=n)
        .map(|i| inside(at(step * i as f64, 0.0), &relief.ring))
        .collect();
    // A spoke that never leaves its ring crosses no gap to judge.
    if ringed.iter().skip(1).all(|r| *r) {
        return SpokeReach::Joins;
    }
    // Most spokes join along their centre line: walk out from the ring
    // while its copper lasts. Only a spoke cut there is flooded across its
    // width.
    let mut beside = 0usize;
    let mut out = 0usize;
    for i in 1..=n {
        if ringed.get(i).copied().unwrap_or(true) {
            continue;
        }
        let t = step * i as f64;
        let c = at(t, 0.0);
        out += 1;
        if in_near(c) {
            return SpokeReach::Joins;
        }
        if index.region_at(c).is_none() {
            // The first sample may sit on the ring's own edge.
            if out > 1 {
                break;
            }
            continue;
        }
        if meets(at(t, side)) || meets(at(t, -side)) {
            beside += 1;
            if beside >= SPOKE_CONTACT_SAMPLES {
                return SpokeReach::Joins;
            }
        }
        if i == n && meets(at(len + side, 0.0)) {
            return SpokeReach::Joins;
        }
    }
    // How far the spoke runs, from the pad's centre: its ring, then the
    // copper the flood reaches.
    let mut reach = (1..=n)
        .filter(|i| ringed.get(*i).copied().unwrap_or(false))
        .map(|i| step * i as f64)
        .fold(0.0, f64::max);
    let cell = |i: usize, j: usize| i * SPOKE_LANES + j;
    let mut holds: Vec<Option<bool>> = vec![None; (n + 1) * SPOKE_LANES];
    let mut seen = vec![false; (n + 1) * SPOKE_LANES];
    let mut copper_at = |i: usize, j: usize| -> bool {
        let Some(slot) = holds.get_mut(cell(i, j)) else {
            return false;
        };
        *slot.get_or_insert_with(|| {
            let p = at(step * i as f64, lane(j));
            own(p) || index.region_at(p).is_some() || in_near(p)
        })
    };
    // Start in the pad (its centre); a via's disc or a pad without an
    // outline starts at the first samples.
    let first = usize::from(relief.pad.is_none());
    let mut stack = Vec::new();
    for j in 0..SPOKE_LANES {
        if copper_at(first, j)
            && let Some(s) = seen.get_mut(cell(first, j))
        {
            *s = true;
            stack.push((first, j));
        }
    }
    let mut beside = 0usize;
    while let Some((i, j)) = stack.pop() {
        let t = step * i as f64;
        let p = at(t, lane(j));
        if !inside(p, &relief.ring) && !own(p) {
            reach = reach.max(t);
            if in_near(p) {
                return SpokeReach::Joins;
            }
            // Copper beside the spoke's outer lanes, or beyond its end.
            let mut probes = Vec::new();
            if j == 0 {
                probes.push(at(t, -side));
            }
            if j + 1 == SPOKE_LANES {
                probes.push(at(t, side));
            }
            for probe in probes {
                if meets(probe) {
                    beside += 1;
                    if beside >= SPOKE_CONTACT_SAMPLES {
                        return SpokeReach::Joins;
                    }
                }
            }
            if i == n && meets(at(len + side, lane(j))) {
                return SpokeReach::Joins;
            }
        }
        let mut next = Vec::with_capacity(4);
        if i < n {
            next.push((i + 1, j));
        }
        if i > 0 {
            next.push((i - 1, j));
        }
        if j + 1 < SPOKE_LANES {
            next.push((i, j + 1));
        }
        if j > 0 {
            next.push((i, j - 1));
        }
        for (a, b) in next {
            if seen.get(cell(a, b)).copied().unwrap_or(true) || !copper_at(a, b) {
                continue;
            }
            if let Some(s) = seen.get_mut(cell(a, b)) {
                *s = true;
            }
            stack.push((a, b));
        }
    }
    SpokeReach::Loose {
        reach: (reach + step).min(len),
    }
}

/// A hatched fill: a border of the hatch width and a grid of lines.
fn hatch(copper: &[Region], h: &Hatch, tol: f64) -> Result<Vec<Region>> {
    let Some([min, max]) = geom::bounds(copper.iter().flatten()) else {
        return Ok(Vec::new());
    };
    let pitch = h.width + h.gap;
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let radius = (max[0] - min[0]).hypot(max[1] - min[1]) / 2.0 + pitch;
    let count = (2.0 * radius / pitch).ceil();
    if !count.is_finite() || count as usize > MAX_HATCH_LINES {
        return Err(Error::Limit(format!(
            "fill.limit: the hatch needs more than {MAX_HATCH_LINES} lines; widen its pitch"
        )));
    }
    let count = count as usize;
    let mut lines = Vec::new();
    for turn in [0.0, 90.0] {
        let (s, c) = (h.angle_deg + turn).to_radians().sin_cos();
        let (along, across) = ([c, s], [-s, c]);
        for k in 0..=count {
            let off = -radius + k as f64 * pitch;
            let mid = [center[0] + across[0] * off, center[1] + across[1] * off];
            lines.extend(geom::bar(
                [mid[0] - along[0] * radius, mid[1] - along[1] * radius],
                [mid[0] + along[0] * radius, mid[1] + along[1] * radius],
                h.width,
            ));
        }
    }
    let grid = geom::union(&lines);
    let inner = geom::offset(copper, -h.width, tol);
    let border = geom::difference(copper, &inner);
    let mut parts = geom::intersect(copper, &grid);
    parts.extend(border);
    Ok(geom::union_regions(&parts))
}

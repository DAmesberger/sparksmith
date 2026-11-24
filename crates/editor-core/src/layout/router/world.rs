//! What the router queries: the board's copper (the obstacle index),
//! keepouts, the board edge and cutouts, and a speculative overlay of what
//! a route step adds and moves. Collision tests use the same copper rings
//! and the same rules (`limits::BoardRules`) as `layout.route_apply`, so a
//! proposal the router finds clear is one the engine accepts.
use super::hull::{Facets, Hull};
use crate::diagnostics::ObjectRef;
use crate::geometry::{Bounds, Point};
use crate::ids::Id;
use crate::layout::CopperGraph;
use crate::layout::board_edge::{BoardEdges, EdgeLoopKind, EdgeVerdict};
use crate::layout::connectivity::keeps_joint;
use crate::layout::grid::BoundsGrid;
use crate::layout::limits::BoardRules;
use crate::layout::obstacles::{ObstacleKind, Obstacles};
use crate::layout::physical::{self, HoleOwner};
use crate::layout::posture::Posture;
use crate::layout::{CURVE_SLACK_MM, CopperRing, placed_keepouts};
use crate::model::{BoardShape, CopperLayer, Keepout, Project, Track, Via};
use crate::rules::CopperObject;
use crate::rules::signal::{DiffPair, DiffPairId};
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

/// How far past the rule a hull is grown, mm: room for rounding and for
/// the conservative judgement of flattened curves.
pub(super) const MARGIN: f64 = 1e-4 + 2.0 * CURVE_SLACK_MM;

/// The board as the router sees it, built once per document revision
/// (the desktop caches it; a command builds it once).
pub struct RouteIndex {
    pub(super) obstacles: Obstacles,
    /// Drilled holes of pads and mounting holes: they pass through every
    /// copper layer and keep their own clearances (plated, unplated).
    holes: Vec<Hole>,
    hole_grid: BoundsGrid,
    keepouts: Vec<(Keepout, CopperRing)>,
    cutouts: Vec<CopperRing>,
    edges: BoardEdges,
    /// The revision's copper graph: where shoved chains end and which
    /// tracks follow a moved via. Built on first use unless given.
    graph: OnceLock<Arc<CopperGraph>>,
}

impl RouteIndex {
    /// The index of `project`, sharing `graph`, the copper graph of the
    /// same revision (the engine's or the editor's).
    pub fn with_graph(project: &Project, graph: Arc<CopperGraph>) -> Self {
        let index = Self::of(project);
        let _ = index.graph.set(graph);
        index
    }

    pub fn of(project: &Project) -> Self {
        Self::filtered(project, &|_| true)
    }

    /// The index of `project` for queries near `area` alone: the copper
    /// and holes within reach of it, in [`Self::of`]'s order, and every
    /// keepout, cutout and edge, so a query of a piece within `area`
    /// finds what the whole board's index finds, in the same order.
    pub(crate) fn near(project: &Project, area: &Bounds) -> Self {
        let rules = BoardRules::of(project);
        // The obstacle grid's reach and the hole grid's, and a margin.
        let reach = crate::layout::copper::check_reach(&rules).max(rules.max_clearance()) + 1.0;
        let area = *area;
        Self::filtered(project, &move |b: &Bounds| {
            crate::layout::grid::within(&area, b, reach)
        })
    }

    fn filtered(project: &Project, keep: &dyn Fn(&Bounds) -> bool) -> Self {
        let ring = |outline: &crate::library::Outline| {
            let ring = outline.flatten(CURVE_SLACK_MM);
            CopperRing {
                bounds: Bounds::of(ring.iter().copied()),
                ring,
                slack: CURVE_SLACK_MM,
                spine: None,
            }
        };
        let keepouts = project
            .keepouts
            .iter()
            .cloned()
            .chain(placed_keepouts(project).into_iter().map(|k| k.keepout))
            .filter(|k| k.forbid.tracks || k.forbid.vias)
            .map(|k| {
                let r = ring(&k.outline);
                (k, r)
            })
            .filter(|(_, r)| r.ring.len() >= 3)
            .collect();
        let cutouts = match project.board_shape {
            BoardShape::Rectangle => Vec::new(),
            BoardShape::Sketch(_) => project
                .board()
                .cutouts
                .iter()
                .map(|c| ring(&c.outline))
                .filter(|r| r.ring.len() >= 3)
                .collect(),
        };
        let placed = crate::layout::obstacles::kept_pads(project, keep);
        let holes: Vec<Hole> = Hole::all(project, &placed)
            .into_iter()
            .filter(|h| h.ring.bounds.is_some_and(|b| keep(&b)))
            .collect();
        let hole_grid = BoundsGrid::new(
            holes.iter().map(|h| h.ring.bounds),
            BoardRules::of(project).max_clearance(),
        );
        Self {
            obstacles: Obstacles::filtered_on(project, keep, &placed),
            holes,
            hole_grid,
            keepouts,
            cutouts,
            edges: BoardEdges::of(project),
            graph: OnceLock::new(),
        }
    }
}

/// A drilled hole the router keeps clear: a pad's hole (a slot included)
/// or a mounting hole.
struct Hole {
    ring: CopperRing,
    /// `PthHole` or `NpthHole`.
    kind: CopperObject,
    /// The net a plated pad hole joins; copper of that net may meet it.
    net: Option<Id>,
    /// How a refusal names it ("J1 pad 17").
    name: String,
    object: ObjectRef,
}

impl Hole {
    /// The pad holes of `placed` and the board's mounting holes
    /// ([`physical::holes`]);
    /// via drills keep their rules as part of their via, board slots as
    /// cutouts.
    fn all(project: &Project, placed: &[crate::layout::PlacedPad]) -> Vec<Self> {
        physical::holes_but_vias(project, placed)
            .into_iter()
            .filter(|h| matches!(h.owner, HoleOwner::Pad { .. } | HoleOwner::MountingHole(_)))
            .map(|h| Self {
                ring: h.feature.ring(),
                kind: CopperObject::hole(h.plated),
                net: h.net,
                object: h.object(),
                name: match h.owner {
                    HoleOwner::MountingHole(_) => "a mounting hole".into(),
                    _ => h.name,
                },
            })
            .collect()
    }
}

/// The core of a piece of copper: a centre-line segment with its half
/// width (a via is a segment of no length).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Core {
    pub a: Point,
    pub b: Point,
    pub half: f64,
}

/// Copper the overlay adds: a step's own tracks and via.
#[derive(Clone, Debug)]
pub(super) struct Piece {
    pub net: Id,
    pub object: CopperObject,
    pub layers: Vec<CopperLayer>,
    pub core: Core,
    pub ring: CopperRing,
    /// A via's drilled hole, which keeps the via hole clearances.
    pub drill: Option<CopperRing>,
}

impl Piece {
    pub fn track(net: Id, layer: CopperLayer, a: Point, b: Point, width: f64) -> Self {
        let half = width / 2.0;
        Self {
            net,
            object: CopperObject::Track,
            layers: vec![layer],
            core: Core {
                a,
                b,
                half: width / 2.0,
            },
            // A track's gaps are measured from its spine (as
            // `CopperRing::of_track` measures them); the flattened outline
            // is never read, so it is not built.
            ring: CopperRing {
                ring: Vec::new(),
                bounds: Some(Bounds {
                    min: [a[0].min(b[0]) - half, a[1].min(b[1]) - half],
                    max: [a[0].max(b[0]) + half, a[1].max(b[1]) + half],
                }),
                slack: 0.0,
                spine: Some((a, b, half)),
            },
            drill: None,
        }
    }

    pub fn via(via: &Via, stack: &[CopperLayer]) -> Self {
        Self {
            net: via.net,
            object: CopperObject::Via,
            layers: stack
                .iter()
                .copied()
                .filter(|l| via.on(*l, stack))
                .collect(),
            core: Core {
                a: via.position,
                b: via.position,
                half: via.diameter / 2.0,
            },
            ring: CopperRing::of_via(via),
            drill: Some(CopperRing::of_drill(via)),
        }
    }
}

/// A chain of tracks a shove moves as one: same net, layer and width,
/// joined end to end where nothing else meets (on the copper graph). Its
/// ends stay put, and so do its pins.
#[derive(Clone, Debug)]
pub(super) struct Line {
    pub net: Id,
    pub layer: CopperLayer,
    pub width: f64,
    /// The tracks in chain order and their obstacle indices.
    pub ids: Vec<Id>,
    pub base: Vec<usize>,
    pub original: Vec<Point>,
    pub points: Vec<Point>,
    pub posture: Posture,
    /// Where other copper joins the chain's tracks in mid-span (a T, a via
    /// on a track): a moved chain must still pass through each.
    pub pins: Vec<Point>,
}

impl Line {
    pub fn piece(&self, segment: usize) -> Option<Piece> {
        let a = *self.points.get(segment)?;
        let b = *self.points.get(segment + 1)?;
        Some(Piece::track(self.net, self.layer, a, b, self.width))
    }

    /// Whether `points`, a new centre line for the chain, keeps every
    /// mid-span joint ([`keeps_joint`]).
    pub fn keeps_pins(&self, points: &[Point]) -> bool {
        self.pins.iter().all(|p| keeps_joint(points, *p))
    }
}

/// A via a shove moved.
#[derive(Clone, Debug)]
pub(super) struct MovedVia {
    pub via: Via,
    pub original: Point,
}

/// The speculative overlay: copper a step adds (fixed), chains and vias
/// it moved, and the board copper they replace (hidden).
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub lines: Vec<Line>,
    pub vias: Vec<MovedVia>,
    pub fixed: Vec<Piece>,
    pub hidden: BTreeSet<usize>,
    /// Nets whose copper never moves: the nets being routed.
    pub frozen: Vec<Id>,
    /// Whether a shove may move vias.
    pub move_vias: bool,
}

/// What a probe collides with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// Board copper by obstacle index.
    Base(usize),
    /// A moved chain's segment.
    Line(usize, usize),
    /// A moved via.
    Via(usize),
    /// The step's own copper.
    Fixed(usize),
    Keepout(usize),
    Cutout(usize),
    /// A pad's drilled hole or a mounting hole, by index into the
    /// index's holes.
    Hole(usize),
    /// The outer board edge (not hugged: it is the outside).
    Edge,
}

/// One collision and what it is made of.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    pub target: Target,
    pub net: Option<Id>,
}

/// The board and rules of one query.
pub(super) struct Ctx<'a> {
    pub project: &'a Project,
    pub index: &'a RouteIndex,
    pub rules: BoardRules<'a>,
    pub stack: Vec<CopperLayer>,
    /// Collision queries made so far: the plan's work budget, counted
    /// rather than timed so a preview and its commit decide alike.
    pub probes: std::cell::Cell<usize>,
    /// A differential pair a shove could not move as one unit (the last
    /// such; the plan's message names it).
    pub held: std::cell::Cell<Option<DiffPairId>>,
}

fn shares(a: &[CopperLayer], b: &[CopperLayer]) -> bool {
    a.iter().any(|l| b.contains(l))
}

impl<'a> Ctx<'a> {
    pub fn new(project: &'a Project, index: &'a RouteIndex) -> Self {
        Self {
            project,
            index,
            rules: BoardRules::of(project),
            stack: project.stackup.layers(),
            probes: std::cell::Cell::new(0),
            held: std::cell::Cell::new(None),
        }
    }

    /// The board track at obstacle index `k` (the index is built from the
    /// project, tracks first).
    pub fn base_track(&self, k: usize) -> Option<&'a Track> {
        let item = self.index.obstacles.items().get(k)?;
        let ObstacleKind::Track(id) = item.kind else {
            return None;
        };
        self.project.tracks.get(k).filter(|t| t.id == id)
    }

    /// The board via at obstacle index `k`.
    pub fn base_via(&self, k: usize) -> Option<&'a Via> {
        let item = self.index.obstacles.items().get(k)?;
        let ObstacleKind::Via(id) = item.kind else {
            return None;
        };
        self.project.via(id)
    }

    pub fn clearance(&self, a: CopperObject, an: Id, b: CopperObject, bn: Option<Id>) -> f64 {
        self.rules.clearance(a, Some(an), b, bn)
    }

    fn edge_limit(&self, probe: &Piece) -> f64 {
        self.rules
            .clearance(probe.object, Some(probe.net), CopperObject::BoardEdge, None)
    }

    /// The board edge or a cutout, when `probe` leaves the board or comes
    /// closer to an edge than the rules allow.
    fn edge_hit(&self, probe: &Piece) -> Option<Hit> {
        let edge = |target| Some(Hit { target, net: None });
        if !probe.ring.inside(self.project.board_size) {
            return edge(Target::Edge);
        }
        let limit = self.edge_limit(probe);
        match self.index.edges.verdict(&probe.ring, limit) {
            EdgeVerdict::Clear => None,
            EdgeVerdict::Outside
            | EdgeVerdict::TooClose {
                near: EdgeLoopKind::Outer,
                ..
            } => edge(Target::Edge),
            EdgeVerdict::Inside(_) | EdgeVerdict::TooClose { .. } => {
                let nearest = self
                    .index
                    .cutouts
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let ga = probe.ring.gap(a, f64::INFINITY);
                        let gb = probe.ring.gap(b, f64::INFINITY);
                        ga.total_cmp(&gb)
                    })
                    .map(|(i, _)| i);
                edge(nearest.map_or(Target::Edge, Target::Cutout))
            }
        }
    }

    /// Every collision of `probe` with copper of nets outside `own`, the
    /// keepouts and the board edge, against the overlay `state`.
    pub fn hits(&self, state: &State, probe: &Piece, own: &[Id]) -> Vec<Hit> {
        let mut out = Vec::new();
        self.collect(state, probe, own, &mut out, false);
        out
    }

    fn collect(&self, state: &State, probe: &Piece, own: &[Id], out: &mut Vec<Hit>, first: bool) {
        self.probes.set(self.probes.get().saturating_add(1));
        let Some(bounds) = probe.ring.bounds else {
            return;
        };
        if let Some(hit) = self.edge_hit(probe) {
            out.push(hit);
            if first {
                return;
            }
        }
        let foreign = |net: Id| !own.contains(&net);
        let push = |hit: Hit, out: &mut Vec<Hit>| {
            out.push(hit);
            first
        };
        let mut near = Vec::new();
        self.index.obstacles.near(&bounds, &mut near);
        let items = self.index.obstacles.items();
        for k in near.drain(..) {
            let Some(o) = items.get(k) else {
                continue;
            };
            if state.hidden.contains(&k)
                || o.net.is_some_and(|n| !foreign(n))
                || self
                    .index
                    .obstacles
                    .reaches_anchor(k, probe.net, &probe.ring)
                || !shares(&probe.layers, &o.layers)
            {
                continue;
            }
            let drill = self.base_via(k).map(CopperRing::of_drill);
            if o.rule_nets()
                .any(|n| self.too_close(probe, &o.ring, drill.as_ref(), o.object(), n))
                && push(
                    Hit {
                        target: Target::Base(k),
                        net: o.net,
                    },
                    out,
                )
            {
                return;
            }
        }
        // Holes pass through every layer. A plated hole of a routed net's
        // pad is that pad's own; any other keeps its hole clearance.
        self.index
            .hole_grid
            .near(&bounds, self.rules.max_clearance(), &mut near);
        for i in near.iter().copied() {
            let Some(hole) = self.index.holes.get(i) else {
                continue;
            };
            if hole.net.is_some_and(|n| !foreign(n)) {
                continue;
            }
            let need = self
                .rules
                .clearance(probe.object, Some(probe.net), hole.kind, hole.net);
            let drilled = probe.drill.as_ref().is_some_and(|d| {
                let spacing = self.rules.clearance(
                    CopperObject::ViaHole,
                    Some(probe.net),
                    hole.kind,
                    hole.net,
                );
                d.closer_than(&hole.ring, spacing)
            });
            if (drilled || probe.ring.closer_than(&hole.ring, need))
                && push(
                    Hit {
                        target: Target::Hole(i),
                        net: hole.net,
                    },
                    out,
                )
            {
                return;
            }
        }
        let reach = self.rules.max_clearance() + 2.0 * CURVE_SLACK_MM;
        let close = |ring: &CopperRing| {
            ring.bounds
                .is_some_and(|b| b.inflated(reach).intersects(&bounds))
        };
        for (l, line) in state.lines.iter().enumerate() {
            if !foreign(line.net) || !probe.layers.contains(&line.layer) {
                continue;
            }
            for s in 0..line.points.len().saturating_sub(1) {
                let Some(piece) = line.piece(s) else {
                    continue;
                };
                if close(&piece.ring) && self.collides(probe, &piece) {
                    let hit = Hit {
                        target: Target::Line(l, s),
                        net: Some(line.net),
                    };
                    if push(hit, out) {
                        return;
                    }
                }
            }
        }
        for (m, moved) in state.vias.iter().enumerate() {
            let piece = Piece::via(&moved.via, &self.stack);
            if foreign(piece.net) && close(&piece.ring) && self.collides(probe, &piece) {
                let hit = Hit {
                    target: Target::Via(m),
                    net: Some(piece.net),
                };
                if push(hit, out) {
                    return;
                }
            }
        }
        for (f, piece) in state.fixed.iter().enumerate() {
            if foreign(piece.net) && close(&piece.ring) && self.collides(probe, piece) {
                let hit = Hit {
                    target: Target::Fixed(f),
                    net: Some(piece.net),
                };
                if push(hit, out) {
                    return;
                }
            }
        }
        let forbidden = |k: &Keepout| match probe.object {
            CopperObject::Via => k.forbid.vias,
            _ => k.forbid.tracks,
        };
        for (i, (k, ring)) in self.index.keepouts.iter().enumerate() {
            if forbidden(k)
                && probe.layers.iter().any(|l| k.on(*l))
                && ring.bounds.is_some_and(|b| b.intersects(&bounds))
                && probe.ring.gap(ring, 1e-6) <= 1e-9
            {
                let hit = Hit {
                    target: Target::Keepout(i),
                    net: None,
                };
                if push(hit, out) {
                    return;
                }
            }
        }
    }

    fn collides(&self, probe: &Piece, other: &Piece) -> bool {
        shares(&probe.layers, &other.layers)
            && self.too_close(
                probe,
                &other.ring,
                other.drill.as_ref(),
                other.object,
                Some(other.net),
            )
    }

    /// Whether `probe` comes closer than the rules allow to copper `ring`
    /// (an `object` of `net`, drilled by `drill` when it is a via): copper
    /// to copper, and each via drill to the other's copper and drill.
    fn too_close(
        &self,
        probe: &Piece,
        ring: &CopperRing,
        drill: Option<&CopperRing>,
        object: CopperObject,
        net: Option<Id>,
    ) -> bool {
        let hole = CopperObject::ViaHole;
        let gap = |a: CopperObject, b: CopperObject| self.clearance(a, probe.net, b, net);
        probe.ring.closer_than(ring, gap(probe.object, object))
            || drill.is_some_and(|d| probe.ring.closer_than(d, gap(probe.object, hole)))
            || probe.drill.as_ref().is_some_and(|own| {
                own.closer_than(ring, gap(hole, object))
                    || drill.is_some_and(|d| own.closer_than(d, gap(hole, hole)))
            })
    }

    /// The hull a probe of `net` (a `object` whose copper reaches `grow`
    /// from its centre line) must keep its centre line out of to clear
    /// `target`.
    pub fn hull(
        &self,
        state: &State,
        target: Target,
        net: Id,
        object: CopperObject,
        grow: f64,
        facets: Facets,
    ) -> Option<Hull> {
        // A via keeps its drill's clearance too, from the drill's edge.
        let drilled = |drill: Option<&CopperRing>, other_net: Option<Id>| {
            drill.and_then(|d| d.bounds).map_or(0.0, |b| {
                (b.max[0] - b.min[0]) / 2.0
                    + self.clearance(object, net, CopperObject::ViaHole, other_net)
            })
        };
        let segment =
            |core: Core, other: CopperObject, other_net: Option<Id>, drill: Option<&CopperRing>| {
                let need = self.clearance(object, net, other, other_net);
                let reach = (core.half + need).max(drilled(drill, other_net));
                Hull::of_segment(core.a, core.b, reach + grow + MARGIN, facets)
            };
        match target {
            Target::Base(k) => {
                let o = self.index.obstacles.items().get(k)?;
                if let Some(t) = self.base_track(k) {
                    return segment(
                        Core {
                            a: t.start,
                            b: t.end,
                            half: t.width / 2.0,
                        },
                        CopperObject::Track,
                        Some(t.net),
                        None,
                    );
                }
                if let Some(v) = self.base_via(k) {
                    return segment(
                        Core {
                            a: v.position,
                            b: v.position,
                            half: v.diameter / 2.0,
                        },
                        CopperObject::Via,
                        Some(v.net),
                        Some(&CopperRing::of_drill(v)),
                    );
                }
                let need = self.clearance(
                    object,
                    net,
                    o.object(),
                    o.limiting_net(&self.rules, object, net),
                );
                Hull::of_points(&o.ring.ring, o.ring.slack + need + grow + MARGIN, facets)
            }
            Target::Line(l, s) => {
                let piece = state.lines.get(l)?.piece(s)?;
                segment(piece.core, piece.object, Some(piece.net), None)
            }
            Target::Via(m) => {
                let piece = Piece::via(&state.vias.get(m)?.via, &self.stack);
                segment(
                    piece.core,
                    piece.object,
                    Some(piece.net),
                    piece.drill.as_ref(),
                )
            }
            Target::Fixed(f) => {
                let piece = state.fixed.get(f)?;
                segment(
                    piece.core,
                    piece.object,
                    Some(piece.net),
                    piece.drill.as_ref(),
                )
            }
            Target::Keepout(i) => {
                let (_, ring) = self.index.keepouts.get(i)?;
                Hull::of_points(&ring.ring, ring.slack + grow + MARGIN, facets)
            }
            Target::Cutout(i) => {
                let ring = self.index.cutouts.get(i)?;
                let limit = self
                    .rules
                    .clearance(object, Some(net), CopperObject::BoardEdge, None);
                Hull::of_points(&ring.ring, ring.slack + limit + grow + MARGIN, facets)
            }
            Target::Hole(i) => {
                let hole = self.index.holes.get(i)?;
                let need = self.rules.clearance(object, Some(net), hole.kind, hole.net);
                Hull::of_points(
                    &hole.ring.ring,
                    hole.ring.slack + need + grow + MARGIN,
                    facets,
                )
            }
            Target::Edge => None,
        }
    }

    /// Whether `target` may be moved by a shove in `state`.
    pub fn movable(&self, state: &State, target: Target) -> bool {
        match target {
            Target::Base(k) => {
                if let Some(t) = self.base_track(k) {
                    !t.locked && !state.frozen.contains(&t.net)
                } else if let Some(v) = self.base_via(k) {
                    state.move_vias
                        && !v.locked
                        && !state.frozen.contains(&v.net)
                        && self.pair_of(v.net).is_none()
                } else {
                    false
                }
            }
            Target::Line(..) => true,
            Target::Via(_) => state.move_vias,
            _ => false,
        }
    }

    /// The differential pair `net` is a half of.
    pub fn pair_of(&self, net: Id) -> Option<&'a DiffPair> {
        self.project.rules.pair_of(net)
    }

    /// Why the head could not pass a pair it could not shove, when one
    /// held and the overlay `state` (the plan's result) leaves it unmoved:
    /// a later attempt may have found a way that moves it whole after all.
    pub fn held_note(&self, state: &State) -> Option<String> {
        let pair = self.project.rules.diff_pair(self.held.get()?)?;
        if state
            .lines
            .iter()
            .any(|l| pair.has(l.net) && l.points != l.original)
        {
            return None;
        }
        Some(format!(
            "Pair {} can't move as one here, so it stays put",
            pair.name
        ))
    }

    /// The copper graph of the index's revision: which copper is joined
    /// and where.
    pub fn graph(&self) -> &'a CopperGraph {
        self.index
            .graph
            .get_or_init(|| Arc::new(CopperGraph::of(self.project)))
    }

    /// The document object `hit` is, when it is a track, via or pad of
    /// the board as it stands (not shoved copper, a keepout or an edge).
    pub fn object(&self, hit: &Hit) -> Option<ObjectRef> {
        match hit.target {
            Target::Base(k) => self.index.obstacles.object(k),
            Target::Hole(i) => self.index.holes.get(i).map(|h| h.object),
            _ => None,
        }
    }

    /// Why the committed board refuses `probe` for `hit`, with the rule
    /// it misses: "Too close to a via on GND: needs 0.2 mm (Fine 4/4)",
    /// "Crosses keepout Antenna".
    pub fn refusal(&self, probe: &Piece, hit: &Hit) -> String {
        let need = |object: CopperObject, net: Option<Id>| {
            self.rules
                .clearance_need(probe.object, Some(probe.net), object, net)
                .short()
        };
        match hit.target {
            Target::Base(k) => match self.index.obstacles.items().get(k) {
                Some(o) => {
                    let net = o.limiting_net(&self.rules, probe.object, probe.net);
                    let label = self.index.obstacles.label(k);
                    // The copper keeps its clearance and the via's drilled
                    // hole does not: name the hole and the hole's rule
                    // ([`Self::too_close`] tests both).
                    let copper = self.clearance(probe.object, probe.net, o.object(), net);
                    let hole = CopperObject::ViaHole;
                    let drilled = self.base_via(k).map(CopperRing::of_drill).is_some_and(|d| {
                        !probe.ring.closer_than(&o.ring, copper)
                            && probe
                                .ring
                                .closer_than(&d, self.clearance(probe.object, probe.net, hole, net))
                    });
                    if drilled {
                        format!("Too close to the hole of {label}: {}", need(hole, net))
                    } else {
                        format!("Too close to {label}: {}", need(o.object(), net))
                    }
                }
                None => self.describe(hit),
            },
            Target::Hole(i) => match self.index.holes.get(i) {
                Some(h) => format!(
                    "Too close to the hole of {}: {}",
                    h.name,
                    need(h.kind, h.net)
                ),
                None => self.describe(hit),
            },
            Target::Keepout(i) => {
                let name = self
                    .index
                    .keepouts
                    .get(i)
                    .map_or("", |(k, _)| k.name.as_str());
                if name.is_empty() {
                    "Crosses a keepout".into()
                } else {
                    format!("Crosses keepout {name}")
                }
            }
            Target::Cutout(_) => format!(
                "Too close to a cutout: {}",
                need(CopperObject::BoardEdge, None)
            ),
            Target::Edge => format!(
                "Too close to the board edge: {}",
                need(CopperObject::BoardEdge, None)
            ),
            _ => self.describe(hit),
        }
    }

    /// A refusal naming what `hit` is ("Too close to a via on GND").
    pub fn describe(&self, hit: &Hit) -> String {
        match hit.target {
            Target::Base(k) => format!("Blocked by {}", self.index.obstacles.label(k)),
            Target::Keepout(i) => {
                let name = self
                    .index
                    .keepouts
                    .get(i)
                    .map_or("", |(k, _)| k.name.as_str());
                if name.is_empty() {
                    "Blocked by a keepout".into()
                } else {
                    format!("Blocked by keepout {name}")
                }
            }
            Target::Cutout(_) => "Blocked by a cutout".into(),
            Target::Hole(i) => match self.index.holes.get(i) {
                Some(h) => format!("Blocked by the hole of {}", h.name),
                None => "Blocked by a hole".into(),
            },
            Target::Edge => "Blocked by the board edge".into(),
            _ => match hit.net {
                Some(net) => format!("Blocked by copper on {}", self.project.net_name(net)),
                None => "Blocked".into(),
            },
        }
    }
}

//! Obstacle queries: every other net's copper (tracks, vias and the
//! board's lands, [`super::physical::lands`]: pads with or without a net
//! and mounting-hole washers) with its layers and exact outline, filed in a grid, and the clearance
//! test a proposed track must pass. The length tuner (`meander`) asks it
//! for room; a future push-and-shove router asks the same index which
//! copper a proposed segment collides with (`hits`) before it moves that
//! copper. Built once per document revision; the rules are read per query
//! (`limits::BoardRules`), so class and pair clearances apply as in the
//! board checks.
use super::grid::BoundsGrid;
use super::limits::BoardRules;
use super::{CopperRing, copper};
use crate::diagnostics::ObjectRef;
use crate::geometry::Point;
use crate::ids::Id;
use crate::model::{CopperLayer, Project, Track};
use crate::rules::CopperObject;

/// What an obstacle is, for messages and for a shove strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObstacleKind {
    Track(Id),
    Via(Id),
    /// A land: its index among the lands the index was made of
    /// ([`super::physical::lands`] for a whole board's).
    Land(usize),
}

/// One piece of copper, of a net or of none (a fiducial, a tab, an
/// unconnected pin, a washer: every route keeps clear of it).
#[derive(Clone, Debug)]
pub struct Obstacle {
    pub net: Option<Id>,
    pub kind: ObstacleKind,
    /// The copper layers it occupies.
    pub layers: Vec<CopperLayer>,
    pub ring: CopperRing,
}

impl Obstacle {
    pub(crate) fn object(&self) -> CopperObject {
        match self.kind {
            ObstacleKind::Track(_) => CopperObject::Track,
            ObstacleKind::Via(_) => CopperObject::Via,
            ObstacleKind::Land(_) => CopperObject::Pad,
        }
    }
}

/// The board's copper, indexed for clearance queries.
#[derive(Debug, Default)]
pub struct Obstacles {
    items: Vec<Obstacle>,
    grid: BoundsGrid,
    reach: f64,
    /// Pad labels for messages, by index into `items`.
    labels: Vec<String>,
    /// What each item is in the document, by index into `items`.
    objects: Vec<ObjectRef>,
}

/// The placed pads of `project` whose extent `keep` takes: a pad's land
/// and holes lie within its extent, so only these are made.
pub(crate) fn kept_pads(
    project: &Project,
    keep: &dyn Fn(&crate::geometry::Bounds) -> bool,
) -> Vec<super::PlacedPad> {
    super::placed_pads(project)
        .into_iter()
        .filter(|p| keep(&p.extent()))
        .collect()
}

impl Obstacles {
    /// Every track, via and land of `project`.
    pub fn of(project: &Project) -> Self {
        Self::filtered(project, &|_| true)
    }

    /// The tracks, vias and lands of `project` whose bounds `keep` takes,
    /// in [`Self::of`]'s order: an index that answers every query near
    /// the kept area as the whole board's does.
    pub(crate) fn filtered(
        project: &Project,
        keep: &dyn Fn(&crate::geometry::Bounds) -> bool,
    ) -> Self {
        Self::filtered_on(project, keep, &kept_pads(project, keep))
    }

    /// [`Self::filtered`] with `placed`, the placed pads whose extent
    /// `keep` takes ([`kept_pads`]).
    pub(crate) fn filtered_on(
        project: &Project,
        keep: &dyn Fn(&crate::geometry::Bounds) -> bool,
        placed: &[super::PlacedPad],
    ) -> Self {
        let layers = project.stackup.layers();
        let mut items = Vec::new();
        let mut labels = Vec::new();
        let mut objects = Vec::new();
        let kept = |a: Point, b: Point, r: f64| {
            keep(&crate::geometry::Bounds {
                min: [a[0].min(b[0]) - r, a[1].min(b[1]) - r],
                max: [a[0].max(b[0]) + r, a[1].max(b[1]) + r],
            })
        };
        for track in project
            .tracks
            .iter()
            .filter(|t| kept(t.start, t.end, t.width / 2.0))
        {
            items.push(Obstacle {
                net: Some(track.net),
                kind: ObstacleKind::Track(track.id),
                layers: vec![track.layer],
                ring: CopperRing::of_track(track),
            });
            labels.push(format!("a track on {}", project.net_name(track.net)));
            objects.push(ObjectRef::Track { id: track.id });
        }
        for via in project
            .vias
            .iter()
            .filter(|v| kept(v.position, v.position, v.diameter / 2.0))
        {
            items.push(Obstacle {
                net: Some(via.net),
                kind: ObstacleKind::Via(via.id),
                layers: layers
                    .iter()
                    .copied()
                    .filter(|l| via.on(*l, &layers))
                    .collect(),
                ring: CopperRing::of_via(via),
            });
            labels.push(format!("a via on {}", project.net_name(via.net)));
            objects.push(ObjectRef::Via { id: via.id });
        }
        for (index, land) in super::physical::lands_of(project, placed)
            .into_iter()
            .enumerate()
        {
            if !land.ring.bounds.is_some_and(|b| keep(&b)) {
                continue;
            }
            labels.push(match land.net() {
                Some(net) => format!("{} ({})", land.name, project.net_name(net)),
                None => land.name.clone(),
            });
            objects.push(land.object());
            items.push(Obstacle {
                net: land.net(),
                kind: ObstacleKind::Land(index),
                layers: layers.iter().copied().filter(|l| land.on(*l)).collect(),
                ring: land.ring,
            });
        }
        let reach = copper::check_reach(&BoardRules::of(project));
        let grid = BoundsGrid::new(items.iter().map(|i| i.ring.bounds), reach);
        Self {
            items,
            grid,
            reach,
            labels,
            objects,
        }
    }

    pub fn items(&self) -> &[Obstacle] {
        &self.items
    }

    /// Every obstacle whose bounds come within the index's reach (the
    /// largest clearance) of `bounds`, ascending, into `out`.
    pub(crate) fn near(&self, bounds: &crate::geometry::Bounds, out: &mut Vec<usize>) {
        self.grid.near(bounds, self.reach, out);
    }

    /// The message label of obstacle `k` ("a via on GND", "R1 pad 2 (VCC)").
    pub(crate) fn label(&self, k: usize) -> &str {
        self.labels.get(k).map_or("copper", String::as_str)
    }

    /// The document object obstacle `k` is (a track, via or pad).
    pub(crate) fn object(&self, k: usize) -> Option<ObjectRef> {
        self.objects.get(k).copied()
    }

    /// Every other-net obstacle on `layer` closer to a track of `net`
    /// from `a` to `b` of `width` than the rules allow, by index into
    /// [`Self::items`]. Copper of `net` itself is never an obstacle.
    pub fn hits(
        &self,
        rules: &BoardRules<'_>,
        net: Id,
        layer: CopperLayer,
        a: Point,
        b: Point,
        width: f64,
    ) -> Vec<usize> {
        let track = Track {
            id: 0,
            net,
            start: a,
            end: b,
            width,
            layer,
            locked: false,
        };
        let ring = CopperRing::of_track(&track);
        let Some(bounds) = ring.bounds else {
            return Vec::new();
        };
        let mut near = Vec::new();
        self.grid.near(&bounds, self.reach, &mut near);
        near.into_iter()
            .filter(|k| {
                self.items.get(*k).is_some_and(|o| {
                    o.net != Some(net)
                        && o.layers.contains(&layer)
                        && ring.closer_than(
                            &o.ring,
                            rules.clearance(CopperObject::Track, Some(net), o.object(), o.net),
                        )
                })
            })
            .collect()
    }

    /// The first obstacle a track of `net` from `a` to `b` would be too
    /// close to, as a refusal text ("Too close to a via on GND: needs
    /// 0.2 mm (…)"), or `None` when the track has room.
    #[allow(clippy::too_many_arguments)]
    pub fn conflict(
        &self,
        rules: &BoardRules<'_>,
        net: Id,
        layer: CopperLayer,
        a: Point,
        b: Point,
        width: f64,
    ) -> Option<String> {
        let first = *self.hits(rules, net, layer, a, b, width).first()?;
        let o = self.items.get(first)?;
        let need = rules.clearance_need(CopperObject::Track, Some(net), o.object(), o.net);
        Some(format!(
            "Too close to {}: {}",
            self.labels.get(first).map_or("copper", String::as_str),
            need.short()
        ))
    }
}

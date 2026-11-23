//! Contacts made by fabricated tie bridges. A bridge has several nets and
//! no island. Only the individual lands are members of their net's islands.
use super::{BoundsGrid, CopperGraph, CopperRing, build};
use crate::diagnostics::{Finding, ObjectRef};
use crate::geometry::Bounds;
use crate::ids::{Id, PadId};
use crate::layout::{PlacedBridge, check_pair_budget, fill, physical, placed_bridges};
use crate::model::{CopperLayer, Project};
use crate::rules::CopperObject;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TieIsland {
    pub net: Id,
    pub pad: PadId,
    pub island: usize,
}

#[derive(Clone, Debug)]
pub struct TieJoin {
    pub bridge: PlacedBridge,
    pub islands: Vec<TieIsland>,
    /// Physical contacts outside the declared pad group, including contacts
    /// on one of the end nets. No blanket exemption for an entire footprint.
    pub foreign: Vec<ObjectRef>,
}

struct Item {
    object: ObjectRef,
    layer: CopperLayer,
    through: bool,
    net: Option<Id>,
    kind: CopperObject,
    ring: CopperRing,
}

impl CopperGraph {
    pub fn ties(&self) -> &[TieJoin] {
        &self.ties
    }

    pub(super) fn record_ties(&mut self, project: &Project, fills: &[fill::Fresh<'_>]) {
        let bridges = placed_bridges(project);
        if bridges.is_empty() {
            return;
        }
        let mut items = Vec::new();
        for land in physical::lands(project) {
            items.push(Item {
                object: land.object(),
                layer: land.layer.into(),
                through: land.through,
                kind: CopperObject::Pad,
                net: land.net(),
                ring: land.ring,
            });
        }
        for track in &self.tracks {
            items.push(Item {
                object: ObjectRef::Track { id: track.id },
                layer: track.layer,
                through: false,
                kind: CopperObject::Track,
                net: Some(track.net),
                ring: build::track_copper(track),
            });
        }
        for via in &self.vias {
            for &layer in self.layers.iter().filter(|l| via.on(**l, &self.layers)) {
                items.push(Item {
                    object: ObjectRef::Via { id: via.id },
                    layer,
                    through: false,
                    kind: CopperObject::Via,
                    net: Some(via.net),
                    ring: build::via_copper(via),
                });
            }
        }
        for b in &bridges {
            items.push(Item {
                object: ObjectRef::Instance { id: b.instance },
                layer: b.layer,
                through: false,
                kind: CopperObject::Track,
                net: None,
                ring: bridge_ring(b),
            });
        }
        let pad_indices: BTreeMap<_, _> = self
            .pads
            .iter()
            .enumerate()
            .map(|(i, p)| ((p.instance, p.pad), i))
            .collect();
        let fill_indices: Vec<_> = fills
            .iter()
            .map(|f| (f, fill::RegionIndex::filled(&f.copper.regions)))
            .collect();
        let rules = crate::layout::limits::BoardRules::of(project);
        let reach = crate::layout::copper::check_reach(&rules).max(build::JOIN_REACH_MM);
        let grid = BoundsGrid::new(items.iter().map(|i| i.ring.bounds), reach);
        let mut near = Vec::new();
        let mut remaining = check_pair_budget(items.len());
        for bridge in bridges {
            let copper = bridge_ring(&bridge);
            let Some(bounds) = copper.bounds else {
                continue;
            };
            let mut islands = Vec::new();
            let mut foreign = Vec::new();
            let owner = ObjectRef::Instance {
                id: bridge.instance,
            };
            grid.near(&bounds, reach, &mut near);
            for item in near.iter().filter_map(|i| items.get(*i)) {
                if remaining == 0 {
                    self.partial = true;
                    break;
                }
                remaining -= 1;
                if (!item.through && item.layer != bridge.layer) || item.object == owner {
                    continue;
                }
                let contact = build::joins(&copper, &item.ring);
                if let ObjectRef::Pad { instance, pad } = item.object
                    && instance == bridge.instance
                    && bridge.pads.contains(&pad)
                {
                    if contact
                        && let Some(index) = pad_indices.get(&(instance, pad))
                        && let Some(island) = self.pad_island(*index)
                        && let Some(net) = item.net
                    {
                        islands.push(TieIsland { net, pad, island });
                    }
                    continue;
                }
                if contact {
                    if !foreign.contains(&item.object) {
                        foreign.push(item.object);
                    }
                } else if bridge.nets.iter().any(|n| {
                    copper.closer_than(
                        &item.ring,
                        rules.clearance(CopperObject::Track, Some(*n), item.kind, item.net),
                    )
                }) {
                    let name = project
                        .instance(bridge.instance)
                        .map_or("Tie", |i| &i.designator);
                    self.tie_findings.push(Finding::new(
                        "board.clearance",
                        format!("{name} bridge is too close to neighbouring copper."),
                        vec![owner, item.object],
                    ));
                }
            }
            for (f, index) in &fill_indices {
                if f.layer == bridge.layer && !build::islands_joined(index, &copper).is_empty() {
                    foreign.push(ObjectRef::Fill { id: f.id });
                }
            }
            self.ties.push(TieJoin {
                bridge,
                islands,
                foreign,
            });
        }
    }
}

fn bridge_ring(b: &PlacedBridge) -> CopperRing {
    let ring = b.outline.flatten(crate::layout::copper::CURVE_SLACK_MM);
    let bounds = Bounds::of(ring.iter().copied());
    CopperRing {
        ring,
        bounds,
        slack: if b
            .outline
            .segments
            .iter()
            .any(|s| matches!(s, crate::library::Segment::Arc { .. }))
        {
            crate::layout::copper::CURVE_SLACK_MM
        } else {
            0.0
        },
        spine: None,
    }
}

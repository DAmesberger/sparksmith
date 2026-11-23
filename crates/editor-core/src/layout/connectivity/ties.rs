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
        let tie_rows = crate::instances::ties::rows(project);
        if tie_rows.is_empty() {
            return;
        }
        let bridges = placed_bridges(project);
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
        self.other_contacts(
            project,
            &tie_rows,
            &items,
            &grid,
            &fill_indices,
            &mut remaining,
        );
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
    fn other_contacts(
        &mut self,
        project: &Project,
        ties: &[crate::instances::ties::TieRow],
        items: &[Item],
        grid: &BoundsGrid,
        fills: &[(&fill::Fresh<'_>, fill::RegionIndex)],
        remaining: &mut usize,
    ) {
        let mut pairs = BTreeMap::<(Id, Id), Vec<&crate::instances::ties::TieRow>>::new();
        for t in ties {
            let ends: std::collections::BTreeSet<Id> = t.ends.values().flatten().copied().collect();
            for &a in &ends {
                for &b in ends.range((std::ops::Bound::Excluded(a), std::ops::Bound::Unbounded)) {
                    pairs.entry((a, b)).or_default().push(t);
                }
            }
        }
        let anchors = ties
            .iter()
            .filter_map(|t| {
                Some((
                    t.instance,
                    &project.instance(t.instance)?.tie.as_ref()?.anchors,
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let mut anchored_ends = std::collections::BTreeSet::new();
        let mut near = Vec::new();
        let mut record = |a: Id, b: Id, x: ObjectRef, y: ObjectRef, at: crate::geometry::Point| {
            let Some(tied) = pairs.get(&(a.min(b), a.max(b))) else {
                return;
            };
            for t in tied {
                if !self.tie_contacts.insert((x, y, t.instance)) {
                    continue;
                }
                self.tie_contacts.insert((y, x, t.instance));
                if t.purpose == crate::library::tie::TiePurpose::KelvinSense {
                    let mut allowed = false;
                    if let Some(ends) = anchors.get(&t.instance) {
                        for (end, anchor) in *ends {
                            let land = ObjectRef::Pad {
                                instance: anchor.instance,
                                pad: anchor.pad,
                            };
                            if (land == x && t.ends.get(end) == Some(&Some(b)))
                                || (land == y && t.ends.get(end) == Some(&Some(a)))
                            {
                                allowed = true;
                                anchored_ends.insert((t.instance, *end));
                            }
                        }
                    }
                    if !allowed {
                        self.tie_findings.push(Finding::new(
                            "tie.kelvin_off_anchor",
                            format!(
                                "{} sense copper meets the other net away from its anchor land.",
                                t.designator
                            ),
                            vec![ObjectRef::Instance { id: t.instance }, x, y],
                        ));
                    }
                    continue;
                }
                self.tie_findings.push(Finding::new(
                    "tie.second_join",
                    format!(
                        "{} and {} also meet near ({:.2}, {:.2}) mm outside {}.",
                        project.net_name(a),
                        project.net_name(b),
                        at[0],
                        at[1],
                        t.designator
                    ),
                    vec![ObjectRef::Instance { id: t.instance }, x, y],
                ));
                if t.purpose == crate::library::tie::TiePurpose::StarGround {
                    self.tie_findings.push(Finding::new(
                        "tie.star_points",
                        format!("{} has an additional copper join.", t.designator),
                        vec![ObjectRef::Instance { id: t.instance }, x, y],
                    ));
                }
            }
        };
        for (i, item) in items.iter().enumerate() {
            let (Some(a), Some(bounds)) = (item.net, item.ring.bounds) else {
                continue;
            };
            grid.near(&bounds, build::JOIN_REACH_MM, &mut near);
            for other in near
                .iter()
                .filter(|j| **j > i)
                .filter_map(|j| items.get(*j))
            {
                let Some(b) = other.net else {
                    continue;
                };
                if !(item.through || other.through || item.layer == other.layer)
                    || !pairs.contains_key(&(a.min(b), a.max(b)))
                {
                    continue;
                }
                if *remaining == 0 {
                    self.partial = true;
                    return;
                }
                *remaining -= 1;
                if build::joins(&item.ring, &other.ring) {
                    record(
                        a,
                        b,
                        item.object,
                        other.object,
                        other.ring.bounds.map_or(bounds.center(), |v| v.center()),
                    );
                }
            }
            for (f, index) in fills {
                if (item.through || item.layer == f.layer)
                    && pairs.contains_key(&(a.min(f.net), a.max(f.net)))
                    && !build::islands_joined(index, &item.ring).is_empty()
                {
                    record(
                        a,
                        f.net,
                        item.object,
                        ObjectRef::Fill { id: f.id },
                        bounds.center(),
                    );
                }
            }
        }
        for tie in ties
            .iter()
            .filter(|t| t.purpose == crate::library::tie::TiePurpose::KelvinSense)
        {
            if !anchored_ends.iter().any(|(id, _)| *id == tie.instance)
                || anchors.get(&tie.instance).is_some_and(|ends| {
                    ends.keys()
                        .any(|e| !anchored_ends.contains(&(tie.instance, *e)))
                })
            {
                self.tie_findings.push(Finding::new(
                    "tie.kelvin_off_anchor",
                    format!(
                        "{} needs sense copper joined at its anchor land.",
                        tie.designator
                    ),
                    vec![ObjectRef::Instance { id: tie.instance }],
                ));
            }
        }
    }

    /// A clearance report about these exact objects is already represented by a tie finding.
    pub(crate) fn accounts_for(&self, objects: &[ObjectRef]) -> bool {
        let [a, b, ..] = objects else {
            return false;
        };
        self.tie_contacts.iter().any(|(x, y, _)| a == x && b == y)
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

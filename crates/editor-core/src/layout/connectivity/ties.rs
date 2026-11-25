//! Contacts made by fabricated tie bridges. A bridge has several nets and
//! no island. Only the individual lands are members of their net's islands.
use super::{BoundsGrid, CopperGraph, CopperRing, build};
use crate::diagnostics::{Finding, FindingCode, ObjectRef};
use crate::geometry::Bounds;
use crate::ids::{Id, PadId};
use crate::layout::{PlacedBridge, check_pair_budget, fill, physical, placed_bridges};
use crate::model::{CopperLayer, Project};
use crate::rules::CopperObject;
use std::collections::BTreeMap;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
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
    bridge: Option<crate::ids::BridgeId>,
    kind: CopperObject,
    ring: CopperRing,
}

/// A sense end has a routing destination on its named land. This copy
/// belongs only to the connectivity graph, never to fabricated copper.
pub(super) fn sense_destinations(
    project: &Project,
    pads: &[crate::layout::WorldPad],
) -> Vec<crate::layout::WorldPad> {
    if !project
        .instances
        .iter()
        .any(|i| i.ties.values().any(|u| !u.anchors.is_empty()))
    {
        return vec![];
    }
    let lands: BTreeMap<_, _> = pads.iter().map(|p| ((p.instance, p.pad), p)).collect();
    let mut destinations = Vec::new();
    for row in crate::instances::ties::rows(project) {
        let Some(usage) = project
            .instance(row.instance)
            .and_then(|i| i.ties.get(&row.group))
        else {
            continue;
        };
        for (end, anchor) in &usage.anchors {
            let Some(&Some(net)) = row.ends.get(end) else {
                continue;
            };
            let Some(&land) = lands.get(&(anchor.instance, anchor.pad)) else {
                continue;
            };
            if land.net == net || !row.ends.values().any(|n| *n == Some(land.net)) {
                continue;
            }
            let mut dest = land.clone();
            dest.terminal = crate::ids::Endpoint {
                instance: row.instance,
                terminal: *end,
            };
            dest.net = net;
            // Keep the real pad identity so picking and messages resolve to the anchor.
            destinations.push(dest);
        }
    }
    destinations
}

impl CopperGraph {
    pub fn anchor_points(
        &self,
        instance: Id,
        end: crate::ids::TerminalId,
    ) -> &[(crate::geometry::Point, CopperLayer)] {
        self.anchor_points
            .get(&(instance, end))
            .map_or(&[], Vec::as_slice)
    }
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
                bridge: None,
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
                bridge: None,
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
                    bridge: None,
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
                bridge: Some(b.bridge),
                kind: CopperObject::Track,
                net: None,
                ring: bridge_ring(b),
            });
        }
        let pad_indices: BTreeMap<_, _> = self
            .pads
            .iter()
            .enumerate()
            .filter(|(_, p)| p.instance == p.terminal.instance)
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
        let edges = crate::layout::board_edge::BoardEdges::of(project);
        let holes: Vec<_> = if bridges.is_empty() {
            Vec::new()
        } else {
            physical::holes(project)
                .into_iter()
                .map(|h| (h.feature.ring(), h))
                .collect()
        };
        for bridge in bridges {
            let copper = bridge_ring(&bridge);
            let Some(bounds) = copper.bounds else {
                continue;
            };
            self.tie_findings
                .extend(bridge_holes(project, &rules, &bridge, &copper, &holes));
            let mut islands = Vec::new();
            let mut foreign = Vec::new();
            let owner = ObjectRef::Instance {
                id: bridge.instance,
            };
            let edge_net = bridge.nets.iter().copied().max_by(|a, b| {
                let gap = |net| {
                    rules.clearance(
                        CopperObject::Track,
                        Some(net),
                        CopperObject::BoardEdge,
                        None,
                    )
                };
                gap(*a).total_cmp(&gap(*b))
            });
            self.tie_findings
                .extend(crate::layout::copper::edge_finding(
                    &edges,
                    &rules,
                    &copper,
                    CopperObject::Track,
                    edge_net,
                    owner,
                    || {
                        format!(
                            "{} bridge {}",
                            project
                                .instance(bridge.instance)
                                .map_or("Tie", |i| &i.designator),
                            bridge.bridge.0
                        )
                    },
                ));
            grid.near(&bounds, reach, &mut near);
            for item in near.iter().filter_map(|i| items.get(*i)) {
                if remaining == 0 {
                    self.partial = true;
                    break;
                }
                remaining -= 1;
                if (!item.through && item.layer != bridge.layer)
                    || (item.object == owner && item.bridge == Some(bridge.bridge))
                {
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
                } else if copper.closer_than(
                    &item.ring,
                    bridge
                        .nets
                        .iter()
                        .map(|n| {
                            rules.clearance(CopperObject::Track, Some(*n), item.kind, item.net)
                        })
                        .reduce(f64::max)
                        .unwrap_or_else(|| {
                            rules.clearance(CopperObject::Track, None, item.kind, item.net)
                        }),
                ) {
                    let name = project
                        .instance(bridge.instance)
                        .map_or("Tie", |i| &i.designator);
                    self.tie_findings.push(Finding::new(
                        FindingCode::BoardClearance,
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
                    (t.instance, t.group),
                    &project.instance(t.instance)?.ties.get(&t.group)?.anchors,
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let mut anchored_ends = std::collections::BTreeSet::new();
        let mut near = Vec::new();
        let mut record = |a: Id,
                          b: Id,
                          x: ObjectRef,
                          y: ObjectRef,
                          at: crate::geometry::Point,
                          layer: CopperLayer| {
            let Some(tied) = pairs.get(&(a.min(b), a.max(b))) else {
                return;
            };
            for t in tied {
                if !self.tie_contacts.insert((x, y, t.instance, t.group, layer)) {
                    continue;
                }
                self.tie_contacts.insert((y, x, t.instance, t.group, layer));
                if t.purpose == crate::library::tie::TiePurpose::KelvinSense {
                    let mut allowed = false;
                    if let Some(ends) = anchors.get(&(t.instance, t.group)) {
                        for (end, anchor) in *ends {
                            let land = ObjectRef::Pad {
                                instance: anchor.instance,
                                pad: anchor.pad,
                            };
                            if (land == x && t.ends.get(end) == Some(&Some(b)))
                                || (land == y && t.ends.get(end) == Some(&Some(a)))
                            {
                                allowed = true;
                                self.anchor_points
                                    .entry((t.instance, *end))
                                    .or_default()
                                    .push((at, layer));
                                anchored_ends.insert((t.instance, *end));
                                let contacts =
                                    self.anchor_contacts.entry((t.instance, *end)).or_default();
                                let sense = if land == x { y } else { x };
                                if !contacts.contains(&sense) {
                                    contacts.push(sense);
                                }
                            }
                        }
                    }
                    if allowed {
                        self.contact_pairs.insert((x, y));
                        self.contact_pairs.insert((y, x));
                    } else {
                        // It stays a short too, which no waiver accepts: this
                        // one says where the net should meet.
                        self.tie_findings.push(Finding::new(
                            FindingCode::TieKelvinOffAnchor,
                            format!(
                                "{} sense copper meets the other net away from its anchor land.",
                                t.label(project)
                            ),
                            t.objects([x, y]),
                        ));
                    }
                    continue;
                }
                self.contact_pairs.insert((x, y));
                self.contact_pairs.insert((y, x));
                let copper = |object: ObjectRef, net: Id| match object {
                    ObjectRef::Fill { id } => project
                        .fill(id)
                        .map_or_else(|| "a fill".into(), |f| fill::fill_label(project, f)),
                    ObjectRef::Track { .. } => format!("a track on {}", project.net_name(net)),
                    ObjectRef::Via { .. } => format!("a via on {}", project.net_name(net)),
                    ObjectRef::Pad { instance, pad } => format!(
                        "pad {} of {}",
                        pad.0,
                        project
                            .instance(instance)
                            .map_or("a part", |i| &i.designator)
                    ),
                    _ => "copper".into(),
                };
                self.tie_findings.push(Finding::new(
                    FindingCode::TieSecondJoin,
                    format!(
                        "{} and {} also meet near ({:.2}, {:.2}) mm through {} and {}, outside {}.",
                        project.net_name(a),
                        project.net_name(b),
                        at[0],
                        at[1],
                        copper(x, a),
                        copper(y, b),
                        t.label(project)
                    ),
                    t.objects([x, y]),
                ));
                if t.purpose == crate::library::tie::TiePurpose::StarGround {
                    self.tie_findings.push(Finding::new(
                        FindingCode::TieStarPoints,
                        format!("{} has an additional copper join.", t.label(project)),
                        t.objects([x, y]),
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
                        build::contact_point(&item.ring, &other.ring).unwrap_or(bounds.center()),
                        if item.through {
                            other.layer
                        } else {
                            item.layer
                        },
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
                        build::fill_contact_point(index, &item.ring).unwrap_or(bounds.center()),
                        f.layer,
                    );
                }
            }
        }
        for (i, (a, index_a)) in fills.iter().enumerate() {
            for (b, index_b) in fills.iter().skip(i + 1) {
                if a.layer != b.layer || !pairs.contains_key(&(a.net.min(b.net), a.net.max(b.net)))
                {
                    continue;
                }
                if *remaining == 0 {
                    self.partial = true;
                    return;
                }
                *remaining -= 1;
                if let Some(at) = fill_contact(a, index_b).or_else(|| fill_contact(b, index_a)) {
                    record(
                        a.net,
                        b.net,
                        ObjectRef::Fill { id: a.id },
                        ObjectRef::Fill { id: b.id },
                        at,
                        a.layer,
                    );
                }
            }
        }
        for tie in ties
            .iter()
            .filter(|t| t.purpose == crate::library::tie::TiePurpose::KelvinSense)
        {
            if !tie
                .ends
                .keys()
                .any(|end| anchored_ends.contains(&(tie.instance, *end)))
                || anchors.get(&(tie.instance, tie.group)).is_some_and(|ends| {
                    ends.keys()
                        .any(|e| !anchored_ends.contains(&(tie.instance, *e)))
                })
            {
                self.tie_findings.push(Finding::new(
                    FindingCode::TieKelvinOffAnchor,
                    format!(
                        "{} needs sense copper joined at its anchor land.",
                        tie.label(project)
                    ),
                    tie.objects([]),
                ));
            }
        }
    }

    /// A clearance report about these exact objects is already represented by a tie finding.
    pub(crate) fn accounts_for(&self, objects: &[ObjectRef]) -> bool {
        let [a, b, ..] = objects else {
            return false;
        };
        self.contact_pairs.contains(&(*a, *b))
    }
}

/// A bridge's copper against every drilled hole (via drills, pad and board
/// holes) but the plated ones of the nets it joins, by the hole kind's
/// track clearance: a bridge is copper no other hole check reads.
fn bridge_holes(
    project: &Project,
    rules: &crate::layout::limits::BoardRules<'_>,
    bridge: &PlacedBridge,
    copper: &CopperRing,
    holes: &[(CopperRing, physical::Hole)],
) -> Vec<Finding> {
    let Some(bounds) = copper.bounds else {
        return Vec::new();
    };
    let reach = crate::layout::copper::check_reach(rules);
    let near = bounds.inflated(reach);
    let part = project
        .instance(bridge.instance)
        .map_or("Tie", |i| &i.designator);
    let mut out = Vec::new();
    for (ring, hole) in holes {
        if !ring.bounds.is_some_and(|b| near.intersects(&b))
            || hole.plated && hole.net.is_some_and(|n| bridge.nets.contains(&n))
        {
            continue;
        }
        let (code, kind) = match hole.owner {
            physical::HoleOwner::Via(_) => {
                (FindingCode::BoardViaDrillClearance, CopperObject::ViaHole)
            }
            physical::HoleOwner::Pad { .. } => (
                FindingCode::BoardPadHoleClearance,
                CopperObject::hole(hole.plated),
            ),
            _ => (
                FindingCode::BoardHoleClearance,
                CopperObject::hole(hole.plated),
            ),
        };
        let needs = |net| rules.clearance_need(CopperObject::Track, net, kind, hole.net);
        let need = bridge
            .nets
            .iter()
            .map(|n| needs(Some(*n)))
            .max_by(|a, b| a.mm.total_cmp(&b.mm))
            .unwrap_or_else(|| needs(None));
        if !copper.closer_than(ring, need.mm) {
            continue;
        }
        let gap = copper.gap(ring, need.mm + 1.0);
        out.push(
            Finding::new(
                code,
                format!(
                    "{part} bridge is {} mm from the hole of {}; {}",
                    crate::layout::limits::mm_against(gap, need.mm),
                    hole.name,
                    need.text_at(rules.project(), gap)
                ),
                vec![
                    ObjectRef::Instance {
                        id: bridge.instance,
                    },
                    hole.object(),
                ],
            )
            .requiring(need.at_most(gap))
            .measuring([need.measured(gap)]),
        );
    }
    out
}

pub(crate) fn bridge_ring(b: &PlacedBridge) -> CopperRing {
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

/// A boundary inside the other fill, or two boundaries meeting. Scan
/// both directions for containment and include the edges of cut-outs.
fn fill_contact(
    fill: &fill::Fresh<'_>,
    other: &fill::RegionIndex,
) -> Option<crate::geometry::Point> {
    let reach = super::JOIN_TOLERANCE_MM;
    for region in &fill.copper.regions {
        for ring in std::iter::once(&region.outer).chain(&region.holes) {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                if other.region_at(*a).is_some() {
                    return Some(*a);
                }
                let lo = [a[0].min(b[0]) - reach, a[1].min(b[1]) - reach];
                let hi = [a[0].max(b[0]) + reach, a[1].max(b[1]) + reach];
                for (_, c, d) in other.edges_near(lo, hi) {
                    if crate::geometry::segments_gap(*a, *b, c, d) <= reach {
                        return Some(build::nearest_contact(*a, *b, c, d));
                    }
                }
            }
        }
    }
    None
}

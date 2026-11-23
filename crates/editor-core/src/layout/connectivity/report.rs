//! Placed joins and current margins for the project query and inspectors.
use crate::diagnostics::ObjectRef;
use crate::geometry::Bounds;
use crate::ids::{Endpoint, TerminalId};
use crate::instances::ties::{TieRow, rows};
use crate::library::tie::TiePurpose;
use crate::model::{Layer, Project};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// A named schematic tie and the board locations where its ends meet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TieReport {
    #[serde(flatten)]
    pub tie: TieRow,
    pub joins: Vec<JoinPoint>,
    pub anchors: BTreeMap<TerminalId, AnchorReport>,
    pub current: crate::layout::connectivity::TieCurrent,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JoinPoint {
    pub at: crate::geometry::Point,
    pub layer: crate::model::CopperLayer,
    pub bridge: Option<crate::ids::BridgeId>,
    pub islands: Vec<crate::layout::connectivity::TieIsland>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnchorReport {
    pub land: crate::library::tie::Anchor,
    pub sense_island: Option<usize>,
    pub at: Option<crate::geometry::Point>,
    pub contacts: Vec<ObjectRef>,
    pub points: Vec<(crate::geometry::Point, crate::model::CopperLayer)>,
}

/// Include unassigned lands, which have a physical position before they have a copper island.
fn land_sites(project: &Project) -> BTreeMap<(Endpoint, Layer), Bounds> {
    let mut sites = BTreeMap::new();
    for pad in crate::layout::placed_pads(project) {
        if let Some(end) = pad.terminal {
            let here = Bounds::point(pad.center());
            sites
                .entry((end, pad.layer))
                .and_modify(|b: &mut Bounds| *b = b.union(here))
                .or_insert(here);
        }
    }
    sites
}

pub fn report(project: &Project, graph: &crate::layout::CopperGraph) -> Vec<TieReport> {
    let rows = rows(project);
    let mut currents = graph.tie_currents(project, &rows);
    let sites = std::cell::OnceCell::new();
    rows.into_iter()
        .map(|tie| {
            let mut joins: Vec<_> = graph
                .ties()
                .iter()
                .filter(|b| b.bridge.instance == tie.instance && b.bridge.group == tie.group)
                .filter_map(|b| {
                    let ring = b.bridge.outline.flatten(crate::layout::CURVE_SLACK_MM);
                    Some(JoinPoint {
                        at: crate::geometry::polygon_marker(&ring)?,
                        layer: b.bridge.layer,
                        bridge: Some(b.bridge.bridge),
                        islands: b.islands.clone(),
                    })
                })
                .collect();
            let pads = graph.pads();
            if joins.is_empty()
                && !tie.form.needs_bridge()
                && tie.purpose != TiePurpose::KelvinSense
            {
                let sites = sites.get_or_init(|| land_sites(project));
                for layer in [Layer::Top, Layer::Bottom] {
                    let site = tie
                        .ends
                        .keys()
                        .filter_map(|&terminal| {
                            let end = Endpoint {
                                instance: tie.instance,
                                terminal,
                            };
                            sites.get(&(end, layer)).copied()
                        })
                        .reduce(Bounds::union);
                    let Some(site) = site else { continue };
                    let islands = pads
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| {
                            p.terminal.instance == tie.instance
                                && p.instance == tie.instance
                                && tie.ends.contains_key(&p.terminal.terminal)
                                && p.on(layer)
                        })
                        .filter_map(|(i, p)| {
                            Some(super::TieIsland {
                                net: p.net,
                                pad: p.pad,
                                island: graph.pad_island(i)?,
                            })
                        })
                        .collect();
                    joins.push(JoinPoint {
                        at: site.center(),
                        layer: layer.into(),
                        bridge: None,
                        islands,
                    });
                }
            }
            let mut anchors = BTreeMap::new();
            if let Some(usage) = project
                .instance(tie.instance)
                .and_then(|i| i.ties.get(&tie.group))
            {
                for (&end, &land) in &usage.anchors {
                    let at = pads
                        .iter()
                        .find(|p| p.instance == land.instance && p.pad == land.pad)
                        .map(|p| p.position);
                    let contacts = graph
                        .anchor_contacts
                        .get(&(tie.instance, end))
                        .into_iter()
                        .flatten()
                        .copied()
                        .collect();
                    let sense_island = pads
                        .iter()
                        .position(|p| {
                            p.terminal
                                == (Endpoint {
                                    instance: tie.instance,
                                    terminal: end,
                                })
                                && p.instance == land.instance
                                && p.pad == land.pad
                        })
                        .and_then(|i| graph.pad_island(i));
                    anchors.insert(
                        end,
                        AnchorReport {
                            land,
                            at,
                            contacts,
                            points: graph.anchor_points(tie.instance, end).to_vec(),
                            sense_island,
                        },
                    );
                }
            }
            let current = currents
                .remove(&(tie.instance, tie.group))
                .unwrap_or_default();
            TieReport {
                tie,
                joins,
                anchors,
                current,
            }
        })
        .collect()
}

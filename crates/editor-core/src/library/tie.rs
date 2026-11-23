//! Deliberate contacts between separately named and routed nets.
use super::publish::issue;
use super::{
    BindingDefinition, ComponentDefinition, FootprintDefinition, Library, Outline, PackageModel,
    PadKind, PadSide,
};
use crate::diagnostics::{Finding, FindingCode, ObjectRef};
use crate::ids::{BridgeId, DefinitionId, Id, PadId, TerminalId, TieId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use sparksmith_generator_abi::v1::{Assembly, JumperBridge, JumperState, TieForm};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TieSpec {
    pub id: crate::ids::TieId,
    pub ends: Vec<TerminalId>,
    pub form: TieForm,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TiePurpose {
    #[default]
    None,
    StarGround,
    KelvinSense,
    ShieldBond,
    SupplyJoin,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TieUse {
    pub purpose: TiePurpose,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub anchors: BTreeMap<TerminalId, Anchor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub instance: Id,
    pub pad: PadId,
}

/// One fabricated copper region in the footprint frame and its declared lands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bridge {
    pub id: BridgeId,
    pub side: PadSide,
    pub outline: Outline,
    pub pads: Vec<PadId>,
}

impl TieSpec {
    fn valid_ends(&self, component: &ComponentDefinition) -> bool {
        (2..=8).contains(&self.ends.len())
            && (self.form != TieForm::ZeroOhm || self.ends.len() == 2)
            && self.ends.iter().collect::<BTreeSet<_>>().len() == self.ends.len()
            && self.ends.iter().all(|id| {
                component
                    .terminal(*id)
                    .and_then(|t| t.functions())
                    .is_some_and(|fs| {
                        !fs.functions.is_empty()
                            && fs
                                .functions
                                .iter()
                                .all(|f| f.direction == crate::functions::Direction::Passive)
                    })
            })
    }
}

/// A landless copper implementation obtains its physical contacts from anchors.
pub(super) fn anchored(
    component: &ComponentDefinition,
    binding: &BindingDefinition,
    lib: &Library,
) -> bool {
    !component.ties.is_empty()
        && component.ties.iter().all(|t| t.form == TieForm::Copper)
        && lib.package(binding.package).is_some_and(|p| {
            p.definition.terminals.is_empty() && matches!(p.definition.model, PackageModel::NoBody)
        })
        && binding.mappings.is_empty()
        && !binding.footprints.is_empty()
        && binding.footprints.iter().all(|r| {
            lib.footprint(*r)
                .is_some_and(|f| f.definition.pads.is_empty() && f.definition.bridges.is_empty())
        })
}

pub(super) fn findings(
    component: &ComponentDefinition,
    id: DefinitionId,
    lib: &Library,
    out: &mut Vec<Finding>,
) {
    let mut report = |code, message: &str| {
        out.push(issue(
            code,
            message.into(),
            vec![ObjectRef::Definition { id }],
        ))
    };
    let mut ids = BTreeSet::new();
    let mut ends = BTreeSet::new();
    for spec in &component.ties {
        if spec.id.0 == 0 || !ids.insert(spec.id) || spec.ends.iter().any(|e| !ends.insert(*e)) {
            report(
                FindingCode::TieEnds,
                "Independent tie groups need distinct identities and disjoint terminals.",
            );
        }
        let open = |form: TieForm| {
            matches!(
                form,
                TieForm::SolderJumper {
                    default: JumperState::Open,
                    ..
                }
            )
        };
        if component
            .ties
            .first()
            .is_some_and(|s| open(s.form) != open(spec.form))
        {
            report(
                FindingCode::TieAssembly,
                "Tie groups on one component must agree on its default population.",
            );
        }
        if !spec.valid_ends(component) {
            report(
                FindingCode::TieEnds,
                "Choose distinct passive terminals for the tie ends.",
            );
        }
        if component.assembly != spec.form.assembly() {
            report(
                FindingCode::TieAssembly,
                "The assembly choice must agree with the tie form.",
            );
        }
        if component.implementations.is_empty() {
            report(
                FindingCode::TiePackage,
                "Choose a physical implementation for the tie.",
            );
        }
    }
    for binding in component
        .implementations
        .iter()
        .filter_map(|r| lib.binding(*r))
    {
        let binding = &binding.definition;
        if anchored(component, binding, lib) {
            continue;
        }
        let Some(package) = lib.package(binding.package) else {
            continue;
        };
        for fp in binding.footprints.iter().filter_map(|r| lib.footprint(*r)) {
            let fp = &fp.definition;
            if component.ties.is_empty() && fp.bridges.is_empty() {
                continue;
            }
            let pads = TiePads::of(fp, binding, component);
            let mut bridges = BTreeMap::<TieId, Vec<&Bridge>>::new();
            for bridge in &fp.bridges {
                if let Some(group) = pads.owner(bridge) {
                    bridges.entry(group).or_default().push(bridge);
                } else {
                    report(
                        FindingCode::TieUnownedBridge,
                        "Each bridge must contact pads belonging to one declared tie group.",
                    );
                }
            }
            for spec in &component.ties {
                let no_body = matches!(package.definition.model, PackageModel::NoBody);
                if spec.form == TieForm::Copper && !no_body {
                    report(
                        FindingCode::TieBody,
                        "A copper tie needs a package without a body.",
                    );
                }
                if spec.form == TieForm::ZeroOhm {
                    if no_body {
                        report(
                            FindingCode::TieBody,
                            "A zero-ohm link needs a package with a body.",
                        );
                    }
                    if !spec.ends.iter().all(|end| pads.counts.get(end) == Some(&1)) {
                        report(
                            FindingCode::TieEnds,
                            "A zero-ohm link needs one land for each of its two ends.",
                        );
                    }
                }
                if matches!(spec.form, TieForm::SolderJumper { .. })
                    && spec.ends.iter().any(|end| !pads.counts.contains_key(end))
                {
                    report(
                        FindingCode::TieEnds,
                        "Every solder jumper end needs a conductive land.",
                    );
                }
                let group = bridges.get(&spec.id).map_or(&[][..], Vec::as_slice);
                if spec.form.needs_bridge() {
                    if !bridges_join(&pads, spec, group) {
                        report(
                            FindingCode::TieBridge,
                            "The copper bridge must connect the declared lands of every tie end.",
                        );
                    }
                } else if !group.is_empty() {
                    report(
                        FindingCode::TieBridge,
                        "This tie form needs separate lands without fabricated bridges.",
                    );
                }
            }
        }
    }
}

/// Resolve mapped copper lands once per footprint. A bridge belongs to a
/// group only if every land it declares maps to an end of that group.
pub(crate) struct TiePads {
    ends: BTreeMap<PadId, (TerminalId, TieId)>,
    counts: BTreeMap<TerminalId, usize>,
}

impl TiePads {
    pub(crate) fn of(
        fp: &FootprintDefinition,
        binding: &BindingDefinition,
        component: &ComponentDefinition,
    ) -> Self {
        let groups: BTreeMap<_, _> = component
            .ties
            .iter()
            .flat_map(|s| s.ends.iter().map(move |e| (*e, s.id)))
            .collect();
        let mappings: BTreeMap<_, _> = binding
            .mappings
            .iter()
            .map(|m| (m.package_terminal, m.terminal))
            .collect();
        let mut ends = BTreeMap::new();
        let mut counts = BTreeMap::new();
        for pad in &fp.pads {
            if matches!(pad.kind, PadKind::Smd | PadKind::PlatedThrough)
                && let Some(end) = pad.terminal.and_then(|pin| mappings.get(&pin))
                && let Some(group) = groups.get(end)
            {
                ends.insert(pad.id, (*end, *group));
                *counts.entry(*end).or_default() += 1;
            }
        }
        Self { ends, counts }
    }

    pub(crate) fn terminal(&self, pad: PadId) -> Option<TerminalId> {
        self.ends.get(&pad).map(|e| e.0)
    }

    pub(crate) fn owner(&self, bridge: &Bridge) -> Option<TieId> {
        let (_, group) = *self.ends.get(bridge.pads.first()?)?;
        bridge
            .pads
            .iter()
            .all(|p| self.ends.get(p).is_some_and(|e| e.1 == group))
            .then_some(group)
    }
}

// Check the authored pad groups as a connected network. Footprint validation
// separately checks that each declared land actually touches its bridge.
fn bridges_join(pads: &TiePads, spec: &TieSpec, bridges: &[&Bridge]) -> bool {
    let mut groups: Vec<BTreeSet<PadId>> = bridges
        .iter()
        .map(|b| b.pads.iter().copied().collect())
        .collect();
    let Some(mut joined) = groups.pop() else {
        return false;
    };
    while let Some(index) = groups.iter().position(|g| !joined.is_disjoint(g)) {
        joined.extend(groups.swap_remove(index));
    }
    let ends: BTreeSet<_> = joined
        .into_iter()
        .filter_map(|id| pads.terminal(id))
        .collect();
    groups.is_empty() && spec.ends.iter().all(|e| ends.contains(e))
}

impl Bridge {
    pub(super) fn valid(&self, footprint: &FootprintDefinition) -> bool {
        let ring = self.outline.flatten(super::CHECK_CHORD_MM);
        self.outline.closed()
            && ring.len() >= 3
            && crate::geometry::signed_area2(&ring).abs() > 0.0
            && !crate::geometry::polygon_self_intersects(&ring)
            && self.pads.len() >= 2
            && self.pads.iter().collect::<BTreeSet<_>>().len() == self.pads.len()
            && self.pads.iter().all(|id| {
                footprint
                    .pads
                    .iter()
                    .find(|p| p.id == *id)
                    .is_some_and(|pad| bridge_touches_pad(&self.outline, self.side, pad))
            })
    }
}

/// Physical contact in the footprint frame. Used for authored and imported
/// bridges. A boundary contact on the land is sufficient; drill voids are
/// removed before testing the copper.
pub fn bridge_touches_pad(outline: &Outline, side: PadSide, pad: &super::Pad) -> bool {
    if pad.kind != PadKind::PlatedThrough && (pad.kind != PadKind::Smd || pad.side != side) {
        return false;
    }
    let chord = super::PAD_ARC_TOLERANCE_MM;
    let bridge = outline.flatten(chord);
    let land = pad.outline(chord);
    if bridge.len() < 3 || land.len() < 3 {
        return false;
    }
    let edges = |ring: &[crate::geometry::Point]| -> Vec<_> {
        ring.iter()
            .copied()
            .zip(ring.iter().copied().cycle().skip(1))
            .collect()
    };
    // A point on either outer boundary is copper unless it lies strictly inside a drill.
    let holes: Vec<_> = pad
        .hole_shape()
        .into_iter()
        .chain(pad.vias.iter().map(|v| crate::geometry::Shape::Circle {
            center: pad.to_footprint(v.at()),
            radius: v.drill() / 2.,
        }))
        .map(|s| s.flatten(chord))
        .collect();
    let on_land = |p| {
        !holes
            .iter()
            .any(|h| crate::geometry::polygon_contains(h, p))
    };
    if bridge
        .iter()
        .any(|&p| on_land(p) && crate::geometry::polygon_contains(&land, p))
        || land
            .iter()
            .any(|&p| on_land(p) && crate::geometry::polygon_contains(&bridge, p))
    {
        return true;
    }
    let boundary = edges(&land);
    let edge_meets = |a, b, c, d| crate::geometry::segments_gap(a, b, c, d) <= f64::EPSILON * 16.;
    for (a, b) in edges(&bridge) {
        if boundary.iter().any(|&(c, d)| edge_meets(a, b, c, d)) {
            return true;
        }
        for hole in &holes {
            if edges(hole).into_iter().any(|(c, d)| edge_meets(a, b, c, d)) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests;

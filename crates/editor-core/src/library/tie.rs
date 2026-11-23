//! Deliberate contacts between separately named and routed nets.
use super::publish::issue;
use super::{
    BindingDefinition, ComponentDefinition, FootprintDefinition, Library, Outline, PackageModel,
    PadKind, PadSide,
};
use crate::diagnostics::{Finding, ObjectRef};
use crate::ids::{BridgeId, DefinitionId, Id, PadId, TerminalId};
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
            && (self.form != TieForm::ZeroOhm
                || (self.ends.len() == 2 && component.terminals.len() == 2))
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
    component.ties.iter().any(|t| t.form == TieForm::Copper)
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
                "tie.ends",
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
                "tie.assembly",
                "Tie groups on one component must agree on its default population.",
            );
        }
        if !spec.valid_ends(component) {
            report(
                "tie.ends",
                "Choose distinct passive terminals for the tie ends.",
            );
        }
        if component.assembly != spec.form.assembly() {
            report(
                "tie.assembly",
                "The assembly choice must agree with the tie form.",
            );
        }
        if component.implementations.is_empty() {
            report(
                "tie.package",
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
            let group_of = |b: &Bridge| owner(b, fp, binding, component);
            if fp.bridges.iter().any(|b| group_of(b).is_none()) {
                report(
                    "tie.unowned_bridge",
                    "Each bridge must contact pads belonging to one declared tie group.",
                );
            }
            for spec in &component.ties {
                let no_body = matches!(package.definition.model, PackageModel::NoBody);
                if spec.form == TieForm::Copper && !no_body {
                    report("tie.body", "A copper tie needs a package without a body.");
                }
                if spec.form == TieForm::ZeroOhm {
                    if no_body {
                        report("tie.body", "A zero-ohm link needs a package with a body.");
                    }
                    if fp.pads.len() != 2
                        || !spec.ends.iter().all(|end| {
                            fp.pads.iter().any(|p| {
                                p.terminal.and_then(|t| binding.terminal_of(t)) == Some(*end)
                            })
                        })
                    {
                        report(
                            "tie.ends",
                            "A zero-ohm link needs one land for each of its two ends.",
                        );
                    }
                }
                if spec.form.needs_bridge() {
                    if !bridges_join(
                        fp,
                        binding,
                        spec,
                        fp.bridges.iter().filter(|b| group_of(b) == Some(spec.id)),
                    ) {
                        report(
                            "tie.bridge",
                            "The copper bridge must connect the declared lands of every tie end.",
                        );
                    }
                } else if fp.bridges.iter().any(|b| group_of(b) == Some(spec.id)) {
                    report(
                        "tie.bridge",
                        "This tie form needs separate lands without fabricated bridges.",
                    );
                }
            }
        }
    }
}

pub(crate) fn owner(
    b: &Bridge,
    fp: &FootprintDefinition,
    binding: &BindingDefinition,
    component: &ComponentDefinition,
) -> Option<crate::ids::TieId> {
    component
        .ties
        .iter()
        .find(|s| {
            b.pads.iter().all(|id| {
                fp.pad(*id)
                    .and_then(|p| p.terminal)
                    .and_then(|t| binding.terminal_of(t))
                    .is_some_and(|t| s.ends.contains(&t))
            })
        })
        .map(|s| s.id)
}

// Check the authored pad groups as a connected network. Footprint validation
// separately checks that each declared land actually touches its bridge.
fn bridges_join<'a>(
    fp: &FootprintDefinition,
    binding: &BindingDefinition,
    spec: &TieSpec,
    bridges: impl Iterator<Item = &'a Bridge>,
) -> bool {
    let mut groups: Vec<BTreeSet<PadId>> =
        bridges.map(|b| b.pads.iter().copied().collect()).collect();
    let Some(mut joined) = groups.pop() else {
        return false;
    };
    while let Some(index) = groups.iter().position(|g| !joined.is_disjoint(g)) {
        joined.extend(groups.swap_remove(index));
    }
    groups.is_empty()
        && spec.ends.iter().all(|end| {
            fp.pads.iter().any(|p| {
                joined.contains(&p.id)
                    && p.terminal.and_then(|t| binding.terminal_of(t)) == Some(*end)
            })
        })
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

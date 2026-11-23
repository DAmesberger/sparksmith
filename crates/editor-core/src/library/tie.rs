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
    component
        .tie
        .as_ref()
        .is_some_and(|t| t.form == TieForm::Copper)
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
    if let Some(spec) = &component.tie {
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
            let Some(spec) = &component.tie else {
                if !fp.bridges.is_empty() {
                    report(
                        "tie.unowned_bridge",
                        "Only a tie can use a footprint with copper bridges.",
                    );
                }
                continue;
            };
            let no_body = matches!(package.definition.model, PackageModel::NoBody);
            if spec.form == TieForm::Copper && !no_body {
                report("tie.body", "A copper tie needs a package without a body.");
            }
            if spec.form == TieForm::ZeroOhm && no_body {
                report("tie.body", "A zero-ohm link needs a package with a body.");
            }
            if spec.form.needs_bridge() {
                if !bridges_join(fp, binding, spec) {
                    report(
                        "tie.bridge",
                        "The copper bridge must connect the declared lands of every tie end.",
                    );
                }
            } else if !fp.bridges.is_empty() {
                report(
                    "tie.bridge",
                    "This tie form needs separate lands without fabricated bridges.",
                );
            }
        }
    }
}

// Check the authored pad groups as a connected network. Footprint validation
// separately checks that each declared land actually touches its bridge.
fn bridges_join(fp: &FootprintDefinition, binding: &BindingDefinition, spec: &TieSpec) -> bool {
    let mut groups = Vec::new();
    for bridge in &fp.bridges {
        let mut group = BTreeSet::new();
        for pad in &bridge.pads {
            let Some(end) = fp
                .pads
                .iter()
                .find(|p| p.id == *pad)
                .and_then(|p| p.terminal)
                .and_then(|p| binding.terminal_of(p))
            else {
                return false;
            };
            if !spec.ends.contains(&end) {
                return false;
            }
            group.insert(*pad);
        }
        groups.push(group);
    }
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
                    .is_some_and(|pad| {
                        (pad.kind == PadKind::PlatedThrough
                            || (pad.kind == PadKind::Smd && pad.side == self.side))
                            && crate::geometry::polygons_overlap(
                                &ring,
                                &pad.outline(super::CHECK_CHORD_MM),
                            )
                    })
            })
    }
}

#[cfg(test)]
mod tests;

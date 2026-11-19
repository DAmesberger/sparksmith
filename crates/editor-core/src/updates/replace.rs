//! A placed part's component replaced by a component of another lineage
//! (an imported or generated part by a reviewed library part), staged as a
//! [`super::Change::ReplaceComponent`] so the update's preview binds it to
//! the project revision and lists its impact, and its apply is one undo
//! step. Terminal identity is lineage-local, so every terminal of the old
//! component is mapped explicitly to one of the new component's, or to
//! nothing; nothing is inferred from matching names or numbers. Wires,
//! nets, no-connect markers and pair endpoints follow the mapping; the
//! placement keeps its position, side and rotation, and copper the new
//! footprint no longer reaches stays where it is, listed by the preview.
use super::{invalid, not_found};
use crate::diagnostics::{Error, Result};
use crate::fields::FieldOverride;
use crate::fields::{PropertyEdit, PropertyInput};
use crate::ids::{ComponentRef, DefinitionId, Endpoint, Id, TerminalId, UnitId};
use crate::instances::{self, Regroup, UnitCarry, UnitTarget};
use crate::layout;
use crate::model::{EditPolicy, PhysicalChoice, Project, WireEnd};
use crate::schematic;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One old terminal and the new terminal it becomes; `to` absent drops it
/// (its wires are removed and it leaves its net).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TerminalPair {
    /// A terminal of the instances' current component.
    pub from: TerminalId,
    /// A terminal of the target component, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<TerminalId>,
}

/// One old unit (placed or marked unused) and where it goes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UnitPair {
    pub from: UnitId,
    pub to: UnitTarget,
}

/// The parameters of a replacement, shared by every instance named.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    /// The target component revision, of another lineage.
    pub target: ComponentRef,
    /// The target's symbol lineage; required when it offers several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representation: Option<DefinitionId>,
    /// The target's binding and footprint; required when it offers
    /// several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical: Option<PhysicalChoice>,
    /// Every terminal of the current component, once.
    pub terminals: Vec<TerminalPair>,
    /// Where each placed or unused unit goes; may be left out when the old
    /// and the new symbol have one unit each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<UnitPair>,
}

/// The checked terminal mapping: old → new (or dropped).
pub(super) type TerminalMapping = BTreeMap<TerminalId, Option<TerminalId>>;

/// Replace `ids`' component by `replacement.target` on `next`.
pub(super) fn replace(next: &mut Project, ids: &[Id], replacement: &Replacement) -> Result<()> {
    if ids.is_empty() || ids.len() > next.instances.len() {
        return Err(invalid(
            "Name at least one of the project's instances to replace.",
        ));
    }
    let selected: BTreeSet<Id> = ids.iter().copied().collect();
    if selected.len() != ids.len() {
        return Err(invalid("An instance occurs twice in the replacement."));
    }
    let target = next
        .library
        .component(replacement.target)
        .cloned()
        .ok_or_else(|| {
            not_found(
                "The target component revision is not in the project library; import it first.",
            )
        })?;
    let mut lineage = None;
    for id in ids {
        let instance = next
            .instance(*id)
            .ok_or_else(|| not_found("The replacement names an unknown instance."))?;
        if instance.component.id == replacement.target.id {
            return Err(invalid(&format!(
                "{} already uses that component's lineage; update its revision with a component-revision update instead.",
                instance.designator
            )));
        }
        match lineage {
            None => lineage = Some(instance.component),
            Some(c) if c == instance.component => {}
            Some(_) => {
                return Err(invalid(
                    "The instances replaced together must use the same component revision, so one terminal mapping fits them all.",
                ));
            }
        }
    }
    let old_ref = lineage.ok_or_else(|| invalid("No instance to replace."))?;
    let old = next
        .library
        .component(old_ref)
        .cloned()
        .ok_or_else(|| not_found("The instances' component is not in the project library."))?;
    let mapping = check_mapping(
        &old.definition,
        &old.meta.name,
        &target.definition,
        &target.meta.name,
        &replacement.terminals,
    )?;
    let representation = match replacement.representation {
        Some(r) if target.definition.representation(r).is_some() => r,
        Some(r) => {
            return Err(invalid(&format!(
                "‘{}’ does not offer the symbol lineage {r}.",
                target.meta.name
            )));
        }
        None => match target.definition.representations.as_slice() {
            [one] => one.id,
            _ => {
                return Err(invalid(&format!(
                    "‘{}’ offers {} symbols; name one as the representation.",
                    target.meta.name,
                    target.definition.representations.len()
                )));
            }
        },
    };
    let physical = match replacement.physical {
        Some(choice) => {
            instances::resolve_physical(&next.library, &target.definition, choice).map_err(
                |_| {
                    invalid(&format!(
                        "‘{}’ does not offer that binding and footprint.",
                        target.meta.name
                    ))
                },
            )?;
            Some(choice)
        }
        None => only_choice(next, &target.definition, &target.meta.name)?,
    };
    for id in ids {
        replace_instance(next, *id, replacement, &mapping, representation, physical)?;
    }
    for id in ids {
        instances::check_instance(next, *id)?;
    }
    Ok(())
}

/// The target's one binding and footprint, or none when it offers none.
fn only_choice(
    project: &Project,
    target: &crate::library::ComponentDefinition,
    name: &str,
) -> Result<Option<PhysicalChoice>> {
    let mut choices = Vec::new();
    for binding in &target.implementations {
        if let Some(b) = project.library.binding(*binding) {
            for footprint in &b.definition.footprints {
                choices.push(PhysicalChoice {
                    binding: binding.id,
                    footprint: footprint.id,
                });
            }
        }
    }
    match choices.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        _ => Err(invalid(&format!(
            "‘{name}’ offers {} footprints; name the binding and footprint as physical.",
            choices.len()
        ))),
    }
}

/// Every terminal of `old` mapped once, each to a terminal of `target` or
/// to nothing. Refused in plain words otherwise; a name is never matched.
pub(super) fn check_mapping(
    old: &crate::library::ComponentDefinition,
    old_name: &str,
    target: &crate::library::ComponentDefinition,
    target_name: &str,
    pairs: &[TerminalPair],
) -> Result<TerminalMapping> {
    if pairs.len() > crate::library::MAX_PACKAGE_TERMINALS {
        return Err(Error::Limit(format!(
            "A terminal mapping lists at most {} terminals.",
            crate::library::MAX_PACKAGE_TERMINALS
        )));
    }
    let label = |c: &crate::library::ComponentDefinition, t: TerminalId| {
        c.terminal(t)
            .map_or_else(|| t.to_string(), |x| format!("{} ({t})", x.name))
    };
    let mut mapping = TerminalMapping::new();
    for pair in pairs {
        if old.terminal(pair.from).is_none() {
            return Err(invalid(&format!(
                "‘{old_name}’ has no terminal {}; map the terminals of the component being replaced.",
                pair.from
            )));
        }
        if let Some(to) = pair.to
            && target.terminal(to).is_none()
        {
            return Err(invalid(&format!("‘{target_name}’ has no terminal {to}.")));
        }
        if mapping.insert(pair.from, pair.to).is_some() {
            return Err(invalid(&format!(
                "Terminal {} is mapped twice; give each old terminal one new terminal.",
                label(old, pair.from)
            )));
        }
    }
    let missing: Vec<String> = old
        .terminals
        .iter()
        .filter(|t| !mapping.contains_key(&t.id))
        .map(|t| label(old, t.id))
        .collect();
    if !missing.is_empty() {
        return Err(invalid(&format!(
            "The terminal mapping is incomplete: map {} of ‘{old_name}’ to a terminal of ‘{target_name}’, or to none to drop it. Matching names are never taken as the same pin.",
            missing.join(", ")
        )));
    }
    Ok(mapping)
}

fn remap(mapping: &TerminalMapping, id: Id, end: Endpoint) -> Option<Option<Endpoint>> {
    (end.instance == id).then(|| {
        mapping
            .get(&end.terminal)
            .copied()
            .flatten()
            .map(|terminal| Endpoint {
                instance: id,
                terminal,
            })
    })
}

fn replace_instance(
    next: &mut Project,
    id: Id,
    replacement: &Replacement,
    mapping: &TerminalMapping,
    representation: DefinitionId,
    physical: Option<PhysicalChoice>,
) -> Result<()> {
    let old = next
        .instance(id)
        .cloned()
        .ok_or_else(|| not_found("The replacement names an unknown instance."))?;
    let name = old.designator.clone();
    if old.physical.is_some() && physical.is_none() {
        return Err(invalid(&format!(
            "{name} is on the board, but the target offers no footprint."
        )));
    }
    let target = next
        .library
        .component(replacement.target)
        .map(|c| c.definition.clone())
        .ok_or_else(|| not_found("The target component revision is not in the project library."))?;
    let symbol_ref = target
        .representation(representation)
        .ok_or_else(|| invalid("The target's symbol is missing."))?;
    let new_symbol = next
        .library
        .symbol(symbol_ref)
        .map(|s| s.definition.clone())
        .ok_or_else(|| invalid("The target's symbol is missing from the project library."))?;
    // Dropped terminals keep no wires.
    let dropped: BTreeSet<TerminalId> = mapping
        .iter()
        .filter(|(_, to)| to.is_none())
        .map(|(from, _)| *from)
        .collect();
    schematic::disconnect_terminals(next, id, &dropped, EditPolicy::Consequential)?;
    // Units: explicit, or one to one between single-unit symbols.
    let mut units: Vec<(UnitId, UnitTarget)> =
        replacement.units.iter().map(|u| (u.from, u.to)).collect();
    if units.is_empty()
        && let [only] = new_symbol.units.as_slice()
    {
        let used: BTreeSet<UnitId> = next
            .occurrences_of(id)
            .map(|o| o.unit)
            .chain(old.unused_units.iter().copied())
            .collect();
        if let [from] = used.iter().copied().collect::<Vec<_>>().as_slice() {
            units.push((*from, UnitTarget::Unit(only.id)));
        }
    }
    let UnitCarry {
        moved,
        dropped: gone,
        unused,
    } = instances::carry_units(next, &old, &new_symbol, true, &units)?;
    for occurrence in gone {
        schematic::unplace_unit(next, occurrence, EditPolicy::Consequential)?;
    }
    // Every reference to an old terminal follows the mapping.
    for wire in next.wires.iter_mut() {
        for end in [&mut wire.from, &mut wire.to] {
            if let WireEnd::Terminal(endpoint) = *end
                && let Some(mapped) = remap(mapping, id, endpoint)
            {
                *end = WireEnd::Terminal(
                    mapped
                        .ok_or_else(|| invalid("A wire on a dropped terminal was not removed."))?,
                );
            }
        }
    }
    for net in next.nets.iter_mut() {
        let mut terminals: Vec<Endpoint> = net
            .terminals
            .iter()
            .filter_map(|t| remap(mapping, id, *t).unwrap_or(Some(*t)))
            .collect();
        terminals.sort_unstable();
        terminals.dedup();
        net.terminals = terminals;
    }
    for pair in &mut next.rules.diff_pairs {
        if let Some(ends) = &pair.endpoints {
            let all: Vec<Endpoint> = ends
                .positive
                .iter()
                .chain(&ends.negative)
                .copied()
                .collect();
            let mapped: Option<Vec<Endpoint>> = all
                .iter()
                .map(|e| remap(mapping, id, *e).unwrap_or(Some(*e)))
                .collect();
            // A pair end on a dropped terminal leaves the path to the copper.
            pair.endpoints = match mapped.as_deref() {
                Some([a, b, c, d]) => Some(crate::rules::signal::PairEndpoints {
                    positive: [*a, *b],
                    negative: [*c, *d],
                }),
                _ => None,
            };
        }
    }
    for disagreement in &mut next.net_disagreements {
        for group in &mut disagreement.groups {
            *group = group
                .iter()
                .filter_map(|e| remap(mapping, id, *e).unwrap_or(Some(*e)))
                .collect();
        }
        disagreement.groups.retain(|g| !g.is_empty());
    }
    next.net_disagreements.retain(|d| d.groups.len() >= 2);
    let no_connect: BTreeSet<TerminalId> = old
        .no_connect
        .iter()
        .filter_map(|t| mapping.get(t).copied().flatten())
        .collect();
    instances::instance_mut(next, id)?.no_connect = no_connect;
    // A wired terminal whose pin lies in a unit that is not placed needs a
    // decision the replacement does not take: refused in its words.
    let placed: BTreeSet<UnitId> = moved.iter().map(|(_, unit)| *unit).collect();
    let Regroup { places, .. } = instances::regroup(next, id, &new_symbol, &placed, |_| None)?;
    if !places.is_empty() {
        return Err(invalid(&format!(
            "{name}: a wired terminal moves to a unit that is not placed; map the units."
        )));
    }
    // Fields the target does not select drop their overrides; the rest
    // must fit the target's fields.
    let inherit: Vec<PropertyEdit> = old
        .properties
        .iter()
        .filter(|(lineage, value)| {
            target.field(**lineage).is_none() && matches!(value, FieldOverride::Assigned(_))
        })
        .map(|(lineage, _)| PropertyEdit {
            field: *lineage,
            input: PropertyInput::Inherit,
        })
        .collect();
    let properties = instances::carry_properties(&next.library, &old, &target, &inherit)?;
    instances::retarget(
        next,
        id,
        instances::Retarget {
            component: replacement.target,
            representation,
            physical,
            unused_units: unused,
            units: moved,
            properties,
            // Function identities are the old lineage's.
            functions: Default::default(),
        },
    )?;
    let instance = instances::instance_mut(next, id)?;
    // The exact Part and the pin configuration name the old component.
    instance.part = None;
    instance.pin_config = Default::default();
    if physical.is_none() {
        layout::detach_instance(next, id, EditPolicy::Consequential)?;
    }
    Ok(())
}

/// Suggested mappings for a preview: each old terminal with the new
/// terminals of the same name, offered only; applying needs the mapping
/// stated explicitly.
pub fn suggest_by_name(
    old: &crate::library::ComponentDefinition,
    target: &crate::library::ComponentDefinition,
) -> Vec<(TerminalId, String, Vec<TerminalId>)> {
    old.terminals
        .iter()
        .map(|t| {
            let same: Vec<TerminalId> = target
                .terminals
                .iter()
                .filter(|n| n.name.eq_ignore_ascii_case(&t.name))
                .map(|n| n.id)
                .collect();
            (t.id, t.name.clone(), same)
        })
        .collect()
}

/// Two board pads are the same copper: shape, layer, side and through-hole
/// alike, centres within a micrometre and the same turn.
fn same_copper(a: &layout::WorldPad, b: &layout::WorldPad) -> bool {
    const EPS: f64 = 1e-6;
    let turn = (a.rotation_deg - b.rotation_deg).rem_euclid(360.0);
    a.shape == b.shape
        && a.layer == b.layer
        && a.through == b.through
        && a.mirrored == b.mirrored
        && (a.position[0] - b.position[0]).abs() < EPS
        && (a.position[1] - b.position[1]).abs() < EPS
        && (turn < EPS || 360.0 - turn < EPS)
}

/// Per replaced instance: whether its copper on the board stays the same.
pub(super) fn copper_identity(
    project: &Project,
    next: &Project,
    ids: &[Id],
) -> Vec<super::CopperIdentity> {
    let (before, after) = (layout::pads(project), layout::pads(next));
    let (old_labels, new_labels) = (layout::pad_labels(project), layout::pad_labels(next));
    let label = |labels: &BTreeMap<(Id, crate::ids::PadId), String>, pad: &layout::WorldPad| {
        labels
            .get(&(pad.instance, pad.pad))
            .cloned()
            .unwrap_or_else(|| pad.pad.to_string())
    };
    ids.iter()
        .filter_map(|id| {
            let designator = next.instance(*id)?.designator.clone();
            let old: Vec<&layout::WorldPad> = before.iter().filter(|p| p.instance == *id).collect();
            let new: Vec<&layout::WorldPad> = after.iter().filter(|p| p.instance == *id).collect();
            let mut matched = vec![false; new.len()];
            let (mut old_only, mut renumbered) = (Vec::new(), Vec::new());
            for pad in &old {
                let found = new.iter().enumerate().find(|(i, n)| {
                    !matched.get(*i).copied().unwrap_or(true) && same_copper(pad, n)
                });
                match found {
                    Some((i, n)) => {
                        if let Some(slot) = matched.get_mut(i) {
                            *slot = true;
                        }
                        let (a, b) = (label(&old_labels, pad), label(&new_labels, n));
                        if a != b {
                            renumbered.push(format!("{a} → {b}"));
                        }
                    }
                    None => old_only.push(label(&old_labels, pad)),
                }
            }
            let new_only: Vec<String> = new
                .iter()
                .zip(&matched)
                .filter(|(_, m)| !**m)
                .map(|(n, _)| label(&new_labels, n))
                .collect();
            Some(super::CopperIdentity {
                instance: *id,
                designator,
                identical: old_only.is_empty() && new_only.is_empty(),
                old_only,
                new_only,
                renumbered,
            })
        })
        .collect()
}

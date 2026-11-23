//! A placed part's component replaced by a component of another lineage
//! (an imported or generated part by a reviewed library part), staged as a
//! [`super::Change::ReplaceComponent`] so the update's preview binds it to
//! the project revision and lists its impact, and its apply is one undo
//! step. Terminal identity is lineage-local, so every terminal of the old
//! component is mapped explicitly to one of the new component's, or to
//! nothing; nothing is inferred from matching names or numbers. Wires,
//! nets, no-connect markers and pair endpoints follow the mapping. The
//! placement keeps its side; by default it moves (and turns by quarter
//! turns) so the new footprint's mapped pads best fit the old ones, since
//! two footprints of one package may put their origins in different
//! places ([`Alignment`]); `keep_origin` keeps position and rotation.
//! Copper the new footprint no longer reaches stays where it is, listed by
//! the preview.
mod copper;
pub(super) use copper::copper_identity;

use super::{invalid, not_found};
use crate::diagnostics::{Error, Result};
use crate::fields::FieldOverride;
use crate::fields::{PropertyEdit, PropertyInput};
use crate::geometry::Point;
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
    /// Where a placed part's new footprint goes: fitted to its mapped
    /// pads (the default) or at the old origin.
    #[serde(default, skip_serializing_if = "Alignment::is_fit")]
    pub alignment: Alignment,
}

/// Where a replaced part's new footprint is placed on the board.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// Moved, and turned by quarter turns, so the pads of each mapped
    /// terminal best fit the old pads mapped to it (least squares over
    /// each terminal's pad centres); the side stays.
    #[default]
    FitPads,
    /// The old placement's origin, rotation and side, whatever the new
    /// footprint's origin.
    KeepOrigin,
}

impl Alignment {
    fn is_fit(&self) -> bool {
        *self == Self::FitPads
    }
}

/// How far one mapped terminal's new pads lie from the old pads mapped to
/// it, centroid to centroid, after the placement.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PadResidual {
    /// The old pads, by number.
    pub old: Vec<String>,
    /// The new pads, by number.
    pub new: Vec<String>,
    pub residual_mm: f64,
}

/// How a replaced part is placed: the move and turn the alignment chose
/// and the residual of every mapped terminal with pads on both footprints.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlacementFit {
    pub instance: Id,
    pub designator: String,
    pub alignment: Alignment,
    /// The placement's move on the board, mm.
    pub moved_mm: [f64; 2],
    /// The placement's turn, degrees, in (−180, 180].
    pub turned_deg: f64,
    /// Largest residual first.
    pub pads: Vec<PadResidual>,
    pub max_residual_mm: f64,
}

impl PlacementFit {
    /// "U3 is placed so its mapped pads fit the old ones: moves (0.000,
    /// 3.150) mm, turns 0°; largest pad residual 0.000 mm (41 → 41)."
    pub fn line(&self) -> String {
        let how = match self.alignment {
            Alignment::FitPads => "is placed so its mapped pads fit the old ones",
            Alignment::KeepOrigin => "keeps its origin",
        };
        let worst = self
            .pads
            .first()
            .map(|p| format!(" ({} → {})", p.old.join(", "), p.new.join(", ")))
            .unwrap_or_default();
        format!(
            "{} {how}: moves ({:.3}, {:.3}) mm, turns {}°; largest pad residual {:.3} mm{worst}.",
            self.designator,
            self.moved_mm[0],
            self.moved_mm[1],
            self.turned_deg,
            self.max_residual_mm
        )
    }
}

/// One mapped terminal's pads on both footprints, as centroids: `from` the
/// new pads', `to` the old pads'.
struct Correspondence {
    old: Vec<String>,
    new: Vec<String>,
    from: Point,
    to: Point,
}

type PadsOf<'a> = (
    Vec<&'a layout::PlacedPadCentre>,
    Vec<&'a layout::PlacedPadCentre>,
);

/// Each new terminal's pads paired with the old pads of every terminal
/// mapped to it; a terminal without pads on either side is left out.
fn correspondences(
    mapping: &TerminalMapping,
    old: &[layout::PlacedPadCentre],
    new: &[layout::PlacedPadCentre],
) -> Vec<Correspondence> {
    let mut by_new: BTreeMap<TerminalId, PadsOf<'_>> = BTreeMap::new();
    for pad in old {
        if let Some(Some(to)) = mapping.get(&pad.terminal) {
            by_new.entry(*to).or_default().0.push(pad);
        }
    }
    for pad in new {
        if let Some(entry) = by_new.get_mut(&pad.terminal) {
            entry.1.push(pad);
        }
    }
    by_new
        .into_values()
        .filter_map(|(olds, news)| {
            Some(Correspondence {
                from: centroid(news.iter().map(|p| p.position))?,
                to: centroid(olds.iter().map(|p| p.position))?,
                old: olds.iter().map(|p| p.label.clone()).collect(),
                new: news.iter().map(|p| p.label.clone()).collect(),
            })
        })
        .collect()
}

fn centroid(points: impl Iterator<Item = Point>) -> Option<Point> {
    let (mut sum, mut n) = ([0.0, 0.0], 0.0);
    for p in points {
        sum = [sum[0] + p[0], sum[1] + p[1]];
        n += 1.0;
    }
    (n > 0.0).then(|| [sum[0] / n, sum[1] / n])
}

fn turn(point: Point, deg: f64) -> Point {
    let (s, c) = deg.to_radians().sin_cos();
    [c * point[0] - s * point[1], s * point[0] + c * point[1]]
}

/// The rigid move that best carries each `from` onto its `to`, as a turn
/// (degrees) and then a shift: the least-squares rotation (2D Kabsch)
/// rounded to the nearest quarter turn, as footprints of one package are
/// drawn on the same axes, then the least-squares shift for that turn. A
/// misfit of any other angle shows in the residuals. `None` without a
/// pair.
fn fit(pairs: &[(Point, Point)]) -> Option<(f64, Point)> {
    let from = centroid(pairs.iter().map(|(a, _)| *a))?;
    let to = centroid(pairs.iter().map(|(_, b)| *b))?;
    let (mut cross, mut dot) = (0.0, 0.0);
    for (a, b) in pairs {
        let a = [a[0] - from[0], a[1] - from[1]];
        let b = [b[0] - to[0], b[1] - to[1]];
        cross += a[0] * b[1] - a[1] * b[0];
        dot += a[0] * b[0] + a[1] * b[1];
    }
    let free = if cross == 0.0 && dot == 0.0 {
        0.0
    } else {
        cross.atan2(dot).to_degrees()
    };
    let quarter = ((free / 90.0).round() * 90.0).rem_euclid(360.0);
    let turned = turn(from, quarter);
    Some((quarter, [to[0] - turned[0], to[1] - turned[1]]))
}

/// A length on the nanometre grid the board stores.
fn on_grid(mm: f64) -> f64 {
    (mm * 1e6).round() / 1e6
}

/// Move `id`'s placement so its mapped pads fit `before`, the old
/// footprint's pads on the board.
fn align(
    next: &mut Project,
    id: Id,
    mapping: &TerminalMapping,
    before: &[layout::PlacedPadCentre],
) -> Result<()> {
    let after = layout::placed_pad_centres(next, id);
    let pairs: Vec<(Point, Point)> = correspondences(mapping, before, &after)
        .into_iter()
        .map(|c| (c.from, c.to))
        .collect();
    let Some((quarter, shift)) = fit(&pairs) else {
        return Ok(());
    };
    let placement = next
        .placements
        .iter_mut()
        .find(|p| p.instance == id)
        .ok_or_else(|| not_found("The replaced part's placement is missing."))?;
    let moved = turn(placement.position, quarter);
    let position = [on_grid(moved[0] + shift[0]), on_grid(moved[1] + shift[1])];
    if !position.iter().all(|v| v.is_finite()) {
        return Err(invalid("The fitted placement is not a finite position."));
    }
    placement.position = position;
    placement.rotation_deg = (placement.rotation_deg + quarter).rem_euclid(360.0);
    Ok(())
}

/// Per replaced, placed instance: how it is placed and how far each mapped
/// terminal's new pads lie from the old ones; `project` before and `next`
/// after the replacement.
pub(super) fn placement_fits(
    project: &Project,
    next: &Project,
    ids: &[Id],
    replacement: &Replacement,
) -> Vec<PlacementFit> {
    let mapping: TerminalMapping = replacement
        .terminals
        .iter()
        .map(|p| (p.from, p.to))
        .collect();
    ids.iter()
        .filter_map(|id| {
            let a = project.placements.iter().find(|p| p.instance == *id)?;
            let b = next.placements.iter().find(|p| p.instance == *id)?;
            let before = layout::placed_pad_centres(project, *id);
            let after = layout::placed_pad_centres(next, *id);
            let mut pads: Vec<PadResidual> = correspondences(&mapping, &before, &after)
                .into_iter()
                .map(|c| PadResidual {
                    old: c.old,
                    new: c.new,
                    residual_mm: on_grid((c.from[0] - c.to[0]).hypot(c.from[1] - c.to[1])),
                })
                .collect();
            pads.sort_by(|x, y| y.residual_mm.total_cmp(&x.residual_mm));
            let turned = (b.rotation_deg - a.rotation_deg).rem_euclid(360.0);
            Some(PlacementFit {
                instance: *id,
                designator: next.instance(*id)?.designator.clone(),
                alignment: replacement.alignment,
                moved_mm: [
                    on_grid(b.position[0] - a.position[0]),
                    on_grid(b.position[1] - a.position[1]),
                ],
                turned_deg: if turned > 180.0 {
                    turned - 360.0
                } else {
                    turned
                },
                max_residual_mm: pads.first().map_or(0.0, |p| p.residual_mm),
                pads,
            })
        })
        .collect()
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
    for (local, family) in next.sheet_part_families(ids) {
        // Run the terminal and unit mapping on the drawing exactly once. The
        // original physical part may already have left the board.
        super::edit_drawing_part(next, local, |next| {
            replace_instance(next, local, replacement, &mapping, representation, physical)
        })?;
        for &id in &family {
            if id != local {
                // Its units are in the drawing. The independent physical
                // part still needs its net references and pad fit carried.
                let old = next
                    .instance(id)
                    .ok_or_else(|| invalid("Missing physical part."))?;
                let mut choice = replacement.clone();
                choice.units.retain(|u| old.unused_units.contains(&u.from));
                replace_instance(next, id, &choice, &mapping, representation, physical)?;
            }
        }
        for id in family {
            instances::check_instance(next, id)?;
        }
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
    let before = layout::placed_pad_centres(next, id);
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
    } else if replacement.alignment == Alignment::FitPads {
        align(next, id, mapping, &before)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(terminal: u32, label: &str, position: Point) -> layout::PlacedPadCentre {
        layout::PlacedPadCentre {
            terminal: TerminalId(terminal),
            label: label.into(),
            position,
        }
    }

    fn near(a: Point, b: Point) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    /// A pure shift is found as a shift, without a turn.
    #[test]
    fn a_shift_fits_without_turning() -> Result<()> {
        let pairs = [([0.0, 0.0], [3.15, 0.0]), ([4.0, 1.0], [7.15, 1.0])];
        let (quarter, shift) = fit(&pairs).ok_or_else(|| invalid("no fit"))?;
        assert_eq!(quarter, 0.0);
        assert!(near(shift, [3.15, 0.0]), "{shift:?}");
        Ok(())
    }

    /// A quarter turn with a shift is found; a slightly skewed fit rounds
    /// to no turn, its misfit left to the residuals.
    #[test]
    fn a_quarter_turn_fits_by_turning() -> Result<()> {
        let from = [[0.0, 0.0], [4.0, 0.0], [2.0, 3.0]];
        let pairs: Vec<(Point, Point)> = from
            .iter()
            .map(|p| {
                let t = turn(*p, 90.0);
                (*p, [t[0] + 10.0, t[1] - 2.0])
            })
            .collect();
        let (quarter, shift) = fit(&pairs).ok_or_else(|| invalid("no fit"))?;
        assert_eq!(quarter, 90.0);
        assert!(near(shift, [10.0, -2.0]), "{shift:?}");
        let skewed: Vec<(Point, Point)> = from.iter().map(|p| (*p, turn(*p, 3.0))).collect();
        assert_eq!(fit(&skewed).map(|f| f.0), Some(0.0));
        assert_eq!(fit(&[]), None);
        Ok(())
    }

    /// Several old terminals mapped to one new terminal (a thermal pad's
    /// windows to one pad) are one correspondence: their centroid against
    /// the new pad's. A dropped terminal takes no part.
    #[test]
    fn terminals_mapped_together_fit_by_their_centroid() {
        let mapping: TerminalMapping = [
            (TerminalId(1), Some(TerminalId(1))),
            (TerminalId(2), Some(TerminalId(9))),
            (TerminalId(3), Some(TerminalId(9))),
            (TerminalId(4), None),
        ]
        .into_iter()
        .collect();
        let old = [
            pad(1, "1", [-5.0, 0.0]),
            pad(2, "41_1", [-1.0, 0.0]),
            pad(3, "41_2", [1.0, 0.0]),
            pad(4, "NC", [9.0, 9.0]),
        ];
        let new = [pad(1, "1", [-5.0, 0.0]), pad(9, "41", [0.0, 0.0])];
        let found = correspondences(&mapping, &old, &new);
        assert_eq!(found.len(), 2);
        let thermal = found.iter().find(|c| c.new == ["41"]);
        assert!(thermal.is_some_and(|c| c.old == ["41_1", "41_2"] && near(c.to, [0.0, 0.0])));
    }
}

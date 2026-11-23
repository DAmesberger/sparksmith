//! Expand the definition DAG into stable occurrences before net reconciliation.
//! Drawings retain their local identifiers. Only physical components are allocated.
use super::{Result, conflict, invalid};
use crate::diagnostics::Error;
use crate::ids::{Id, SheetId};
use crate::model::{
    ComponentInstance, Project,
    sheet::{SheetPath, SheetUse},
};
use std::collections::{BTreeMap, BTreeSet};

/// Check every definition, including disconnected ones, before expanding.
/// The size check counts a lower bound on bytes the saved path table needs.
pub fn paths(project: &Project, cancelled: impl Fn() -> bool) -> Result<Vec<(SheetPath, SheetId)>> {
    let mut children = BTreeMap::<_, Vec<_>>::new();
    let mut degrees: BTreeMap<_, usize> = project.sheets.list.iter().map(|s| (s.id, 0)).collect();
    for symbol in &project.sheet_symbols {
        let degree = degrees
            .get_mut(&symbol.child)
            .ok_or_else(|| invalid("Missing child sheet."))?;
        *degree = degree.checked_add(1).ok_or_else(expansion_limit)?;
        children
            .entry(symbol.sheet)
            .or_default()
            .push((symbol.id, symbol.child));
    }
    let mut ready: Vec<_> = degrees
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut pending: Vec<_> = ready
        .iter()
        .filter(|id| !project.sheets.definitions.contains(id))
        .map(|id| (SheetPath::root(*id), *id))
        .collect();
    // Count paths and total path lengths on the DAG before enumerating any.
    // Even a small drawing can double the number of occurrences at every level.
    let minimum_record = serde_json::to_vec(&SheetUse {
        path: SheetPath::root(SheetId::FIRST),
        definition: SheetId::FIRST,
        parts: BTreeMap::new(),
    })?
    .len();
    let mut totals: BTreeMap<SheetId, (usize, usize)> =
        pending.iter().map(|(_, sheet)| (*sheet, (1, 0))).collect();
    // Charge the electrical nodes expanded by the net graph as well. A
    // repeated sheet can contain many labels or sheet pins and no parts.
    let bytes_per_point = serde_json::to_vec(&(0usize, crate::model::WireEnd::junction(1)))?.len();
    let mut points = BTreeMap::<SheetId, usize>::new();
    for sheet in project
        .labels
        .iter()
        .map(|l| l.sheet)
        .chain(project.junctions.iter().map(|j| j.sheet))
        .chain(
            project
                .sheet_symbols
                .iter()
                .flat_map(|s| s.pins.iter().map(|_| s.sheet)),
        )
    {
        *points.entry(sheet).or_default() += 1;
    }
    let mut bytes = 0usize;
    let mut visited = 0;
    while let Some(sheet) = ready.pop() {
        if cancelled() {
            return Err(conflict("Sheet expansion was cancelled."));
        }
        visited += 1;
        let (count, lengths) = totals.get(&sheet).copied().unwrap_or_default();
        let nodes = points.get(&sheet).copied().unwrap_or_default();
        bytes = count
            .checked_mul(nodes)
            .and_then(|n| n.checked_mul(bytes_per_point))
            .and_then(|n| bytes.checked_add(n))
            .filter(|n| *n <= crate::model::DOCUMENT_BYTES)
            .ok_or_else(expansion_limit)?;
        for (_, child) in children.get(&sheet).into_iter().flatten() {
            let length = lengths.checked_add(count).ok_or_else(expansion_limit)?;
            let total = totals.entry(*child).or_default();
            total.0 = total.0.checked_add(count).ok_or_else(expansion_limit)?;
            total.1 = total.1.checked_add(length).ok_or_else(expansion_limit)?;
            bytes = minimum_record
                .checked_mul(count)
                .and_then(|n| n.checked_add(length))
                .and_then(|n| n.checked_add(bytes))
                .ok_or_else(expansion_limit)?;
            if bytes > crate::model::DOCUMENT_BYTES {
                return Err(expansion_limit());
            }
            if let Some(n) = degrees.get_mut(child) {
                *n -= 1;
                if *n == 0 {
                    ready.push(*child);
                }
            }
        }
    }
    if visited != degrees.len() {
        return Err(conflict("The sheet hierarchy has a cycle."));
    }
    let mut out = Vec::new();
    while let Some((path, sheet)) = pending.pop() {
        if cancelled() {
            return Err(conflict("Sheet expansion was cancelled."));
        }
        for (symbol, child) in children.get(&sheet).into_iter().flatten() {
            pending.push((path.child(*symbol), *child));
        }
        out.push((path, sheet));
    }
    out.sort_unstable();
    Ok(out)
}

fn expansion_limit() -> Error {
    Error::Limit("The expanded sheet paths exceed the project document size.".into())
}

fn locals_by_sheet(project: &Project) -> BTreeMap<SheetId, BTreeSet<Id>> {
    let mut parts = BTreeMap::<_, BTreeSet<_>>::new();
    for o in &project.occurrences {
        parts.entry(o.sheet).or_default().insert(o.instance);
    }
    parts
}

fn check_units(project: &Project, paths: &[(SheetPath, SheetId)]) -> Result<()> {
    let mut counts = BTreeMap::<_, usize>::new();
    for (_, sheet) in paths {
        *counts.entry(*sheet).or_default() += 1;
    }
    let mut sheets = BTreeMap::<_, BTreeSet<_>>::new();
    for o in &project.occurrences {
        sheets.entry(o.instance).or_default().insert(o.sheet);
    }
    for (id, sheets) in sheets {
        if sheets.len() > 1 && sheets.iter().any(|sheet| counts.get(sheet) != Some(&1)) {
            let part = project
                .drawing_instance(id)
                .ok_or_else(|| invalid("Missing component in sheet definition."))?;
            let units = project
                .occurrences_of(id)
                .map(|o| {
                    format!(
                        "unit {} on {}",
                        o.unit,
                        project.sheet(o.sheet).map_or("missing sheet", |s| &s.name)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(conflict(&format!(
                "Bring the units of {} onto one sheet before repeating it: {units}.",
                part.designator
            )));
        }
    }
    Ok(())
}

/// These choices belong to the drawing. The designator, board markings,
/// text offsets and population remain independent on each physical part.
fn share(target: &mut ComponentInstance, source: &ComponentInstance) {
    target.component = source.component;
    target.representation = source.representation;
    target.physical = source.physical;
    target.properties.clone_from(&source.properties);
    target.unused_units.clone_from(&source.unused_units);
    target.no_connect.clone_from(&source.no_connect);
    target.functions.clone_from(&source.functions);
    target.pin_config.clone_from(&source.pin_config);
    target.part = source.part;
}

fn shared_equal(a: &ComponentInstance, b: &ComponentInstance) -> bool {
    a.component == b.component
        && a.representation == b.representation
        && a.physical == b.physical
        && a.properties == b.properties
        && a.unused_units == b.unused_units
        && a.no_connect == b.no_connect
        && a.functions == b.functions
        && a.pin_config == b.pin_config
        && a.part == b.part
}

pub(crate) fn reconcile(
    base: &Project,
    project: &mut Project,
    reviewed: bool,
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    if project.sheet_symbols.is_empty()
        && project.sheets.definitions.is_empty()
        && base.sheets.uses.is_empty()
    {
        return Ok(());
    }
    project
        .sheets
        .definitions
        .extend(project.sheet_symbols.iter().map(|s| s.child));
    let paths = paths(project, &cancelled)?;
    check_units(project, &paths)?;
    let locals = locals_by_sheet(project);
    let mut sources = BTreeMap::new();
    for sheet in &project.sheets.definitions {
        for local in locals.get(sheet).into_iter().flatten() {
            let source = project
                .drawing_instance(*local)
                .ok_or_else(|| invalid("Missing component in sheet definition."))?;
            sources.insert(*local, source.clone());
        }
    }
    let old: BTreeMap<_, _> = base.sheets.uses.iter().map(|u| (&u.path, u)).collect();
    let mut old_ids = BTreeSet::new();
    let mut old_locals = BTreeSet::new();
    for u in &base.sheets.uses {
        for (local, physical) in &u.parts {
            old_ids.insert(*physical);
            old_locals.insert(*local);
            if let (Some(source), Some(before), Some(after)) = (
                sources.get_mut(local),
                base.instance(*physical),
                project.instance(*physical),
            ) && !shared_equal(before, after)
            {
                share(source, after);
            }
        }
    }
    let mut adopted = BTreeSet::new();
    let mut counts = BTreeMap::<_, usize>::new();
    for (_, sheet) in &paths {
        *counts.entry(*sheet).or_default() += 1;
    }
    let mut stored_bytes = 0usize;
    for (sheet, count) in &counts {
        for local in locals.get(sheet).into_iter().flatten() {
            if let Some(source) = sources.get(local) {
                let size = serde_json::to_vec(source)?.len();
                stored_bytes = size
                    .checked_mul(*count)
                    .and_then(|n| stored_bytes.checked_add(n))
                    .filter(|n| *n <= crate::model::DOCUMENT_BYTES)
                    .ok_or_else(expansion_limit)?;
            }
        }
    }
    let mut uses = Vec::new();
    let mut live = BTreeSet::new();
    for (path, definition) in paths.into_iter().filter(|(p, _)| !p.symbols.is_empty()) {
        let mut parts = BTreeMap::new();
        for local in locals.get(&definition).into_iter().flatten() {
            if cancelled() {
                return Err(conflict("Sheet expansion was cancelled."));
            }
            let source = sources
                .get(local)
                .ok_or_else(|| invalid("Missing component in sheet definition."))?;
            let kept = old
                .get(&path)
                .filter(|u| u.definition == definition)
                .and_then(|u| u.parts.get(local))
                .copied();
            // Nesting a formerly top-level design retains its first channel's physical IDs.
            let retained = kept.or_else(|| {
                old.iter()
                    .find(|(p, u)| {
                        p.root != path.root
                            && u.definition == definition
                            && path.symbols.ends_with(&p.symbols)
                            && !p.symbols.is_empty()
                    })
                    .and_then(|(_, u)| u.parts.get(local))
                    .copied()
                    .filter(|id| !live.contains(id))
            });
            let physical = if let Some(id) = retained {
                id
            } else if (!old_locals.contains(local)
                || (counts.get(&definition) == Some(&1)
                    && project
                        .occurrences_of(*local)
                        .any(|o| o.sheet != definition)))
                && !project.sheets.locals.iter().any(|i| i.id == *local)
                && (adopted.insert(*local) || counts.get(&definition) == Some(&1))
            {
                *local
            } else {
                let id = project.allocate()?;
                let prefix = &project
                    .library
                    .component(source.component)
                    .ok_or_else(|| invalid("Missing component revision."))?
                    .definition
                    .designator_prefix;
                let designator = crate::instances::next_designator(project, prefix)?;
                project.instances.push(source.duplicate(id, designator));
                id
            };
            live.insert(physical);
            parts.insert(*local, physical);
            if let Some(instance) = project.instance(physical) {
                if !shared_equal(instance, source) {
                    share(crate::instances::instance_mut(project, physical)?, source);
                }
            } else {
                return Err(conflict(
                    "Remove this component from its shared sheet definition.",
                ));
            }
        }
        uses.push(SheetUse {
            path,
            definition,
            parts,
        });
    }
    let all_locals: BTreeSet<_> = sources.keys().copied().collect();
    let mut retired: BTreeSet<_> = old_ids
        .union(&all_locals)
        .copied()
        .filter(|id| !live.contains(id))
        .collect();
    // A unit moved to an ordinary page, or returned to unplaced, keeps its
    // original component. Only the extra physical occurrences disappear.
    retired.retain(|id| !old_locals.contains(id) || all_locals.contains(id));
    if !reviewed
        && project
            .placements
            .iter()
            .any(|p| retired.contains(&p.instance))
    {
        return Err(conflict(
            "Review the placements removed with this sheet symbol.",
        ));
    }
    if !retired.is_empty() {
        project
            .placements
            .retain(|p| !retired.contains(&p.instance));
        project.instances.retain(|i| !retired.contains(&i.id));
    }
    project.sheets.locals = sources
        .into_values()
        .filter(|i| !live.contains(&i.id))
        .collect();
    if *project.sheets.uses != uses {
        project.sheets.uses = uses.into();
    }
    Ok(())
}

pub(crate) fn validate(project: &Project) -> Result<()> {
    if project.sheet_symbols.is_empty()
        && project.sheets.definitions.is_empty()
        && project.sheets.uses.is_empty()
        && project.sheets.locals.is_empty()
    {
        return Ok(());
    }
    let paths = paths(project, || false)?;
    check_units(project, &paths)?;
    let locals = locals_by_sheet(project);
    let want: BTreeMap<_, _> = paths
        .into_iter()
        .filter(|(p, _)| !p.symbols.is_empty())
        .collect();
    if want.len() != project.sheets.uses.len()
        || project
            .sheets
            .definitions
            .iter()
            .any(|id| project.sheet(*id).is_none())
    {
        return Err(invalid(
            "Sheet occurrence paths do not match the hierarchy.",
        ));
    }
    let drawing_ids: BTreeSet<_> = locals.values().flatten().copied().collect();
    let mut visited = BTreeSet::new();
    let mut assigned = BTreeMap::new();
    for u in &project.sheets.uses {
        if want.get(&u.path) != Some(&u.definition)
            || !visited.insert(&u.path)
            || !project.sheets.definitions.contains(&u.definition)
            || !u
                .parts
                .keys()
                .eq(locals.get(&u.definition).into_iter().flatten())
        {
            return Err(invalid("Invalid sheet occurrence binding."));
        }
        for (local, physical) in &u.parts {
            let source = project
                .drawing_instance(*local)
                .ok_or_else(|| invalid("Missing sheet-local component."))?;
            let instance = project
                .instance(*physical)
                .ok_or_else(|| invalid("Missing physical sheet component."))?;
            if (*local != *physical && drawing_ids.contains(physical))
                || instance.board_only
                || assigned
                    .insert(*physical, (*local, u.definition))
                    .is_some_and(|(l, s)| l != *local || s == u.definition)
                || !shared_equal(source, instance)
            {
                return Err(invalid(
                    "Physical component disagrees with its sheet definition.",
                ));
            }
        }
    }
    for definition in &project.sheets.definitions {
        for local in locals.get(definition).into_iter().flatten() {
            if project.instance(*local).is_some() && !assigned.contains_key(local) {
                return Err(invalid(
                    "An unused sheet component cannot be a physical board part.",
                ));
            }
        }
    }
    for local in &project.sheets.locals {
        if project.instances.iter().any(|i| i.id == local.id)
            || !locals
                .iter()
                .any(|(s, ids)| project.sheets.definitions.contains(s) && ids.contains(&local.id))
        {
            return Err(invalid("Invalid sheet-local component."));
        }
    }
    Ok(())
}

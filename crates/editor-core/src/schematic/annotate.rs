//! Designator annotation across sheets (`schematic.annotate`).
//!
//! Instances are visited in page order, then by position on their sheet
//! (rows: top to bottom, then left to right; columns: left to right, then
//! top to bottom), at their first placed unit; unplaced instances come last
//! in creation order. A *numbered* designator ends in a digit (`R12`, and a
//! name a person gave, such as `J1` on a component whose prefix is `U`). By
//! default only the others (`R?`, `R`, `U_new`) are numbered, as the
//! component's prefix with the lowest free number, so a numbered designator
//! never changes; `all` renumbers every visited instance, which a person
//! sees listed first in the window.
use super::{Result, invalid};
use crate::ids::{Id, SheetId};
use crate::instances;
use crate::model::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnnotateOrder {
    /// Top to bottom, then left to right.
    #[default]
    Rows,
    /// Left to right, then top to bottom.
    Columns,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnnotateMode {
    /// Number only designators that do not end in a digit; keep the rest,
    /// whatever their letters.
    #[default]
    Unannotated,
    /// Renumber every visited instance.
    All,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Numbering {
    /// 1, 2, 3 … across the whole design.
    #[default]
    Sequential,
    /// From 100 × page + 1 on each page (R101 on page 1, R201 on page 2).
    SheetHundreds,
}

/// One designator `schematic.annotate` changed.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Renumbered {
    pub instance: Id,
    pub from: String,
    pub to: String,
}

/// Whether `designator` is numbered: it ends in a digit. Annotation keeps
/// it unless asked to renumber all, whatever its letters: a name a person
/// gave (`J1` on a component with the prefix `U`) is theirs.
pub fn numbered(designator: &str) -> bool {
    designator
        .bytes()
        .last()
        .is_some_and(|b| b.is_ascii_digit())
}

/// Where an instance is visited: page, then the order's coordinates.
type Key = (usize, i64, i64, Id);

fn key(
    project: &Project,
    instance: Id,
    order: AnnotateOrder,
    pages: &BTreeMap<(SheetId, Option<&crate::model::sheet::SheetPath>), usize>,
) -> Key {
    let binding = project.sheet_binding(instance);
    let first = project
        .physical_units(instance)
        .filter_map(|o| {
            let path = binding
                .filter(|(u, _)| u.definition == o.sheet)
                .map(|(u, _)| &u.path);
            Some((*pages.get(&(o.sheet, path))?, o))
        })
        .min_by_key(|(page, o)| (*page, o.unit));
    let Some((page, o)) = first else {
        return (usize::MAX, 0, 0, instance);
    };
    // Whole nanometres: sorting is total and exact.
    let x = o.position.x_nm.get();
    let down = -o.position.y_nm.get();
    match order {
        AnnotateOrder::Rows => (page, down, x, instance),
        AnnotateOrder::Columns => (page, x, down, instance),
    }
}

/// Number designators. `sheets` limits which instances are visited (by the
/// sheet of their first unit); empty visits all, and unplaced instances
/// are visited only then. Returns what changed.
pub fn annotate(
    project: &mut Project,
    order: AnnotateOrder,
    mode: AnnotateMode,
    numbering: Numbering,
    sheets: &[SheetId],
) -> Result<Vec<Renumbered>> {
    for sheet in sheets {
        super::labels::check_sheet(project, *sheet)?;
    }
    let prefix_of = |id: Id| -> Option<String> {
        let instance = project.instance(id)?;
        let prefix = &project
            .library
            .component(instance.component)?
            .definition
            .designator_prefix;
        crate::library::designator_prefix_valid(prefix).then(|| prefix.clone())
    };
    let printed = project.printed_pages();
    let pages: BTreeMap<_, _> = printed
        .iter()
        .enumerate()
        .map(|(i, (s, p))| ((s.id, *p), i + 1))
        .collect();
    let mut visited: Vec<(Key, Id, String)> = project
        .instances
        .iter()
        .filter_map(|i| {
            let key = key(project, i.id, order, &pages);
            let in_scope = if sheets.is_empty() {
                true
            } else {
                key.0 != usize::MAX
                    && printed
                        .get(key.0 - 1)
                        .is_some_and(|(s, _)| sheets.contains(&s.id))
            };
            let prefix = prefix_of(i.id)?;
            let annotated = numbered(&i.designator);
            let target = match mode {
                AnnotateMode::All => true,
                AnnotateMode::Unannotated => !annotated,
            };
            (in_scope && target).then_some((key, i.id, prefix))
        })
        .collect();
    visited.sort_by_key(|v| v.0);
    let targets: BTreeSet<Id> = visited.iter().map(|(_, id, _)| *id).collect();
    // Every designator that stays, by lower case.
    let mut taken: BTreeSet<String> = project
        .instances
        .iter()
        .filter(|i| !targets.contains(&i.id))
        .map(|i| i.designator.to_lowercase())
        .collect();
    let mut next: BTreeMap<(String, usize), usize> = BTreeMap::new();
    let mut assigned: BTreeMap<Id, String> = BTreeMap::new();
    for ((page, ..), id, prefix) in &visited {
        let base = match numbering {
            Numbering::Sequential => 1,
            Numbering::SheetHundreds if *page != usize::MAX => page.saturating_mul(100) + 1,
            Numbering::SheetHundreds => 1,
        };
        let cursor = next
            .entry((prefix.to_ascii_uppercase(), base))
            .or_insert(base);
        let mut n = *cursor;
        // At most one number per designator is taken, so a free one lies
        // within `taken.len()` of the cursor.
        let limit = n.saturating_add(taken.len());
        let designator = loop {
            if n > limit {
                return Err(invalid("No free designator number is left."));
            }
            let candidate = format!("{prefix}{n}");
            if !taken.contains(&candidate.to_lowercase()) {
                break candidate;
            }
            n += 1;
        };
        *cursor = n + 1;
        taken.insert(designator.to_lowercase());
        assigned.insert(*id, designator);
    }
    let mut changes = Vec::new();
    for instance in &mut project.instances {
        if let Some(to) = assigned.remove(&instance.id)
            && to != instance.designator
        {
            changes.push(Renumbered {
                instance: instance.id,
                from: std::mem::replace(&mut instance.designator, to.clone()),
                to,
            });
        }
    }
    instances::check_designators(project)?;
    Ok(changes)
}

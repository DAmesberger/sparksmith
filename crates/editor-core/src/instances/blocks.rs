//! Blocks: component instances that belong together in the whole design,
//! on the sheets and on the board (docs/design/layout-blocks.md). This is
//! their membership, which is explicit: a part joins or leaves only by a
//! command, never by where it lies. What a block does on the board (its
//! border, its move and its lock) is `layout::blocks`.
//!
//! Each `block.*` command is one undo step.
use crate::diagnostics::{Error, Result};
use crate::geometry::Point;
use crate::ids::Id;
use crate::model::Project;
use crate::model::block::{Block, MAX_BLOCK_NAME};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "params", deny_unknown_fields)]
pub enum BlockCommand {
    /// Make a block of `parts`: parts that belong together. Named "Block 1"
    /// (the first free number) unless `name` says. A part is in one block
    /// at most. The outcome names it (`Outcome::BlockAdded`).
    #[serde(rename = "block.add")]
    Add {
        parts: Vec<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Rename a block.
    #[serde(rename = "block.rename")]
    Rename { id: Id, name: String },
    /// Take parts into a block (`add`) and let parts go (`remove`), as one
    /// undo step. Nothing moves, on a sheet or on the board. A block keeps
    /// at least one part.
    #[serde(rename = "block.members")]
    Members {
        id: Id,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add: Vec<Id>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remove: Vec<Id>,
    },
    /// Delete blocks as one undo step. Their parts stay where they are and
    /// as they are.
    #[serde(rename = "block.delete")]
    Delete { ids: Vec<Id> },
    /// Move a block's placed parts as one on the board, as one undo step.
    /// Mirrored onto the other side (`flip`, left to right) and turned by
    /// `turn_deg` counter-clockwise about the middle of its border, then
    /// slid by `by` (mm). The attached copper is carried as by
    /// `layout.move_parts`. A locked block, or a part of it that is locked
    /// or held by its dimensions, refuses the whole move and is named.
    /// Symbols stay.
    #[serde(rename = "block.board_move")]
    BoardMove {
        id: Id,
        #[serde(default)]
        by: Point,
        #[serde(default)]
        turn_deg: f64,
        #[serde(default)]
        flip: bool,
    },
    /// Lock or unlock blocks on the board, as one undo step. While a block
    /// is locked nothing moves it or one of its parts there by hand; the
    /// parts' own locks stay as they are. Symbols are not locked.
    #[serde(rename = "block.board_lock")]
    BoardLock { ids: Vec<Id>, locked: bool },
}

// A new command chooses its permission class here. Who may edit the board
// decides over what a block does there.
declare_entries!(BlockCommand in project {
    Add "block.add" Edit "Make a block" [since(10, 0), returns(block_added)],
    Rename "block.rename" Edit "Rename a block" [since(10, 0)],
    Members "block.members" Edit "Change a block's parts" [since(10, 0)],
    Delete "block.delete" Edit "Delete blocks" [since(10, 0)],
    BoardMove "block.board_move" Edit "Move a block on the board" [in layout, since(3, 1)],
    BoardLock "block.board_lock" Edit "Lock or unlock blocks on the board" [in layout, since(3, 1)],
});

/// The block `instance` is in.
pub fn block_of(project: &Project, instance: Id) -> Option<&Block> {
    project
        .blocks
        .iter()
        .find(|b| b.members.binary_search(&instance).is_ok())
}

/// The part by its designator, else by its ID.
fn part_name(project: &Project, id: Id) -> String {
    project
        .instance(id)
        .map_or_else(|| format!("part {id}"), |i| i.designator.clone())
}

pub(crate) fn find(project: &Project, id: Id) -> Result<&Block> {
    project
        .block(id)
        .ok_or_else(|| Error::NotFound(format!("unknown block {id}")))
}

fn edit(project: &mut Project, id: Id) -> Result<&mut Block> {
    project
        .blocks
        .iter_mut()
        .find(|b| b.id == id)
        .ok_or_else(|| Error::NotFound(format!("unknown block {id}")))
}

/// `name` as it is stored; refused when it is empty, too long, not
/// printable or another block's (`except` is the renamed one).
fn named(project: &Project, name: &str, except: Option<Id>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(Error::Invalid("A block needs a name".into()));
    }
    if name.len() > MAX_BLOCK_NAME {
        return Err(Error::Limit(format!(
            "A block's name is at most {MAX_BLOCK_NAME} bytes"
        )));
    }
    if let Some(other) = project
        .blocks
        .iter()
        .find(|b| Some(b.id) != except && b.name.to_lowercase() == name.to_lowercase())
    {
        return Err(Error::Conflict(format!(
            "There is already a block {}",
            other.name
        )));
    }
    Ok(name.to_owned())
}

/// "Block 1", or the first number no block has.
fn free_name(project: &Project) -> String {
    let taken: BTreeSet<String> = project
        .blocks
        .iter()
        .map(|b| b.name.to_lowercase())
        .collect();
    (1..=project.blocks.len().saturating_add(1))
        .map(|n| format!("Block {n}"))
        .find(|name| !taken.contains(&name.to_lowercase()))
        .unwrap_or_default()
}

/// The named parts in identity order: each once, in the project, and in no
/// block but `own`.
fn members(project: &Project, parts: &[Id], own: Option<Id>) -> Result<BTreeSet<Id>> {
    let mut out = BTreeSet::new();
    for &id in parts {
        let Some(part) = project.instance(id) else {
            return Err(Error::NotFound(format!(
                "No component instance has ID {id}"
            )));
        };
        if !out.insert(id) {
            return Err(Error::Invalid(format!(
                "{} is named twice",
                part.designator
            )));
        }
        if let Some(block) = block_of(project, id).filter(|b| Some(b.id) != own) {
            return Err(Error::Conflict(format!(
                "{} is in the block {}; a part is in one block",
                part.designator, block.name
            )));
        }
    }
    Ok(out)
}

/// `block.add`: a block of `parts`, named "Block n" unless named.
pub fn add(project: &mut Project, name: Option<&str>, parts: &[Id]) -> Result<Id> {
    let members = members(project, parts, None)?;
    if members.is_empty() {
        return Err(Error::Invalid("A block needs at least one part".into()));
    }
    let name = match name {
        Some(name) => named(project, name, None)?,
        None => free_name(project),
    };
    let id = project.allocate()?;
    project.blocks.push(Block {
        id,
        name,
        members: members.into_iter().collect(),
        locked: false,
    });
    Ok(id)
}

/// `block.rename`.
pub fn rename(project: &mut Project, id: Id, name: &str) -> Result<()> {
    find(project, id)?;
    let name = named(project, name, Some(id))?;
    edit(project, id)?.name = name;
    Ok(())
}

/// `block.members`: parts taken in and let go.
pub fn set_members(project: &mut Project, id: Id, take: &[Id], release: &[Id]) -> Result<()> {
    let block = find(project, id)?;
    if take.is_empty() && release.is_empty() {
        return Err(Error::Invalid("name at least one part".into()));
    }
    let mut next: BTreeSet<Id> = block.members.iter().copied().collect();
    for &part in release {
        if !next.remove(&part) {
            return Err(Error::Conflict(format!(
                "{} is not in the block {}",
                part_name(project, part),
                block.name
            )));
        }
    }
    next.extend(members(project, take, Some(id))?);
    if next.is_empty() {
        return Err(Error::Conflict(format!(
            "The block {} would have no part left; delete the block instead",
            block.name
        )));
    }
    edit(project, id)?.members = next.into_iter().collect();
    Ok(())
}

/// `block.delete`: the blocks go, their parts stay as they are.
pub fn delete(project: &mut Project, ids: &[Id]) -> Result<()> {
    named_once(project, ids)?;
    project.blocks.retain(|b| !ids.contains(&b.id));
    Ok(())
}

/// Every named block once and present, and at least one.
pub(crate) fn named_once(project: &Project, ids: &[Id]) -> Result<()> {
    if ids.is_empty() {
        return Err(Error::Invalid("name at least one block".into()));
    }
    let mut seen = BTreeSet::new();
    for &id in ids {
        let block = find(project, id)?;
        if !seen.insert(id) {
            return Err(Error::Invalid(format!("{} is named twice", block.name)));
        }
    }
    Ok(())
}

/// Members that no longer exist leave their block, and a block left
/// without a member goes. Run after every edit of the schematic; leaves the
/// blocks untouched when nothing is dropped.
pub fn follow_instances(project: &mut Project) {
    let gone = |id: &Id| project.instance(*id).is_none();
    if !project.blocks.iter().any(|b| b.members.iter().any(gone)) {
        return;
    }
    let live: BTreeSet<Id> = project.instances.iter().map(|i| i.id).collect();
    for block in project.blocks.iter_mut() {
        block.members.retain(|id| live.contains(id));
    }
    project.blocks.retain(|b| !b.members.is_empty());
}

/// The blocks' invariants: named once each, with members that exist, in
/// identity order, each in one block.
pub(crate) fn check(project: &Project) -> Result<()> {
    let invalid = |why: &str| Err(Error::Invalid(format!("block: {why}")));
    let mut names = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for block in project.blocks.iter() {
        let name = block.name.as_str();
        if name.trim() != name
            || name.is_empty()
            || name.len() > MAX_BLOCK_NAME
            || name.chars().any(char::is_control)
            || !names.insert(name.to_lowercase())
        {
            return invalid("block names are unique, trimmed and printable");
        }
        if block.members.is_empty() || !block.members.is_sorted_by(|a, b| a < b) {
            return invalid("a block lists its parts once each, in order, and has at least one");
        }
        for id in &block.members {
            if project.instance(*id).is_none() {
                return invalid("a block names a part that is not in the design");
            }
            if !seen.insert(*id) {
                return invalid("a part is in one block at most");
            }
        }
    }
    Ok(())
}

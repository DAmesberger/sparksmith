//! Layout blocks: parts that belong together and move as one
//! (docs/design/layout-blocks.md). Each `layout.block_*` command is one undo
//! step. A block owns no copper: what a move carries or leaves is the carry's
//! (`dimensions::carry`), as for any move of several parts.
//!
//! The border is derived: the convex hull of the placed members'
//! courtyards, without a margin. It is soft; nothing checks against it.
use super::board_edge::{BoardEdges, EdgeVerdict};
use super::outline::{self, BoardShapeEvaluator};
use super::{CopperRing, placed_pads};
use crate::diagnostics::{Error, Result};
use crate::geometry::{Point, point_valid};
use crate::ids::Id;
use crate::model::Project;
use crate::model::block::{Block, MAX_BLOCK_NAME};
use crate::model::{Layer, Placement};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Chord tolerance for a border's arcs and circles, mm: the courtyard
/// check's.
const CHORD_MM: f64 = 0.01;

/// A block as `layout.blocks` lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockRow {
    pub id: Id,
    pub name: String,
    pub locked: bool,
    pub parts: Vec<BlockPart>,
    /// The corners of its border on the board, counter-clockwise, mm.
    /// Empty while no part of it is on the board.
    pub border: Vec<Point>,
}

/// A part of a listed block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockPart {
    pub instance: Id,
    pub designator: String,
    /// The side it is on; absent while it is not on the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<Layer>,
}

/// The block `instance` is in.
pub fn block_of(project: &Project, instance: Id) -> Option<&Block> {
    project
        .blocks
        .iter()
        .find(|b| b.members.binary_search(&instance).is_ok())
}

/// A block's members on the board, in identity order.
pub fn placed_members(project: &Project, block: &Block) -> Vec<Id> {
    block
        .members
        .iter()
        .copied()
        .filter(|id| project.placement_of(*id).is_some())
        .collect()
}

/// Why a hand move of `instance` is refused by its block, if it is.
pub(crate) fn lock_refusal(project: &Project, instance: Id, verb: &str) -> Option<Error> {
    let block = block_of(project, instance).filter(|b| b.locked)?;
    Some(Error::Conflict(format!(
        "{} is in the locked block {}; unlock the block to {verb} it",
        outline::part_name(project, instance),
        block.name
    )))
}

/// Why the block cannot move, if it cannot: it is locked, none of its parts
/// is on the board, or one of them is locked or held by its dimensions.
pub fn move_refusal(
    project: &Project,
    block: &Block,
    verb: &str,
    evaluator: Option<&dyn BoardShapeEvaluator>,
) -> Option<Error> {
    if block.locked {
        return Some(Error::Conflict(format!(
            "The block {} is locked; unlock it to {verb} it",
            block.name
        )));
    }
    let members = placed_members(project, block);
    if members.is_empty() {
        return Some(Error::Conflict(format!(
            "No part of the block {} is on the board",
            block.name
        )));
    }
    members
        .into_iter()
        .find_map(|id| super::move_refusal(project, id, evaluator))
}

/// Every block's border with the block's id, in block order. A member
/// without a courtyard counts with its outline ([`super::part_envelope`]).
/// A block with no part on the board has none.
pub fn borders(project: &Project) -> Vec<(Id, Vec<Point>)> {
    if project.blocks.is_empty() {
        return Vec::new();
    }
    let mut of: HashMap<Id, Vec<Point>> = HashMap::new();
    for court in super::placed_courtyards(project) {
        of.entry(court.instance)
            .or_default()
            .extend(court.shape.flatten(CHORD_MM));
    }
    project
        .blocks
        .iter()
        .filter_map(|block| {
            let mut points = Vec::new();
            for id in &block.members {
                let Some(placement) = project.placement_of(*id) else {
                    continue;
                };
                match of.get(id) {
                    Some(court) => points.extend_from_slice(court),
                    None => points.extend(
                        super::part_envelope(project, placement)
                            .into_iter()
                            .flatten(),
                    ),
                }
            }
            let ring = hull(points);
            (!ring.is_empty()).then_some((block.id, ring))
        })
        .collect()
}

/// The convex hull of `points`, counter-clockwise from the lowest of the
/// leftmost (Andrew's monotone chain); points in line are left out.
fn hull(mut points: Vec<Point>) -> Vec<Point> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let turn = |a: Point, b: Point, c: Point| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let half = |points: &mut dyn Iterator<Item = Point>| -> Vec<Point> {
        let mut out: Vec<Point> = Vec::new();
        for p in points {
            while let [.., a, b] = out[..]
                && turn(a, b, p) <= 0.0
            {
                out.pop();
            }
            out.push(p);
        }
        // Its last point is the other half's first.
        out.pop();
        out
    };
    let mut ring = half(&mut points.iter().copied());
    ring.extend(half(&mut points.iter().rev().copied()));
    ring
}

/// The rows of `layout.blocks`.
pub fn rows(project: &Project) -> Vec<BlockRow> {
    let mut borders: BTreeMap<Id, Vec<Point>> = borders(project).into_iter().collect();
    project
        .blocks
        .iter()
        .map(|b| BlockRow {
            id: b.id,
            name: b.name.clone(),
            locked: b.locked,
            parts: b
                .members
                .iter()
                .map(|&instance| BlockPart {
                    instance,
                    designator: outline::part_name(project, instance),
                    layer: project.placement_of(instance).map(|p| p.layer),
                })
                .collect(),
            border: borders.remove(&b.id).unwrap_or_default(),
        })
        .collect()
}

fn find(project: &Project, id: Id) -> Result<&Block> {
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

/// `layout.block_add`: a block of `parts`, named "Block n" unless named.
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

/// `layout.block_rename`.
pub fn rename(project: &mut Project, id: Id, name: &str) -> Result<()> {
    find(project, id)?;
    let name = named(project, name, Some(id))?;
    edit(project, id)?.name = name;
    Ok(())
}

/// `layout.block_members`: parts taken in and let go.
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
                outline::part_name(project, part),
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

/// `layout.block_lock`.
pub fn lock(project: &mut Project, id: Id, locked: bool) -> Result<()> {
    edit(project, id)?.locked = locked;
    Ok(())
}

/// `layout.block_delete`: the block goes, its parts stay as they are.
pub fn delete(project: &mut Project, id: Id) -> Result<()> {
    find(project, id)?;
    project.blocks.retain(|b| b.id != id);
    Ok(())
}

/// What `layout.block_move` does to a block's parts, all at once: mirror
/// them onto the other side (`flip`, left to right), turn them by
/// `turn_deg` counter-clockwise, both about the middle of the border's
/// bounding box, then slide them by `by`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockMove {
    pub by: Point,
    pub turn_deg: f64,
    pub flip: bool,
}

impl BlockMove {
    /// The command the block's move is.
    pub fn command(self, id: Id) -> super::LayoutCommand {
        super::LayoutCommand::MoveBlock {
            id,
            by: self.by,
            turn_deg: self.turn_deg,
            flip: self.flip,
        }
    }

    /// `p` moved, turning and mirroring about `middle`.
    fn pose(&self, p: &Placement, middle: Point) -> Placement {
        let mut out = p.clone();
        let mut at = [p.position[0] - middle[0], p.position[1] - middle[1]];
        if self.flip {
            at[0] = -at[0];
            // A mirror about the vertical is one about the part's own
            // horizontal (what the side's mirror is) and a half turn.
            out.rotation_deg = 180.0 - out.rotation_deg;
            out.layer = super::placed::opposite(p.layer);
        }
        // A quarter turn is exact, so four of them are where they began.
        let quarters = self.turn_deg / 90.0;
        let (sin, cos) = if quarters.fract() == 0.0 {
            match quarters.rem_euclid(4.0) {
                0.0 => (0.0, 1.0),
                1.0 => (1.0, 0.0),
                2.0 => (0.0, -1.0),
                _ => (-1.0, 0.0),
            }
        } else {
            self.turn_deg.to_radians().sin_cos()
        };
        out.position = [
            middle[0] + cos * at[0] - sin * at[1] + self.by[0],
            middle[1] + sin * at[0] + cos * at[1] + self.by[1],
        ];
        if self.flip || self.turn_deg != 0.0 {
            out.rotation_deg = (out.rotation_deg + self.turn_deg).rem_euclid(360.0);
        }
        out
    }
}

/// The placed members of block `id` as `how` moves them.
pub fn moved(project: &Project, id: Id, how: &BlockMove) -> Result<Vec<Placement>> {
    let block = find(project, id)?;
    let middle = middle(project, block);
    Ok(placed_members(project, block)
        .into_iter()
        .filter_map(|m| project.placement_of(m))
        .map(|p| how.pose(p, middle))
        .collect())
}

/// The middle of the bounding box of the block's border.
fn middle(project: &Project, block: &Block) -> Point {
    let ring = borders(project)
        .into_iter()
        .find(|(id, _)| *id == block.id)
        .map(|(_, ring)| ring)
        .unwrap_or_default();
    let mid = |k: usize| {
        let (lo, hi) = ring
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |r, p| {
                (r.0.min(p[k]), r.1.max(p[k]))
            });
        if lo <= hi { (lo + hi) / 2.0 } else { 0.0 }
    };
    [mid(0), mid(1)]
}

/// `layout.block_move`. A locked block, and a member locked or held by its
/// dimensions, refuse the whole move and are named. The engine carries the
/// members' copper at once and names the connections it opens.
pub fn move_block(
    project: &mut Project,
    id: Id,
    how: &BlockMove,
    evaluator: Option<&dyn BoardShapeEvaluator>,
) -> Result<()> {
    if !point_valid(how.by) || !how.turn_deg.is_finite() {
        return Err(Error::Invalid("the block's move is not finite".into()));
    }
    let block = find(project, id)?;
    let verb = match how {
        BlockMove { flip: true, .. } => "flip",
        BlockMove { turn_deg, .. } if *turn_deg != 0.0 => "rotate",
        _ => "move",
    };
    if let Some(error) = move_refusal(project, block, verb, evaluator) {
        return Err(error);
    }
    for next in moved(project, id, how)? {
        if !point_valid(next.position) {
            return Err(Error::Invalid(format!(
                "{}'s new pose is not finite",
                outline::part_name(project, next.instance)
            )));
        }
        if let Some(p) = project
            .placements
            .iter_mut()
            .find(|p| p.instance == next.instance)
        {
            *p = next;
        }
    }
    Ok(())
}

/// The parts of `moved` with a pad beyond the board's outline now (as
/// `board.outside_outline` judges it) that had none `before`, in one line
/// for the person to be told at once: "C3 and 1 more extend beyond the
/// board".
pub(crate) fn put_over_edge(before: &Project, after: &Project, moved: &[Id]) -> Option<String> {
    let over = |project: &Project| -> BTreeSet<Id> {
        let edges = BoardEdges::of(project);
        placed_pads(project)
            .iter()
            .filter(|p| moved.contains(&p.instance))
            .filter(|p| {
                matches!(
                    edges.verdict(&CopperRing::of_placed_pad(p), 0.0),
                    EdgeVerdict::Outside
                )
            })
            .map(|p| p.instance)
            .collect()
    };
    let now = over(after);
    if now.is_empty() {
        return None;
    }
    let was = over(before);
    let new: Vec<Id> = now.difference(&was).copied().collect();
    let first = outline::part_name(after, *new.first()?);
    Some(match new.len() {
        1 => format!("{first} extends beyond the board"),
        n => format!("{first} and {} more extend beyond the board", n - 1),
    })
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
    let invalid = |why: &str| Err(Error::Invalid(format!("layout.block: {why}")));
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

#[cfg(test)]
mod tests;

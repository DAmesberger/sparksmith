//! Layout blocks on the canvas, and their card: the derived border and its
//! name, the level of detail a block is drawn at, the drag of a whole block
//! (one `layout.block_move`) and Make block.
//!
//! The level of detail only changes what is drawn (and so what a click
//! takes): a border, its parts, or its pads and copper too. Parts outside
//! blocks are always drawn, and so is what is selected. While a tool is
//! armed everything is drawn.
use super::group::{Group, GroupDrag};
use super::hold_mark::{self, Hold};
use super::{BoardView, Field, PAD_NUMBER_CAP, PcbEditor, Tool, body_polygon};
use crate::kit::severity::Level;
use crate::kit::{self, ItemRow, RowAction};
use crate::refusal::{self, Origin};
use egui::{Align2, Pos2, Shape, Stroke, Ui};
use sparksmith_core::geometry::{polygon_contains, segment_distance};
use sparksmith_core::layout::LayoutCommand;
use sparksmith_core::layout::blocks::{self, BlockMove};
use sparksmith_core::model::Block;
use sparksmith_core::{Id, Layer, Operation, Point, Project};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{self, Button, type_face};
use std::collections::{BTreeMap, BTreeSet};

/// Why a locked block's Rotate 90° and Flip are disabled.
const LOCKED: &str = "Unlock the block to move it";
/// The border's line: a dash, a gap, a dot, a gap, in points.
const DASHES: [f32; 2] = [8.0, 2.0];
const GAPS: [f32; 2] = [3.0, 3.0];
/// A block drawn as its border alone is tinted inside this much.
const BODY_SHADE: f32 = 0.12;

/// How much of a block is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Detail {
    Border,
    Parts,
    Copper,
}

/// The level of detail the person chose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Choice {
    /// By the zoom: the border until a part is large enough for its
    /// designator, the parts until every part is, then everything.
    #[default]
    Auto,
    Fixed(Detail),
}

impl Choice {
    const ALL: [Self; 4] = [
        Self::Auto,
        Self::Fixed(Detail::Border),
        Self::Fixed(Detail::Parts),
        Self::Fixed(Detail::Copper),
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto by zoom",
            Self::Fixed(Detail::Border) => "Border",
            Self::Fixed(Detail::Parts) => "Parts",
            Self::Fixed(Detail::Copper) => "Pads and copper",
        }
    }

    fn options() -> [(Self, String); 4] {
        Self::ALL.map(|c| (c, c.label().to_owned()))
    }

    pub(super) fn next(self) -> Self {
        let at = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Self::ALL.get(at + 1).copied().unwrap_or(Self::Auto)
    }
}

/// What the canvas leaves out of the blocks drawn with less than all.
#[derive(Debug, Default)]
struct Hidden {
    /// Parts of which nothing is drawn.
    parts: BTreeSet<Id>,
    /// Parts whose pads are not drawn.
    pads: BTreeSet<Id>,
    tracks: BTreeSet<Id>,
    vias: BTreeSet<Id>,
}

/// The editor's side of the blocks.
#[derive(Debug, Default)]
pub(super) struct Blocks {
    /// The level of detail chosen; the canvas's, for every project.
    pub choice: Choice,
    /// Every placed block's border, per revision.
    borders: Vec<(Id, Vec<Point>)>,
    /// How much of each block this frame draws, and what that hides.
    detail: BTreeMap<Id, Detail>,
    hidden: Hidden,
    hidden_for: Option<(u64, BTreeMap<Id, Detail>)>,
    /// The card's Name row: the block, its name last seen and the text
    /// being edited.
    name: (Option<Id>, String, String),
    /// Every part of the card's list instead of the first eight.
    all: bool,
}

impl Blocks {
    /// What outlives the document.
    pub fn kept(&self) -> Self {
        Self {
            choice: self.choice,
            ..Default::default()
        }
    }

    /// The document changed.
    pub fn cache(&mut self, project: &Project) {
        self.borders = blocks::borders(project);
    }

    fn border(&self, id: Id) -> Option<&[Point]> {
        self.borders
            .iter()
            .find(|(of, _)| *of == id)
            .map(|(_, ring)| ring.as_slice())
    }
}

/// "5 parts".
fn parts(count: usize) -> String {
    match count {
        1 => "1 part".to_owned(),
        n => format!("{n} parts"),
    }
}

impl PcbEditor {
    /// Select a block.
    pub fn select_block(&mut self, id: Id) {
        self.set_selection(None, None, None);
        self.block = Some(id);
    }

    /// Select a block and bring it into view (its Browser row).
    pub fn show_block(&mut self, id: Id, project: &Project) {
        self.select_block(id);
        let ring = blocks::borders(project)
            .into_iter()
            .find(|(of, _)| *of == id)
            .map(|(_, ring)| ring)
            .unwrap_or_default();
        self.table.reveal = centre(&ring);
    }

    pub fn selected_block(&self) -> Option<Id> {
        self.block
    }

    pub(super) fn block_selected(&self, id: Id) -> bool {
        self.block == Some(id) || self.group.blocks.contains(&id)
    }

    /// The parts a drag of the group moves: its own and its blocks'.
    pub(super) fn group_parts(&self, project: &Project) -> BTreeSet<Id> {
        let mut out = self.group.parts.clone();
        for block in self.group.blocks.iter().filter_map(|id| project.block(*id)) {
            out.extend(blocks::placed_members(project, block));
        }
        out
    }

    /// A block of the selected parts; `Err` says why not.
    pub fn make_block(&self) -> Result<Operation, String> {
        if self.view_3d {
            return Err("Switch to the Layout view to use it".into());
        }
        let parts: Vec<Id> = self.selected().parts.into_iter().collect();
        if parts.is_empty() {
            return Err("Select the block's parts".into());
        }
        Ok(Operation::Layout(LayoutCommand::AddBlock {
            parts,
            name: None,
        }))
    }

    /// The selected parts taken into the one selected block.
    pub(super) fn add_to_block(&self) -> Option<(Id, Operation)> {
        let mut blocks = self.group.blocks.iter();
        let (Some(&id), None) = (blocks.next(), blocks.next()) else {
            return None;
        };
        let add: Vec<Id> = self.group.parts.iter().copied().collect();
        (!add.is_empty()).then_some((
            id,
            Operation::Layout(LayoutCommand::SetBlockMembers {
                id,
                add,
                remove: Vec::new(),
            }),
        ))
    }

    /// Start dragging block `id` from board point `at`, as a group of its
    /// parts; `Err` says why it does not move.
    pub(super) fn start_block_drag(
        &mut self,
        project: &Project,
        id: Id,
        at: Point,
    ) -> Result<(), String> {
        let Some(block) = project.block(id) else {
            return Ok(());
        };
        if let Some(error) = blocks::move_refusal(project, block, "move", self.shape.evaluator) {
            return Err(format!(
                "Can't move the block {}: {}",
                block.name,
                refusal::shown_error(&error)
            ));
        }
        self.group_drag = Some(GroupDrag {
            parts: blocks::placed_members(project, block)
                .into_iter()
                .filter_map(|id| project.placement_of(id).cloned())
                .collect(),
            block: Some(id),
            grab: at,
            by: [0.0, 0.0],
        });
        Ok(())
    }

    /// How far a drag has moved the block's border: a drag of all its parts.
    fn border_delta(&self, project: &Project, block: &Block) -> Point {
        match &self.group_drag {
            Some(drag)
                if block.members.iter().all(|id| {
                    project.placement_of(*id).is_none()
                        || drag.parts.iter().any(|p| p.instance == *id)
                }) =>
            {
                drag.by
            }
            _ => [0.0, 0.0],
        }
    }

    /// How much of each block this frame draws, at `view`'s zoom.
    pub(super) fn block_details(
        &mut self,
        painter: &egui::Painter,
        view: BoardView,
        project: &Project,
        revision: u64,
    ) {
        let detail: BTreeMap<Id, Detail> = if self.tool == Tool::Select {
            project
                .blocks
                .iter()
                .map(|b| {
                    let detail = match self.blocks.choice {
                        Choice::Auto => readable(painter, view, project, b),
                        Choice::Fixed(detail) => detail,
                    };
                    (b.id, detail)
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let key = Some((revision, detail));
        if self.blocks.hidden_for != key {
            let hidden = match &key {
                Some((_, detail)) => self.hidden(project, detail),
                None => Hidden::default(),
            };
            self.blocks.hidden = hidden;
            self.blocks.hidden_for = key;
        }
        if let Some((_, detail)) = &self.blocks.hidden_for
            && self.blocks.detail != *detail
        {
            self.blocks.detail = detail.clone();
        }
    }

    /// What blocks drawn at `detail` leave out. The copper left out is what
    /// lies inside the border on a net of a part of the block: other nets'
    /// copper there is not the block's and stays.
    fn hidden(&self, project: &Project, detail: &BTreeMap<Id, Detail>) -> Hidden {
        let mut out = Hidden::default();
        for block in project.blocks.iter() {
            let detail = detail.get(&block.id).copied().unwrap_or(Detail::Copper);
            let Some(ring) = self.blocks.border(block.id) else {
                continue;
            };
            if detail == Detail::Copper {
                continue;
            }
            out.pads.extend(block.members.iter().copied());
            if detail == Detail::Border {
                out.parts.extend(block.members.iter().copied());
            }
            let nets: BTreeSet<Id> = self
                .board_pads
                .iter()
                .filter(|p| block.members.binary_search(&p.placed.instance).is_ok())
                .filter_map(|p| p.placed.net)
                .collect();
            let inside = |p: Point| polygon_contains(ring, p);
            out.tracks.extend(
                project
                    .tracks
                    .iter()
                    .filter(|t| nets.contains(&t.net) && inside(t.start) && inside(t.end))
                    .map(|t| t.id),
            );
            out.vias.extend(
                project
                    .vias
                    .iter()
                    .filter(|v| nets.contains(&v.net) && inside(v.position))
                    .map(|v| v.id),
            );
        }
        out
    }

    /// Whether nothing of the part is drawn: its block is its border alone.
    pub(super) fn part_hidden(&self, instance: Id) -> bool {
        self.blocks.hidden.parts.contains(&instance) && !self.is_selected(instance)
    }

    /// Whether the part's pads are not drawn.
    pub(super) fn pads_hidden(&self, instance: Id) -> bool {
        self.blocks.hidden.pads.contains(&instance) && !self.is_selected(instance)
    }

    pub(super) fn track_hidden(&self, id: Id) -> bool {
        self.blocks.hidden.tracks.contains(&id)
            && self.track != Some(id)
            && !self.group.tracks.contains(&id)
    }

    pub(super) fn via_hidden(&self, id: Id) -> bool {
        self.blocks.hidden.vias.contains(&id)
            && self.via != Some(id)
            && !self.group.vias.contains(&id)
    }

    /// Whether the block is drawn as its border alone.
    pub(super) fn border_only(&self, id: Id) -> bool {
        self.blocks.detail.get(&id) == Some(&Detail::Border)
    }

    /// The block whose border is under `at`, or which is drawn as its border
    /// alone and holds `at`; the last made first.
    pub(super) fn pick_block(&self, at: Point, tolerance: f64) -> Option<Id> {
        self.blocks
            .borders
            .iter()
            .rev()
            .find(|(id, ring)| {
                (self.border_only(*id) && polygon_contains(ring, at))
                    || edges(ring).any(|(a, b)| segment_distance(at, a, b) <= tolerance)
            })
            .map(|(id, _)| *id)
    }

    /// The blocks the rectangle or ⌘A takes as blocks: those drawn as their
    /// border alone, and all of them while the filter leaves parts out;
    /// elsewhere their parts are taken. `takes` says what the rectangle does.
    pub(super) fn blocks_taken(&self, takes: impl Fn(&[Point]) -> bool) -> BTreeSet<Id> {
        if !self.filter.blocks {
            return BTreeSet::new();
        }
        self.blocks
            .borders
            .iter()
            .filter(|(id, _)| self.border_only(*id) || !self.filter.parts)
            .filter(|(_, ring)| takes(ring))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Every block's border and name over the board.
    pub(super) fn paint_blocks(&self, painter: &egui::Painter, view: BoardView, project: &Project) {
        let t = Tokens::get();
        for (id, ring) in &self.blocks.borders {
            let Some(block) = project.block(*id) else {
                continue;
            };
            let by = self.border_delta(project, block);
            let mut points: Vec<Pos2> = ring
                .iter()
                .map(|p| view.screen([p[0] + by[0], p[1] + by[1]]))
                .collect();
            let Some(first) = points.first().copied() else {
                continue;
            };
            let selected = self.block_selected(*id);
            let color = if selected {
                t.body_selected
            } else {
                t.text_dim
            };
            let border = self.border_only(*id);
            if border {
                painter.add(Shape::convex_polygon(
                    points.clone(),
                    color.gamma_multiply(BODY_SHADE),
                    Stroke::NONE,
                ));
            }
            let bounds = egui::Rect::from_points(&points);
            points.push(first);
            painter.extend(Shape::dashed_line_with_offset(
                &points,
                Stroke::new(if selected { 2.0 } else { 1.5 }, color),
                &DASHES,
                &GAPS,
                0.0,
            ));
            let font = type_face::semibold(t.font_label);
            let name = painter.layout_no_wrap(block.name.clone(), font, color);
            let mark = if block.locked {
                hold_mark::GLYPH + t.space_4
            } else {
                0.0
            };
            let width = name.size().x + mark;
            // Under the border, in its middle where nothing else is drawn.
            let at = egui::pos2(
                bounds.center().x - width / 2.0,
                if border {
                    bounds.center().y - name.size().y
                } else {
                    bounds.bottom() + t.space_2
                },
            );
            if block.locked {
                let glyph = egui::pos2(at.x + hold_mark::GLYPH / 2.0, at.y + name.size().y / 2.0);
                hold_mark::paint_glyph(painter, glyph, hold_mark::GLYPH, Hold::Locked);
            }
            let below = at.y + name.size().y;
            painter.galley(egui::pos2(at.x + mark, at.y), name, color);
            if border {
                painter.text(
                    egui::pos2(bounds.center().x, below),
                    Align2::CENTER_TOP,
                    parts(block.members.len()),
                    egui::FontId::proportional(t.font_small),
                    color,
                );
            }
        }
    }

    /// The view popover's row: how much of the blocks is drawn.
    pub(super) fn block_detail_row(&mut self, ui: &mut Ui) {
        let key = crate::keys::shown(ui.ctx(), "pcb.block-detail");
        kit::keyed(ui, key, |ui| {
            kit::choice_row(ui, "Blocks", &mut self.blocks.choice, &Choice::options())
        });
    }

    /// BLOCK: its name, its lock and its parts.
    pub(super) fn block_section(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        block: &Block,
    ) -> Option<Operation> {
        if (self.blocks.name.0, self.blocks.name.1.as_str())
            != (Some(block.id), block.name.as_str())
        {
            self.blocks.name = (Some(block.id), block.name.clone(), block.name.clone());
        }
        let mut action = None;
        kit::section(ui, "pcb-block", "Block", true, None, |ui| {
            let error = self.errors.0.get(&Field::BlockName).cloned();
            let name =
                kit::checked_text_row(ui, "Name", &mut self.blocks.name.2, "", error.is_some());
            if let Some(error) = &error {
                kit::field_error(ui, &name, error, true);
            }
            if name.changed() {
                self.errors.0.remove(&Field::BlockName);
            }
            if name.lost_focus() && self.blocks.name.2.trim() != block.name {
                self.errors.0.remove(&Field::BlockName);
                action = Some((
                    LayoutCommand::RenameBlock {
                        id: block.id,
                        name: self.blocks.name.2.clone(),
                    },
                    Origin::Field(Field::BlockName.id()),
                ));
            }
            let mut locked = block.locked;
            let lock = ui.scope(|ui| kit::check_row(ui, "Locked", &mut locked));
            crate::app::help_sheet::region(ui, lock.response.rect, "entry/layout.block_lock", 3);
            if lock.inner {
                action = Some((
                    LayoutCommand::SetBlockLock {
                        id: block.id,
                        locked,
                    },
                    Origin::Command,
                ));
            }
        });
        let count = block.members.len();
        let mut select = None;
        kit::section(ui, "pcb-block-parts", "Parts", true, Some(count), |ui| {
            let sides: Vec<&str> = block
                .members
                .iter()
                .map(|id| match project.placement_of(*id).map(|p| p.layer) {
                    Some(Layer::Top) => "Top",
                    Some(Layer::Bottom) => "Bottom",
                    None => "",
                })
                .collect();
            let column = kit::value_column(ui, &sides);
            let max = if self.blocks.all {
                count
            } else {
                kit::LIST_MAX_ROWS
            };
            let list = kit::list_frame(ui, count, max, |ui, index| {
                let (Some(&id), Some(side)) = (block.members.get(index), sides.get(index)) else {
                    return;
                };
                let designator = project.instance(id).map_or("", |i| i.designator.as_str());
                let take = format!("Take {designator} out of the block");
                let name = [designator];
                let mut row = ItemRow::new(&name, column);
                row.selectable = true;
                row.badge = match project.placement_of(id) {
                    None => Some((Level::ToDo, "Not on the board")),
                    Some(p) if p.locked => Some((Level::Info, "Locked")),
                    Some(_) => None,
                };
                // The last part leaves with the block.
                row.action = (count > 1).then_some(RowAction {
                    icon: "close",
                    label: &take,
                    tip: "",
                });
                let (response, ()) = kit::item_row(ui, &row, |ui| {
                    ui.label(*side);
                });
                if response.action {
                    action = Some((
                        LayoutCommand::SetBlockMembers {
                            id: block.id,
                            add: Vec::new(),
                            remove: vec![id],
                        },
                        Origin::Command,
                    ));
                } else if response.clicked {
                    select = Some(id);
                }
            });
            if list.show_all {
                self.blocks.all = true;
            }
        });
        if let Some(id) = select {
            self.set_selection(Some(id), None, None);
        }
        let (command, origin) = action?;
        refusal::mark(ui.ctx(), origin);
        Some(Operation::Layout(command))
    }

    /// The card's footer: Delete, Rotate 90° and Flip of the whole block.
    pub(super) fn block_footer(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        block: &Block,
    ) -> Option<Operation> {
        let mut action = None;
        if ui_kit::button(ui, "Delete", Button::Normal)
            .on_hover_text("Delete the block; its parts stay as they are")
            .clicked()
        {
            action = Some(LayoutCommand::DeleteBlock { id: block.id });
        }
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            for (label, tip, how) in [
                ("Rotate 90°", "Turn it about its middle", turn()),
                ("Flip", "Mirror it onto the other side", flip()),
            ] {
                if ui_kit::button(ui, label, Button::Enabled(!block.locked))
                    .on_hover_text(tip)
                    .on_disabled_hover_text(LOCKED)
                    .clicked()
                    && !block.locked
                {
                    name_move(ui.ctx(), project, block.id, &how);
                    action = Some(how.command(block.id));
                }
            }
        });
        action.map(Operation::Layout)
    }

    /// The Placement card's row: the block the part is in, which it selects.
    pub(super) fn part_block_row(&mut self, ui: &mut Ui, project: &Project, instance: Id) {
        let Some(block) = blocks::block_of(project, instance) else {
            return;
        };
        let clicked = kit::row(ui, "Block", |ui| {
            ui_kit::button(ui, block.name.as_str(), Button::Small)
                .on_hover_text("Select this block")
                .clicked()
        });
        if clicked {
            self.select_block(block.id);
        }
    }
}

/// A refusal's toast names the action and the block ("Can't flip the block
/// Converter: …").
pub(super) fn name_move(ctx: &egui::Context, project: &Project, id: Id, how: &BlockMove) {
    let Some(block) = project.block(id) else {
        return;
    };
    let verb = if how.flip { "flip" } else { "rotate" };
    refusal::name_action(ctx, format!("{verb} the block {}", block.name));
}

pub(super) fn turn() -> BlockMove {
    BlockMove {
        turn_deg: 90.0,
        ..Default::default()
    }
}

pub(super) fn flip() -> BlockMove {
    BlockMove {
        flip: true,
        ..Default::default()
    }
}

/// How much of a block reads at this zoom: its parts once a designator fits
/// on one, its pads and copper once every part's does.
fn readable(painter: &egui::Painter, view: BoardView, project: &Project, block: &Block) -> Detail {
    let t = Tokens::get();
    let (mut any, mut all) = (false, true);
    for id in &block.members {
        let Some((instance, polygon)) = project.instance(*id).and_then(|i| {
            let placement = project.placement_of(*id)?;
            Some((i, body_polygon(project, placement)?))
        }) else {
            continue;
        };
        let body = egui::Rect::from_points(&polygon.map(|p| view.screen(p)));
        let font = type_face::semibold(t.font_label);
        let text = painter.layout_no_wrap(instance.designator.clone(), font, t.text);
        let fits = text.size().x <= body.width() && t.font_label * PAD_NUMBER_CAP <= body.height();
        any |= fits;
        all &= fits;
    }
    match (any, all) {
        (_, true) => Detail::Copper,
        (true, false) => Detail::Parts,
        (false, false) => Detail::Border,
    }
}

fn edges(ring: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    let next = ring.iter().skip(1).chain(ring.first());
    ring.iter().copied().zip(next.copied())
}

/// The middle of the ring's bounding box.
fn centre(ring: &[Point]) -> Option<Point> {
    let first = *ring.first()?;
    let (lo, hi) = ring.iter().fold((first, first), |(lo, hi), p| {
        (
            [lo[0].min(p[0]), lo[1].min(p[1])],
            [hi[0].max(p[0]), hi[1].max(p[1])],
        )
    });
    Some([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0])
}

impl Group {
    /// Whether it holds nothing a Flip of its parts would leave half done:
    /// blocks flip as blocks, one at a time.
    pub(super) fn flips_by_part(&self) -> bool {
        self.blocks.is_empty()
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;

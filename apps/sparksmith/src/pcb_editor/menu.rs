//! The Layout canvas's right-click menus (`crate::context_menu`): for a
//! part, a track, a via, a fill and the empty board. A right-click selects
//! what a click there would select first, unless it is already selected.
//! Every row sends the operation its card button or key sends, or runs a
//! registry command.
use super::edge_card::{dimensions_of, nearest_edge, spec_at};
use super::{PcbEditor, Tool, blocks, opposite};
use crate::context_menu::{Entry, Rows};
use sparksmith_core::layout::LayoutCommand;
use sparksmith_core::layout::dimensions::{self, DimensionCommand};
use sparksmith_core::layout::fill::{FillCommand, RefillTarget};
use sparksmith_core::updates::{Change, InstanceChange};
use sparksmith_core::{Engine, Id, Operation};

/// What a click at a board point picks, in the order a click selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    /// A pad: its part, and the pad's net.
    Pad(Id, Option<Id>),
    Tie(Id, sparksmith_core::TieId),
    Hole(super::BoardFeatureId),
    Via(Id, Id),
    Track(Id, Id),
    Part(Id),
    /// A block's border, or a block drawn as its border alone.
    Block(Id),
    Fill(Id, Option<Id>),
    Nothing,
}

/// What a Layout menu is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Part(Id),
    Track(Id),
    Via(Id),
    Fill(Id),
    Block(Id),
    /// The group selection a right-click landed on.
    Group,
    Board,
}

/// A Layout menu row's own action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Act {
    Rotate(Id),
    Flip(Id),
    Lock(Id, bool),
    /// Dimension the part to its nearest named edge, in place.
    Constrain(Id),
    /// Free a part one dimension holds (delete it).
    Free(Id),
    Remove(Id),
    DeleteTrack(Id),
    DeleteVia(Id),
    SelectNet(Id),
    Refill(Id),
    DeleteFill(Id),
    /// Delete the group's tracks, vias and fills.
    DeleteGroup,
    /// Lock or unlock the whole group.
    LockGroup(bool),
    /// Toggle whether the part is populated in the active variant.
    Population(Id),
    /// The same for every part of the group, in one step.
    GroupPopulation,
    RotateBlock(Id),
    FlipBlock(Id),
    LockBlock(Id, bool),
    DeleteBlock(Id),
    /// Select the block a part is in.
    SelectBlock(Id),
    /// Let a part go from its block.
    Release(Id, Id),
    /// Take the group's parts into its block.
    AddToBlock,
}

impl PcbEditor {
    /// Select what a click on `hit` selects, unless it is selected; the
    /// menu's target.
    pub(super) fn menu_select(&mut self, hit: Hit) -> Target {
        // On an object of the group: the group's menu, the group kept.
        if super::group::Pick::of(hit).is_some_and(|p| self.group.contains(p)) {
            return Target::Group;
        }
        match hit {
            Hit::Tie(instance, _) => {
                self.select_hit(hit);
                Target::Part(instance)
            }
            Hit::Pad(instance, _) | Hit::Part(instance) => {
                if self.instance != Some(instance) {
                    self.select_hit(hit);
                }
                Target::Part(instance)
            }
            Hit::Via(id, _) => {
                if self.via != Some(id) {
                    self.select_hit(hit);
                }
                Target::Via(id)
            }
            Hit::Track(id, _) => {
                if self.track != Some(id) {
                    self.select_hit(hit);
                }
                Target::Track(id)
            }
            Hit::Fill(id, _) => {
                if self.fill != Some(id) {
                    self.select_hit(hit);
                }
                Target::Fill(id)
            }
            Hit::Block(id) => {
                if self.block != Some(id) {
                    self.select_hit(hit);
                }
                Target::Block(id)
            }
            Hit::Hole(_) => {
                self.select_hit(hit);
                Target::Board
            }
            Hit::Nothing => Target::Board,
        }
    }

    /// The net rows of a track or via on `net`.
    fn net_rows(&self, ctx: &egui::Context, net: Id) -> Rows<Act> {
        let lit = self.highlighted == Some(net);
        vec![
            None,
            Some(Entry::act(
                "Select net",
                "select",
                Act::SelectNet(net),
                true,
            )),
            Some(Entry::command(
                ctx,
                "pcb.highlight-net",
                if lit {
                    "Clear net highlight"
                } else {
                    "Highlight net"
                },
                self.view_toggle_state(crate::commands::PcbView::HighlightNet)
                    .is_ok(),
            )),
        ]
    }

    /// The menu's rows for `target`; none once it is gone.
    pub(super) fn menu_rows(
        &mut self,
        ctx: &egui::Context,
        engine: &Engine,
        target: Target,
    ) -> Rows<Act> {
        let project = engine.project();
        let reviewing = self.update.is_open();
        match target {
            Target::Part(instance) => {
                let Some(placement) = project.placements.iter().find(|p| p.instance == instance)
                else {
                    return Vec::new();
                };
                let free = !placement.locked && !reviewing;
                let held = dimensions_of(project, instance).len();
                let edge = match held {
                    1 => Entry::act("Free part", "close", Act::Free(instance), !reviewing),
                    _ => Entry::act(
                        "Dimension to edge",
                        "line",
                        Act::Constrain(instance),
                        held == 0
                            && free
                            && nearest_edge(&project.board(), placement.position).is_some(),
                    ),
                };
                let block = sparksmith_core::layout::blocks::block_of(project, instance);
                let mut rows = vec![
                    Some(
                        Entry::act("Rotate 90°", "spin", Act::Rotate(instance), free)
                            .key_of(ctx, "pcb.rotate"),
                    ),
                    Some(Entry::act(
                        "Flip side",
                        "pattern_flip",
                        Act::Flip(instance),
                        free,
                    )),
                    None,
                    Some(Entry::act(
                        if placement.locked { "Unlock" } else { "Lock" },
                        "ground",
                        Act::Lock(instance, !placement.locked),
                        !reviewing,
                    )),
                    Some(edge),
                    crate::variants::menu_entry(
                        project,
                        &[instance],
                        Act::Population(instance),
                        !reviewing,
                    ),
                ];
                if let Some(block) = block {
                    rows.extend([
                        None,
                        Some(Entry::act(
                            format!("Select block {}", block.name),
                            "select",
                            Act::SelectBlock(block.id),
                            true,
                        )),
                        // The last part leaves with the block.
                        Some(Entry::act(
                            "Take out of the block",
                            "close",
                            Act::Release(block.id, instance),
                            block.members.len() > 1 && !reviewing,
                        )),
                    ]);
                }
                rows.extend([
                    None,
                    Some(Entry::act(
                        "Delete…",
                        "delete",
                        Act::Remove(instance),
                        !reviewing,
                    )),
                ]);
                rows
            }
            Target::Track(id) => {
                let Some(track) = project.tracks.iter().find(|t| t.id == id) else {
                    return Vec::new();
                };
                let mut rows = vec![Some(
                    Entry::act(
                        "Delete",
                        "delete",
                        Act::DeleteTrack(id),
                        !track.locked && !reviewing,
                    )
                    .key_of(ctx, "pcb.delete"),
                )];
                rows.extend(self.net_rows(ctx, track.net));
                rows
            }
            Target::Via(id) => {
                let Some(via) = project.vias.iter().find(|v| v.id == id) else {
                    return Vec::new();
                };
                let mut rows = vec![Some(
                    Entry::act(
                        "Delete",
                        "delete",
                        Act::DeleteVia(id),
                        !via.locked && !reviewing,
                    )
                    .key_of(ctx, "pcb.delete"),
                )];
                rows.extend(self.net_rows(ctx, via.net));
                rows
            }
            Target::Fill(id) => {
                let Some(fill) = project.fill(id) else {
                    return Vec::new();
                };
                let free = !fill.locked && !reviewing;
                vec![
                    Some(Entry::act("Refill", "update", Act::Refill(id), free)),
                    None,
                    Some(
                        Entry::act("Delete", "delete", Act::DeleteFill(id), free)
                            .key_of(ctx, "pcb.delete"),
                    ),
                ]
            }
            Target::Block(id) => {
                let Some(block) = project.block(id) else {
                    return Vec::new();
                };
                let free = !block.locked && !reviewing;
                vec![
                    Some(
                        Entry::act("Rotate 90°", "spin", Act::RotateBlock(id), free)
                            .key_of(ctx, "pcb.rotate"),
                    ),
                    Some(Entry::act(
                        "Flip side",
                        "pattern_flip",
                        Act::FlipBlock(id),
                        free,
                    )),
                    None,
                    Some(Entry::act(
                        if block.locked { "Unlock" } else { "Lock" },
                        "ground",
                        Act::LockBlock(id, !block.locked),
                        !reviewing,
                    )),
                    None,
                    Some(
                        Entry::act("Delete", "delete", Act::DeleteBlock(id), !reviewing)
                            .key_of(ctx, "pcb.delete"),
                    ),
                ]
            }
            Target::Group => {
                let parts: Vec<Id> = self.group.parts.iter().copied().collect();
                let count = parts.len();
                let locked = self.group.all_locked(project);
                let arrange = |id: &'static str, label: &str, fewest: usize| {
                    Some(Entry::command(
                        ctx,
                        id,
                        label,
                        count >= fewest && !reviewing,
                    ))
                };
                let mut rows = vec![
                    arrange("pcb.align-left", "Align left edges", 2),
                    arrange("pcb.align-right", "Align right edges", 2),
                    arrange("pcb.align-top", "Align top edges", 2),
                    arrange("pcb.align-bottom", "Align bottom edges", 2),
                    arrange("pcb.align-centres-x", "Align horizontal centres", 2),
                    arrange("pcb.align-centres-y", "Align vertical centres", 2),
                    arrange("pcb.distribute-x", "Distribute horizontally", 3),
                    arrange("pcb.distribute-y", "Distribute vertically", 3),
                    None,
                    Some(Entry::act(
                        if locked { "Unlock" } else { "Lock" },
                        "ground",
                        Act::LockGroup(!locked),
                        !reviewing,
                    )),
                ];
                rows.push(Some(match self.add_to_block() {
                    Some(_) => Entry::act("Add to block", "plus", Act::AddToBlock, !reviewing),
                    None => Entry::command(
                        ctx,
                        "pcb.make-block",
                        "Make block",
                        self.group.blocks.is_empty() && self.make_block().is_ok() && !reviewing,
                    ),
                }));
                // Only a group with parts has a population to toggle.
                if let Some(entry) =
                    crate::variants::menu_entry(project, &parts, Act::GroupPopulation, !reviewing)
                {
                    rows.push(Some(entry));
                }
                rows.extend([
                    None,
                    Some(
                        Entry::act(
                            "Delete",
                            "delete",
                            Act::DeleteGroup,
                            self.delete_group().is_some() && !reviewing,
                        )
                        .key_of(ctx, "pcb.delete"),
                    ),
                ]);
                rows
            }
            Target::Board => {
                let ribbon = self.ribbon(engine);
                let on = |id: &str| {
                    ribbon
                        .groups
                        .iter()
                        .flat_map(|g| &g.tools)
                        .any(|t| t.id == id && t.enabled)
                };
                vec![
                    Some(Entry::command(ctx, "pcb.route", "Route", on("route"))),
                    Some(Entry::command(ctx, "pcb.via", "Via", on("via"))),
                    None,
                    Some(Entry::command(ctx, "view.fit", "Fit view", true)),
                ]
            }
        }
    }

    /// Run a row's action: the operation its card button or key sends.
    pub(super) fn menu_act(&mut self, engine: &Engine, act: Act) -> Option<Operation> {
        let project = engine.project();
        let placement = |instance: Id| project.placements.iter().find(|p| p.instance == instance);
        let layout = |command| Some(Operation::Layout(command));
        match act {
            Act::Rotate(instance) => {
                let p = placement(instance)?;
                layout(LayoutCommand::Move {
                    instance,
                    position: p.position,
                    rotation_deg: (p.rotation_deg + 90.0).rem_euclid(360.0),
                    layer: p.layer,
                })
            }
            Act::Flip(instance) => {
                let p = placement(instance)?;
                layout(LayoutCommand::Move {
                    instance,
                    position: p.position,
                    rotation_deg: self.flipped_rotation(project, p),
                    layer: opposite(p.layer),
                })
            }
            Act::Lock(instance, locked) => {
                layout(LayoutCommand::SetPlacementLock { instance, locked })
            }
            Act::Constrain(instance) => {
                let p = placement(instance)?;
                let board = project.board();
                let edge = nearest_edge(&board, p.position)?;
                let sketch = dimensions::board_sketch(project, self.shape.evaluator?).ok()?;
                let dimension = spec_at(&sketch, instance, edge, p.position)?;
                Some(Operation::Dimension(DimensionCommand::Add { dimension }))
            }
            Act::Free(instance) => match dimensions_of(project, instance).as_slice() {
                [one] => Some(Operation::Dimension(DimensionCommand::Delete {
                    id: one.id,
                })),
                _ => None,
            },
            Act::Remove(instance) => {
                self.update.begin(
                    engine,
                    Change::Instance {
                        edit: InstanceChange::Remove { instance },
                    },
                );
                self.end_route();
                self.cancel_drag();
                None
            }
            Act::DeleteTrack(id) => layout(LayoutCommand::DeleteTrack { id }),
            Act::DeleteVia(id) => layout(LayoutCommand::DeleteVia { id }),
            Act::SelectNet(net) => {
                self.set_selection(None, None, Some(net));
                None
            }
            Act::Refill(id) => Some(Operation::Fill(FillCommand::Refill {
                target: RefillTarget::One(id),
            })),
            Act::DeleteFill(id) => Some(Operation::Fill(FillCommand::Delete { id })),
            Act::DeleteGroup => self.delete_group(),
            Act::LockGroup(locked) => layout(LayoutCommand::LockObjects {
                objects: self.group.objects(),
                locked,
            }),
            Act::Population(instance) => crate::variants::toggle(project, vec![instance]),
            Act::RotateBlock(id) => layout(blocks::turn().command(id)),
            Act::FlipBlock(id) => layout(blocks::flip().command(id)),
            Act::LockBlock(id, locked) => layout(LayoutCommand::SetBlockLock { id, locked }),
            Act::DeleteBlock(id) => layout(LayoutCommand::DeleteBlock { id }),
            Act::SelectBlock(id) => {
                self.select_block(id);
                None
            }
            Act::Release(id, instance) => layout(LayoutCommand::SetBlockMembers {
                id,
                add: Vec::new(),
                remove: vec![instance],
            }),
            Act::AddToBlock => self.add_to_block().map(|(_, operation)| operation),
            Act::GroupPopulation => {
                crate::variants::toggle(project, self.group.parts.iter().copied().collect())
            }
        }
    }

    /// A registry command a menu row asked the app to run, once.
    pub fn take_menu_command(&mut self) -> Option<&'static str> {
        self.menu_command.take()
    }

    /// Show the open menu; returns the operation a chosen row sends.
    pub(super) fn show_menu(&mut self, ctx: &egui::Context, engine: &Engine) -> Option<Operation> {
        let target = *self.menu.target()?;
        if self.tool != Tool::Select {
            self.menu.close();
            return None;
        }
        let rows = self.menu_rows(ctx, engine, target);
        match self.menu.show(ctx, &rows)? {
            crate::context_menu::Run::Command(id) => {
                self.menu_command = Some(id);
                None
            }
            crate::context_menu::Run::Act(act) => {
                let project = engine.project();
                match act {
                    Act::RotateBlock(id) => blocks::name_move(ctx, project, id, &blocks::turn()),
                    Act::FlipBlock(id) => blocks::name_move(ctx, project, id, &blocks::flip()),
                    _ => {}
                }
                self.menu_act(engine, act)
            }
        }
    }
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;

//! Layout blocks on the canvas, in headless egui frames on the engine: what
//! a click, the rectangle and ⌘A take at each level of detail, the drag of
//! a whole block and what refuses it, and that the level of detail changes
//! only what is drawn.
use super::super::raster_tests::view_of;
use super::super::{Inspected, menu};
use super::*;
use crate::kit::toast;
use egui::{Event, Modifiers, PointerButton, Rect};
use sparksmith_core::layout::RouteRequest;
use sparksmith_core::{Endpoint, Engine, Outcome, TerminalId};
use sparksmith_ui::fonts;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

impl PcbEditor {
    /// Where the whole app's canvas, drawn over `surface`, shows `p`.
    pub(crate) fn shown_at(
        &self,
        ctx: &egui::Context,
        surface: Rect,
        project: &Project,
        p: Point,
    ) -> Pos2 {
        let rect = crate::shell::canvas_clear(ctx, surface);
        super::super::board_view(
            rect,
            project.board_size,
            self.zoom,
            self.pan,
            self.from_bottom,
        )
        .screen(p)
    }
}

struct Canvas {
    engine: Engine,
    editor: PcbEditor,
    ctx: egui::Context,
    block: Id,
    parts: [Id; 2],
}

/// The demo with R1 at (10, 10) and R2 at (25, 10), a track between them,
/// and a block of both.
fn canvas() -> Fallible<Canvas> {
    let mut engine = Engine::demo()?;
    let id = |engine: &Engine, d: &str| {
        engine
            .project()
            .instance_by_designator(d)
            .map(|i| i.id)
            .ok_or("no part")
    };
    let parts = [id(&engine, "R1")?, id(&engine, "R2")?];
    let run =
        |engine: &mut Engine, command| engine.apply(engine.revision(), Operation::Layout(command));
    for (instance, position) in parts.into_iter().zip([[10.0, 10.0], [25.0, 10.0]]) {
        run(
            &mut engine,
            LayoutCommand::Move {
                instance,
                position,
                rotation_deg: 0.0,
                layer: Layer::Top,
            },
        )?;
    }
    let net = engine
        .project()
        .net_of(Endpoint {
            instance: parts[0],
            terminal: TerminalId(2),
        })
        .map(|n| n.id)
        .ok_or("no net")?;
    run(
        &mut engine,
        LayoutCommand::Route {
            request: RouteRequest {
                net,
                start: [11.2, 10.0],
                end: [23.8, 10.0],
                width: 0.3,
                layer: Layer::Top.into(),
            },
        },
    )?;
    let outcome = run(
        &mut engine,
        LayoutCommand::AddBlock {
            parts: parts.to_vec(),
            name: Some("Divider".into()),
        },
    )?;
    let Outcome::BlockAdded { block } = outcome else {
        return Err("no block".into());
    };
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    let editor = PcbEditor::default();
    let mut canvas = Canvas {
        engine,
        editor,
        ctx,
        block,
        parts,
    };
    for _ in 0..3 {
        canvas.frame(Modifiers::NONE, vec![]);
    }
    Ok(canvas)
}

impl Canvas {
    fn paint(
        &mut self,
        modifiers: Modifiers,
        events: Vec<Event>,
    ) -> (egui::FullOutput, Option<Operation>) {
        let mut command = None;
        let mut all = vec![Event::ModifiersChanged(modifiers)];
        all.extend(events);
        let Self {
            engine,
            editor,
            ctx,
            ..
        } = self;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))),
                events: all,
                ..Default::default()
            },
            |ui| command = editor.canvas(ui, engine),
        );
        output.textures_delta.clear();
        (output, command)
    }

    fn frame(&mut self, modifiers: Modifiers, events: Vec<Event>) -> Option<Operation> {
        self.paint(modifiers, events).1
    }

    fn at(&self, p: Point) -> Pos2 {
        view_of(&self.editor, &self.engine).screen(p)
    }

    fn press(at: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn click(&mut self, p: Point, modifiers: Modifiers) {
        let at = self.at(p);
        self.frame(
            modifiers,
            vec![
                Event::PointerMoved(at),
                Self::press(at, true, modifiers),
                Self::press(at, false, modifiers),
            ],
        );
        self.frame(modifiers, vec![]);
    }

    fn drag(&mut self, from: Point, to: Point, modifiers: Modifiers) -> Option<Operation> {
        let (from, to) = (self.at(from), self.at(to));
        let mut last = None;
        for events in [
            vec![
                Event::PointerMoved(from),
                Self::press(from, true, modifiers),
            ],
            vec![Event::PointerMoved(from + (to - from) * 0.25)],
            vec![Event::PointerMoved(to)],
            vec![Self::press(to, false, modifiers)],
        ] {
            last = self.frame(modifiers, events).or(last);
        }
        last
    }

    fn key(&mut self, key: egui::Key, modifiers: Modifiers) -> Option<Operation> {
        self.frame(
            modifiers,
            vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
        )
    }

    fn apply(&mut self, operation: Option<Operation>) -> Fallible {
        let operation = operation.ok_or("no operation")?;
        self.engine.apply_hosted(
            self.engine.revision(),
            operation,
            sparksmith_core::Evaluators {
                footprint: None,
                board: super::super::board_shape::host_evaluator(),
            },
        )?;
        self.frame(Modifiers::NONE, vec![]);
        Ok(())
    }

    fn layout(&mut self, command: LayoutCommand) -> Fallible {
        self.apply(Some(Operation::Layout(command)))
    }

    /// Draw the blocks at `detail`.
    fn show(&mut self, detail: Detail) {
        self.editor.blocks.choice = Choice::Fixed(detail);
        self.frame(Modifiers::NONE, vec![]);
    }

    /// The middle of the border's lowest side, clear of every part.
    fn on_border(&self) -> Fallible<Point> {
        let ring = self.editor.blocks.border(self.block).ok_or("no border")?;
        let low = ring.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
        Ok([17.5, low])
    }

    fn toasts(&self) -> Vec<String> {
        toast::take_posted(&self.ctx)
            .into_iter()
            .map(|p| p.text)
            .collect()
    }

    fn at_of(&self, part: usize) -> Fallible<Point> {
        Ok(self
            .engine
            .project()
            .placement_of(self.parts[part])
            .ok_or("not placed")?
            .position)
    }
}

fn near(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

/// The painted texts of a frame.
fn texts(output: &egui::FullOutput) -> Vec<String> {
    crate::kit::text::tests::painted(output)
        .into_iter()
        .map(|(text, ..)| text)
        .collect()
}

/// A click on the border takes the block, also where it runs along a part's
/// courtyard, and one on a part's body the part; drawn as
/// its border alone the block is taken anywhere in it, and its parts are
/// not. The selection filter passes the click on.
#[test]
fn a_click_takes_the_block_or_the_part_by_what_is_drawn() -> Fallible {
    let mut c = canvas()?;
    let border = c.on_border()?;
    c.show(Detail::Copper);
    c.click(border, Modifiers::NONE);
    assert_eq!(
        c.editor.inspected(c.engine.project()),
        Inspected::Block(c.block)
    );
    c.click([10.0, 10.0], Modifiers::NONE);
    assert_eq!(c.editor.selected_instance(), Some(c.parts[0]));
    assert_eq!(c.editor.selected_block(), None);
    // Between the parts there is only the track.
    c.click([17.5, 10.0], Modifiers::NONE);
    assert!(c.editor.selected_track().is_some());
    c.click([55.0, 35.0], Modifiers::NONE);
    c.show(Detail::Border);
    for at in [[10.0, 10.0], [17.5, 10.0], border] {
        c.click(at, Modifiers::NONE);
        assert_eq!(c.editor.selected_block(), Some(c.block), "{at:?}");
        c.click([55.0, 35.0], Modifiers::NONE);
        assert_eq!(c.editor.selected_block(), None);
    }
    c.editor.filter.blocks = false;
    c.click(border, Modifiers::NONE);
    assert_eq!(c.editor.inspected(c.engine.project()), Inspected::Board);
    Ok(())
}

/// The levels of detail draw less and less, change no document, and leave
/// what is selected and what is on no block drawn; an armed tool draws all.
#[test]
fn the_level_of_detail_changes_only_what_is_drawn() -> Fallible {
    let mut c = canvas()?;
    let before = c.engine.document().clone();
    let revision = c.engine.revision();
    let track = c.engine.project().tracks.first().ok_or("no track")?.id;
    let mut drawn = Vec::new();
    for detail in [Detail::Copper, Detail::Parts, Detail::Border] {
        c.show(detail);
        let (output, _) = c.paint(Modifiers::NONE, vec![]);
        drawn.push(output.shapes.len());
        let texts = texts(&output);
        assert!(texts.iter().any(|t| t == "Divider"), "{texts:?}");
        assert_eq!(
            texts.iter().any(|t| t == "2 parts"),
            detail == Detail::Border
        );
        assert_eq!(c.editor.part_hidden(c.parts[0]), detail == Detail::Border);
        assert_eq!(c.editor.pads_hidden(c.parts[0]), detail != Detail::Copper);
        assert_eq!(c.editor.track_hidden(track), detail != Detail::Copper);
    }
    assert!(drawn[0] > drawn[1] && drawn[1] > drawn[2], "{drawn:?}");
    c.editor.set_selection(Some(c.parts[0]), None, None);
    assert!(!c.editor.part_hidden(c.parts[0]) && c.editor.part_hidden(c.parts[1]));
    c.editor.set_selection(None, None, None);
    c.editor.tool = Tool::Via;
    c.frame(Modifiers::NONE, vec![]);
    assert!(!c.editor.part_hidden(c.parts[0]) && !c.editor.track_hidden(track));
    c.editor.tool = Tool::Select;
    // A part that leaves the block is drawn.
    let [r1, _] = c.parts;
    c.frame(Modifiers::NONE, vec![]);
    assert!(c.editor.part_hidden(r1));
    assert_eq!(c.engine.document(), &before);
    assert_eq!(c.engine.revision(), revision);
    c.layout(LayoutCommand::SetBlockMembers {
        id: c.block,
        add: vec![],
        remove: vec![r1],
    })?;
    c.frame(Modifiers::NONE, vec![]);
    assert!(!c.editor.part_hidden(r1));
    Ok(())
}

/// By the zoom: the border while no designator fits on its part, everything
/// once each does; the choice goes round and is kept for the next project.
#[test]
fn the_zoom_sets_the_level_of_detail() -> Fallible {
    let mut c = canvas()?;
    assert_eq!(c.editor.blocks.choice, Choice::Auto);
    let mut seen = Vec::new();
    for zoom in [0.2, 0.5, 1.0, 4.0, 12.0] {
        c.editor.zoom = zoom;
        c.frame(Modifiers::NONE, vec![]);
        seen.push(*c.editor.blocks.detail.get(&c.block).ok_or("no detail")?);
    }
    assert!(seen.is_sorted(), "{seen:?}");
    assert_eq!(seen.first(), Some(&Detail::Border));
    assert_eq!(seen.last(), Some(&Detail::Copper));
    let mut choice = Choice::Auto;
    for expected in [
        Choice::Fixed(Detail::Border),
        Choice::Fixed(Detail::Parts),
        Choice::Fixed(Detail::Copper),
        Choice::Auto,
    ] {
        choice = choice.next();
        assert_eq!(choice, expected);
    }
    c.editor.blocks.choice = Choice::Fixed(Detail::Parts);
    c.editor.document_opened();
    assert_eq!(c.editor.blocks.choice, Choice::Fixed(Detail::Parts));
    assert!(c.editor.blocks.borders.is_empty());
    Ok(())
}

/// A part's designator that no longer fits shows its block by parts: two
/// parts of different size at one zoom.
#[test]
fn a_block_shows_its_parts_once_one_of_them_reads() -> Fallible {
    let mut c = canvas()?;
    let mut seen = BTreeSet::new();
    // A long designator reads later than a short one.
    c.apply(Some(Operation::Instance(
        sparksmith_core::InstanceCommand::SetDesignator {
            instance: c.parts[1],
            designator: "R22222222".into(),
        },
    )))?;
    for step in 0..60 {
        c.editor.zoom = 0.2 + f32::from(u8::try_from(step)?) * 0.1;
        c.frame(Modifiers::NONE, vec![]);
        seen.insert(*c.editor.blocks.detail.get(&c.block).ok_or("no detail")?);
    }
    assert_eq!(
        seen,
        BTreeSet::from([Detail::Border, Detail::Parts, Detail::Copper])
    );
    Ok(())
}

/// Dragging the border is one `layout.block_move` of all its parts; a part
/// is still dragged alone, and an unmoved drag is nothing.
#[test]
fn dragging_the_border_moves_the_block() -> Fallible {
    let mut c = canvas()?;
    c.show(Detail::Copper);
    let border = c.on_border()?;
    let to = [border[0] + 3.0, border[1] + 4.0];
    let operation = c.drag(border, to, Modifiers::NONE);
    let expected = BlockMove {
        by: [3.0, 4.0],
        ..Default::default()
    };
    match &operation {
        Some(Operation::Layout(LayoutCommand::MoveBlock {
            id,
            by,
            turn_deg,
            flip,
        })) => {
            assert_eq!(*id, c.block);
            assert!(
                near(*by, expected.by) && *turn_deg == 0.0 && !flip,
                "{by:?}"
            );
        }
        other => return Err(format!("{other:?}").into()),
    }
    c.apply(operation)?;
    assert!(near(c.at_of(0)?, [13.0, 14.0]) && near(c.at_of(1)?, [28.0, 14.0]));
    assert_eq!(c.editor.selected_block(), Some(c.block));
    let alone = c.drag([13.0, 14.0], [14.0, 14.0], Modifiers::NONE);
    assert!(
        matches!(
            alone,
            Some(Operation::Layout(LayoutCommand::Move { instance, .. })) if instance == c.parts[0]
        ),
        "{alone:?}"
    );
    Ok(())
}

/// A locked block, a locked part and a part its dimensions hold refuse the
/// drag; each is named and nothing moves.
#[test]
fn what_holds_a_block_is_named() -> Fallible {
    use sparksmith_core::layout::dimensions::{self, DimensionCommand};
    use sparksmith_core::library::footprint_source::{DimKind, DimValue, SketchRef};
    use sparksmith_core::model::board::ConstraintTarget;
    use sparksmith_core::model::dimension::{
        BoardRef, FeatureRef, PartDimensionSpec, PartRelation,
    };
    let mut c = canvas()?;
    c.show(Detail::Copper);
    let border = c.on_border()?;
    let to = [border[0] + 3.0, border[1]];
    let (block, [r1, r2]) = (c.block, c.parts);
    c.layout(LayoutCommand::SetBlockLock {
        id: block,
        locked: true,
    })?;
    assert!(c.drag(border, to, Modifiers::NONE).is_none());
    let said = c.toasts();
    assert!(
        said.iter()
            .any(|t| t.contains("Can't move the block Divider")
                && t.contains("unlock it to move it")),
        "{said:?}"
    );
    // Nor does one part of it alone.
    assert!(
        c.drag([10.0, 10.0], [12.0, 10.0], Modifiers::NONE)
            .is_none()
    );
    let said = c.toasts();
    assert!(
        said.iter()
            .any(|t| t.contains("R1 is in the locked block Divider")),
        "{said:?}"
    );
    c.layout(LayoutCommand::SetBlockLock {
        id: block,
        locked: false,
    })?;
    c.layout(LayoutCommand::SetPlacementLock {
        instance: r2,
        locked: true,
    })?;
    assert!(c.drag(border, to, Modifiers::NONE).is_none());
    let said = c.toasts();
    assert!(said.iter().any(|t| t.contains("R2 is locked")), "{said:?}");
    c.layout(LayoutCommand::SetPlacementLock {
        instance: r2,
        locked: false,
    })?;
    // R1 held from two edges.
    for (edge, mm) in [("left", 10.0), ("bottom", 10.0)] {
        let ev = super::super::board_shape::host_evaluator().ok_or("no evaluator")?;
        let board = dimensions::board_sketch(c.engine.project(), ev)?;
        let curve = board
            .edges
            .iter()
            .find(|e| e.name.0 == edge)
            .map(|e| e.curve.clone())
            .ok_or("no edge")?;
        c.apply(Some(Operation::Dimension(DimensionCommand::Add {
            dimension: PartDimensionSpec {
                kind: PartRelation::Dim(DimKind::Distance),
                refs: vec![
                    BoardRef::Part(FeatureRef::Origin {
                        target: ConstraintTarget::Placement(r1),
                    }),
                    BoardRef::Board(SketchRef::Curve(curve)),
                ],
                value: Some(DimValue::Fixed(mm)),
                reference: false,
            },
        })))?;
    }
    let border = c.on_border()?;
    assert!(c.drag(border, to, Modifiers::NONE).is_none());
    let said = c.toasts();
    assert!(
        said.iter()
            .any(|t| t.starts_with("Can't move the block Divider: R1")),
        "{said:?}"
    );
    // And the engine refuses the same turn and flip, in the same words.
    for how in [turn(), flip()] {
        let before = c.engine.document().clone();
        let error = c
            .apply(Some(Operation::Layout(how.command(block))))
            .err()
            .ok_or("moved")?;
        assert!(error.to_string().contains("R1"), "{error}");
        assert_eq!(c.engine.document(), &before);
    }
    Ok(())
}

/// The rectangle and ⌘A take parts where they are drawn and the block where
/// its border stands for them; with the parts filtered out, the block.
#[test]
fn the_rectangle_and_select_all_take_what_is_drawn() -> Fallible {
    let mut c = canvas()?;
    let (from, to) = ([2.0, 2.0], [35.0, 20.0]);
    let blocks: BTreeSet<Id> = [c.block].into();
    let parts: BTreeSet<Id> = c.parts.into();
    c.show(Detail::Copper);
    c.drag(from, to, Modifiers::NONE);
    let taken = c.editor.selected();
    assert_eq!(
        (taken.parts, taken.blocks),
        (parts.clone(), BTreeSet::new())
    );
    c.editor.set_selection(None, None, None);
    c.show(Detail::Border);
    c.drag(from, to, Modifiers::NONE);
    let taken = c.editor.selected();
    assert!(taken.parts.is_empty() && taken.tracks.is_empty());
    assert_eq!(taken.blocks, blocks);
    c.editor.select_all(c.engine.project());
    assert_eq!(c.editor.selected().blocks, blocks);
    c.editor.set_selection(None, None, None);
    c.show(Detail::Copper);
    c.editor.filter.parts = false;
    c.drag(from, to, Modifiers::NONE);
    let taken = c.editor.selected();
    assert!(taken.parts.is_empty());
    assert_eq!(taken.blocks, blocks);
    // The filter switched off leaves the selection too.
    c.editor.filter.blocks = false;
    c.editor.deselect_filtered(c.engine.project());
    assert!(c.editor.selected().blocks.is_empty());
    Ok(())
}

/// Keys and the menu: ⌘R turns the selected block, Delete deletes it and
/// leaves its parts, and the part's menu lets it go.
#[test]
fn keys_and_menus_act_on_the_block() -> Fallible {
    let mut c = canvas()?;
    c.show(Detail::Copper);
    let border = c.on_border()?;
    c.click(border, Modifiers::NONE);
    let hover = c.at([55.0, 35.0]);
    c.frame(Modifiers::NONE, vec![Event::PointerMoved(hover)]);
    let turned = c.key(egui::Key::R, Modifiers::COMMAND);
    assert!(
        matches!(
            turned,
            Some(Operation::Layout(LayoutCommand::MoveBlock { turn_deg, .. })) if turn_deg == 90.0
        ),
        "{turned:?}"
    );
    let block = c.block;
    let target = c.editor.menu_select(menu::Hit::Block(block));
    assert_eq!(target, menu::Target::Block(block));
    let ctx = c.ctx.clone();
    let rows = c.editor.menu_rows(&ctx, &c.engine, target);
    let labels: Vec<&str> = rows.iter().flatten().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["Rotate 90°", "Flip side", "Lock", "Delete"]);
    let placements = c.engine.project().placements.clone();
    let deleted = c.key(egui::Key::Delete, Modifiers::NONE);
    c.apply(deleted)?;
    assert!(c.engine.project().blocks.is_empty());
    assert_eq!(c.engine.project().placements, placements);
    assert_eq!(c.editor.inspected(c.engine.project()), Inspected::Board);
    c.engine.undo()?;
    c.frame(Modifiers::NONE, vec![]);
    let r1 = c.parts[0];
    let rows = c.editor.menu_rows(&ctx, &c.engine, menu::Target::Part(r1));
    let labels: Vec<&str> = rows.iter().flatten().map(|e| e.label.as_str()).collect();
    assert!(labels.contains(&"Select block Divider"), "{labels:?}");
    let out = c.editor.menu_act(&c.engine, menu::Act::Release(block, r1));
    c.apply(out)?;
    assert_eq!(
        c.engine.project().block(block).ok_or("gone")?.members,
        [c.parts[1]]
    );
    Ok(())
}

/// A block and a part selected together move together, Add to block takes
/// the part in, and the flip of parts stays off.
#[test]
fn a_block_and_a_part_are_one_group() -> Fallible {
    let mut c = canvas()?;
    let (block, [r1, r2]) = (c.block, c.parts);
    c.layout(LayoutCommand::SetBlockMembers {
        id: block,
        add: vec![],
        remove: vec![r2],
    })?;
    let ring = c.editor.blocks.border(block).ok_or("no border")?.to_vec();
    let low = ring.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    c.show(Detail::Copper);
    c.click([10.0, low], Modifiers::NONE);
    c.click([25.0, 10.0], Modifiers::SHIFT);
    let group = c.editor.selected();
    assert_eq!(group.summary(), "1 part, 1 block");
    assert!(!group.flips_by_part());
    let (into, add) = c.editor.add_to_block().ok_or("no add")?;
    assert_eq!(into, block);
    let moved = c.drag([25.0, 10.0], [26.0, 12.0], Modifiers::NONE);
    match &moved {
        Some(Operation::Layout(LayoutCommand::MoveParts { moves, .. })) => {
            let ids: BTreeSet<Id> = moves.iter().map(|m| m.instance).collect();
            assert_eq!(ids, BTreeSet::from([r1, r2]));
        }
        other => return Err(format!("{other:?}").into()),
    }
    c.apply(moved)?;
    assert!(near(c.at_of(0)?, [11.0, 12.0]) && near(c.at_of(1)?, [26.0, 12.0]));
    c.apply(Some(add))?;
    assert_eq!(
        c.engine.project().block(block).ok_or("gone")?.members.len(),
        2
    );
    Ok(())
}

//! Layout blocks through the whole app's frame and accessibility tree:
//! Make block on two clicked parts, the name typed on its card, the Browser
//! row, the drag of the border, the lock and its refusal, undo, Delete, the
//! level of detail by key, and the file that keeps the block.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton};
use sparksmith_core::layout::LayoutCommand;
use sparksmith_core::model::Block;
use sparksmith_core::{Id, Layer, Point};

/// R1 at (10, 10) and R2 at (25, 10) on the demo board, in Layout, no
/// tracks.
fn layout() -> Fallible<(Ax, [Id; 2])> {
    let mut engine = Engine::demo()?;
    let mut parts = Vec::new();
    for (name, position) in [("R1", [10.0, 10.0]), ("R2", [25.0, 10.0])] {
        let instance = engine
            .project()
            .instance_by_designator(name)
            .ok_or("no part")?
            .id;
        engine.apply(
            engine.revision(),
            Operation::Layout(LayoutCommand::Move {
                instance,
                position,
                rotation_deg: 0.0,
                layer: Layer::Top,
            }),
        )?;
        parts.push(instance);
    }
    let parts = <[Id; 2]>::try_from(parts).map_err(|_| "two parts")?;
    let mut ax = Ax::with_engine(engine, Workspace::Pcb)?;
    ax.app.pcb.set_3d(false);
    // Every part is drawn whatever the zoom: Pads and copper.
    for _ in 0..3 {
        ax.key_with(Key::B, Modifiers::SHIFT);
    }
    Ok((ax, parts))
}

fn at(ax: &Ax, p: Point) -> Fallible<egui::Pos2> {
    let board = ax.find(Role::Canvas, "Board")?;
    Ok(ax
        .app
        .pcb
        .shown_at(&ax.ctx, board.rect, ax.app.engine.project(), p))
}

fn click(ax: &mut Ax, p: Point, modifiers: Modifiers) -> Fallible {
    let at = at(ax, p)?;
    ax.click_at_with(at, PointerButton::Primary, modifiers);
    Ok(())
}

fn drag(ax: &mut Ax, from: Point, to: Point) -> Fallible {
    let (from, to) = (at(ax, from)?, at(ax, to)?);
    let press = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    ax.frame(vec![Event::PointerMoved(from)]);
    ax.frame(vec![press(from, true)]);
    ax.frame(vec![Event::PointerMoved(from + (to - from) * 0.25)]);
    ax.frame(vec![Event::PointerMoved(to)]);
    ax.frame(vec![press(to, false)]);
    ax.settle();
    Ok(())
}

fn block(ax: &Ax) -> Fallible<Block> {
    match &ax.app.engine.project().blocks[..] {
        [one] => Ok(one.clone()),
        other => Err(format!("{} blocks", other.len()).into()),
    }
}

/// The middle of the border's lowest side.
fn border(ax: &Ax) -> Fallible<Point> {
    let borders = sparksmith_core::layout::blocks::borders(ax.app.engine.project());
    let (_, ring) = borders.first().ok_or("no border")?;
    let low = ring.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let x = ring.iter().map(|p| p[0]).sum::<f64>() / ring.len() as f64;
    Ok([x, low])
}

fn place(ax: &Ax, id: Id) -> Fallible<Point> {
    Ok(ax
        .app
        .engine
        .project()
        .placement_of(id)
        .ok_or("not placed")?
        .position)
}

fn said(ax: &Ax, text: &str) -> bool {
    ax.app.toasts.items().any(|t| t.text.contains(text))
}

/// Both parts clicked and ⌘G pressed.
fn made() -> Fallible<(Ax, [Id; 2])> {
    let (mut ax, parts) = layout()?;
    click(&mut ax, [10.0, 10.0], Modifiers::NONE)?;
    click(&mut ax, [25.0, 10.0], Modifiers::SHIFT)?;
    assert_eq!(ax.app.pcb.selected().parts.len(), 2, "{}", ax.dump());
    ax.command_key(Key::G);
    ax.settle();
    Ok((ax, parts))
}

#[test]
fn a_block_is_made_named_moved_and_deleted() -> Fallible {
    let (mut ax, [r1, r2]) = made()?;
    let before = block(&ax)?;
    assert_eq!(before.name, "Block 1");
    assert_eq!(before.members.len(), 2);
    assert_eq!(ax.app.pcb.selected_block(), Some(before.id));

    // Its name, typed on its card.
    ax.click(Role::TextInput, "Name")?;
    ax.key_with(Key::A, Modifiers::COMMAND);
    ax.type_text("Divider");
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(block(&ax)?.name, "Divider");
    // The Browser lists it, and its row selects it.
    click(&mut ax, [55.0, 35.0], Modifiers::NONE)?;
    assert_eq!(ax.app.pcb.selected_block(), None);
    if ax.all_prefix(Role::Button, "Divider").is_empty() {
        let folder = ax.find_prefix(Role::Button, "Blocks")?;
        ax.click_at(folder.center());
        ax.settle();
    }
    let row = ax.find_prefix(Role::Button, "Divider")?;
    ax.click_at(row.center());
    assert_eq!(ax.app.pcb.selected_block(), Some(before.id));
    click(&mut ax, [55.0, 35.0], Modifiers::NONE)?;

    // The border drags both parts, in one step.
    let grab = border(&ax)?;
    drag(&mut ax, grab, [grab[0] + 3.0, grab[1] + 4.0])?;
    assert_eq!(place(&ax, r1)?, [13.0, 14.0]);
    assert_eq!(place(&ax, r2)?, [28.0, 14.0]);
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(place(&ax, r1)?, [10.0, 10.0]);
    assert_eq!(place(&ax, r2)?, [25.0, 10.0]);

    // Delete takes the block and leaves the parts; undo brings it back.
    let grab = border(&ax)?;
    click(&mut ax, grab, Modifiers::NONE)?;
    assert_eq!(ax.app.pcb.selected_block(), Some(before.id));
    let placements = ax.app.engine.project().placements.clone();
    ax.key(Key::Delete);
    ax.settle();
    assert!(ax.app.engine.project().blocks.is_empty());
    assert_eq!(ax.app.engine.project().placements, placements);
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(block(&ax)?.name, "Divider");
    Ok(())
}

#[test]
fn a_locked_block_stays_and_says_so() -> Fallible {
    let (mut ax, [r1, _]) = made()?;
    ax.click(Role::CheckBox, "Locked")?;
    ax.settle();
    assert!(block(&ax)?.locked);
    let grab = border(&ax)?;
    drag(&mut ax, grab, [grab[0] + 3.0, grab[1]])?;
    assert_eq!(place(&ax, r1)?, [10.0, 10.0]);
    assert!(
        said(&ax, "The block Block 1 is locked; unlock it to move it"),
        "{:?}",
        ax.texts
    );
    drag(&mut ax, [10.0, 10.0], [12.0, 10.0])?;
    assert_eq!(place(&ax, r1)?, [10.0, 10.0]);
    assert!(said(&ax, "R1 is in the locked block Block 1"));
    // Its Rotate 90° and Flip are off; unlocked, Flip mirrors both parts.
    let grab = border(&ax)?;
    click(&mut ax, grab, Modifiers::NONE)?;
    assert!(ax.find(Role::Button, "Flip")?.disabled);
    ax.click(Role::CheckBox, "Locked")?;
    ax.settle();
    ax.button("Flip")?;
    ax.settle();
    assert_eq!(place(&ax, r1)?, [25.0, 10.0]);
    assert!(
        ax.app
            .engine
            .project()
            .placements
            .iter()
            .filter(|p| block(&ax).is_ok_and(|b| b.members.contains(&p.instance)))
            .all(|p| p.layer == Layer::Bottom)
    );
    Ok(())
}

/// A name another block has is refused under the field, and the text stays.
#[test]
fn a_taken_name_is_refused_at_its_field() -> Fallible {
    let (mut ax, [r1, r2]) = layout()?;
    for (id, name) in [(r1, "Left"), (r2, "Right")] {
        let revision = ax.app.engine.revision();
        ax.app.engine.apply(
            revision,
            Operation::Layout(LayoutCommand::AddBlock {
                parts: vec![id],
                name: Some(name.into()),
            }),
        )?;
    }
    let first = ax.app.engine.project().blocks[0].id;
    ax.app.pcb.select_block(first);
    ax.settle();
    ax.click(Role::TextInput, "Name")?;
    ax.key_with(Key::A, Modifiers::COMMAND);
    ax.type_text("right");
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(ax.app.engine.project().blocks[0].name, "Left");
    assert!(
        ax.texts.iter().any(|t| t.contains("already a block Right")),
        "{:?}",
        ax.texts
    );
    assert!(ax.texts.iter().any(|t| t == "right"), "{:?}", ax.texts);
    Ok(())
}

/// Without parts selected Make block says what to do; a part in a block is
/// named and nothing is made.
#[test]
fn make_block_says_why_not() -> Fallible {
    let (mut ax, _) = made()?;
    click(&mut ax, [55.0, 35.0], Modifiers::NONE)?;
    ax.command_key(Key::G);
    ax.settle();
    assert!(said(&ax, "Select the block's parts"));
    click(&mut ax, [10.0, 10.0], Modifiers::NONE)?;
    ax.command_key(Key::G);
    ax.settle();
    assert!(said(&ax, "R1 is in the block Block 1"), "{:?}", ax.texts);
    block(&ax)?;
    // The part's card names its block and selects it.
    ax.button("Block 1")?;
    assert_eq!(ax.app.pcb.selected_block(), Some(block(&ax)?.id));
    Ok(())
}

/// ⇧B steps the level of detail; the document and its revision stay.
#[test]
fn the_level_of_detail_key_changes_no_document() -> Fallible {
    let (mut ax, _) = made()?;
    let revision = ax.app.engine.revision();
    let document = ax.app.engine.document().clone();
    click(&mut ax, [55.0, 35.0], Modifiers::NONE)?;
    let shown = |ax: &Ax| ax.texts.iter().any(|t| t == "2 parts");
    let mut seen = Vec::new();
    for _ in 0..4 {
        ax.key_with(Key::B, Modifiers::SHIFT);
        ax.settle();
        seen.push(shown(&ax));
    }
    // By the zoom, the border, the parts, pads and copper: the count stands
    // in the border only.
    assert_eq!(seen[1..], [true, false, false]);
    assert_eq!(ax.app.engine.document(), &document);
    assert_eq!(ax.app.engine.revision(), revision);
    Ok(())
}

/// ⌘S writes the block; the file opens with it.
#[test]
fn a_saved_file_keeps_the_block() -> Fallible {
    let (mut ax, _) = made()?;
    let made = block(&ax)?;
    ax.command_key(Key::S);
    let path = ax.path();
    ax.until("the save", |ax| {
        !ax.app.files.dirty() && path.exists()
    })?;
    let opened = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    assert_eq!(opened.project().blocks[..], [made]);
    Ok(())
}

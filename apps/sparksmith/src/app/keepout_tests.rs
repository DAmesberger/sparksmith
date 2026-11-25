//! Board keepouts through the whole app: drawn with the Keepout tool,
//! selected by a click on the edge, changed on the card and deleted, one
//! undo step each.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;
use egui::{Key, Modifiers, PointerButton};
use sparksmith_core::Point;
use sparksmith_core::model::fill::KeepoutLayers;

fn at(ax: &Ax, p: Point) -> Fallible<egui::Pos2> {
    let board = ax.find(Role::Canvas, "Board")?;
    Ok(ax
        .app
        .pcb
        .shown_at(&ax.ctx, board.rect, ax.app.engine.project(), p))
}

fn click(ax: &mut Ax, p: Point) -> Fallible {
    let at = at(ax, p)?;
    ax.click_at_with(at, PointerButton::Primary, Modifiers::NONE);
    Ok(())
}

#[test]
fn a_keepout_is_drawn_changed_and_deleted() -> Fallible {
    let mut ax = Ax::new(Workspace::Pcb)?;
    ax.app.pcb.set_3d(false);
    ax.settle();
    ax.button("Keepout")?;
    ax.settle();
    // On the top layer only, and no pads either.
    let label = ax.find(Role::Label, "Layers")?;
    let combo = ax
        .all_prefix(Role::ComboBox, "")
        .into_iter()
        .find(|n| (n.center().y - label.center().y).abs() < 4.0)
        .cloned()
        .ok_or_else(|| format!("no Layers dropdown in:\n{}", ax.dump()))?;
    ax.click_at(combo.center());
    ax.button("Top only")?;
    ax.click(Role::CheckBox, "No pads")?;
    let revision = ax.app.engine.revision();
    for corner in [[40.0, 5.0], [50.0, 5.0], [50.0, 15.0], [40.0, 15.0]] {
        click(&mut ax, corner)?;
    }
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(ax.app.engine.revision(), revision + 1, "one step");
    let keepout = ax
        .app
        .engine
        .project()
        .keepouts
        .first()
        .cloned()
        .ok_or("no keepout")?;
    assert_eq!(
        keepout.layers,
        KeepoutLayers::Only(vec![sparksmith_core::CopperLayer::Top])
    );
    assert!(keepout.forbid.pads && keepout.forbid.tracks);
    // Back to Select; a click on its edge selects it.
    ax.key(Key::Escape);
    ax.key(Key::Escape);
    ax.button("Select")?;
    click(&mut ax, [45.0, 5.0])?;
    ax.settle();
    assert!(ax.has(Role::CheckBox, "No vias"), "{}", ax.dump());
    ax.click(Role::CheckBox, "No vias")?;
    ax.settle();
    let edited = ax.app.engine.project().keepouts.first().cloned();
    assert!(edited.is_some_and(|k| !k.forbid.vias), "the card edits it");
    ax.key(Key::Delete);
    ax.settle();
    assert!(ax.app.engine.project().keepouts.is_empty());
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(ax.app.engine.project().keepouts.len(), 1, "undo");
    Ok(())
}

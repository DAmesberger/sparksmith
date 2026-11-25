//! Board text through the whole app: placed with the Text tool, selected
//! by a click on it, changed on its card and deleted, one undo step each.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;
use egui::{Key, Modifiers, PointerButton};
use sparksmith_core::Point;
use sparksmith_core::model::Layer;

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

/// The dropdown in the row labelled `label`.
fn dropdown(ax: &Ax, label: &str) -> Fallible<egui::Pos2> {
    let label = ax.find(Role::Label, label)?;
    ax.all_prefix(Role::ComboBox, "")
        .into_iter()
        .find(|n| (n.center().y - label.center().y).abs() < 4.0)
        .map(|n| n.center())
        .ok_or_else(|| format!("no dropdown in:\n{}", ax.dump()).into())
}

#[test]
fn board_text_is_placed_changed_and_deleted() -> Fallible {
    let mut ax = Ax::new(Workspace::Pcb)?;
    ax.app.pcb.set_3d(false);
    ax.settle();
    ax.button("Text")?;
    ax.settle();
    let revision = ax.app.engine.revision();
    // Without text a click places nothing and says why.
    click(&mut ax, [30.0, 20.0])?;
    ax.settle();
    assert_eq!(ax.app.engine.revision(), revision);
    assert!(
        ax.dump().contains("Type the text to place first"),
        "{}",
        ax.dump()
    );
    let field = ax.find(Role::MultilineTextInput, "Text")?;
    ax.click_at(field.center());
    ax.type_text("REV A");
    ax.settle();
    click(&mut ax, [30.2, 20.1])?;
    ax.settle();
    assert_eq!(ax.app.engine.revision(), revision + 1, "one step");
    let text = ax
        .app
        .engine
        .project()
        .board_texts
        .first()
        .cloned()
        .ok_or("no text")?;
    assert_eq!(text.text, "REV A");
    assert_eq!(text.side, Layer::Top);
    // Centred on the 0.5 mm grid.
    assert_eq!(text.position.to_mm_f64(), [30.0, 20.0]);
    let min = ax
        .app
        .engine
        .project()
        .rules
        .effective()
        .silkscreen
        .text_height_min;
    assert!((text.height_nm.to_mm_f64() - min.max(0.1)).abs() < 1e-9);
    // Back to Select; a click on the text selects it, and its card moves
    // it to the bottom silkscreen.
    ax.key(Key::Escape);
    ax.button("Select")?;
    click(&mut ax, [30.0, 20.0])?;
    ax.settle();
    let side = dropdown(&ax, "Side")?;
    ax.click_at(side);
    ax.button("Bottom silkscreen (mirrored)")?;
    ax.settle();
    let edited = ax.app.engine.project().board_texts.first().cloned();
    assert!(
        edited
            .as_ref()
            .is_some_and(|t| t.side == Layer::Bottom && t.text == "REV A"),
        "{edited:?}"
    );
    ax.key(Key::Delete);
    ax.settle();
    assert!(ax.app.engine.project().board_texts.is_empty());
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(ax.app.engine.project().board_texts.len(), 1, "undo");
    Ok(())
}

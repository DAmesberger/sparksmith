//! Discard draft through the whole app (run 4, C10).
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;

/// After Discard draft the start page shows without taking the keyboard,
/// so Ctrl+Z undoes the discard.
#[test]
fn undo_after_discard_brings_the_draft_back() -> Fallible {
    let mut ax = Ax::new(Workspace::Part)?;
    ax.settle();
    ax.button("Blank component")?;
    ax.settle();
    let drafts = ax.app.engine.drafts().items.len();
    ax.app.part.on_tab("details");
    ax.settle();
    ax.click(Role::Button, "Discard draft…")?;
    ax.settle();
    ax.button("Discard")?;
    ax.settle();
    assert!(ax.app.engine.drafts().items.len() < drafts, "discarded");
    assert!(!ax.ctx.text_edit_focused(), "a field took the keyboard");
    ax.command_key(egui::Key::Z);
    ax.settle();
    assert_eq!(
        ax.app.engine.drafts().items.len(),
        drafts,
        "Ctrl+Z undid the discard"
    );
    Ok(())
}

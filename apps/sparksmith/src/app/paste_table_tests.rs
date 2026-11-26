//! Paste table through the whole app (run 4, C2): a table typed into
//! a blank component's Pins tab, key by key, in a full-HD window.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;

/// The table run 4 typed, line by line.
const TYPED: &str = include_str!("paste_table_typed.txt");

/// A blank component open on its Pins tab with Paste table open, in a
/// window the size run 4 used.
fn pasting() -> Fallible<Ax> {
    let mut ax = Ax::new(Workspace::Part)?;
    ax.window = egui::vec2(1920.0, 1080.0);
    ax.settle();
    ax.button("Blank component")?;
    ax.settle();
    ax.app.part.on_tab("pins");
    ax.settle();
    ax.button("Paste table…")?;
    ax.settle();
    Ok(ax)
}

fn typed(ax: &Ax) -> Fallible<String> {
    Ok(ax
        .app
        .part
        .paste_text()
        .ok_or("Paste table closed")?
        .to_owned())
}

/// Run 4, C2: every key of a typed table lands in the table, to its last
/// line; the keyboard never moves on to Separator. Run 4 lost it to its
/// virtual keyboard, which put "_" on Tab's key code (egui-winit then names
/// the key Tab); a keyboard sends "_" as Shift and minus.
#[test]
fn a_table_typed_key_by_key_keeps_the_keyboard() -> Fallible {
    let mut ax = pasting()?;
    let field = ax.find_prefix(Role::MultilineTextInput, "")?.center();
    ax.click_at(field);
    ax.settle();
    for (at, c) in TYPED.char_indices() {
        if c == '\n' {
            ax.key(egui::Key::Enter);
        } else if c == '_' {
            // As a keyboard sends it: Shift and the minus key, then "_".
            ax.late(vec![
                egui::Event::Key {
                    key: egui::Key::Minus,
                    physical_key: Some(egui::Key::Minus),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                },
                egui::Event::Text("_".into()),
            ]);
        } else {
            ax.type_text(&c.to_string());
        }
        let text = typed(&ax)?;
        assert_eq!(
            text.len(),
            at + c.len_utf8(),
            "the keyboard left the table after {:?}:\n{}",
            text.lines().last(),
            ax.dump()
        );
    }
    Ok(())
}

/// A fast typist's Backspace and the next letter in one frame both count:
/// the press is never taken for the letter's.
#[test]
fn backspace_and_a_letter_in_one_frame_keep_both() -> Fallible {
    let mut ax = pasting()?;
    let field = ax.find_prefix(Role::MultilineTextInput, "")?.center();
    ax.click_at(field);
    ax.settle();
    ax.type_text("Pin,Nb");
    let key = |key| egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    ax.late(vec![
        key(egui::Key::Backspace),
        key(egui::Key::A),
        egui::Event::Text("a".into()),
    ]);
    assert_eq!(typed(&ax)?, "Pin,Na");
    // Backspace straight before text no key names ("<" on some layouts).
    ax.late(vec![
        key(egui::Key::Backspace),
        egui::Event::Text("m".into()),
    ]);
    assert_eq!(typed(&ax)?, "Pin,Nm");
    Ok(())
}

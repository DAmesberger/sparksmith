//! The window's workflow entry points on the whole app: the palette's
//! generated entry, a PDF dropped on the window and `assist.offer` through
//! the window each put the workflow's compact confirmation in the chat.
use crate::app::ax_harness::{Ax, Fallible};
use crate::shell::Workspace;
use egui::{Key, Role};
use sparksmith_automation::WindowHost as _;
use sparksmith_core::Engine;

fn offers(ax: &Ax) -> usize {
    ax.app.workflow_cards.offers()
}

/// ⌘P, "datasheet component", Enter: the palette's entry for the workflow,
/// made from its data, shows its card. ("datasheet" alone also names the
/// datasheet-to-body workflow, which sorts first by title.)
#[test]
fn the_palette_offers_a_workflow_by_its_title() -> Fallible {
    for size in [egui::vec2(1024.0, 640.0), egui::vec2(1920.0, 1080.0)] {
        let mut ax = Ax::new(Workspace::Pcb)?;
        ax.window = size;
        ax.app.palette.open();
        ax.settle();
        ax.type_text("datasheet component");
        ax.settle();
        ax.find(Role::Label, "Assistant: Datasheet → component")?;
        assert!(
            ax.all(Role::Label, "Assistant: Datasheet → 3D body")
                .is_empty()
        );
        ax.key(Key::Enter);
        assert_eq!(offers(&ax), 1);
        assert!(!ax.app.palette.is_open());
        // The chat opens with the card in its transcript.
        ax.settle();
        assert!(ax.app.assistant.open);
        ax.find(Role::Label, "Datasheet → component")?;
        ax.find(Role::Button, "Accept and continue")?;
    }
    Ok(())
}

/// A PDF dropped on the window offers the datasheet workflow; a file no
/// workflow takes is said in a toast.
#[test]
fn a_pdf_dropped_on_the_window_offers_the_datasheet_workflow() -> Fallible {
    let mut ax = Ax::new(Workspace::Part)?;
    let pdf = ax.dir.path().join("tlv757p.pdf");
    std::fs::write(&pdf, b"%PDF-1.7\n1 0 obj << /Type /Page >> endobj\n")?;
    let text = ax.dir.path().join("notes.txt");
    std::fs::write(&text, "notes")?;
    let dropped = |path: &std::path::Path| -> egui::DroppedFileHandle {
        std::sync::Arc::new(Dropped(path.to_path_buf()))
    };
    ax.drop_files(vec![dropped(&pdf), dropped(&text)]);
    ax.settle();
    assert_eq!(offers(&ax), 1);
    // Read off the UI thread for its pages, then shown on the card.
    ax.until("the dropped PDF is read", |ax| {
        ax.find(Role::Button, "tlv757p.pdf · 1 page · 1 KB ⌄")
            .is_ok()
    })?;
    assert!(
        ax.texts.iter().any(|t| t == "No workflow takes notes.txt"),
        "{:?}",
        ax.texts
    );
    Ok(())
}

/// `assist.offer` through the window shows the card the front door checked.
#[test]
fn the_window_shows_an_offer_in_the_chat() -> Fallible {
    let mut ax = Ax::with_engine(Engine::demo()?, Workspace::Pcb)?;
    let listed = crate::workflow_cards::tests::shipped();
    let offer = sparksmith_assist::workflow::Offer::check(
        &listed,
        "datasheet-to-component",
        serde_json::Map::new(),
        "You asked for a part.",
    )?;
    let card = ax
        .app
        .window()
        .offer(&offer)
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(card, "card-1");
    assert_eq!(offers(&ax), 1);
    Ok(())
}

/// A file dropped on the window from this computer.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// `/` in the chat's composer lists the workflows; Enter shows the chosen
/// one's card in the transcript and clears the composer.
#[test]
fn the_slash_menu_in_the_composer_offers_a_workflow() -> Fallible {
    let mut ax = Ax::new(Workspace::Pcb)?;
    ax.key_with(Key::A, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    ax.ctx
        .memory_mut(|m| m.request_focus(crate::assistant::Assistant::composer_id()));
    ax.frame(vec![]);
    ax.type_text("/datasheet");
    ax.settle();
    ax.find(Role::Button, "Datasheet → component")?;
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(offers(&ax), 1);
    assert!(
        ax.app.assistant.composer.is_empty(),
        "{:?}",
        ax.app.assistant.composer
    );
    ax.find(Role::Button, "Accept and continue")?;
    Ok(())
}

//! The Place palette's search through the whole app, with the shipped
//! library and generators (run 3, A2 and A3): a word matches where a word
//! starts, a maker's name never stands for a kind of part, words people
//! say mean what the search vocabulary gives them, and a bare multiplier
//! reads as the kinds that take it bare.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::{Key, Modifiers};

/// The palette open on `query`, once the shipped generators are read.
fn searched(query: &str) -> Fallible<Ax> {
    let mut ax = Ax::new(Workspace::Schematic)?;
    ax.button("Place")?;
    ax.settle();
    ax.type_text(query);
    ax.until("the shipped generators", |ax| {
        ax.app.schematic.palette_ref().shelf_ready()
    })?;
    ax.settle();
    Ok(ax)
}

/// The rows the palette lists, top to bottom.
fn rows(ax: &Ax) -> Vec<String> {
    ax.app.schematic.palette_ref().row_names()
}

#[test]
fn fuse_finds_no_part_named_after_a_maker() -> Fallible {
    let ax = searched("fuse")?;
    let shown = rows(&ax);
    assert!(!shown.iter().any(|r| r.contains("Littelfuse")), "{shown:?}");
    Ok(())
}

#[test]
fn button_finds_the_tactile_switches() -> Fallible {
    let ax = searched("button")?;
    let shown = rows(&ax);
    assert!(!shown.iter().any(|r| r.contains("MDSM")), "{shown:?}");
    assert!(
        shown
            .iter()
            .any(|r| r.to_lowercase().contains("tactile") || r.contains("EVQP")),
        "{shown:?}"
    );
    Ok(())
}

#[test]
fn a_bare_nano_is_a_capacitance_and_enter_makes_the_capacitor() -> Fallible {
    let mut ax = searched("100n 0603")?;
    let shown = rows(&ax);
    let first = shown.first().ok_or("no rows")?;
    assert!(first.to_lowercase().contains("capacitor"), "{shown:?}");
    assert!(
        !shown.iter().any(|r| r.to_lowercase().contains("resistor")),
        "{shown:?}"
    );
    let components = ax.app.engine.project().library.components.len();
    ax.key(Key::Enter);
    ax.until("the capacitor being made", |ax| {
        ax.app.engine.project().library.components.len() > components
    })?;
    let made = ax
        .app
        .engine
        .project()
        .library
        .components
        .last()
        .map(|c| c.meta.name.to_lowercase())
        .unwrap_or_default();
    assert!(made.contains("capacitor"), "{made}");
    Ok(())
}

#[test]
fn sma_lists_the_sma_diode_package() -> Fallible {
    let mut ax = searched("sma")?;
    let shown = rows(&ax);
    assert!(shown.iter().any(|r| r.contains("SMA")), "{shown:?}");
    // Enter on nothing places nothing.
    ax.key_with(Key::A, Modifiers::COMMAND);
    ax.type_text("zzzz");
    ax.settle();
    assert!(rows(&ax).is_empty(), "{:?}", rows(&ax));
    let before = ax.app.engine.revision();
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(ax.app.engine.revision(), before, "Enter on no rows");
    Ok(())
}

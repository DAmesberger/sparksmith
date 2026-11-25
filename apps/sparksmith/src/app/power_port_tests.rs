//! The Power tool's card through the whole app: a typed supply name draws
//! a supply symbol, a name the design already has draws as its ports do,
//! and a style chosen by hand is kept.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::Key;
use egui::accesskit::Role;

/// The style the Power card shows, read from its Style row's dropdown.
fn shown_style(ax: &Ax) -> Fallible<String> {
    let label = ax.find(Role::Label, "Style")?;
    let combo = ax
        .all_prefix(Role::ComboBox, "")
        .into_iter()
        .find(|n| (n.center().y - label.center().y).abs() < 4.0)
        .cloned()
        .ok_or_else(|| format!("no Style dropdown in:\n{}", ax.dump()))?;
    ax.seen
        .iter()
        .find(|(_, r)| combo.rect.contains(r.center()))
        .map(|(t, _)| t.clone())
        .ok_or_else(|| "no text in the Style dropdown".into())
}

fn type_name(ax: &mut Ax, name: &str) -> Fallible {
    ax.click(Role::TextInput, "Name")?;
    ax.key_with(Key::A, egui::Modifiers::COMMAND);
    ax.type_text(name);
    ax.key(Key::Enter);
    ax.settle();
    Ok(())
}

#[test]
fn a_supply_name_draws_a_supply_symbol() -> Fallible {
    let mut ax = Ax::new(Workspace::Schematic)?;
    ax.button("Power")?;
    ax.settle();
    assert!(shown_style(&ax)?.contains("Ground"), "GND first");
    type_name(&mut ax, "+3V3")?;
    assert!(shown_style(&ax)?.contains("Bar"), "{}", shown_style(&ax)?);
    type_name(&mut ax, "VBUS")?;
    assert!(shown_style(&ax)?.contains("Arrow"));
    type_name(&mut ax, "VDD_IO")?;
    assert!(shown_style(&ax)?.contains("Bar"));
    Ok(())
}

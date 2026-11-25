//! Annotate through the whole app's frame and accessibility tree: a name a
//! person gave is kept, whatever its letters, and renaming one (Renumber
//! all) is listed on the sheet card before anything changes.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;
use sparksmith_core::{Id, InstanceCommand};

fn designator(ax: &Ax, id: Id) -> String {
    ax.app
        .engine
        .project()
        .instance(id)
        .map(|i| i.designator.clone())
        .unwrap_or_default()
}

/// The demo's R1 renamed J1 by hand and R2 left unnumbered (`R?`).
fn sheet() -> Fallible<(Ax, Id, Id)> {
    let mut engine = Engine::demo()?;
    let id = |e: &Engine, d: &str| -> Fallible<Id> {
        Ok(e.project().instance_by_designator(d).ok_or("no part")?.id)
    };
    let (r1, r2) = (id(&engine, "R1")?, id(&engine, "R2")?);
    for (instance, designator) in [(r1, "J1"), (r2, "R?")] {
        engine.apply(
            engine.revision(),
            Operation::Instance(InstanceCommand::SetDesignator {
                instance,
                designator: designator.into(),
            }),
        )?;
    }
    let ax = Ax::with_engine(engine, Workspace::Schematic)?;
    Ok((ax, r1, r2))
}

/// The ribbon's Annotate, the topmost of the buttons so named.
fn ribbon_annotate(ax: &mut Ax) -> Fallible {
    let top = ax
        .all(Role::Button, "Annotate")
        .into_iter()
        .min_by(|a, b| a.center().y.total_cmp(&b.center().y))
        .cloned()
        .ok_or("no Annotate")?;
    ax.click_at(top.center());
    ax.settle();
    Ok(())
}

fn said(ax: &Ax, text: &str) -> bool {
    ax.app.toasts.items().any(|t| t.text.contains(text))
}

#[test]
fn annotate_keeps_a_given_name_and_names_what_it_numbers() -> Fallible {
    let (mut ax, j1, r2) = sheet()?;
    ax.button("Annotate")?;
    ax.settle();
    assert_eq!(designator(&ax, j1), "J1", "a given name is kept");
    assert_eq!(designator(&ax, r2), "R1");
    assert!(
        said(&ax, "Annotated 1 designator: R? → R1"),
        "{:?}",
        ax.texts
    );
    Ok(())
}

#[test]
fn renumbering_a_given_name_is_listed_first() -> Fallible {
    let (mut ax, j1, r2) = sheet()?;
    // Tall enough for the sheet card's Annotate section.
    ax.window.y = 1600.0;
    ax.settle();
    // Renumber all, on the sheet card's Annotate section.
    ax.click(Role::DisclosureTriangle, "Annotate")?;
    ax.settle();
    let label = ax.find(Role::Label, "Designators")?;
    let combo = ax
        .all_prefix(Role::ComboBox, "")
        .into_iter()
        .find(|n| (n.center().y - label.center().y).abs() < 4.0)
        .cloned()
        .ok_or_else(|| format!("no Designators dropdown in:\n{}", ax.dump()))?;
    ax.click_at(combo.center());
    ax.button("Renumber all")?;
    ax.settle();
    let revision = ax.app.engine.revision();
    ribbon_annotate(&mut ax)?;
    // Nothing changed yet: the card lists the renames.
    assert_eq!(ax.app.engine.revision(), revision);
    assert_eq!(designator(&ax, j1), "J1");
    assert!(
        ax.texts
            .iter()
            .any(|t| t == "Renames 1 numbered designator"),
        "{:?}",
        ax.texts
    );
    assert!(ax.texts.iter().any(|t| t == "J1"), "{:?}", ax.texts);
    ax.button("Cancel")?;
    ax.settle();
    assert_eq!(ax.app.engine.revision(), revision);
    ribbon_annotate(&mut ax)?;
    ax.button("Rename")?;
    ax.settle();
    let renamed = [designator(&ax, j1), designator(&ax, r2)];
    assert!(renamed.iter().all(|d| d.starts_with('R')), "{renamed:?}");
    ax.command_key(egui::Key::Z);
    ax.settle();
    assert_eq!(designator(&ax, j1), "J1", "one undo step");
    Ok(())
}

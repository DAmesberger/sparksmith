//! Assembly variants through the whole app, by role and name
//! ([`Ax`]): the Assembly variants card's add, rename, duplicate, active
//! choice and delete, each one undo step, and the Placement card's
//! population checks.
use super::ax_harness::{Ax, Fallible};
use super::*;
use crate::pcb_editor::variant_fixtures::{board, lite_and_alt};
use egui::{Key, Modifiers, Role};

fn names(ax: &Ax) -> Vec<String> {
    let project = ax.app.engine.project();
    project
        .variants
        .list
        .iter()
        .map(|v| v.name.clone())
        .collect()
}

fn active(ax: &Ax) -> Option<String> {
    ax.app
        .engine
        .project()
        .variants
        .active()
        .map(|v| v.name.clone())
}

/// Each change in the card is one command and one undo step; undoing them
/// all leaves no variant.
#[test]
fn the_card_adds_renames_duplicates_activates_and_deletes_variants() -> Fallible {
    let mut ax = Ax::with_engine(board()?.engine, Workspace::Pcb)?;
    let ctx = ax.ctx.clone();
    ax.app.run_command(&ctx, "pcb.variants");
    ax.settle();

    ax.button("Add variant")?;
    ax.settle();
    assert_eq!(names(&ax), ["Variant 1"]);

    ax.click(Role::TextInput, "Name")?;
    ax.key_with(Key::A, Modifiers::COMMAND);
    ax.type_text("Lite");
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(names(&ax), ["Lite"], "renamed on Enter");

    ax.button("Duplicate")?;
    ax.settle();
    assert_eq!(names(&ax), ["Lite", "Lite copy"]);

    // The Active dropdown is named by its row; it is the combo box on the
    // row's line. Its option "Lite" is the one that is not the list row.
    let label = ax.find(Role::Label, "Active")?;
    let combo = ax
        .all_prefix(Role::ComboBox, "")
        .into_iter()
        .find(|n| (n.center().y - label.center().y).abs() < 4.0)
        .cloned()
        .ok_or_else(|| format!("no Active dropdown in:\n{}", ax.dump()))?;
    let row = ax.find(Role::Button, "Lite")?;
    ax.click_at(combo.center());
    let option = ax
        .all(Role::Button, "Lite")
        .into_iter()
        .find(|n| n.rect != row.rect)
        .cloned()
        .ok_or_else(|| format!("no option Lite in:\n{}", ax.dump()))?;
    ax.click_at(option.center());
    ax.settle();
    assert_eq!(active(&ax).as_deref(), Some("Lite"));
    assert!(
        ax.texts.iter().any(|t| t == "Lite variant"),
        "the active variant's pill"
    );

    // The duplicate is chosen: Delete variant removes it.
    ax.button("Delete variant")?;
    ax.settle();
    assert_eq!(names(&ax), ["Lite"]);

    for (step, (expected, on)) in [
        (vec!["Lite", "Lite copy"], Some("Lite")),
        (vec!["Lite", "Lite copy"], None),
        (vec!["Lite"], None),
        (vec!["Variant 1"], None),
        (vec![], None),
    ]
    .into_iter()
    .enumerate()
    {
        ax.command_key(Key::Z);
        ax.settle();
        assert_eq!(names(&ax), expected, "undo {step}");
        assert_eq!(active(&ax).as_deref(), on, "undo {step}");
    }
    Ok(())
}

/// The Placement card of a selected part shows its population in the
/// default and, with Lite active, in Lite; each check is one step.
#[test]
fn the_placement_card_sets_the_population_in_the_default_and_the_active_variant() -> Fallible {
    let mut b = board()?;
    lite_and_alt(&mut b)?;
    let r1 = b.r1;
    let mut ax = Ax::with_engine(b.engine, Workspace::Pcb)?;
    // The Assembly section sits right under Placement: the harness's
    // normal-height window shows it above the pinned footer without
    // scrolling.
    ax.app.pcb.set_selection(Some(r1), None, None);
    ax.settle();
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, ax.window);
    for row in ["Populated by default", "Populated in Lite"] {
        assert!(
            ax.seen
                .iter()
                .any(|(text, rect)| text == row && window.contains_rect(*rect)),
            "{row} shows without scrolling:\n{}",
            ax.dump()
        );
    }
    let populated = |ax: &Ax, variant: bool| -> Fallible<bool> {
        let project = ax.app.engine.project();
        let instance = project.instance(r1).ok_or("R1")?;
        let scope = if variant {
            project.variants.active()
        } else {
            None
        };
        Ok(sparksmith_core::instances::variants::population(instance, scope).is_populated())
    };
    ax.click(Role::CheckBox, "Populated in Lite")?;
    ax.settle();
    assert!(!populated(&ax, true)?, "left off in Lite:\n{}", ax.dump());
    assert!(populated(&ax, false)?, "still fitted by default");
    assert!(
        ax.find(Role::CheckBox, "Populated in Lite")?.toggled == Some(false),
        "the check follows"
    );
    ax.click(Role::CheckBox, "Populated by default")?;
    ax.settle();
    assert!(!populated(&ax, false)?, "left off by default");
    ax.command_key(Key::Z);
    ax.settle();
    assert!(populated(&ax, false)?, "one step takes the default back");
    assert!(!populated(&ax, true)?, "Lite still leaves it off");
    ax.command_key(Key::Z);
    ax.settle();
    assert!(populated(&ax, true)?, "and one more takes Lite back");
    Ok(())
}

/// The schematic Instance card shows its Assembly section right under
/// Properties, so a normal-height window shows it without scrolling.
#[test]
fn the_instance_card_shows_the_assembly_section_without_scrolling() -> Fallible {
    let mut b = board()?;
    lite_and_alt(&mut b)?;
    let r1 = b.r1;
    let mut ax = Ax::with_engine(b.engine, Workspace::Schematic)?;
    let project = ax.app.engine.project().clone();
    ax.app.schematic.select_instance(Some(r1), &project);
    ax.settle();
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, ax.window);
    for row in ["Populated by default", "Populated in Lite"] {
        assert!(
            ax.seen
                .iter()
                .any(|(text, rect)| text == row && window.contains_rect(*rect)),
            "{row} shows without scrolling:\n{}",
            ax.dump()
        );
    }
    ax.click(Role::CheckBox, "Populated in Lite")?;
    ax.settle();
    let project = ax.app.engine.project();
    let instance = project.instance(r1).ok_or("R1")?;
    assert!(
        !sparksmith_core::instances::variants::population(instance, project.variants.active())
            .is_populated(),
        "left off in Lite"
    );
    Ok(())
}

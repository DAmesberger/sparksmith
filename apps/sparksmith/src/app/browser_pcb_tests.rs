//! A part's row in the Browser, clicked while PCB is shown, selects the
//! part there (run 3, A11): the workspace stays and the Placement card is
//! the part's.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;

#[test]
fn a_part_row_in_pcb_selects_the_placement_there() -> Fallible {
    let mut ax = Ax::new(Workspace::Pcb)?;
    ax.app.pcb.set_3d(false);
    ax.settle();
    let r1 = ax
        .app
        .engine
        .project()
        .instance_by_designator("R1")
        .ok_or("no R1")?
        .id;
    ax.app.pick(browser::Pick::Instance(r1));
    ax.settle();
    assert_eq!(ax.app.workspace, Workspace::Pcb, "stays in PCB");
    assert_eq!(ax.app.pcb.selected_instance(), Some(r1));
    assert!(ax.has(Role::Label, "Placement") || ax.texts.iter().any(|t| t.starts_with("R1")));
    // In the schematic the same row still shows the symbol.
    ax.app.workspace = Workspace::Schematic;
    ax.app.pick(browser::Pick::Instance(r1));
    ax.settle();
    assert_eq!(ax.app.workspace, Workspace::Schematic);
    assert_eq!(ax.app.schematic.selected_instance(), Some(r1));
    Ok(())
}

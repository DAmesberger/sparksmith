//! A board-level replacement preview over two original, imported component lineages.
mod support;

use sparksmith_core::TerminalId;
use sparksmith_core::library::standards::Context;
use sparksmith_core::updates::{Alignment, Change, CopperIdentity, Replacement, TerminalPair};
use support::{Board, Footprint, Pad};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn preview(old: Vec<Pad>, new: Vec<Pad>) -> Fallible<CopperIdentity> {
    let _scope = Context::fixture()?.enter();
    let mut first = Footprint::new("J1", [6.0, 6.0, 0.0], false, old);
    first.name = "Synthetic:First".into();
    let mut second = Footprint::new("J2", [13.0, 6.0, 0.0], false, new);
    second.name = "Synthetic:Second".into();
    let board = Board {
        copper: 2,
        origin: None,
        setup: String::new(),
        footprints: vec![first, second],
        items: r#"(gr_rect (start 0 0) (end 30 20) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#.into(),
    };
    let imported = support::import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let old = project.instance_by_designator("J1").ok_or("missing J1")?;
    let new = project.instance_by_designator("J2").ok_or("missing J2")?;
    let plan = imported.engine.preview_update(Change::ReplaceComponent {
        instances: vec![old.id],
        replacement: Replacement {
            target: new.component,
            terminals: vec![TerminalPair {
                from: TerminalId(1),
                to: Some(TerminalId(1)),
            }],
            units: vec![],
            physical: None,
            representation: None,
            alignment: Alignment::KeepOrigin,
        },
    });
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    assert_eq!(plan.copper.len(), 1);
    Ok(plan
        .copper
        .into_iter()
        .next()
        .ok_or("missing copper review")?)
}

fn pin() -> Pad {
    Pad::smd("1", "rect", [0.0; 3], [1.0; 2])
}

#[test]
fn resizing_only_an_unnumbered_tab_changes_the_review() -> Fallible<()> {
    let pads = |width| vec![pin(), Pad::smd("", "rect", [0.0, 2.0, 0.0], [width; 2])];
    let copper = preview(pads(1.0), pads(2.0))?;
    assert!(!copper.identical);
    // The unchanged 1 × 1 mm mapped land is absent from the differences.
    // The unnumbered tab's display fallback is its pad ID.
    assert_eq!(copper.old_only, ["2"]);
    assert_eq!(copper.new_only, ["2"]);
    Ok(())
}

#[test]
fn changing_a_plated_bore_inside_the_same_two_mm_land_changes_the_review() -> Fallible<()> {
    let pad = |drill| vec![Pad::tht("1", "circle", [0.0; 3], [2.0; 2], drill)];
    for (before, after) in [
        ("0.5", "0.8"),
        ("0.5", "oval 0.5 0.8"),
        ("oval 0.5 0.8", "oval 0.8 0.5"),
    ] {
        let copper = preview(pad(before), pad(after))?;
        assert!(!copper.identical, "{before} to {after}");
        assert_eq!(copper.old_only, ["1"]);
        assert_eq!(copper.new_only, ["1"]);
    }
    Ok(())
}

#[test]
fn a_bare_hole_moving_relative_to_the_lands_changes_the_review() -> Fallible<()> {
    let pads = |x| vec![pin(), Pad::npth([x, 2.0, 0.0], 0.8)];
    let copper = preview(pads(0.0), pads(1.0))?;
    assert!(!copper.identical);
    assert_eq!(copper.old_only, ["2"]);
    assert_eq!(copper.new_only, ["2"]);
    Ok(())
}

#[test]
fn an_offset_bore_differs_even_when_the_outer_land_stays_put() -> Fallible<()> {
    // The imported pad's (at) is the drill centre. Its offset moves the land
    // back to the same board position as the centred pad.
    let old = Pad::tht("1", "circle", [0.0; 3], [2.0; 2], "0.5");
    let new = Pad::tht(
        "1",
        "circle",
        [0.25, 0.0, 0.0],
        [2.0; 2],
        "0.5 (offset -0.25 0)",
    );
    let copper = preview(vec![old], vec![new])?;
    assert!(!copper.identical);
    assert_eq!(copper.old_only, ["1"]);
    assert_eq!(copper.new_only, ["1"]);
    Ok(())
}

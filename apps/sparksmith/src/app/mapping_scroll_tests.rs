//! The Mapping tab through the whole app: a part with many terminals (the
//! ESP32-S31 module of the dev board example) lists every one, scrolled to
//! with the wheel, and a Locate brings its row into view.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;

fn esp32() -> Fallible<Engine> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    Ok(Engine::from_file(sparksmith_io::read_project(&path)?)?)
}

/// U1's component open on the Mapping tab, and its package terminals'
/// designations in order.
fn mapping() -> Fallible<(Ax, Vec<String>)> {
    let engine = esp32()?;
    let u1 = engine
        .project()
        .instance_by_designator("U1")
        .ok_or("no U1")?
        .clone();
    let package = sparksmith_core::instances::resolve(&engine.project().library, &u1)?
        .physical
        .ok_or("no package")?
        .package
        .definition
        .terminals
        .iter()
        .map(|p| p.designation.clone())
        .collect::<Vec<_>>();
    let mut ax = Ax::with_engine(engine, Workspace::Part)?;
    ax.app.pick(browser::Pick::EditComponent(u1.component.id));
    ax.settle();
    ax.app.part.on_tab("mapping");
    ax.settle();
    Ok((ax, package))
}

fn row(name: &str) -> String {
    format!("Package terminal {name}")
}

#[test]
fn every_terminal_of_a_large_part_is_reached_by_scrolling() -> Fallible {
    let (mut ax, terminals) = mapping()?;
    assert!(terminals.len() > 60, "{} terminals", terminals.len());
    let (first, last) = (
        terminals.first().ok_or("none")?,
        terminals.last().ok_or("none")?,
    );
    assert!(ax.has(Role::Button, &row(first)), "{}", ax.dump());
    assert!(!ax.has(Role::Button, &row(last)), "only the rows in view");
    let canvas = ax.find(Role::Button, &row(first))?.center();
    for _ in 0..200 {
        if ax.has(Role::Button, &row(last)) {
            break;
        }
        ax.frame(vec![
            egui::Event::PointerMoved(canvas),
            egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -200.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
    }
    ax.settle();
    let shown = ax.find(Role::Button, &row(last))?;
    assert!(
        shown.rect.bottom() <= ax.window.y,
        "the last row is on screen: {:?}",
        shown.rect
    );
    assert!(
        !ax.has(Role::Button, &row(first)),
        "scrolled past the first"
    );
    Ok(())
}

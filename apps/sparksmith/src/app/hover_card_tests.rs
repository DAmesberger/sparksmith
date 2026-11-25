//! The hover card through the whole app's frame: the same part, said the same
//! way on the sheet, the board and in PCB 3D, for the eye and for a screen
//! reader, off what it is about.
use super::ax_harness::{Ax, Fallible};
use super::*;
use crate::about::{Subject, about};
use egui::Role;
use sparksmith_core::Id;

fn part(ax: &Ax, designator: &str) -> Fallible<Id> {
    let project = ax.app.engine.project();
    let part = project.instance_by_designator(designator);
    Ok(part.ok_or("no such part")?.id)
}

/// The one card shown says all of `subject`, and is not at `at`.
fn says(ax: &Ax, subject: Subject, at: egui::Pos2) -> Fallible {
    let card = about(ax.app.engine.project(), subject).ok_or("nothing to say")?;
    let node = ax.find(Role::Tooltip, &card.spoken())?;
    assert!(!node.rect.contains(at), "{subject:?}");
    assert!(ax.texts.contains(&card.title), "{:?}", ax.texts);
    Ok(())
}

fn cards(ax: &Ax) -> usize {
    ax.all_prefix(Role::Tooltip, "").len()
}

/// On the sheet: R1's body, one of its pins and the wire, each in the frame
/// the pointer gets there; nothing on empty sheet or while wiring.
#[test]
fn the_sheet_says_what_is_under_the_pointer() -> Fallible {
    let mut ax = Ax::new(Workspace::Schematic)?;
    let r1 = part(&ax, "R1")?;
    let body = ax.app.schematic.screen_point([-12.0, 0.0]);
    ax.hover(body);
    says(&ax, Subject::Part(r1), body)?;

    let project = ax.app.engine.project();
    let wire = project.wires.first().ok_or("the demo has a wire")?.clone();
    let (a, b) = sparksmith_core::schematic::wire_ends(project, &wire).ok_or("ends")?;
    let pin = wire.from.terminal().ok_or("it starts at a pin")?;
    let end = ax.app.schematic.screen_point(a);
    ax.hover(end);
    says(&ax, Subject::Pin(pin), end)?;

    let middle = sparksmith_core::schematic::nearest_on_wire(
        ax.app.engine.project(),
        wire.id,
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0],
    )
    .ok_or("on the wire")?;
    let at = ax.app.schematic.screen_point(middle);
    ax.hover(at);
    let net = ax
        .app
        .engine
        .project()
        .net_of(pin)
        .map(|n| n.id)
        .ok_or("its net")?;
    let subject = Subject::Wire {
        wire: wire.id,
        net: Some(net),
    };
    says(&ax, subject, at)?;

    ax.hover(ax.app.schematic.clear_rect().left_top() + egui::vec2(3.0, 3.0));
    assert_eq!(cards(&ax), 0, "{}", ax.dump());
    ax.hover(body);
    assert_eq!(cards(&ax), 1);
    ax.key(egui::Key::Escape);
    assert_eq!(cards(&ax), 0, "Esc");
    ax.hover(end);
    assert_eq!(cards(&ax), 1, "the next thing has its own");
    ax.key(egui::Key::W);
    ax.hover(body);
    assert_eq!(cards(&ax), 0, "wiring");
    Ok(())
}

/// In Layout and PCB 3D the part reads as on the sheet. A pressed button and
/// an open menu close the card.
#[test]
fn the_board_and_3d_say_the_same_of_a_part() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut ax = Ax::with_engine(Engine::showcase()?, Workspace::Pcb)?;
    let canvas = ax.find(Role::Canvas, "Board")?;
    // From the middle of the board outwards.
    let centre = canvas.rect.center();
    let mut points: Vec<egui::Pos2> = (0..80 * 60)
        .map(|k| {
            let cell = egui::vec2((k % 80) as f32 - 40.0, (k / 80) as f32 - 30.0);
            centre + cell * 14.0
        })
        .filter(|p| canvas.rect.contains(*p))
        .collect();
    points.sort_by(|a, b| a.distance(centre).total_cmp(&b.distance(centre)));
    let found = points.into_iter().find_map(|at| {
        ax.hover(at);
        let card = ax.all_prefix(Role::Tooltip, "U").first()?.name.clone();
        Some((at, card))
    });
    let (at, layout) = found.ok_or("no part under any point of the board")?;
    ax.frame(vec![egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(cards(&ax), 0, "pressed");
    ax.frame(vec![egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    ax.right_click_at(at);
    assert_eq!(cards(&ax), 0, "a menu is open");
    ax.key(egui::Key::Escape);
    assert!(layout.lines().count() > 1, "{layout:?}");

    ax.button("3D")?;
    ax.until("the board's model", |ax| {
        let project = Arc::clone(ax.app.engine.project_arc());
        ax.app
            .scene
            .display(&crate::scene::ModelKey::Project(project))
            == crate::scene::Display::Current
    })?;
    ax.settle();
    // A point of U1's body, not of its pads.
    let u1 = part(&ax, "U1")?;
    let rect = ax.app.scene.viewport.rect().ok_or("no 3D view")?;
    let at = (0..60 * 60)
        .map(|k| {
            let cell = egui::vec2((k % 60) as f32 / 60.0, (k / 60) as f32 / 60.0);
            rect.min + rect.size() * cell
        })
        .find(|p| {
            let hit = ax.app.scene.viewport.surface_hit(*p);
            hit.and_then(|hit| hit.item)
                .and_then(|i| ax.app.scene.source(i))
                .is_some_and(|s| s.package_instance() == Some(u1))
        })
        .ok_or("U1 is not on screen")?;
    ax.hover(at);
    says(&ax, Subject::Part(u1), at)?;
    let requests = ax.app.scene.requests();
    ax.settle();
    assert_eq!(ax.app.scene.requests(), requests, "hover asks for no model");
    ax.key(egui::Key::Escape);
    assert_eq!(cards(&ax), 0, "Esc");
    Ok(())
}

/// A drawn dimension's label says what it measures between; with the
/// Dimension tool in hand it says nothing.
#[test]
fn a_drawn_dimension_says_what_it_measures() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let engine = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    let mut ax = Ax::with_engine(engine, Workspace::Pcb)?;
    let j1 = part(&ax, "J1")?;
    ax.app.pcb.set_selection(Some(j1), None, None);
    ax.settle();
    let drawn = ax.app.pcb.dimensions_drawn();
    let (_, text, rect, _) = drawn.first().ok_or("J1's dimension is drawn")?.clone();
    ax.hover(rect.center());
    let card = ax.find_prefix(Role::Tooltip, &text)?;
    assert!(card.name.contains(" to "), "{:?}", card.name);
    assert!(!card.rect.intersects(rect));
    let ctx = ax.ctx.clone();
    ax.app.run_command(&ctx, "pcb.dimension");
    ax.settle();
    ax.hover(rect.center());
    assert_eq!(cards(&ax), 0, "a tool is armed");
    Ok(())
}

/// Pictures of the board with a part's card, in both themes, for a person
/// to look at (`SPARKSMITH_PICTURES`).
#[test]
fn the_card_is_drawn_in_both_themes() -> Fallible {
    let Some(dir) = std::env::var_os("SPARKSMITH_PICTURES") else {
        return Ok(());
    };
    std::fs::create_dir_all(&dir)?;
    let engine = Engine::showcase()?;
    let project = engine.project();
    let u1 = project.instance_by_designator("U1").ok_or("U1")?.id;
    for dark in [false, true] {
        sparksmith_ui::theme::set_dark(dark);
        for (name, subject) in [
            ("part", Subject::Part(u1)),
            ("via", Subject::Via(project.vias.first().ok_or("a via")?.id)),
            (
                "track",
                Subject::Track(project.tracks.first().ok_or("a track")?.id),
            ),
        ] {
            let card = about(project, subject).ok_or("a card")?;
            let picture = crate::pcb_editor::raster_tests::picture_of(
                [460.0, 260.0],
                sparksmith_ui::theme::Tokens::get().viewport_bottom,
                |ui| {
                    let room = ui.max_rect();
                    let thing =
                        egui::Rect::from_min_size(egui::pos2(30.0, 90.0), egui::vec2(60.0, 60.0));
                    ui.painter()
                        .rect_filled(thing, 2.0, egui::Color32::from_rgb(150, 110, 60));
                    let over = crate::kit::hover_card::Over::Rect(thing);
                    crate::kit::hover_card::show(ui, &card, &over, Some(thing.center()), room);
                },
            );
            let file = format!("hover-{name}-{}.png", if dark { "dark" } else { "light" });
            let png = picture.png().ok_or("no picture")?;
            std::fs::write(std::path::Path::new(&dir).join(file), png)?;
        }
    }
    sparksmith_ui::theme::set_dark(false);
    Ok(())
}

/// Light and dark: the card's inks read on its panel (WCAG 2 AA for text).
#[test]
fn the_card_reads_in_both_themes() {
    use sparksmith_ui::theme::Tokens;
    for t in [Tokens::light(), Tokens::dark()] {
        for ink in [t.text, t.text_dim] {
            let light = |c: egui::Color32| {
                let [r, g, b, _] = egui::Rgba::from(c).to_array();
                0.2126 * r + 0.7152 * g + 0.0722 * b + 0.05
            };
            let (a, b) = (light(ink), light(t.panel));
            let ratio = a.max(b) / a.min(b);
            assert!(ratio >= 4.5, "{ink:?} on {:?}: {ratio}", t.panel);
        }
    }
}

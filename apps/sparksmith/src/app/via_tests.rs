//! One via through the whole app: dragged by a press on it (no selection
//! rectangle), refused while locked, and placed by its card's Position.
use super::ax_harness::{Ax, Fallible};
use super::*;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton};
use sparksmith_core::layout::{LayoutCommand, ViaRequest};
use sparksmith_core::model::ViaSpan;
use sparksmith_core::{Id, Point};

/// The demo with a lone via of the first net at (30, 20), in Layout.
fn board() -> Fallible<(Ax, Id)> {
    let mut engine = Engine::demo()?;
    let net = engine.project().nets.first().ok_or("no net")?.id;
    let outcome = engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::AddVia {
            via: ViaRequest {
                net,
                position: [30.0, 20.0],
                drill: 0.3,
                diameter: 0.6,
                span: ViaSpan::Through,
                tented: true,
                fill: Default::default(),
            },
        }),
    )?;
    let sparksmith_core::Outcome::ViaAdded { via } = outcome else {
        return Err("no via".into());
    };
    let mut ax = Ax::with_engine(engine, Workspace::Pcb)?;
    ax.app.pcb.set_3d(false);
    ax.settle();
    Ok((ax, via))
}

fn at(ax: &Ax, p: Point) -> Fallible<egui::Pos2> {
    let board = ax.find(Role::Canvas, "Board")?;
    Ok(ax
        .app
        .pcb
        .shown_at(&ax.ctx, board.rect, ax.app.engine.project(), p))
}

fn place(ax: &Ax, via: Id) -> Fallible<Point> {
    Ok(ax.app.engine.project().via(via).ok_or("no via")?.position)
}

fn drag(ax: &mut Ax, from: Point, to: Point) -> Fallible {
    let (from, to) = (at(ax, from)?, at(ax, to)?);
    let press = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    ax.frame(vec![Event::PointerMoved(from)]);
    ax.frame(vec![press(from, true)]);
    ax.frame(vec![Event::PointerMoved(from + (to - from) * 0.25)]);
    ax.frame(vec![Event::PointerMoved(to)]);
    ax.frame(vec![press(to, false)]);
    ax.settle();
    Ok(())
}

#[test]
fn a_via_is_dragged_and_a_locked_one_says_so() -> Fallible {
    let (mut ax, via) = board()?;
    let revision = ax.app.engine.revision();
    drag(&mut ax, [30.0, 20.0], [32.0, 21.0])?;
    assert_eq!(place(&ax, via)?, [32.0, 21.0]);
    assert_eq!(ax.app.engine.revision(), revision + 1, "one step");
    assert_eq!(ax.app.pcb.selected().vias.len(), 1, "it is selected");
    ax.command_key(Key::Z);
    ax.settle();
    assert_eq!(place(&ax, via)?, [30.0, 20.0]);
    ax.click(Role::CheckBox, "Locked")?;
    ax.settle();
    drag(&mut ax, [30.0, 20.0], [33.0, 20.0])?;
    assert_eq!(place(&ax, via)?, [30.0, 20.0]);
    assert!(
        ax.app
            .toasts
            .items()
            .any(|t| t.text.contains("The via is locked")),
        "{:?}",
        ax.texts
    );
    Ok(())
}

#[test]
fn a_via_is_placed_by_its_typed_position() -> Fallible {
    let (mut ax, via) = board()?;
    let p = at(&ax, [30.0, 20.0])?;
    ax.click_at(p);
    ax.settle();
    let field = ax
        .all(Role::TextInput, "Position")
        .first()
        .map(|n| n.center())
        .ok_or_else(|| format!("no Position:\n{}", ax.dump()))?;
    ax.click_at(field);
    ax.command_key(Key::A);
    ax.type_text("27.5");
    ax.key(Key::Enter);
    ax.settle();
    assert_eq!(place(&ax, via)?, [27.5, 20.0]);
    Ok(())
}

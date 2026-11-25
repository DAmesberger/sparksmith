//! The card on the Layout canvas: beside the hovered thing in the frame it is
//! hovered, gone when a tool, a press, Esc or another place says so.
use super::*;
use crate::pcb_editor::board_view;
use egui::{Event, Key, Modifiers, PointerButton};
use sparksmith_ui::fonts;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const SCREEN: egui::Vec2 = egui::vec2(900.0, 700.0);

struct Bench {
    engine: Engine,
    ctx: egui::Context,
    editor: PcbEditor,
    view: BoardView,
    texts: Vec<(String, Rect)>,
}

impl Bench {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let engine = Engine::showcase()?;
        let ctx = egui::Context::default();
        fonts::setup_bundled_fonts(&ctx);
        let rect = Rect::from_min_size(Pos2::ZERO, SCREEN);
        let view = board_view(
            rect,
            engine.project().board_size,
            1.0,
            egui::Vec2::ZERO,
            false,
        );
        let mut bench = Self {
            engine,
            ctx,
            editor: PcbEditor::default(),
            view,
            texts: Vec::new(),
        };
        bench.frame(vec![]);
        bench.frame(vec![]);
        Ok(bench)
    }

    fn frame(&mut self, events: Vec<Event>) {
        let Self {
            engine,
            ctx,
            editor,
            view,
            ..
        } = self;
        let mut at = Rect::NOTHING;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
                events,
                ..Default::default()
            },
            |ui| {
                at = ui.available_rect_before_wrap();
                let _ = editor.canvas(ui, engine);
            },
        );
        *view = board_view(
            at,
            engine.project().board_size,
            editor.zoom,
            editor.pan,
            editor.from_bottom,
        );
        out.textures_delta.clear();
        self.texts = crate::kit::text::tests::painted(&out)
            .into_iter()
            .map(|(text, rect, ..)| (text, rect))
            .collect();
    }

    fn hover(&mut self, at: Point) {
        let pos = self.view.screen(at);
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    /// The card's title, when a card is shown.
    fn shows(&self, title: &str) -> bool {
        self.texts.iter().any(|(t, _)| t == title)
    }

    fn title(&self, subject: Subject) -> String {
        crate::about::about(self.engine.project(), subject).map_or_else(String::new, |c| c.title)
    }

    /// A via, where it is.
    fn via(&self) -> Result<(Id, Point), String> {
        let via = self.engine.project().vias.first().ok_or("no via")?;
        Ok((via.id, via.position))
    }
}

/// Every part, pad, via and track that a click would pick says what it is,
/// at once and off the thing.
#[test]
fn the_card_is_what_a_click_would_pick() -> TestResult {
    let mut b = Bench::new()?;
    let project = b.engine.project().clone();
    let places: Vec<Point> = sparksmith_core::layout::placed_pads(&project)
        .iter()
        .map(|p| p.center())
        .chain(project.placements.iter().map(|p| p.position))
        .chain(project.vias.iter().map(|v| v.position))
        .chain(
            project
                .tracks
                .iter()
                .map(|t| [(t.start[0] + t.end[0]) / 2.0, (t.start[1] + t.end[1]) / 2.0]),
        )
        .collect();
    let mut kinds = std::collections::BTreeSet::new();
    for at in places {
        b.hover(at);
        let Some((_, Some((subject, place)))) = b.editor.hover_card.found.clone() else {
            continue;
        };
        kinds.insert(match subject {
            Subject::Part(_) => "part",
            Subject::Pad { .. } => "pad",
            Subject::Via(_) => "via",
            Subject::Track(_) => "track",
            _ => "other",
        });
        let title = b.title(subject);
        let Some((_, rect)) = b.texts.iter().find(|(t, _)| *t == title) else {
            return Err(format!("{subject:?} at {at:?}: no card {title:?}").into());
        };
        assert!(!rect.contains(b.view.screen(at)), "{subject:?}");
        if let Place::Corners(points) = place {
            let thing = Over::bounds(points.into_iter().map(|p| b.view.screen(p)));
            if let Some(Over::Rect(thing)) = thing {
                let fills = thing.width() > SCREEN.x / 2.0 || thing.height() > SCREEN.y / 2.0;
                assert!(fills || !thing.intersects(*rect), "{subject:?}");
            }
        }
    }
    for kind in ["pad", "via", "track"] {
        assert!(kinds.contains(kind), "{kinds:?}");
    }
    Ok(())
}

/// The same size at every zoom.
#[test]
fn the_card_is_the_same_size_at_every_zoom() -> TestResult {
    let mut b = Bench::new()?;
    let (id, at) = b.via()?;
    let title = b.title(Subject::Via(id));
    let mut sizes = Vec::new();
    for zoom in [1.0, 4.0] {
        b.editor.zoom = zoom;
        b.frame(vec![]);
        // Bring the via into view.
        let off = b.view.screen(at) - Pos2::ZERO - SCREEN / 2.0;
        b.editor.pan -= off;
        b.frame(vec![]);
        b.hover(at);
        let found = b.texts.iter().find(|(t, _)| *t == title);
        sizes.push(found.ok_or("no card")?.1.size());
    }
    assert_eq!(sizes[0], sizes[1]);
    Ok(())
}

/// Esc closes it and leaves the key to its command; the next thing has its
/// own. A press, a tool and leaving the board close it.
#[test]
fn the_card_closes_on_esc_press_tool_and_leaving() -> TestResult {
    let mut b = Bench::new()?;
    let (id, at) = b.via()?;
    let title = b.title(Subject::Via(id));
    b.hover(at);
    assert!(b.shows(&title));
    b.editor.highlighted = None;
    b.frame(vec![Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(!b.shows(&title), "Esc");
    b.frame(vec![]);
    assert!(!b.shows(&title), "it stays closed on the same thing");
    b.frame(vec![Event::PointerGone]);
    b.hover(at);
    assert!(b.shows(&title), "hovered again");
    let pos = b.view.screen(at);
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    b.frame(vec![button(true)]);
    assert!(!b.shows(&title), "pressed");
    b.frame(vec![button(false)]);
    b.frame(vec![]);
    assert!(b.shows(&title), "released");
    b.editor.tool = Tool::Route;
    b.frame(vec![]);
    assert!(!b.shows(&title), "a tool");
    b.editor.tool = Tool::Select;
    b.frame(vec![]);
    assert!(b.shows(&title));
    b.frame(vec![Event::PointerGone]);
    assert!(!b.shows(&title), "left");
    Ok(())
}

/// With the keyboard on the board and no pointer, the selected thing's card
/// shows; the card takes no focus of its own.
#[test]
fn the_keyboards_board_shows_the_selected_things_card() -> TestResult {
    let mut b = Bench::new()?;
    let (id, _) = b.via()?;
    let title = b.title(Subject::Via(id));
    b.editor.select_via(id, None);
    b.frame(vec![Event::PointerGone]);
    assert!(!b.shows(&title), "not without the keyboard");
    b.frame(vec![Event::Key {
        key: Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    b.frame(vec![]);
    let focus = b.ctx.memory(|m| m.focused());
    assert!(focus.is_some(), "the board takes the keyboard");
    assert!(b.shows(&title));
    assert_ne!(focus, Some(egui::Id::unique("hover-card")));
    Ok(())
}

/// Nothing is looked for again while nothing changes: the found object is
/// kept for its place, and one move finds one.
#[test]
fn an_unchanged_frame_looks_for_nothing() -> TestResult {
    let mut b = Bench::new()?;
    let (_, at) = b.via()?;
    b.hover(at);
    let Some((was, _)) = b.editor.hover_card.found.take() else {
        return Err("nothing found".into());
    };
    // A marker no search would make: kept, so no search ran.
    b.editor.hover_card.found = Some((was, None));
    b.frame(vec![]);
    assert!(matches!(b.editor.hover_card.found, Some((_, None))));
    b.hover([at[0] + 0.01, at[1]]);
    assert!(matches!(b.editor.hover_card.found, Some((_, Some(_)))));
    Ok(())
}

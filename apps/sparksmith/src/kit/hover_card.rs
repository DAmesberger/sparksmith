//! The hover card: what the thing under the pointer is, beside it. It is
//! drawn the frame the thing is hovered, in points (the same size at every
//! zoom), over everything and never over the thing or the pointer. It is
//! paint and one accessibility node: no pointer, key or focus goes to it.
use super::text;
use egui::{Id, LayerId, Order, Pos2, Rect, Role, Sense, Ui, Vec2, pos2, vec2};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::type_face;

/// The widest a card's text gets, points.
const WIDTH: f32 = 260.0;
/// Between the card and the thing, points.
const GAP: f32 = 8.0;
/// Room kept round the pointer, points.
const POINTER: f32 = 14.0;
/// Lines of one text at most: a long description ends in "…"; Properties and
/// the accessibility node have it all.
const LINES: usize = 4;

/// What a card says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Content {
    pub title: String,
    /// What the thing is, in words.
    pub words: Option<String>,
    pub rows: Vec<(&'static str, String)>,
    /// Sentences under the rows.
    pub notes: Vec<String>,
}

impl Content {
    pub fn new(title: String, rows: Vec<(&'static str, String)>) -> Self {
        Self {
            title,
            words: None,
            rows,
            notes: Vec::new(),
        }
    }

    /// All of it, for a screen reader.
    pub fn spoken(&self) -> String {
        std::iter::once(self.title.clone())
            .chain(self.words.clone())
            .chain(
                self.rows
                    .iter()
                    .map(|(label, value)| format!("{label} {value}")),
            )
            .chain(self.notes.iter().cloned())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Where the hovered thing is on screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Over {
    Rect(Rect),
    /// A track or wire: its path.
    Path(Vec<Pos2>),
}

impl Over {
    /// The bounds of `points`.
    pub fn bounds(points: impl IntoIterator<Item = Pos2>) -> Option<Self> {
        let rect = points
            .into_iter()
            .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(p, p)));
        rect.is_finite().then_some(Self::Rect(rect))
    }

    /// Whether a card at `card` lies on it.
    fn under(&self, card: Rect) -> bool {
        match self {
            Self::Rect(r) => r.intersects(card),
            Self::Path(points) => points.windows(2).any(|w| match w {
                [a, b] => crosses(card, *a, *b),
                _ => false,
            }),
        }
    }

    fn centre(&self) -> Pos2 {
        match self {
            Self::Rect(r) => r.center(),
            Self::Path(points) => points.get(points.len() / 2).copied().unwrap_or_default(),
        }
    }
}

/// Whether the segment from `a` to `b` meets `rect` (Liang–Barsky).
fn crosses(rect: Rect, a: Pos2, b: Pos2) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, q) in [
        (-d.x, a.x - rect.left()),
        (d.x, rect.right() - a.x),
        (-d.y, a.y - rect.top()),
        (d.y, rect.bottom() - a.y),
    ] {
        if p.abs() < f32::EPSILON {
            if q < 0.0 {
                return false;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    t0 <= t1
}

/// Where a card of `size` goes: beside the thing, inside `room`, over
/// neither the thing nor the pointer. A thing that fills the room leaves no
/// such place: then the corner farthest from the pointer.
pub fn place(size: Vec2, over: &Over, pointer: Option<Pos2>, room: Rect) -> Rect {
    let at = pointer.unwrap_or_else(|| over.centre());
    let fit = |min: Pos2| {
        let max = (room.max - size).to_vec2();
        let low = room.min.to_vec2();
        let v = min.to_vec2();
        Rect::from_min_size(
            pos2(v.x.min(max.x).max(low.x), v.y.min(max.y).max(low.y)),
            size,
        )
    };
    let clear = |card: &Rect| {
        !over.under(card.expand(1.0))
            && pointer.is_none_or(|p| !card.expand(POINTER / 2.0).contains(p))
    };
    let beside = |r: Rect| {
        let (x, y) = (at.x - size.x / 2.0, at.y - size.y / 2.0);
        [
            pos2(r.right() + GAP, y),
            pos2(r.left() - GAP - size.x, y),
            pos2(x, r.bottom() + GAP),
            pos2(x, r.top() - GAP - size.y),
        ]
    };
    let mut tries: Vec<Pos2> = Vec::new();
    match over {
        Over::Rect(r) => tries.extend(beside(*r)),
        Over::Path(_) => {
            let near = Rect::from_center_size(at, Vec2::splat(2.0 * POINTER));
            tries.extend([
                near.right_bottom(),
                pos2(near.right(), near.top() - size.y),
                pos2(near.left() - size.x, near.bottom()),
                near.left_top() - size,
            ]);
            tries.extend(beside(near));
        }
    }
    let mut corners = [
        room.left_top(),
        room.right_top() - vec2(size.x, 0.0),
        room.left_bottom() - vec2(0.0, size.y),
        room.right_bottom() - size,
    ];
    corners.sort_by(|a, b| {
        let far = |c: &Pos2| (*c + size / 2.0).distance(at);
        far(b).total_cmp(&far(a))
    });
    tries
        .into_iter()
        .chain(corners)
        .map(fit)
        .find(clear)
        .unwrap_or_else(|| fit(corners[0]))
}

/// Draw `content` beside `over`, and say it to screen readers. Returns
/// where it is.
pub fn show(ui: &Ui, content: &Content, over: &Over, pointer: Option<Pos2>, room: Rect) -> Rect {
    let t = Tokens::get();
    let body = egui::FontId::proportional(t.font_small);
    let width = WIDTH.min(room.width() - 2.0 * t.space_8).max(1.0);
    // Each line: its text, where it starts in the card, its ink.
    let mut lines = Vec::new();
    let mut y = 0.0;
    let mut wide = 0.0_f32;
    let mut line = |galley: std::sync::Arc<egui::Galley>, x: f32, ink, advance: bool| {
        wide = wide.max(x + galley.size().x);
        let height = galley.size().y;
        lines.push((galley, vec2(x, y), ink));
        if advance {
            y += height + t.space_2;
        }
    };
    let heading = type_face::medium(t.font_label);
    line(
        text::wrapped(ui, &content.title, heading, t.text, width, LINES),
        0.0,
        t.text,
        true,
    );
    if let Some(words) = &content.words {
        line(
            text::wrapped(ui, words, body.clone(), t.text_dim, width, LINES),
            0.0,
            t.text_dim,
            true,
        );
    }
    let labels: Vec<_> = content
        .rows
        .iter()
        .map(|(label, _)| text::one_line(ui, label, body.clone(), t.text_dim, width / 2.0))
        .collect();
    let column = labels.iter().map(|g| g.size().x).fold(0.0, f32::max) + t.space_8;
    for (label, (_, value)) in labels.into_iter().zip(&content.rows) {
        line(label, 0.0, t.text_dim, false);
        let room = (width - column).max(1.0);
        line(
            text::wrapped(ui, value, body.clone(), t.text, room, LINES),
            column,
            t.text,
            true,
        );
    }
    for note in &content.notes {
        line(
            text::wrapped(ui, note, body.clone(), t.text_dim, width, LINES),
            0.0,
            t.text_dim,
            true,
        );
    }
    let inset = vec2(t.space_8, t.space_6);
    let size = vec2(wide, y - t.space_2) + 2.0 * inset;
    let card = place(size, over, pointer, room);
    let id = Id::unique("hover-card");
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, id))
        .with_clip_rect(room);
    t.container_rect(&painter, card, t.radius_sm, t.panel, t.border);
    for (galley, at, ink) in lines {
        painter.galley(card.min + inset + at, galley, ink);
    }
    let response = ui.interact(card, id, Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(Role::Tooltip, true, content.spoken()));
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(Role::Tooltip);
        node.set_live(egui::accesskit::Live::Polite);
    });
    card
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: Rect = Rect {
        min: Pos2::ZERO,
        max: Pos2::new(800.0, 600.0),
    };
    const SIZE: Vec2 = Vec2::new(180.0, 90.0);

    /// Wherever the thing and the pointer on it are, the card is in the
    /// room and on neither, as long as there is such a place.
    #[test]
    fn a_card_is_beside_the_thing_and_in_the_room() {
        for half in [2.0, 15.0, 80.0] {
            for x in (0..=16).map(|i| i as f32 * 50.0) {
                for y in (0..=12).map(|i| i as f32 * 50.0) {
                    let pointer = pos2(x, y);
                    let thing = Rect::from_center_size(pointer, Vec2::splat(2.0 * half));
                    let card = place(SIZE, &Over::Rect(thing), Some(pointer), ROOM);
                    assert!(ROOM.contains_rect(card), "{pointer:?} {half}: {card:?}");
                    assert!(!card.intersects(thing), "{pointer:?} {half}: {card:?}");
                }
            }
        }
    }

    /// Along a track or wire at any angle, the card lies off all of it.
    #[test]
    fn a_card_is_off_the_path_it_is_about() {
        for k in 0..16 {
            let (s, c) = (k as f32 * std::f32::consts::PI / 16.0).sin_cos();
            let reach = vec2(c, s) * 900.0;
            let pointer = pos2(400.0, 300.0);
            let path = vec![pointer - reach, pointer + reach];
            let card = place(SIZE, &Over::Path(path.clone()), Some(pointer), ROOM);
            assert!(ROOM.contains_rect(card));
            assert!(!crosses(card, path[0], path[1]), "{k}: {card:?}");
            assert!(!card.contains(pointer));
        }
    }

    /// A thing that fills the view leaves the far corner, and never the
    /// pointer's place.
    #[test]
    fn a_thing_that_fills_the_room_gets_the_far_corner() {
        let pointer = pos2(700.0, 500.0);
        let card = place(SIZE, &Over::Rect(ROOM.expand(50.0)), Some(pointer), ROOM);
        assert_eq!(card.min, ROOM.min);
        assert!(!card.contains(pointer));
    }
}

//! Constraint glyphs of a sketch on a canvas, drawn the same in the
//! footprint sketch and the board shape: a small icon beside the curve or
//! point a constraint holds, stepped along where glyphs or labels would
//! overlap. Which constraints show is the caller's choice; the rectangles
//! drawn come back for picking on the next frame.
//!
//! The icons and the look follow Solidcraft's `sketch_tools.rs` and
//! `icons/markers.rs` (pinned revision; see ATTRIBUTION.md).
use egui::{Align2, Pos2, Rect, Stroke, Vec2, vec2};
use sparksmith_core::Point;
use sparksmith_core::library::footprint_source::SketchConstraintId;
use sparksmith_solidcraft::footprint::view::{SketchView, ViewConstraint, ViewGeometry};
use sparksmith_ui::icons;
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::type_face;

/// Constraint glyph size, in pixels (Solidcraft's `GLYPH`).
const GLYPH: f32 = 15.0;
/// Most glyphs drawn in one frame.
const MAX_GLYPHS: usize = 512;

/// A drawn glyph or label and the constraint it is of.
pub type Placed = (SketchConstraintId, Rect);

/// How the glyphs' constraints stand.
#[derive(Clone, Copy, Debug, Default)]
pub struct State<'a> {
    pub conflicting: &'a [SketchConstraintId],
    pub redundant: &'a [SketchConstraintId],
    pub selected: &'a [SketchConstraintId],
    pub hover: Option<Pos2>,
}

/// Solidcraft's glyph icon for a constraint type (`icons/markers.rs`).
fn icon(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "Coincident" => "c_coincident",
        "Parallel" => "c_parallel",
        "Perpendicular" => "c_perpendicular",
        "Collinear" => "c_collinear",
        "Tangent" => "c_tangent",
        "Equal" => "c_equal",
        "Concentric" => "c_concentric",
        "MidPoint" => "c_midpoint",
        "Symmetry" => "c_symmetry",
        "Fix" => "c_fix",
        _ => return None,
    })
}

/// Solidcraft's glyph letter where there is no icon.
fn letter(kind: &str) -> &'static str {
    match kind {
        "Horizontal" => "-",
        "Vertical" => "|",
        _ => "?",
    }
}

/// Where a curve's glyph goes: its middle on screen, the direction off the
/// curve, and how long the curve is there.
fn base(
    screen: &dyn Fn(Point) -> Pos2,
    geometry: &ViewGeometry,
    line: &[Point],
) -> Option<(Pos2, Vec2, f32)> {
    let (first, last) = (line.first()?, line.last()?);
    let mid = line.get(line.len() / 2)?;
    let along = screen(*last) - screen(*first);
    let length = along.length();
    let normal = if length > 1e-3 {
        vec2(-along.y, along.x) / length
    } else {
        vec2(0.0, -1.0)
    };
    let mid = match geometry {
        ViewGeometry::Line { a, b } => screen([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]),
        _ => screen(*mid),
    };
    Some((mid, normal, length.max(GLYPH * 2.0)))
}

/// The canvas glyphs are drawn on: what of it shows, and the sketch on it.
pub struct Canvas<'a> {
    pub painter: &'a egui::Painter,
    pub rect: Rect,
    pub screen: &'a dyn Fn(Point) -> Pos2,
    /// Samples a curve.
    pub polyline: &'a dyn Fn(&ViewGeometry) -> Vec<Point>,
    pub sketch: &'a SketchView,
}

/// Draw the glyphs of `shown` that fall on the canvas, clear of `labels`.
/// Returns where each was drawn.
pub fn paint<'a>(
    canvas: &Canvas<'_>,
    shown: impl Iterator<Item = &'a ViewConstraint>,
    labels: &[Placed],
    state: State<'_>,
) -> Vec<Placed> {
    let Canvas {
        painter,
        rect,
        screen,
        polyline,
        sketch,
    } = *canvas;
    let t = Tokens::get();
    let mut out: Vec<Placed> = Vec::new();
    let mut taken: Vec<Rect> = labels.iter().map(|(_, r)| r.expand(2.0)).collect();
    for constraint in shown {
        if out.len() >= MAX_GLYPHS {
            break;
        }
        let at = match (constraint.curves.first(), constraint.points.first()) {
            (Some(c), _) => sketch
                .curve(c)
                .and_then(|c| base(screen, &c.geometry, &polyline(&c.geometry)))
                .filter(|(_, _, length)| *length >= GLYPH * 1.5),
            (None, Some(p)) => sketch
                .point(p)
                .map(|p| (screen(p.at), vec2(0.7, -0.7), GLYPH * 2.0)),
            _ => None,
        };
        let Some((at, normal, _)) = at else {
            continue;
        };
        let along = vec2(normal.y, -normal.x);
        let mut at = at + normal * GLYPH;
        for _ in 0..8 {
            let r = Rect::from_center_size(at, vec2(GLYPH, GLYPH));
            if !taken.iter().any(|q| q.intersects(r)) {
                break;
            }
            at += along * (GLYPH + 2.0);
        }
        let r = Rect::from_center_size(at, vec2(GLYPH, GLYPH));
        if !rect.intersects(r) {
            continue;
        }
        let failing = state.conflicting.contains(&constraint.id);
        let lit =
            state.selected.contains(&constraint.id) || state.hover.is_some_and(|h| r.contains(h));
        let ink = if failing {
            t.sketch_conflict
        } else if lit {
            t.accent
        } else if state.redundant.contains(&constraint.id) {
            t.glyph_text.gamma_multiply(0.45)
        } else {
            t.glyph_text
        };
        painter.rect_filled(
            r,
            t.radius_sm.saturating_sub(1),
            if lit { t.accent_soft } else { t.glyph_bg },
        );
        painter.rect_stroke(
            r,
            t.radius_sm.saturating_sub(1),
            Stroke::new(1.0, if failing || lit { ink } else { t.glyph_edge }),
            egui::StrokeKind::Inside,
        );
        match icon(constraint.kind) {
            Some(icon) => {
                icons::paint_icon(painter, r.shrink(2.0), icon, f32::from(ink.a()) / 255.0);
            }
            None => {
                painter.text(
                    at,
                    Align2::CENTER_CENTER,
                    letter(constraint.kind),
                    type_face::semibold(t.font_small),
                    ink,
                );
            }
        }
        taken.push(r.expand(1.0));
        out.push((constraint.id.clone(), r));
    }
    out
}

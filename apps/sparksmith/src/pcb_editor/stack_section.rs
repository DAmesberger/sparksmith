//! The stack in cross-section, drawn to scale: foils in their layer
//! colours, cores and prepregs in two tints, solder mask, and the drills
//! through it as barrels. The Stackup dialog draws the board's stackup and
//! a fabricator's with it; a drill view draws its spans with the same
//! barrels.
use crate::kit;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use sparksmith_core::{CopperLayer, DielectricKind, Project, Stackup, ViaFill, ViaSpan};
use sparksmith_ui::theme::Tokens;

/// What a band of the section is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BandKind {
    Mask,
    Copper(CopperLayer),
    Core,
    Prepreg,
}

/// One layer of the stack, its thickness to scale.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Band {
    pub(super) kind: BandKind,
    pub(super) mm: f64,
    pub(super) label: String,
}

/// The holes drilled from one copper layer to another, by index from the
/// top.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Barrel {
    pub(super) first: usize,
    pub(super) last: usize,
    /// Filled and plated over.
    pub(super) capped: bool,
    pub(super) label: String,
}

/// The bands of `stackup`, top to bottom.
pub(super) fn bands(stackup: &Stackup) -> Vec<Band> {
    let mask = |shown: bool| {
        shown.then(|| Band {
            kind: BandKind::Mask,
            mm: stackup.mask_um / 1000.0,
            label: "Mask".into(),
        })
    };
    let layers = stackup.layers();
    let mut out: Vec<Band> = mask(stackup.mask_top).into_iter().collect();
    for (i, (layer, foil)) in layers.iter().zip(&stackup.copper).enumerate() {
        out.push(Band {
            kind: BandKind::Copper(*layer),
            mm: foil.thickness_um / 1000.0,
            label: layer.name(),
        });
        if let Some(d) = stackup.dielectrics.get(i) {
            let (kind, name) = match d.kind {
                DielectricKind::Core => (BandKind::Core, "Core"),
                DielectricKind::Prepreg => (BandKind::Prepreg, "Prepreg"),
            };
            let dk =
                d.dk.map_or_else(String::new, |v| format!(" · {}", kit::number_text(v)));
            out.push(Band {
                kind,
                mm: d.thickness_mm,
                label: format!("{name} {}{dk}", kit::number_text(d.thickness_mm)),
            });
        }
    }
    out.extend(mask(stackup.mask_bottom));
    out
}

/// The board's vias as barrels: one for each span and fill in use.
pub(super) fn barrels(project: &Project) -> Vec<Barrel> {
    let stackup = &project.stackup;
    let last = stackup.copper.len().saturating_sub(1);
    let mut out: Vec<(Barrel, usize)> = Vec::new();
    for via in &project.vias {
        let ends = match via.span {
            ViaSpan::Through => Some((0, last)),
            ViaSpan::Between { from, to } => stackup.index(from).zip(stackup.index(to)),
        };
        let Some((a, b)) = ends else {
            continue;
        };
        let capped = via.fill == ViaFill::Capped;
        let (first, end) = (a.min(b), a.max(b));
        match out
            .iter_mut()
            .find(|(d, _)| (d.first, d.last, d.capped) == (first, end, capped))
        {
            Some((_, n)) => *n += 1,
            None => out.push((
                Barrel {
                    first,
                    last: end,
                    capped,
                    label: String::new(),
                },
                1,
            )),
        }
    }
    out.sort_by_key(|(d, _)| (d.first, d.last, d.capped));
    out.into_iter()
        .map(|(barrel, n)| Barrel {
            label: if barrel.capped {
                format!("{n} capped")
            } else {
                kit::counted(n, "via", "vias")
            },
            ..barrel
        })
        .collect()
}

/// Where each band lies, in points from the section's top: `height` shared
/// out by thickness.
pub(super) fn spans(bands: &[Band], height: f32) -> Vec<(f32, f32)> {
    let total: f64 = bands.iter().map(|b| b.mm).sum();
    let mut from = 0.0;
    bands
        .iter()
        .map(|band| {
            let top = from;
            from += band.mm;
            if total > 0.0 {
                let at = |mm: f64| (mm / total) as f32 * height;
                (at(top), at(from))
            } else {
                (0.0, 0.0)
            }
        })
        .collect()
}

/// Label centres as near `centres` (ascending) as `line` apart and inside
/// `0..height` allow.
pub(super) fn spread(centres: &[f32], line: f32, height: f32) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::with_capacity(centres.len());
    for centre in centres {
        let least = out.last().map_or(line / 2.0, |y| y + line);
        out.push(centre.max(least));
    }
    let mut most = height - line / 2.0;
    for y in out.iter_mut().rev() {
        *y = y.min(most);
        most = *y - line;
    }
    out
}

fn ink(kind: BandKind) -> Color32 {
    let t = Tokens::get();
    match kind {
        BandKind::Mask => t.text_dim.gamma_multiply(0.7),
        BandKind::Copper(layer) => crate::palette::board().copper_layer(layer),
        BandKind::Core => t.text_dim.gamma_multiply(0.42),
        BandKind::Prepreg => t.text_dim.gamma_multiply(0.22),
    }
}

/// Draw `bands` and `barrels` across the width of `ui`: copper named at the
/// left, dielectrics at the right.
pub(super) fn show(ui: &mut Ui, bands: &[Band], barrels: &[Barrel]) -> egui::Response {
    let t = Tokens::get();
    let font = FontId::proportional(t.font_small);
    let line = t.font_small + t.space_2;
    let copper = bands
        .iter()
        .filter(|b| matches!(b.kind, BandKind::Copper(_)))
        .count();
    let height = (t.control_height * 7.0).max(line * copper as f32);
    let foot = if barrels.is_empty() { 0.0 } else { line };
    let width = ui.available_width().max(0.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, height + foot + 2.0 * t.space_4), Sense::hover());
    let top = rect.top() + t.space_4;
    let stack = Rect::from_min_max(
        pos2(rect.left() + width * 0.22, top),
        pos2(rect.left() + width * 0.5, top + height),
    );
    let spans = spans(bands, height);
    let painter = ui.painter_at(rect);
    for (band, (from, to)) in bands.iter().zip(&spans) {
        // A foil thinner than a point is still seen.
        let least = (to - from).max(1.0);
        let y = (from + to) / 2.0;
        painter.rect_filled(
            Rect::from_min_max(
                pos2(stack.left(), top + y - least / 2.0),
                pos2(stack.right(), top + y + least / 2.0),
            ),
            0.0,
            ink(band.kind),
        );
    }
    let foils: Vec<(f32, f32)> = bands
        .iter()
        .zip(&spans)
        .filter(|(b, _)| matches!(b.kind, BandKind::Copper(_)))
        .map(|(_, s)| *s)
        .collect();
    let step = stack.width() / (barrels.len() as f32 + 1.0);
    for (i, barrel) in barrels.iter().enumerate() {
        let (Some(first), Some(last)) = (foils.get(barrel.first), foils.get(barrel.last)) else {
            continue;
        };
        let x = stack.left() + step * (i as f32 + 1.0);
        let body = Rect::from_min_max(
            pos2(x - t.space_4, top + first.0),
            pos2(x + t.space_4, top + last.1),
        );
        let fill = if barrel.capped { t.text_dim } else { t.panel };
        painter.rect(
            body,
            0.0,
            fill,
            Stroke::new(1.5, t.text),
            egui::StrokeKind::Inside,
        );
        painter.text(
            pos2(x, top + height + foot),
            Align2::CENTER_BOTTOM,
            &barrel.label,
            font.clone(),
            t.text_dim,
        );
    }
    for left in [true, false] {
        let side: Vec<(&Band, f32)> = bands
            .iter()
            .zip(&spans)
            .filter(|(b, _)| match b.kind {
                BandKind::Copper(_) => left,
                BandKind::Core | BandKind::Prepreg => !left,
                BandKind::Mask => false,
            })
            .map(|(b, (from, to))| (b, (from + to) / 2.0))
            .collect();
        let centres: Vec<f32> = side.iter().map(|(_, y)| *y).collect();
        for ((band, y), at) in side.iter().zip(spread(&centres, line, height)) {
            let (edge, text, align): (Pos2, Pos2, Align2) = if left {
                (
                    pos2(stack.left(), top + y),
                    pos2(stack.left() - t.space_8, top + at),
                    Align2::RIGHT_CENTER,
                )
            } else {
                (
                    pos2(stack.right(), top + y),
                    pos2(stack.right() + t.space_8, top + at),
                    Align2::LEFT_CENTER,
                )
            };
            painter.add(Shape::line_segment(
                [edge, text],
                Stroke::new(0.5, t.text_dim),
            ));
            let gap = if left { -t.space_2 } else { t.space_2 };
            painter.text(
                text + vec2(gap, 0.0),
                align,
                &band.label,
                font.clone(),
                t.text,
            );
        }
    }
    response.on_hover_text(
        "To scale, dielectrics in mm with their Dk; a foil is drawn at least one point thick",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// The 4-layer test preset: mask, foil, prepreg 0.2, foil, core 1.06,
    /// foil, prepreg 0.2, foil, mask; every band its share of the height.
    #[test]
    fn bands_are_to_scale_top_to_bottom() -> TestResult {
        let stackup = Stackup::preset(4)?;
        let bands = bands(&stackup);
        assert_eq!(bands.len(), 9);
        let total: f64 = bands.iter().map(|b| b.mm).sum();
        assert!((total - stackup.thickness_mm() - 2.0 * stackup.mask_um / 1000.0).abs() < 1e-9);
        let spans = spans(&bands, 200.0);
        assert_eq!(spans.first().map(|s| s.0), Some(0.0));
        assert!(spans.last().is_some_and(|s| (s.1 - 200.0).abs() < 1e-3));
        for (band, (from, to)) in bands.iter().zip(&spans) {
            let share = (f64::from(to - from) / 200.0 - band.mm / total).abs();
            assert!(share < 1e-5, "{band:?}");
        }
        Ok(())
    }

    #[test]
    fn labels_keep_their_order_and_do_not_overlap() {
        let at = spread(&[1.0, 1.5, 2.0, 99.0, 99.5], 12.0, 100.0);
        for pair in at.windows(2) {
            assert!(pair[1] - pair[0] >= 12.0 - 1e-3, "{at:?}");
        }
        assert!(at.iter().all(|y| (6.0..=94.0).contains(y)), "{at:?}");
        // Labels with room stay where they are.
        assert_eq!(spread(&[20.0, 60.0], 12.0, 100.0), [20.0, 60.0]);
    }

    #[test]
    fn vias_are_counted_by_span_and_fill() -> TestResult {
        use sparksmith_core::{Engine, Operation, layout::LayoutCommand, layout::ViaRequest};
        let mut engine = Engine::demo()?;
        let net = engine.project().nets.first().ok_or("no net")?.id;
        for x in [20.0, 24.0] {
            engine.apply(
                engine.revision(),
                Operation::Layout(LayoutCommand::AddVia {
                    via: ViaRequest::fixture(net, [x, 20.0]),
                }),
            )?;
        }
        let drawn = barrels(engine.project());
        assert_eq!(drawn.len(), 1);
        assert_eq!((drawn[0].first, drawn[0].last), (0, 1));
        assert_eq!(drawn[0].label, "2 vias");
        Ok(())
    }
}

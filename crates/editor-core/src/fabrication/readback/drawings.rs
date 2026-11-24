//! The fabrication and assembly drawings. What they are to print
//! ([`DrawingData`]) is held to the document where it can be told apart from
//! the writer (holes, parts, board size, release); the PDFs are held to it:
//! every table row, the title block, every hole at its place and size, every
//! part's designator. Not told: the outline's line work, pads and bodies,
//! dimension lines, drill symbols, polarity dots and the notes' order.
use super::oracle;
use super::tables::outline_box;
use super::{Fallible, MOST, Reading, distance};
use crate::fabrication::{DrawingData, HoleFeature};
use crate::geometry::Point;
use crate::model::Layer;
use sparksmith_fabcheck::drills::{self, DrillHit, Feature};
use sparksmith_fabcheck::pdf::{self, Page};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Holes of the drawing and the document are the same numbers, mm.
const SAME_MM: f64 = 1e-5;
/// A place on paper, mm: the drawing's points hold about a thousandth.
const PAPER_MM: f64 = 0.02;

pub(super) fn check(r: &mut Reading<'_, '_>, data: &DrawingData) -> Fallible<()> {
    let fabrication = r.name("fabrication.pdf");
    let assembly = r.name("assembly.pdf");
    design(r, data, &fabrication, &assembly)?;
    if let Some(pages) = pages(r, &fabrication) {
        tables(r, data, &fabrication, &pages);
        holes(r, data, &fabrication, &pages);
    }
    if let Some(pages) = pages(r, &assembly) {
        parts(r, data, &assembly, &pages);
    }
    Ok(())
}

fn pages(r: &mut Reading<'_, '_>, name: &str) -> Option<Vec<Page>> {
    let bytes = r.bytes(name, true)?;
    match pdf::pages(bytes) {
        Ok(pages) => Some(pages),
        Err(e) => {
            r.unreadable(name, &e.to_string());
            None
        }
    }
}

/// What is to be printed against the document.
fn design(
    r: &mut Reading<'_, '_>,
    data: &DrawingData,
    fabrication: &str,
    assembly: &str,
) -> Fallible<()> {
    let project = r.project;
    for plated in [true, false] {
        let printed: Vec<DrillHit> = data
            .holes
            .iter()
            .filter(|h| h.plated == plated)
            .map(|h| match h.feature {
                HoleFeature::Round { at, diameter } => DrillHit {
                    feature: Feature::Round { at },
                    diameter,
                },
                HoleFeature::Slot { from, to, width } => DrillHit {
                    feature: Feature::Slot { from, to },
                    diameter: width,
                },
            })
            .collect();
        let wanted: Vec<DrillHit> = oracle::holes(project)?
            .iter()
            .filter(|h| h.plated == plated)
            .map(|h| DrillHit {
                feature: if h.slot() {
                    Feature::Slot {
                        from: h.from,
                        to: h.to,
                    }
                } else {
                    Feature::Round { at: h.from }
                },
                diameter: h.width,
            })
            .collect();
        let diff = drills::pair(&printed, &wanted, SAME_MM);
        for (hits, missing) in [(&diff.unmatched_a, false), (&diff.unmatched_b, true)] {
            for hit in hits.iter().take(MOST) {
                let (Feature::Round { at } | Feature::Slot { from: at, .. }) = hit.feature;
                let words = format!("a hole of {:.3} mm", hit.diameter);
                let (written, design) = if missing {
                    ("no such hole", words.as_str())
                } else {
                    (words.as_str(), "no such hole")
                };
                r.say(fabrication, Some(at), written, design);
            }
        }
    }
    let [min, max] = outline_box(project);
    let (a, b) = (data.bounds.min, data.bounds.max);
    if distance(a, min) > 1e-3 || distance(b, max) > 1e-3 {
        let words = |a: Point, b: Point| format!("{:.3} × {:.3} mm", b[0] - a[0], b[1] - a[1]);
        r.say(fabrication, None, &words(a, b), &words(min, max));
    }
    let identity = r.identity;
    if (&data.release, &data.project_id) != (&identity.revision, &identity.guid) {
        let written = format!("the release {} of {}", data.release, data.project_id);
        let design = format!("{} of {}", identity.revision, identity.guid);
        r.say(fabrication, None, &written, &design);
    }
    for (layer, word) in [(Layer::Top, "top"), (Layer::Bottom, "bottom")] {
        let mut left: BTreeMap<_, _> = project
            .placements
            .iter()
            .filter(|p| p.layer == layer && project.instance(p.instance).is_some())
            .map(|p| (p.instance, (p.position, p.rotation_deg)))
            .collect();
        let printed = data
            .sides
            .iter()
            .filter(|s| s.side == layer)
            .flat_map(|s| &s.parts);
        for part in printed {
            let placed = left.remove(&part.instance);
            let same = placed.is_some_and(|(at, turn)| {
                distance(at, part.at) <= SAME_MM && (turn - part.rotation_deg).abs() <= 1e-6
            });
            if !same {
                let (name, turn) = (&part.designator, part.rotation_deg);
                let written = format!("{name} on the {word} turned {turn}°");
                let design = placed.map_or_else(
                    || "no such part there".into(),
                    |(at, turn)| format!("it at ({:.3}, {:.3}) turned {turn}°", at[0], at[1]),
                );
                r.say(assembly, Some(part.at), &written, &design);
            }
        }
        for (at, _) in left.values().take(MOST) {
            r.say(assembly, Some(*at), "no such part", "a part");
        }
    }
    Ok(())
}

/// `text` as the drawings' fonts can show it.
fn shown(text: &str) -> String {
    text.chars()
        .map(|c| match u32::from(c) {
            32..=126 | 0xA0..=0xFF => c,
            0x2013 | 0x2014 | 0x2018 | 0x2019 | 0x201C | 0x201D | 0x2022 | 0x2026 => c,
            _ => '?',
        })
        .collect()
}

/// The text runs of pages, each to be found once.
struct Runs {
    left: BTreeMap<String, usize>,
    /// Runs cut to fit, without their ellipsis.
    cut: Vec<String>,
}

impl Runs {
    fn of<'a>(pages: impl Iterator<Item = &'a Page>) -> Self {
        let mut runs = Self {
            left: BTreeMap::new(),
            cut: Vec::new(),
        };
        for (text, _) in pages.flat_map(|p| &p.texts) {
            if let Some(cut) = text.strip_suffix('…') {
                runs.cut.push(cut.to_owned());
            }
            *runs.left.entry(text.clone()).or_default() += 1;
        }
        runs
    }

    fn take(&mut self, text: &str) -> bool {
        let text = shown(text);
        if let Some(n) = self.left.get_mut(&text).filter(|n| **n > 0) {
            *n -= 1;
            return true;
        }
        match self.cut.iter().position(|c| text.starts_with(c.as_str())) {
            Some(k) => {
                self.cut.swap_remove(k);
                true
            }
            None => false,
        }
    }
}

/// Every cell of the drill and stackup tables, the title blocks and the
/// words of the notes.
fn tables(r: &mut Reading<'_, '_>, data: &DrawingData, name: &str, pages: &[Page]) {
    let mut runs = Runs::of(pages.iter());
    let mut wanted: Vec<String> = Vec::new();
    for t in &data.drills.tools {
        wanted.extend([
            format!("T{}", t.number),
            format!("{:.3}", t.diameter),
            if t.plated { "yes" } else { "no" }.to_owned(),
            if t.slot { "slot" } else { "hole" }.to_owned(),
            t.function.label().to_owned(),
            t.count.to_string(),
        ]);
    }
    for row in &data.stackup {
        wanted.extend([row.name.clone(), row.material.clone()]);
        if row.thickness_mm > 0.0 {
            wanted.push(format!("{:.3}", row.thickness_mm));
        }
    }
    for _ in pages {
        wanted.extend([
            data.release.clone(),
            data.project_id.clone(),
            data.finish.clone(),
            data.profile.clone(),
            format!("{} / {}", data.mask, data.legend),
        ]);
    }
    for k in 0..2 {
        let size = data.bounds.max[k] - data.bounds.min[k];
        wanted.push(format!("{size:.2} mm"));
    }
    let lost: Vec<String> = wanted.into_iter().filter(|w| !runs.take(w)).collect();
    for text in lost.iter().take(MOST) {
        r.say(name, None, "no such text", &format!("“{text}”"));
    }
    let words: BTreeSet<&str> = pages
        .iter()
        .flat_map(|p| &p.texts)
        .flat_map(|(t, _)| t.split_whitespace())
        .collect();
    let notes: Vec<String> = data.notes.iter().map(|n| shown(n)).collect();
    let lost = notes
        .iter()
        .flat_map(|n| n.split_whitespace())
        .find(|w| !words.contains(w));
    if let Some(word) = lost {
        let design = format!("“{word}”");
        r.say(name, None, "no such word in its notes", &design);
    }
}

/// The scale a sheet states: "Viewed from the top. Scale 1:2.5. All …".
fn scale_of(text: &str) -> Option<f64> {
    let stated = text.split_once("Scale ")?.1.split_once(". ")?.0;
    let (a, b) = stated.split_once(':')?;
    let scale = a.parse::<f64>().ok()? / b.parse::<f64>().ok()?;
    // Stated to two decimals: the one scale that is not exact, 3:4, reads
    // 1:1.33.
    Some(if (scale - 0.75).abs() < 0.004 {
        0.75
    } else {
        scale
    })
}

/// Every hole on the first sheet, at its place and size by the sheet's scale.
fn holes(r: &mut Reading<'_, '_>, data: &DrawingData, name: &str, pages: &[Page]) {
    let Some(page) = pages.first() else {
        r.say(name, None, "no sheet", "a sheet");
        return;
    };
    let scale = page.texts.iter().find_map(|(t, _)| scale_of(t));
    let Some(scale) = scale.filter(|s| *s > 0.0) else {
        r.say(name, None, "no scale", "a scale");
        return;
    };
    // What a hole is drawn as: a circle of its size, or a line as wide as
    // its slot.
    let rounds: Vec<(Point, f64)> = data
        .holes
        .iter()
        .filter_map(|h| match h.feature {
            HoleFeature::Round { at, diameter } => Some((at, diameter / 2.0 * scale)),
            HoleFeature::Slot { .. } => None,
        })
        .collect();
    let slots: Vec<(Point, Point, f64)> = data
        .holes
        .iter()
        .filter_map(|h| match h.feature {
            HoleFeature::Slot { from, to, width } => Some((from, to, width * scale)),
            HoleFeature::Round { .. } => None,
        })
        .collect();
    // Where the board's corner is on paper: the overall dimensions start
    // there, and nothing else tells. So it is taken from the first hole, at
    // each place that could be it, and every other hole must agree.
    let candidates: Vec<Point> = match (rounds.first(), slots.first()) {
        (Some((at, radius)), _) => page
            .circles
            .iter()
            .filter(|(_, r)| (r - radius).abs() < PAPER_MM)
            .map(|(c, _)| [c[0] - at[0] * scale, c[1] - at[1] * scale])
            .collect(),
        (None, Some((from, _, width))) => page
            .lines
            .iter()
            .filter(|(_, _, pen)| (pen - width).abs() < PAPER_MM)
            .flat_map(|(a, b, _)| [*a, *b])
            .map(|p| [p[0] - from[0] * scale, p[1] - from[1] * scale])
            .collect(),
        (None, None) => return,
    };
    let circles = Filed::of(page.circles.iter().map(|(c, r)| (*c, *r)));
    let lines = Filed::of(page.lines.iter().flat_map(|(a, b, pen)| {
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        [(m, *pen)]
    }));
    // The first hole drawn nowhere, for the candidate most holes agree with.
    let mut best: Option<(usize, Point)> = None;
    for o in &candidates {
        let to = |p: Point| [o[0] + p[0] * scale, o[1] + p[1] * scale];
        let lost = rounds
            .iter()
            .position(|(at, radius)| !circles.has(to(*at), *radius));
        let lost = lost.map(|k| (k, rounds[k].0)).or_else(|| {
            let k = slots.iter().position(|(a, b, width)| {
                let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                !lines.has(to(m), *width)
            })?;
            Some((rounds.len() + k, slots[k].0))
        });
        match lost {
            None => return,
            Some((k, at)) if best.is_none_or(|(n, _)| k >= n) => best = Some((k, at)),
            Some(_) => {}
        }
    }
    let at = best
        .map(|(_, at)| at)
        .or(rounds.first().map(|h| h.0))
        .or(slots.first().map(|h| h.0));
    r.say(name, at, "no hole drawn there", "a hole");
}

/// Sizes filed by place on paper.
struct Filed {
    cells: HashMap<(i64, i64), Vec<(Point, f64)>>,
}

impl Filed {
    const CELL_MM: f64 = 0.5;

    fn cell(p: Point) -> (i64, i64) {
        let k = |v: f64| (v / Self::CELL_MM).floor() as i64;
        (k(p[0]), k(p[1]))
    }

    fn of(items: impl Iterator<Item = (Point, f64)>) -> Self {
        let mut cells: HashMap<_, Vec<_>> = HashMap::new();
        for (p, size) in items {
            cells.entry(Self::cell(p)).or_default().push((p, size));
        }
        Self { cells }
    }

    fn has(&self, at: Point, size: f64) -> bool {
        let (x, y) = Self::cell(at);
        (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dy| (dx, dy)))
            .filter_map(|(dx, dy)| self.cells.get(&(x + dx, y + dy)))
            .flatten()
            .any(|(p, s)| distance(*p, at) < PAPER_MM && (s - size).abs() < PAPER_MM)
    }
}

/// A page for each side, each part named on its own, and the release.
fn parts(r: &mut Reading<'_, '_>, data: &DrawingData, name: &str, pages: &[Page]) {
    let sides = data.sides.len().max(1);
    if pages.len() != sides {
        let written = format!("{} pages", pages.len());
        r.say(name, None, &written, &format!("{sides}"));
    }
    for (side, page) in data.sides.iter().zip(pages) {
        let mut runs = Runs::of(std::iter::once(page));
        let word = match side.side {
            Layer::Top => "top",
            Layer::Bottom => "bottom",
        };
        let mut lost: Vec<(Option<Point>, String)> = Vec::new();
        for text in [
            format!("Assembly drawing, {word} side"),
            data.release.clone(),
            data.project_id.clone(),
        ] {
            if !runs.take(&text) {
                lost.push((None, text));
            }
        }
        for part in side.parts.iter().filter(|p| !p.cut_trace) {
            if !runs.take(&part.designator) {
                lost.push((Some(part.at), part.designator.clone()));
            }
        }
        for (at, text) in lost.into_iter().take(MOST) {
            let design = format!("“{text}” on the {word} side");
            r.say(name, at, "no such text", &design);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scale_of;

    #[test]
    fn every_scale_a_sheet_states_is_read() {
        for (text, scale) in [
            ("Viewed from the top. Scale 5:1. All dimensions in mm.", 5.0),
            (
                "Viewed from the top. Scale 2.5:1. All dimensions in mm.",
                2.5,
            ),
            (
                "Viewed from the top. Scale 1:1.33. All dimensions in mm.",
                0.75,
            ),
            (
                "Viewed from the top. Scale 1:2.5. All dimensions in mm.",
                0.4,
            ),
            (
                "Viewed from the top. Scale 1:20. All dimensions in mm.",
                0.05,
            ),
        ] {
            assert_eq!(scale_of(text), Some(scale), "{text}");
        }
        assert_eq!(scale_of("Notes"), None);
    }
}

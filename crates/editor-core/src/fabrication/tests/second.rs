//! A second opinion on the written layers, by another method than the point
//! oracle's: every object of every file measured on its own (its place from
//! the file, its area from the exact polygon reader) and paired one to one
//! with an object of the document whose area is a formula. No point is
//! sampled and nothing is compared as an image. The places come from the
//! core's own placed pads ([`crate::layout::placed_pads`]), where the point
//! oracle works out its own, so a mistake in either placement shows as a
//! disagreement between the two.
use super::Fallible;
use super::{excellon_reader, reader};
use crate::fabrication::FabricationSet;
use crate::geometry::Point;
use crate::layout::{self, PlacedPad};
use crate::library::{MaskRule, Outline, PadKind, PadShape, PasteRule, Segment, ShapeNm};
use crate::model::board::CutoutKind;
use crate::model::{CopperLayer, Layer, Project};
use sparksmith_fabcheck::{outline, polygons};
use std::f64::consts::PI;

/// A shape by what its formulas need: a box with some corners rounded (a
/// circle and a stadium are ones too), or an outline of lines and arcs.
#[derive(Clone, Debug)]
enum Form {
    Box {
        w: f64,
        h: f64,
        r: f64,
        rounded: usize,
    },
    Outline(Outline),
}

impl Form {
    fn of(shape: &PadShape) -> Self {
        match shape {
            PadShape::Rect { size } => {
                let [w, h] = size.to_mm_f64();
                Self::Box {
                    w,
                    h,
                    r: 0.0,
                    rounded: 0,
                }
            }
            PadShape::RoundRect {
                size,
                radius_ratio,
                max_radius_nm,
                corners,
            } => {
                let [w, h] = size.to_mm_f64();
                let short = w.min(h);
                let r = max_radius_nm
                    .iter()
                    .map(|m| m.to_mm_f64())
                    .fold((radius_ratio * short).min(short / 2.0), f64::min);
                let rounded = [
                    corners.top_left,
                    corners.top_right,
                    corners.bottom_left,
                    corners.bottom_right,
                ]
                .iter()
                .filter(|on| **on)
                .count();
                Self::Box { w, h, r, rounded }
            }
            PadShape::Circle { diameter_nm } => {
                let d = diameter_nm.to_mm_f64();
                Self::Box {
                    w: d,
                    h: d,
                    r: d / 2.0,
                    rounded: 4,
                }
            }
            PadShape::Oblong { size } => {
                let [w, h] = size.to_mm_f64();
                Self::Box {
                    w,
                    h,
                    r: w.min(h) / 2.0,
                    rounded: 4,
                }
            }
            PadShape::Custom { outline } => Self::Outline(outline.clone()),
        }
    }

    /// Area and perimeter: a rounded corner takes `(1 - π/4) r²` from the
    /// box, and is `(2 - π/2) r` shorter than the corner.
    fn measure(&self) -> (f64, f64) {
        match self {
            Self::Box { w, h, r, rounded } => {
                let n = *rounded as f64;
                (
                    w * h - n * (1.0 - PI / 4.0) * r * r,
                    2.0 * (w + h) - n * (2.0 - PI / 2.0) * r,
                )
            }
            Self::Outline(o) => measure(o),
        }
    }

    /// The area grown by `by` mm, when it is known by formula: a convex
    /// shape of area A and perimeter P grows to A + P·by + π·by²; a box
    /// shrinks on every side, its rounded corners with it. `None` for
    /// another shape that changes.
    fn grown(&self, by: f64) -> Option<f64> {
        let (a, p) = self.measure();
        if by == 0.0 {
            return Some(a);
        }
        match self {
            Self::Box { .. } if by > 0.0 => Some(a + p * by + PI * by * by),
            Self::Box { w, h, r, rounded } => {
                let (w, h) = (w + 2.0 * by, h + 2.0 * by);
                if w <= 0.0 || h <= 0.0 {
                    return Some(0.0);
                }
                let form = Self::Box {
                    w,
                    h,
                    r: (r + by).max(0.0),
                    rounded: *rounded,
                };
                Some(form.measure().0)
            }
            Self::Outline(_) => None,
        }
    }
}

/// Area and length of a closed outline: the shoelace sum of its corners,
/// each arc's segment added where it bulges out, taken where it bulges in.
fn measure(outline: &Outline) -> (f64, f64) {
    let (mut twice, mut arcs, mut length) = (0.0, 0.0, 0.0);
    let mut a = outline.start;
    for s in &outline.segments {
        let b = s.to();
        twice += a[0] * b[1] - b[0] * a[1];
        let chord = (b[0] - a[0]).hypot(b[1] - a[1]);
        match *s {
            Segment::Arc { bulge, .. } if bulge.abs() > 1e-12 && chord > 1e-12 => {
                let sweep = 4.0 * bulge.atan();
                let r = chord / (2.0 * (sweep / 2.0).sin().abs());
                arcs += r * r / 2.0 * (sweep - sweep.sin());
                length += r * sweep.abs();
            }
            _ => length += chord,
        }
        a = b;
    }
    ((twice / 2.0 + arcs).abs(), length)
}

/// The area of a closed drawn shape (a paste window).
fn window(shape: &ShapeNm) -> Option<f64> {
    Some(match shape {
        ShapeNm::Circle { radius_nm, .. } => PI * radius_nm.to_mm_f64().powi(2),
        ShapeNm::Rect { a, b, radius_nm } => {
            let (a, b) = (a.to_mm_f64(), b.to_mm_f64());
            Form::Box {
                w: (a[0] - b[0]).abs(),
                h: (a[1] - b[1]).abs(),
                r: radius_nm.to_mm_f64(),
                rounded: 4,
            }
            .measure()
            .0
        }
        ShapeNm::Polygon { points, radii_nm } => {
            let p: Vec<Point> = points.iter().map(|p| p.to_mm_f64()).collect();
            let n = p.len();
            let mut twice = 0.0;
            let mut fillets = 0.0;
            for k in 0..n {
                let (a, b, c) = (p[(k + n - 1) % n], p[k], p[(k + 1) % n]);
                twice += b[0] * c[1] - c[0] * b[1];
                let r = radii_nm.get(k).map_or(0.0, |r| r.to_mm_f64());
                let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
                let turn = (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]);
                let half = turn.abs() / 2.0;
                fillets += r * r * (half.tan() - half) * turn.signum();
            }
            let area: f64 = twice / 2.0;
            area.abs() - fillets * area.signum()
        }
        _ => return None,
    })
}

/// What one layer must hold: a place, an area, and how it is told.
#[derive(Clone, Debug)]
struct Want {
    at: Point,
    /// How far from `at` the object may be: 0 for one written by its
    /// centre, the shape's reach for a region.
    within: f64,
    area: f64,
    /// How far the polygon reader's chords may leave the area.
    slack: f64,
    what: String,
}

fn want(pad: &PlacedPad, form: &Form, by: f64, area: f64, what: &str) -> Want {
    let (_, perimeter) = form.measure();
    let reach = match form {
        Form::Box { w, h, .. } => w.hypot(*h) / 2.0,
        Form::Outline(_) => perimeter / 2.0,
    };
    Want {
        at: pad.pad_transform().apply([0.0, 0.0]),
        within: reach + by.abs(),
        area,
        // Chords within 0.25 µm, coordinates to 1 nm, 1 µm chords of a
        // grown outline.
        slack: 1e-5 + (perimeter + 10.0 * by.abs()) * 0.0012,
        what: format!("{what} of pad {} of part {}", pad.pad.id, pad.instance),
    }
}

/// `pad`'s land grown by `by`. A custom outline has no formula: its area is
/// its perimeter's band, within half of it.
fn offset(pad: &PlacedPad, form: &Form, by: f64, what: &str) -> Want {
    match form.grown(by) {
        Some(area) => want(pad, form, by, area, what),
        None => {
            let (area, perimeter) = form.measure();
            let mut w = want(pad, form, by, (area + perimeter * by).max(0.0), what);
            w.slack += perimeter * by.abs() / 2.0;
            w
        }
    }
}

/// One object read from a file.
struct Got {
    at: Point,
    flash: bool,
    area: f64,
}

/// The objects of a layer that are not fill copper, tracks or drawn lines.
fn lands(file: &reader::GerberFile) -> Fallible<Vec<Got>> {
    let mut out = Vec::new();
    for object in &file.objects {
        let flash = matches!(object.kind, reader::Kind::Flash { .. });
        let region = matches!(object.kind, reader::Kind::Region { .. });
        if !(flash || region && object.object_attributes.contains_key(".C")) {
            continue;
        }
        let shapes = polygons::object_shapes(file, object)?;
        out.push(Got {
            at: place(object, &shapes),
            flash,
            area: polygons::area_mm2(&shapes),
        });
    }
    Ok(out)
}

/// Every object of a mask or paste layer.
fn openings(file: &reader::GerberFile) -> Fallible<Vec<Got>> {
    let mut out = Vec::new();
    for object in &file.objects {
        let shapes = polygons::object_shapes(file, object)?;
        out.push(Got {
            at: place(object, &shapes),
            flash: matches!(object.kind, reader::Kind::Flash { .. }),
            area: polygons::area_mm2(&shapes),
        });
    }
    Ok(out)
}

/// A flash's centre, else the middle of the shape's box.
fn place(object: &reader::Object, shapes: &polygons::Shapes) -> Point {
    if let reader::Kind::Flash { at } = object.kind {
        return at;
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in shapes.iter().flatten().flatten() {
        for (k, v) in [f64::from(p.x) / 1e6, f64::from(p.y) / 1e6]
            .into_iter()
            .enumerate()
        {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
}

/// Pair every wanted object with a written one, one to one.
fn paired(layer: &str, mut got: Vec<Got>, want: &[Want], out: &mut Vec<String>) {
    for w in want {
        let found = got
            .iter()
            .enumerate()
            .map(|(k, g)| {
                let d = (g.at[0] - w.at[0]).hypot(g.at[1] - w.at[1]);
                (k, d, g)
            })
            .filter(|(_, d, g)| {
                let near = if g.flash { 2e-6 } else { w.within };
                *d <= near && (g.area - w.area).abs() <= w.slack
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _, _)| k);
        match found {
            Some(k) => {
                got.swap_remove(k);
            }
            None => out.push(format!("{layer}: no {} ({:.5} mm²)", w.what, w.area)),
        }
    }
    for g in got {
        out.push(format!(
            "{layer}: an object at {:?} ({:.5} mm²) that nothing asks for",
            g.at, g.area
        ));
    }
}

fn disc(at: Point, diameter: f64, what: &str) -> Want {
    Want {
        at,
        within: 0.0,
        area: PI * diameter * diameter / 4.0,
        slack: 1e-5 + PI * diameter * 0.0003,
        what: what.to_owned(),
    }
}

/// Everything `set` writes of `project` that is not what the document asks
/// for, by measures and counts.
pub(super) fn disagreements(project: &Project, set: &FabricationSet) -> Fallible<Vec<String>> {
    let mut out = Vec::new();
    let text = |suffix: &str| -> Option<&str> {
        set.files
            .iter()
            .find(|f| f.name.ends_with(suffix))
            .and_then(|f| std::str::from_utf8(&f.bytes).ok())
    };
    let pads = layout::placed_pads(project);
    let board = project.board();
    let rules = project.rules.effective();
    for layer in project.stackup.layers() {
        let n = project.stackup.index(layer).map_or(0, |i| i + 1);
        let name = match layer {
            CopperLayer::Top => format!("-copper-l{n}-top.gbr"),
            CopperLayer::Bottom => format!("-copper-l{n}-bottom.gbr"),
            CopperLayer::Inner(k) => format!("-copper-l{n}-inner{k}.gbr"),
        };
        let Some(source) = text(&name) else {
            out.push(format!("{name}: missing"));
            continue;
        };
        let file = reader::parse(source)?;
        let mut wants: Vec<Want> = pads
            .iter()
            .filter(|p| p.on_copper(layer))
            .map(|p| {
                let form = Form::of(&p.pad.shape);
                want(p, &form, 0.0, form.measure().0, "the land")
            })
            .collect();
        wants.extend(
            project
                .vias
                .iter()
                .map(|v| disc(v.position, v.diameter, "a via")),
        );
        wants.extend(board.holes.iter().filter_map(|h| {
            h.pad_diameter
                .filter(|_| h.plated)
                .map(|d| disc(h.at, d, "a washer"))
        }));
        paired(&name, lands(&file)?, &wants, &mut out);
        // Tracks: each drawn once, either way round, at its width.
        let mut drawn: Vec<(Point, Point, f64)> = Vec::new();
        for object in &file.objects {
            if let reader::Kind::Draw { from, to } = object.kind {
                let shapes = polygons::object_shapes(&file, object)?;
                drawn.push((from, to, polygons::area_mm2(&shapes)));
            }
        }
        for t in project.tracks.iter().filter(|t| t.layer == layer) {
            let close = |a: Point, b: Point| (a[0] - b[0]).hypot(a[1] - b[1]) < 2e-6;
            let length = (t.end[0] - t.start[0]).hypot(t.end[1] - t.start[1]);
            let area = length * t.width + PI * t.width * t.width / 4.0;
            let found = drawn.iter().position(|(a, b, drawn)| {
                (close(*a, t.start) && close(*b, t.end) || close(*a, t.end) && close(*b, t.start))
                    && (drawn - area).abs() < 1e-5 + (length + t.width) * 0.0006
            });
            match found {
                Some(k) => {
                    drawn.swap_remove(k);
                }
                None => out.push(format!("{name}: no track {}", t.id)),
            }
        }
        if !drawn.is_empty() {
            out.push(format!("{name}: {} draws of no track", drawn.len()));
        }
        // Fill copper, as much as is stored, net by net.
        for net in project
            .fills
            .iter()
            .filter(|f| f.layer == layer)
            .map(|f| f.net)
            .collect::<std::collections::BTreeSet<_>>()
        {
            let pours: Vec<_> = project
                .fills
                .iter()
                .filter(|f| f.layer == layer && f.net == net)
                .filter_map(|f| f.computed.as_ref())
                .collect();
            // Pours of one net may overlap: their sum says nothing then.
            if pours.len() != 1 {
                continue;
            }
            let (mut area, mut around) = (0.0, 0.0);
            for region in pours.iter().flat_map(|c| &c.regions) {
                for (k, ring) in std::iter::once(&region.outer)
                    .chain(&region.holes)
                    .enumerate()
                {
                    let (a, l) = ring_measure(ring);
                    area += if k == 0 { a } else { -a };
                    around += l;
                }
            }
            let label = net.map_or("", |n| project.net_name(n));
            let shapes = polygons::render(&file, |o| {
                matches!(o.kind, reader::Kind::Region { .. })
                    && !o.object_attributes.contains_key(".C")
                    && o.object_attributes
                        .get(".N")
                        .map(|v| v.join(","))
                        .unwrap_or_default()
                        == label
            })?;
            let written = polygons::area_mm2(&shapes);
            if (written - area).abs() > 1e-4 + around * 2e-6 {
                out.push(format!(
                    "{name}: {written:.5} mm² of fill on {label:?}, {area:.5} stored"
                ));
            }
        }
    }
    for side in [Layer::Top, Layer::Bottom] {
        let word = if side == Layer::Top { "top" } else { "bottom" };
        let mut mask = Vec::new();
        let mut paste = Vec::new();
        for p in &pads {
            let form = Form::of(&p.pad.shape);
            let through = matches!(p.pad.kind, PadKind::PlatedThrough | PadKind::NonPlatedHole);
            if through || p.layer == side {
                let by = match p.pad.mask {
                    MaskRule::Inherit => Some(rules.mask.expansion),
                    MaskRule::Tented => None,
                    MaskRule::Expansion(e) => Some(e.to_mm_f64()),
                };
                if let Some(by) = by {
                    mask.push(offset(p, &form, by, "the mask opening"));
                }
            }
            if p.layer != side {
                continue;
            }
            let shrunk = |by: f64| offset(p, &form, -by, "the paste");
            match &p.pad.paste {
                PasteRule::Inherit
                    if matches!(p.pad.kind, PadKind::Smd | PadKind::Mechanical)
                        && p.pad.hole.is_none() =>
                {
                    paste.push(shrunk(rules.paste.reduction));
                }
                PasteRule::Inherit | PasteRule::None => {}
                PasteRule::Reduction(r) => paste.push(shrunk(r.to_mm_f64())),
                PasteRule::Windows(windows) => {
                    for w in windows {
                        let area = window(&w.shape).ok_or("a window has no formula")?;
                        let mut one = want(p, &form, 0.0, area, "a paste window");
                        // A window lies anywhere in its land, and is not
                        // written by the land's centre.
                        one.within = f64::INFINITY;
                        paste.push(one);
                    }
                }
            }
        }
        mask.extend(project.vias.iter().filter(|v| !v.tented).map(|v| {
            disc(
                v.position,
                v.diameter + 2.0 * rules.mask.expansion,
                "an open via",
            )
        }));
        mask.extend(board.holes.iter().map(|h| {
            let size = h.pad_diameter.filter(|_| h.plated).unwrap_or(h.drill);
            disc(h.at, size + 2.0 * rules.mask.expansion, "a mounting hole")
        }));
        paste.retain(|w| w.area > 0.0);
        for (name, wants) in [
            (format!("-mask-{word}.gbr"), mask),
            (format!("-paste-{word}.gbr"), paste),
        ] {
            match text(&name) {
                Some(source) => {
                    paired(&name, openings(&reader::parse(source)?)?, &wants, &mut out);
                }
                None if wants.is_empty() => {}
                None => out.push(format!("{name}: missing")),
            }
        }
    }
    // The profile: as many contours, as long in all.
    match text("-profile.gbr") {
        None => out.push("the profile: missing".into()),
        Some(source) => {
            let drawn = outline::contours(&reader::parse(source)?)?;
            let design: Vec<f64> = std::iter::once(&board.outer)
                .chain(
                    board
                        .cutouts
                        .iter()
                        .filter(|c| c.kind != (CutoutKind::Slot { plated: true }))
                        .map(|c| &c.outline),
                )
                .map(|o| measure(o).1)
                .collect();
            let length: f64 = drawn.iter().map(outline::Contour::length).sum();
            let whole: f64 = design.iter().sum();
            if drawn.len() != design.len() || (length - whole).abs() > 1e-4 {
                out.push(format!(
                    "the profile: {} contours, {length:.5} mm; {} of {whole:.5} mm in the design",
                    drawn.len(),
                    design.len()
                ));
            }
        }
    }
    // The drills: as many holes of each size and plating.
    for (plated, suffix) in [(true, "-plated.drl"), (false, "-non-plated.drl")] {
        let mut design: Vec<i64> = Vec::new();
        let size = |mm: f64| (mm * 1000.0).round() as i64;
        if plated {
            design.extend(project.vias.iter().map(|v| size(v.drill)));
            design.extend(
                board
                    .cutouts
                    .iter()
                    .filter(|c| c.kind == (CutoutKind::Slot { plated: true }))
                    .map(|c| {
                        // Two half turns: the width is twice their radius.
                        let (area, length) = measure(&c.outline);
                        size(slot_width(area, length))
                    }),
            );
        }
        for p in &pads {
            if let Some(h) = p.pad.hole.as_ref().filter(|h| h.plated == plated) {
                design.push(size(match h.shape {
                    crate::library::HoleShape::Round { diameter_nm } => diameter_nm.to_mm_f64(),
                    crate::library::HoleShape::Slot { width_nm, .. } => width_nm.to_mm_f64(),
                }));
            }
            if plated {
                design.extend(p.pad.vias.iter().map(|v| size(v.drill_nm.to_mm_f64())));
            }
        }
        design.extend(
            board
                .holes
                .iter()
                .filter(|h| h.plated == plated)
                .map(|h| size(h.drill)),
        );
        let mut written: Vec<i64> = Vec::new();
        if let Some((_, source)) = set
            .files
            .iter()
            // "-plated.drl" ends "-non-plated.drl" too.
            .filter(|f| f.name.ends_with(suffix) && f.name.contains("-non-") != plated)
            .filter_map(|f| Some((&f.name, std::str::from_utf8(&f.bytes).ok()?)))
            .next()
        {
            let file = excellon_reader::parse(source)?;
            for hit in &file.hits {
                written.push(size(file.tools.get(&hit.tool()).copied().ok_or("a tool")?));
            }
        }
        design.sort_unstable();
        written.sort_unstable();
        if design != written {
            out.push(format!(
                "{suffix}: holes of {written:?} µm, {design:?} in the design"
            ));
        }
    }
    Ok(out)
}

/// Area and length of a ring of chords.
fn ring_measure(ring: &[Point]) -> (f64, f64) {
    let (mut twice, mut length) = (0.0, 0.0);
    for k in 0..ring.len() {
        let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
        twice += a[0] * b[1] - b[0] * a[1];
        length += (b[0] - a[0]).hypot(b[1] - a[1]);
    }
    (f64::abs(twice) / 2.0, length)
}

/// The width of a straight slot of this area and length round: with sides
/// `l` and width `w`, the area is `l·w + π w²/4` and the length round
/// `2 l + π w`, so `π w²/4 - round·w/2 + area = 0`, its smaller root.
fn slot_width(area: f64, round: f64) -> f64 {
    let (a, b, c) = (PI / 4.0, -round / 2.0, area);
    (-b - (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a)
}

/// The checker itself: what it must report, seeded in the files.
#[test]
fn the_second_opinion_reports_what_is_not_the_design() -> Fallible<()> {
    let project = crate::Engine::showcase()?.project().clone();
    let set = crate::fabrication::generate(
        &crate::fabrication::ProductionSnapshot::resolve(&project, None)?,
        &Default::default(),
        &Default::default(),
    )?;
    assert_eq!(disagreements(&project, &set)?, Vec::<String>::new());
    let seed = |suffix: &str, change: &dyn Fn(&str) -> String| -> Fallible<FabricationSet> {
        let mut set = set.clone();
        for f in set.files.iter_mut().filter(|f| f.name.ends_with(suffix)) {
            f.bytes = change(std::str::from_utf8(&f.bytes)?).into_bytes();
        }
        Ok(set)
    };
    // The first line of a kind, left out.
    let without = |end: &'static str| {
        move |text: &str| {
            let mut gone = false;
            text.lines()
                .filter(|l| {
                    let this = !gone && l.ends_with(end);
                    gone |= this;
                    !this
                })
                .map(|l| format!("{l}\n"))
                .collect::<String>()
        }
    };
    // The first aperture of a kind, of another size.
    let resized = |from: &'static str, to: &'static str| {
        move |text: &str| {
            let mut done = false;
            text.lines()
                .map(|l| {
                    if !done && l.starts_with("%ADD") && l.contains(from) {
                        done = true;
                        format!("{}\n", l.replacen(from, to, 1))
                    } else {
                        format!("{l}\n")
                    }
                })
                .collect::<String>()
        }
    };
    let mut missed = Vec::new();
    for (what, seeded) in [
        (
            "a land missing",
            seed("-copper-l1-top.gbr", &without("D03*"))?,
        ),
        (
            "a land missing on an inner layer",
            seed("-copper-l2-inner1.gbr", &without("D03*"))?,
        ),
        (
            "apertures of another size",
            seed("-copper-l1-top.gbr", &resized("C,0", "C,1"))?,
        ),
        (
            "a mask opening missing",
            seed("-mask-top.gbr", &without("D03*"))?,
        ),
        (
            "a paste pad missing",
            seed("-paste-top.gbr", &without("D03*"))?,
        ),
        (
            "a hole missing",
            seed("-plated.drl", &|t: &str| {
                let mut gone = false;
                t.lines()
                    .filter(|l| {
                        let this = !gone && l.starts_with('X');
                        gone |= this;
                        !this
                    })
                    .map(|l| format!("{l}\n"))
                    .collect::<String>()
            })?,
        ),
        (
            "an edge of the profile moved",
            seed("-profile.gbr", &|t: &str| {
                t.replacen("X44000000", "X44100000", 1)
            })?,
        ),
        (
            "a pour without its holes",
            seed("-copper-l4-bottom.gbr", &|t: &str| {
                t.replace("%LPC*%\n", "%LPD*%\n")
            })?,
        ),
    ] {
        let found = disagreements(&project, &seeded);
        // A damaged file the reader refuses is reported too.
        if matches!(&found, Ok(found) if found.is_empty()) {
            missed.push(what);
        }
    }
    assert!(missed.is_empty(), "{missed:?}");
    Ok(())
}

/// The formulas, against areas worked by hand.
#[test]
fn the_formulas_hold() {
    let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} {b}");
    let rect = Form::Box {
        w: 2.0,
        h: 1.0,
        r: 0.0,
        rounded: 0,
    };
    close(rect.measure().0, 2.0);
    // Grown 0.1: two by one, six by 0.1 round it and a disc at the corners.
    close(rect.grown(0.1).unwrap_or(0.0), 2.6 + PI * 0.01);
    close(rect.grown(-0.1).unwrap_or(0.0), 1.8 * 0.8);
    let disc = Form::Box {
        w: 1.0,
        h: 1.0,
        r: 0.5,
        rounded: 4,
    };
    close(disc.measure().0, PI / 4.0);
    close(disc.measure().1, PI);
    close(disc.grown(0.25).unwrap_or(0.0), PI * 0.75 * 0.75);
    close(disc.grown(-0.25).unwrap_or(0.0), PI * 0.0625);
    // A slot 1 mm wide with sides of 3 mm.
    close(slot_width(3.0 + PI / 4.0, 6.0 + PI), 1.0);
    // A half disc of radius 1: area π/2, length π + 2.
    let half = Outline {
        start: [1.0, 0.0],
        segments: vec![
            Segment::Arc {
                to: [-1.0, 0.0],
                bulge: 1.0,
            },
            Segment::Line { to: [1.0, 0.0] },
        ],
    };
    close(measure(&half).0, PI / 2.0);
    close(measure(&half).1, PI + 2.0);
}

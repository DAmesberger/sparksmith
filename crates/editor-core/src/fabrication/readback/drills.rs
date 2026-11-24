//! The drill files, the drill map and the drill table.
use super::oracle::{self, DesignHole};
use super::{Fallible, MOST, Reading, distance};
use crate::geometry::Point;
use sparksmith_fabcheck::drills::{self, DrillHit, Feature};
use sparksmith_fabcheck::excellon;
use sparksmith_fabcheck::gerber::{Kind, unescape};
use std::collections::{BTreeMap, HashMap};

/// How far a hit may lie from its hole and its tool from its size, mm: the
/// drill files hold micrometres.
const HIT_MM: f64 = 0.0008;

/// A hole's tool: plated, size in µm, slot, use.
type Tool = (bool, i64, bool, &'static str);

fn tool(h: &DesignHole) -> Tool {
    let size = (h.width / 0.001).round() as i64;
    (h.plated, size, h.slot(), h.word())
}

fn feature(h: &DesignHole) -> Feature {
    if h.slot() {
        Feature::Slot {
            from: h.from,
            to: h.to,
        }
    } else {
        Feature::Round { at: h.from }
    }
}

pub(super) fn check(r: &mut Reading<'_, '_>) -> Fallible<()> {
    let project = r.project;
    let holes = oracle::holes(project)?;
    let layers = project.stackup.copper.len();
    let allowance = project.rules.profile.as_ref().map(|p| &p.drill_files);
    for (plated, suffix, function) in [
        (true, "plated.drl", format!("Plated,1,{layers},PTH")),
        (
            false,
            "non-plated.drl",
            format!("NonPlated,1,{layers},NPTH"),
        ),
    ] {
        let name = r.name(suffix);
        let design: Vec<&DesignHole> = holes.iter().filter(|h| h.plated == plated).collect();
        if design.is_empty() && !r.files.contains_key(name.as_str()) {
            continue;
        }
        let Some(text) = r.text(&name, true) else {
            continue;
        };
        let file = match excellon::parse(text) {
            Ok(file) => file,
            Err(e) => {
                r.unreadable(&name, &e.to_string());
                continue;
            }
        };
        if !file.metric {
            r.say(&name, None, "inches", "mm");
        }
        let stated = |k: &str| {
            file.attribute(k)
                .map(|v| v.iter().map(|v| unescape(v)).collect::<Vec<String>>())
        };
        r.head(&name, stated, &function, "Positive");
        // Every hole once, by what it is for.
        let mut read = 0;
        for use_ in ["ViaDrill", "ComponentDrill", "MechanicalDrill"] {
            let written = drills::hits(&file, |f| f.first().is_some_and(|f| f == use_))?;
            read += written.len();
            let wanted: Vec<DrillHit> = design
                .iter()
                .filter(|h| h.function() == use_)
                .map(|h| DrillHit {
                    feature: feature(h),
                    diameter: allowance.map_or(h.width, |a| a.file_size(h.width, plated)),
                })
                .collect();
            let diff = drills::pair(&written, &wanted, HIT_MM);
            for hit in diff.unmatched_a.iter().take(MOST) {
                let (at, words) = words(hit, use_, plated);
                r.say(&name, Some(at), &words, "no such hole");
            }
            for hit in diff.unmatched_b.iter().take(MOST) {
                let (at, words) = words(hit, use_, plated);
                r.say(&name, Some(at), "no such hole", &words);
            }
        }
        if read != file.hits.len() {
            let n = file.hits.len() - read;
            r.say(&name, None, &format!("{n} hits of no use"), "none");
        }
    }
    let table = table(r, &holes);
    map(r, &holes, &table)
}

fn words(hit: &DrillHit, use_: &str, plated: bool) -> (Point, String) {
    let plating = if plated { "plated" } else { "non-plated" };
    match hit.feature {
        Feature::Round { at } => (
            at,
            format!("a {plating} hole of {:.3} mm ({use_})", hit.diameter),
        ),
        Feature::Slot { from, to } => (
            from,
            format!(
                "a {plating} slot of {:.3} mm to ({:.3}, {:.3}) ({use_})",
                hit.diameter, to[0], to[1]
            ),
        ),
    }
}

/// The drill table against the holes; the map symbol of each tool.
fn table(r: &mut Reading<'_, '_>, holes: &[DesignHole]) -> BTreeMap<Tool, String> {
    let mut symbols = BTreeMap::new();
    let name = r.name("drill-table.txt");
    let Some(text) = r.text(&name, true) else {
        return symbols;
    };
    let mut left: BTreeMap<Tool, usize> = BTreeMap::new();
    for h in holes {
        *left.entry(tool(h)).or_default() += 1;
    }
    let mut numbers = Vec::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let [number, size, plating, kind, use_, count, symbol] = words.as_slice() else {
            continue;
        };
        let Some(number) = number.strip_prefix('T').and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let key: Option<Tool> = size.parse::<f64>().ok().and_then(|mm| {
            let use_ = ["via", "component", "mechanical"]
                .into_iter()
                .find(|u| u == use_)?;
            let size = (mm * 1e3).round() as i64;
            Some((*plating == "plated", size, *kind == "slot", use_))
        });
        let wanted = key.and_then(|k| left.remove(&k));
        if wanted.is_none() || wanted.map(|n| n.to_string()).as_deref() != Some(*count) {
            let wanted = wanted.map_or_else(|| "no such tool".into(), |n| format!("{n} of them"));
            let line = words.join(" ");
            r.say(&name, None, &format!("the tool “{line}”"), &wanted);
        }
        if let Some(key) = key {
            symbols.insert(key, (*symbol).to_owned());
            numbers.push((number, key));
        }
    }
    for ((plated, size, slot, use_), n) in left {
        let plating = if plated { "plated" } else { "non-plated" };
        let kind = if slot { "slots" } else { "holes" };
        let words = format!(
            "{n} {plating} {kind} of {:.3} mm ({use_})",
            size as f64 / 1e3
        );
        r.say(&name, None, "no such tool", &words);
    }
    tools(r, &name, &numbers);
    symbols
}

/// The table's tool numbers are the drill files' own.
fn tools(r: &mut Reading<'_, '_>, name: &str, numbers: &[(u32, Tool)]) {
    let allowance = r
        .project
        .rules
        .profile
        .as_ref()
        .map(|p| p.drill_files.clone());
    for (plated, suffix) in [(true, "plated.drl"), (false, "non-plated.drl")] {
        let drill = r.name(suffix);
        // One that cannot be read is told where its hits are checked.
        let Some(file) = r
            .files
            .get(drill.as_str())
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|t| excellon::parse(t).ok())
        else {
            continue;
        };
        for (number, size) in &file.tools {
            let stated = numbers.iter().find(|(n, _)| n == number).map(|(_, t)| *t);
            let fits = stated.is_some_and(|(p, um, _, _)| {
                let mm = um as f64 / 1e3;
                let mm = allowance.as_ref().map_or(mm, |a| a.file_size(mm, p));
                p == plated && (mm - size).abs() < HIT_MM
            });
            if !fits {
                let words = match stated {
                    Some((_, um, _, _)) => format!("T{number} as {:.3} mm", um as f64 / 1e3),
                    None => format!("no T{number}"),
                };
                r.say(
                    name,
                    None,
                    &words,
                    &format!("T{number} of {size:.3} mm in {drill}"),
                );
            }
        }
    }
}

/// A drawn piece of the drill map.
#[derive(Clone, Copy, Debug)]
enum Piece {
    Line(Point, Point),
    Circle(Point, f64),
}

impl Piece {
    /// Where a piece is filed: the middle of it.
    fn middle(&self) -> Point {
        match *self {
            Self::Line(a, b) => [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0],
            Self::Circle(c, _) => c,
        }
    }

    fn is(&self, other: &Self) -> bool {
        let close = |a: Point, b: Point| distance(a, b) < 1e-5;
        match (*self, *other) {
            (Self::Line(a, b), Self::Line(c, d)) => {
                close(a, c) && close(b, d) || close(a, d) && close(b, c)
            }
            (Self::Circle(a, r), Self::Circle(b, s)) => close(a, b) && (r - s).abs() < 1e-5,
            _ => false,
        }
    }
}

/// Pieces, found by where they are.
#[derive(Default)]
struct Pieces {
    at: HashMap<(i64, i64), Vec<Piece>>,
}

impl Pieces {
    const CELL_MM: f64 = 0.01;

    fn cell(p: Point) -> (i64, i64) {
        let k = |v: f64| (v / Self::CELL_MM).floor() as i64;
        (k(p[0]), k(p[1]))
    }

    fn put(&mut self, piece: Piece) {
        self.at
            .entry(Self::cell(piece.middle()))
            .or_default()
            .push(piece);
    }

    fn take(&mut self, piece: &Piece) -> bool {
        let (x, y) = Self::cell(piece.middle());
        for (dx, dy) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (dx, dy))) {
            if let Some(list) = self.at.get_mut(&(x + dx, y + dy))
                && let Some(k) = list.iter().position(|p| p.is(piece))
            {
                list.swap_remove(k);
                return true;
            }
        }
        false
    }

    fn first(&self) -> Option<Piece> {
        self.at.values().flatten().next().copied()
    }
}

/// A map symbol, 1 mm across, at `c`, by the name the drill table gives.
fn symbol(name: &str, c: Point) -> Vec<Piece> {
    let h = 0.5;
    let p = |x: f64, y: f64| [c[0] + x * h, c[1] + y * h];
    let ring = |points: &[Point]| -> Vec<Piece> {
        (0..points.len())
            .map(|k| Piece::Line(points[k], points[(k + 1) % points.len()]))
            .collect()
    };
    let plus = vec![
        Piece::Line(p(-1.0, 0.0), p(1.0, 0.0)),
        Piece::Line(p(0.0, -1.0), p(0.0, 1.0)),
    ];
    let ex = vec![
        Piece::Line(p(-1.0, -1.0), p(1.0, 1.0)),
        Piece::Line(p(-1.0, 1.0), p(1.0, -1.0)),
    ];
    let square = ring(&[p(-1.0, -1.0), p(1.0, -1.0), p(1.0, 1.0), p(-1.0, 1.0)]);
    let circle = vec![Piece::Circle(c, h)];
    match name {
        "circle" => circle,
        "cross" => plus,
        "x" => ex,
        "square" => square,
        "triangle" => ring(&[p(-1.0, -1.0), p(1.0, -1.0), p(0.0, 1.0)]),
        "diamond" => ring(&[p(0.0, -1.0), p(1.0, 0.0), p(0.0, 1.0), p(-1.0, 0.0)]),
        "circle-cross" => [circle, plus].concat(),
        "square-x" => [square, ex].concat(),
        _ => Vec::new(),
    }
}

/// Every hole on the map with its tool's symbol, the board's contours, and
/// nothing else.
fn map(
    r: &mut Reading<'_, '_>,
    holes: &[DesignHole],
    symbols: &BTreeMap<Tool, String>,
) -> Fallible<()> {
    if holes.is_empty() {
        return Ok(());
    }
    let Some((name, file)) = r.gerber("drill-map.gbr", "Drillmap", "Positive", true) else {
        return Ok(());
    };
    let mut drawn = Pieces::default();
    for object in &file.objects {
        match &object.kind {
            Kind::Draw { from, to } if object.dark => drawn.put(Piece::Line(*from, *to)),
            Kind::Arc {
                from, to, center, ..
            } if object.dark && distance(*from, *to) < 1e-9 => {
                drawn.put(Piece::Circle(*center, distance(*from, *center)));
            }
            // An arc of a contour, by its ends.
            Kind::Arc { from, to, .. } if object.dark => drawn.put(Piece::Line(*from, *to)),
            _ => r.say(&name, None, "an object that is no line", "lines only"),
        }
    }
    let mut lost = Vec::new();
    for hole in holes {
        let Some(shape) = symbols.get(&tool(hole)) else {
            continue;
        };
        let mut pieces = symbol(shape, hole.from);
        if hole.slot() {
            pieces.extend(symbol(shape, hole.to));
            pieces.push(Piece::Line(hole.from, hole.to));
        }
        if pieces.is_empty() || !pieces.iter().all(|p| drawn.take(p)) {
            lost.push((hole.from, format!("the symbol “{shape}” of {}", hole.what)));
        }
    }
    for (at, words) in lost.into_iter().take(MOST) {
        r.say(&name, Some(at), "no such symbol", &words);
    }
    for edge in oracle::contours(r.project, true)
        .iter()
        .flat_map(|c| &c.edges)
    {
        // A board edge shorter than the format draws nothing.
        if edge.length() > 2e-6 && !drawn.take(&Piece::Line(edge.from(), edge.to())) {
            r.say(
                &name,
                Some(edge.from()),
                "no such line",
                "an edge of the board",
            );
            break;
        }
    }
    if let Some(piece) = drawn.first() {
        r.say(&name, Some(piece.middle()), "a line", "none there");
    }
    Ok(())
}

//! What ends up on the board beyond the two-layer proof board: copper on
//! every layer of a four-layer board with two pours a layer drawn past the
//! board's edge, the legend of both sides, and what the fabricator drills
//! from (the drill map and table against the drill files, the job file
//! against the files). Each check with seeded faults it must report.
use super::Fallible;
use super::oracle::{self, Image};
use super::proof::{
    Files, SPACING_SLACK_MM, board, edit, file, layer_name, line_numbers, lines, mirrored, regions,
    shifted, without, written,
};
use super::{excellon_reader, reader};
use crate::Engine;
use crate::fabrication::{self, FabricationOptions, ProductionSnapshot};
use crate::geometry::Point;
use crate::ids::GraphicId;
use crate::library::{
    DrawingLayer, FootprintGraphic, Item, Justify, Rotation, Side, Text, TextContent,
};
use crate::model::fill::{FillSettings, IslandRemoval};
use crate::model::{Fill, Layer, Project};
use crate::rules::{CopperObject, RuleKey, RuleOverride, RuleValue};
use crate::tests::support::{nm, pt};
use sparksmith_fabcheck::outline;
use sparksmith_fabcheck::spacing::{self, Against};
use std::collections::BTreeMap;

/// The clearances of the layered board, mm. The via rule exceeds what the
/// drill rule leaves from the copper (0.20 - 0.15 mm), so it is the one
/// that binds; the edge rule binds at the outer outline, which every pour
/// is drawn past.
const RULES: [(CopperObject, f64); 8] = [
    (CopperObject::Track, 0.21),
    (CopperObject::Pad, 0.24),
    (CopperObject::Via, 0.30),
    (CopperObject::Fill, 0.26),
    (CopperObject::BoardEdge, 0.37),
    (CopperObject::ViaHole, 0.20),
    (CopperObject::PthHole, 0.28),
    (CopperObject::NpthHole, 0.31),
];

fn rule(object: CopperObject) -> f64 {
    RULES
        .iter()
        .find(|(o, _)| *o == object)
        .map_or(0.0, |(_, mm)| *mm)
}

fn against(against: Against) -> f64 {
    rule(match against {
        Against::Fill => CopperObject::Fill,
        Against::Track => CopperObject::Track,
        Against::Via => CopperObject::Via,
        Against::Pad => CopperObject::Pad,
        Against::ViaHole => CopperObject::ViaHole,
        Against::PlatedHole => CopperObject::PthHole,
        Against::UnplatedHole => CopperObject::NpthHole,
    })
}

/// The showcase's parts, tracks and vias on its four layers, with two pours
/// on every layer: ground on the left, another net with vias on the right,
/// overlapping by 2 mm and drawn 1 mm past the 44 by 28 mm board. `loose`
/// scales the clearance of fills to one kind of object.
fn layered(loose: Option<(CopperObject, f64)>) -> Fallible<Project> {
    let mut p = Engine::showcase()?.project().clone();
    let ground = p
        .nets
        .iter()
        .find(|n| n.name == "GND")
        .ok_or("the showcase's ground")?
        .id;
    let other = p
        .vias
        .iter()
        .find(|v| v.net != ground)
        .ok_or("a via of another net")?
        .net;
    for (object, mm) in RULES {
        let key = RuleKey::Clearance {
            a: CopperObject::Fill,
            b: object,
        }
        .normalized();
        let mm = match loose {
            Some((o, scale)) if o == object => mm * scale,
            _ => mm,
        };
        p.rules.overrides.retain(|o| o.key != key);
        p.rules.overrides.push(RuleOverride {
            key,
            value: RuleValue::Length(mm),
        });
    }
    p.fills.clear();
    for layer in p.stackup.layers() {
        for (net, x0, x1) in [(ground, -1.0, 23.0), (other, 21.0, 45.0)] {
            let id = p.allocate()?;
            p.fills.push(Fill {
                id,
                name: format!("Pour {id}"),
                net: Some(net),
                layer,
                region: Default::default(),
                outline: lines(&[[x0, -1.0], [x1, -1.0], [x1, 29.0], [x0, 29.0]]),
                priority: 0,
                settings: FillSettings {
                    islands: IslandRemoval::Never,
                    ..FillSettings::default()
                },
                locked: false,
                computed: None,
            });
        }
    }
    super::proof::refill(&mut p)?;
    Ok(p)
}

/// What is wrong with the copper files of `project`, each with the check
/// that says so. The images of the files `passed` holds unchanged are taken
/// as checked.
fn copper_faults(project: &Project, files: &Files, passed: &Files) -> Fallible<Vec<String>> {
    let mut out = Vec::new();
    let b = project.board().bounds();
    let window = [
        [b.min[0] - 2.0, b.min[1] - 2.0],
        [b.max[0] + 2.0, b.max[1] + 2.0],
    ];
    let mut layers = Vec::new();
    for layer in project.stackup.layers() {
        let (name, function) = layer_name(project, layer)?;
        let text = match file(files, &name) {
            Ok(text) => text,
            Err(e) => {
                out.push(format!("files: {e}"));
                continue;
            }
        };
        let parsed = reader::parse(text)?;
        let stated = parsed.file_attribute(".FileFunction").map(|f| f.join(","));
        if stated.as_deref() != Some(function.as_str()) {
            out.push(format!("files: {name} says it is {stated:?}"));
        }
        if file(passed, &name).ok() != Some(text) {
            let differs = oracle::differences(
                &Image::of(&parsed)?,
                &oracle::copper(project, layer),
                window,
                0.25,
            );
            if let Some(first) = differs.first() {
                out.push(format!(
                    "image: {name}: {first}, and {} more",
                    differs.len() - 1
                ));
            }
        }
        layers.push((name, parsed));
    }
    let parsed: Vec<_> = layers.iter().map(|(_, f)| f).collect();
    let mut drills = Vec::new();
    for (suffix, plated) in [("plated.drl", true), ("non-plated.drl", false)] {
        if let Ok(text) = file(files, suffix) {
            drills.push((excellon_reader::parse(text)?, plated));
        }
    }
    let drills: Vec<_> = drills.iter().map(|(f, p)| (f, *p)).collect();
    for s in spacing::fill_spacings(&parsed, &drills, 1.5)? {
        if s.gap_mm < against(s.against) - SPACING_SLACK_MM {
            out.push(format!("spacing {:?}: {s:?}", s.against));
        }
    }
    let profile = outline::contours(&reader::parse(file(files, "profile.gbr")?)?)?;
    for (name, layer) in &layers {
        if let Some(gap) = spacing::edge_gap(layer, &profile)?
            && gap < rule(CopperObject::BoardEdge) - SPACING_SLACK_MM
        {
            out.push(format!("edge: {name}: {gap} mm"));
        }
    }
    Ok(out)
}

#[test]
fn copper_on_every_layer_is_the_design_and_keeps_every_rule() -> Fallible<()> {
    let project = layered(None)?;
    assert_eq!(project.stackup.layers().len(), 4);
    let files = written(&project)?;
    let found = copper_faults(&project, &files, &Vec::new())?;
    assert!(found.is_empty(), "{found:#?}");
    // Every rule binds somewhere, on inner layers too where it can.
    let profile = outline::contours(&reader::parse(file(&files, "profile.gbr")?)?)?;
    let mut layers = Vec::new();
    for layer in project.stackup.layers() {
        layers.push(reader::parse(file(
            &files,
            &layer_name(&project, layer)?.0,
        )?)?);
    }
    for (k, layer) in layers.iter().enumerate() {
        let gap = spacing::edge_gap(layer, &profile)?.ok_or("fill copper")?;
        assert!(
            (gap - rule(CopperObject::BoardEdge)).abs() < 0.02,
            "layer {k}: {gap} mm from the edge"
        );
    }
    let plated = excellon_reader::parse(file(&files, "plated.drl")?)?;
    let all = spacing::fill_spacings(&layers.iter().collect::<Vec<_>>(), &[(&plated, true)], 1.5)?;
    for kind in [Against::Fill, Against::Via, Against::Pad, Against::Track] {
        let least = all
            .iter()
            .filter(|s| s.against == kind)
            .map(|s| s.gap_mm)
            .fold(f64::INFINITY, f64::min);
        assert!(
            least < against(kind) + 0.02,
            "{kind:?} never binds: {least} mm, rule {}",
            against(kind)
        );
    }
    for inner in ["Copper,L2,Inr", "Copper,L3,Inr"] {
        for kind in [Against::Fill, Against::Via] {
            assert!(
                all.iter().any(|s| s.layer == inner && s.against == kind),
                "{inner}: {kind:?} not measured"
            );
        }
    }
    Ok(())
}

#[test]
fn every_seeded_copper_fault_is_reported() -> Fallible<()> {
    let project = layered(None)?;
    let clean = written(&project)?;
    let report =
        |files: &Files| -> Fallible<Vec<String>> {
            // A damaged file the reader refuses is reported too.
            Ok(copper_faults(&project, files, &clean)
                .unwrap_or_else(|e| vec![format!("refused: {e}")]))
        };
    let mut missed = Vec::new();
    // Copper computed under looser rules, held to the board's own.
    for (object, by) in [
        (CopperObject::BoardEdge, "edge:"),
        (CopperObject::Fill, "spacing Fill:"),
        (CopperObject::Via, "spacing Via:"),
        (CopperObject::Pad, "spacing Pad:"),
        (CopperObject::Track, "spacing Track:"),
    ] {
        let found = report(&written(&layered(Some((object, 0.6)))?)?)?;
        if !found.iter().any(|f| f.starts_with(by)) {
            missed.push(format!("{object:?} at 0.6 of its rule: {found:?}"));
        }
    }
    let inner = "copper-l2-inner1.gbr";
    type Seed = Box<dyn Fn(&mut Files)>;
    let faults: Vec<(&str, &str, Seed)> = vec![
        (
            "an inner pour missing",
            "image:",
            Box::new(move |f: &mut Files| {
                edit(f, inner, |l| match regions(&l).first().cloned() {
                    Some(r) => without(l, r),
                    None => l,
                });
            }),
        ),
        (
            "an inner layer mirrored",
            "image:",
            Box::new(move |f: &mut Files| {
                edit(f, inner, |l| {
                    l.iter()
                        .map(|s| {
                            if s.starts_with('X') {
                                mirrored(s, 'X', 22.0, false)
                            } else {
                                s.clone()
                            }
                        })
                        .collect()
                });
            }),
        ),
        (
            "the inner layers swapped",
            "files:",
            Box::new(|f: &mut Files| {
                let at = |f: &Files, n: &str| f.iter().position(|x| x.0 == format!("proof-{n}"));
                if let (Some(i), Some(j)) =
                    (at(f, "copper-l2-inner1.gbr"), at(f, "copper-l3-inner2.gbr"))
                {
                    let text = f[i].1.clone();
                    f[i].1 = f[j].1.clone();
                    f[j].1 = text;
                }
            }),
        ),
        (
            "an inner layer missing",
            "files:",
            Box::new(move |f: &mut Files| f.retain(|x| x.0 != format!("proof-{inner}"))),
        ),
        (
            "a via missing on an inner layer",
            "image:",
            Box::new(move |f: &mut Files| {
                edit(f, inner, |l| {
                    let k = line_numbers(&l.join("\n"), |s| s.ends_with("D03*"));
                    match k.first().copied() {
                        Some(k) => without(l, k..=k),
                        None => l,
                    }
                });
            }),
        ),
    ];
    for (what, by, seed) in &faults {
        let mut files = clean.clone();
        seed(&mut files);
        assert!(files != clean, "{what} seeded nothing");
        let found = report(&files)?;
        if !found.iter().any(|f| f.starts_with(by)) {
            missed.push(format!("{what}: {found:?}"));
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
    Ok(())
}

/// The proof board with a legend on both sides and an asymmetric letter on
/// each part, clear of everything else it prints.
fn lettered() -> Fallible<Project> {
    let mut p = board()?;
    p.stackup.silkscreen_top = true;
    p.stackup.silkscreen_bottom = true;
    let footprint = &mut p
        .library
        .footprints
        .first_mut()
        .ok_or("the demo footprint")?
        .definition;
    footprint.graphics.push(FootprintGraphic {
        id: GraphicId(90),
        layer: DrawingLayer::Silkscreen,
        side: Side::Component,
        item: Item::Text(Text {
            content: TextContent::Literal("L".into()),
            at: pt([-8.0, 8.0]),
            height_nm: nm(1.5),
            rotation: Rotation::default(),
            justify: Justify::default(),
            visible: true,
            line_spacing: 1.5,
        }),
    });
    if let Some(second) = p.placements.get_mut(1) {
        second.rotation_deg = 0.0;
    }
    super::proof::refill(&mut p)?;
    Ok(p)
}

/// How far from its anchor a text may print: its box with its pen.
fn text_reach(text: &oracle::InkText) -> f64 {
    let wide = 0.6 * (text.characters as f64 + 1.0);
    let high = 2.0 * text.lines as f64;
    text.height * wide.hypot(high)
}

fn legend_faults(project: &Project, files: &Files) -> Fallible<Vec<String>> {
    let mut out = Vec::new();
    for (side, word, code) in [(Layer::Top, "top", "Top"), (Layer::Bottom, "bottom", "Bot")] {
        let name = format!("legend-{word}.gbr");
        let (inks, texts) = oracle::legend(project, side)?;
        if inks.is_empty() && texts.is_empty() {
            continue;
        }
        let text = match file(files, &name) {
            Ok(text) => text,
            Err(e) => {
                out.push(format!("files: {e}"));
                continue;
            }
        };
        let parsed = reader::parse(text)?;
        let stated = parsed.file_attribute(".FileFunction").map(|f| f.join(","));
        if stated != Some(format!("Legend,{code}")) {
            out.push(format!("files: {name} says it is {stated:?}"));
        }
        let image = Image::of(&parsed)?;
        // Every shape is printed, to its pen's edge.
        for ink in inks.iter().filter(|i| i.half > 0.0) {
            let lost = ink
                .edges
                .iter()
                .flat_map(|e| e.samples(0.05))
                .flat_map(|(p, right)| {
                    [-1.0, 0.0, 1.0].map(|s| {
                        let d = s * (ink.half - 0.005).max(0.0);
                        ink.frame
                            .to_board([p[0] + right[0] * d, p[1] + right[1] * d])
                    })
                })
                .filter(|p| !image.dark(*p))
                .count();
            if lost > 0 {
                out.push(format!("shape: {name}: {} lacks {lost} points", ink.what));
            }
        }
        // Every text prints something in its box.
        for t in &texts {
            let reach = t.height * 0.5;
            let printed = (-25..=25).any(|i| {
                (-25..=25).any(|j| {
                    image.dark([
                        t.at[0] + reach * f64::from(i) / 25.0,
                        t.at[1] + reach * f64::from(j) / 25.0,
                    ])
                })
            });
            if !printed {
                out.push(format!("text: {name}: {:?} is not printed", t.text));
            }
        }
        // And nothing else is.
        for object in image.object_points() {
            let strays = |p: &Point| {
                let on_shape = inks.iter().any(|ink| {
                    let q = ink.frame.to_pad(*p);
                    let d = ink
                        .edges
                        .iter()
                        .map(|e| e.distance(q))
                        .fold(f64::INFINITY, f64::min);
                    d <= ink.half + 0.005
                        || ink.filled && {
                            let mut area = oracle::Area::default();
                            area.edges.clone_from(&ink.edges);
                            area.inside(q)
                        }
                });
                let in_text = texts
                    .iter()
                    .any(|t| (p[0] - t.at[0]).hypot(p[1] - t.at[1]) <= text_reach(t));
                !on_shape && !in_text
            };
            // The box's centre of a bent stroke is not on it: its edges are.
            if let Some(p) = object.iter().skip(1).find(|p| strays(p)) {
                out.push(format!("extra: {name}: ink at {p:?}"));
                break;
            }
        }
    }
    Ok(out)
}

/// Which quarters of its own ink a letter at `at` fills: upper left, upper
/// right, lower left, lower right, as seen from the top.
fn quarters(image: &Image, at: Point) -> Fallible<[usize; 4]> {
    let mut dark = Vec::new();
    for i in -70..=70 {
        for j in -70..=70 {
            let p = [at[0] + 0.02 * f64::from(i), at[1] + 0.02 * f64::from(j)];
            if image.dark(p) {
                dark.push(p);
            }
        }
    }
    let bound =
        |k: usize, f: fn(f64, f64) -> f64, from: f64| dark.iter().map(|p| p[k]).fold(from, f);
    let middle = |k: usize| {
        (bound(k, f64::min, f64::INFINITY) + bound(k, f64::max, f64::NEG_INFINITY)) / 2.0
    };
    if dark.is_empty() {
        return Err("no letter".into());
    }
    let (x, y) = (middle(0), middle(1));
    let mut out = [0; 4];
    for p in &dark {
        let k = usize::from(p[0] > x) + if p[1] > y { 0 } else { 2 };
        out[k] += 1;
    }
    Ok(out)
}

#[test]
fn the_legend_prints_every_shape_and_text_and_nothing_else() -> Fallible<()> {
    let project = lettered()?;
    let files = written(&project)?;
    for side in [Layer::Top, Layer::Bottom] {
        let (inks, texts) = oracle::legend(&project, side)?;
        assert!(!inks.is_empty() && texts.len() >= 2, "{side:?}");
    }
    let found = legend_faults(&project, &files)?;
    assert!(found.is_empty(), "{found:#?}");
    // An L reads from the side it is printed on: its stem is on the left
    // from the top, and on the right, seen from the top, on the bottom.
    let letter = |side: Layer| -> Fallible<Point> {
        let (_, texts) = oracle::legend(&project, side)?;
        Ok(texts.iter().find(|t| t.text == "L").ok_or("the letter")?.at)
    };
    let top = Image::read(file(&files, "legend-top.gbr")?)?;
    let [ul, ur, ll, lr] = quarters(&top, letter(Layer::Top)?)?;
    assert!(
        ul > 20 && ll > 20 && lr > 20 && ur < ul / 4,
        "{ul} {ur} {ll} {lr}"
    );
    let bottom = Image::read(file(&files, "legend-bottom.gbr")?)?;
    let [ul, ur, ll, lr] = quarters(&bottom, letter(Layer::Bottom)?)?;
    assert!(
        ur > 20 && ll > 20 && lr > 20 && ul < ur / 4,
        "{ul} {ur} {ll} {lr}"
    );
    Ok(())
}

#[test]
fn every_seeded_legend_fault_is_reported() -> Fallible<()> {
    let project = lettered()?;
    let clean = written(&project)?;
    let name = "legend-top.gbr";
    type Seed = Box<dyn Fn(&mut Files)>;
    // The first stroke that is not in a region.
    let drop_stroke = move |f: &mut Files| {
        edit(f, name, |l| {
            let inside = regions(&l);
            let first = line_numbers(&l.join("\n"), |s| s.ends_with("D01*"))
                .into_iter()
                .find(|k| !inside.iter().any(|r| r.contains(k)));
            match first {
                Some(k) => without(l, k..=k),
                None => l,
            }
        });
    };
    // The header and apertures alone.
    let blank = move |f: &mut Files| {
        edit(f, name, |l| {
            l.into_iter()
                .filter(|s| {
                    ["%FS", "%MO", "%TF", "%AD", "G75", "M02"]
                        .iter()
                        .any(|keep| s.starts_with(keep))
                })
                .collect()
        });
    };
    let faults: Vec<(&str, &str, Seed)> = vec![
        (
            "the legend missing",
            "files:",
            Box::new(move |f: &mut Files| f.retain(|x| x.0 != format!("proof-{name}"))),
        ),
        ("a silkscreen line missing", "shape:", Box::new(drop_stroke)),
        ("every line and letter missing", "text:", Box::new(blank)),
        (
            "the legend mirrored",
            "shape:",
            Box::new(move |f: &mut Files| {
                edit(f, name, |l| {
                    l.iter()
                        .map(|s| match s.as_str() {
                            "G02*" => "G03*".to_owned(),
                            "G03*" => "G02*".to_owned(),
                            _ if s.starts_with('X') => {
                                mirrored(&mirrored(s, 'X', 30.0, false), 'I', 0.0, false)
                            }
                            _ => s.clone(),
                        })
                        .collect()
                });
            }),
        ),
        (
            "the sides swapped",
            "files:",
            Box::new(|f: &mut Files| {
                let at = |f: &Files, n: &str| f.iter().position(|x| x.0 == format!("proof-{n}"));
                if let (Some(i), Some(j)) = (at(f, "legend-top.gbr"), at(f, "legend-bottom.gbr")) {
                    let text = f[i].1.clone();
                    f[i].1 = f[j].1.clone();
                    f[j].1 = text;
                }
            }),
        ),
        (
            "ink that is not the design's",
            "extra:",
            Box::new(move |f: &mut Files| {
                edit(f, name, |l| {
                    l.join("\n")
                        .replacen(
                            "M02*",
                            "X50000000Y36000000D02*\nG01*\nX52000000D01*\nM02*",
                            1,
                        )
                        .lines()
                        .map(str::to_owned)
                        .collect()
                });
            }),
        ),
    ];
    let mut missed = Vec::new();
    for (what, by, seed) in &faults {
        let mut files = clean.clone();
        seed(&mut files);
        assert!(files != clean, "{what} seeded nothing");
        let found =
            legend_faults(&project, &files).unwrap_or_else(|e| vec![format!("refused: {e}")]);
        if !found.iter().any(|f| f.starts_with(by)) {
            missed.push(format!("{what}: {found:?}"));
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
    Ok(())
}

/// Every text file of the set: the layers, the drill files, the drill table
/// and the job file.
fn written_all(project: &Project) -> Fallible<Files> {
    let set = fabrication::generate(
        &ProductionSnapshot::resolve(project, None)?,
        &FabricationOptions {
            base_name: Some("proof".into()),
            ..FabricationOptions::default()
        },
        &Default::default(),
    )?;
    set.files
        .into_iter()
        .filter(|f| {
            [".gbr", ".drl", ".txt", ".gbrjob"]
                .iter()
                .any(|e| f.name.ends_with(e))
        })
        .map(|f| Ok((f.name, String::from_utf8(f.bytes)?)))
        .collect()
}

/// A drawn piece of the drill map.
#[derive(Clone, Copy, Debug)]
enum Piece {
    Line(Point, Point),
    Circle(Point, f64),
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

impl Piece {
    fn is(&self, other: &Self) -> bool {
        // The drill files hold micrometres, the map nanometres.
        let close = |a: Point, b: Point| distance(a, b) < 0.0015;
        match (*self, *other) {
            (Self::Line(a, b), Self::Line(c, d)) => {
                close(a, c) && close(b, d) || close(a, d) && close(b, c)
            }
            (Self::Circle(a, r), Self::Circle(b, s)) => close(a, b) && (r - s).abs() < 0.0015,
            _ => false,
        }
    }
}

/// A map symbol, 1 mm across, at `c`, by the name the drill table gives.
fn symbol(name: &str, c: Point) -> Fallible<Vec<Piece>> {
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
    Ok(match name {
        "circle" => circle,
        "cross" => plus,
        "x" => ex,
        "square" => square,
        "triangle" => ring(&[p(-1.0, -1.0), p(1.0, -1.0), p(0.0, 1.0)]),
        "diamond" => ring(&[p(0.0, -1.0), p(1.0, 0.0), p(0.0, 1.0), p(-1.0, 0.0)]),
        "circle-cross" => [circle, plus].concat(),
        "square-x" => [square, ex].concat(),
        other => return Err(format!("no symbol {other:?}").into()),
    })
}

/// What is wrong with the drill map and table as the map and table of the
/// drill files: every tool of the files in the table with its size, plating,
/// kind and count; every hit on the map with its tool's symbol; nothing on
/// the map besides but the board's outline, cutouts and slots.
fn map_faults(project: &Project, files: &Files) -> Fallible<Vec<String>> {
    let mut out = Vec::new();
    // T, diameter, plating, kind, function, count, symbol.
    let mut table: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for line in file(files, "drill-table.txt")?.lines() {
        let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        if let (7, Some(number)) = (
            words.len(),
            words
                .first()
                .and_then(|w| w.strip_prefix('T'))
                .and_then(|n| n.parse().ok()),
        ) {
            table.insert(number, words);
        }
    }
    let mut want: Vec<Piece> = Vec::new();
    let mut used = Vec::new();
    for (suffix, plating) in [("plated.drl", "plated"), ("non-plated.drl", "non-plated")] {
        let Ok(text) = file(files, suffix) else {
            continue;
        };
        let drill = excellon_reader::parse(text)?;
        for (number, diameter) in &drill.tools {
            let hits: Vec<_> = drill.hits.iter().filter(|h| h.tool() == *number).collect();
            let Some(row) = table.get(number) else {
                out.push(format!("table: T{number} is not in it"));
                continue;
            };
            used.push(*number);
            let slot = hits
                .iter()
                .any(|h| matches!(h, excellon_reader::Hit::Slot { .. }));
            let kind = if slot { "slot" } else { "hit" };
            let size: f64 = row[1].parse()?;
            if (size - diameter).abs() > 1e-9
                || row[2] != plating
                || row[3] != kind
                || row[5] != hits.len().to_string()
            {
                out.push(format!(
                    "table: T{number} is {row:?}, drilled {diameter} {plating} {kind} {}",
                    hits.len()
                ));
            }
            for hit in hits {
                match *hit {
                    excellon_reader::Hit::Round { at, .. } => want.extend(symbol(&row[6], at)?),
                    excellon_reader::Hit::Slot { from, to, .. } => {
                        want.extend(symbol(&row[6], from)?);
                        want.extend(symbol(&row[6], to)?);
                        want.push(Piece::Line(from, to));
                    }
                }
            }
        }
    }
    for number in table.keys().filter(|n| !used.contains(n)) {
        out.push(format!("table: T{number} drills nothing"));
    }
    let mut left: Vec<Piece> = Vec::new();
    for object in &reader::parse(file(files, "drill-map.gbr")?)?.objects {
        match &object.kind {
            reader::Kind::Draw { from, to } => left.push(Piece::Line(*from, *to)),
            reader::Kind::Arc {
                from, to, center, ..
            } if distance(*from, *to) < 1e-9 => {
                left.push(Piece::Circle(*center, distance(*from, *center)));
            }
            // An arc of a contour, by its ends.
            reader::Kind::Arc { from, to, .. } => left.push(Piece::Line(*from, *to)),
            other => out.push(format!("map: {other:?}")),
        }
    }
    let mut lost = 0;
    for piece in &want {
        match left.iter().position(|p| p.is(piece)) {
            Some(k) => {
                left.swap_remove(k);
            }
            None => lost += 1,
        }
    }
    if lost > 0 {
        out.push(format!("map: {lost} of {} pieces missing", want.len()));
    }
    // The board: every edge of every contour, and nothing else.
    let mut edges: Vec<Piece> = oracle::contours(project, true)
        .iter()
        .flat_map(|c| &c.edges)
        .map(|e| Piece::Line(e.from(), e.to()))
        .collect();
    left.retain(|p| match edges.iter().position(|e| e.is(p)) {
        Some(k) => {
            edges.swap_remove(k);
            false
        }
        None => true,
    });
    if !left.is_empty() || !edges.is_empty() {
        out.push(format!(
            "map: extra {left:?}, board edges not drawn {edges:?}"
        ));
    }
    Ok(out)
}

/// What is wrong with the job file as the description of the files: each
/// layer and drill file once, with the function and polarity it states
/// itself, and the board's size, layers and thickness from the document.
fn job_faults(project: &Project, files: &Files) -> Fallible<Vec<String>> {
    let mut out = Vec::new();
    let (_, text) = files
        .iter()
        .find(|(name, _)| name == "proof.gbrjob")
        .ok_or("the job file")?;
    let job: serde_json::Value = serde_json::from_str(text)?;
    let mut listed: BTreeMap<String, (String, String)> = BTreeMap::new();
    for f in job["FilesAttributes"].as_array().ok_or("files")? {
        let word = |k: &str| f[k].as_str().unwrap_or_default().to_owned();
        if listed
            .insert(word("Path"), (word("FileFunction"), word("FilePolarity")))
            .is_some()
        {
            out.push(format!("files: {} listed twice", word("Path")));
        }
    }
    for (name, text) in files {
        let (function, polarity) = if name.ends_with(".gbr") {
            let parsed = reader::parse(text)?;
            let attribute = |k: &str| parsed.file_attribute(k).map(|v| v.join(","));
            (attribute(".FileFunction"), attribute(".FilePolarity"))
        } else if name.ends_with(".drl") {
            let parsed = excellon_reader::parse(text)?;
            let attribute = |k: &str| parsed.attribute(k).map(|v| v.join(","));
            (attribute(".FileFunction"), attribute(".FilePolarity"))
        } else {
            continue;
        };
        let (Some(function), Some(polarity)) = (function, polarity) else {
            out.push(format!("files: {name} states no function or polarity"));
            continue;
        };
        // A solder mask file holds its openings; every other layer its
        // material.
        let right = if function.starts_with("Soldermask") {
            "Negative"
        } else {
            "Positive"
        };
        if polarity != right {
            out.push(format!("polarity: {name} is {polarity}"));
        }
        match listed.remove(name) {
            None => out.push(format!("files: {name} is not listed")),
            Some(stated) if stated != (function.clone(), polarity.clone()) => {
                out.push(format!("files: {name} is listed as {stated:?}"));
            }
            Some(_) => {}
        }
    }
    for name in listed.keys() {
        out.push(format!("files: {name} is listed and not written"));
    }
    let b = project.board().bounds();
    let stack = &project.stackup;
    let thickness: f64 = stack
        .copper
        .iter()
        .map(|c| c.thickness_um / 1000.0)
        .chain(stack.dielectrics.iter().map(|d| d.thickness_mm))
        .sum();
    let specs = &job["GeneralSpecs"];
    for (what, stated, design) in [
        ("width", &specs["Size"]["X"], b.max[0] - b.min[0]),
        ("height", &specs["Size"]["Y"], b.max[1] - b.min[1]),
        ("layers", &specs["LayerNumber"], stack.copper.len() as f64),
        ("thickness", &specs["BoardThickness"], thickness),
    ] {
        if !stated.as_f64().is_some_and(|v| (v - design).abs() < 2e-6) {
            out.push(format!("board: {what} is {stated}, {design} in the design"));
        }
    }
    let foils: Vec<f64> = job["MaterialStackup"]
        .as_array()
        .ok_or("the stackup")?
        .iter()
        .filter(|m| m["Type"] == "Copper")
        .filter_map(|m| m["Thickness"].as_f64())
        .collect();
    let design: Vec<f64> = stack
        .copper
        .iter()
        .map(|c| c.thickness_um / 1000.0)
        .collect();
    if foils.len() != design.len() || foils.iter().zip(&design).any(|(a, b)| (a - b).abs() > 2e-6) {
        out.push(format!("board: foils {foils:?}, {design:?} in the design"));
    }
    Ok(out)
}

#[test]
fn the_drill_map_the_table_and_the_job_file_describe_the_files() -> Fallible<()> {
    let project = board()?;
    let files = written_all(&project)?;
    let found = map_faults(&project, &files)?;
    assert!(found.is_empty(), "{found:#?}");
    let found = job_faults(&project, &files)?;
    assert!(found.is_empty(), "{found:#?}");
    Ok(())
}

#[test]
fn every_seeded_fault_of_what_is_drilled_from_is_reported() -> Fallible<()> {
    let project = board()?;
    let clean = written_all(&project)?;
    type Seed = Box<dyn Fn(&mut Files)>;
    // The `k`th line from the end that ends a draw of the map.
    let map_draw = |change: fn(&str) -> Option<String>| {
        move |f: &mut Files| {
            edit(f, "drill-map.gbr", move |l| {
                let last = line_numbers(&l.join("\n"), |s| s.ends_with("D01*"))
                    .last()
                    .copied();
                l.into_iter()
                    .enumerate()
                    .filter_map(|(k, s)| if Some(k) == last { change(&s) } else { Some(s) })
                    .collect()
            });
        }
    };
    let replace = |suffix: &'static str, from: &'static str, to: &'static str| {
        move |f: &mut Files| {
            edit(f, suffix, |l| {
                l.join("\n")
                    .replacen(from, to, 1)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            });
        }
    };
    // The job file's own name has no dash.
    let job = |from: &'static str, to: &'static str| {
        move |f: &mut Files| {
            for (name, text) in f.iter_mut() {
                if name == "proof.gbrjob" {
                    *text = text.replacen(from, to, 1);
                }
            }
        }
    };
    let faults: Vec<(&str, &str, Seed)> = vec![
        (
            "a piece of a map symbol missing",
            "map:",
            Box::new(map_draw(|_| None)),
        ),
        (
            "a map symbol drawn 0.1 mm off",
            "map:",
            Box::new(|f: &mut Files| {
                edit(f, "drill-map.gbr", |mut l| {
                    if let Some(k) =
                        line_numbers(&l.join("\n"), |s| s.starts_with('X') && s.ends_with("D02*"))
                            .last()
                    {
                        l[*k] = shifted(&l[*k], 'X', 0.1, false);
                    }
                    l
                });
            }),
        ),
        (
            "a hole on the map that is not drilled",
            "map:",
            Box::new(|f: &mut Files| {
                edit(f, "plated.drl", |l| {
                    let k = line_numbers(&l.join("\n"), |s| s.starts_with('X'));
                    match k.first().copied() {
                        Some(k) => without(l, k..=k),
                        None => l,
                    }
                });
            }),
        ),
        (
            "a tool of another size than the table says",
            "table:",
            Box::new(replace("plated.drl", "C0.300", "C0.350")),
        ),
        (
            "plated tools the table calls non-plated",
            "table:",
            Box::new(|f: &mut Files| {
                edit(f, "drill-table.txt", |l| {
                    l.into_iter()
                        .map(|s| {
                            if s.starts_with('T') {
                                s.replacen(" plated ", " non-plated ", 1)
                            } else {
                                s
                            }
                        })
                        .collect()
                });
            }),
        ),
        (
            "another symbol in the table than on the map",
            "map:",
            Box::new(|f: &mut Files| {
                edit(f, "drill-table.txt", |l| {
                    l.into_iter()
                        .map(|s| {
                            if s.starts_with("T1 ") && s.ends_with(" circle") {
                                s.replace(" circle", " diamond")
                            } else {
                                s
                            }
                        })
                        .collect()
                });
            }),
        ),
        (
            "a file the job file names wrongly",
            "files:",
            Box::new(job("proof-profile.gbr", "proof-outline.gbr")),
        ),
        (
            "a layer the job file calls another",
            "files:",
            Box::new(job("Copper,L1,Top", "Copper,L2,Bot")),
        ),
        (
            "a mask the job file calls positive",
            "files:",
            Box::new(job("\"Negative\"", "\"Positive\"")),
        ),
        (
            "a mask that says it is positive",
            "polarity:",
            Box::new(replace(
                "mask-top.gbr",
                "%TF.FilePolarity,Negative*%",
                "%TF.FilePolarity,Positive*%",
            )),
        ),
        (
            "another layer count",
            "board:",
            Box::new(job("\"LayerNumber\": 2", "\"LayerNumber\": 4")),
        ),
        (
            "another board width",
            "board:",
            Box::new(job("\"X\": 60", "\"X\": 61")),
        ),
    ];
    let mut missed = Vec::new();
    for (what, by, seed) in &faults {
        let mut files = clean.clone();
        seed(&mut files);
        assert!(files != clean, "{what} seeded nothing");
        let mut found =
            map_faults(&project, &files).unwrap_or_else(|e| vec![format!("refused: {e}")]);
        found.extend(job_faults(&project, &files)?);
        if !found.iter().any(|f| f.starts_with(by)) {
            missed.push(format!("{what}: {found:?}"));
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
    Ok(())
}

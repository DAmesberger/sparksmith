//! The Gerber layers: copper, mask, paste, legend and profile.
use super::oracle::{self, Image};
use super::{Fallible, MOST, Reading, distance, image, nm};
use crate::geometry::Point;
use crate::model::{CopperLayer, Layer};
use sparksmith_fabcheck::gerber::{GerberFile, Kind, Object, Template};
use sparksmith_fabcheck::{outline, polygons};
use std::collections::BTreeMap;

/// How far apart the profile's contours may lie, mm.
const PROFILE_MM: f64 = 0.0005;
/// How far a pad's flash may lie from its centre, mm: the format's step.
const FLASH_MM: f64 = 5e-6;

pub(super) fn check(r: &mut Reading<'_, '_>) -> Fallible<()> {
    for (k, layer) in r.project.stackup.layers().into_iter().enumerate() {
        let n = k + 1;
        let (suffix, side) = match layer {
            CopperLayer::Top => (format!("copper-l{n}-top.gbr"), "Top"),
            CopperLayer::Bottom => (format!("copper-l{n}-bottom.gbr"), "Bot"),
            CopperLayer::Inner(i) => (format!("copper-l{n}-inner{i}.gbr"), "Inr"),
        };
        let function = format!("Copper,L{n},{side}");
        if let Some((name, file)) = r.gerber(&suffix, &function, "Positive", true) {
            copper(r, &name, &file, layer)?;
            objects(r, &name, &file, layer);
        }
    }
    let project = r.project;
    let stack = &project.stackup;
    for (side, word, code, masked, printed) in [
        (
            Layer::Top,
            "top",
            "Top",
            stack.mask_top,
            stack.silkscreen_top,
        ),
        (
            Layer::Bottom,
            "bottom",
            "Bot",
            stack.mask_bottom,
            stack.silkscreen_bottom,
        ),
    ] {
        if masked {
            let function = format!("Soldermask,{code}");
            let file = r.gerber(&format!("mask-{word}.gbr"), &function, "Negative", true);
            if let Some((name, file)) = file {
                let design = oracle::mask(r.project, side);
                openings(r, &name, &file, &design, "mask opening")?;
            }
        }
        // A side without paste or ink has no file: its file must be missing
        // only when there is nothing to print.
        let design = oracle::paste(r.project, side)?;
        let suffix = format!("paste-{word}.gbr");
        let file = r.gerber(&suffix, &format!("Paste,{code}"), "Positive", false);
        let (name, file) = file.unwrap_or_else(|| (r.name(&suffix), GerberFile::default()));
        openings(r, &name, &file, &design, "paste")?;
        if printed {
            legend(r, side, word, code)?;
        }
    }
    profile(r)
}

/// The image of a copper layer, every point of it.
fn copper(
    r: &mut Reading<'_, '_>,
    name: &str,
    file: &GerberFile,
    layer: CopperLayer,
) -> Fallible<()> {
    let prims = oracle::copper_but_fills(r.project, layer);
    let mut design = polygons::Shapes::new();
    for fill in r.project.fills.iter().filter(|f| f.layer == layer) {
        for region in fill.computed.iter().flat_map(|c| &c.regions) {
            design.extend(image::region(region)?);
        }
    }
    for prim in &prims {
        design.extend(image::solid(prim)?);
    }
    if !r.going(name) {
        return Ok(());
    }
    let written = match polygons::render(file, |_| true) {
        Ok(shapes) => shapes,
        Err(e) => {
            r.unreadable(name, &e.to_string());
            return Ok(());
        }
    };
    if !r.going(name) {
        return Ok(());
    }
    let what = |p: Point| {
        prims
            .iter()
            .find(|prim| prim.excess(p) <= 0.0)
            .map_or_else(|| "a fill".to_owned(), |prim| prim.what.clone())
    };
    let wrong = image::compare(&written, &design, what, MOST)?;
    r.wrong(name, wrong, "copper");
    Ok(())
}

/// What an object of a copper file is: kind, where and how large in nm, net.
type Piece = (&'static str, [i64; 4], i64, String);

fn track(a: Point, b: Point, width: f64, net: &str) -> Piece {
    let (a, b) = ([nm(a[0]), nm(a[1])], [nm(b[0]), nm(b[1])]);
    let (a, b) = (a.min(b), a.max(b));
    ("track", [a[0], a[1], b[0], b[1]], nm(width), net.to_owned())
}

fn round(kind: &'static str, at: Point, diameter: f64, net: &str) -> Piece {
    (
        kind,
        [nm(at[0]), nm(at[1]), 0, 0],
        nm(diameter),
        net.to_owned(),
    )
}

/// A pad in the file or the design: part, label and net.
type PadName = (String, String, String);

/// Every track, via, washer and pad of a copper layer once, on its net, and
/// no other object.
fn objects(r: &mut Reading<'_, '_>, name: &str, file: &GerberFile, layer: CopperLayer) {
    let project = r.project;
    let mut left: BTreeMap<Piece, i64> = BTreeMap::new();
    let mut count = |piece: Piece, by: i64| *left.entry(piece).or_default() += by;
    for t in project.tracks.iter().filter(|t| t.layer == layer) {
        count(track(t.start, t.end, t.width, project.net_name(t.net)), 1);
    }
    for v in &project.vias {
        count(
            round("via", v.position, v.diameter, project.net_name(v.net)),
            1,
        );
    }
    for h in &project.board().holes {
        if let (true, Some(d)) = (h.plated, h.pad_diameter) {
            count(round("washer", h.at, d, ""), 1);
        }
    }
    let labels = crate::layout::pad_labels(project);
    let nets: BTreeMap<_, _> = crate::layout::placed_pads(project)
        .into_iter()
        .map(|p| ((p.instance, p.pad.id), p.net))
        .collect();
    let mut pads: BTreeMap<PadName, Vec<Point>> = BTreeMap::new();
    for pad in oracle::pads(project) {
        if !oracle::has_copper(&pad, layer) {
            continue;
        }
        let key = (pad.instance, pad.pad.id);
        let net = nets.get(&key).copied().flatten();
        let part = project
            .instance(pad.instance)
            .map_or_else(String::new, |i| i.designator.clone());
        pads.entry((
            part,
            labels.get(&key).cloned().unwrap_or_default(),
            net.map_or_else(String::new, |n| project.net_name(n).to_owned()),
        ))
        .or_default()
        .push(pad.frame.to_board([0.0; 2]));
    }
    let fills: Vec<&str> = project
        .fills
        .iter()
        .filter(|f| f.layer == layer)
        .map(|f| f.net.map_or("", |n| project.net_name(n)))
        .collect();
    let mut wrong: Vec<(Option<Point>, String, String)> = Vec::new();
    let mut regions: Vec<PadName> = Vec::new();
    for object in file.objects.iter().filter(|o| o.dark) {
        let first = |k: &str| {
            object
                .object_attributes
                .get(k)
                .and_then(|v| v.first())
                .map_or("", String::as_str)
        };
        let net = object
            .object_attributes
            .get(".N")
            .map(|v| v.join(","))
            .unwrap_or_default();
        let function = object
            .aperture_attributes
            .get(".AperFunction")
            .and_then(|v| v.first())
            .map_or("", String::as_str);
        let diameter = || diameter(file, object);
        match (function, &object.kind) {
            ("Conductor", Kind::Region { contour }) => {
                if !fills.contains(&net.as_str()) {
                    let what = format!("fill copper on net “{net}”");
                    wrong.push((Some(contour.start), what, "no fill on that net".into()));
                }
            }
            ("EtchedComponent", Kind::Region { .. }) => {}
            ("Conductor", Kind::Draw { from, to }) => {
                count(track(*from, *to, diameter(), &net), -1);
            }
            // A track shorter than the format is a dot.
            ("Conductor", Kind::Flash { at }) => count(track(*at, *at, diameter(), &net), -1),
            ("ViaPad", Kind::Flash { at }) => count(round("via", *at, diameter(), &net), -1),
            ("WasherPad", Kind::Flash { at }) => {
                count(round("washer", *at, diameter(), ""), -1);
            }
            (_, kind) if object.object_attributes.contains_key(".C") => {
                let label = object
                    .object_attributes
                    .get(".P")
                    .and_then(|v| v.get(1))
                    .cloned()
                    .unwrap_or_default();
                let key = (first(".C").to_owned(), label, net);
                match kind {
                    Kind::Flash { at } => {
                        let at = *at;
                        let nearest = pads.get_mut(&key).and_then(|l| {
                            let k = l.iter().position(|p| distance(*p, at) <= FLASH_MM)?;
                            Some(l.swap_remove(k))
                        });
                        if nearest.is_none() {
                            let (part, label, net) = key;
                            wrong.push((
                                Some(at),
                                format!("{part} pad {label} on net “{net}”"),
                                "no such pad there".into(),
                            ));
                        }
                    }
                    _ => regions.push(key),
                }
            }
            (other, _) => wrong.push((
                None,
                format!("an object of the function “{other}”"),
                "none".into(),
            )),
        }
    }
    // A land drawn as a region has no centre of its own: the image tells
    // where it is, this that it is there.
    for key in regions {
        if pads.get_mut(&key).and_then(Vec::pop).is_none() {
            let (part, label, net) = key;
            wrong.push((
                None,
                format!("{part} pad {label} on net “{net}”"),
                "no such pad".into(),
            ));
        }
    }
    for ((part, label, net), at) in &pads {
        for p in at {
            wrong.push((
                Some(*p),
                "no such pad".into(),
                format!("{part} pad {label} on net “{net}”"),
            ));
        }
    }
    for ((kind, at, size, net), n) in &left {
        let words = format!("a {kind} of {} mm on net “{net}”", *size as f64 / 1e6);
        let at = Some([at[0] as f64 / 1e6, at[1] as f64 / 1e6]);
        match n {
            0 => {}
            n if *n > 0 => wrong.push((at, format!("no such {kind}"), words)),
            _ => wrong.push((at, words, format!("no such {kind}"))),
        }
    }
    for (at, written, design) in wrong.into_iter().take(2 * MOST) {
        r.say(name, at, &written, &design);
    }
}

/// The size of an object's round aperture (none reads as nothing).
fn diameter(file: &GerberFile, object: &Object) -> f64 {
    match object
        .aperture
        .and_then(|a| file.apertures.get(&a))
        .map(|a| &a.template)
    {
        Some(Template::Circle {
            diameter,
            hole: None,
        }) => *diameter,
        _ => -1.0,
    }
}

/// Mask and paste: points either side of every edge.
fn openings(
    r: &mut Reading<'_, '_>,
    name: &str,
    file: &GerberFile,
    design: &[oracle::Prim],
    material: &str,
) -> Fallible<()> {
    let image = match Image::of(file) {
        Ok(image) => image,
        Err(e) => {
            r.unreadable(name, &e.to_string());
            return Ok(());
        }
    };
    let wrong = oracle::openings(&image, design, MOST, &mut || (r.go)(name))?;
    r.wrong(name, wrong, material);
    Ok(())
}

/// The legend: every shape and every stroke of every text, and nothing else.
fn legend(r: &mut Reading<'_, '_>, side: Layer, word: &str, code: &str) -> Fallible<()> {
    let mut inks = oracle::silkscreen(r.project, side)?;
    inks.extend(oracle::lettering(r.project, side));
    let mut design = polygons::Shapes::new();
    for ink in &inks {
        design.extend(image::ink(ink)?);
    }
    let suffix = format!("legend-{word}.gbr");
    let function = format!("Legend,{code}");
    let file = r.gerber(&suffix, &function, "Positive", !design.is_empty());
    let Some((name, file)) = file else {
        return Ok(());
    };
    let written = match polygons::render(&file, |_| true) {
        Ok(shapes) => shapes,
        Err(e) => {
            r.unreadable(&name, &e.to_string());
            return Ok(());
        }
    };
    let what = |p: Point| {
        inks.iter()
            .find(|ink| {
                let q = ink.frame.to_pad(p);
                ink.edges.iter().any(|e| e.distance(q) <= ink.half)
            })
            .map_or_else(String::new, |ink| ink.what.clone())
    };
    let wrong = image::compare(&written, &design, what, MOST)?;
    r.wrong(&name, wrong, "ink");
    Ok(())
}

/// Every contour of the profile, both ways.
fn profile(r: &mut Reading<'_, '_>) -> Fallible<()> {
    let Some((name, file)) = r.gerber("profile.gbr", "Profile,NP", "Positive", true) else {
        return Ok(());
    };
    let drawn = match outline::contours(&file) {
        Ok(drawn) => drawn,
        Err(e) => {
            r.unreadable(&name, &e.to_string());
            return Ok(());
        }
    };
    let diff = outline::compare(&drawn, &oracle::profile(r.project), PROFILE_MM);
    let start = |c: &outline::Contour| c.edges.first().map(outline::Edge::from);
    let length = |c: &outline::Contour| format!("a contour {:.3} mm long", c.length());
    for c in diff.only_a.iter().take(MOST) {
        r.say(&name, start(c), &length(c), "no such contour");
    }
    for c in diff.only_b.iter().take(MOST) {
        r.say(&name, start(c), "no such contour", &length(c));
    }
    if diff.matched == 0 && diff.only_a.is_empty() && diff.only_b.is_empty() {
        r.say(&name, None, "no contour", "a board outline");
    }
    Ok(())
}

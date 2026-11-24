//! The job file, the pick-and-place files and the bills of materials.
use super::oracle::{self, Edge};
use super::{Fallible, MOST, Reading, distance};
use crate::completeness;
use crate::fabrication::{BottomRotation, PlacementOrigin, assembly, job as writer};
use crate::geometry::Point;
use crate::model::{DielectricKind, Layer, Project};
use crate::rules::CopperObject;
use serde_json::Value;
use sparksmith_fabcheck::{csv, excellon, gerber};
use std::collections::BTreeMap;

/// A length of the job file, mm.
const JOB_MM: f64 = 2e-6;
/// The board's size, whose arcs are flattened here, mm.
const SIZE_MM: f64 = 5e-4;
/// A pick-and-place coordinate (four decimals), mm, and an angle.
const PLACE_MM: f64 = 6e-5;
const PLACE_DEG: f64 = 6e-4;

/// The box of the board's outline.
pub(super) fn outline_box(project: &Project) -> [Point; 2] {
    let mut b = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    let outer = oracle::Area::of_outline(&project.board().outer);
    for p in outer.edges.iter().flat_map(|e: &Edge| e.flattened(1e-4)) {
        for k in 0..2 {
            b[0][k] = b[0][k].min(p[k]);
            b[1][k] = b[1][k].max(p[k]);
        }
    }
    b
}

/// The job file as the description of the files and the board.
pub(super) fn job(r: &mut Reading<'_, '_>) -> Fallible<()> {
    let name = format!("{}.gbrjob", r.stem);
    let Some(text) = r.text(&name, true) else {
        return Ok(());
    };
    let job: Value = match serde_json::from_str(text) {
        Ok(job) => job,
        Err(e) => {
            r.unreadable(&name, &e.to_string());
            return Ok(());
        }
    };
    listed(r, &name, &job);
    let project = r.project;
    let identity = r.identity;
    let specs = &job["GeneralSpecs"];
    let date = r.options.creation_date.clone();
    for (what, stated, design) in [
        (
            "the project",
            &specs["ProjectId"]["Name"],
            Some(identity.name.clone()),
        ),
        (
            "the project id",
            &specs["ProjectId"]["GUID"],
            Some(identity.guid.clone()),
        ),
        (
            "the revision",
            &specs["ProjectId"]["Revision"],
            Some(identity.revision.clone()),
        ),
        (
            "the finish",
            &specs["Finish"],
            Some(writer::finish(project.fabrication.pad_finish).to_owned()),
        ),
        ("the creation date", &job["Header"]["CreationDate"], date),
    ] {
        if stated.as_str() != design.as_deref() {
            let design = design.unwrap_or_else(|| "none".into());
            r.say(
                &name,
                None,
                &format!("{what} {stated}"),
                &format!("“{design}”"),
            );
        }
    }
    // The fills the design has, as IPC-4761 types.
    let capped = project
        .vias
        .iter()
        .any(|v| v.fill == crate::model::ViaFill::Capped);
    let design = if capped {
        serde_json::json!(["VII"])
    } else {
        Value::Null
    };
    let stated = &specs["ViaProtection"];
    if *stated != design {
        r.say(
            &name,
            None,
            &format!("the via protection {stated}"),
            &design.to_string(),
        );
    }
    let stack = &project.stackup;
    let [min, max] = outline_box(project);
    let thickness: f64 = stack
        .copper
        .iter()
        .map(|c| c.thickness_um / 1000.0)
        .chain(stack.dielectrics.iter().map(|d| d.thickness_mm))
        .sum();
    let rules = project.rules.effective();
    let gap = |a, b| rules.copper.clearance.get(a, b);
    let rule = &job["DesignRules"][0];
    use CopperObject::{Fill, Pad, Track};
    for (what, stated, design, within) in [
        ("the width", &specs["Size"]["X"], max[0] - min[0], SIZE_MM),
        ("the height", &specs["Size"]["Y"], max[1] - min[1], SIZE_MM),
        (
            "the layer count",
            &specs["LayerNumber"],
            stack.copper.len() as f64,
            0.1,
        ),
        ("the thickness", &specs["BoardThickness"], thickness, JOB_MM),
        (
            "the pad to pad clearance",
            &rule["PadToPad"],
            gap(Pad, Pad),
            JOB_MM,
        ),
        (
            "the pad to track clearance",
            &rule["PadToTrack"],
            gap(Pad, Track),
            JOB_MM,
        ),
        (
            "the track to track clearance",
            &rule["TrackToTrack"],
            gap(Track, Track),
            JOB_MM,
        ),
        (
            "the track to fill clearance",
            &rule["TrackToRegion"],
            gap(Track, Fill),
            JOB_MM,
        ),
        (
            "the fill to fill clearance",
            &rule["RegionToRegion"],
            gap(Fill, Fill),
            JOB_MM,
        ),
        (
            "the track width",
            &rule["MinLineWidth"],
            rules.copper.track_width_min,
            JOB_MM,
        ),
    ] {
        if !stated
            .as_f64()
            .is_some_and(|v| (v - design).abs() <= within)
        {
            r.say(
                &name,
                None,
                &format!("{what} {stated}"),
                &format!("{design}"),
            );
        }
    }
    let written: Vec<(String, String, Option<f64>)> = job["MaterialStackup"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let word = |k: &str| m[k].as_str().unwrap_or_default().to_owned();
            (word("Type"), word("Name"), m["Thickness"].as_f64())
        })
        .collect();
    let design = materials(project);
    for k in 0..written.len().max(design.len()) {
        let words = |m: Option<&(String, String, Option<f64>)>| match m {
            Some((kind, name, Some(mm))) => format!("{kind} “{name}” of {mm:.6} mm"),
            Some((kind, name, None)) => format!("{kind} “{name}”"),
            None => "nothing".into(),
        };
        let (a, b) = (words(written.get(k)), words(design.get(k)));
        if a != b {
            r.say(&name, None, &format!("{a} as material {}", k + 1), &b);
        }
    }
    Ok(())
}

/// The board's materials, top to bottom.
fn materials(project: &Project) -> Vec<(String, String, Option<f64>)> {
    let stack = &project.stackup;
    let mut out = Vec::new();
    let mask = Some(stack.mask_um / 1000.0);
    let mut put = |kind: &str, name: String, mm: Option<f64>| out.push((kind.to_owned(), name, mm));
    if stack.silkscreen_top {
        put("Legend", "Top legend".into(), None);
    }
    if stack.mask_top {
        put("SolderMask", "Top solder mask".into(), mask);
    }
    for (k, foil) in stack.copper.iter().enumerate() {
        let layer = stack.layers().get(k).map(|l| l.name());
        let name = format!("{} copper", layer.unwrap_or_default());
        put("Copper", name, Some(foil.thickness_um / 1000.0));
        if let Some(d) = stack.dielectrics.get(k) {
            let kind = match d.kind {
                DielectricKind::Core => "Core",
                DielectricKind::Prepreg => "Prepreg",
            };
            put(
                "Dielectric",
                format!("{kind} {}", k + 1),
                Some(d.thickness_mm),
            );
        }
    }
    if stack.mask_bottom {
        put("SolderMask", "Bottom solder mask".into(), mask);
    }
    if stack.silkscreen_bottom {
        put("Legend", "Bottom legend".into(), None);
    }
    out
}

/// Every layer and drill file listed once, with the function and polarity it
/// states itself, and no other.
fn listed(r: &mut Reading<'_, '_>, name: &str, job: &Value) {
    let mut left: BTreeMap<String, (String, String)> = BTreeMap::new();
    for f in job["FilesAttributes"].as_array().into_iter().flatten() {
        let word = |k: &str| f[k].as_str().unwrap_or_default().to_owned();
        let path = word("Path");
        let stated = (word("FileFunction"), word("FilePolarity"));
        if left.insert(path.clone(), stated).is_some() {
            r.say(name, None, &format!("{path} twice"), "it once");
        }
    }
    let files: Vec<(&str, &str)> = r
        .files
        .iter()
        .filter_map(|(n, b)| Some((*n, std::str::from_utf8(b).ok()?)))
        .collect();
    for (file, text) in files {
        let stated = if file.ends_with(".gbr") {
            gerber::parse(text).ok().map(|f| {
                [".FileFunction", ".FilePolarity"]
                    .map(|k| f.file_attribute(k).map(|v| v.join(",")).unwrap_or_default())
            })
        } else if file.ends_with(".drl") {
            excellon::parse(text).ok().map(|f| {
                [".FileFunction", ".FilePolarity"]
                    .map(|k| f.attribute(k).map(|v| v.join(",")).unwrap_or_default())
            })
        } else {
            continue;
        };
        // One that cannot be read is told where it is read.
        let Some([function, polarity]) = stated else {
            left.remove(file);
            continue;
        };
        match left.remove(file) {
            None => r.say(name, None, &format!("no {file}"), file),
            Some(listed) if listed != (function.clone(), polarity.clone()) => r.say(
                name,
                None,
                &format!("{file} as {}, {}", listed.0, listed.1),
                &format!("{function}, {polarity}"),
            ),
            Some(_) => {}
        }
    }
    for file in left.keys() {
        r.say(name, None, file, "no such file");
    }
}

/// The records of a table after its header, which must be `header`.
fn records(r: &mut Reading<'_, '_>, name: &str, header: &[&str]) -> Option<Vec<Vec<String>>> {
    let text = r.text(name, true)?;
    match csv::records(text) {
        Ok(mut records) => {
            let first = if records.is_empty() {
                Vec::new()
            } else {
                records.remove(0)
            };
            if first != header {
                r.say(
                    name,
                    None,
                    &format!("the columns {}", first.join(", ")),
                    &format!("the columns {}", header.join(", ")),
                );
                return None;
            }
            Some(records)
        }
        Err(e) => {
            r.unreadable(name, &e.to_string());
            None
        }
    }
}

pub(super) fn assembly(r: &mut Reading<'_, '_>) -> Fallible<()> {
    for (side, word, suffix) in [
        (Layer::Top, "Top", "pos-top.csv"),
        (Layer::Bottom, "Bottom", "pos-bottom.csv"),
    ] {
        positions(r, side, word, suffix)?;
    }
    let parts = parts(r)?;
    lines(r, &parts)
}

/// A part as a pick-and-place file places it.
struct Place {
    designator: String,
    value: String,
    package: String,
    at: Point,
    angle: f64,
    /// Where it is on the board.
    on_board: Point,
}

/// Every part a side is placed with: position and angle from the document,
/// by the convention as its documentation states it.
fn places(r: &Reading<'_, '_>, side: Layer) -> Fallible<Vec<Place>> {
    let project = r.project;
    let convention = &r.options.placement;
    let origin = match convention.origin {
        PlacementOrigin::BoardOrigin => [0.0; 2],
        PlacementOrigin::OutlineLowerLeft => outline_box(project)[0],
        PlacementOrigin::Point { at } => at,
    };
    let mut out = Vec::new();
    for placement in project.placements.iter().filter(|p| p.layer == side) {
        let Some(instance) = project.instance(placement.instance) else {
            continue;
        };
        let decision = completeness::assembly(project, instance);
        let Some(package) = assembly::package_name(project, instance) else {
            continue;
        };
        if !decision.placement {
            continue;
        }
        let [x, y] = placement.position;
        let (mut x, y) = (x - origin[0], y - origin[1]);
        let mut angle = placement.rotation_deg;
        if side == Layer::Bottom {
            if convention.bottom == BottomRotation::ViewedFromBottom {
                angle = 180.0 - angle;
            }
            if convention.mirror_bottom_x {
                x = -x;
            }
        }
        angle += convention
            .rotation_offsets
            .iter()
            .find(|o| crate::rules::glob_matches(&o.package, &package))
            .map_or(0.0, |o| o.degrees);
        out.push(Place {
            designator: instance.designator.clone(),
            value: assembly::instance_value(project, instance, &decision)?,
            package,
            at: [x, y],
            angle,
            on_board: placement.position,
        });
    }
    Ok(out)
}

fn positions(r: &mut Reading<'_, '_>, side: Layer, word: &str, suffix: &str) -> Fallible<()> {
    let name = r.name(suffix);
    let mut left = places(r, side)?;
    if left.is_empty() && !r.files.contains_key(name.as_str()) {
        return Ok(());
    }
    let header = [
        "Designator",
        "Value",
        "Package",
        "X (mm)",
        "Y (mm)",
        "Rotation (deg)",
        "Side",
    ];
    let Some(records) = records(r, &name, &header) else {
        return Ok(());
    };
    for record in &records {
        let [designator, value, package, x, y, angle, on] = record.as_slice() else {
            continue;
        };
        let number = |s: &str| s.parse::<f64>().unwrap_or(f64::NAN);
        let (at, turn) = ([number(x), number(y)], number(angle));
        let placed = |p: &Place| {
            let off = (turn - p.angle).rem_euclid(360.0);
            distance(at, p.at) <= PLACE_MM * 2.0 && off.min(360.0 - off) <= PLACE_DEG
        };
        let named = |p: &Place| p.designator == *designator;
        let k = left
            .iter()
            .position(|p| named(p) && placed(p))
            .or_else(|| left.iter().position(named));
        let Some(k) = k else {
            r.say(&name, None, designator, "no such part there");
            continue;
        };
        let part = left.swap_remove(k);
        if !placed(&part) {
            let ours = part.angle.rem_euclid(360.0);
            r.say(
                &name,
                Some(part.on_board),
                &format!("{designator} at ({x}, {y}) turned {angle}°"),
                &format!("({:.4}, {:.4}) turned {ours:.3}°", part.at[0], part.at[1]),
            );
        }
        if (value, package, on.as_str()) != (&part.value, &part.package, word) {
            r.say(
                &name,
                Some(part.on_board),
                &format!("{designator} as “{value}”, “{package}”, {on}"),
                &format!("“{}”, “{}”, {word}", part.value, part.package),
            );
        }
    }
    for part in left.into_iter().take(MOST) {
        r.say(&name, Some(part.on_board), "no such part", &part.designator);
    }
    Ok(())
}

/// What a part is: its value and package.
type Bought = [String; 2];

/// Every part on the bill of materials once; what it is, by designator.
fn parts(r: &mut Reading<'_, '_>) -> Fallible<BTreeMap<String, Bought>> {
    let project = r.project;
    let mut left: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    let mut bought = BTreeMap::new();
    for instance in project
        .instances
        .iter()
        .filter(|i| crate::instances::on_bom(&project.library, i))
    {
        let decision = completeness::assembly(project, instance);
        let text = |t: &Option<String>| t.clone().unwrap_or_default();
        let part: Bought = [
            assembly::instance_value(project, instance, &decision)?,
            assembly::package_name(project, instance).unwrap_or_default(),
        ];
        let fitted = if decision.populated() { "yes" } else { "no" };
        let mut record = part.to_vec();
        record.extend([
            text(&decision.purchase.manufacturer),
            text(&decision.purchase.mpn),
            fitted.to_owned(),
        ]);
        left.entry(instance.designator.clone())
            .or_default()
            .push(record);
        bought.insert(instance.designator.clone(), part);
    }
    let name = r.name("bom-parts.csv");
    let header = [
        "Designator",
        "Value",
        "Package",
        "Manufacturer",
        "MPN",
        "Fitted",
    ];
    if let Some(records) = records(r, &name, &header) {
        for record in &records {
            let Some((designator, rest)) = record.split_first() else {
                continue;
            };
            let same = left.get_mut(designator).and_then(|l| {
                let k = l
                    .iter()
                    .position(|p| p == rest)
                    .or((!l.is_empty()).then_some(0))?;
                Some(l.swap_remove(k))
            });
            if same.as_deref() != Some(rest) {
                let design = same.map_or_else(|| "no such part".into(), |p| p.join(", "));
                let written = format!("{designator}: {}", rest.join(", "));
                r.say(&name, None, &written, &design);
            }
        }
        for (designator, l) in left.iter().filter(|(_, l)| !l.is_empty()).take(MOST) {
            let design = format!("{designator}, {} times", l.len());
            r.say(&name, None, "no such part", &design);
        }
    }
    Ok(bought)
}

/// Every line of the grouped bill of materials, and that it says of each
/// part what `bom-parts.csv` does.
fn lines(r: &mut Reading<'_, '_>, parts: &BTreeMap<String, Bought>) -> Fallible<()> {
    let report = completeness::bom(r.project)?;
    // Its designators, manufacturer and MPN.
    let mut left: Vec<[String; 3]> = report
        .lines
        .iter()
        .filter(|l| l.fitted)
        .map(|l| {
            let text = |t: &Option<String>| t.clone().unwrap_or_default();
            [l.designators.join(" "), text(&l.manufacturer), text(&l.mpn)]
        })
        .collect();
    let name = r.name("bom.csv");
    let header = [
        "Line",
        "Quantity",
        "Designators",
        "Value",
        "Package",
        "Manufacturer",
        "MPN",
        "Status",
    ];
    let Some(records) = records(r, &name, &header) else {
        return Ok(());
    };
    for record in &records {
        let [_, quantity, designators, value, package, maker, mpn, _] = record.as_slice() else {
            continue;
        };
        match left.iter().position(|l| l[0] == *designators) {
            Some(k) => {
                let [_, m, n] = left.remove(k);
                if (&m, &n) != (maker, mpn) {
                    let written = format!("{designators}: {maker}, {mpn}");
                    r.say(&name, None, &written, &format!("{m}, {n}"));
                }
            }
            None => {
                let written = format!("a line of {designators}");
                r.say(&name, None, &written, "no such line");
            }
        }
        let each: Vec<&str> = designators.split_whitespace().collect();
        if quantity.parse::<usize>().ok() != Some(each.len()) {
            let written = format!("{quantity} of {designators}");
            r.say(&name, None, &written, &each.len().to_string());
        }
        // The parts of a line are alike; the first says what they are.
        if let Some([v, p]) = each.first().and_then(|d| parts.get(*d))
            && (v, p) != (value, package)
        {
            let written = format!("{designators}: {value}, {package}");
            r.say(&name, None, &written, &format!("{v}, {p}"));
        }
    }
    for [line, ..] in left.into_iter().take(MOST) {
        let design = format!("a line of {line}");
        r.say(&name, None, "no such line", &design);
    }
    Ok(())
}

//! The written files read back and rasterised, against the core's own
//! geometry rendered independently (`expected`): every copper layer, both
//! masks and pastes, the profile and both drill files, at 10 µm pixels,
//! for the showcase, a sketched board with arcs, a cutout, slots and
//! mounting holes, and a 10,000-via plane. Format details are checked on
//! the same files.
use super::Fallible;
use super::excellon_reader;
use super::expected;
use super::raster::{self, Bitmap, Comparison, Shape};
use super::reader::{self, GerberFile, Kind};
use crate::Engine;
use crate::fabrication::{self, FabricationOptions, FabricationSet};
use crate::ids::Id;
use crate::library::footprint_source::{EvaluatorRef, SketchBlob, SolveSummary};
use crate::library::{Outline, Segment};
use crate::model::board::{
    BoardFeatureId, BoardShape, BoardSketch, CutoutKind, SolvedBoard, SolvedCutout,
    SolvedMountingHole,
};
use crate::model::{CopperLayer, Layer, Project};
use std::time::Instant;

/// Pixel size of the comparisons, mm.
const PIXEL: f64 = 0.01;

fn text<'a>(set: &'a FabricationSet, suffix: &str) -> Fallible<&'a str> {
    let file = set
        .files
        .iter()
        .find(|f| f.name.ends_with(suffix))
        .ok_or_else(|| format!("no file ending {suffix}"))?;
    Ok(std::str::from_utf8(&file.bytes)?)
}

fn has(set: &FabricationSet, suffix: &str) -> bool {
    set.files.iter().any(|f| f.name.ends_with(suffix))
}

fn gerber(set: &FabricationSet, suffix: &str) -> Fallible<GerberFile> {
    reader::parse(text(set, suffix)?)
}

/// A Gerber layer rendered on a blank copy of `grid`.
fn rendered(file: &GerberFile, grid: &Bitmap) -> Fallible<Bitmap> {
    let mut b = grid.blank();
    raster::render_gerber(file, &mut b)?;
    Ok(b)
}

/// The profile's draws chained into closed contours and filled even-odd:
/// the board less its cutouts and slots.
fn profile_region(file: &GerberFile, grid: &Bitmap) -> Fallible<Bitmap> {
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut last: Option<[f64; 2]> = None;
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    for object in &file.objects {
        let (from, points) = match &object.kind {
            Kind::Draw { from, to } => (*from, vec![*to]),
            Kind::Arc {
                from,
                to,
                center,
                clockwise,
            } => (*from, raster::arc_points(*from, *to, *center, *clockwise)),
            _ => return Err("the profile holds only draws".into()),
        };
        if !last.is_some_and(|l| near(l, from)) {
            contours.push(vec![from]);
        }
        let current = contours.last_mut().ok_or("no contour")?;
        current.extend(points);
        last = current.last().copied();
    }
    for c in &contours {
        let (Some(a), Some(b)) = (c.first(), c.last()) else {
            return Err("empty contour".into());
        };
        if !near(*a, *b) {
            return Err(format!("open profile contour from {a:?} to {b:?}").into());
        }
    }
    let mut out = grid.blank();
    out.render(&Shape::Poly { contours }, true);
    Ok(out)
}

/// The XOR limit for a layer: an area band 2 µm wide along every edge
/// (curves are flattened within 0.5 µm by both renderings, and a pixel
/// centre falls in the band with that probability), at least 4 pixels.
pub(super) fn allowed(c: &Comparison, perimeter_mm: f64) -> f64 {
    (perimeter_mm * 0.002).max(4.0 * c.pixel * c.pixel)
}

/// A rough perimeter of a rendering's dark area, from its edge pixels.
pub(super) fn perimeter(b: &Bitmap) -> f64 {
    let (w, h) = (b.width, b.height);
    let mut edges = 0u64;
    for j in 0..h {
        for i in 0..w {
            let here = b.get(i, j);
            if i + 1 < w && here != b.get(i + 1, j) {
                edges += 1;
            }
            if j + 1 < h && here != b.get(i, j + 1) {
                edges += 1;
            }
        }
    }
    edges as f64 * b.pixel
}

/// A block of two by two pixels that differ, if any: a difference thicker
/// than the edges' sampling.
pub(super) fn thick(a: &Bitmap, b: &Bitmap) -> Option<[f64; 2]> {
    let differs = |i, j| a.get(i, j) != b.get(i, j);
    (0..a.height.saturating_sub(1))
        .flat_map(|j| (0..a.width.saturating_sub(1)).map(move |i| (i, j)))
        .find(|&(i, j)| {
            differs(i, j) && differs(i + 1, j) && differs(i, j + 1) && differs(i + 1, j + 1)
        })
        .map(|(i, j)| {
            [
                a.min[0] + (i as f64 + 1.0) * a.pixel,
                a.min[1] + (j as f64 + 1.0) * a.pixel,
            ]
        })
}

/// Compare every layer of `set` with the core geometry of `project`;
/// returns (layer, comparison) rows after asserting each is within its
/// limit.
fn compare_all(
    project: &Project,
    set: &FabricationSet,
    label: &str,
) -> Fallible<Vec<(String, Comparison)>> {
    // The window is offset by an irrational-looking margin so no pixel
    // centre lies exactly on a grid-aligned edge: two holes of one fill
    // sharing an edge at a pixel centre are sampled differently by an
    // even-odd fill (the core) and by clear-polarity regions (the file),
    // a measurement artefact, not a geometry difference.
    let grid = expected::board_bitmap(project, 1.003_17, PIXEL)?;
    let mut rows = Vec::new();
    let mut check = |name: String, got: Bitmap, want: Bitmap| -> Fallible<()> {
        let c = raster::compare(&got, &want)?;
        let limit = allowed(&c, perimeter(&want));
        // The band is spread along the edges: no difference is two pixels
        // thick, however small its area (a missing paste island).
        if let Some(at) = thick(&got, &want) {
            return Err(format!("{label} {name}: differs round {at:?}").into());
        }
        if c.xor_area > 0.0 {
            let mut where_ = Vec::new();
            for j in 0..got.height {
                for i in 0..got.width {
                    if got.get(i, j) != want.get(i, j) && where_.len() < 6 {
                        where_.push((
                            got.min[0] + (i as f64 + 0.5) * got.pixel,
                            got.min[1] + (j as f64 + 0.5) * got.pixel,
                            got.get(i, j),
                        ));
                    }
                }
            }
            eprintln!("  first differing pixels (x, y, written dark): {where_:?}");
        }
        eprintln!(
            "{label} {name}: written {:.4} mm², core {:.4} mm², XOR {:.5} mm² ({:.2e}), limit {:.5}",
            c.area_a, c.area_b, c.xor_area, c.xor_share, limit
        );
        if c.xor_area > limit {
            return Err(format!("{label} {name}: XOR {} mm² over {limit}", c.xor_area).into());
        }
        if c.area_b > 0.0 && c.area_a <= 0.0 {
            return Err(format!("{label} {name}: written layer is empty").into());
        }
        rows.push((name, c));
        Ok(())
    };
    for layer in project.stackup.layers() {
        let name = fabrication::artwork::copper_name(project, layer);
        let got = rendered(&gerber(set, &format!("-{name}.gbr"))?, &grid)?;
        let mut want = grid.blank();
        expected::copper(project, layer, &mut want)?;
        check(name, got, want)?;
    }
    for side in [Layer::Top, Layer::Bottom] {
        let word = if side == Layer::Top { "top" } else { "bottom" };
        let got = rendered(&gerber(set, &format!("-mask-{word}.gbr"))?, &grid)?;
        let mut want = grid.blank();
        expected::mask(project, side, &mut want)?;
        check(format!("mask-{word}"), got, want)?;
        let mut want = grid.blank();
        expected::paste(project, side, &mut want)?;
        let got = if has(set, &format!("-paste-{word}.gbr")) {
            rendered(&gerber(set, &format!("-paste-{word}.gbr"))?, &grid)?
        } else {
            grid.blank()
        };
        check(format!("paste-{word}"), got, want)?;
    }
    // The profile mills the outline, cutouts and non-plated slots; plated
    // straight slots are routed by the plated drill file instead.
    let got = profile_region(&gerber(set, "-profile.gbr")?, &grid)?;
    let mut want = grid.blank();
    expected::outline(project, &mut want)?;
    check("profile".into(), got, want)?;
    for (plated, suffix) in [(true, "-plated.drl"), (false, "-non-plated.drl")] {
        let mut got = grid.blank();
        if has(set, suffix) {
            raster::render_drill(&excellon_reader::parse(text(set, suffix)?)?, &mut got)?;
        }
        let mut want = grid.blank();
        expected::drills(project, plated, &mut want)?;
        check(suffix.trim_start_matches('-').into(), got, want)?;
    }
    Ok(rows)
}

fn showcase() -> Fallible<Project> {
    Ok(Engine::showcase()?.project().clone())
}

/// Two independently refilled EN pours around a 0.6 mm GND via. Their
/// clearances begin 0.30 mm from its centre and extend 0.30 and 1.00 mm.
#[test]
fn overlapping_pours_keep_each_others_positive_copper() -> Fallible<()> {
    use crate::Operation;
    use crate::layout::fill::{FillCommand, RefillTarget};
    use crate::layout::{LayoutCommand, ViaRequest};
    use crate::model::fill::{FillRegion, FillSettings, FillSpec, IslandRemoval};

    let mut engine = Engine::showcase()?;
    let net = |name: &str| {
        engine
            .project()
            .nets
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.id)
    };
    let en = net("EN").ok_or("missing EN")?;
    let gnd = net("GND").ok_or("missing GND")?;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::AddVia {
            via: ViaRequest::fixture(gnd, [40.0, 24.5]),
        }),
    )?;
    for (name, min, max, clearance) in [
        ("large", [36.0, 20.5], [43.0, 27.5], 0.30),
        ("small", [37.0, 21.5], [42.0, 26.5], 1.00),
    ] {
        let outline = Outline {
            start: min,
            segments: [[max[0], min[1]], max, [min[0], max[1]], min]
                .into_iter()
                .map(|to| Segment::Line { to })
                .collect(),
        };
        engine.apply(
            engine.revision(),
            Operation::Fill(FillCommand::Add {
                fill: Box::new(FillSpec {
                    name: name.into(),
                    net: Some(en),
                    layer: CopperLayer::Top,
                    region: FillRegion::Drawn,
                    outline,
                    priority: 0,
                    settings: FillSettings {
                        clearance: Some(clearance),
                        islands: IslandRemoval::Never,
                        ..Default::default()
                    },
                }),
            }),
        )?;
    }
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Refill {
            target: RefillTarget::All,
        }),
    )?;
    let project = engine.project();
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(project, None)?,
        &Default::default(),
        &Default::default(),
    )?;
    let grid = Bitmap::new([35.0, 20.0], [44.0, 28.0], PIXEL)?;
    let image = rendered(&gerber(&set, "-copper-l1-top.gbr")?, &grid)?;
    // Physical sample points, independent of either stored polygon.
    for (x, dark) in [
        (40.00, true),  // Via.
        (40.45, false), // Both pours clear the via.
        (40.55, false), // Just inside the smaller clearance.
        (40.65, true),  // Just outside it.
        (40.80, true),  // Only the large pour supplies this copper.
        (41.50, true),  // Both pours supply copper.
        (42.50, true),  // Outside the smaller pour.
        (43.15, false), // Outside both pours.
    ] {
        assert_eq!(image.at([x, 24.5]), dark, "copper at ({x}, 24.5) mm");
    }
    Ok(())
}

fn bulge_quarter() -> f64 {
    (std::f64::consts::PI / 8.0).tan()
}

/// A rectangle `w` × `h` with every corner rounded by `r`, counter-clockwise.
fn rounded(w: f64, h: f64, r: f64) -> Outline {
    let b = bulge_quarter();
    Outline {
        start: [r, 0.0],
        segments: vec![
            Segment::Line { to: [w - r, 0.0] },
            Segment::Arc {
                to: [w, r],
                bulge: b,
            },
            Segment::Line { to: [w, h - r] },
            Segment::Arc {
                to: [w - r, h],
                bulge: b,
            },
            Segment::Line { to: [r, h] },
            Segment::Arc {
                to: [0.0, h - r],
                bulge: b,
            },
            Segment::Line { to: [0.0, r] },
            Segment::Arc {
                to: [r, 0.0],
                bulge: b,
            },
        ],
    }
}

fn circle(c: [f64; 2], radius: f64) -> Outline {
    Outline {
        start: [c[0] + radius, c[1]],
        segments: vec![
            Segment::Arc {
                to: [c[0] - radius, c[1]],
                bulge: 1.0,
            },
            Segment::Arc {
                to: [c[0] + radius, c[1]],
                bulge: 1.0,
            },
        ],
    }
}

/// A straight slot from `a` to `b` (horizontal), `width` wide.
fn slot(a: [f64; 2], b: [f64; 2], width: f64) -> Outline {
    let h = width / 2.0;
    Outline {
        start: [a[0], a[1] - h],
        segments: vec![
            Segment::Line {
                to: [b[0], b[1] - h],
            },
            Segment::Arc {
                to: [b[0], b[1] + h],
                bulge: 1.0,
            },
            Segment::Line {
                to: [a[0], a[1] + h],
            },
            Segment::Arc {
                to: [a[0], a[1] - h],
                bulge: 1.0,
            },
        ],
    }
}

/// The showcase on a sketched board: 2 mm rounded corners, a round cutout,
/// a non-plated and a plated milled slot, a plated mounting hole with a
/// ring and a bare one, with its fills computed over that shape.
fn sketched() -> Fallible<Project> {
    let mut p = showcase()?;
    let [w, h] = p.board_size;
    let mut solved = SolvedBoard::rectangle(p.board_size);
    solved.outer = rounded(w, h, 2.0);
    solved.cutouts = vec![
        SolvedCutout {
            id: Some(BoardFeatureId(1)),
            kind: CutoutKind::Cutout,
            outline: circle([40.5, 24.5], 1.75),
            curves: Vec::new(),
        },
        SolvedCutout {
            id: Some(BoardFeatureId(2)),
            kind: CutoutKind::Slot { plated: false },
            outline: slot([28.0, 2.5], [33.0, 2.5], 1.0),
            curves: Vec::new(),
        },
        SolvedCutout {
            id: Some(BoardFeatureId(3)),
            kind: CutoutKind::Slot { plated: true },
            outline: slot([3.0, 25.0], [6.0, 25.0], 1.2),
            curves: Vec::new(),
        },
    ];
    solved.holes = vec![
        SolvedMountingHole {
            id: BoardFeatureId(4),
            at: [41.0, 3.0],
            drill: 2.2,
            plated: true,
            pad_diameter: Some(4.0),
        },
        SolvedMountingHole {
            id: BoardFeatureId(5),
            at: [3.0, 3.0],
            drill: 1.6,
            plated: false,
            pad_diameter: None,
        },
    ];
    let evaluator = EvaluatorRef {
        engine: "test".into(),
        rev: "0".into(),
    };
    p.board_shape = BoardShape::Sketch(Box::new(BoardSketch {
        evaluator: evaluator.clone(),
        sketch: SketchBlob("{}".into()),
        parameters: Vec::new(),
        cutouts: Vec::new(),
        holes: Vec::new(),
        edges: Vec::new(),
        next_feature: 6,
        solve: SolveSummary {
            evaluator,
            status: Default::default(),
            dof: 0,
            result_hash: String::new(),
        },
        solved,
        current: true,
        outline_fit: 0.01,
        fault: None,
        idle: Vec::new(),
    }));
    refill(&mut p)?;
    Ok(p)
}

fn refill(p: &mut Project) -> Fallible<()> {
    let computed = crate::layout::fill::refill(p, crate::layout::fill::RefillTarget::All)?;
    for (id, copper) in computed {
        if let Some(fill) = p.fills.iter_mut().find(|f| f.id == id) {
            fill.computed = Some(copper);
        }
    }
    Ok(())
}

#[test]
fn the_showcase_layers_match_the_core_geometry() -> Fallible<()> {
    let project = showcase()?;
    let start = Instant::now();
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    eprintln!(
        "showcase: {} files, {} bytes, generated in {:?}",
        set.files.len(),
        set.files.iter().map(|f| f.bytes.len()).sum::<usize>(),
        start.elapsed()
    );
    let rows = compare_all(&project, &set, "showcase")?;
    // 4 copper, 2 masks, 2 pastes, profile and two drill comparisons.
    assert_eq!(rows.len(), 4 + 2 + 2 + 1 + 2);
    Ok(())
}

/// The exact polygon path (`sparksmith_fabcheck::polygons`) agrees with the
/// raster on every written layer of the showcase and the sketched board:
/// the polygon area lies within the raster's sampling bound of the raster
/// area, and a layer compared with itself has no difference.
#[test]
fn the_polygon_path_agrees_with_the_raster_on_every_layer() -> Fallible<()> {
    use sparksmith_fabcheck::{compare, polygons};
    for (label, project) in [("showcase", showcase()?), ("sketched", sketched()?)] {
        let set = fabrication::generate(
            &fabrication::ProductionSnapshot::resolve(&project, None)?,
            &FabricationOptions::default(),
            &Default::default(),
        )?;
        let grid = expected::board_bitmap(&project, 1.003_17, PIXEL)?;
        let mut layers = 0;
        for file in set.files.iter().filter(|f| f.name.ends_with(".gbr")) {
            if file.name.ends_with("-profile.gbr") {
                continue;
            }
            let parsed = reader::parse(std::str::from_utf8(&file.bytes)?)?;
            let raster = rendered(&parsed, &grid)?;
            let exact = polygons::render(&parsed, |_| true)?;
            let area = polygons::area_mm2(&exact);
            let limit = allowed(&raster::compare(&raster, &raster)?, perimeter(&raster));
            assert!(
                (area - raster.area_mm2()).abs() <= limit,
                "{label} {}: polygons {area} mm², raster {} mm², limit {limit}",
                file.name,
                raster.area_mm2()
            );
            let same = compare::compare(&exact, &exact, 1000)?;
            assert_eq!((same.difference_mm2, same.worst_width_um), (0.0, 0.0));
            layers += 1;
        }
        assert!(layers >= 8, "{label}: {layers} layers");
    }
    Ok(())
}

#[test]
fn a_sketched_board_with_arcs_cutouts_slots_and_holes_matches() -> Fallible<()> {
    let project = sketched()?;
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    compare_all(&project, &set, "sketched")?;
    // The plated slot is routed in the plated drill file, not the profile.
    let plated = excellon_reader::parse(text(&set, "-plated.drl")?)?;
    let slots: Vec<_> = plated
        .hits
        .iter()
        .filter(|h| matches!(h, excellon_reader::Hit::Slot { .. }))
        .collect();
    assert_eq!(slots.len(), 1);
    let profile = gerber(&set, "-profile.gbr")?;
    // Outer outline (4 lines, 4 arcs), cutout (2 arcs), the non-plated
    // slot (2 lines, 2 arcs): no plated slot.
    assert_eq!(profile.objects.len(), 8 + 2 + 4);
    assert!(profile.objects.iter().all(|o| o.dark));
    Ok(())
}

#[test]
fn layers_carry_their_file_aperture_and_object_attributes() -> Fallible<()> {
    let project = sketched()?;
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    let top = gerber(&set, "-copper-l1-top.gbr")?;
    assert!(top.metric);
    assert_eq!((top.integer_digits, top.decimals), (4, 6));
    assert_eq!(
        top.file_attribute(".FileFunction"),
        Some(&["Copper".to_string(), "L1".into(), "Top".into()][..])
    );
    assert_eq!(
        top.file_attribute(".FilePolarity"),
        Some(&["Positive".to_string()][..])
    );
    assert!(top.file_attribute(".GenerationSoftware").is_some());
    assert!(top.file_attribute(".SameCoordinates").is_some());
    // Without a date option there is no creation date (byte-identical sets).
    assert!(top.file_attribute(".CreationDate").is_none());
    let inner = gerber(&set, "-copper-l2-inner1.gbr")?;
    assert_eq!(
        inner.file_attribute(".FileFunction"),
        Some(&["Copper".to_string(), "L2".into(), "Inr".into()][..])
    );
    let bottom = gerber(&set, "-copper-l4-bottom.gbr")?;
    assert_eq!(
        bottom.file_attribute(".FileFunction"),
        Some(&["Copper".to_string(), "L4".into(), "Bot".into()][..])
    );
    // Every flash on copper names its function; pads name their
    // component, pad and net.
    let function = |o: &reader::Object| {
        o.aperture_attributes
            .get(".AperFunction")
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default()
    };
    for o in top.flashes() {
        assert!(!function(o).is_empty(), "{o:?}");
    }
    let pads: Vec<_> = top
        .objects
        .iter()
        .filter(|o| matches!(function(o).as_str(), "SMDPad" | "ComponentPad"))
        .collect();
    assert!(!pads.is_empty());
    for pad in &pads {
        let p = pad.object_attributes.get(".P").ok_or("pad without .P")?;
        let c = pad.object_attributes.get(".C").ok_or("pad without .C")?;
        assert_eq!(p.first(), c.first());
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(pad.object_attributes.contains_key(".N"));
    }
    let vias: Vec<_> = top.flashes().filter(|o| function(o) == "ViaPad").collect();
    assert_eq!(vias.len(), project.vias.len());
    for via in vias {
        let net = via
            .object_attributes
            .get(".N")
            .and_then(|v| v.first())
            .ok_or("via net")?;
        assert!(project.nets.iter().any(|n| &n.name == net), "{net}");
    }
    assert!(top.flashes().any(|o| function(o) == "WasherPad"));
    // Tracks are round-aperture draws with their net; fills are regions,
    // their holes clear.
    let tracks = top
        .objects
        .iter()
        .filter(|o| matches!(o.kind, Kind::Draw { .. }) && function(o) == "Conductor")
        .count();
    assert_eq!(
        tracks,
        project
            .tracks
            .iter()
            .filter(|t| t.layer == CopperLayer::Top)
            .count()
    );
    let regions: Vec<_> = bottom.regions().collect();
    assert!(regions.iter().any(|r| r.dark && function(r) == "Conductor"));
    assert!(regions.iter().any(|r| !r.dark), "fill holes are clear");
    // Masks are negative images of the openings; the profile and legend
    // positive.
    for side in ["Top", "Bot"] {
        let word = if side == "Top" { "top" } else { "bottom" };
        let mask = gerber(&set, &format!("-mask-{word}.gbr"))?;
        assert_eq!(
            mask.file_attribute(".FileFunction"),
            Some(&["Soldermask".to_string(), side.into()][..])
        );
        assert_eq!(
            mask.file_attribute(".FilePolarity"),
            Some(&["Negative".to_string()][..])
        );
    }
    let profile = gerber(&set, "-profile.gbr")?;
    assert_eq!(
        profile.file_attribute(".FileFunction"),
        Some(&["Profile".to_string(), "NP".into()][..])
    );
    let legend = gerber(&set, "-legend-top.gbr")?;
    assert_eq!(
        legend.file_attribute(".FileFunction"),
        Some(&["Legend".to_string(), "Top".into()][..])
    );
    assert!(legend.objects.len() > 20, "designators and outlines");
    // Drill files: attributes, tools as the table says, metric.
    let plated = excellon_reader::parse(text(&set, "-plated.drl")?)?;
    assert!(plated.metric);
    assert_eq!(
        plated.attribute(".FileFunction"),
        Some(vec!["Plated".into(), "1".into(), "4".into(), "PTH".into()])
    );
    let tools: Vec<_> = set.drills.plated().collect();
    assert_eq!(plated.tools.len(), tools.len());
    for t in tools {
        let d = plated.tools.get(&t.number).ok_or("tool missing")?;
        assert!((d - t.diameter).abs() < 1e-9);
    }
    let non = excellon_reader::parse(text(&set, "-non-plated.drl")?)?;
    assert_eq!(
        non.attribute(".FileFunction"),
        Some(vec![
            "NonPlated".into(),
            "1".into(),
            "4".into(),
            "NPTH".into()
        ])
    );
    let tools: Vec<_> = set.drills.non_plated().collect();
    assert!(!tools.is_empty(), "the board has non-plated holes");
    assert_eq!(non.tools.len(), tools.len());
    for t in tools {
        let d = non.tools.get(&t.number).ok_or("tool missing")?;
        assert!((d - t.diameter).abs() < 1e-9);
    }
    // E2E UX run finding 17: one number names one tool across the drill
    // table and both files.
    assert!(
        non.tools.keys().all(|n| !plated.tools.contains_key(n)),
        "plated {:?}, non-plated {:?}",
        plated.tools,
        non.tools
    );
    let numbers: Vec<u32> = set.drills.tools.iter().map(|t| t.number).collect();
    let expected: Vec<u32> = (1..=u32::try_from(numbers.len())?).collect();
    assert_eq!(numbers, expected, "numbered from 1 across both files");
    // The job file lists every Gerber and drill file in stackup order.
    let job: serde_json::Value = serde_json::from_str(text(&set, ".gbrjob")?)?;
    assert_eq!(job["GeneralSpecs"]["LayerNumber"], 4);
    let paths: Vec<&str> = job["FilesAttributes"]
        .as_array()
        .ok_or("files")?
        .iter()
        .filter_map(|f| f["Path"].as_str())
        .collect();
    let gerbers = set
        .files
        .iter()
        .filter(|f| f.name.ends_with(".gbr") || f.name.ends_with(".drl"))
        .count();
    assert_eq!(paths.len(), gerbers);
    assert!(paths[0].ends_with("copper-l1-top.gbr"));
    let thickness = job["GeneralSpecs"]["BoardThickness"]
        .as_f64()
        .ok_or("thickness")?;
    assert!((thickness - project.stackup.thickness_mm()).abs() < 1e-6);
    Ok(())
}

#[test]
fn the_same_design_gives_the_same_bytes_and_a_date_only_when_asked() -> Fallible<()> {
    let project = sketched()?;
    let a = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    let b = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    assert_eq!(a, b);
    let dated = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions {
            creation_date: Some("2026-11-02T12:00:00Z".into()),
            ..Default::default()
        },
        &Default::default(),
    )?;
    let top = gerber(&dated, "-copper-l1-top.gbr")?;
    assert_eq!(
        top.file_attribute(".CreationDate"),
        Some(&["2026-11-02T12:00:00Z".to_string()][..])
    );
    Ok(())
}

/// 10,000 through vias under a 200 × 200 mm bottom plane: the set is
/// generated in seconds (target), and its bottom copper, plated drill and
/// mask still match the core at 20 µm.
#[test]
fn a_ten_thousand_via_board_is_written_in_seconds() -> Fallible<()> {
    let mut project = showcase()?;
    let id = |p: &Project, name: &str| -> Fallible<Id> {
        Ok(p.nets
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.id)
            .ok_or(name.to_string())?)
    };
    let (gnd, vcc) = (id(&project, "GND")?, id(&project, "VCC")?);
    project.board_size = [200.0, 200.0];
    project.placements.clear();
    project.tracks.clear();
    project.vias.clear();
    project.fills.clear();
    for i in 0..100 {
        for j in 0..100 {
            let id = project.allocate()?;
            project.vias.push(crate::model::Via {
                id,
                net: if (i + j) % 2 == 0 { gnd } else { vcc },
                position: [1.0 + 1.98 * f64::from(i), 1.0 + 1.98 * f64::from(j)],
                drill: 0.3,
                diameter: 0.6,
                span: crate::model::ViaSpan::Through,
                tented: (i + j) % 3 != 0,
                locked: false,
            });
        }
    }
    let fill = project.allocate()?;
    project.fills.push(crate::model::Fill::from_spec(
        fill,
        crate::model::fill::FillSpec {
            region: crate::model::fill::FillRegion::Drawn,
            name: "GND".into(),
            net: Some(gnd),
            layer: CopperLayer::Bottom,
            outline: Outline {
                start: [0.0, 0.0],
                segments: vec![
                    Segment::Line { to: [200.0, 0.0] },
                    Segment::Line { to: [200.0, 200.0] },
                    Segment::Line { to: [0.0, 200.0] },
                    Segment::Line { to: [0.0, 0.0] },
                ],
            },
            priority: 0,
            settings: Default::default(),
        },
    ));
    refill(&mut project)?;
    let start = Instant::now();
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    let elapsed = start.elapsed();
    let bytes: usize = set.files.iter().map(|f| f.bytes.len()).sum();
    eprintln!(
        "10k vias: {} files, {:.1} MB, generated in {elapsed:?}",
        set.files.len(),
        bytes as f64 / 1e6
    );
    assert!(elapsed.as_secs_f64() < 30.0, "{elapsed:?}");
    let grid = expected::board_bitmap(&project, 0.5, 0.02)?;
    let start = Instant::now();
    for (suffix, want) in [
        ("-copper-l4-bottom.gbr", {
            let mut b = grid.blank();
            expected::copper(&project, CopperLayer::Bottom, &mut b)?;
            b
        }),
        ("-mask-top.gbr", {
            let mut b = grid.blank();
            expected::mask(&project, Layer::Top, &mut b)?;
            b
        }),
    ] {
        let got = rendered(&gerber(&set, suffix)?, &grid)?;
        let c = raster::compare(&got, &want)?;
        eprintln!(
            "10k vias {suffix}: written {:.3} mm², core {:.3} mm², XOR {:.5} mm² ({:.2e})",
            c.area_a, c.area_b, c.xor_area, c.xor_share
        );
        assert!(c.xor_share < 1e-4, "{suffix}: {c:?}");
    }
    let mut got = grid.blank();
    raster::render_drill(
        &excellon_reader::parse(text(&set, "-plated.drl")?)?,
        &mut got,
    )?;
    let mut want = grid.blank();
    expected::drills(&project, true, &mut want)?;
    let c = raster::compare(&got, &want)?;
    eprintln!(
        "10k vias plated drill: XOR {:.5} mm² ({:.2e})",
        c.xor_area, c.xor_share
    );
    assert!(c.xor_share < 1e-4, "{c:?}");
    eprintln!("10k vias: read back and compared in {:?}", start.elapsed());
    Ok(())
}

/// Layer naming, the job file and the drill span follow the stackup for
/// any count: the showcase on twelve layers, its supply plane moved to
/// Inner 10.
#[test]
fn a_twelve_layer_board_names_every_layer_and_spans_its_drills() -> Fallible<()> {
    use crate::layout::LayoutCommand;
    use crate::layout::restack::{LayerMove, StackupEdit};
    let mut engine = Engine::showcase()?;
    engine.apply(
        engine.revision(),
        crate::Operation::Layout(LayoutCommand::EditStackup(StackupEdit {
            stackup: crate::model::Stackup::preset(12)?,
            moves: vec![LayerMove {
                from: CopperLayer::Inner(2),
                to: CopperLayer::Inner(10),
            }],
            removed: Vec::new(),
        })),
    )?;
    let project = engine.project().clone();
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    let copper: Vec<&str> = set
        .files
        .iter()
        .filter(|f| f.name.contains("-copper-l"))
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(copper.len(), 12, "{copper:?}");
    let inner = gerber(&set, "-copper-l11-inner10.gbr")?;
    assert_eq!(
        inner.file_attribute(".FileFunction"),
        Some(&["Copper".to_string(), "L11".into(), "Inr".into()][..])
    );
    assert!(
        !inner.objects.is_empty(),
        "the moved plane draws on Inner 10"
    );
    let bottom = gerber(&set, "-copper-l12-bottom.gbr")?;
    assert_eq!(
        bottom.file_attribute(".FileFunction"),
        Some(&["Copper".to_string(), "L12".into(), "Bot".into()][..])
    );
    assert!(text(&set, "-plated.drl")?.contains("TF.FileFunction,Plated,1,12,PTH"));
    let job: serde_json::Value = serde_json::from_str(text(&set, ".gbrjob")?)?;
    assert_eq!(job["GeneralSpecs"]["LayerNumber"], 12);
    Ok(())
}

#[test]
fn the_stencil_preserves_each_asymmetric_window_corner() -> Fallible<()> {
    use crate::library::{ApertureId, PasteRule, PasteWindow, ShapeNm};
    use crate::tests::support::{nm, pt};
    for low in [0.02, 0.12] {
        let mut source = Engine::demo()?.to_file();
        let pad = &mut source.project.library.footprints[0].definition.pads[0];
        pad.paste = PasteRule::Windows(vec![PasteWindow {
            id: ApertureId(1),
            shape: ShapeNm::Polygon {
                points: [[-0.25, -0.25], [0.25, -0.25], [0.25, 0.25], [-0.25, 0.25]]
                    .map(pt)
                    .to_vec(),
                radii_nm: [low, 0.20, 0., 0.].map(nm).to_vec(),
            },
        }]);
        let engine = Engine::from_file(source)?;
        let project = engine.project();
        let layers = fabrication::artwork::layers(
            project,
            &Default::default(),
            &Default::default(),
            "stencil",
        )?;
        let file = layers
            .iter()
            .find(|f| f.name.ends_with("-paste-top.gbr"))
            .ok_or("missing top stencil")?;
        let frame = crate::layout::placed_pads(project)[0].pad_transform();
        let [x, y] = frame.translate;
        let image = rendered(
            &reader::parse(std::str::from_utf8(&file.bytes)?)?,
            &Bitmap::new([x - 0.27, y - 0.27], [x + 0.27, y + 0.27], 0.001)?,
        )?;
        for (point, inside) in [
            ([-0.23, -0.23], low == 0.02),
            ([0.23, -0.23], false),
            ([0.24, 0.24], true),
            ([-0.24, 0.24], true),
        ] {
            assert_eq!(
                image.at(frame.apply(point)),
                inside,
                "corner {point:?} at radius {low}"
            );
        }
        let area = 0.25 - (1. - std::f64::consts::FRAC_PI_4) * (low * low + 0.20_f64.powi(2));
        assert!(
            (image.area_mm2() - area).abs() < 0.0005,
            "expected {area}, read {}",
            image.area_mm2()
        );
    }
    Ok(())
}

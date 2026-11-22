//! The read-back oracle of the board's physical inventory
//! (`layout::physical`): the written Gerber and Excellon files are parsed
//! back by the independent reader and checked against the inventory the
//! checks measure, so a conductor or drill the checks never saw cannot
//! reach the fabricator. The gate board (`physical::tests::gate_board`)
//! carries one deliberate violation of each kind the checks once missed;
//! each must name its object and block production.
use super::Fallible;
use super::compare::{allowed, perimeter};
use super::excellon_reader::{self, Hit};
use super::expected::{self, raster_shape};
use super::raster::{self, Bitmap, Shape};
use super::reader;
use crate::diagnostics::ObjectRef;
use crate::fabrication::{self, FabricationOptions, FabricationSet, PreflightReport};
use crate::layout::physical::{self, HoleFeature, tests::gate_board};
use crate::model::{CopperLayer, Project};
use crate::{Engine, Query, QueryResult};

/// Pixel size of the copper comparisons, mm.
const PIXEL: f64 = 0.01;
/// How far a read-back drill may sit from its hole, mm: the files write
/// micrometres.
const DRILL_MM: f64 = 0.001;

fn report(engine: &Engine) -> Fallible<PreflightReport> {
    match engine.query(Query::Preflight { profile: None })? {
        QueryResult::Preflight(report) => Ok(*report),
        other => Err(format!("unexpected preflight result: {other:?}").into()),
    }
}

fn file<'a>(set: &'a FabricationSet, suffix: &str) -> Option<&'a str> {
    set.files
        .iter()
        .find(|f| f.name.ends_with(suffix))
        .and_then(|f| std::str::from_utf8(&f.bytes).ok())
}

/// The inventory's copper on `layer`, rendered without the writer: every
/// land (pads with or without a net, washers), track, via and stored fill
/// region.
fn inventory_copper(project: &Project, layer: CopperLayer, bitmap: &mut Bitmap) {
    for land in physical::lands(project).iter().filter(|l| l.on(layer)) {
        bitmap.render(&raster_shape(&land.outline), true);
    }
    for track in project.tracks.iter().filter(|t| t.layer == layer) {
        bitmap.render(
            &Shape::Capsule {
                a: track.start,
                b: track.end,
                r: track.width / 2.0,
            },
            true,
        );
    }
    let layers = project.stackup.layers();
    for via in project.vias.iter().filter(|v| v.on(layer, &layers)) {
        bitmap.render(
            &Shape::Disc {
                c: via.position,
                r: via.diameter / 2.0,
            },
            true,
        );
    }
    for fill in project.fills.iter().filter(|f| f.layer == layer) {
        for region in fill.computed.iter().flat_map(|c| &c.regions) {
            let mut contours = vec![region.outer.clone()];
            contours.extend(region.holes.iter().cloned());
            bitmap.render(&Shape::Poly { contours }, true);
        }
    }
}

/// Every plotted conductor of every copper layer lies in the inventory,
/// and every land of the inventory is plotted: the area dark in exactly
/// one of the read-back layer and the inventory's rendering stays within
/// sampling (a 2 µm band along the edges).
fn copper_matches(project: &Project, set: &FabricationSet) -> Fallible<()> {
    let grid = expected::board_bitmap(project, 1.003_17, PIXEL)?;
    for layer in project.stackup.layers() {
        let name = fabrication::artwork::copper_name(project, layer);
        let source = file(set, &format!("-{name}.gbr")).ok_or(format!("no {name}"))?;
        let mut got = grid.blank();
        raster::render_gerber(&reader::parse(source)?, &mut got)?;
        let mut want = grid.blank();
        inventory_copper(project, layer, &mut want);
        let mut extra = got.clone();
        extra.subtract(&want)?;
        let mut missing = want.clone();
        missing.subtract(&got)?;
        let band = allowed(&raster::compare(&got, &want)?, perimeter(&want));
        assert!(
            extra.area_mm2() <= band && missing.area_mm2() <= band,
            "{name}: plotted outside the inventory {} mm², inventory not plotted {} mm² (band {band})",
            extra.area_mm2(),
            missing.area_mm2()
        );
    }
    Ok(())
}

/// Every drill and slot of both Excellon files is one hole of the
/// inventory, of its size and plating, and every hole of the inventory is
/// drilled once.
fn drills_match(project: &Project, set: &FabricationSet) -> Fallible<()> {
    let mut holes = physical::holes(project);
    for (plated, suffix) in [(true, "-plated.drl"), (false, "-non-plated.drl")] {
        let Some(source) = file(set, suffix) else {
            continue;
        };
        let drill = excellon_reader::parse(source)?;
        for hit in &drill.hits {
            let size = drill
                .tools
                .get(&hit.tool())
                .copied()
                .ok_or("a hit of no tool")?;
            let at = holes.iter().position(|h| {
                h.plated == plated
                    && (h.feature.diameter() - size).abs() <= DRILL_MM
                    && match (h.feature, hit) {
                        (HoleFeature::Round { at, .. }, Hit::Round { at: p, .. }) => {
                            crate::geometry::distance(at, *p) <= DRILL_MM
                        }
                        (HoleFeature::Slot { from, to, .. }, Hit::Slot { from: a, to: b, .. }) => {
                            crate::geometry::distance(from, *a) <= DRILL_MM
                                && crate::geometry::distance(to, *b) <= DRILL_MM
                        }
                        _ => false,
                    }
            });
            let Some(at) = at else {
                return Err(
                    format!("{suffix}: {hit:?} ({size} mm) is no hole of the inventory").into(),
                );
            };
            holes.swap_remove(at);
        }
    }
    assert!(holes.is_empty(), "not drilled: {holes:#?}");
    Ok(())
}

#[test]
fn every_deliberate_violation_names_its_object_and_blocks_production() -> Fallible<()> {
    let (engine, gate) = gate_board()?;
    let report = report(&engine)?;
    let blocking = |code: &str, object: ObjectRef| {
        report
            .items
            .iter()
            .any(|i| i.code == code && i.objects.contains(&object) && i.blocks())
    };
    let pad = |pad| ObjectRef::Pad {
        instance: gate.r1,
        pad,
    };
    let track = |id| ObjectRef::Track { id };
    let cases = [
        (
            "fiducial across a net",
            "board.clearance",
            pad(gate.fiducial),
        ),
        (
            "fiducial across a net",
            "board.clearance",
            track(gate.over_fiducial),
        ),
        ("washer, net A", "board.clearance", track(gate.washer_a)),
        ("washer, net B", "board.clearance", track(gate.washer_b)),
        (
            "fill over another net",
            "fill.clearance",
            ObjectRef::Fill { id: gate.fill },
        ),
        ("undersize track", "board.track_width", track(gate.thin)),
        ("undersize pad drill", "board.drill_size", pad(gate.drilled)),
        (
            "track through a keepout",
            "keepout.track",
            track(gate.through_keepout),
        ),
    ];
    let missed: Vec<_> = cases
        .iter()
        .filter(|(_, code, object)| !blocking(code, *object))
        .map(|(what, code, object)| format!("{what}: {code} {object:?}"))
        .collect();
    assert!(
        missed.is_empty(),
        "{missed:#?}\n{:#?}",
        report
            .items
            .iter()
            .map(|i| (&i.code, &i.objects, &i.title))
            .collect::<Vec<_>>()
    );
    assert!(report.blocking >= cases.len() - 1);
    // The washer's finding names it in words: the board object alone does
    // not say which hole.
    assert!(
        report
            .items
            .iter()
            .any(|i| i.code == "board.clearance" && i.title.contains("mounting hole")),
        "{:#?}",
        report.items
    );
    Ok(())
}

#[test]
fn every_plotted_conductor_and_drill_is_in_the_physical_inventory() -> Fallible<()> {
    let (engine, _) = gate_board()?;
    let project = engine.project();
    let set = fabrication::generate(project, &FabricationOptions::default())?;
    copper_matches(project, &set)?;
    drills_match(project, &set)?;
    // The showcase too: four layers, inner fills, every pad kind it has.
    let showcase = Engine::showcase()?;
    let set = fabrication::generate(showcase.project(), &FabricationOptions::default())?;
    copper_matches(showcase.project(), &set)?;
    drills_match(showcase.project(), &set)?;
    Ok(())
}

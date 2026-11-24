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
            "fill.overlap",
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
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(project, None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    copper_matches(project, &set)?;
    drills_match(project, &set)?;
    // The showcase too: four layers, inner fills, every pad kind it has.
    let showcase = Engine::showcase()?;
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(showcase.project(), None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    copper_matches(showcase.project(), &set)?;
    drills_match(showcase.project(), &set)?;
    Ok(())
}

/// Copper the read-back layers must not hold, drawn from the board outline
/// and the inventory alone, never from a stored fill: (copper outside the
/// board, mm²; fill copper within a clearance of other copper, mm²). The
/// fill copper is the read-back copper no land, track or via of the
/// inventory explains. Its keep-away is each other conductor grown by its
/// fill clearance rule (less two pixels of sampling), less that
/// conductor's own copper and its own net's fills.
fn keep_away(project: &Project, set: &FabricationSet) -> Fallible<(f64, f64)> {
    use crate::layout::limits::BoardRules;
    use crate::rules::CopperObject;
    let board = project.board();
    let outline = |b: &crate::model::board::SolvedBoard| -> Vec<Vec<[f64; 2]>> {
        let mut rings = vec![b.outer.flatten(0.002)];
        rings.extend(b.cutouts.iter().map(|c| c.outline.flatten(0.002)));
        rings
    };
    let limits = BoardRules::of(project);
    let (mut outside, mut near) = (0.0, 0.0);
    for layer in project.stackup.layers() {
        let name = fabrication::artwork::copper_name(project, layer);
        let source = file(set, &format!("-{name}.gbr")).ok_or(format!("no {name}"))?;
        let gerber = reader::parse(source)?;
        // Outside the board: on a canvas reaching every stored fill, coarse.
        let mut extent = board.bounds();
        for fill in &project.fills {
            for p in fill
                .computed
                .iter()
                .flat_map(|c| &c.regions)
                .flat_map(|r| &r.outer)
            {
                extent = extent.including(*p);
            }
        }
        let mut wide = Bitmap::new(
            [extent.min[0] - 1.0, extent.min[1] - 1.0],
            [extent.max[0] + 1.0, extent.max[1] + 1.0],
            0.05,
        )?;
        let mut got = wide.blank();
        raster::render_gerber(&gerber, &mut got)?;
        wide.render(
            &Shape::Poly {
                contours: outline(&board),
            },
            true,
        );
        let band = perimeter(&wide) * 0.05 * 0.05 * 2.0;
        got.subtract(&wide)?;
        if got.area_mm2() > band {
            outside += got.area_mm2();
        }
        // Fill copper near other copper, at 10 µm.
        let grid = expected::board_bitmap(project, 1.003_17, PIXEL)?;
        let mut copper = grid.blank();
        raster::render_gerber(&gerber, &mut copper)?;
        let mut objects = grid.blank();
        // Every non-fill conductor of the layer, by its key: (shape, the
        // gap a fill keeps to it, its net).
        let mut conductors: Vec<(Shape, Shape, Option<crate::ids::Id>)> = Vec::new();
        let grown = |shape: &crate::geometry::Shape, by: f64| -> Shape {
            let ring = shape.flatten(0.002);
            let mut parts = vec![(true, raster_shape(shape))];
            for k in 0..ring.len() {
                if let (Some(a), Some(b)) = (ring.get(k), ring.get((k + 1) % ring.len())) {
                    parts.push((
                        true,
                        Shape::Capsule {
                            a: *a,
                            b: *b,
                            r: by,
                        },
                    ));
                }
            }
            Shape::Composite { parts }
        };
        for land in physical::lands(project).iter().filter(|l| l.on(layer)) {
            let gap = limits.clearance(CopperObject::Fill, None, CopperObject::Pad, land.net());
            conductors.push((
                raster_shape(&land.outline),
                grown(&land.outline, gap - 2.0 * PIXEL),
                land.net(),
            ));
        }
        for t in project.tracks.iter().filter(|t| t.layer == layer) {
            let gap = limits.clearance(CopperObject::Fill, None, CopperObject::Track, Some(t.net));
            let (a, b, r) = (t.start, t.end, t.width / 2.0);
            conductors.push((
                Shape::Capsule { a, b, r },
                Shape::Capsule {
                    a,
                    b,
                    r: r + gap - 2.0 * PIXEL,
                },
                Some(t.net),
            ));
        }
        let layers = project.stackup.layers();
        for v in project.vias.iter().filter(|v| v.on(layer, &layers)) {
            let gap = limits.clearance(CopperObject::Fill, None, CopperObject::Via, Some(v.net));
            let (c, r) = (v.position, v.diameter / 2.0);
            conductors.push((
                Shape::Disc { c, r },
                Shape::Disc {
                    c,
                    r: r + gap - 2.0 * PIXEL,
                },
                Some(v.net),
            ));
        }
        for (shape, _, _) in &conductors {
            objects.render(shape, true);
        }
        let mut fill = copper.clone();
        fill.subtract(&objects)?;
        let mut forbidden = grid.blank();
        for (k, (_, around, net)) in conductors.iter().enumerate() {
            let mut zone = grid.blank();
            zone.render(around, true);
            // Its own conductor: the same net's copper, or this land.
            let mut own = grid.blank();
            for (j, (shape, _, other)) in conductors.iter().enumerate() {
                if j == k || (net.is_some() && other == net) {
                    own.render(shape, true);
                }
            }
            for f in project
                .fills
                .iter()
                .filter(|f| f.layer == layer && net.is_some() && f.net == *net)
            {
                for region in f.computed.iter().flat_map(|c| &c.regions) {
                    let mut contours = vec![region.outer.clone()];
                    contours.extend(region.holes.iter().cloned());
                    own.render(&Shape::Poly { contours }, true);
                }
            }
            zone.subtract(&own)?;
            forbidden.union(&zone)?;
        }
        let mut hit = fill.clone();
        let mut clear = fill;
        clear.subtract(&forbidden)?;
        hit.subtract(&clear)?;
        if hit.area_mm2() > 4.0 * PIXEL * PIXEL {
            near += hit.area_mm2();
        }
    }
    Ok((outside, near))
}

/// The independent negative assertion: the shipped showcase's plotted
/// fills keep off the board edge and keep their gap to other copper,
/// measured from the outline and the inventory alone. The two stored-fill
/// faults of the re-review, drawn the same way, are found by it, and the
/// production check blocks both.
#[test]
fn plotted_fill_copper_keeps_away_from_other_copper_and_the_board_edge() -> Fallible<()> {
    let options = FabricationOptions::default();
    let generate = |engine: &Engine| {
        fabrication::generate(
            &fabrication::ProductionSnapshot::resolve(engine.project(), None)?,
            &options,
            &Default::default(),
        )
    };
    let showcase = Engine::showcase()?;
    let (outside, near) = keep_away(showcase.project(), &generate(&showcase)?)?;
    assert_eq!((outside, near), (0.0, 0.0), "the clean showcase");

    // A 1 × 1 mm island at (90, 80), off the 44 × 28 mm board.
    let mut island = Engine::showcase()?;
    let fill = island.project().fills.first().ok_or("a fill")?;
    let (id, mut copper) = (fill.id, fill.computed.clone().ok_or("copper")?);
    copper.regions.push(crate::model::fill::FilledRegion {
        outer: vec![[90.0, 80.0], [91.0, 80.0], [91.0, 81.0], [90.0, 81.0]],
        holes: Vec::new(),
    });
    island.apply(
        island.revision(),
        crate::Operation::Fill(crate::layout::fill::FillCommand::Store {
            fill: id,
            copper: Box::new(copper),
        }),
    )?;
    let (outside, _) = keep_away(island.project(), &generate(&island)?)?;
    assert!((outside - 1.0).abs() < 0.05, "{outside} mm² off the board");
    assert!(
        report(&island)?
            .items
            .iter()
            .any(|i| i.code == "fill.board_edge" && i.blocks())
    );

    // The gate board's net B fill 0.025 mm from the net A track under a
    // 0.2 mm rule: 8 mm of it at 0.175 mm short, 1.4 mm².
    let (mut gate, g) = crate::layout::physical::tests::gate_board_with(&["fill"])?;
    gate.apply(
        gate.revision(),
        crate::Operation::Rules(crate::rules::RulesCommand::SetOverride {
            key: crate::rules::RuleKey::Clearance {
                a: crate::rules::CopperObject::Fill,
                b: crate::rules::CopperObject::Track,
            },
            value: crate::rules::RuleValue::Length(0.2),
        }),
    )?;
    let key = crate::layout::fill::inputs_key(gate.project(), g.fill)?;
    gate.apply(
        gate.revision(),
        crate::Operation::Fill(crate::layout::fill::FillCommand::Store {
            fill: g.fill,
            copper: Box::new(crate::model::fill::FilledCopper {
                key,
                regions: vec![crate::model::fill::FilledRegion {
                    outer: vec![[46.0, 34.15], [54.0, 34.15], [54.0, 37.0], [46.0, 37.0]],
                    holes: Vec::new(),
                }],
                removed_islands: 0,
                removed_spokes: Vec::new(),
            }),
        }),
    )?;
    let (_, near) = keep_away(gate.project(), &generate(&gate)?)?;
    assert!(near > 1.0 && near < 1.6, "{near} mm² within the clearance");
    assert!(report(&gate)?.items.iter().any(|i| {
        i.code == "fill.clearance"
            && i.objects.contains(&ObjectRef::Track { id: g.under_fill })
            && i.blocks()
    }));
    Ok(())
}

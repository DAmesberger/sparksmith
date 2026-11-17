//! Zones of Rust-built synthetic boards: fills defined from the zone and
//! filled by our engine, rule areas as keepouts.
mod support;

use sparksmith_core::CopperLayer;
use sparksmith_core::model::fill::IslandRemoval;
use sparksmith_core::rules::PadConnection;
use sparksmith_kicad::report::Case;
use support::{Board, Footprint, Pad, import};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

const ZONE: &str = "
  (net 1 \"GND\")
  (gr_rect (start 0 0) (end 30 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))
  (zone (net 1) (net_name \"GND\") (layers \"F&B.Cu\") (hatch edge 0.5) (priority 2)
    (connect_pads yes (clearance 0.3)) (min_thickness 0.25) (filled_areas_thickness no)
    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.4) (island_removal_mode 1))
    (polygon (pts (xy 1 1) (xy 29 1) (xy 29 19) (xy 1 19))))
  (zone (net 0) (net_name \"\") (layer \"F.Cu\") (hatch edge 0.5)
    (connect_pads (clearance 0)) (min_thickness 0.25)
    (keepout (tracks not_allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
    (fill (thermal_gap 0.5) (thermal_bridge_width 0.5))
    (polygon (pts (xy 20 5) (xy 25 5) (xy 25 10) (xy 20 10))))
";

fn zoned() -> Board {
    let gnd = |n: &'static str, x: f64| {
        Pad::smd(n, "rect", [x, 0.0, 0.0], [0.8, 0.6]).with("(net 1 \"GND\")")
    };
    let mut fp = Footprint::new(
        "R1",
        [8.0, 10.0, 0.0],
        false,
        vec![gnd("1", -1.0), gnd("2", 1.0)],
    );
    // A footprint's own rule area: KiCad writes its outline in board
    // coordinates.
    fp.extra = " (zone (net 0) (net_name \"\") (layer \"F.Cu\") (hatch edge 0.5) \
        (connect_pads (clearance 0)) (min_thickness 0.25) \
        (keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed)) \
        (fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) \
        (polygon (pts (xy 4 14) (xy 6 14) (xy 6 16) (xy 4 16))))"
        .into();
    Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![fp],
        items: ZONE.into(),
    }
}

#[test]
fn a_zone_becomes_a_fill_per_layer_filled_by_our_engine() -> Fallible<()> {
    let imported = import(zoned().text().as_bytes())?;
    let project = imported.engine.project();
    assert_eq!(imported.report.count(Case::Zone), 2);
    assert_eq!(project.fills.len(), 2);
    let layers: Vec<CopperLayer> = project.fills.iter().map(|f| f.layer).collect();
    assert_eq!(layers, [CopperLayer::Top, CopperLayer::Bottom]);
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let gnd = project
        .net_of(sparksmith_core::Endpoint {
            instance: r1.id,
            terminal: sparksmith_core::TerminalId(1),
        })
        .ok_or("GND")?
        .id;
    for fill in project.fills.iter() {
        assert_eq!(fill.net, Some(gnd));
        assert_eq!(fill.priority, 2);
        assert_eq!(fill.settings.clearance, Some(0.3));
        assert_eq!(fill.settings.width_min, Some(0.25));
        assert_eq!(fill.settings.connections.smd, Some(PadConnection::Solid));
        assert_eq!(fill.settings.islands, IslandRemoval::Never);
        let thermal = fill.settings.thermal.ok_or("thermal")?;
        assert!((thermal.gap - 0.5).abs() < 1e-9 && (thermal.spoke_width - 0.4).abs() < 1e-9);
        // Our engine filled it: copper of the 28 × 18 mm outline.
        let copper = fill.computed.as_ref().ok_or("not filled")?;
        assert!(!copper.regions.is_empty());
    }
    Ok(())
}

#[test]
fn rule_areas_become_keepouts_on_their_layers() -> Fallible<()> {
    let imported = import(zoned().text().as_bytes())?;
    let project = imported.engine.project();
    assert_eq!(imported.report.count(Case::Keepout), 2);
    assert_eq!(project.keepouts.len(), 2);
    let board = &project.keepouts[0];
    assert!(board.forbid.tracks && board.forbid.fills && !board.forbid.vias);
    // The footprint's area forbids only pour, at KiCad's (4, 14)–(6, 16):
    // ours (4, 6)–(6, 4) about the outline's corner (0, 20).
    let own = &project.keepouts[1];
    assert!(own.forbid.fills && !own.forbid.tracks);
    let b = sparksmith_core::geometry::Bounds::of(own.outline.flatten(1e-3)).ok_or("box")?;
    assert!((b.min[0] - 4.0).abs() < 1e-9 && (b.min[1] - 4.0).abs() < 1e-9);
    assert!((b.max[0] - 6.0).abs() < 1e-9 && (b.max[1] - 6.0).abs() < 1e-9);
    Ok(())
}

#[test]
fn footprint_and_pad_zone_overrides_become_fill_overrides() -> Fallible<()> {
    use sparksmith_core::model::fill::PadFillOverride;
    let mut board = zoned();
    // The footprint: not joined, 0.3 mm spokes; pad 2: thermal at 45° and
    // a 0.4 mm clearance.
    board.footprints[0]
        .extra
        .push_str(" (zone_connect 0) (thermal_width 0.3)");
    board.footprints[0].pads[1] = Pad::smd("2", "rect", [1.0, 0.0, 0.0], [0.8, 0.6])
        .with("(net 1 \"GND\") (zone_connect 1) (thermal_bridge_angle 45) (clearance 0.4)");
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let placement = project
        .placements
        .iter()
        .find(|p| p.instance == r1.id)
        .ok_or("placement")?;
    assert_eq!(
        placement.fill_override,
        PadFillOverride {
            connection: Some(PadConnection::None),
            spoke_width: sparksmith_core::units::Nm::try_new(300_000),
            ..Default::default()
        }
    );
    assert_eq!(placement.pad_fill_overrides.len(), 1);
    let pad = placement.pad_fill_overrides[0];
    assert_eq!(pad.settings.connection, Some(PadConnection::Thermal));
    assert_eq!(
        pad.settings.spoke_angle.map(|a| a.to_degrees_f64()),
        Some(45.0)
    );
    assert_eq!(pad.settings.clearance_mm(), Some(0.4));
    // Pad 2 keeps the footprint's spoke width.
    assert_eq!(
        placement.fill_override_of(pad.pad).spoke_width,
        sparksmith_core::units::Nm::try_new(300_000)
    );
    assert_eq!(imported.report.count(Case::PadZoneOverrides), 1);
    assert_eq!(imported.report.count(Case::FootprintZoneOverrides), 1);
    // Pad 1 follows the footprint, pad 2 its own.
    let joined: Vec<_> = sparksmith_core::layout::placed_pads(project)
        .into_iter()
        .filter(|p| p.instance == r1.id)
        .map(|p| p.fill_override.connection)
        .collect();
    assert_eq!(
        joined,
        [Some(PadConnection::None), Some(PadConnection::Thermal)]
    );
    Ok(())
}

#[test]
fn a_round_pad_without_its_own_spoke_angle_gets_spokes_at_45_degrees() -> Fallible<()> {
    let mut board = zoned();
    // Pad 1 round without an angle, pad 2 round at 0°; pad 3 square.
    board.footprints[0].pads = vec![
        Pad::smd("1", "circle", [-1.0, 0.0, 0.0], [0.8, 0.8]).with("(net 1 \"GND\")"),
        Pad::smd("2", "circle", [1.0, 0.0, 0.0], [0.8, 0.8])
            .with("(net 1 \"GND\") (thermal_bridge_angle 0)"),
        Pad::smd("3", "rect", [3.0, 0.0, 0.0], [0.8, 0.8]).with("(net 1 \"GND\")"),
    ];
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let angles: Vec<_> = sparksmith_core::layout::placed_pads(project)
        .into_iter()
        .filter(|p| p.instance == r1.id)
        .map(|p| p.fill_override.spoke_angle.map(|a| a.to_degrees_f64()))
        .collect();
    assert_eq!(angles, [Some(45.0), Some(0.0), None]);
    // Only the pad that states a zone override counts as one.
    assert_eq!(imported.report.count(Case::PadZoneOverrides), 1);
    Ok(())
}

#[test]
fn a_zone_connecting_pads_in_full_joins_them_solidly() -> Fallible<()> {
    let text = zoned().text().replace(
        "(connect_pads yes (clearance 0.3))",
        "(connect_pads full (clearance 0.3))",
    );
    assert!(text.contains("connect_pads full"));
    let imported = import(text.as_bytes())?;
    for fill in imported.engine.project().fills.iter() {
        assert_eq!(fill.settings.connections.smd, Some(PadConnection::Solid));
        assert_eq!(
            fill.settings.connections.through_hole,
            Some(PadConnection::Solid)
        );
    }
    Ok(())
}

/// A board-wide GND pour over a SOT-23-5 land pattern (0.95 mm pitch,
/// 0.6 × 1.0 mm lands of `shape`) whose middle pin of the three is GND,
/// beside an 0603 capacitor, with 0.3 mm spokes across a 0.25 mm relief
/// gap turned `angle`; with no angle, the fill states no relief of its own
/// and the board takes the shipped JLCPCB 2-layer profile, whose spokes
/// are 0.3 mm across 0.25 mm too. The fill's copper, U1 pad 2's centre and
/// its object reference.
fn sot_pour(
    shape: &'static str,
    angle: Option<f64>,
) -> Fallible<(
    sparksmith_core::model::fill::FilledCopper,
    [f64; 2],
    sparksmith_core::diagnostics::ObjectRef,
)> {
    use sparksmith_core::Operation;
    use sparksmith_core::layout::fill::{FillCommand, RefillTarget};
    use sparksmith_core::rules::ThermalRelief;
    let pad = |n: &'static str, x: f64, y: f64, size: [f64; 2], net: &str| {
        Pad::smd(n, shape, [x, y, 0.0], size).with(&format!("(roundrect_rratio 0.25) {net}"))
    };
    let (gnd, vin, out, en, nc) = (
        "(net 1 \"GND\")",
        "(net 2 \"VIN\")",
        "(net 3 \"OUT\")",
        "(net 4 \"EN\")",
        "(net 5 \"NC\")",
    );
    let lead = [0.6, 1.0];
    let u1 = Footprint::new(
        "U1",
        [12.0, 12.0, 0.0],
        false,
        vec![
            pad("1", -0.95, 1.1, lead, vin),
            pad("2", 0.0, 1.1, lead, gnd),
            pad("3", 0.95, 1.1, lead, en),
            pad("4", 0.95, -1.1, lead, nc),
            pad("5", -0.95, -1.1, lead, out),
        ],
    );
    let c1 = Footprint::new(
        "C1",
        [20.0, 12.0, 0.0],
        false,
        vec![
            pad("1", -0.8, 0.0, [0.9, 1.0], vin),
            pad("2", 0.8, 0.0, [0.9, 1.0], gnd),
        ],
    );
    let board = Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![u1, c1],
        items: "
  (net 1 \"GND\") (net 2 \"VIN\") (net 3 \"OUT\") (net 4 \"EN\") (net 5 \"NC\")
  (gr_rect (start 0 0) (end 35 28) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))
  (zone (net 1) (net_name \"GND\") (layer \"F.Cu\") (hatch edge 0.5)
    (connect_pads (clearance 0.2)) (min_thickness 0.1) (filled_areas_thickness no)
    (fill yes (thermal_gap 0.25) (thermal_bridge_width 0.3) (island_removal_mode 0))
    (polygon (pts (xy 0 0) (xy 35 0) (xy 35 28) (xy 0 28))))
"
        .into(),
    };
    let mut imported = import(board.text().as_bytes())?;
    let engine = &mut imported.engine;
    let project = engine.project();
    let u1 = project.instance_by_designator("U1").ok_or("U1")?.id;
    let middle = sparksmith_core::layout::placed_pads(project)
        .into_iter()
        .find(|p| p.instance == u1 && p.pad.number == "2")
        .ok_or("U1 pad 2")?;
    let me = sparksmith_core::diagnostics::ObjectRef::Pad {
        instance: u1,
        pad: middle.pad.id,
    };
    let centre = middle.pad_transform().apply([0.0, 0.0]);
    let mut fill = project.fills.first().ok_or("no fill")?.clone();
    fill.settings.thermal = angle.map(|angle_deg| ThermalRelief {
        spoke_width: 0.3,
        spoke_count: 4,
        gap: 0.25,
        angle_deg,
    });
    let id = fill.id;
    if angle.is_none() {
        engine.apply(
            engine.revision(),
            Operation::Rules(sparksmith_core::rules::RulesCommand::set_profile(
                sparksmith_core::rules::fixtures::jlcpcb_example()?,
                false,
            )),
        )?;
    }
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Edit {
            fill: Box::new(fill),
        }),
    )?;
    engine.apply(
        engine.revision(),
        Operation::Fill(FillCommand::Refill {
            target: RefillTarget::All,
        }),
    )?;
    let copper = engine
        .project()
        .fill(id)
        .and_then(|f| f.computed.clone())
        .ok_or("not filled")?;
    Ok((copper, centre, me))
}

/// The fill island holding `p`, by index.
fn island_at(copper: &sparksmith_core::model::fill::FilledCopper, p: [f64; 2]) -> Option<usize> {
    use sparksmith_core::geometry::polygon_contains;
    copper.regions.iter().position(|r| {
        polygon_contains(&r.outer, p) && !r.holes.iter().any(|h| polygon_contains(h, p))
    })
}

/// How many thermal spokes of `object` the fill removed.
fn spokes_removed(
    copper: &sparksmith_core::model::fill::FilledCopper,
    object: sparksmith_core::diagnostics::ObjectRef,
) -> Option<u32> {
    copper
        .removed_spokes
        .iter()
        .find(|r| r.object == object)
        .map(|r| r.spokes)
}

/// Run 10, finding 1: on rounded lands, a neighbour's clearance cuts each
/// 45° spoke of the middle pin down to part of its width just past the
/// relief, and the rest of its width runs on into the pour. The spokes
/// join, so all four stay and carry the pad into the pour; judging only
/// each spoke's centre line removed all four.
#[test]
fn a_spoke_narrowed_by_a_neighbours_clearance_still_joins_the_pour() -> Fallible<()> {
    let (copper, centre, me) = sot_pour("roundrect", Some(45.0))?;
    assert_eq!(
        spokes_removed(&copper, me),
        None,
        "{:?}",
        copper.removed_spokes
    );
    let pour = island_at(&copper, [2.0, 2.0]).ok_or("no pour")?;
    // Each spoke crosses the gap (the land's corner lies 0.42 mm out
    // along it, the gap's far side 0.77 mm) as copper of the pour itself.
    for k in 0..4 {
        let a = (45.0 + 90.0 * f64::from(k)).to_radians();
        let p = [centre[0] + 0.6 * a.cos(), centre[1] + 0.6 * a.sin()];
        assert_eq!(island_at(&copper, p), Some(pour), "spoke {k}");
    }
    Ok(())
}

/// True dead ends are still removed: on the same rounded lands turned to
/// 0°, the two spokes along the row end in the neighbours' clearances and
/// go, and the two across it join; on sharp lands the 45° spokes end in
/// the neighbours' clearances with nothing beside them, and all four go.
#[test]
fn spokes_that_end_in_a_clearance_are_still_removed() -> Fallible<()> {
    let (copper, centre, me) = sot_pour("roundrect", Some(0.0))?;
    assert_eq!(spokes_removed(&copper, me), Some(2));
    let pour = island_at(&copper, [2.0, 2.0]).ok_or("no pour")?;
    // Mid-gap: half the land's width (0.3) or length (0.5) plus half the
    // 0.25 mm gap.
    for (deg, kept) in [
        (0.0_f64, false),
        (90.0, true),
        (180.0, false),
        (270.0, true),
    ] {
        let a = deg.to_radians();
        let r = if kept { 0.5 } else { 0.3 } + 0.125;
        let p = [centre[0] + r * a.cos(), centre[1] + r * a.sin()];
        assert_eq!(island_at(&copper, p) == Some(pour), kept, "spoke at {deg}°");
    }
    let (copper, _, me) = sot_pour("rect", Some(45.0))?;
    assert_eq!(spokes_removed(&copper, me), Some(4));
    Ok(())
}

/// The founder's rule of 16 November 2026: the shipped profiles turn the
/// spokes of pads that are not round along the pad's axes. On sharp SOT-23-5
/// lands, where 45° spokes all end in the neighbours' clearances and the
/// pin never joined the pour, the middle pin's spokes across the row now
/// carry it into the pour; the two along the row still end in the
/// neighbours' clearances and go.
#[test]
fn a_sot_23_5_middle_pin_joins_the_pour_with_spokes_along_its_axes() -> Fallible<()> {
    let profile = sparksmith_core::rules::fixtures::jlcpcb_example()?;
    assert_eq!(profile.rules().fill.thermal.angle_deg, 0.0);
    assert_eq!(profile.rules().fill.thermal_angle_round, Some(45.0));
    let (copper, centre, me) = sot_pour("rect", None)?;
    assert_eq!(spokes_removed(&copper, me), Some(2));
    let pour = island_at(&copper, [2.0, 2.0]).ok_or("no pour")?;
    // Mid-gap: half the land's width (0.3) or length (0.5) plus half the
    // 0.25 mm gap.
    for (deg, kept) in [
        (0.0_f64, false),
        (90.0, true),
        (180.0, false),
        (270.0, true),
    ] {
        let a = deg.to_radians();
        let r = if kept { 0.5 } else { 0.3 } + 0.125;
        let p = [centre[0] + r * a.cos(), centre[1] + r * a.sin()];
        assert_eq!(island_at(&copper, p) == Some(pour), kept, "spoke at {deg}°");
    }
    // Nothing at 45°: no diagonal spoke is drawn.
    let a = 45.0_f64.to_radians();
    let p = [centre[0] + 0.6 * a.cos(), centre[1] + 0.6 * a.sin()];
    assert_ne!(island_at(&copper, p), Some(pour));
    Ok(())
}

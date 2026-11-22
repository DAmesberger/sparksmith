//! The shipped USB-C receptacle (GCT USB4105) against the JLCPCB and PCBWay
//! profiles on the ESP32 board (run 10, finding 4; run 11, finding 2).
#[path = "support/esp32_preflight.rs"]
mod esp32_preflight;

use esp32_preflight::{USB_UNCOUPLED_MM, assert_usb_preflight};
use sparksmith_core::layout;
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The shell slots keep the 0.2 mm ring of GCT's recommended layout
/// (1.0 mm lands on 0.6 mm slots, drawing USB4105 revision B4). JLCPCB
/// recommends 0.25 mm and allows down to 0.18 mm: each finding names the
/// profile choice that allows the ring.
#[test]
fn a_recommended_ring_limit_names_the_choice_that_allows_the_ring() -> Fallible {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut p = sparksmith_io::read_project(&path)?.project;
    // The shipped JLCPCB 2-layer profile, at its default (recommended)
    // values.
    p.rules.profile = Some(sparksmith_core::rules::fixtures::jlcpcb_example()?);
    let rings: Vec<String> = layout::checks(&p, &Default::default())
        .into_iter()
        .filter(|f| f.code == "board.pth_annular_ring" && f.message.starts_with("J1 pad S"))
        .map(|f| f.message)
        .collect();
    assert_eq!(rings.len(), 4, "{rings:?}");
    for ring in &rings {
        assert!(
            ring.starts_with("J1 pad S: annular ring 0.2 mm; needs at least 0.25 mm")
                && ring.ends_with("allows 0.18 mm under JLCPCB values: Absolute minimum"),
            "{ring}"
        );
    }
    Ok(())
}

/// Under JLCPCB's published absolute minimum (2-layer, 1 oz PTH annular
/// ring 0.18 mm, the profile's `limits: minimum` choice) GCT's 0.2 mm slot
/// rings pass, and J1's footprint keeps exactly its two locating-hole
/// findings: the drawing's 0.175 mm from each Ø0.65 hole to the GND land
/// beside it, against "NPTH to Track" 0.2 mm, which JLCPCB publishes with
/// no option that relaxes it (docs/e2e-run-11.md, finding 2 follow-up).
#[test]
fn the_absolute_minimum_admits_the_rings_but_not_the_locating_holes() -> Fallible {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut p = sparksmith_io::read_project(&path)?.project;
    let choices = std::collections::BTreeMap::from([("limits".to_owned(), "minimum".to_owned())]);
    p.rules.profile =
        Some(sparksmith_core::rules::fixtures::jlcpcb_example()?.with_choices(&choices)?);
    let j1: Vec<String> = layout::checks(&p, &Default::default())
        .into_iter()
        .filter(|f| f.message.starts_with("J1 "))
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    assert!(
        !j1.iter().any(|f| f.starts_with("board.pth_annular_ring")),
        "{j1:?}"
    );
    let holes: Vec<&String> = j1
        .iter()
        .filter(|f| f.starts_with("board.pad_hole_clearance"))
        .collect();
    assert_eq!(holes.len(), 2, "{j1:?}");
    for (hole, land) in [("17", "A1B12"), ("18", "B1A12")] {
        let want = format!("J1 pad {hole}: hole is 0.175 mm from J1 pad {land} on GND");
        assert!(holes.iter().any(|f| f.contains(&want)), "{want}: {holes:?}");
    }
    Ok(())
}

/// A shipped PCBWay profile, at its defaults or under its published
/// minimum values (`limits: minimum`).
fn pcbway(id: &str, minimum: bool) -> Fallible<sparksmith_core::rules::RuleProfile> {
    let profile = sparksmith_io::rule_profiles::shipped_from_env()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or(format!("the shipped profile {id}"))?;
    if !minimum {
        return Ok(profile);
    }
    let choices = std::collections::BTreeMap::from([("limits".to_owned(), "minimum".to_owned())]);
    Ok(profile.with_choices(&choices)?)
}

/// J1's own findings on the example board under a profile.
fn j1_findings(profile: sparksmith_core::rules::RuleProfile) -> Fallible<Vec<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut p = sparksmith_io::read_project(&path)?.project;
    p.rules.profile = Some(profile);
    Ok(layout::checks(&p, &Default::default())
        .into_iter()
        .filter(|f| f.message.starts_with("J1 "))
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect())
}

/// PCBWay publishes "Min width between NP holes and copper" 0.15 mm, which
/// admits the drawing's 0.175 mm from each locating hole to its GND land,
/// and a 0.2 mm PTH ring at 1 oz ("Normal PTH pad single side width"),
/// which the 0.2 mm slot rings meet. Under the minimum values J1 has no
/// finding on 2 or 4 layers; at the normal values only the two locating
/// holes remain, against "Normal Width between NP holes and copper" 0.2 mm.
#[test]
fn pcbway_admits_the_usb4105_footprint_under_its_minimum_values() -> Fallible {
    for id in ["pcbway-2-layer", "pcbway-4-layer"] {
        let minimum = j1_findings(pcbway(id, true)?)?;
        assert!(minimum.is_empty(), "{id}: {minimum:?}");
        let normal = j1_findings(pcbway(id, false)?)?;
        assert_eq!(normal.len(), 2, "{id}: {normal:?}");
        for (hole, land) in [("17", "A1B12"), ("18", "B1A12")] {
            let want = format!(
                "board.pad_hole_clearance: J1 pad {hole}: hole is 0.175 mm from J1 pad {land} on GND"
            );
            assert!(
                normal.iter().any(|f| f.starts_with(&want)),
                "{id}: {normal:?}"
            );
        }
    }
    Ok(())
}

/// The 4-layer example board is designed under PCBWay 4-layer at its
/// minimum values (the replay sets the profile, chooses `limits: minimum`
/// and routes the USB pair with `layout.route_pair`, which takes its width
/// and gap from the profile). It holds no waiver. The only preflight item
/// names the lengths excluded from the USB pair's impedance calculation.
#[test]
fn the_example_board_passes_pcbway_4_layer_without_waivers() -> Fallible {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let file = sparksmith_io::read_project(&path)?;
    assert_eq!(
        file.project.rules.profile,
        Some(pcbway("pcbway-4-layer", true)?),
        "the board pins the shipped PCBWay 4-layer profile at its minimum values"
    );
    assert!(file.project.production.waivers.is_empty());
    let mut engine = sparksmith_core::Engine::from_file(file)?;
    // A file holds no fill copper; fill it as opening does.
    engine.refill_stale()?;
    let preflight = match engine.query(sparksmith_core::Query::Preflight { profile: None })? {
        sparksmith_core::QueryResult::Preflight(report) => report,
        _ => return Err("preflight".into()),
    };
    assert_usb_preflight(&preflight, USB_UNCOUPLED_MM);
    Ok(())
}

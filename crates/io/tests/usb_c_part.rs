//! The shipped USB-C receptacle (GCT USB4105) against the JLCPCB profile
//! on the ESP32 board (run 10, finding 4).
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

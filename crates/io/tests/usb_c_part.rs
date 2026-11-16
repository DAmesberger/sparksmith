//! The built-in USB-C receptacle (GCT USB4105) against the JLCPCB profile
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

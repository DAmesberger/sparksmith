//! Lean parts (docs/part-templates.md §11): cold and cached cost of the
//! shipped ESP32-S31-WROOM-3 part file and of inserting it into a board.
//! Measured in a release build:
//!
//! `cargo test --release -p sparksmith-solidcraft --test part_timings -- --ignored --nocapture`
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sparksmith_core::library::package_model::PartValue;
use sparksmith_solidcraft::models::part_file;
use sparksmith_solidcraft::native::Native;
use std::time::{Duration, Instant};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[test]
#[ignore = "timings"]
fn part_file_timings() {
    // The repository's shipped material library (the program loads it
    // through sparksmith-io at start-up).
    let materials =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/materials");
    let library = sparksmith_core::materials::MaterialLibrary::load(|name| {
        std::fs::read_to_string(materials.join(name))
            .map_err(|e| sparksmith_core::Error::Invalid(e.to_string()))
    })
    .map_err(|e| e.to_string());
    let standards = sparksmith_core::library::standards::Context::fixture().unwrap();
    let context = std::sync::Arc::new(
        sparksmith_core::library::standards::Context::new((**standards.standards()).clone())
            .with_materials(library),
    );
    let _context = context.enter();
    let bytes = &std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../library/parts/esp32-s31-wroom-3.solidcraft"
    ))
    .unwrap();
    for round in 1..=3 {
        // A parameter set this process has not evaluated: cold.
        let mut values = part_file::PartValues::new();
        values.insert(
            "can_round".into(),
            PartValue::Nm(250_000 + i64::from(round) * 10_000),
        );
        let t = Instant::now();
        let part = part_file::evaluate(bytes, &values).unwrap();
        let cold = t.elapsed();
        assert!(!part.blocked(), "{:?}", part.findings);
        // The same set again: the cache.
        let t = Instant::now();
        let again = part_file::evaluate(bytes, &values).unwrap();
        let cached = t.elapsed();
        assert!(std::sync::Arc::ptr_eq(&part, &again));
        // Into a board design: one definition, one evaluation.
        let mut board = Native::new("board").unwrap();
        let t = Instant::now();
        part_file::insert(&mut board, &part.stem(), &part, &Default::default()).unwrap();
        let insert = t.elapsed();
        assert!(board.evaluation_error().is_none());
        eprintln!(
            "round {round}: part file cold (read + apply values + evaluate + inspect) {:.1} ms \
             (Solidcraft evaluation {:.1} ms) | cached {:.3} ms | insert into a board {:.1} ms | {} bodies",
            ms(cold),
            part.evaluate_ms,
            ms(cached),
            ms(insert),
            part.bodies.len()
        );
    }
    let (hits, evaluations) = part_file::cache_counts();
    eprintln!("cache: {hits} hits, {evaluations} evaluations");
}

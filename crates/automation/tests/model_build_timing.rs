//! What the board's 3D build costs on the ESP32-S31 example and the
//! shipped showcase: the first build in a process (every package model
//! built) and a second, unedited build in the same process (process-wide
//! model caches warm). Slice S4 of docs/board-sketch-references.md builds
//! generated models as standalone named designs and inserts them; this
//! compares that with the build before it.
//!
//! `nix develop -c cargo test --profile ci -p sparksmith-automation --test model_build_timing -- --ignored --nocapture --test-threads 1`
//! Run each test in its own process for a cold first build
//! (`-- --exact esp32_build_timing`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sparksmith_core::{Engine, Evaluators, Project};
use sparksmith_solidcraft::models::imported::SourceCache;
use sparksmith_solidcraft::project;
use std::time::Instant;

fn esp32() -> Project {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    sparksmith_io::read_project(&path)
        .unwrap()
        .into_document()
        .project
        .as_ref()
        .clone()
}

fn showcase() -> Project {
    let engine = Engine::showcase_hosted_fixture(Evaluators {
        footprint: Some(&sparksmith_solidcraft::footprint::SketchEvaluator),
        board: Some(&sparksmith_solidcraft::board::SketchBoardEvaluator),
    })
    .unwrap();
    engine.project().clone()
}

/// Build `project` `runs` times in this process with one source cache, as
/// the model worker does, and print each build's time.
fn time(name: &str, project: &Project, runs: usize) {
    let sources = SourceCache::default();
    let mut times = Vec::new();
    for _ in 0..runs {
        let start = Instant::now();
        let model = project::build_with(project, &sources, None).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1e3);
        assert!(!model.geometry.is_empty());
    }
    let label: Vec<String> = times.iter().map(|t| format!("{t:.0}")).collect();
    eprintln!(
        "{name}: {} placements; builds (ms, first cold): {}",
        project.placements.len(),
        label.join(", ")
    );
}

#[test]
#[ignore = "benchmark"]
fn esp32_build_timing() {
    // The 3D build reads the shipped material library.
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    time("esp32-s31-devboard", &esp32(), 4);
}

#[test]
#[ignore = "benchmark"]
fn showcase_build_timing() {
    // The 3D build reads the shipped material library.
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    time("showcase", &showcase(), 4);
}

/// A saved board (`SPARKSMITH_BUILD_FILE=board.sparksmith`) built for the
/// live view twice in one process, as opening it again does: the second
/// build evaluates no part file (each is evaluated once, then served by
/// the process-wide part-file cache). Prints both builds' times and the
/// cache's counts.
///
/// `SPARKSMITH_BUILD_FILE=… nix develop -c cargo test --profile ci -p sparksmith-automation --test model_build_timing -- --ignored --nocapture --exact a_saved_board_evaluates_each_part_file_once`
#[test]
#[ignore = "builds the design named by SPARKSMITH_BUILD_FILE"]
fn a_saved_board_evaluates_each_part_file_once() {
    use sparksmith_solidcraft::models::part_file;
    let Ok(path) = std::env::var("SPARKSMITH_BUILD_FILE") else {
        panic!("set SPARKSMITH_BUILD_FILE=board.sparksmith");
    };
    let context = sparksmith_io::standards::shipped_context().unwrap();
    let _context = context.enter();
    let project = sparksmith_io::read_project(std::path::Path::new(&path))
        .unwrap()
        .into_document()
        .project
        .as_ref()
        .clone();
    let mut evaluated = Vec::new();
    for round in 1..=2 {
        let (_, before) = part_file::cache_counts();
        let start = Instant::now();
        project::build_flat(
            &project,
            &SourceCache::default(),
            None,
            project::Flat::Meshed,
        )
        .unwrap();
        let ms = start.elapsed().as_secs_f64() * 1e3;
        let (hits, after) = part_file::cache_counts();
        eprintln!(
            "build {round}: {ms:.0} ms, {} part files evaluated (cache: {hits} hits, {after} \
             evaluations)",
            after - before
        );
        evaluated.push(after - before);
    }
    assert!(evaluated[0] > 0, "the first build evaluates the parts");
    assert_eq!(evaluated[1], 0, "the second build evaluates none again");
}

//! Scale benchmark: a synthetic large board built here (original: a grid
//! of the showcase's parts, a via field and routed channels; no real
//! design), timed through the paths the app takes. It reports; it holds no
//! time bar, so a loaded machine cannot fail it. The founder's targets
//! (10,000 vias in seconds, a 3D view that opens instantly) are targets
//! until a quiet machine measures them.
//!
//! `cargo test --profile ci -p sparksmith-solidcraft --test large_board -- --ignored --nocapture`
//! (release-like; a debug build is several times slower).
mod large_board_fixture;

use large_board_fixture::{BOARD, Fallible, TRACK_WIDTH, VIA_GRID, large_board};
use sparksmith_core::issues::IssueScope;
use sparksmith_core::layout::fill::RefillTarget;
use sparksmith_core::layout::{self, LayoutCommand, RouteRequest};
use sparksmith_core::model::Layer;
use sparksmith_core::{Engine, ObjectRef, Operation, Query, QueryResult, format};
use sparksmith_solidcraft::models::imported::SourceCache;
use sparksmith_solidcraft::project::{self, Flat, plan};
use std::time::Instant;

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

/// Best and median of `times`, ms.
fn best_median(times: &mut [f64]) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    (
        times.first().copied().unwrap_or(f64::NAN),
        times.get(times.len() / 2).copied().unwrap_or(f64::NAN),
    )
}

#[test]
#[ignore = "scale benchmark; run with --ignored --nocapture"]
fn large_board_timings() -> Fallible {
    let load = std::fs::read_to_string("/proc/loadavg").ok().or_else(|| {
        std::process::Command::new("sysctl")
            .args(["-n", "vm.loadavg"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
    });
    eprintln!("load average: {}", load.unwrap_or_default().trim());
    let materials =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/materials");
    let library = sparksmith_core::materials::MaterialLibrary::load(|name| {
        std::fs::read_to_string(materials.join(name))
            .map_err(|error| sparksmith_core::Error::Invalid(error.to_string()))
    })?;
    let standards = sparksmith_core::library::standards::Context::fixture()?;
    let context = std::sync::Arc::new(
        sparksmith_core::library::standards::Context::new((**standards.standards()).clone())
            .with_materials(Ok(library)),
    );
    let _context = context.enter();
    let start = Instant::now();
    let board = large_board()?;
    eprintln!(
        "built in {:.0} ms: {} placements, {} pads, {} nets, {} vias, {} tracks",
        ms(start),
        board.placements.len(),
        layout::placed_pads(&board).len(),
        board.nets.len(),
        board.vias.len(),
        board.tracks.len()
    );

    // Open: decode and validate the saved file.
    let mut file = Engine::showcase()?.to_file();
    file.project = board;
    let start = Instant::now();
    let encoded = format::encode(&file);
    let saved = ms(start);
    let start = Instant::now();
    let opened = match encoded {
        Ok(bytes) => {
            eprintln!(
                "save (encode): {saved:.0} ms, {:.1} MiB",
                bytes.len() as f64 / f64::from(1 << 20)
            );
            let decoded = format::decode(&bytes)?;
            Engine::from_file(decoded)?
        }
        Err(error) => {
            eprintln!("file not encodable ({error}); opening from memory");
            Engine::from_file(file)?
        }
    };
    eprintln!("open (decode and validate): {:.0} ms", ms(start));
    let mut engine = opened;

    // The copper graph and the full board check.
    let start = Instant::now();
    let graph = engine.copper_graph();
    eprintln!("copper graph: {:.0} ms", ms(start));
    let start = Instant::now();
    let findings = layout::checks_with(engine.project(), &graph, engine.check_settings());
    eprintln!(
        "board checks: {:.0} ms, {} findings ({} clearance)",
        ms(start),
        findings.len(),
        findings
            .iter()
            .filter(|f| f.code == "board.clearance")
            .count()
    );
    let mut codes = std::collections::BTreeMap::<&str, usize>::new();
    for finding in &findings {
        *codes.entry(finding.code.as_str()).or_default() += 1;
    }
    eprintln!("  by code: {codes:?}");
    if let Some(f) = findings.iter().find(|f| f.code == "board.clearance") {
        eprintln!("  e.g. {}", f.message);
    }
    assert!(
        findings.iter().all(|f| f.code != "board.check_limit"),
        "the checks stopped at a limit"
    );
    let start = Instant::now();
    let QueryResult::Issues(whole) = engine.query(Query::Issues {
        scope: IssueScope::All,
    })?
    else {
        return Err("not an issue report".into());
    };
    eprintln!(
        "project.issues (every area): {:.0} ms, {} rows",
        ms(start),
        whole.rows.len()
    );
    // A selection's issues as the UI path gets them: the selection-scoped
    // query (what a cache miss ran on the UI thread before 22 November
    // 2026) against the projection from the one report of every area
    // (`IssueReport::scoped`, what the views do now). Each must equal the
    // other.
    let p = engine.project();
    let selections: Vec<(&str, Vec<ObjectRef>)> = vec![
        (
            "one part",
            vec![ObjectRef::Instance {
                id: p.placements.last().ok_or("part")?.instance,
            }],
        ),
        (
            "one track",
            vec![ObjectRef::Track {
                id: p.tracks.get(p.tracks.len() / 2).ok_or("track")?.id,
            }],
        ),
        (
            "every track (19,462)",
            p.tracks
                .iter()
                .map(|t| ObjectRef::Track { id: t.id })
                .collect(),
        ),
    ];
    for (what, objects) in selections {
        let scope = IssueScope::Selection { objects };
        let mut query = Vec::new();
        let mut projected = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let QueryResult::Issues(asked) = engine.query(Query::Issues {
                scope: scope.clone(),
            })?
            else {
                return Err("not an issue report".into());
            };
            query.push(ms(start));
            let start = Instant::now();
            let scoped = whole.scoped(&scope, engine.project());
            projected.push(ms(start));
            assert_eq!(scoped, asked, "the projection equals the query");
        }
        let (qb, qm) = best_median(&mut query);
        let (pb, pm) = best_median(&mut projected);
        eprintln!(
            "selection issues, {what}: query best {qb:.1} / median {qm:.1} ms; projection best {pb:.3} / median {pm:.3} ms"
        );
    }

    // Typical edits, each a committed operation through the engine.
    let via = engine.project().vias[VIA_GRID * VIA_GRID / 2].clone();
    let track = engine.project().tracks[engine.project().tracks.len() / 2].clone();
    let edits = [
        (
            "via_tent",
            Operation::Layout(LayoutCommand::SetViaTented {
                id: via.id,
                tented: false,
            }),
        ),
        (
            "route (new track)",
            Operation::Layout(LayoutCommand::Route {
                request: RouteRequest {
                    net: track.net,
                    start: track.end,
                    end: [track.end[0] - 0.6, track.end[1]],
                    width: TRACK_WIDTH,
                    layer: track.layer,
                },
            }),
        ),
        (
            "track_set width",
            Operation::Layout(LayoutCommand::SetTrack {
                id: track.id,
                width: Some(0.16),
                layer: None,
            }),
        ),
        (
            "move a part",
            Operation::Layout(LayoutCommand::Move {
                instance: engine.project().placements.last().ok_or("part")?.instance,
                position: [BOARD[0] - 20.0, BOARD[1] - 20.0],
                rotation_deg: 90.0,
                layer: Layer::Top,
            }),
        ),
    ];
    for (what, operation) in edits {
        let at = engine.revision();
        let start = Instant::now();
        let outcome = engine.apply(at, operation);
        let applied = ms(start);
        let start = Instant::now();
        let _ = engine.copper_graph();
        eprintln!(
            "apply {what}: {applied:.0} ms ({}), then copper graph {:.0} ms",
            match &outcome {
                Ok(_) => "applied".to_owned(),
                Err(error) => format!("refused: {error}"),
            },
            ms(start)
        );
    }
    // A typical edit repeated: toggling one via's tenting, best and mean
    // of 20 (the machine is shared).
    let mut times = Vec::new();
    for k in 0..20 {
        let at = engine.revision();
        let start = Instant::now();
        engine.apply(
            at,
            Operation::Layout(LayoutCommand::SetViaTented {
                id: via.id,
                tented: k % 2 == 1,
            }),
        )?;
        times.push(ms(start));
    }
    eprintln!(
        "via_tent x20: best {:.0} ms, mean {:.0} ms",
        times.iter().copied().fold(f64::MAX, f64::min),
        times.iter().sum::<f64>() / times.len() as f64
    );
    let start = Instant::now();
    engine.undo()?;
    eprintln!("undo: {:.0} ms", ms(start));

    // Refill every pour.
    let start = Instant::now();
    let filled = layout::fill::refill(engine.project(), RefillTarget::All)?;
    eprintln!("refill {} pours: {:.0} ms", filled.len(), ms(start));

    // The 3D view: the board plan, the live model, and its update after
    // an edit.
    let start = Instant::now();
    let planned = plan::of(engine.project())?;
    eprintln!(
        "board plan: {:.0} ms, {} bodies",
        ms(start),
        planned.bodies().count()
    );
    let sources = SourceCache::default();
    let start = Instant::now();
    let model = project::build_flat(engine.project(), &sources, None, Flat::Meshed)?;
    eprintln!("3D live model (meshed): {:.0} ms", ms(start));
    let at = engine.revision();
    engine.apply(
        at,
        Operation::Layout(LayoutCommand::SetViaTented {
            id: via.id,
            tented: true,
        }),
    )?;
    let start = Instant::now();
    let (_, rebuilt) = project::update(model, engine.project(), &sources, None)?;
    eprintln!(
        "3D update after a via edit: {:.0} ms ({rebuilt:?})",
        ms(start)
    );
    Ok(())
}

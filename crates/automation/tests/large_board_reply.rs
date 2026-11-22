//! Scale benchmark: a compact automation reply for a small edit on the
//! large board (`crates/solidcraft-integration/tests/large_board_fixture`),
//! with 16 MiB of packed asset bytes in its library so the cost of
//! anything that serializes the whole project shows. It reports; it holds
//! no time bar.
//!
//! `cargo test --profile ci -p sparksmith-automation --test large_board_reply -- --ignored --nocapture`
//! Peak memory: run the test binary under `/usr/bin/time -l`, once as is
//! and once with `REPLY_BENCH_BUILD_ONLY=1` (the board without replies).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../../solidcraft-integration/tests/large_board_fixture/mod.rs"]
mod large_board_fixture;

use large_board_fixture::{Fallible, large_board};
use serde_json::{Value, json};
use sparksmith_automation::gate::Caller;
use sparksmith_automation::session::Session;
use sparksmith_core::library::bundle::{AssetBytes, PackedAsset, asset_hash};
use sparksmith_core::{Engine, Shared};
use std::time::Instant;

const ASSET_BYTES: usize = 16 << 20;

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

fn tent(session: &Session, via: u64, tented: bool) -> Value {
    json!({"command": "project.apply", "params": {
        "expected_revision": session.engine().revision(),
        "operation": {"editor": "Layout", "operation": {"command": "layout.via_tent", "params": {
            "id": via, "tented": tented
        }}},
    }})
}

fn layout(session: &Session, command: &str, params: Value) -> Value {
    json!({"command": "project.apply", "params": {
        "expected_revision": session.engine().revision(),
        "operation": {"editor": "Layout", "operation": {"command": command, "params": params}},
    }})
}

/// Eleven warm replies after a first one, each from `request(k)`; each must
/// name `kind`'s object `id` as changed. Prints best and median.
fn replies(
    session: &mut Session,
    label: &str,
    kind: &str,
    id: u64,
    request: impl Fn(&Session, usize) -> Value,
) {
    let mut times = Vec::new();
    for k in 0..12 {
        let request = request(session, k);
        let start = Instant::now();
        let reply = session.respond(Caller::Operator, request);
        times.push(ms(start));
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(
            reply["result"]["changes"]["changed"][kind],
            json!([id]),
            "{reply}"
        );
    }
    let mut warm = times[1..].to_vec();
    warm.sort_by(f64::total_cmp);
    eprintln!(
        "compact reply, {label}: first {:.0} ms; then best {:.0} ms, median {:.0} ms ({} replies)",
        times[0],
        warm[0],
        warm[warm.len() / 2],
        warm.len()
    );
}

#[test]
#[ignore = "scale benchmark; run with --ignored --nocapture"]
fn compact_reply_for_a_small_edit_on_the_large_board() -> Fallible {
    let load = std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .or_else(|| std::fs::read_to_string("/proc/loadavg").ok());
    eprintln!("load average: {}", load.unwrap_or_default().trim());
    let standards = sparksmith_core::library::standards::Context::fixture()?;
    let _context = standards.enter();
    let mut board = large_board()?;
    // Asset bytes the library carries (as a project with attached STEP
    // models does); deterministic, incompressible enough.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let bytes: Vec<u8> = (0..ASSET_BYTES)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    let mut assets: Vec<PackedAsset> = board.library.assets.iter().cloned().collect();
    assets.push(PackedAsset {
        hash: asset_hash(&bytes),
        media_type: "model/step".into(),
        data: AssetBytes::new(bytes),
    });
    board.library.assets = Shared::new(assets);
    let mut file = Engine::showcase()?.to_file();
    file.project = board;
    let engine = Engine::from_file(file)?;
    eprintln!(
        "board: {} placements, {} vias, {} tracks, {} MiB of packed assets",
        engine.project().placements.len(),
        engine.project().vias.len(),
        engine.project().tracks.len(),
        ASSET_BYTES >> 20
    );
    if std::env::var_os("REPLY_BENCH_BUILD_ONLY").is_some() {
        return Ok(());
    }
    let via = engine.project().vias[engine.project().vias.len() / 2].id;
    let mut session = Session::new(engine);
    let mut times = Vec::new();
    let mut size = 0;
    for k in 0..12 {
        let request = tent(&session, via, k % 2 == 1);
        let start = Instant::now();
        let reply = session.respond(Caller::Operator, request);
        times.push(ms(start));
        assert_eq!(reply["ok"], true, "{reply}");
        let result = &reply["result"];
        assert_eq!(
            result["changes"]["changed"]["vias"],
            json!([via]),
            "{result}"
        );
        size = reply.to_string().len();
    }
    let first = times[0];
    let mut warm = times[1..].to_vec();
    warm.sort_by(f64::total_cmp);
    eprintln!(
        "compact reply, via tent: first {first:.0} ms; then best {:.0} ms, median {:.0} ms ({} replies); reply {size} bytes",
        warm[0],
        warm[warm.len() / 2],
        warm.len()
    );
    // A via moved on the same (largest) net, and a track's width.
    let at = session
        .engine()
        .project()
        .via(via)
        .map(|v| v.position)
        .ok_or("via")?;
    replies(&mut session, "via move", "vias", via, |s, k| {
        let dx = if k % 2 == 0 { 0.01 } else { 0.0 };
        layout(
            s,
            "layout.via_move",
            json!({"id": via, "position": [at[0] + dx, at[1]]}),
        )
    });
    let track =
        session.engine().project().tracks[session.engine().project().tracks.len() / 2].clone();
    replies(&mut session, "track width", "tracks", track.id, |s, k| {
        let width = if k % 2 == 0 {
            track.width * 0.9
        } else {
            track.width
        };
        layout(
            s,
            "layout.track_set",
            json!({"id": track.id, "width": width}),
        )
    });
    // What each reply's findings delta costs: the design checks of the
    // new state (ERC, then the board checks with their copper graph).
    let project = session.engine().project();
    let settings = *session.engine().check_settings();
    let start = Instant::now();
    let erc = sparksmith_core::erc::checks(project).len();
    let erc_ms = ms(start);
    let start = Instant::now();
    let board = sparksmith_core::layout::checks(project, &settings).len();
    eprintln!(
        "  of which the findings: ERC {erc_ms:.0} ms ({erc}), board checks with their copper graph {:.0} ms ({board})",
        ms(start)
    );
    // What a reply serialized and diffed before 22 November 2026, twice
    // (the old and the new state): the whole project as a JSON tree.
    let start = Instant::now();
    let tree = serde_json::to_value(project)?;
    let tree_ms = ms(start);
    eprintln!(
        "  whole-project JSON tree (each reply built two before): {tree_ms:.0} ms, {:.1} MiB as text",
        tree.to_string().len() as f64 / f64::from(1 << 20)
    );
    Ok(())
}

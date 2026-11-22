//! The 3D selection rings with many selected objects: the rings follow the
//! selection and the shown layers, and a benchmark of the per-frame work
//! with thousands of selected tracks.
use super::*;
use sparksmith_core::CopperLayer;

/// The demo board with `count` short top-layer tracks in a grid strip
/// added below it, each its own pick identity.
fn track_project(count: usize) -> Result<sparksmith_core::Project, Box<dyn std::error::Error>> {
    let mut project = crate::model_scene::tests::demo_project()?;
    let net = project
        .nets
        .first()
        .map(|n| n.id)
        .ok_or("demo has no net")?;
    let columns = 64usize;
    let rows = count.div_ceil(columns);
    let [width, depth] = project.board_size;
    project.board_size = [
        width.max(columns as f64 * 1.0 + 6.0),
        depth + rows as f64 * 0.8 + 4.0,
    ];
    for k in 0..count {
        let id = project.allocate()?;
        let x = 3.0 + (k % columns) as f64 * 1.0;
        let y = depth + 2.0 + (k / columns) as f64 * 0.8;
        project.tracks.push(sparksmith_core::Track {
            id,
            net,
            start: [x, y],
            end: [x + 0.6, y],
            width: 0.15,
            layer: CopperLayer::Top,
            locked: false,
        });
    }
    Ok(project)
}

fn track_viewport(count: usize) -> Result<Viewport, Box<dyn std::error::Error>> {
    let project = track_project(count)?;
    // The live view's meshed build, as the 3D worker makes it.
    let model = sparksmith_solidcraft::project::build_flat(
        &project,
        &Default::default(),
        None,
        sparksmith_solidcraft::project::Flat::Meshed,
    )?;
    let mut viewport = Viewport::detached();
    viewport.set_model(crate::model_scene::build(model.geometry, &model.stack)?);
    viewport.fit();
    Ok(viewport)
}

/// The pick identities of the top copper bodies.
fn track_items(viewport: &Viewport) -> Vec<u64> {
    let mut items: Vec<u64> = viewport
        .model
        .as_ref()
        .map(|m| {
            m.geometry
                .iter()
                .filter(|g| ViewLayer::of_body(g) == ViewLayer::Copper(CopperLayer::Top))
                .filter_map(|g| g.item)
                .collect()
        })
        .unwrap_or_default();
    items.sort_unstable();
    items.dedup();
    items
}

/// Every selected track gets its ring; hiding its layer takes the rings
/// away and showing it brings them back; a smaller selection rings only
/// its own. The rings follow selection and layers, never a stale cache.
#[test]
fn rings_follow_the_selection_and_the_layers() -> Result<(), Box<dyn std::error::Error>> {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut viewport = track_viewport(40)?;
    let (w, h) = (800.0, 600.0);
    let items = track_items(&viewport);
    assert!(items.len() >= 40, "{} track bodies", items.len());
    viewport.highlight_selected(items.clone(), items.clone());
    // Every strip track is small in the fitted view (the demo's own long
    // track is not, and gets none).
    let all = viewport.selection_rings(w, h).len();
    assert!(
        all >= 40 && all <= items.len(),
        "{all} rings, {} tracks",
        items.len()
    );
    // Drawn twice, the same rings.
    assert_eq!(viewport.selection_rings(w, h).len(), all);
    // Every layer the selected bodies are on, hidden: no rings.
    let mut layers: Vec<ViewLayer> = viewport
        .model
        .as_ref()
        .map(|m| {
            m.geometry
                .iter()
                .filter(|g| g.item.is_some_and(|i| items.contains(&i)))
                .map(ViewLayer::of_body)
                .collect()
        })
        .unwrap_or_default();
    layers.dedup();
    for layer in &layers {
        viewport.set_layer(*layer, false);
    }
    assert!(viewport.selection_rings(w, h).is_empty(), "hidden layers");
    for layer in &layers {
        viewport.set_layer(*layer, true);
    }
    assert_eq!(viewport.selection_rings(w, h).len(), all);
    // Selected one at a time, each small track rings alone: as many as
    // rang together.
    let mut ringed = Vec::new();
    for item in &items {
        viewport.highlight_selected(vec![*item], vec![*item]);
        if viewport.selection_rings(w, h).len() == 1 {
            ringed.push(*item);
        }
    }
    assert_eq!(ringed.len(), all);
    let few: Vec<u64> = ringed.iter().copied().take(5).collect();
    viewport.highlight_selected(few.clone(), few);
    assert_eq!(viewport.selection_rings(w, h).len(), 5);
    viewport.highlight_selected(vec![], vec![]);
    assert!(viewport.selection_rings(w, h).is_empty());
    Ok(())
}

/// The per-frame selection work of the 3D view with thousands of selected
/// tracks: the rings a redraw asks for while the camera moves (selection
/// and model unchanged). Reports; holds no time bar.
///
/// `cargo test --profile ci -p sparksmith --bin sparksmith selection_rings_scale -- --ignored --nocapture`
#[test]
#[ignore = "scale benchmark; run with --ignored --nocapture"]
fn selection_rings_scale() -> Result<(), Box<dyn std::error::Error>> {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let load = std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .or_else(|| std::fs::read_to_string("/proc/loadavg").ok());
    eprintln!("load average: {}", load.unwrap_or_default().trim());
    let count = 4_000;
    let start = std::time::Instant::now();
    let mut viewport = track_viewport(count)?;
    let items = track_items(&viewport);
    let bodies = viewport.model.as_ref().map_or(0, |m| m.geometry.len());
    eprintln!(
        "built in {:.0} ms: {} selected tracks, {bodies} bodies",
        start.elapsed().as_secs_f64() * 1e3,
        items.len()
    );
    viewport.highlight_selected(items.clone(), items);
    let (w, h) = (1600.0, 1000.0);
    let mut times = Vec::new();
    let mut rings = 0;
    for frame in 0..30 {
        // The camera turns a little each frame, as an orbit does.
        viewport.camera.yaw += 0.01;
        let start = std::time::Instant::now();
        rings = viewport.selection_rings(w, h).len();
        times.push(start.elapsed().as_secs_f64() * 1e3);
        if frame == 0 {
            eprintln!("first frame: {:.2} ms", times[0]);
        }
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "selection rings per frame ({rings} rings, 30 frames): best {:.2} ms, median {:.2} ms, worst {:.2} ms",
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
    Ok(())
}

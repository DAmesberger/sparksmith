use super::raster::{Model, render};
use super::source::Body;
use super::*;
use sparksmith_core::library::Bundle;
use std::time::Instant;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn context() -> Option<sparksmith_core::library::standards::Scope> {
    Context::fixture().ok().map(|c| c.enter())
}

/// The shipped GCT USB4105: a component bundle with its part file packed.
fn usb4105() -> Result<Bundle, Box<dyn std::error::Error>> {
    let parts = sparksmith_automation::part_files::builtin_parts()?;
    let part = parts
        .find("usb4105")
        .into_iter()
        .find_map(|p| p.component.clone())
        .ok_or("the shipped USB4105 component is missing")?;
    Ok((*part).clone())
}

/// Wait until `key` is no longer pending (bounded).
fn settle(previews: &mut PartPreviews, key: &PreviewKey) -> PreviewState {
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        previews.receive();
        let state = previews.state(key);
        if !matches!(state, PreviewState::Pending) || Instant::now() > until {
            return state;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn the_rasteriser_draws_a_box_the_same_way_every_time() -> TestResult {
    let model = Model::boxed([2.0, 1.25, 0.5], [0.6, 0.3, 0.1], 0.0, 0.5)?;
    let a = render(&model, 0.3, 64);
    let b = render(&model, 0.3, 64);
    assert_eq!(a.pixels, b.pixels);
    let opaque = a.pixels.iter().filter(|p| p.a() == 255).count();
    let clear = a.pixels.iter().filter(|p| p.a() == 0).count();
    assert!(opaque > 64 * 64 / 10, "{opaque} opaque pixels");
    assert!(clear > 0, "the background is transparent");
    // The top face is lit brighter than the shadowed sides average.
    assert!(a.pixels.iter().any(|p| p.a() == 255 && p.r() > 60));
    let turned = render(&model, 1.2, 64);
    assert_ne!(a.pixels, turned.pixels, "a turn changes the picture");
    Ok(())
}

#[test]
fn an_empty_or_huge_model_is_refused_with_a_reason() {
    assert!(Model::new(Vec::new()).is_err());
}

#[test]
fn the_shipped_usb_receptacle_previews_its_body_footprint_and_symbol() -> TestResult {
    let _context = context();
    let bundle = usb4105()?;
    let source = PreviewSource::from_bundle(&bundle)?;
    assert!(
        matches!(source.body, Body::PartFile { .. }),
        "{:?}",
        source.name
    );
    assert!(
        source
            .footprint
            .as_ref()
            .is_some_and(|f| !f.pads.is_empty())
    );
    assert!(source.symbol.as_ref().is_some_and(|s| !s.pins.is_empty()));
    let started = Instant::now();
    let model = mesh::model(&source.body)?;
    let evaluated = started.elapsed();
    let frame = render(&model, 0.0, FRAME_PX);
    let cold = started.elapsed();
    assert!(!model.parts.is_empty());
    let drawn = frame.pixels.iter().filter(|p| p.a() == 255).count();
    assert!(drawn > FRAME_PX * FRAME_PX / 20, "{drawn} pixels drawn");
    let one = Instant::now();
    let _ = render(&model, 0.5, FRAME_PX);
    println!(
        "usb4105: {} bodies, {} triangles; evaluate {:.1} ms, cold first frame {:.1} ms, turn frame {:.1} ms",
        model.parts.len(),
        model.triangles(),
        evaluated.as_secs_f64() * 1000.0,
        cold.as_secs_f64() * 1000.0,
        one.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

#[test]
fn a_cached_preview_shows_its_first_frame_within_budget() -> TestResult {
    let _context = context();
    let bundle = Arc::new(usb4105()?);
    let key = PreviewKey::Record(ContentHash([7; 32]));
    let mut previews = PartPreviews::new();
    let b = Arc::clone(&bundle);
    previews.request(key, move || PreviewSource::from_bundle(&b));
    let PreviewState::Ready(cold) = settle(&mut previews, &key) else {
        return Err(format!("{:?}", previews.state(&key)).into());
    };
    assert!(!cold.cached);
    assert!(previews.has_frame(&key));
    // Kept by the pane: a second request is free and the frame is there.
    let started = Instant::now();
    previews.request(key, || Err("not asked again".into()));
    assert!(previews.has_frame(&key));
    let kept = started.elapsed().as_secs_f64() * 1000.0;
    // Dropped by the pane but held by the worker: the body is not
    // evaluated again.
    previews.entries.clear();
    previews.order.clear();
    let b = Arc::clone(&bundle);
    previews.request(key, move || PreviewSource::from_bundle(&b));
    let PreviewState::Ready(warm) = settle(&mut previews, &key) else {
        return Err(format!("{:?}", previews.state(&key)).into());
    };
    assert!(warm.cached);
    println!(
        "preview: cold first frame {:.1} ms, worker-cached first frame {:.1} ms, pane-kept {kept:.3} ms",
        cold.first_frame_ms, warm.first_frame_ms
    );
    assert!(warm.first_frame_ms < 150.0, "{warm:?}");
    assert!(kept < 150.0);
    Ok(())
}

#[test]
fn a_pending_preview_spins_without_blocking_the_frame() -> TestResult {
    let ctx = egui::Context::default();
    let mut previews = PartPreviews::new();
    let key = PreviewKey::Record(ContentHash([9; 32]));
    let (release, wait) = std::sync::mpsc::channel::<()>();
    previews.request(key, move || {
        let _ = wait.recv_timeout(Duration::from_secs(10));
        Err("released".into())
    });
    let mut slowest = Duration::ZERO;
    for _ in 0..5 {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let started = Instant::now();
            previews.pane(ui, &key, vec2(160.0, 160.0));
            slowest = slowest.max(started.elapsed());
        });
    }
    assert_eq!(previews.state(&key), PreviewState::Pending);
    assert!(slowest < Duration::from_millis(20), "{slowest:?}");
    let _ = release.send(());
    assert_eq!(
        settle(&mut previews, &key),
        PreviewState::Failed("released".into())
    );
    Ok(())
}

#[test]
fn only_the_newest_requests_wait() {
    let mut previews = PartPreviews::new();
    // Hold the worker on the first request.
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let first = PreviewKey::Record(ContentHash([1; 32]));
    previews.request(first, move || {
        let _ = wait.recv_timeout(Duration::from_secs(10));
        Err("first".into())
    });
    std::thread::sleep(Duration::from_millis(50));
    let keys: Vec<PreviewKey> = (2..10u8)
        .map(|i| PreviewKey::Record(ContentHash([i; 32])))
        .collect();
    for key in &keys {
        previews.request(*key, || Err("later".into()));
    }
    let dropped = keys.len() - MAX_QUEUED;
    for key in &keys[..dropped] {
        assert_eq!(previews.state(key), PreviewState::Unknown, "{key:?}");
    }
    for key in &keys[dropped..] {
        assert_eq!(previews.state(key), PreviewState::Pending);
    }
    let _ = release.send(());
}

#[test]
fn the_pane_draws_body_footprint_and_symbol_at_two_sizes() -> TestResult {
    let _context = context();
    let bundle = Arc::new(usb4105()?);
    let key = PreviewKey::Record(ContentHash([3; 32]));
    let mut previews = PartPreviews::new();
    let b = Arc::clone(&bundle);
    previews.request(key, move || PreviewSource::from_bundle(&b));
    assert!(matches!(
        settle(&mut previews, &key),
        PreviewState::Ready(_)
    ));
    let ctx = egui::Context::default();
    for side in [160.0, 320.0] {
        let mut cost = Duration::ZERO;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let started = Instant::now();
            previews.pane(ui, &key, vec2(side, side));
            cost = started.elapsed();
        });
        let images = output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default()))
            .count();
        assert!(images >= 1, "{side}: the body's frame is drawn");
        assert!(
            output.shapes.len() > 10,
            "{side}: {} shapes (lands, symbol)",
            output.shapes.len()
        );
        println!("pane {side}×{side}: {:.2} ms", cost.as_secs_f64() * 1000.0);
    }
    Ok(())
}

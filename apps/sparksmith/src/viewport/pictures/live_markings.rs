//! Markings as the window draws them: parts the shipped generators make,
//! placed on the demo board, built by the model worker's own board path
//! ([`crate::scene::live_board`]: the live model and its marking decals)
//! and drawn by the viewport's renderer, with the Markings layer on and
//! off. A chip resistor's value code, an electrolytic's print and a radio
//! module's shield label (an artwork template: its variant from the part
//! number, the maker's mark from the installed artwork) must each change
//! the pixels over their part.
//!
//! The maker's mark and the conformity mark here are original synthetic
//! fixtures standing in for installed library artwork; Sparksmith's
//! repository holds no maker's artwork.
//! `SPARKSMITH_PICTURES=dir` also writes the pictures.
use super::marking_rules::{Case, give_part, place};
use super::*;
use sparksmith_core::library::marking::LaidElement;
use std::collections::BTreeMap;
use std::sync::Arc;

/// An original synthetic maker's mark: a ring and a bar.
const SYNTHETIC_LOGO: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 50 10"><circle cx="5" cy="5" r="4" fill="none" stroke="black" stroke-width="2"/><rect x="12" y="2" width="36" height="6"/></svg>"#;
/// An original synthetic conformity mark: two bars.
const SYNTHETIC_MARK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 10"><rect width="5" height="10"/><rect x="8" width="5" height="10"/></svg>"#;

static CASES: [Case; 3] = [
    Case {
        name: "resistor-0603-10k",
        words: Some("resistor 0603 10k"),
        preset: None,
        tolerance: Some("5%"),
        mpn: None,
        distance: 5.0,
        top: 0.45,
    },
    Case {
        name: "electrolytic-6.3x7.7",
        words: Some("electrolytic 6.3x7.7 100u 16V"),
        preset: None,
        tolerance: None,
        mpn: None,
        distance: 16.0,
        top: 7.7,
    },
    Case {
        name: "module-wroom-1-n16r8",
        words: None,
        preset: Some(("module", "module-esp32-s3-wroom-1")),
        tolerance: None,
        mpn: Some("ESP32-S3-WROOM-1-N16R8"),
        distance: 34.0,
        top: 3.1,
    },
];

/// The pixels whose luminance differs by more than a tenth between `a`
/// and `b`.
fn changed(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, q)| (luminance(p) - luminance(q)).abs() > 0.1)
        .count()
}

struct Board {
    project: Project,
    placed: Vec<(&'static Case, u64, [f64; 2])>,
}

fn board() -> Result<Board, Box<dyn std::error::Error>> {
    // Another shipped library (`SPARKSMITH_PICTURES_LIBRARY`), for pictures
    // of what other generator bundles make.
    let root = std::env::var_os("SPARKSMITH_PICTURES_LIBRARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library"));
    let mut session = sparksmith_automation::Session::new(sparksmith_core::Engine::demo()?);
    session.set_shipped(sparksmith_io::shipped::ShippedLibrary::at(&root)?);
    const PITCH: f64 = 40.0;
    let y = session
        .engine()
        .project()
        .placements
        .iter()
        .map(|p| p.position[1])
        .fold(f64::NEG_INFINITY, f64::max)
        + PITCH;
    let mut placed = Vec::new();
    for (k, case) in CASES.iter().enumerate() {
        let at = [PITCH * (k as f64 + 1.0), y];
        placed.push((case, place(&mut session, case, at)?, at));
    }
    let mut project = session.engine().project().clone();
    project.fabrication = Fabrication::default();
    project.board_size = [
        project.board_size[0].max(PITCH * (CASES.len() as f64 + 1.0)),
        project.board_size[1].max(y + PITCH),
    ];
    for (case, instance, _) in &placed {
        if let Some(mpn) = case.mpn {
            give_part(&mut project, *instance, mpn)?;
        }
    }
    Ok(Board { project, placed })
}

/// The fixture context with `art` installed as library artwork.
fn with_art(art: &[(&str, &str)]) -> Result<Arc<Context>, Box<dyn std::error::Error>> {
    let installed: BTreeMap<String, Arc<str>> = art
        .iter()
        .map(|(id, svg)| ((*id).to_owned(), Arc::from(*svg)))
        .collect();
    Ok(Arc::new(
        (*sparksmith_core::library::standards::Context::fixture()?)
            .clone()
            .with_artwork(installed),
    ))
}

use sparksmith_core::library::standards::Context;

/// The live scene of `project` in a viewport, and its board's top.
fn live(project: &Project) -> Result<(Viewport, f64, usize), Box<dyn std::error::Error>> {
    let (scene, geometry) = crate::scene::live_board(project)?;
    let decals = geometry
        .iter()
        .filter(|g| g.kind == BodyKind::Marking)
        .count();
    let top = geometry
        .iter()
        .filter(|g| g.kind == BodyKind::Substrate)
        .map(|g| g.max[2])
        .reduce(f64::max)
        .ok_or("no board")?;
    let mut v = Viewport::detached();
    v.set_model(scene);
    v.fit_to(4.0 / 3.0);
    Ok((v, top, decals))
}

#[test]
fn markings_draw_in_the_live_view_on_a_resistor_an_electrolytic_and_a_module()
-> Result<(), Box<dyn std::error::Error>> {
    let context = with_art(&[
        ("espressif-logo", SYNTHETIC_LOGO),
        ("ce-mark", SYNTHETIC_MARK),
    ])?;
    let _context = context.enter();
    let Board { project, placed } = board()?;
    // What each marking lays out: the resistor's code, the can's values,
    // and the label's variant bound from the part number.
    let laid = |instance: u64| sparksmith_core::commands::instance_marking(&project, instance);
    let texts = |instance: u64| -> Result<Vec<String>, Box<dyn std::error::Error>> {
        Ok(laid(instance)?
            .iter()
            .flat_map(|l| l.all())
            .flat_map(|l| &l.elements)
            .filter_map(|e| match e {
                LaidElement::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect())
    };
    let [(_, resistor, _), (_, can, _), (_, module, _)] = placed.as_slice() else {
        return Err("three parts".into());
    };
    assert_eq!(texts(*resistor)?, ["103"]);
    assert_eq!(texts(*can)?, ["100", "16V"]);
    let label = laid(*module)?.ok_or("the module has no marking")?;
    let bound = label
        .all()
        .flat_map(|l| &l.elements)
        .find_map(|e| match e {
            LaidElement::Artwork { bound, .. } => Some(bound.clone()),
            _ => None,
        })
        .ok_or("the module's label is no artwork")?;
    assert_eq!(bound.get("code").map(String::as_str), Some("N16R8"));
    assert_eq!(bound.get("?code").map(String::as_str), Some("1"));

    let (mut v, board_top, decals) = live(&project)?;
    assert!(decals >= 3, "{decals} decal meshes in the live scene");
    let Some(mut gpu) = Gpu::new() else {
        if std::env::var_os("SPARKSMITH_REQUIRE_GPU").is_some() {
            return Err("no GPU adapter".into());
        }
        return Ok(());
    };
    let dir = std::env::var_os("SPARKSMITH_PICTURES").map(std::path::PathBuf::from);
    if let Some(dir) = &dir {
        std::fs::create_dir_all(dir)?;
    }
    let (w, h) = (800, 600);
    let mut off = crate::layer_view::LayerView::default();
    off.set(ViewLayer::Markings, false);
    for (case, _, at) in &placed {
        aim(
            &mut v,
            Vec3::new(at[0], at[1], board_top + case.top),
            case.distance,
            0.0,
            89.0,
        );
        let mut shots = Vec::new();
        for (name, layers) in [
            ("with", crate::layer_view::LayerView::default()),
            ("without", off.clone()),
        ] {
            v.set_layers(layers);
            esp32::settled(&mut v);
            let rgba = gpu.draw(&mut v, w, h, 2.0).ok_or("no picture")?;
            if let Some(dir) = &dir {
                let png =
                    sparksmith_solidcraft::viewport::png(w as usize, h as usize, rgba.clone())
                        .ok_or("png encoding failed")?;
                std::fs::write(dir.join(format!("live-{}-{name}.png", case.name)), png)?;
            }
            shots.push(rgba);
        }
        let differ = changed(&shots[0], &shots[1]);
        // Hand-set floors well under what each marking covers at this
        // distance: the code's three digits, the can's two lines, the
        // label's mark, name, variant, FCC line and conformity mark.
        let floor = match case.name {
            n if n.starts_with("resistor") => 400,
            n if n.starts_with("electrolytic") => 400,
            _ => 1500,
        };
        assert!(
            differ >= floor,
            "{}: the markings change {differ} pixels (at least {floor} expected)",
            case.name
        );
    }
    v.set_layers(crate::layer_view::LayerView::default());
    // The installed maker's mark is drawn: without it the label loses the
    // pixels of its mark and nothing else fails.
    let (_, _, at) = placed.get(2).ok_or("no module")?;
    let module_case = &CASES[2];
    let mut label_shot = |v: &mut Viewport| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        aim(
            v,
            Vec3::new(at[0], at[1], board_top + module_case.top),
            module_case.distance,
            0.0,
            89.0,
        );
        esp32::settled(v);
        gpu.draw(v, w, h, 2.0).ok_or_else(|| "no picture".into())
    };
    let with_mark = label_shot(&mut v)?;
    drop(_context);
    let bare = with_art(&[])?;
    let _bare = bare.enter();
    let (mut v, _, _) = live(&project)?;
    let without_mark = label_shot(&mut v)?;
    if let Some(dir) = &dir {
        let png =
            sparksmith_solidcraft::viewport::png(w as usize, h as usize, without_mark.clone())
                .ok_or("png encoding failed")?;
        std::fs::write(
            dir.join("live-module-wroom-1-n16r8-no-installed-art.png"),
            png,
        )?;
    }
    let differ = changed(&with_mark, &without_mark);
    assert!(differ >= 200, "the installed marks change {differ} pixels");
    Ok(())
}

/// The shipped example board as the window draws it: close-ups of its
/// module, a resistor and the regulator, written to `SPARKSMITH_PICTURES`
/// (with the library artwork installed under `SPARKSMITH_LIBRARY_DIR`, if
/// set).
#[test]
#[ignore = "writes pictures"]
fn write_example_live_pictures() -> Result<(), Box<dyn std::error::Error>> {
    // Artwork as a library installs it: `SPARKSMITH_ARTWORK=dir` of
    // `<id>.svg` files.
    let mut art = BTreeMap::new();
    if let Some(dir) = std::env::var_os("SPARKSMITH_ARTWORK") {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if let (Some(id), Some("svg")) = (
                path.file_stem().and_then(|s| s.to_str()),
                path.extension().and_then(|e| e.to_str()),
            ) {
                art.insert(
                    id.to_owned(),
                    Arc::from(sparksmith_io::artwork::read_svg(&path)?),
                );
            }
        }
    }
    let context = Arc::new(
        (*sparksmith_core::library::standards::Context::fixture()?)
            .clone()
            .with_artwork(art),
    );
    let _context = context.enter();
    let Some(dir) = std::env::var_os("SPARKSMITH_PICTURES").map(std::path::PathBuf::from) else {
        return Err("set SPARKSMITH_PICTURES".into());
    };
    std::fs::create_dir_all(&dir)?;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut engine = sparksmith_core::Engine::from_file(sparksmith_io::read_project(&path)?)?;
    engine.refill_stale()?;
    let project = engine.project().clone();
    let (mut v, top, decals) = live(&project)?;
    eprintln!("{decals} decal meshes");
    let Some(mut gpu) = Gpu::new() else {
        return Err("no GPU adapter".into());
    };
    let (w, h) = (1000, 750);
    for (designator, height, distance) in [("U1", 3.5, 34.0), ("R1", 0.45, 5.0), ("U2", 1.1, 8.0)] {
        let (at, _) = part_at(&project, designator).ok_or("no such part")?;
        for (view, pitch) in [("top", 89.0), ("angle", 50.0)] {
            aim(
                &mut v,
                at + Vec3::new(0.0, 0.0, top + height),
                distance,
                20.0,
                pitch,
            );
            esp32::settled(&mut v);
            let rgba = gpu.draw(&mut v, w, h, 2.0).ok_or("no picture")?;
            let png = sparksmith_solidcraft::viewport::png(w as usize, h as usize, rgba)
                .ok_or("png encoding failed")?;
            std::fs::write(dir.join(format!("example-{designator}-{view}.png")), png)?;
        }
    }
    // The generated parts of the live-view test, with the artwork given.
    let Board { project, placed } = board()?;
    let (mut v, top, _) = live(&project)?;
    for (case, _, at) in &placed {
        for (view, pitch) in [("top", 89.0), ("angle", 45.0)] {
            aim(
                &mut v,
                Vec3::new(at[0], at[1], top + case.top),
                case.distance * 1.15,
                0.0,
                pitch,
            );
            esp32::settled(&mut v);
            let rgba = gpu.draw(&mut v, w, h, 2.0).ok_or("no picture")?;
            let png = sparksmith_solidcraft::viewport::png(w as usize, h as usize, rgba)
                .ok_or("png encoding failed")?;
            std::fs::write(dir.join(format!("generated-{}-{view}.png", case.name)), png)?;
        }
    }
    Ok(())
}

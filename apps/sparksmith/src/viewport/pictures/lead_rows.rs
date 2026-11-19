//! Pictures of a four-sided gull-wing (one lead per pin, each on its land)
//! and of the land grid arrays (lands proud of the body), each alone on the
//! demo board in place of its first part:
//! `SPARKSMITH_PICTURES=dir cargo test -p sparksmith write_lead_row_pictures -- --ignored`.
use super::*;

#[test]
#[ignore = "writes lead-row and land-grid pictures"]
fn write_lead_row_pictures() -> Result<(), Box<dyn std::error::Error>> {
    use sparksmith_core::library::PackageModel;
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let Some(dir) = std::env::var_os("SPARKSMITH_PICTURES") else {
        return Err("set SPARKSMITH_PICTURES".into());
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    let Some(mut gpu) = Gpu::new() else {
        return Err("no GPU adapter".into());
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    // (name, generator, preset, viewing distance mm)
    let parts: [(&str, &str, &str, f64); 3] = [
        ("lqfp-32", "gull-wing", "gull-wing-lqfp-32-7x7-0.8", 16.0),
        ("lga-64", "grid", "grid-lga-64-0.65-8x8-6x6", 14.0),
        (
            "lga-14",
            "no-lead",
            "no-lead-lga-14-3x5-0.8-adi-cc-14-1",
            10.0,
        ),
    ];
    for (name, generator, preset, distance) in parts {
        let mut session = sparksmith_automation::Session::new(sparksmith_core::Engine::demo()?);
        session.set_shipped(sparksmith_io::shipped::ShippedLibrary::at(&root)?);
        let revision = session.engine().revision();
        let made = session.dispatch(serde_json::json!({
            "command": "library.generate_part",
            "params": {"generator": generator, "scope": "package", "preset": preset,
                       "params": {}, "expected_revision": revision},
        }));
        if made["ok"] != true {
            return Err(format!("{name}: not made: {made}").into());
        }
        let mut project = session.engine().project().clone();
        project.fabrication = Fabrication::default();
        let generated = project
            .library
            .packages
            .last()
            .map(|p| p.definition.clone())
            .ok_or("no package")?;
        if !matches!(generated.model, PackageModel::PartFile(_)) {
            return Err(format!("{name}: no part file").into());
        }
        let target = &mut project
            .library
            .packages
            .first_mut()
            .ok_or("no demo package")?
            .definition;
        target.envelope_nm = generated.envelope_nm;
        target.model = generated.model.clone();
        // Alone: the demo's second resistor, 8.25 mm away, would stand
        // in the leads of a part this large.
        project.placements.truncate(1);
        let mut v = viewport(&project)?;
        let at = centre(&v, BodyKind::ModelBody).ok_or("no body")?;
        let (w, h) = (1280, 960);
        // Overall, from the other side, and low along a lead row.
        for (view, yaw, pitch, near) in [
            ("a", 35.0, 35.0, 1.0),
            ("b", 200.0, 25.0, 1.0),
            ("c", 80.0, 12.0, 0.5),
        ] {
            aim(
                &mut v,
                Vec3::new(at.x, at.y, at.z / 2.0),
                distance * near,
                yaw,
                pitch,
            );
            let rgba = gpu.draw(&mut v, w, h, 2.0).ok_or("no picture")?;
            let png = sparksmith_solidcraft::viewport::png(w as usize, h as usize, rgba)
                .ok_or("png encoding failed")?;
            std::fs::write(dir.join(format!("{name}-{view}.png")), png)?;
        }
    }
    Ok(())
}

//! GP2 through the session: `library.generate_part` makes a part whose
//! package uses the part file the module names, from the generator's
//! bundle, at the module's values; saved, reopened with the generator
//! folder gone, the project is identical and its packed part file builds
//! the same 3D model through Solidcraft.
use super::super::Session;
use serde_json::{Value, json};
use sparksmith_core::Engine;
use sparksmith_core::library::PackageModel;
use sparksmith_io::shipped::{MARKER, ShippedLibrary};
use sparksmith_solidcraft::models::part_file;
use std::path::{Path, PathBuf};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The repository's chip generator bundle (`cargo xtask generators build`
/// keeps it equal to the crate's module and part files).
const BUNDLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library/generators/chip");

/// A shipped library in `dir` holding only the chip generator's bundle.
fn library(dir: &Path) -> TestResult<PathBuf> {
    let root = dir.join("library");
    std::fs::create_dir_all(root.join("generators"))?;
    std::fs::write(root.join(MARKER), "{\"format\":1}\n")?;
    copy_dir(Path::new(BUNDLE), &root.join("generators/chip"))?;
    Ok(root)
}

fn copy_dir(from: &Path, to: &Path) -> TestResult {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        let target = to.join(path.file_name().ok_or("no file name")?);
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn session(root: &Path) -> TestResult<Session> {
    let mut session = Session::fixture(Engine::default());
    session.set_shipped(ShippedLibrary::at(root)?);
    Ok(session)
}

fn call(session: &mut Session, command: &str, mut params: Value) -> Value {
    if let Some(map) = params.as_object_mut()
        && command != "project.save"
    {
        map.insert(
            "expected_revision".into(),
            json!(session.engine().revision()),
        );
    }
    session.dispatch(json!({"command": command, "params": params}))
}

/// The package's 3D model as built: its part file (the project's own
/// packed copy) evaluated at the package's values, each body's name and
/// material with the bounds, and the stored derived data equal to it.
fn model(session: &Session) -> TestResult<Vec<String>> {
    let library = &session.engine().project().library;
    let package = library.packages.first().ok_or("no package")?;
    let PackageModel::PartFile(part) = &package.definition.model else {
        return Err("the package has no part file".into());
    };
    let model = library.model(part.model).ok_or("no model")?;
    let bytes = library
        .assets
        .iter()
        .find(|a| a.hash == model.definition.asset.hash)
        .ok_or("the part file is not packed")?;
    let evaluated = part_file::evaluate(bytes.data.as_slice(), &part.values)?;
    assert!(part_file::matches(
        &part.derived,
        &part_file::derived(&evaluated)?
    ));
    Ok(evaluated
        .bodies
        .iter()
        .map(|b| {
            let [lo, hi] = b.bounds_mm;
            format!("{} {:?} {:.3}..{:.3}", b.name, b.material, lo[0], hi[0])
        })
        .collect())
}

#[test]
fn a_generated_part_keeps_its_model_offline() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    let mut made = session(&root)?;
    let reply = call(
        &mut made,
        "library.generate_part",
        json!({"generator": "chip", "scope": "component", "preset": "capacitor-0805", "params": {}}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let built = model(&made)?;
    assert_eq!(
        built,
        [
            "body Some(BariumTitanate) -0.500..0.500",
            "termination 1 Some(TinMatte) -1.000..-0.500",
            "termination 2 Some(TinMatte) 0.500..1.000",
        ]
    );
    let path = temp.path().join("model.sparksmith");
    let saved = call(
        &mut made,
        "project.save",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(saved["ok"], true, "{saved}");

    std::fs::remove_dir_all(root.join("generators"))?;
    let mut reopened = session(&root)?;
    let opened = call(
        &mut reopened,
        "project.open",
        json!({"path": path.display().to_string()}),
    );
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(reopened.engine().to_file(), made.engine().to_file());
    assert_eq!(model(&reopened)?, built);
    Ok(())
}

/// Every preset of the through-hole generator (phase C) is made through
/// the session: its part file, evaluated at the module's values, passes
/// the host's checks (no blocking finding, bounds on the envelope, every
/// lead contact under a lead body) and binds a lead body per terminal.
#[test]
fn every_through_hole_preset_is_made_with_its_model() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("library");
    std::fs::create_dir_all(root.join("generators"))?;
    std::fs::write(root.join(MARKER), "{\"format\":1}\n")?;
    copy_dir(
        &Path::new(BUNDLE).with_file_name("tht"),
        &root.join("generators/tht"),
    )?;
    let presets = serde_json::from_str::<Vec<Value>>(include_str!(
        "../../../../generators/tht/presets.json"
    ))?;
    assert_eq!(presets.len(), 22);
    for preset in &presets {
        let key = preset["key"].as_str().ok_or("no key")?;
        let mut session = session(&root)?;
        let reply = call(
            &mut session,
            "library.generate_part",
            json!({"generator": "tht", "scope": "component", "preset": key, "params": {}}),
        );
        assert_eq!(reply["ok"], true, "{key}: {reply}");
        let library = &session.engine().project().library;
        let package = &library.packages.first().ok_or("no package")?.definition;
        let PackageModel::PartFile(part) = &package.model else {
            return Err(format!("{key}: no part file").into());
        };
        assert_eq!(
            part.derived.leads.len(),
            package.terminals.len(),
            "{key}: a lead body per terminal"
        );
    }
    Ok(())
}

/// Every tabbed preset makes a part through the session from the shipped
/// bundle: its part file at the module's values passes the checks (the
/// envelope, a DPAK's flange side included, and every lead contact under a
/// lead body bound to its terminal).
#[test]
fn every_tabbed_preset_makes_a_part_with_its_model() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    copy_dir(
        &Path::new(BUNDLE).with_file_name("tabbed"),
        &root.join("generators/tabbed"),
    )?;
    for preset in [
        "tabbed-sot-23-3",
        "tabbed-sot-23-5",
        "tabbed-sot-23-6",
        "tabbed-sc-70-3",
        "tabbed-sc-70-5",
        "tabbed-sc-70-6",
        "tabbed-sot-223",
        "tabbed-sot-89",
        "tabbed-dpak",
        "tabbed-d2pak",
    ] {
        let mut made = session(&root)?;
        let reply = call(
            &mut made,
            "library.generate_part",
            json!({"generator": "tabbed", "scope": "component", "preset": preset, "params": {}}),
        );
        assert_eq!(reply["ok"], true, "{preset}: {reply}");
        let library = &made.engine().project().library;
        let package = library.packages.first().ok_or("no package")?;
        let PackageModel::PartFile(part) = &package.definition.model else {
            return Err(format!("{preset}: no part file").into());
        };
        assert_eq!(
            part.derived.leads.len(),
            package.definition.terminals.len(),
            "{preset}: every lead bound"
        );
    }
    Ok(())
}

/// The electromechanical families and the module's ESP32-S3-WROOM-1 are
/// made through the session from their shipped bundles at every preset
/// (and each part-file variant): each part file at the module's values
/// passes the host's checks (no blocking finding, bounds on the envelope
/// within 10 µm, every lead contact under a lead body) and binds the
/// expected lead bodies; the bounds are the datasheet's outline.
#[test]
fn every_electromechanical_preset_makes_a_part_with_its_model() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    // (generator, preset, overrides, lead bodies, outline x y z in mm); the
    // WROOM-1 binds 40 pads, their 40 castellation platings and nine
    // ground pads.
    type Case = (&'static str, &'static str, Value, usize, [f64; 3]);
    let cases: [Case; 18] = [
        ("tactile", "tactile-evqp2-ground", json!({}), 4, [5.5, 4.1, 2.1]),
        ("tactile", "tactile-evqp2", json!({}), 4, [5.5, 3.5, 2.1]),
        ("tactile", "tactile-tl3305a", json!({}), 4, [7.5, 4.5, 3.8]),
        ("tactile", "tactile-b3s-1000", json!({}), 4, [9.0, 6.6, 4.3]),
        ("reed", "reed-mdsm-4", json!({}), 2, [19.3, 2.28, 2.84]),
        ("buzzer", "buzzer-cmi-9653s-smt", json!({}), 2, [10.1, 9.6, 5.0]),
        ("buzzer", "buzzer-cmi-9653s-smt", json!({"label": true}), 2, [10.1, 9.6, 5.0]),
        ("electrolytic", "electrolytic-6.3x5.4", json!({}), 2, [7.8, 6.6, 5.5]),
        ("electrolytic", "electrolytic-panasonic-d", json!({"vent": false}), 2, [7.8, 6.6, 5.8]),
        ("electrolytic", "electrolytic-kemet-edk-6.3x7.7", json!({}), 2, [7.8, 6.6, 7.7]),
        ("electrolytic", "electrolytic-panasonic-g", json!({}), 2, [12.0, 10.3, 10.2]),
        ("power-inductor", "power-inductor-xel4030", json!({}), 2, [4.0, 4.0, 3.1]),
        ("power-inductor", "power-inductor-srr6028", json!({}), 2, [6.8, 6.8, 2.8]),
        ("power-inductor", "power-inductor-6x6-shielded", json!({}), 2, [6.0, 6.0, 4.5]),
        ("standoff", "standoff-wurth-9774100360r", json!({}), 1, [6.0, 6.0, 10.0]),
        ("standoff", "standoff-wurth-9774060360r", json!({"spigot_length": 0}), 1, [6.0, 6.0, 6.0]),
        ("module", "module-esp32-s3-wroom-1", json!({}), 89, [18.0, 25.5, 3.1]),
        ("module", "module-esp32-s3-wroom-1", json!({"can_emboss": false}), 89, [18.0, 25.5, 3.1]),
    ];
    for name in ["tactile", "reed", "buzzer", "electrolytic", "power-inductor", "standoff", "module"] {
        copy_dir(
            &Path::new(BUNDLE).with_file_name(name),
            &root.join("generators").join(name),
        )?;
    }
    for (generator, preset, params, leads, outline) in cases {
        let mut made = session(&root)?;
        let reply = call(
            &mut made,
            "library.generate_part",
            json!({"generator": generator, "scope": "component", "preset": preset,
                "params": params}),
        );
        assert_eq!(reply["ok"], true, "{preset} {params}: {reply}");
        let library = &made.engine().project().library;
        let package = library.packages.first().ok_or("no package")?;
        let PackageModel::PartFile(part) = &package.definition.model else {
            return Err(format!("{preset}: no part file").into());
        };
        assert_eq!(part.derived.leads.len(), leads, "{preset} {params}: lead bodies");
        let [lo, hi] = part.derived.bounds_nm;
        for axis in 0..2 {
            let size = (hi[axis].to_mm_f64() - lo[axis].to_mm_f64()).max(
                2.0 * hi[axis].to_mm_f64().abs().max(lo[axis].to_mm_f64().abs()),
            );
            assert!(
                (size - outline[axis]).abs() < 0.011,
                "{preset} {params}: axis {axis} spans {size} mm, the drawing {}",
                outline[axis]
            );
        }
        assert!(
            (hi[2].to_mm_f64() - outline[2]).abs() < 0.011,
            "{preset} {params}: top at {} mm, the drawing {}",
            hi[2].to_mm_f64(),
            outline[2]
        );
    }
    Ok(())
}

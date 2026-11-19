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
        (
            "tactile",
            "tactile-evqp2-ground",
            json!({}),
            4,
            [5.5, 4.1, 2.1],
        ),
        ("tactile", "tactile-evqp2", json!({}), 4, [5.5, 3.5, 2.1]),
        ("tactile", "tactile-tl3305a", json!({}), 4, [7.5, 4.5, 3.8]),
        ("tactile", "tactile-b3s-1000", json!({}), 4, [9.0, 6.6, 4.3]),
        ("reed", "reed-mdsm-4", json!({}), 2, [19.3, 2.28, 2.84]),
        (
            "buzzer",
            "buzzer-cmi-9653s-smt",
            json!({}),
            2,
            [10.1, 9.6, 5.0],
        ),
        (
            "buzzer",
            "buzzer-cmi-9653s-smt",
            json!({"label": true}),
            2,
            [10.1, 9.6, 5.0],
        ),
        (
            "electrolytic",
            "electrolytic-6.3x5.4",
            json!({}),
            2,
            [7.8, 6.6, 5.5],
        ),
        (
            "electrolytic",
            "electrolytic-panasonic-d",
            json!({"vent": false}),
            2,
            [7.8, 6.6, 5.8],
        ),
        (
            "electrolytic",
            "electrolytic-kemet-edk-6.3x7.7",
            json!({}),
            2,
            [7.8, 6.6, 7.7],
        ),
        (
            "electrolytic",
            "electrolytic-panasonic-g",
            json!({}),
            2,
            [12.0, 10.3, 10.2],
        ),
        (
            "power-inductor",
            "power-inductor-xel4030",
            json!({}),
            2,
            [4.0, 4.0, 3.1],
        ),
        (
            "power-inductor",
            "power-inductor-srr6028",
            json!({}),
            2,
            [6.8, 6.8, 2.8],
        ),
        (
            "power-inductor",
            "power-inductor-6x6-shielded",
            json!({}),
            2,
            [6.0, 6.0, 4.5],
        ),
        (
            "standoff",
            "standoff-wurth-9774100360r",
            json!({}),
            1,
            [6.0, 6.0, 10.0],
        ),
        (
            "standoff",
            "standoff-wurth-9774060360r",
            json!({"spigot_length": 0}),
            1,
            [6.0, 6.0, 6.0],
        ),
        (
            "module",
            "module-esp32-s3-wroom-1",
            json!({}),
            89,
            [18.0, 25.5, 3.1],
        ),
        (
            "module",
            "module-esp32-s3-wroom-1",
            json!({"can_emboss": false}),
            89,
            [18.0, 25.5, 3.1],
        ),
    ];
    for name in [
        "tactile",
        "reed",
        "buzzer",
        "electrolytic",
        "power-inductor",
        "standoff",
        "module",
    ] {
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
        assert_eq!(
            part.derived.leads.len(),
            leads,
            "{preset} {params}: lead bodies"
        );
        let [lo, hi] = part.derived.bounds_nm;
        for axis in 0..2 {
            let size = (hi[axis].to_mm_f64() - lo[axis].to_mm_f64())
                .max(2.0 * hi[axis].to_mm_f64().abs().max(lo[axis].to_mm_f64().abs()));
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

/// The shipped library of the repository (every generator's bundle).
const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library");

/// One preset (with parameter overrides) made as a complete part from the
/// shipped bundles: the session holding it.
fn made(generator: &str, preset: &str, params: &Value) -> TestResult<Session> {
    let mut session = session(Path::new(SHIPPED))?;
    let reply = call(
        &mut session,
        "library.generate_part",
        json!({"generator": generator, "scope": "component", "preset": preset, "params": params}),
    );
    if reply["ok"] != true {
        return Err(format!("{generator} {preset} {params}: {reply}").into());
    }
    Ok(session)
}

/// The made part's package with the module's lead contacts (a generated
/// package stores none until a body is fitted to a footprint; these are
/// the contacts `library.make_body` stores), and its footprint.
fn landed(
    session: &Session,
    generator: &str,
    preset: &str,
    params: &Value,
) -> TestResult<(
    sparksmith_core::library::PackageDefinition,
    sparksmith_core::library::FootprintDefinition,
)> {
    use sparksmith_core::library::PackageLead;
    use sparksmith_core::units::{Nm, PointNm};
    let request = serde_json::from_value(json!({"generator": generator,
        "scope": "component", "preset": preset, "params": params,
        "expected_revision": session.engine().revision()}))?;
    let (_, _, made) = session.run_request(request)?;
    let generation = made.map_err(|f| format!("{preset}: {f:?}"))?.generation;
    let library = &session.engine().project().library;
    let mut package = library
        .packages
        .first()
        .ok_or("no package")?
        .definition
        .clone();
    for lead in &generation.package.geometry.leads {
        let terminal = package
            .terminals
            .iter()
            .find(|t| t.designation == lead.key)
            .ok_or_else(|| format!("{preset}: no terminal {}", lead.key))?
            .id;
        package.leads.push(PackageLead {
            terminal,
            contact: PointNm::new(Nm::new(lead.contact[0])?, Nm::new(lead.contact[1])?),
        });
    }
    let footprint = library
        .footprints
        .first()
        .ok_or("no footprint")?
        .definition
        .clone();
    Ok((package, footprint))
}

/// Every preset of the IC and discrete generators, and each part-file
/// variant, placed on either side of the board at every quarter turn: each
/// lead contact of the body, placed as the 3D view places it, lands on its
/// pad as the copper is drawn (the `board.body_off_pads` rule), pin 1's
/// lead lands pad 1 and the leads seat on the board.
#[test]
fn every_ic_preset_lands_its_leads_at_every_quarter_turn() -> TestResult {
    use sparksmith_core::layout::bodies::{PinOne, placed_body};
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let variants: [(&str, &str, Value); 4] = [
        (
            "no-lead",
            "no-lead-qfn-16-4x4-0.65-ti-rgv",
            json!({"wettable_flank": true}),
        ),
        (
            "no-lead",
            "no-lead-dfn-8-2x3-0.5",
            json!({"wettable_flank": true}),
        ),
        (
            "gull-wing",
            "gull-wing-tssop-14-4.4",
            json!({"pin1_bevel": true}),
        ),
        (
            "tabbed",
            "tabbed-sot-23-3",
            json!({"marking_pin1_dot": true}),
        ),
    ];
    let mut cases: Vec<(String, String, Value)> = variants
        .iter()
        .map(|(g, p, v)| ((*g).to_owned(), (*p).to_owned(), v.clone()))
        .collect();
    for generator in ["gull-wing", "tabbed", "no-lead", "chip", "molded", "grid"] {
        let text = std::fs::read_to_string(
            Path::new(SHIPPED)
                .join("../generators")
                .join(generator)
                .join("presets.json"),
        )?;
        for preset in serde_json::from_str::<Vec<Value>>(&text)? {
            let key = preset["key"].as_str().ok_or("no key")?;
            cases.push((generator.to_owned(), key.to_owned(), json!({})));
        }
    }
    for (generator, preset, params) in &cases {
        let session = made(generator, preset, params)?;
        let project = session.engine().project();
        let (package, footprint) = landed(&session, generator, preset, params)?;
        let (package, footprint) = (&package, &footprint);
        for layer in ["Top", "Bottom"] {
            for rotation in [0.0, 90.0, 180.0, 270.0] {
                let placement = serde_json::from_value(json!({"instance": 1,
                    "position": [20.0, 15.0], "rotation_deg": rotation, "layer": layer,
                    "locked": false}))?;
                let body = placed_body(project, &placement, preset, package, footprint, None)
                    .ok_or_else(|| format!("{preset}: no lead contacts"))?;
                assert_eq!(
                    body.off_pads, 0,
                    "{preset} {params} {layer} {rotation}°: {:?}",
                    body.leads
                );
                // A grid array has no terminal 1 (its first ball is A1).
                assert!(
                    matches!(body.pin_one, PinOne::Agrees)
                        || (generator == "grid" && body.pin_one == PinOne::Unknown),
                    "{preset} {params} {layer} {rotation}°: {:?}",
                    body.pin_one
                );
                assert!(
                    body.seat_mm.is_some_and(|z| z.abs() < 1e-6),
                    "{preset} {layer} {rotation}°: seats at {:?}",
                    body.seat_mm
                );
            }
        }
    }
    Ok(())
}

/// The presets made after a manufacturer's drawing match it: the model's
/// outline (its part file's bounds at the module's values) and land 1's
/// place are the drawing's.
#[test]
fn real_part_presets_match_their_drawings() -> TestResult {
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    // (preset, outline x y z mm, land 1 centre x y mm)
    type Case = (&'static str, [f64; 3], [f64; 2]);
    let cases: [Case; 3] = [
        // ADI CC-14-1: 5 × 3 × 0.95; the long sides' lands 1.01 mm off
        // the long axis, the end ones 0.49 mm from the ends at 0.8 mm.
        (
            "no-lead-lga-14-3x5-0.8-adi-cc-14-1",
            [5.0, 3.0, 0.95],
            [-2.0, -1.0100],
        ),
        // Sensirion SHT4x: 1.5 × 1.5 × 0.54, terminals 0.3 mm at 0.8 mm.
        (
            "no-lead-dfn-4-1.5x1.5-0.8-sensirion-sht4x",
            [1.5, 1.5, 0.54],
            [-0.4, -0.6],
        ),
        // TI RGV0016A: 4 × 4 × 0.9, terminals 0.55 mm long at 0.65 mm.
        (
            "no-lead-qfn-16-4x4-0.65-ti-rgv",
            [4.0, 4.0, 0.9],
            [-0.975, -1.725],
        ),
    ];
    for (preset, outline, land1) in cases {
        let session = made("no-lead", preset, &json!({}))?;
        let (package, _) = landed(&session, "no-lead", preset, &json!({}))?;
        let PackageModel::PartFile(part) = &package.model else {
            return Err(format!("{preset}: no part file").into());
        };
        let [lo, hi] = part.derived.bounds_nm;
        for axis in 0..3 {
            let from = if axis == 2 { 0.0 } else { lo[axis].to_mm_f64() };
            let size = hi[axis].to_mm_f64() - from;
            assert!(
                (size - outline[axis]).abs() < 0.011,
                "{preset}: axis {axis} spans {size} mm, the drawing {}",
                outline[axis]
            );
        }
        let lead = package
            .leads
            .iter()
            .map(|l| l.contact.to_mm_f64())
            .next()
            .ok_or("no lead")?;
        let at = lead;
        assert!(
            (at[0] - land1[0]).abs() < 0.002 && (at[1] - land1[1]).abs() < 0.002,
            "{preset}: land 1 at {at:?}, the drawing {land1:?}"
        );
    }
    Ok(())
}

/// A generated part carries its preset's default top marking, as data: a
/// resistor's value code printed on its overcoat, a SOT-23-6's code
/// laser-etched with a pin-1 dot; an MLCC none.
#[test]
fn generated_parts_carry_their_presets_marking() -> TestResult {
    use sparksmith_core::library::marking::{Finish, MarkingItem, MarkingSpec};
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let marking = |preset: (&str, &str)| -> TestResult<Option<Box<MarkingSpec>>> {
        let session = made(preset.0, preset.1, &json!({}))?;
        let library = &session.engine().project().library;
        let component = library.components.first().ok_or("no component")?;
        Ok(component.definition.marking.clone())
    };
    let resistor = marking(("chip", "resistor-0603"))?.ok_or("no resistor marking")?;
    assert_eq!(resistor.finish, Finish::InkWhite);
    assert!(
        matches!(&resistor.items[..], [MarkingItem::Text { template, .. }]
            if template.0.starts_with("{field:D") && template.0.ends_with("|code3}")),
        "{resistor:?}"
    );
    let sot = marking(("tabbed", "tabbed-sot-23-6"))?.ok_or("no SOT-23-6 marking")?;
    assert_eq!(sot.finish, Finish::LaserEtch);
    assert!(
        matches!(
            &sot.items[..],
            [MarkingItem::Text { .. }, MarkingItem::Polarity { .. }]
        ),
        "{sot:?}"
    );
    assert!(marking(("chip", "capacitor-0603"))?.is_none());
    Ok(())
}

/// Every gull-wing and grid preset's part file, at the module's values,
/// draws exactly one lead body per land, each over its own land: the
/// land's centre lies inside the lead's plan and the lead is centred on it
/// across its row (a four-sided file once drew one side's leads twice,
/// 0.25 mm apart along the lead). A land grid's lands stand proud of the
/// body, its underside clear of the seating plane (flush lands drew both
/// faces on one plane).
#[test]
fn every_gull_wing_and_grid_preset_draws_one_lead_per_land() -> TestResult {
    use sparksmith_core::library::BodyRole;
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let mut cases = vec![(
        "gull-wing".to_owned(),
        "gull-wing-tssop-14-4.4".to_owned(),
        json!({"pin1_bevel": true}),
    )];
    for generator in ["gull-wing", "grid"] {
        let text = std::fs::read_to_string(
            Path::new(SHIPPED)
                .join("../generators")
                .join(generator)
                .join("presets.json"),
        )?;
        for preset in serde_json::from_str::<Vec<Value>>(&text)? {
            let key = preset["key"].as_str().ok_or("no key")?;
            cases.push((generator.to_owned(), key.to_owned(), json!({})));
        }
    }
    for (generator, preset, params) in &cases {
        let session = made(generator, preset, params)?;
        let request = serde_json::from_value(json!({"generator": generator,
            "scope": "component", "preset": preset, "params": params,
            "expected_revision": session.engine().revision()}))?;
        let (_, _, generated) = session.run_request(request)?;
        let generation = generated
            .map_err(|f| format!("{preset}: {f:?}"))?
            .generation;
        let library = &session.engine().project().library;
        let package = library.packages.first().ok_or("no package")?;
        let PackageModel::PartFile(part) = &package.definition.model else {
            return Err(format!("{preset}: no part file").into());
        };
        let model = library.model(part.model).ok_or("no model")?;
        let bytes = library
            .assets
            .iter()
            .find(|a| a.hash == model.definition.asset.hash)
            .ok_or("the part file is not packed")?;
        let evaluated = part_file::evaluate(bytes.data.as_slice(), &part.values)?;
        let leads: Vec<&part_file::PartBody> = evaluated
            .bodies
            .iter()
            .filter(|b| part_file::role_of(b.material) == BodyRole::Termination)
            .collect();
        let pads = &generation.footprint.pads;
        assert_eq!(
            leads.len(),
            pads.len(),
            "{preset} {params}: {} lead bodies for {} lands: {:?}",
            leads.len(),
            pads.len(),
            leads
                .iter()
                .map(|b| (&b.name, b.bounds_mm))
                .collect::<Vec<_>>()
        );
        for pad in pads {
            let at = pad.at.map(|v| v as f64 / 1e6);
            let over: Vec<&&part_file::PartBody> = leads
                .iter()
                .filter(|b| {
                    let [lo, hi] = b.bounds_mm;
                    (0..2).all(|k| at[k] > lo[k] && at[k] < hi[k])
                })
                .collect();
            let [lead] = over[..] else {
                return Err(format!(
                    "{preset} {params}: land {} at {at:?} under {} lead bodies",
                    pad.key,
                    over.len()
                )
                .into());
            };
            // Across the row: the lead's narrower extent in plan.
            let [lo, hi] = lead.bounds_mm;
            let across = usize::from(hi[0] - lo[0] >= hi[1] - lo[1]);
            let centre = (lo[across] + hi[across]) / 2.0;
            assert!(
                (centre - at[across]).abs() < 1e-3,
                "{preset} {params}: land {} at {at:?}, lead {} centred at {centre}",
                pad.key,
                lead.name
            );
        }
        if generator == "grid" && preset.starts_with("grid-lga") {
            let body = evaluated
                .bodies
                .iter()
                .find(|b| b.name == "body")
                .ok_or("no body")?;
            assert!(
                body.bounds_mm[0][2] > 1e-3,
                "{preset}: the body's underside at {} mm, on the lands' plane",
                body.bounds_mm[0][2]
            );
        }
    }
    Ok(())
}

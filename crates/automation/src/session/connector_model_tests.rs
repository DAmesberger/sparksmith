//! The connector generators (wire-to-board, board-to-board, microSD)
//! through the session: each of their presets makes a part from the
//! shipped bundle whose part file passes the host's checks (no blocking
//! finding, bounds on the envelope, every lead contact under a lead body
//! bound to its terminal), and at the sizes of the makers' drawings the
//! built model's lead tips stand on the drawings' recommended lands, so a
//! body placed on an imported board's footprint by its pads sits right.
use super::super::Session;
use serde_json::{Value, json};
use sparksmith_core::Engine;
use sparksmith_core::library::PackageModel;
use sparksmith_io::shipped::{MARKER, ShippedLibrary};
use sparksmith_solidcraft::models::part_file;
use std::path::{Path, PathBuf};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library/generators");

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

/// A shipped library in `dir` holding the three connector bundles.
fn library(dir: &Path) -> TestResult<PathBuf> {
    let root = dir.join("library");
    std::fs::create_dir_all(root.join("generators"))?;
    std::fs::write(root.join(MARKER), "{\"format\":1}\n")?;
    for name in ["wire-to-board", "board-to-board", "microsd"] {
        copy_dir(
            &Path::new(SHIPPED).join(name),
            &root.join("generators").join(name),
        )?;
    }
    Ok(root)
}

/// Make `preset` of `generator`; the package's part file evaluated at its
/// values and the lead bodies the host bound, by terminal.
fn make(
    root: &Path,
    generator: &str,
    preset: &str,
) -> TestResult<(
    std::sync::Arc<part_file::EvaluatedPart>,
    Vec<(String, String)>,
    usize,
)> {
    let mut session = Session::fixture(Engine::default());
    session.set_shipped(ShippedLibrary::at(root)?);
    let reply = session.dispatch(json!({"command": "library.generate_part", "params": {
        "generator": generator, "scope": "component", "preset": preset, "params": {},
        "expected_revision": session.engine().revision()}}));
    if reply["ok"] != Value::Bool(true) {
        return Err(format!("{generator} {preset}: {reply}").into());
    }
    let library = &session.engine().project().library;
    let package = &library.packages.first().ok_or("no package")?.definition;
    let PackageModel::PartFile(part) = &package.model else {
        return Err(format!("{preset}: no part file").into());
    };
    let model = library.model(part.model).ok_or("no model")?;
    let bytes = library
        .assets
        .iter()
        .find(|a| a.hash == model.definition.asset.hash)
        .ok_or("the part file is not packed")?;
    let evaluated = part_file::evaluate(bytes.data.as_slice(), &part.values)?;
    let leads = part
        .derived
        .leads
        .iter()
        .map(|l| (l.body.clone(), l.terminal.clone()))
        .collect();
    Ok((evaluated, leads, package.terminals.len()))
}

/// The presets of the three generators make parts with their models, a
/// lead body bound to every terminal that has a lead.
#[test]
fn every_connector_preset_makes_a_part_with_its_model() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    for (generator, presets) in [
        (
            "wire-to-board",
            include_str!("../../../../generators/wire-to-board/presets.json"),
        ),
        (
            "board-to-board",
            include_str!("../../../../generators/board-to-board/presets.json"),
        ),
        (
            "microsd",
            include_str!("../../../../generators/microsd/presets.json"),
        ),
    ] {
        let presets = serde_json::from_str::<Vec<Value>>(presets)?;
        // Every third preset and the last: each series (entry, pitch,
        // lead style, alignment pins) differs from its neighbours only in
        // its count, and each part file evaluates in 0.2–0.7 s; the module's
        // own tests run every preset.
        let last = presets.len() - 1;
        for (_, preset) in presets
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 3 == 0 || *i == last)
        {
            let key = preset["key"].as_str().ok_or("no key")?;
            let (_, leads, terminals) = make(&root, generator, key)?;
            let bound: std::collections::BTreeSet<&str> =
                leads.iter().map(|(_, t)| t.as_str()).collect();
            assert_eq!(bound.len(), terminals, "{generator} {key}");
        }
    }
    Ok(())
}

/// At the drawings' sizes, every contact body's tip (its far end along
/// y on the board) lies on the drawing's recommended land, and the
/// contacts stand at the lands' pitch: mm, from the makers' public
/// drawings (see the presets' citations).
#[test]
fn lead_tips_stand_on_the_drawings_lands() -> TestResult {
    let temp = tempfile::tempdir()?;
    let root = library(temp.path())?;
    // generator, preset, body prefix, tip side (+1: max y, −1: min y, 0:
    // both rows, |y|), tip expected, land range along y, first x, pitch,
    // contacts in the row.
    type Case<'a> = (
        &'a str,
        &'a str,
        &'a str,
        i32,
        f64,
        [f64; 2],
        f64,
        f64,
        usize,
    );
    let cases: [Case<'_>; 6] = [
        // JST SH SM04B-SRSS-TB: 0.6 × 1.55 mm lands at 1.0 mm; tails
        // 0.7 mm past the 4.25 mm shroud.
        (
            "wire-to-board",
            "wtb-sh-side-4",
            "contact",
            1,
            0.375,
            [-0.775, 0.775],
            -1.5,
            1.0,
            4,
        ),
        (
            "wire-to-board",
            "wtb-sh-side-6",
            "contact",
            1,
            0.375,
            [-0.775, 0.775],
            -2.5,
            1.0,
            6,
        ),
        // JST PH S2B-PH-SM4-TB: 1.0 × 3.5 mm lands at 2.0 mm.
        (
            "wire-to-board",
            "wtb-ph-side-2",
            "contact",
            1,
            1.55,
            [-1.75, 1.75],
            -1.0,
            2.0,
            2,
        ),
        // Samtec FTSH-105-01: 2.79 mm lands, outer edges 6.86 mm apart;
        // tails 5.8 mm tip to tip.
        (
            "board-to-board",
            "b2b-ftsh-5-01",
            "tail",
            0,
            2.9,
            [0.64, 3.43],
            -2.54,
            1.27,
            10,
        ),
        // Samtec TSM-104/108-03-x-DV-A: tails 7.44 mm tip to tip.
        (
            "board-to-board",
            "b2b-tsm-4-03-a",
            "tail",
            0,
            3.72,
            [1.055, 3.845],
            -3.81,
            2.54,
            8,
        ),
        // Hirose DM3D-SF: 1.75 mm contact lands at 1.1 mm, contact 8 at
        // −4.5 mm.
        (
            "microsd",
            "microsd-dm3d",
            "contact",
            -1,
            -0.65,
            [-0.875, 0.875],
            -4.5,
            1.1,
            8,
        ),
    ];
    for (generator, preset, prefix, side, tip, land, first, pitch, count) in cases {
        let (part, _, _) = make(&root, generator, preset)?;
        let mut xs = Vec::new();
        for body in part.bodies.iter().filter(|b| b.name.starts_with(prefix)) {
            let [lo, hi] = body.bounds_mm;
            let at = match side {
                1 => hi[1],
                -1 => lo[1],
                _ => hi[1].abs().max(lo[1].abs()),
            };
            assert!((at - tip).abs() < 1e-6, "{preset} {}: tip {at}", body.name);
            assert!(land[0] <= at && at <= land[1], "{preset} {}", body.name);
            assert!(lo[2].abs() < 1e-6, "{preset} {}: on the board", body.name);
            xs.push((lo[0] + hi[0]) / 2.0);
        }
        xs.sort_by(f64::total_cmp);
        xs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        let expected: Vec<f64> = (0..count / if side == 0 { 2 } else { 1 })
            .map(|k| first + k as f64 * pitch)
            .collect();
        assert_eq!(xs.len(), expected.len(), "{preset}: {xs:?}");
        for (x, e) in xs.iter().zip(&expected) {
            assert!((x - e).abs() < 1e-6, "{preset}: {xs:?}");
        }
    }
    Ok(())
}

//! The shipped stackups load, and a broken file is refused naming the file
//! and the field.
use super::*;
use crate::rule_profiles::shipped_profiles;
use sparksmith_core::model::{DielectricKind, GlassStyle};
use sparksmith_core::{CopperLayer, Stackup};
use std::path::PathBuf;

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn error(bytes: &[u8]) -> String {
    decode(bytes)
        .err()
        .map_or_else(String::new, |e| e.to_string())
}

/// A shipped file's JSON, to break.
fn example() -> Fallible<serde_json::Value> {
    let shipped = ShippedLibrary::for_checkout()?;
    let first = load(&shipped)?;
    let stackup = first.first().ok_or("no stackup ships")?;
    Ok(serde_json::to_value(FabStackupFile {
        format: FAB_STACKUP_FORMAT,
        fab_stackup: stackup.clone(),
    })?)
}

/// Every one of the 122 loads, belongs to a shipped profile of its maker
/// and builds a stackup the board rules take, whose copper and dielectrics
/// add to the pressed layers.
#[test]
fn every_shipped_stackup_loads_and_builds() -> Fallible<()> {
    let shipped = ShippedLibrary::for_checkout()?;
    let all = load(&shipped)?;
    assert_eq!(all.len(), 122);
    let profiles = shipped_profiles(&shipped)?;
    for s in &all {
        for id in &s.fab_profiles {
            let profile = profiles
                .iter()
                .find(|p| p.id == *id)
                .ok_or_else(|| format!("{}: no profile {id}", s.id))?;
            assert_eq!(profile.fabricator.as_deref(), Some(s.fabricator.as_str()));
        }
        let board = Stackup::preset(s.copper_layers)?;
        let taken = s.stackup(&board, false)?;
        let dielectrics: f64 = taken.dielectrics.iter().map(|d| d.thickness_mm).sum();
        let listed: f64 = s
            .layers
            .iter()
            .filter(|l| l.role != sparksmith_core::layout::fab_stackup::FabLayerRole::Copper)
            .map(sparksmith_core::layout::fab_stackup::FabLayer::pressed_mm)
            .sum();
        assert!((dielectrics - listed).abs() < 1e-5, "{}", s.id);
        assert!(s.is_build_of(&taken), "{}", s.id);
        // A layer has a Dk exactly where its plies state one.
        for (d, listed) in taken.dielectrics.iter().zip(s.dielectrics()) {
            assert_eq!(d.dk, listed.dk(), "{}", s.id);
            let averaged = listed.average_dk().ok_or("a ply has no Dk")?;
            let (low, high) = listed
                .plies
                .iter()
                .filter_map(|p| p.dk)
                .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(v), h.max(v)));
            assert!((low - 1e-4..=high + 1e-4).contains(&averaged), "{}", s.id);
        }
    }
    Ok(())
}

/// Known stackups, by the published tables (stackups report): JLC04161H-7628
/// is 0.2104 / 1.065 / 0.2104 mm; its 3313 sibling has 3313 glass, Dk 4.1;
/// PCBWay's 0.8 mm one presses 0.196 mm prepregs to 0.1715 mm and its 2
/// foils' 0.5 oz base finishes at 1 oz.
#[test]
fn known_stackups_read_as_published() -> Fallible<()> {
    let all = load(&ShippedLibrary::for_checkout()?)?;
    let board = Stackup::preset(4)?;
    let a = find(&all, "jlc04161h-7628")?.stackup(&board, false)?;
    let mm: Vec<f64> = a.dielectrics.iter().map(|d| d.thickness_mm).collect();
    assert_eq!(mm, [0.2104, 1.065, 0.2104]);
    let um: Vec<f64> = a.copper.iter().map(|c| c.thickness_um).collect();
    assert_eq!(um, [35.0, 15.2, 15.2, 35.0]);
    assert_eq!(a.dielectrics[1].kind, DielectricKind::Core);
    assert_eq!(a.dielectrics[1].glass, GlassStyle::Unspecified);
    let b = find(&all, "jlc04161h-3313")?.stackup(&board, false)?;
    assert_eq!(b.dielectrics[0].glass, GlassStyle::Style3313);
    assert_eq!(b.dielectrics[0].dk, Some(4.1));
    assert_eq!(b.layers().len(), 4);
    assert_eq!(b.index(CopperLayer::Bottom), Some(3));
    let c = find(&all, "pcbway-4-layer-0-8mm-1oz-1oz-30pc")?;
    let taken = c.stackup(&board, false)?;
    assert_eq!(taken.dielectrics[0].thickness_mm, 0.1715);
    assert_eq!(taken.copper[0].thickness_um, 35.0);
    assert_eq!(c.layers_sum_mm, 0.727);
    assert_eq!(c.pressed_sum_mm(), 0.678);
    // A 7628 prepreg of a brand is 7628 glass.
    let d = find(&all, "jlc06101h-7628a")?.stackup(&Stackup::preset(6)?, false)?;
    assert_eq!(d.dielectrics[0].glass, GlassStyle::Style7628);
    Ok(())
}

#[test]
fn a_broken_file_is_refused_naming_the_field() -> Fallible<()> {
    let good = example()?;
    decode(&serde_json::to_vec(&good)?)?;
    let broken = |edit: &dyn Fn(&mut serde_json::Value)| -> Fallible<String> {
        let mut value = good.clone();
        edit(&mut value);
        Ok(error(&serde_json::to_vec(&value)?))
    };
    let said = broken(&|v| v["fab_stackup"]["layers"][0]["extra"] = 1.into())?;
    assert!(
        said.contains("fab_stackup.layers[0]") && said.contains("extra"),
        "{said}"
    );
    let said = broken(&|v| v["fab_stackup"]["layers"][1]["dk"] = "4.4".into())?;
    assert!(said.contains("fab_stackup.layers[1].dk"), "{said}");
    let said = broken(&|v| v["fab_stackup"]["layers"][1]["thickness_mm"] = 0.into())?;
    assert!(
        said.contains("fab_stackup.") && said.contains("layers[1].thickness_mm"),
        "{said}"
    );
    let said = broken(&|v| v["fab_stackup"]["nominal"]["board_mm"] = serde_json::Value::Null)?;
    assert!(said.contains("fab_stackup.nominal.board_mm"), "{said}");
    let said = broken(&|v| {
        if let Some(body) = v["fab_stackup"].as_object_mut() {
            body.remove("source");
        }
    })?;
    assert!(said.contains("source"), "{said}");
    let said = broken(&|v| v["format"] = 2.into())?;
    assert!(said.contains("unsupported stackup format 2"), "{said}");
    assert!(error(b"{}").contains("no format"));
    assert!(!error(b"{\"format\":1").is_empty());
    Ok(())
}

/// A library of one broken, misnamed or repeated file.
fn library(dir: &Path, files: &[(&str, Vec<u8>)]) -> Fallible<ShippedLibrary> {
    let root: PathBuf = dir.join("library");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join(crate::shipped::MARKER), "{\"format\":1}")?;
    for (name, bytes) in files {
        let path = root.join("stackups").join(name);
        std::fs::create_dir_all(path.parent().ok_or("no folder")?)?;
        std::fs::write(path, bytes)?;
    }
    Ok(ShippedLibrary::at(&root)?)
}

#[test]
fn a_bad_library_names_its_file() -> Fallible<()> {
    let good = example()?;
    let bytes = serde_json::to_vec(&good)?;
    let id = good["fab_stackup"]["id"]
        .as_str()
        .ok_or("no id")?
        .to_owned();
    let maker = good["fab_stackup"]["fabricator"]
        .as_str()
        .ok_or("no maker")?
        .to_owned();
    type Files = Vec<(String, Vec<u8>)>;
    let cases: [(&str, Files); 4] = [
        (
            "broken.json",
            vec![(format!("{maker}/broken.json"), b"{".to_vec())],
        ),
        (
            "not named for its id",
            vec![(format!("{maker}/other.json"), bytes.clone())],
        ),
        (
            "folder is not named",
            vec![(format!("elsewhere/{id}.json"), bytes.clone())],
        ),
        ("", vec![(format!("{maker}/{id}.json"), bytes.clone())]),
    ];
    for (needle, files) in cases {
        let temp = tempfile::tempdir()?;
        let files: Vec<(&str, Vec<u8>)> =
            files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
        let read = load(&library(temp.path(), &files)?);
        if needle.is_empty() {
            assert_eq!(read?.len(), 1);
        } else {
            let said = read.err().map_or_else(String::new, |e| e.to_string());
            assert!(
                said.contains(needle) && said.contains(".json"),
                "{needle}: {said}"
            );
        }
    }
    // A family file beside the folders is not one of them, and an empty
    // shelf lists nothing.
    let temp = tempfile::tempdir()?;
    let family = library(temp.path(), &[("family.json", b"{}".to_vec())])?;
    assert!(load(&family)?.is_empty());
    assert!(find(&[], "none").is_err());
    Ok(())
}

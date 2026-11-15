#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice S4: envelope and STEP models built as named designs give the
//! same face names on every build, in this process and in a fresh one.
use super::*;
use crate::models::imported::tests::{fixture_step, model};
#[allow(unused_imports)]
use crate::models::tests::{nm, pt};
use solidcraft_engine::doc::naming;
use sparksmith_core::PackageTerminalId;
use sparksmith_core::library::{PackageModel, PackageTerminal};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A 0603 chip resistor's package with its envelope as its model.
pub(crate) fn resistor() -> PackageDefinition {
    PackageDefinition {
        next_local: 3,
        terminals: (1..=2)
            .map(|i| PackageTerminal {
                id: PackageTerminalId(i),
                designation: i.to_string(),
            })
            .collect(),
        model: PackageModel::default(),
        envelope_nm: [nm(1.6), nm(0.8), nm(0.45)],
        ..PackageDefinition::default()
    }
}

/// Every body of a design with its face names, in order.
/// A design's bodies, each with its face names.
type Names = Vec<(String, Vec<String>)>;

fn names(state: &ModelState) -> Names {
    state
        .bodies
        .iter()
        .map(|b| (b.name.clone(), naming::face_names(b).to_vec()))
        .collect()
}

/// The names of the resistor's envelope design and the STEP fixture's
/// design, each built anew (not from the cache).
fn built_names() -> Fallible<Vec<Names>> {
    let envelope = names(envelope(&resistor())?.state());
    let bytes = fixture_step()?;
    let seen = crate::models::inspect::inspect(&bytes, ModelFormat::Step)?;
    let step = names(step(&model(ModelFormat::Step, &bytes, &seen), &bytes)?.state());
    Ok(vec![envelope, step])
}

/// The retired chip builder's resistor face names, as saved references
/// name them. Solidcraft writes a fillet's nested edge in parentheses (`F5:blend(<edge>)`, upstream
/// #11359); names saved in the older colon form (`F5:blend:<edge>`) still read
/// as the same names (`old_blend_names_read_as_the_current_ones`).
const RESISTOR_NAMES: [(&str, &[&str]); 4] = [
    (
        "body",
        &[
            "F1:+z",
            "F1:-x",
            "F1:-y",
            "F1:+x",
            "F5:blend(F1:+y|F1:+z)",
            "F1:+y",
            "F5:blend(F1:+y|F1:-z)",
            "F1:-z",
            "F5:blend(F1:-y|F1:-z)",
            "F5:blend(F1:+z|F1:-y)",
        ],
    ),
    (
        "overcoat",
        &["F2:+z", "F2:+y", "F2:-x", "F2:-y", "F2:+x", "F2:-z"],
    ),
    (
        "termination 1",
        &[
            "F6:blend(F3:-x|F3:-y)",
            "F6:blend(F3:+z|F3:-x)",
            "F3:-x",
            "F6:blend(F3:-x|F3:-z)",
            "F3:-z",
            "F3:-y",
            "F3:+z",
            "F3:+y",
            "F3:+x",
            "F6:blend(F3:+y|F3:-x)",
        ],
    ),
    (
        "termination 2",
        &[
            "F7:blend(F4:+x|F4:+y)",
            "F7:blend(F4:+x|F4:-y)",
            "F7:blend(F4:+x|F4:-z)",
            "F7:blend(F4:+x|F4:+z)",
            "F4:-y",
            "F4:-z",
            "F4:-x",
            "F4:+z",
            "F4:+y",
            "F4:+x",
        ],
    ),
];

/// A reference saved before Solidcraft wrote nested names in parentheses
/// names a fillet face `F5:blend:F1:+y|F1:+z`: it reads as the same face
/// name as the current text, so it still finds its face.
#[test]
fn old_blend_names_read_as_the_current_ones() -> Fallible {
    use solidcraft_engine::doc::persistent_name::FaceName;
    let mut blends = 0;
    for name in RESISTOR_NAMES.iter().flat_map(|(_, faces)| faces.iter()) {
        let Some((head, edge)) = name.split_once(":blend(") else {
            continue;
        };
        let edge = edge.strip_suffix(')').ok_or("a closed blend name")?;
        let old = format!("{head}:blend:{edge}");
        assert_eq!(FaceName::parse(&old)?, FaceName::parse(name)?, "{old}");
        assert_eq!(FaceName::parse(&old)?.to_string(), *name);
        blends += 1;
    }
    assert_eq!(blends, 12);
    Ok(())
}

/// Built twice, the designs have the same bodies and face names; the
/// cache gives the same design back without building it.
#[test]
fn a_named_design_has_the_same_names_on_every_build() -> Fallible {
    let _context = crate::looks::test_context().map(|c| c.enter());
    assert_eq!(built_names()?, built_names()?);
    let package = resistor();
    let a = super::package(&package)?;
    let b = super::package(&package)?;
    assert!(Arc::ptr_eq(&a, &b));
    Ok(())
}

const DUMP: &str = "SPARKSMITH_NAMED_DESIGN_DUMP";

/// Writes [`built_names`] to the file `SPARKSMITH_NAMED_DESIGN_DUMP` names;
/// run in a fresh process by the next test.
#[test]
#[ignore = "run in a fresh process by a_named_design_has_the_same_names_in_a_fresh_process"]
fn dump_named_design_names() -> Fallible {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let path = std::env::var(DUMP)?;
    std::fs::write(path, serde_json::to_vec(&built_names()?)?)?;
    Ok(())
}

/// Another process builds the same designs with the same names.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_named_design_has_the_same_names_in_a_fresh_process() -> Fallible {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("names.json");
    let status = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "models::named::tests::dump_named_design_names",
            "--ignored",
            "--test-threads",
            "1",
        ])
        .env(DUMP, &path)
        .status()?;
    assert!(status.success());
    let theirs: Vec<Names> = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(theirs, built_names()?);
    Ok(())
}

/// The envelope design is one box: six faces of one feature.
#[test]
fn an_envelope_is_a_one_box_design() -> Fallible {
    let design = super::package(&resistor())?;
    let got = names(design.state());
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, ENVELOPE_BODY);
    assert_eq!(got[0].1.len(), 6, "{got:?}");
    let b = design.state().bodies[0].outline_bounds();
    assert!((b.min.x + 0.8).abs() < 1e-9 && (b.max.z - 0.45).abs() < 1e-9);
    Ok(())
}

/// The STEP design names its bodies by their keys in the file, and the
/// file is parsed once while the board's source cache holds it.
#[test]
fn a_step_design_names_bodies_by_their_keys() -> Fallible {
    let bytes = fixture_step()?;
    let seen = crate::models::inspect::inspect(&bytes, ModelFormat::Step)?;
    let definition = model(ModelFormat::Step, &bytes, &seen);
    let design = step(&definition, &bytes)?;
    let bodies: Vec<&str> = design
        .state()
        .bodies
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    let keys: Vec<&str> = seen.bodies.iter().map(|b| b.key.as_str()).collect();
    assert_eq!(bodies, keys);
    Ok(())
}

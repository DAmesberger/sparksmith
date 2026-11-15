//! The chain and the golden files: one per format version, of the same
//! small original design (`Engine::demo`, two connected parts placed on
//! the board, with their library and open drafts), kept forever.
//!
//! Each old golden converts to the current golden byte for byte, and the
//! current golden opens and saves byte for byte. A golden file is never
//! regenerated once committed. When `FILE_VERSION` rises, the new
//! version's golden is the previous golden converted by the new step
//! (`write_missing_golden_files`, which writes only files that are
//! missing), checked by hand before it is committed.
//!
//! Converting to version 5 runs the shipped generators and Solidcraft
//! (typed package forms), which this crate does not depend on: the tests
//! that convert, and the writer, are `sparksmith-automation`'s
//! (`tests/format_golden.rs`), with its converter host.
use super::*;
use crate::{read_project, write_project};
use sparksmith_core::Engine;
use std::path::PathBuf;

type TestResult = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn golden(version: u32) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/format")
        .join(format!("v{version}"))
        .join("demo.sparksmith")
}

#[test]
fn the_chain_has_one_step_from_every_older_version() {
    for version in 1..FILE_VERSION {
        assert_eq!(
            STEPS.iter().filter(|s| s.from == version).count(),
            1,
            "one step from version {version}"
        );
    }
    assert!(STEPS.iter().all(|s| s.from < FILE_VERSION));
}

#[test]
fn every_version_has_a_golden_file() {
    for version in 1..=FILE_VERSION {
        assert!(golden(version).is_file(), "{}", golden(version).display());
    }
}

#[test]
fn the_current_golden_opens_and_saves_byte_for_byte() -> TestResult {
    let dir = tempfile::tempdir()?;
    let saved = dir.path().join("saved.sparksmith");
    let file = read_project(&golden(FILE_VERSION))?;
    Engine::from_file(file.clone())?;
    write_project(&saved, &file)?;
    assert_eq!(fs_read(&saved)?, fs_read(&golden(FILE_VERSION))?);
    Ok(())
}

#[test]
fn the_chain_refuses_what_it_cannot_convert() {
    let options = Options::default();
    let newer = upgrade(
        &mut serde_json::json!({"version": FILE_VERSION + 1}),
        &options,
    );
    assert!(newer.is_err_and(|e| e.to_string().contains("newer")));
    for bad in [
        serde_json::json!({}),
        serde_json::json!({"version": 0}),
        serde_json::json!({"version": "2"}),
    ] {
        assert!(upgrade(&mut bad.clone(), &options).is_err(), "{bad}");
    }
    let mut current = serde_json::json!({"version": FILE_VERSION});
    assert_eq!(
        upgrade(&mut current, &options).ok(),
        Some(Upgraded {
            from: FILE_VERSION,
            values: 0
        })
    );
}

fn fs_read(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

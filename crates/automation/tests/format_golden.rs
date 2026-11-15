//! The format golden files (`crates/io/tests/fixtures/format/v<N>/`),
//! converted with the shipped host (`convert_host::ShippedHost`): every
//! old golden converts to the current golden byte for byte. Converting to
//! version 5 runs the shipped generators and Solidcraft, so these tests
//! live here rather than in `sparksmith-io` (whose `convert::tests` hold
//! the rest).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sparksmith_automation::convert_host::ShippedHost;
use sparksmith_core::Engine;
use sparksmith_core::format::{CONTAINER_DOCUMENT, FILE_VERSION};
use sparksmith_io::convert::{Options, convert_path};
use sparksmith_io::shipped::ShippedLibrary;
use sparksmith_io::{read_project, write_project};
use std::io::{Read, Write};
use std::path::PathBuf;

type TestResult = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn golden(version: u32) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../io/tests/fixtures/format")
        .join(format!("v{version}"))
        .join("demo.sparksmith")
}

fn options() -> Result<Options, Box<dyn std::error::Error + Send + Sync>> {
    let library = ShippedLibrary::at(&PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../library"
    )))?;
    Ok(ShippedHost::new(&library)?.options())
}

fn fs_read(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

#[test]
fn each_old_golden_converts_to_the_current_golden() -> TestResult {
    let dir = tempfile::tempdir()?;
    for version in 1..FILE_VERSION {
        let path = dir.path().join(format!("old-v{version}.sparksmith"));
        std::fs::copy(golden(version), &path)?;
        let refused = read_project(&path).err().ok_or("an old file is refused")?;
        assert!(
            refused.to_string().contains(&format!(
                "`{} convert FILE`",
                sparksmith_core::product::SHORT_NAME
            )),
            "{refused}"
        );
        let lines = convert_path(&path, &options()?)?;
        assert!(
            lines[0].contains(&format!(
                "converted from version {version} to {FILE_VERSION}"
            )),
            "{lines:?}"
        );
        assert_eq!(
            fs_read(&path)?,
            fs_read(&golden(FILE_VERSION))?,
            "v{version}"
        );
        let backup = path.with_file_name(format!("old-v{version}.sparksmith.v{version}"));
        assert_eq!(fs_read(&backup)?, fs_read(&golden(version))?);
        Engine::from_file(read_project(&path)?)?;
        // A converted file is current: a second run changes nothing.
        let again = convert_path(&path, &options()?)?;
        assert_eq!(
            again,
            [format!(
                "{}: already version {FILE_VERSION}",
                path.display()
            )]
        );
        assert_eq!(fs_read(&path)?, fs_read(&golden(FILE_VERSION))?);
    }
    Ok(())
}

/// Writes the golden files that are missing (run with `--ignored` after
/// raising `FILE_VERSION`). The first golden of all was the demo design as
/// version 2 saved it; version 1 is that file as the version 1 writer
/// wrote it, which differed only in the version number.
#[test]
#[ignore = "writes golden files; run by hand when a version is added"]
fn write_missing_golden_files() -> TestResult {
    if !golden(2).exists() {
        std::fs::create_dir_all(golden(2).parent().ok_or("folder")?)?;
        write_project(&golden(2), &Engine::demo()?.to_file())?;
    }
    if !golden(1).exists() {
        std::fs::create_dir_all(golden(1).parent().ok_or("folder")?)?;
        let bytes = fs_read(&golden(2))?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))?;
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            if entry.name()? == CONTAINER_DOCUMENT {
                let mut text = String::new();
                entry.read_to_string(&mut text)?;
                let old = text.replacen("\"version\": 2,", "\"version\": 1,", 1);
                assert_ne!(old, text);
                out.start_file(CONTAINER_DOCUMENT, options)?;
                out.write_all(old.as_bytes())?;
            } else {
                drop(entry);
                out.raw_copy_file(archive.by_index_raw(i)?)?;
            }
        }
        std::fs::write(golden(1), out.finish()?.into_inner())?;
    }
    let (current, previous) = (golden(FILE_VERSION), golden(FILE_VERSION - 1));
    if !current.exists() {
        std::fs::create_dir_all(current.parent().ok_or("folder")?)?;
        std::fs::copy(&previous, &current)?;
        convert_path(&current, &options()?)?;
        let backup = format!("{}.v{}", current.display(), FILE_VERSION - 1);
        std::fs::remove_file(backup)?;
    }
    Ok(())
}

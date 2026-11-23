//! The shipped example projects as files (file format slice F2): opening
//! and saving one changes no byte, and refilling its fills, whose copper the
//! file does not hold, changes no byte either.
use sparksmith_core::Engine;
use std::path::{Path, PathBuf};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Every shipped example project, the dev board first.
fn examples() -> [PathBuf; 6] {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    [
        root.join("esp32-s31-devboard/esp32-s31-devboard.sparksmith"),
        root.join("esp32-s31-wroom-3/esp32-s31-wroom-3.sparksmith"),
        root.join("esp32-s31-wroom-3/run1/esp32-s31-wroom-3.sparksmith"),
        root.join("esp32-s31-wroom-3/run-mcp-1/ESP32-S31-WROOM-3.sparksmith"),
        root.join("opa2333-mcp-1/OPA2333AIDR.sparksmith"),
        root.join("tlv75733-mcp-1/TLV75733PDBVR.sparksmith"),
    ]
}

#[test]
fn an_example_opens_and_saves_byte_for_byte() -> Fallible {
    let dir = tempfile::tempdir()?;
    for path in examples() {
        let file = sparksmith_io::read_project(&path)?;
        let saved = dir.path().join("saved.sparksmith");
        sparksmith_io::write_project(&saved, &file)?;
        assert!(
            std::fs::read(&saved)? == std::fs::read(&path)?,
            "{} changed on open and save",
            path.display()
        );
    }
    Ok(())
}

/// Rewrite stored-format fixtures without assigning the module template a
/// manufacturing identity. The dev board is authored by its command replay.
#[test]
#[ignore = "writes the component examples and IO golden through the project writer"]
fn regenerate_stored_project_fixtures() -> Fallible {
    if std::env::var("SPARKSMITH_WRITE_EXAMPLE").as_deref() != Ok("1") {
        return Err("set SPARKSMITH_WRITE_EXAMPLE=1 to rewrite the tracked fixtures".into());
    }
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format/demo.sparksmith");
    for path in examples().into_iter().skip(1).chain([golden]) {
        let file = sparksmith_io::read_project(&path)?;
        sparksmith_io::write_project(&path, &file)?;
    }
    Ok(())
}

#[test]
fn refilling_the_examples_fills_changes_no_byte() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = &examples()[0];
    let mut engine = Engine::from_file(sparksmith_io::read_project(path)?)?;
    assert!(!engine.project().fills.is_empty());
    assert!(engine.project().fills.iter().all(|f| f.computed.is_none()));
    let stored = engine.refill_stale()?;
    assert_eq!(stored, engine.project().fills.len());
    assert!(engine.project().fills.iter().all(|f| f.computed.is_some()));
    let saved = dir.path().join("refilled.sparksmith");
    sparksmith_io::write_project(&saved, &engine.to_file())?;
    assert!(std::fs::read(&saved)? == std::fs::read(path)?);
    // Unchanged inputs: nothing is stale, nothing is stored again.
    assert_eq!(engine.refill_stale()?, 0);
    Ok(())
}

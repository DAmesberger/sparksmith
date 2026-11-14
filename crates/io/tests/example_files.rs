//! The shipped example projects as files (file format slice F2): opening
//! and saving one changes no byte, and refilling its fills, whose copper the
//! file does not hold, changes no byte either.
use sparksmith_core::Engine;
use std::path::{Path, PathBuf};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn examples() -> [PathBuf; 2] {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    [
        root.join("esp32-s31-devboard/esp32-s31-devboard.sparksmith"),
        root.join("esp32-s31-wroom-3/esp32-s31-wroom-3.sparksmith"),
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

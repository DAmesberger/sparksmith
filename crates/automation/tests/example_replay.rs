//! The committed ESP32-S31 dev board is what its replay builds, byte for
//! byte: the replay (`examples/esp32-s31-devboard/regenerate.sh`) runs
//! through a session here and its save is compared with the committed
//! file. A generator, part file or command change that alters the board
//! fails this test until the example is regenerated (and, when the replay
//! carries a generated package's body, rebased:
//! `examples/esp32-s31-devboard/authoring/rebase.py`).
use serde_json::{Value, json};
use sparksmith_automation::Session;
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const EXAMPLE: &str = "examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith";

#[test]
fn the_committed_dev_board_is_what_its_replay_builds() -> Fallible {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let replay = std::fs::read_to_string(
        root.join("examples/esp32-s31-devboard/esp32-s31-devboard.replay.jsonl"),
    )?;
    let dir = tempfile::tempdir()?;
    let saved = dir.path().join("esp32-s31-devboard.sparksmith");
    let mut session = Session::default();
    let mut saves = 0;
    for (n, line) in replay.lines().enumerate() {
        let mut request: Value = serde_json::from_str(line)?;
        let command = request["command"].as_str().unwrap_or_default().to_string();
        // Paths the replay names are relative to the repository root; its
        // one save, of the example, goes to a scratch file instead.
        if command == "project.save" {
            let path = request["params"]["path"].as_str().ok_or("save path")?;
            assert_eq!(path, EXAMPLE, "line {}", n + 1);
            request["params"]["path"] = json!(saved.display().to_string());
            saves += 1;
        } else if command == "project.open" || command == "library.use_part_file" {
            let path = request["params"]["path"].as_str().ok_or("path")?;
            request["params"]["path"] = json!(root.join(path).display().to_string());
        }
        let reply = session.dispatch(request);
        assert_eq!(reply["ok"], true, "line {}: {reply}", n + 1);
    }
    assert_eq!(saves, 1);
    let built = std::fs::read(&saved)?;
    let committed = std::fs::read(root.join(EXAMPLE))?;
    assert!(
        built == committed,
        "{EXAMPLE} is not what its replay builds ({} bytes built, {} committed): run \
         `sh examples/esp32-s31-devboard/regenerate.sh` (after \
         `python3 -B examples/esp32-s31-devboard/authoring/rebase.py` when a generator changed) \
         and commit the example",
        built.len(),
        committed.len()
    );
    Ok(())
}

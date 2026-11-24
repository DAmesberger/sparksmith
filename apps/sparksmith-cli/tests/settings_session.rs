//! A headless session takes the user's Settings as the app's engine does
//! (`sparksmith_io::settings::configure_from_env`): with "Require a reason
//! to waive errors" on in the settings file, `fabrication.waive` without a
//! reason is refused through the real CLI, and with it off it is not.
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The replies of one operator stdio session under `config`.
fn session(config: &Path, cache: &Path, requests: &[Value]) -> Fallible<Vec<Value>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", config)
        .env("SPARKSMITH_CACHE_DIR", cache)
        .args(["stdio", "--operator"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    {
        let mut stdin = child.stdin.take().ok_or("stdin")?;
        for request in requests {
            writeln!(stdin, "{request}")?;
        }
    }
    let out = child.wait_with_output()?;
    String::from_utf8(out.stdout)?
        .lines()
        .map(|l| Ok(serde_json::from_str(l)?))
        .collect()
}

fn waive(reason: &str) -> Value {
    json!({"command": "project.apply", "params": {
        "expected_revision": 0,
        "operation": {"editor": "Fabrication", "operation": {
            "command": "fabrication.waive",
            "params": {"keys": ["board.clearance []"], "reason": reason}}},
    }})
}

/// The replies to an empty and then a given reason, from a fresh project
/// at revision 0, with `require` in the settings file.
fn waivers(require: bool) -> Fallible<Vec<Value>> {
    let dir = tempfile::tempdir()?;
    let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
    let settings = sparksmith_io::settings::UserSettings {
        require_waiver_reason: require,
        ..Default::default()
    };
    sparksmith_io::settings::write_settings(
        &config.join(sparksmith_io::settings::SETTINGS_FILE),
        &settings,
    )?;
    let replies = session(&config, &cache, &[waive(""), waive("prototype")])?;
    assert_eq!(replies.len(), 2, "{replies:?}");
    Ok(replies)
}

#[test]
fn a_required_waiver_reason_holds_in_a_headless_session() -> Fallible<()> {
    let required = waivers(true)?;
    assert_eq!(required[0]["ok"], false, "{}", required[0]);
    assert!(
        required[0]["error"]
            .to_string()
            .contains("fabrication.waiver_reason"),
        "{}",
        required[0]
    );
    // Refused, so the document stayed at revision 0 for the next waiver.
    assert_eq!(required[1]["ok"], true, "{}", required[1]);

    // The control: with the setting off, an empty reason is taken.
    let optional = waivers(false)?;
    assert_eq!(optional[0]["ok"], true, "{}", optional[0]);
    Ok(())
}

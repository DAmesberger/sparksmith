//! A fabrication profile named by id resolves through the user's profile
//! library: the CLI runs with a temporary config folder
//! (`SPARKSMITH_CONFIG_DIR`) holding the two sourced test fixture profiles,
//! imported through `fab-profile import`.
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

type Fallible = Result<(), Box<dyn std::error::Error>>;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/editor-core/src/rules/fixtures"
);

fn cli(config: &Path, cache: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sparksmith"));
    command
        .env("SPARKSMITH_CONFIG_DIR", config)
        .env("SPARKSMITH_CACHE_DIR", cache);
    command
}

/// Run `args` and return its standard output; a failure is an error.
fn run(config: &Path, cache: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let out = cli(config, cache).args(args).output()?;
    if !out.status.success() {
        return Err(format!("{args:?}: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// One reply per request line of an operator stdio session.
fn session(
    config: &Path,
    cache: &Path,
    requests: &[Value],
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let mut child = cli(config, cache)
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

#[test]
fn the_production_check_resolves_a_profile_id_through_the_library() -> Fallible {
    let dir = tempfile::tempdir()?;
    let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
    for fixture in ["prototype", "fine"] {
        let file = format!("{FIXTURES}/{fixture}.json");
        run(&config, &cache, &["fab-profile", "import", &file])?;
    }
    let listed: Value = serde_json::from_str(&run(&config, &cache, &["fab-profile", "list"])?)?;
    let ids: Vec<&str> = listed["profiles"]
        .as_array()
        .ok_or("profiles")?
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "test-fine",
            "test-prototype",
            "jlcpcb-2-layer",
            "jlcpcb-4-layer",
            "pcbway-2-layer",
            "pcbway-4-layer",
            "standard"
        ],
        "the user's profiles, then the shipped ones"
    );

    // The showcase on the library's prototype profile, by id.
    let board = dir.path().join("board.sparksmith");
    let board_text = board.display().to_string();
    run(
        &config,
        &cache,
        &["showcase", &board_text, "--profile", "test-prototype"],
    )?;
    let fine: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURES}/fine.json"))?)?;
    let preflight = |profile: Value| {
        json!({"command": "project.query", "params": {
            "query": "fabrication.preflight", "params": {"profile": profile}}})
    };
    let replies = session(
        &config,
        &cache,
        &[
            json!({"command": "project.open", "params": {"expected_revision": 0, "path": board_text}}),
            preflight(json!("test-fine")),
            preflight(fine["fab_profile"].clone()),
            preflight(json!("no-such-profile")),
        ],
    )?;
    let [opened, by_id, whole, unknown] = replies.as_slice() else {
        return Err(format!("four replies expected: {replies:?}").into());
    };
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(by_id["ok"], true, "{by_id}");
    // The id reads as the whole profile does.
    assert_eq!(by_id["result"], whole["result"]);
    assert_eq!(by_id["result"]["result"]["profile"], "Test fine");
    // Its fills were computed under the prototype: stale under the fine one.
    assert!(
        by_id["result"]["result"]["blocking"].as_u64() > Some(0),
        "{by_id}"
    );
    assert_eq!(unknown["ok"], false, "{unknown}");
    Ok(())
}

/// `project.new` never asks: without `profile` it takes the user's default
/// (Settings), else the shipped default (`library/defaults/new-project.json`); `profile` names any
/// library or shipped id. `rules.fab_profiles` lists the board's own first,
/// then the shipped ones.
#[test]
fn a_new_project_takes_the_shipped_standard_profile_unless_one_is_named() -> Fallible {
    let dir = tempfile::tempdir()?;
    let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
    let profile_id =
        json!({"command": "project.inspect", "params": {"at": "/project/rules/profile/id"}});
    let replies = session(
        &config,
        &cache,
        &[
            json!({"command": "project.new", "params": {"expected_revision": 0}}),
            profile_id.clone(),
            json!({"command": "project.query", "params": {"query": "rules.fab_profiles", "params": {}}}),
            json!({"command": "project.new", "params": {"expected_revision": 1, "profile": "jlcpcb-4-layer"}}),
            profile_id,
        ],
    )?;
    let [new, first, listed, named, second] = replies.as_slice() else {
        return Err(format!("five replies expected: {replies:?}").into());
    };
    assert_eq!(new["ok"], true, "{new}");
    assert_eq!(first["result"]["value"], "standard", "{first}");
    let ids: Vec<&str> = listed["result"]["result"]
        .as_array()
        .ok_or("profiles")?
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "standard",
            "jlcpcb-2-layer",
            "jlcpcb-4-layer",
            "pcbway-2-layer",
            "pcbway-4-layer"
        ],
        "{listed}"
    );
    assert_eq!(listed["result"]["result"][0]["board"], true, "{listed}");
    assert_eq!(named["ok"], true, "{named}");
    assert_eq!(second["result"]["value"], "jlcpcb-4-layer", "{second}");
    Ok(())
}

/// The person's default fab profile, kept in the settings in `config`.
fn set_default(config: &Path, default: Option<&str>) -> Fallible {
    sparksmith_io::settings::write_settings(
        &config.join(sparksmith_io::settings::SETTINGS_FILE),
        &sparksmith_io::settings::UserSettings {
            default_rule_profile: default.map(str::to_owned),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// A default that is gone is never silently another profile: automation
/// refuses a new project and names it, one named in the request is taken,
/// and a file that has its own rules opens.
#[test]
fn a_default_that_is_gone_is_refused_by_name() -> Fallible {
    let dir = tempfile::tempdir()?;
    let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
    let file = dir.path().join("board.sparksmith").display().to_string();
    run(&config, &cache, &["parts", &file])?;
    set_default(&config, Some("gone"))?;
    let other = dir.path().join("other.sparksmith").display().to_string();
    for command in ["parts", "showcase"] {
        let refused = cli(&config, &cache).args([command, &other]).output()?;
        let said = String::from_utf8_lossy(&refused.stderr).into_owned();
        assert!(!refused.status.success() && said.contains("gone"), "{said}");
    }
    let replies = session(
        &config,
        &cache,
        &[
            json!({"command": "project.new", "params": {"expected_revision": 0}}),
            json!({"command": "project.new", "params": {
                "expected_revision": 0, "profile": "jlcpcb-2-layer"}}),
            json!({"command": "project.open", "params": {"expected_revision": 1, "path": file}}),
        ],
    )?;
    let [refused, named, opened] = replies.as_slice() else {
        return Err(format!("three replies expected: {replies:?}").into());
    };
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(refused.to_string().contains("gone"), "{refused}");
    assert_eq!(named["ok"], true, "{named}");
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(())
}

/// A session starts on the person's default profile, else on the shipped
/// one; so does every project file the CLI makes on its own.
#[test]
fn every_project_the_cli_starts_or_writes_has_a_profile() -> Fallible {
    let at = json!({"command": "project.inspect", "params": {"at": "/project/rules/profile/id"}});
    for (default, starts_on) in [
        (None, "standard"),
        (Some("jlcpcb-4-layer"), "jlcpcb-4-layer"),
    ] {
        let dir = tempfile::tempdir()?;
        let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
        set_default(&config, default)?;
        let replies = session(&config, &cache, std::slice::from_ref(&at))?;
        assert_eq!(replies[0]["result"]["value"], starts_on, "{replies:?}");
        for command in ["parts", "showcase"] {
            let file = dir.path().join(format!("{command}.sparksmith"));
            run(&config, &cache, &[command, &file.display().to_string()])?;
            let project = sparksmith_io::read_project(&file)?.project;
            let profile = project.rules.profile.map(|p| p.id);
            assert_eq!(profile.as_deref(), Some(starts_on), "{command}");
        }
    }
    Ok(())
}

/// The CLI reaches a profile's manufacturing options as the desktop does:
/// `rules.effective` lists them with the board's choices,
/// `rules.preview_fab_options` shows the limits that change and
/// `rules.set_fab_options` applies them, never asking.
#[test]
fn a_session_previews_and_sets_fab_options() -> Fallible {
    let dir = tempfile::tempdir()?;
    let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
    let minimum = json!({"options": {"limits": "minimum"}});
    let effective =
        json!({"command": "project.query", "params": {"query": "rules.effective", "params": {}}});
    let replies = session(
        &config,
        &cache,
        &[
            json!({"command": "project.new", "params": {
                "expected_revision": 0, "profile": "jlcpcb-2-layer"}}),
            effective.clone(),
            json!({"command": "project.query", "params": {
                "query": "rules.preview_fab_options", "params": minimum}}),
            json!({"command": "project.apply", "params": {"expected_revision": 1, "operation": {
                "editor": "Rules",
                "operation": {"command": "rules.set_fab_options", "params": minimum}}}}),
            effective,
        ],
    )?;
    let [new, before, preview, applied, after] = replies.as_slice() else {
        return Err(format!("five replies expected: {replies:?}").into());
    };
    assert_eq!(new["ok"], true, "{new}");
    let profile = &before["result"]["result"]["profile"];
    let options: Vec<&str> = profile["options"]
        .as_array()
        .ok_or("options")?
        .iter()
        .filter_map(|o| o["id"].as_str())
        .collect();
    assert_eq!(
        options,
        ["limits", "copper_weight", "mask_colour", "drill"],
        "{before}"
    );
    assert_eq!(profile["chosen"]["limits"], "recommended", "{before}");
    let changes = preview["result"]["result"]["changes"]
        .as_array()
        .ok_or("changes")?;
    // The PTH ring, four PTH clearances, the smallest drill and the
    // fiducial count (two at JLCPCB's absolute minimum, three recommended).
    assert_eq!(changes.len(), 7, "{preview}");
    assert_eq!(applied["ok"], true, "{applied}");
    let after = &after["result"]["result"];
    assert_eq!(after["profile"]["chosen"]["limits"], "minimum", "{after}");
    assert_eq!(
        after["effective"]["holes"]["pth_annular_ring_min"], 0.18,
        "{after}"
    );
    Ok(())
}

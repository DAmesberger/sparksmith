//! `fabrication.profile_suggestions` through a headless session: the
//! board is checked under every installed profile of the user's library
//! (a temporary `SPARKSMITH_CONFIG_DIR`) and the shipped one, and the
//! fabricators are named from the user's records. The profiles are the
//! sourced test fixture with invented fabricators; none is a fabricator's
//! numbers.
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

type Fallible = Result<(), Box<dyn std::error::Error>>;

const PROTOTYPE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/editor-core/src/rules/fixtures/prototype.json"
);

fn cli(config: &Path, cache: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sparksmith"));
    command
        .env("SPARKSMITH_CONFIG_DIR", config)
        .env("SPARKSMITH_CACHE_DIR", cache);
    command
}

fn run(config: &Path, cache: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let out = cli(config, cache).args(args).output()?;
    if !out.status.success() {
        return Err(format!("{args:?}: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

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

/// The prototype fixture as `id` of `fabricator`, its via annular ring
/// minimum `ring`, written as a user profile file.
fn write_profile(config: &Path, id: &str, fabricator: &str, ring: f64) -> Fallible {
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(PROTOTYPE)?)?;
    let profile = file["fab_profile"].as_object_mut().ok_or("fab_profile")?;
    profile.insert("id".into(), json!(id));
    profile.insert("name".into(), json!(format!("Test {id}")));
    profile.insert("fabricator".into(), json!(fabricator));
    let rules = &mut file["fab_profile"]["rules"];
    rules["via_annular_ring_min"]["value"] = json!(ring);
    // The default via keeps the ring: a 0.3 mm drill.
    rules["via_diameter_default"]["value"] = json!(0.3 + 2.0 * ring + 0.1);
    let dir = config.join("fab-profiles");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec_pretty(&file)?,
    )?;
    Ok(())
}

fn write_fabricator(config: &Path, id: &str, name: &str) -> Fallible {
    let dir = config.join("fabricators");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec_pretty(&json!({"format": 1, "fabricator": {"id": id, "name": name}}))?,
    )?;
    Ok(())
}

#[test]
fn suggestions_check_every_installed_profile_and_prefer_the_boards_fabricator() -> Fallible {
    let dir = tempfile::tempdir()?;
    let (config, cache) = (dir.path().join("config"), dir.path().join("cache"));
    // The board's profile asks 0.25 mm of via ring; the showcase's vias
    // have less. Two other profiles allow 0.1 mm, one of each fabricator.
    write_profile(&config, "strict-a", "fab-a", 0.25)?;
    write_profile(&config, "relaxed-a", "fab-a", 0.1)?;
    write_profile(&config, "relaxed-b", "fab-b", 0.1)?;
    write_fabricator(&config, "fab-a", "Fab A")?;
    write_fabricator(&config, "fab-b", "Fab B")?;
    let board = dir.path().join("board.sparksmith");
    let board_text = board.display().to_string();
    // Made under the relaxed profile, so its vias have the 0.15 mm ring
    // that profile's default via gives; then switched to the strict one.
    run(
        &config,
        &cache,
        &["showcase", &board_text, "--profile", "relaxed-a"],
    )?;
    let replies = session(
        &config,
        &cache,
        &[
            json!({"command": "project.open", "params": {"expected_revision": 0, "path": board_text}}),
            json!({"command": "project.apply", "params": {"expected_revision": 2, "operation": {
                "editor": "Rules", "operation": {"command": "rules.set_fab_profile",
                    "params": {"profile": "strict-a"}}}}}),
            json!({"command": "project.query", "params": {
                "query": "fabrication.profile_suggestions", "params": {}}}),
            json!({"command": "project.query", "params": {
                "query": "fabrication.preflight", "params": {}}}),
        ],
    )?;
    let [opened, switched, suggested, preflight] = replies.as_slice() else {
        return Err(format!("four replies expected: {replies:?}").into());
    };
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(switched["ok"], true, "{switched}");
    assert_eq!(suggested["ok"], true, "{suggested}");
    let result = &suggested["result"]["result"];
    assert_eq!(result["profile"], "Test strict-a");
    assert_eq!(result["fabricator"], "fab-a");
    let needs = result["needs"].as_array().ok_or("needs")?;
    let ring = needs
        .iter()
        .find(|n| n["requirement"]["rule"] == "via_annular_ring_min")
        .ok_or_else(|| format!("a via ring need: {needs:?}"))?;
    assert_eq!(ring["requirement"]["kind"], "at_most");
    assert_eq!(ring["limit"], "0.25 mm");
    assert!(
        ring["text"]
            .as_str()
            .is_some_and(|t| t.starts_with("Needs min via annular ring ≤ ")),
        "{ring}"
    );
    let suggestions = result["suggestions"].as_array().ok_or("suggestions")?;
    let ids: Vec<&str> = suggestions
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(ids.first(), Some(&"relaxed-a"), "{ids:?}");
    assert_eq!(suggestions[0]["same_fabricator"], true);
    assert_eq!(suggestions[0]["fabricator_name"], "Fab A");
    let b = suggestions
        .iter()
        .find(|s| s["id"] == "relaxed-b")
        .ok_or_else(|| format!("relaxed-b suggested: {ids:?}"))?;
    assert_eq!(b["fabricator_name"], "Fab B");
    assert!(b.get("same_fabricator").is_none_or(|v| v == false));
    assert!(
        !ids.contains(&"strict-a"),
        "the board's own profile: {ids:?}"
    );
    // Every profile of the board's fabricator comes before any other.
    let first_other = suggestions
        .iter()
        .position(|s| s["same_fabricator"] != true)
        .unwrap_or(suggestions.len());
    assert!(
        suggestions[first_other..]
            .iter()
            .all(|s| s["same_fabricator"] != true)
    );
    // The production check carries the same need on each via finding.
    let items = preflight["result"]["result"]["items"]
        .as_array()
        .ok_or("items")?;
    assert!(
        items
            .iter()
            .filter(|i| i["code"] == "board.via_annular_ring")
            .all(|i| i["needs"]["requirement"]["rule"] == "via_annular_ring_min"),
        "{items:?}"
    );
    assert!(items.iter().any(|i| i["code"] == "board.via_annular_ring"));
    Ok(())
}

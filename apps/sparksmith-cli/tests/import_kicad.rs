//! `sparksmith import-kicad` on an original synthetic board written here
//! (no KiCad fixture files): the outline imports as a board sketch with
//! its cutout, nets, tracks, vias and a zone come in, and the project is
//! saved with the report's detail beside it.
use serde_json::Value;
use std::process::Command;

type Fallible = Result<(), Box<dyn std::error::Error>>;

/// A 30 × 20 mm two-layer board with a rounded corner, a round cutout, two
/// resistors joined by a track, a via and a bottom track, and a ground
/// zone on both sides.
const BOARD: &str = r#"(kicad_pcb (version 20240108) (generator "sparksmith_test") (generator_version "8.0")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (34 "B.Paste" user) (35 "F.Paste" user)
    (38 "B.Mask" user) (39 "F.Mask" user) (44 "Edge.Cuts" user))
  (setup (pad_to_mask_clearance 0.05))
  (net 0 "") (net 1 "SIG") (net 2 "GND")
  (footprint "Synthetic:R" (layer "F.Cu") (at 105 60)
    (property "Reference" "R1" (at 0 -2 0) (layer "F.SilkS"))
    (pad "1" smd rect (at -1 0) (size 0.8 0.6) (layers "F.Cu" "F.Paste" "F.Mask") (net 2 "GND"))
    (pad "2" smd rect (at 1 0) (size 0.8 0.6) (layers "F.Cu" "F.Paste" "F.Mask") (net 1 "SIG")))
  (footprint "Synthetic:R" (layer "F.Cu") (at 125 60)
    (property "Reference" "R2" (at 0 -2 0) (layer "F.SilkS"))
    (pad "1" smd rect (at -1 0) (size 0.8 0.6) (layers "F.Cu" "F.Paste" "F.Mask") (net 1 "SIG"))
    (pad "2" smd rect (at 1 0) (size 0.8 0.6) (layers "F.Cu" "F.Paste" "F.Mask") (net 2 "GND")))
  (gr_line (start 102 70) (end 130 70) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_line (start 130 70) (end 130 50) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_line (start 130 50) (end 100 50) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_line (start 100 50) (end 100 68) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_arc (start 100 68) (mid 100.585786 69.414214) (end 102 70) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_circle (center 115 65) (end 116.5 65) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (segment (start 106 60) (end 115 60) (width 0.25) (layer "F.Cu") (net 1))
  (via (at 115 60) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
  (segment (start 115 60) (end 124 60) (width 0.25) (layer "B.Cu") (net 1))
  (zone (net 2) (net_name "GND") (layers "F&B.Cu") (hatch edge 0.5)
    (connect_pads yes (clearance 0.3)) (min_thickness 0.25)
    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5))
    (polygon (pts (xy 101 51) (xy 129 51) (xy 129 55) (xy 101 55))))
)
"#;

fn count(rows: &Value, case: &str) -> u64 {
    rows.as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["case"] == case)
        .filter_map(|r| r["count"].as_u64())
        .sum()
}

#[test]
fn a_board_imports_with_its_outline_sketch_copper_and_zone() -> Fallible {
    let dir = tempfile::tempdir()?;
    let config = tempfile::tempdir()?;
    let board = dir.path().join("synthetic.kicad_pcb");
    std::fs::write(&board, BOARD)?;
    let target = dir.path().join("imported.sparksmith");
    let out = Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", config.path())
        .env("SPARKSMITH_CACHE_DIR", config.path())
        // A folder with one board names it.
        .args(["import-kicad"])
        .arg(dir.path())
        .arg(&target)
        .output()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reply: Value = serde_json::from_slice(&out.stdout)?;
    let rows = &reply["rows"];
    assert_eq!(count(rows, "outline_sketch"), 1, "{reply:#}");
    assert_eq!(reply["counts"]["cutouts"], 1);
    assert_eq!(reply["counts"]["nets"], 2);
    assert_eq!(reply["counts"]["tracks"], 2);
    assert_eq!(reply["counts"]["vias"], 1);
    assert_eq!(reply["counts"]["fills"], 2);
    assert_eq!(count(rows, "track_refused") + count(rows, "via_refused"), 0);
    assert!(target.is_file());
    // The detail stays beside the new file.
    assert!(dir.path().join("imported.kicad-import.json").is_file());
    Ok(())
}

/// A `.kicad_pro` names its board; a rule looser than the fab profile is
/// named with both values in the summary, the reply and the detail file.
#[test]
fn a_project_file_imports_its_board_and_names_the_rules_not_taken() -> Fallible {
    let dir = tempfile::tempdir()?;
    let config = tempfile::tempdir()?;
    std::fs::write(dir.path().join("synthetic.kicad_pcb"), BOARD)?;
    let project = dir.path().join("synthetic.kicad_pro");
    std::fs::write(
        &project,
        br#"{"board":{"design_settings":{"rules":{"min_through_hole_diameter":0.01}}}}"#,
    )?;
    let target = dir.path().join("imported.sparksmith");
    let out = Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", config.path())
        .env("SPARKSMITH_CACHE_DIR", config.path())
        .arg("import-kicad")
        .arg(&project)
        .arg(&target)
        .output()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reply: Value = serde_json::from_slice(&out.stdout)?;
    assert_eq!(
        count(&reply["rows"], "rule_looser_than_fab"),
        1,
        "{reply:#}"
    );
    assert_eq!(reply["looser_rules"][0]["rule"], "min drill", "{reply:#}");
    assert_eq!(reply["looser_rules"][0]["kicad_mm"], 0.01);
    let summary = reply["summary"].as_str().unwrap_or_default();
    assert!(
        summary.contains("    min drill: KiCad 0.01 mm, fab profile "),
        "{summary}"
    );
    let detail: Value = serde_json::from_slice(&std::fs::read(
        dir.path().join("imported.kicad-import.json"),
    )?)?;
    assert_eq!(detail["looser_rules"][0]["rule"], "min drill");
    Ok(())
}

fn import_preserves_existing(extension: &str) -> Fallible {
    let dir = tempfile::tempdir()?;
    let config = tempfile::tempdir()?;
    let board = dir.path().join("synthetic.kicad_pcb");
    std::fs::write(&board, BOARD)?;
    let target = dir.path().join("imported.sparksmith");
    let occupied = target.with_extension(extension);
    std::fs::write(&occupied, b"existing user data")?;
    let output = Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .env("SPARKSMITH_CONFIG_DIR", config.path())
        .env("SPARKSMITH_CACHE_DIR", config.path())
        .arg("import-kicad")
        .arg(&board)
        .arg(&target)
        .output()?;
    assert!(
        std::fs::read(&occupied)? == b"existing user data",
        "existing {extension} bytes were replaced"
    );
    assert!(
        !output.status.success(),
        "an existing {extension} must be refused"
    );
    assert_eq!(
        std::fs::read_dir(dir.path())?.count(),
        2,
        "preflight refusal writes neither output"
    );
    Ok(())
}

#[test]
fn import_refuses_existing_project_without_replacing_it() -> Fallible {
    import_preserves_existing("sparksmith")
}

#[test]
fn import_refuses_existing_report_without_replacing_it() -> Fallible {
    import_preserves_existing("kicad-import.json")
}

#[test]
fn competing_imports_publish_one_project_and_report() -> Fallible {
    let dir = tempfile::tempdir()?;
    let board = dir.path().join("synthetic.kicad_pcb");
    std::fs::write(&board, BOARD)?;
    let target = dir.path().join("imported.sparksmith");
    let configurations = [tempfile::tempdir()?, tempfile::tempdir()?];
    let mut children = Vec::new();
    for config in &configurations {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_sparksmith"))
                .env("SPARKSMITH_CONFIG_DIR", config.path())
                .env("SPARKSMITH_CACHE_DIR", config.path())
                .arg("import-kicad")
                .arg(&board)
                .arg(&target)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?,
        );
    }
    let outputs = children
        .into_iter()
        .map(|child| child.wait_with_output())
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1,
        "{outputs:?}"
    );
    let winner = outputs
        .iter()
        .find(|output| output.status.success())
        .ok_or("no winner")?;
    let reply: Value = serde_json::from_slice(&winner.stdout)?;
    assert_eq!(
        reply["saved"]["saved_path"],
        target.to_string_lossy().as_ref()
    );
    sparksmith_io::read_project(&target)?;
    let report: Value =
        serde_json::from_slice(&std::fs::read(target.with_extension("kicad-import.json"))?)?;
    assert!(report.is_object());
    assert_eq!(
        std::fs::read_dir(dir.path())?.count(),
        3,
        "no staging files remain"
    );
    Ok(())
}

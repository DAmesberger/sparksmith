//! `import-kicad DIR|FILE.kicad_pcb|FILE.kicad_pro NEW_FILE.sparksmith [--profile ID]`:
//! import a KiCad board (footprints and pads, outline, stackup, nets,
//! tracks, vias and zones) through the one command engine, on the fab
//! profile library's profile (else the default) with SolidCraft's sketch
//! engines, and save it. The import report's counts are printed; its full
//! detail (designators, pads, net names) goes beside the new file as
//! `NEW_FILE.kicad-import.json`, for the user only.
use super::{Result, execute, print};
use serde_json::json;
use sparksmith_automation::Session;
use sparksmith_automation::kicad_import::{self, KicadImport};
use sparksmith_core::product::SHORT_NAME as CLI;
use sparksmith_io::persistence::{StagedNew, check_project_path};
use std::path::Path;

/// The usage line(s).
pub const USAGE: &str =
    "import-kicad DIR|FILE.kicad_pcb|FILE.kicad_pro NEW_FILE.sparksmith [--profile ID]
                                   import a KiCad board through the command engine
                                   (footprints, outline, stackup, nets, tracks, vias,
                                   zones filled by our engine) and save it; DIR names
                                   the folder of one board; the report's counts are
                                   printed, its detail written beside the new file";

/// Run the subcommand with the arguments after its name.
pub fn run(args: &[String]) -> Result<()> {
    let (source, target, choice) = match args {
        [source, target] => (source, target, None),
        [source, target, flag, id] if flag == "--profile" => (source, target, Some(id.as_str())),
        _ => return Err(format!("usage: {CLI} {USAGE}").into()),
    };
    let target = Path::new(target);
    check_project_path(target)?;
    let project_stage = StagedNew::new(target)?;
    let detail = target.with_extension("kicad-import.json");
    let report_stage = StagedNew::new(&detail)?;
    // The one import, the desktop app's too.
    let KicadImport { engine, report, .. } = kicad_import::import(Path::new(source), choice)?;
    let mut session = Session::new(engine);
    // Save through the same front door to our owned staging path. Only
    // the IO publication step may claim the user's new destination.
    let mut saved = execute(
        &mut session,
        json!({"command": "project.save", "params": {"path": project_stage.path()}}),
    )?;
    std::fs::write(
        report_stage.path(),
        serde_json::to_vec_pretty(&report.to_json())?,
    )?;
    project_stage.persist()?;
    report_stage.persist()?;
    saved["saved_path"] = json!(target);
    print(&json!({
        "saved": saved,
        "summary": report.summary_named(),
        "looser_rules": report.looser_rules(),
        "refused_patterns": report.refused_patterns(),
        "counts": report.counts,
        "rows": report.rows(),
        "placeholders": report.placeholders(),
        "detail": detail,
    }))
}

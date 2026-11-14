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
use std::path::Path;

/// The usage line(s).
pub const USAGE: &str = "import-kicad DIR|FILE.kicad_pcb|FILE.kicad_pro NEW_FILE.sparksmith [--profile ID]
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
    // The one import, the desktop app's too.
    let KicadImport { engine, report, .. } = kicad_import::import(Path::new(source), choice)?;
    let mut session = Session::new(engine);
    let saved = execute(
        &mut session,
        json!({"command": "project.save", "params": {"path": target}}),
    )?;
    let detail = Path::new(target).with_extension("kicad-import.json");
    std::fs::write(&detail, serde_json::to_vec_pretty(&report.to_json())?)?;
    print(&json!({
        "saved": saved,
        "summary": report.summary_named(),
        "looser_rules": report.looser_rules(),
        "counts": report.counts,
        "rows": report.rows(),
        "detail": detail,
    }))
}

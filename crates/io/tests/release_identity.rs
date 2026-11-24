//! The release identity covers every delivered file, the drawings
//! included (re-review of 22 November 2026, §5): two exports whose only
//! difference is what a drawing prints (a waiver's reason in the notes, a
//! profile's name in the title block) are two releases, and a
//! drawings-only export identifies its drawings. The PDF's title block
//! states the revision the job file states.
use sparksmith_core::fabrication::{FabricationOptions, Outputs};
use sparksmith_core::model::production::Waiver;
use sparksmith_core::rules::{RuleProfile, RulesCommand, fixtures};
use sparksmith_core::{Engine, Operation};
use sparksmith_io::fabrication::{ExportRequest, export};
use std::path::Path;

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ID: &str = "3b1f0c2e-9d84-8a6b-b7c5-0e2d4f6a8c13";

fn showcase() -> Fallible<sparksmith_core::Project> {
    let mut project = Engine::showcase()?.project().clone();
    project.production.project_id = Some(ID.into());
    Ok(project)
}

/// Export `project` under `profile` to `dir/out` and return the revision
/// the job file states, or (no job file) the one the fabrication
/// drawing's title block states, with the drawing's text.
fn release(
    project: &sparksmith_core::Project,
    profile: Option<RuleProfile>,
    outputs: Outputs,
    dir: &Path,
    out: &str,
) -> Fallible<(String, String)> {
    let output = dir.join(out);
    let report = export(
        project,
        &Default::default(),
        &ExportRequest {
            output: output.clone(),
            zip: false,
            profile,
            options: FabricationOptions {
                base_name: Some("b".into()),
                outputs,
                ..Default::default()
            },
        },
        &Default::default(),
        &mut |_| std::ops::ControlFlow::Continue(()),
    )?;
    assert!(report.preflight.ready(), "{:#?}", report.preflight.items);
    let drawing =
        String::from_utf8_lossy(&std::fs::read(output.join("b-fabrication.pdf"))?).into_owned();
    // The title block's Release row: the drawing states its revision.
    let stated = drawing
        .lines()
        .filter_map(|l| l.strip_prefix('(').and_then(|r| r.strip_suffix(") Tj")))
        .find(|t| t.len() == 16 && t.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
        .ok_or("the drawing states no release")?;
    if let Ok(job) = std::fs::read_to_string(output.join("b.gbrjob")) {
        let json: serde_json::Value = serde_json::from_str(&job)?;
        let revision = json["GeneralSpecs"]["ProjectId"]["Revision"]
            .as_str()
            .ok_or("no job revision")?
            .to_owned();
        assert_eq!(
            stated, revision,
            "the drawing and the job state one release"
        );
    }
    Ok((stated, drawing))
}

/// Every string a PDF shows (`(…) Tj` and `<hex> Tj`), its WinAnsi bytes
/// read as Latin-1 (every character the drawings print but the dash).
fn shown(pdf: &str) -> Vec<String> {
    pdf.lines()
        .filter_map(|l| l.strip_suffix(" Tj"))
        .map(|t| {
            if let Some(hex) = t.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
                (0..hex.len() / 2)
                    .filter_map(|k| u8::from_str_radix(hex.get(2 * k..2 * k + 2)?, 16).ok())
                    .map(char::from)
                    .collect()
            } else {
                t.trim_start_matches('(').trim_end_matches(')').to_owned()
            }
        })
        .collect()
}

fn drawings_only() -> Outputs {
    Outputs {
        gerber: false,
        drill: false,
        assembly: false,
        drawings: true,
    }
}

#[test]
fn a_change_only_the_drawings_print_is_another_release() -> Fallible<()> {
    let dir = tempfile::tempdir()?;
    let project = showcase()?;
    let (base, _) = release(&project, None, Outputs::default(), dir.path(), "base")?;
    let (again, _) = release(&project, None, Outputs::default(), dir.path(), "again")?;
    assert_eq!(base, again, "the same design, the same release");

    // A waiver of a finding the board does not have: no file but the
    // drawing's notes says it.
    let mut waived = project.clone();
    waived.production.waivers.push(Waiver {
        key: "board.acute_copper [{\"kind\":\"board\"}]".into(),
        reason: "Fab-confirmed".into(),
        accepted: Vec::new(),
        footprints: Vec::new(),
    });
    let (reasoned, drawing) = release(&waived, None, Outputs::default(), dir.path(), "waived")?;
    assert!(drawing.contains("Fab-confirmed"), "the note is printed");
    assert_ne!(
        reasoned, base,
        "a waiver-only drawing change is another release"
    );

    // Another profile that differs only in its name (its fills refilled
    // under it, as an export under a profile needs).
    let mut renamed = fixtures::prototype()?;
    renamed.name = "Renamed-prototype".into();
    let mut engine = Engine::showcase()?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(renamed, true)),
    )?;
    engine.refill_stale()?;
    let mut named_project = engine.project().clone();
    named_project.production.project_id = Some(ID.into());
    let (named, drawing) = release(
        &named_project,
        None,
        Outputs::default(),
        dir.path(),
        "named",
    )?;
    assert!(drawing.contains("Renamed-prototype"));
    assert_ne!(named, base, "a profile name only the drawing prints");

    // Drawings alone: each set of drawings is its own release.
    let (a, _) = release(&project, None, drawings_only(), dir.path(), "only-a")?;
    let (b, _) = release(&waived, None, drawings_only(), dir.path(), "only-b")?;
    assert_ne!(a, b, "two drawing-only exports of different drawings");
    Ok(())
}

/// A 24-layer board's fabrication drawing prints every stackup row and
/// every note (re-review §6: 51 rows and 11 notes, 11 rows and every note
/// cut from one sheet), on numbered sheets, the files written to disk.
#[test]
fn a_24_layer_drawing_prints_every_stackup_row_and_note() -> Fallible<()> {
    use sparksmith_core::fabrication::{ProductionSnapshot, drawing};
    use sparksmith_core::layout::LayoutCommand;
    let mut engine = Engine::default();
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(fixtures::prototype()?, false)),
    )?;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::SetStackup(
            sparksmith_core::model::Stackup::preset(24)?,
        )),
    )?;
    let mut project = engine.project().clone();
    project.production.project_id = Some(ID.into());
    let data = drawing::data(
        &ProductionSnapshot::resolve(&project, None)?,
        &Default::default(),
        &Default::default(),
    )?;
    assert!(data.stackup.len() > 40, "{} rows", data.stackup.len());
    let dir = tempfile::tempdir()?;
    let (_, pdf) = release(&project, None, Outputs::default(), dir.path(), "l24")?;
    // For visual review when `SPARKSMITH_DRAWINGS_DIR` is set.
    if let Ok(out) = std::env::var("SPARKSMITH_DRAWINGS_DIR") {
        std::fs::create_dir_all(&out)?;
        std::fs::copy(
            dir.path().join("l24/b-fabrication.pdf"),
            Path::new(&out).join("fabrication-24-layer.pdf"),
        )?;
    }
    let printed = shown(&pdf);
    for row in &data.stackup {
        assert!(printed.contains(&row.name), "{} is printed", row.name);
    }
    for note in &data.notes {
        let last = note.split_whitespace().last().ok_or("an empty note")?;
        assert!(
            printed.iter().any(|t| t.ends_with(last)),
            "the note ending {last} is printed"
        );
    }
    assert!(
        pdf.contains("Fabrication drawing, sheet 2 of 2"),
        "two sheets"
    );
    assert!(!pdf.contains("more rows"), "no row is summarised away");
    Ok(())
}

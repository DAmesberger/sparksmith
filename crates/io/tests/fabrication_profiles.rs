//! One production snapshot feeds the whole fabrication job: an export
//! under a fab profile other than the board's own writes every file under
//! that profile, not only the check. Two profiles that differ in solder mask
//! expansion, the plated-hole convention of the drill files and the
//! minimum track width are exported from one board, and the files on disk
//! are read back: the job file's minimum width, the plated drill tools and
//! the mask openings, and the fabrication drawing's profile.
use sparksmith_core::Engine;
use sparksmith_core::fabrication::{FabricationCommand, FabricationOptions, preflight};
use sparksmith_core::rules::{DrillFiles, RuleEntry, RuleKey, RuleProfile, RuleValue, fixtures};
use sparksmith_io::fabrication::{ExportRequest, export};
use std::collections::BTreeMap;
use std::path::Path;

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ID: &str = "3b1f0c2e-9d84-8a6b-b7c5-0e2d4f6a8c13";

/// The board's own profile: the test prototype (mask expansion 0.05 mm,
/// minimum track 0.05 mm, drill files at finished sizes).
fn own() -> Fallible<RuleProfile> {
    let p = fixtures::prototype()?;
    assert_eq!(p.rules().mask.expansion, 0.05);
    assert_eq!(p.rules().copper.track_width_min, 0.05);
    assert!(matches!(p.drill_files, DrillFiles::Finished { .. }));
    Ok(p)
}

/// Another fabricator: mask expansion 0.1 mm, minimum track 0.15 mm, and
/// drill files that list plated holes 0.1 mm over their finished size.
fn other() -> Fallible<RuleProfile> {
    let mut p = fixtures::prototype()?;
    p.id = "test-other".into();
    p.name = "Test other fabricator".into();
    p.drill_files = DrillFiles::Drilled {
        pth_allowance: 0.1,
        source: "Test fixture.".into(),
    };
    p.set(
        RuleKey::MaskExpansion,
        RuleEntry::value(RuleValue::Length(0.1), "Test fixture."),
    )?;
    p.set(
        RuleKey::TrackWidthMin,
        RuleEntry::value(RuleValue::Length(0.15), "Test fixture."),
    )?;
    Ok(p)
}

fn read(dir: &Path, name: &str) -> Fallible<String> {
    Ok(std::fs::read_to_string(dir.join(name))?)
}

/// Every `TnCd` tool of a drill file: tool number → diameter.
fn tools(text: &str) -> BTreeMap<String, f64> {
    text.lines()
        .filter_map(|l| {
            let (tool, d) = l.strip_prefix('T')?.split_once('C')?;
            Some((tool.to_owned(), d.parse().ok()?))
        })
        .collect()
}

/// Every standard aperture of a Gerber file: D-code → (template, sizes).
fn apertures(text: &str) -> BTreeMap<String, (String, Vec<f64>)> {
    text.lines()
        .filter_map(|l| {
            let body = l.strip_prefix("%ADD")?.strip_suffix("*%")?;
            let split = body.find(|c: char| !c.is_ascii_digit())?;
            let (code, rest) = body.split_at(split);
            let (template, sizes) = rest.split_once(',')?;
            if !matches!(template, "C" | "R" | "O") {
                return None;
            }
            let sizes = sizes
                .split('X')
                .map(str::parse)
                .collect::<Result<Vec<f64>, _>>()
                .ok()?;
            Some((code.to_owned(), (template.to_owned(), sizes)))
        })
        .collect()
}

#[test]
fn every_file_of_an_export_follows_the_chosen_profile() -> Fallible<()> {
    let mut project = Engine::showcase()?.project().clone();
    project.production.project_id = Some(ID.into());
    assert_eq!(project.rules.profile.as_ref(), Some(&own()?));
    let other = other()?;
    let checks = Default::default();
    // The showcase's fills were computed under its own rules, so under the
    // other profile they are stale, which cannot be waived: for that export
    // they are computed under it, as a board switched to it refills them.
    let mut poured = project.clone();
    let under = sparksmith_core::fabrication::ProductionSnapshot::resolve(&project, Some(&other))?
        .project()
        .clone();
    for (id, copper) in sparksmith_core::layout::fill::refill(
        &under,
        sparksmith_core::layout::fill::RefillTarget::All,
    )? {
        if let Some(fill) = poured.fills.iter_mut().find(|f| f.id == id) {
            fill.computed = Some(copper);
        }
    }
    // What else blocks under it is waived, so the export runs.
    let blocking: Vec<String> =
        preflight::report(&poured, &Default::default(), Some(&other), &checks)?
            .items
            .into_iter()
            .filter(|i| i.blocks())
            .map(|i| i.key)
            .collect();
    if !blocking.is_empty() {
        let accepted = preflight::accepted(
            sparksmith_core::fabrication::ProductionSnapshot::resolve(&poured, Some(&other))?
                .project(),
            &Default::default(),
            &checks,
            &blocking,
        )?;
        preflight::apply(
            &mut poured.production,
            FabricationCommand::Waive {
                keys: blocking,
                reason: "test".into(),
                profile: None,
            },
            false,
            &accepted,
        )?;
    }
    let dir = tempfile::tempdir()?;
    let run = |profile: Option<RuleProfile>, out: &str| -> Fallible<std::path::PathBuf> {
        let output = dir.path().join(out);
        let report = export(
            if profile.is_some() { &poured } else { &project },
            &Default::default(),
            &ExportRequest {
                output: output.clone(),
                zip: false,
                profile,
                options: FabricationOptions {
                    base_name: Some("b".into()),
                    ..Default::default()
                },
            },
            &checks,
            &mut |_| std::ops::ControlFlow::Continue(()),
        )?;
        assert_eq!(report.preflight.blocking, 0);
        Ok(output)
    };
    let a = run(None, "own")?;
    let b = run(Some(other.clone()), "other")?;

    // The job file states the rules the files were made under.
    let job = |dir: &Path| -> Fallible<f64> {
        let json: serde_json::Value = serde_json::from_str(&read(dir, "b.gbrjob")?)?;
        json["DesignRules"][0]["MinLineWidth"]
            .as_f64()
            .ok_or_else(|| "no MinLineWidth".into())
    };
    assert_eq!(job(&a)?, 0.05);
    assert_eq!(
        job(&b)?,
        0.15,
        "the job file names the chosen profile's width"
    );

    // Plated holes are listed 0.1 mm over their finished size, unplated
    // holes at it.
    let (plated_a, plated_b) = (
        tools(&read(&a, "b-plated.drl")?),
        tools(&read(&b, "b-plated.drl")?),
    );
    assert!(!plated_a.is_empty());
    assert_eq!(plated_a.len(), plated_b.len());
    for (tool, d) in &plated_a {
        let e = plated_b.get(tool).copied().unwrap_or(f64::NAN);
        assert!((e - (d + 0.1)).abs() < 1e-9, "T{tool}: {d} → {e}");
    }
    if a.join("b-non-plated.drl").exists() {
        assert_eq!(
            tools(&read(&a, "b-non-plated.drl")?),
            tools(&read(&b, "b-non-plated.drl")?)
        );
    }

    // Every inherited mask opening grows by 0.05 mm on each edge: 0.1 mm
    // across. The showcase's pads and open vias all inherit the expansion.
    let (mask_a, mask_b) = (
        apertures(&read(&a, "b-mask-top.gbr")?),
        apertures(&read(&b, "b-mask-top.gbr")?),
    );
    assert!(mask_a.len() >= 2, "{mask_a:?}");
    assert_eq!(mask_a.len(), mask_b.len());
    for (code, (template, sizes)) in &mask_a {
        let (other_template, grown) = mask_b.get(code).ok_or("aperture missing")?;
        assert_eq!(template, other_template, "D{code}");
        for (s, g) in sizes.iter().zip(grown) {
            assert!((g - (s + 0.1)).abs() < 1e-9, "D{code}: {s} → {g}");
        }
    }

    // One project, two releases: every file and both drawings state the
    // project's identity and their own set's revision.
    let release = |dir: &Path| -> Fallible<String> {
        let json: serde_json::Value = serde_json::from_str(&read(dir, "b.gbrjob")?)?;
        let id = &json["GeneralSpecs"]["ProjectId"];
        assert_eq!(id["GUID"], ID);
        let revision = id["Revision"].as_str().ok_or("no revision")?.to_owned();
        let stated = format!("{ID},{revision}");
        for name in ["b-copper-l1-top.gbr", "b-mask-top.gbr", "b-plated.drl"] {
            assert!(read(dir, name)?.contains(&stated), "{name}");
        }
        for pdf in ["b-fabrication.pdf", "b-assembly.pdf"] {
            let text = String::from_utf8_lossy(&std::fs::read(dir.join(pdf))?).into_owned();
            assert!(text.contains(&revision) && text.contains(ID), "{pdf}");
        }
        Ok(revision)
    };
    assert_ne!(release(&a)?, release(&b)?);

    // The fabrication drawing names the profile the files were made under
    // and its drill-file convention.
    let pdf = String::from_utf8_lossy(&std::fs::read(b.join("b-fabrication.pdf"))?).into_owned();
    assert!(
        pdf.contains("Test other fabricator"),
        "the drawing's profile"
    );
    let own_pdf =
        String::from_utf8_lossy(&std::fs::read(a.join("b-fabrication.pdf"))?).into_owned();
    assert!(own_pdf.contains("Test prototype"));
    Ok(())
}

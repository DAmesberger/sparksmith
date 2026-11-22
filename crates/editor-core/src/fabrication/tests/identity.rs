//! The release identity on every manufacturing file: the project's stored
//! identity, not one derived from its editable name, and a revision that
//! names the content of the set, the same in every file of it.
use super::Fallible;
use crate::Engine;
use crate::fabrication::{self, FabricationOptions, FabricationSet, ProductionSnapshot};
use crate::model::{PadFinish, Project};
use std::path::Path;

fn set(project: &Project) -> Fallible<FabricationSet> {
    Ok(fabrication::generate(
        &ProductionSnapshot::resolve(project, None)?,
        &FabricationOptions::default(),
    )?)
}

/// `(name, guid, revision)` as each Gerber, drill and job file states it.
fn stated(set: &FabricationSet) -> Fallible<Vec<(String, [String; 3])>> {
    let mut out = Vec::new();
    for file in &set.files {
        let text = String::from_utf8_lossy(&file.bytes);
        let found = if file.name.ends_with(".gbrjob") {
            let json: serde_json::Value = serde_json::from_str(&text)?;
            let id = &json["GeneralSpecs"]["ProjectId"];
            Some(
                [&id["Name"], &id["GUID"], &id["Revision"]]
                    .map(|v| v.as_str().map(str::to_owned).unwrap_or_default()),
            )
        } else {
            text.lines().find_map(|l| {
                let rest = l
                    .strip_prefix("%TF.ProjectId,")
                    .and_then(|r| r.strip_suffix("*%"))
                    .or_else(|| l.strip_prefix("; #@! TF.ProjectId,"))?;
                let mut parts = rest.rsplitn(3, ',');
                let revision = parts.next()?.to_owned();
                let guid = parts.next()?.to_owned();
                let name = parts.next()?.to_owned();
                Some([name, guid, revision])
            })
        };
        if let Some(id) = found {
            out.push((file.name.clone(), id));
        }
    }
    Ok(out)
}

/// The one identity every file of `set` states.
fn identity(set: &FabricationSet) -> Fallible<[String; 3]> {
    let all = stated(set)?;
    let first = all.first().map(|(_, id)| id.clone()).ok_or("no identity")?;
    for (name, id) in &all {
        assert_eq!(id, &first, "{name} states another identity");
    }
    let gerbers = all.iter().filter(|(n, _)| n.ends_with(".gbr")).count();
    let drills = all.iter().filter(|(n, _)| n.ends_with(".drl")).count();
    let jobs = all.iter().filter(|(n, _)| n.ends_with(".gbrjob")).count();
    assert!(gerbers >= 8 && drills >= 1 && jobs == 1, "{all:?}");
    assert_eq!(
        first,
        [
            set.identity.name.clone(),
            set.identity.guid.clone(),
            set.identity.revision.clone()
        ]
    );
    Ok(first)
}

fn with_id(mut project: Project, id: &str) -> Project {
    project.production.project_id = Some(id.into());
    project
}

const A: &str = "0f3c2a10-5b7e-8d21-9a44-61c0de5f7b12";
const B: &str = "7d10e2b4-0c3a-8f55-a1b2-3c4d5e6f7081";

#[test]
fn boards_of_one_name_differ_and_a_rename_keeps_the_identity() -> Fallible<()> {
    let showcase = Engine::showcase()?.project().clone();
    let a = identity(&set(&with_id(showcase.clone(), A))?)?;
    let b = identity(&set(&with_id(showcase.clone(), B))?)?;
    assert_eq!(a[0], b[0], "one name");
    assert_eq!(a[1], A);
    assert_eq!(b[1], B, "two boards of one name are two projects");

    let mut renamed = with_id(showcase, A);
    renamed.name = "Renamed board".into();
    let r = identity(&set(&renamed)?)?;
    assert_eq!(r[0], "Renamed board");
    assert_eq!(r[1], A, "a rename keeps the identity");
    Ok(())
}

#[test]
fn the_revision_names_what_the_files_say() -> Fallible<()> {
    let project = with_id(Engine::showcase()?.project().clone(), A);
    let first = identity(&set(&project)?)?;
    assert_eq!(first[2].len(), 16);
    assert!(first[2].bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(first[2], "1");
    assert_eq!(
        identity(&set(&project)?)?,
        first,
        "the same design, the same release"
    );

    // A different finish changes the job file, so the release.
    let mut finished = project.clone();
    finished.fabrication.pad_finish = PadFinish::Hasl;
    let changed = identity(&set(&finished)?)?;
    assert_eq!(changed[1], first[1]);
    assert_ne!(changed[2], first[2]);

    // The creation date is not content: a dated set is the same release.
    let dated = fabrication::generate(
        &ProductionSnapshot::resolve(&project, None)?,
        &FabricationOptions {
            creation_date: Some("2026-11-22T10:00:00Z".into()),
            ..Default::default()
        },
    )?;
    assert_eq!(identity(&dated)?, first);
    Ok(())
}

/// A project first saved takes its identity from its file's name, the
/// host's nonce and what it holds; undo, redo and renames keep it, and a
/// scripted project saved under the same file name reproduces it.
#[test]
fn a_first_save_gives_the_identity_and_undo_keeps_it() -> Fallible<()> {
    let mut engine = Engine::showcase()?;
    assert_eq!(engine.project().production.project_id, None);
    let file = Path::new("boards/one.sparksmith");
    let mut again = Engine::showcase()?;
    again.identify_by_file(file, "");
    let mut elsewhere = Engine::showcase()?;
    elsewhere.identify_by_file(Path::new("boards/two.sparksmith"), "");

    let revision = engine.revision();
    engine.apply(
        revision,
        crate::Operation::Schematic(crate::schematic::SchematicCommand::RenameProject {
            name: "Before the save".into(),
        }),
    )?;
    engine.identify_by_file(file, "");
    let id = engine
        .project()
        .production
        .project_id
        .clone()
        .ok_or("no identity")?;
    assert!(crate::model::production::is_guid(&id), "{id}");
    assert_ne!(Some(&id), again.project().production.project_id.as_ref());
    assert_ne!(
        again.project().production.project_id,
        elsewhere.project().production.project_id,
        "two files, two projects"
    );
    let mut nonced = Engine::showcase()?;
    nonced.identify_by_file(file, "0123abcd");
    assert_ne!(
        nonced.project().production.project_id,
        again.project().production.project_id,
        "a host's nonce makes two new projects of one file name two projects"
    );
    let mut replay = Engine::showcase()?;
    replay.identify_by_file(Path::new("elsewhere/one.sparksmith"), "");
    assert_eq!(
        replay.project().production.project_id,
        again.project().production.project_id,
        "the same project saved under the same file name reproduces its identity"
    );
    assert_eq!(engine.revision(), revision + 1, "no revision of its own");

    engine.undo()?;
    assert_eq!(engine.project().production.project_id.as_ref(), Some(&id));
    engine.redo()?;
    assert_eq!(engine.project().production.project_id.as_ref(), Some(&id));
    engine.identify_by_file(Path::new("copy/elsewhere.sparksmith"), "");
    assert_eq!(
        engine.project().production.project_id.as_ref(),
        Some(&id),
        "a copy keeps the identity it has"
    );
    assert!(engine.adopt_project_id("not-an-id".into()).is_err());
    Ok(())
}

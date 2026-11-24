//! The check before every export, on the boards of the other tests: each
//! passes as it is, so the check refuses no right board, and every fault
//! seeded in a set's text files is told.
use super::Fallible;
use super::proof::{self, Files};
use crate::Engine;
use crate::fabrication::readback::{self, Mismatch};
use crate::fabrication::{self, FabricationOptions, ProductionSnapshot};
use crate::model::Project;

/// What the check tells of `project`'s set, after `change`.
fn told(project: &Project, change: impl Fn(&mut Files)) -> Fallible<Vec<Mismatch>> {
    let snapshot = ProductionSnapshot::resolve(project, None)?;
    let mut options = FabricationOptions {
        base_name: Some("proof".into()),
        ..FabricationOptions::default()
    };
    // The PDFs are made by the io crate, which tests them.
    options.outputs.drawings = false;
    let set = fabrication::generate(&snapshot, &options, &Default::default())?;
    let mut files: Files = set
        .files
        .iter()
        .map(|f| Ok((f.name.clone(), String::from_utf8(f.bytes.clone())?)))
        .collect::<Fallible<_>>()?;
    change(&mut files);
    let files: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .map(|(name, text)| (name, text.into_bytes()))
        .collect();
    Ok(readback::check(
        &readback::Set {
            snapshot: &snapshot,
            options: &options,
            identity: &set.identity,
            drawing: None,
        },
        &files,
        &mut |_| true,
    )?)
}

#[test]
fn every_board_of_the_tests_is_its_own_files() -> Fallible<()> {
    let mut turned = Engine::showcase()?.project().clone();
    // Parts at angles of their own, text with them, on both sides.
    for (k, placement) in turned.placements.iter_mut().enumerate() {
        placement.rotation_deg += 7.0 * k as f64 + 0.5;
    }
    for (what, project) in [
        ("the showcase", Engine::showcase()?.project().clone()),
        ("the showcase, its parts turned", turned),
        ("the proof board", proof::board()?),
        ("the demo", Engine::demo()?.project().clone()),
    ] {
        let found = told(&project, |_| {})?;
        assert!(found.is_empty(), "{what}: {found:#?}");
    }
    Ok(())
}

/// What the writer's own code could get wrong, made wrong in the files of
/// the proof board, which has every kind of pad, slot and opening.
#[test]
fn seeded_faults_are_told_with_the_file_and_the_place() -> Fallible<()> {
    let project = proof::board()?;
    type Seed = fn(&mut Files);
    let faults: [(&str, &str, Seed); 8] = [
        ("a routed slot missing", "plated.drl", |f| {
            proof::edit(f, "plated.drl", |l| {
                let k = proof::line_numbers(&l.join("\n"), |s| s.contains("G85"));
                match k.first().copied() {
                    Some(k) => proof::without(l, k..=k),
                    None => l,
                }
            });
        }),
        ("a pour's region missing", "copper-l1-top.gbr", |f| {
            proof::edit(f, "copper-l1-top.gbr", |l| {
                match proof::regions(&l).first().cloned() {
                    Some(r) => proof::without_region(l, r),
                    None => l,
                }
            });
        }),
        ("a layer mirrored", "copper-l1-top.gbr", |f| {
            proof::edit(f, "copper-l1-top.gbr", |l| {
                l.iter()
                    .map(|s| match s.starts_with('X') {
                        true => proof::mirrored(s, 'X', 30.0, false),
                        false => s.clone(),
                    })
                    .collect()
            });
        }),
        ("the masks swapped", "mask-top.gbr", |f| {
            let at = |f: &Files, n: &str| f.iter().position(|x| x.0 == format!("proof-{n}"));
            if let (Some(i), Some(j)) = (at(f, "mask-top.gbr"), at(f, "mask-bottom.gbr")) {
                let text = f[i].1.clone();
                f[i].1 = f[j].1.clone();
                f[j].1 = text;
            }
        }),
        ("a mask opening added", "mask-top.gbr", |f| {
            proof::edit(f, "mask-top.gbr", |l| {
                l.join("\n")
                    .replacen("M02*", "X50000000Y36000000D03*\nM02*", 1)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            });
        }),
        ("every paste opening missing", "paste-top.gbr", |f| {
            f.retain(|x| x.0 != "proof-paste-top.gbr");
        }),
        ("a file that cannot be read", "profile.gbr", |f| {
            proof::edit(f, "profile.gbr", |l| {
                l.join("\n")
                    .replace("M02*", "")
                    .lines()
                    .map(str::to_owned)
                    .collect()
            });
        }),
        ("a layer listed as another", "proof.gbrjob", |f| {
            for (name, text) in f.iter_mut().filter(|x| x.0 == "proof.gbrjob") {
                assert!(text.contains("Copper,L1,Top"), "{name}");
                *text = text.replacen("Copper,L1,Top", "Copper,L2,Bot", 1);
            }
        }),
    ];
    for (what, file, seed) in faults {
        let found = told(&project, seed)?;
        assert!(
            found.iter().any(|m| m.file.ends_with(file)),
            "{what}: {found:#?}"
        );
    }
    Ok(())
}

/// A check that is told to stop tells nothing and is refused.
#[test]
fn a_stopped_check_releases_nothing() -> Fallible<()> {
    let project = Engine::demo()?.project().clone();
    let snapshot = ProductionSnapshot::resolve(&project, None)?;
    let options = FabricationOptions::default();
    let mut calls = 0;
    let stopped = readback::check(
        &readback::Set {
            snapshot: &snapshot,
            options: &options,
            identity: &Default::default(),
            drawing: None,
        },
        &[],
        &mut |_| {
            calls += 1;
            calls < 3
        },
    );
    let error = stopped.err().ok_or("the check went on")?;
    assert_eq!(error.rule(), Some(readback::STOPPED));
    Ok(())
}

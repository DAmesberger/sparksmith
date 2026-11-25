//! The check before every export, on the boards of the other tests: each
//! passes as it is, so the check refuses no right board, and every fault
//! seeded in a set's text files is told.
use super::Fallible;
use super::proof::{self, Files};
use crate::Engine;
use crate::fabrication::readback::{self, Mismatch};
use crate::fabrication::{self, FabricationOptions, ProductionSnapshot};
use crate::model::{Project, ViaFill};

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

fn swap(f: &mut Files, file: &str, from: &str, to: &str) {
    for (name, text) in f.iter_mut().filter(|x| x.0.ends_with(file)) {
        assert!(text.contains(from), "{name} has no {from:?}");
        *text = text.replacen(from, to, 1);
    }
}

/// The proof board with one of its vias filled and capped, under a profile
/// that fills it by size, with a charge that falls on it and one that does
/// not.
fn filled() -> Fallible<Project> {
    let mut project = proof::board()?;
    // The others, of the same drill, stay open.
    assert!(project.vias.len() > 1);
    if let Some(v) = project.vias.first_mut() {
        v.fill = ViaFill::Capped;
    }
    let profile = project.rules.profile.as_mut().ok_or("no profile")?;
    profile.drill_rules = serde_json::from_value(serde_json::json!([{
        "id": "through", "kind": "mechanical", "layers": {"span": "board"},
        "fills": [
            {"fill": "none"},
            {"fill": "capped", "source": "Test fixture.", "order": {
                "form": [{"label": "Covering", "choice": "Capped", "unverified": true}],
                "told_by_size": true, "asks": "Name the holes.", "source": "Test fixture."}}
        ]
    }]))?;
    profile.charges = serde_json::from_value(serde_json::json!([
        {"when": {"fill": "capped"}, "text": "Filling is charged.", "source": "Test fixture."},
        {"when": {"drill_below_mm": 0.01}, "text": "Tiny holes are charged.",
         "source": "Test fixture."}
    ]))?;
    Ok(project)
}

/// Every text file of the set.
fn written(project: &Project) -> Fallible<Files> {
    let files = std::cell::RefCell::new(Files::new());
    told(project, |f| *files.borrow_mut() = f.clone())?;
    Ok(files.into_inner())
}

/// The fabricator's stackup the layers were picked from is named in the
/// order notes, the job file and the fabrication drawing, the read-back
/// compares it, and a board whose layers are not its is not named.
#[test]
fn a_picked_stackup_is_named_where_the_fabricator_reads() -> Fallible<()> {
    let mut project = proof::board()?;
    project.stackup = project.stackup.clone().picked_from("test-4", "TEST-4-7628");
    let found = told(&project, |_| {})?;
    assert!(found.is_empty(), "{found:#?}");
    let files = written(&project)?;
    let notes = proof::file(&files, "order-notes.txt")?;
    assert!(
        notes.contains("\nStackup: TEST-4-7628 (test-4)\n"),
        "{notes}"
    );
    let job = |files: &Files| -> Fallible<String> {
        let job = files.iter().find(|f| f.0 == "proof.gbrjob");
        Ok(job.ok_or("no job file")?.1.clone())
    };
    assert!(job(&files)?.contains("stackup TEST-4-7628"));
    let snapshot = ProductionSnapshot::resolve(&project, None)?;
    let set = fabrication::generate(
        &snapshot,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    let drawing = fabrication::drawing::data(&snapshot, &set.identity, &Default::default())?;
    assert!(
        drawing
            .notes
            .iter()
            .any(|n| n == "Stackup: TEST-4-7628 (test-4)."),
        "{:?}",
        drawing.notes
    );
    for (what, file, from, to) in [
        (
            "another name",
            "order-notes.txt",
            "TEST-4-7628",
            "TEST-4-2116",
        ),
        (
            "no name",
            "order-notes.txt",
            "Stackup: TEST-4-7628 (test-4)\n",
            "",
        ),
        (
            "another name in the job file",
            "proof.gbrjob",
            "stackup TEST-4-7628",
            "stackup TEST-4-2116",
        ),
    ] {
        let found = told(&project, |f| swap(f, file, from, to))?;
        assert!(
            found.iter().any(|m| m.file.ends_with(file)),
            "{what}: {found:#?}"
        );
    }
    let found = told(&project, |f| {
        f.retain(|x| !x.0.ends_with("order-notes.txt"))
    })?;
    assert!(!found.is_empty());
    // Edited layers are not the picked ones, whoever wrote the name.
    project.stackup.dielectrics[0].thickness_mm += 0.05;
    let files = written(&project)?;
    assert!(!files.iter().any(|f| f.0.ends_with("order-notes.txt")));
    assert!(!job(&files)?.contains("TEST-4"));
    // A name that should not be there is told.
    let found = told(&project, |f| {
        f.push((
            "proof-order-notes.txt".into(),
            "ORDER NOTES\nStackup: X (x)\n".into(),
        ));
    })?;
    assert!(!found.is_empty());
    Ok(())
}

#[test]
fn a_filled_via_is_in_every_file_that_tells_the_fabricator() -> Fallible<()> {
    let project = filled()?;
    let found = told(&project, |_| {})?;
    assert!(found.is_empty(), "{found:#?}");
    let files = written(&project)?;
    assert!(proof::file(&files, "plated.drl")?.contains("TA.AperFunction,ViaDrill,VII\n"));
    assert!(proof::file(&files, "drill-table.txt")?.contains("capped"));
    let notes = proof::file(&files, "order-notes.txt")?;
    assert!(notes.contains("  Covering: Capped (form label not verified)\n"));
    assert!(notes.contains("Name the holes."));
    assert!(notes.contains("\"Filling is charged.\""));
    assert!(!notes.contains("Tiny holes"));
    let job = files.iter().find(|f| f.0 == "proof.gbrjob");
    let job: serde_json::Value = serde_json::from_str(&job.ok_or("no job file")?.1)?;
    assert_eq!(
        job["GeneralSpecs"]["ViaProtection"],
        serde_json::json!(["VII"])
    );
    // A board with nothing to order specially has none of it.
    let plain = written(&proof::board()?)?;
    assert!(!plain.iter().any(|f| f.0.ends_with("order-notes.txt")));
    assert!(!proof::file(&plain, "plated.drl")?.contains("VII"));
    assert!(!proof::file(&plain, "drill-table.txt")?.contains("Fill"));
    Ok(())
}

/// Each is red when the check it names is left out of the read-back.
#[test]
fn seeded_faults_of_filled_vias_are_told() -> Fallible<()> {
    type Seed = fn(&mut Files);
    let faults: [(&str, &str, Seed); 11] = [
        ("VII lost from the filled tool", "plated.drl", |f| {
            swap(f, "proof-plated.drl", "ViaDrill,VII", "ViaDrill");
        }),
        ("VII on the open tool", "plated.drl", |f| {
            swap(f, "proof-plated.drl", "ViaDrill\n", "ViaDrill,VII\n");
        }),
        ("filled and open vias in one tool", "plated.drl", |f| {
            proof::edit(f, "plated.drl", |l| {
                let at = l.iter().position(|s| s.ends_with(",VII"));
                let number = at
                    .and_then(|k| l.get(k + 1))
                    .and_then(|s| s.split('C').next())
                    .map(str::to_owned);
                l.into_iter()
                    .filter(|s| Some(s) != number.as_ref())
                    .collect()
            });
        }),
        ("the table calls it open", "drill-table.txt", |f| {
            swap(f, "drill-table.txt", "capped", "none  ");
        }),
        ("the job file without its protection", "proof.gbrjob", |f| {
            swap(f, "proof.gbrjob", "\"VII\"", "");
        }),
        ("the job file with one no tool has", "proof.gbrjob", |f| {
            swap(f, "proof.gbrjob", "\"VII\"", "\"VII\", \"VI\"");
        }),
        (
            "the order notes with another count",
            "order-notes.txt",
            |f| {
                swap(f, "order-notes.txt", "1 hole\n", "2 holes\n");
            },
        ),
        (
            "the order notes fill the open tool",
            "order-notes.txt",
            |f| {
                swap(f, "order-notes.txt", "Fill and cap", "Leave open ");
            },
        ),
        ("a charge that does not fall", "order-notes.txt", |f| {
            swap(
                f,
                "order-notes.txt",
                "\nCharges\n",
                "\nCharges\n  \"Tiny holes are charged.\"\n",
            );
        }),
        ("a charge left out", "order-notes.txt", |f| {
            swap(f, "order-notes.txt", "  \"Filling is charged.\"\n", "");
        }),
        ("no order notes", "order-notes.txt", |f| {
            f.retain(|x| !x.0.ends_with("order-notes.txt"));
        }),
    ];
    let project = filled()?;
    for (what, file, seed) in faults {
        let found = told(&project, seed)?;
        assert!(
            found.iter().any(|m| m.file.ends_with(file)),
            "{what}: {found:#?}"
        );
    }
    // Order notes for a board that needs none.
    let found = told(&proof::board()?, |f| {
        f.push(("proof-order-notes.txt".into(), "ORDER NOTES\n".into()));
    })?;
    assert!(!found.is_empty());
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

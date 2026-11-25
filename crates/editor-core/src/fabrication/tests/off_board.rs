//! What lies off the board (an import keeps such copper, a waiver may accept
//! it): told piece by piece, and in the files exactly as in the design, not
//! cut at the outline. The outline, the profile and the job file are the
//! board's alone.
use super::expected;
use super::proof::{self, Files};
use super::raster::{self, Bitmap};
use super::{Fallible, excellon_reader, reader};
use crate::diagnostics::{Finding, ObjectRef};
use crate::fabrication::readback::{self, Mismatch};
use crate::fabrication::{self, FabricationOptions, ProductionSnapshot, preflight};
use crate::layout;
use crate::library::PadKind;
use crate::model::{CopperLayer, Layer, Project, Track, Via, ViaSpan};
use std::collections::BTreeMap;

/// The proof board (60 by 40 mm, every kind of pad, hole and opening) with
/// its first part wholly off it on the left, its second across its right
/// edge, and tracks and vias across an edge, off the board and over a cutout.
fn board() -> Fallible<Project> {
    let mut p = proof::board()?;
    for (placement, position) in p.placements.iter_mut().zip([[-20.0, 12.0], [59.0, 20.0]]) {
        placement.position = position;
    }
    let net = p.nets.first().ok_or("a net")?.id;
    for at in [[-7.5, -6.25], [60.1, 33.0], [30.0, 32.0]] {
        let id = p.allocate()?;
        p.vias.push(Via {
            id,
            net,
            position: at,
            drill: 0.3,
            diameter: 0.6,
            span: ViaSpan::Through,
            tented: false,
            fill: Default::default(),
            locked: false,
        });
    }
    for (layer, start, end) in [
        (CopperLayer::Top, [50.0, 38.0], [66.5, 44.0]),
        (CopperLayer::Bottom, [-9.0, -3.0], [-2.0, -8.0]),
        (CopperLayer::Top, [29.0, 32.0], [31.0, 32.5]),
    ] {
        let id = p.allocate()?;
        p.tracks.push(Track {
            id,
            net,
            start,
            end,
            width: 0.4,
            layer,
            locked: false,
        });
    }
    proof::refill(&mut p)?;
    Ok(p)
}

fn options() -> FabricationOptions {
    let mut options = FabricationOptions {
        base_name: Some("proof".into()),
        ..FabricationOptions::default()
    };
    // The PDFs are the io crate's, and tested there.
    options.outputs.drawings = false;
    options
}

/// What the check before every export tells of the set, after `change`.
fn told(project: &Project, change: impl Fn(&mut Files)) -> Fallible<Vec<Mismatch>> {
    let snapshot = ProductionSnapshot::resolve(project, None)?;
    let options = options();
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

/// The objects that findings of `codes` name.
fn named(findings: &[Finding], codes: &[&str]) -> Vec<ObjectRef> {
    findings
        .iter()
        .filter(|f| codes.contains(&f.code.as_str()))
        .flat_map(|f| f.objects.clone())
        .collect()
}

#[test]
fn every_piece_off_the_board_is_told_and_blocks() -> Fallible<()> {
    let project = board()?;
    let findings = layout::checks(&project, &Default::default());
    let copper = named(&findings, &["board.outside_outline"]);
    let holes = named(
        &findings,
        &["board.outside_outline", "board.edge_clearance"],
    );
    let [ax, ay] = project.board_size;
    let off = |p: [f64; 2]| p[0] < 0.0 || p[1] < 0.0 || p[0] > ax || p[1] > ay;
    let mut pieces = 0;
    for t in project.tracks.iter().filter(|t| off(t.start) || off(t.end)) {
        assert!(copper.contains(&ObjectRef::Track { id: t.id }), "{t:?}");
        pieces += 1;
    }
    for v in project.vias.iter().filter(|v| off(v.position)) {
        assert!(copper.contains(&ObjectRef::Via { id: v.id }), "{v:?}");
        pieces += 1;
    }
    for pad in layout::placed_pads(&project) {
        let has = |list: &[ObjectRef]| {
            list.contains(&ObjectRef::Pad {
                instance: pad.instance,
                pad: pad.pad.id,
            })
        };
        if off(pad.center()) {
            let list = match pad.pad.kind {
                PadKind::NonPlatedHole => &holes,
                _ => &copper,
            };
            assert!(has(list), "{:?} {}", pad.pad.kind, pad.pad.id);
            pieces += 1;
        }
    }
    assert!(pieces > 12, "{pieces}");
    // Over a cutout is off the board too.
    let over = project.vias.iter().find(|v| v.position == [30.0, 32.0]);
    let over = over.ok_or("the via over the cutout")?.id;
    assert!(copper.contains(&ObjectRef::Via { id: over }));
    // Each is an error of the production check.
    let report = preflight::report(&project, &Default::default(), None, &Default::default())?;
    for f in findings
        .iter()
        .filter(|f| f.code == "board.outside_outline")
    {
        let key = preflight::key(&f.code, &f.objects);
        let item = report.items.iter().find(|i| i.key == key);
        assert!(item.is_some_and(|i| i.blocks()), "{key}");
    }
    Ok(())
}

#[test]
fn files_hold_what_is_off_the_board_as_the_design_has_it() -> Fallible<()> {
    let project = board()?;
    assert_eq!(told(&project, |_| {})?, Vec::new());
    let files = proof::written(&project)?;
    // Painted from the design, far past the board on every side.
    let grid = Bitmap::new([-32.003_17, -14.003_17], [74.0, 50.0], 0.02)?;
    let mut dark = BTreeMap::new();
    let mut compare = |name: &str, got: Bitmap, want: Bitmap| -> Fallible<()> {
        if let Some(at) = super::compare::thick(&got, &want) {
            return Err(format!("{name} differs round {at:?}").into());
        }
        let outside = |b: &Bitmap| {
            (0..b.height)
                .flat_map(|j| (0..b.width).map(move |i| (i, j)))
                .filter(|(i, j)| b.get(*i, *j))
                .filter(|(i, _)| {
                    let x = b.min[0] + (*i as f64 + 0.5) * b.pixel;
                    !(-0.5..60.5).contains(&x)
                })
                .count()
        };
        dark.insert(name.to_owned(), outside(&want));
        Ok(())
    };
    let drawn = |name: &str| -> Fallible<Bitmap> {
        let mut b = grid.blank();
        if let Ok(text) = proof::file(&files, name) {
            raster::render_gerber(&reader::parse(text)?, &mut b)?;
        }
        Ok(b)
    };
    for layer in project.stackup.layers() {
        let name = proof::layer_name(&project, layer)?.0;
        let mut want = grid.blank();
        expected::copper(&project, layer, &mut want)?;
        compare(&name, drawn(&name)?, want)?;
    }
    for (side, word) in [(Layer::Top, "top"), (Layer::Bottom, "bottom")] {
        let mut want = grid.blank();
        expected::mask(&project, side, &mut want)?;
        let name = format!("mask-{word}.gbr");
        compare(&name, drawn(&name)?, want)?;
        let mut want = grid.blank();
        expected::paste(&project, side, &mut want)?;
        let name = format!("paste-{word}.gbr");
        compare(&name, drawn(&name)?, want)?;
    }
    for (plated, name) in [(true, "plated.drl"), (false, "non-plated.drl")] {
        let mut got = grid.blank();
        raster::render_drill(
            &excellon_reader::parse(proof::file(&files, name)?)?,
            &mut got,
        )?;
        let mut want = grid.blank();
        expected::drills(&project, plated, &mut want)?;
        compare(name, got, want)?;
    }
    // It was about something: each of these has material off the board.
    for name in [
        "copper-l1-top.gbr",
        "copper-l2-bottom.gbr",
        "mask-top.gbr",
        "paste-top.gbr",
        "plated.drl",
        "non-plated.drl",
    ] {
        assert!(dark.get(name).is_some_and(|n| *n > 100), "{name}: {dark:?}");
    }
    // The outline and the size are the board's, whatever lies off it.
    let on = proof::board()?;
    let profile = |p: &Project| -> Fallible<String> {
        let files = proof::written(p)?;
        let parsed = reader::parse(proof::file(&files, "profile.gbr")?)?;
        Ok(format!("{:?}", parsed.objects))
    };
    assert_eq!(profile(&project)?, profile(&on)?);
    let size = |p: &Project| -> Fallible<serde_json::Value> {
        let set = fabrication::generate(
            &ProductionSnapshot::resolve(p, None)?,
            &options(),
            &Default::default(),
        )?;
        let job = set.files.iter().find(|f| f.name.ends_with(".gbrjob"));
        let job: serde_json::Value = serde_json::from_slice(&job.ok_or("a job file")?.bytes)?;
        Ok(job["GeneralSpecs"]["Size"].clone())
    };
    assert_eq!(size(&project)?, size(&on)?);
    assert_eq!(size(&project)?["X"], 60.0);
    Ok(())
}

/// The check looks off the board as it does on it.
#[test]
fn a_fault_off_the_board_is_told() -> Fallible<()> {
    let project = board()?;
    type Seed = fn(&mut Files);
    let faults: [(&str, &str, Seed); 5] = [
        ("a via off the board missing", "copper-l1-top.gbr", |f| {
            proof::edit(f, "copper-l1-top.gbr", |mut l| {
                l.retain(|s| !s.starts_with("X-7500000Y-6250000D03"));
                l
            });
        }),
        ("its hole missing", "plated.drl", |f| {
            proof::edit(f, "plated.drl", |mut l| {
                l.retain(|s| !s.starts_with("X-7.500Y-6.250"));
                l
            });
        }),
        ("a track's end cut at the edge", "copper-l1-top.gbr", |f| {
            proof::edit(f, "copper-l1-top.gbr", |l| {
                l.iter()
                    .map(|s| s.replace("X66500000Y44000000", "X60000000Y44000000"))
                    .collect()
            });
        }),
        ("an opening off the board missing", "mask-top.gbr", |f| {
            proof::edit(f, "mask-top.gbr", |mut l| {
                l.retain(|s| !s.starts_with("X-7500000Y-6250000D03"));
                l
            });
        }),
        ("a hole off the board added", "non-plated.drl", |f| {
            proof::edit(f, "non-plated.drl", |l| {
                let at = l.iter().position(|s| s.starts_with('X')).unwrap_or(0);
                let mut l = l;
                l.insert(at, "X-30.0Y-12.0".into());
                l
            });
        }),
    ];
    let clean = proof::written(&project)?;
    let mut missed = Vec::new();
    for (what, file, seed) in faults {
        let mut seeded = clean.clone();
        seed(&mut seeded);
        assert!(seeded != clean, "{what} seeded nothing");
        let found = told(&project, seed)?;
        if !found.iter().any(|m| m.file.ends_with(file)) {
            missed.push(format!("{what}: {found:?}"));
        }
    }
    assert!(missed.is_empty(), "{missed:#?}");
    Ok(())
}

/// While the outline is not valid the shape that is held is an earlier one,
/// or a frame round an import: no file is made of it, whatever was waived.
#[test]
fn a_board_without_a_valid_outline_gives_no_file() -> Fallible<()> {
    use crate::model::BoardShape;
    let mut project = proof::board()?;
    assert!(fabrication::representable(&project).is_ok());
    if let BoardShape::Sketch(sketch) = &mut project.board_shape {
        sketch.current = false;
        sketch.fault = Some(crate::model::board::ShapeFault {
            kind: crate::model::board::ShapeFaultKind::Open,
            message: "The outline is open".into(),
            at: Vec::new(),
        });
    }
    let snapshot = ProductionSnapshot::resolve(&project, None)?;
    let refused = fabrication::generate(&snapshot, &options(), &Default::default());
    let text = refused.err().ok_or("files were made")?.to_string();
    assert!(text.contains("outline"), "{text}");
    Ok(())
}

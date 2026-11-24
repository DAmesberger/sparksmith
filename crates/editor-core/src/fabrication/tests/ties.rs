//! Manufacturing files for a board with a copper tie, two switchable
//! jumpers and a bought zero-ohm link. Read the finished CSVs and Gerbers.
use super::{Fallible, raster, reader};
use crate::fabrication::{self, FabricationSet, ProductionSnapshot};
use crate::ids::{BridgeId, PadId, TerminalId, TieId};
use crate::instances::variants::VariantCommand;
use crate::library::tie::{Bridge, JumperBridge, JumperState, TieForm, TieSpec};
use crate::library::{Definition, FootprintKeepout, PackageModel, PadSide};
use crate::model::Population;
use crate::tests::support::*;
use crate::{Engine, Id, Operation, Outcome};

fn tie_part(engine: &mut Engine, name: &str, form: TieForm) -> Fallible<Fixture> {
    let jumper = matches!(form, TieForm::SolderJumper { .. });
    let mut spec = Spec {
        name,
        terminals: vec![(1, "A", passive()), (2, "B", passive())],
        units: vec![(1, "A", vec![(1, [-4., 0.]), (2, [4., 0.])])],
        package: vec![(1, "1"), (2, "2")],
        mapping: vec![(1, 1), (2, 2)],
        physical: true,
    };
    if jumper {
        spec.terminals.push((3, "C", passive()));
        spec.units[0].2.push((3, [4., 4.]));
        spec.package.push((3, "3"));
        spec.mapping.push((3, 3));
    }
    // The cuttable component also carries a separate solder-only tie. Its
    // first group deliberately has no trace to cut.
    let mixed = matches!(
        form,
        TieForm::SolderJumper {
            bridge: JumperBridge::Trace,
            ..
        }
    );
    if mixed {
        spec.terminals
            .extend([(4, "D", passive()), (5, "E", passive())]);
        spec.units[0].2.extend([(4, [4., 8.]), (5, [4., 12.])]);
        spec.package.extend([(4, "4"), (5, "5")]);
        spec.mapping.extend([(4, 4), (5, 5)]);
    }
    let f = draft(engine, &spec)?;
    let pair = if jumper { [2, 3] } else { [1, 2] };
    rewrite(engine, f.component, |d| {
        if let Definition::Component(c) = d {
            c.ties = vec![TieSpec {
                id: TieId(1),
                form,
                ends: pair.map(TerminalId).to_vec(),
            }];
            if mixed {
                c.ties.insert(
                    0,
                    TieSpec {
                        id: TieId(2),
                        form: TieForm::SolderJumper {
                            bridge: JumperBridge::Solder,
                            default: JumperState::Closed,
                        },
                        ends: vec![TerminalId(4), TerminalId(5)],
                    },
                );
            }
            c.assembly = form.assembly();
        }
    })?;
    if form.needs_bridge() {
        rewrite(engine, f.package, |d| {
            if let Definition::Package(p) = d {
                p.model = PackageModel::NoBody;
            }
        })?;
        rewrite(engine, f.footprint, |d| {
            if let Definition::Footprint(fp) = d {
                let (a, b) = if jumper { (0., 1.5) } else { (-1.5, 0.) };
                fp.bridges = vec![Bridge {
                    id: BridgeId(fp.next_local),
                    pads: pair.map(PadId).to_vec(),
                    side: PadSide::Top,
                    outline: FootprintKeepout::rect("", [a, -0.1], [b, 0.1]).outline,
                }];
                fp.next_local += 1;
            }
        })?;
    }
    publish(engine, f.all())?;
    Ok(f)
}

fn release(engine: &Engine) -> Fallible<FabricationSet> {
    let _context = engine.enter_context();
    Ok(fabrication::generate(
        &ProductionSnapshot::resolve(engine.project(), None)?,
        &Default::default(),
        engine.check_settings(),
    )?)
}

fn text<'a>(set: &'a FabricationSet, suffix: &str) -> Fallible<&'a str> {
    let file = set
        .files
        .iter()
        .find(|f| f.name.ends_with(suffix))
        .ok_or_else(|| format!("missing {suffix}"))?;
    Ok(std::str::from_utf8(&file.bytes)?)
}

fn opens(set: &FabricationSet, centres: &[[f64; 2]]) -> Fallible<Vec<bool>> {
    let film = reader::parse(text(set, "-paste-top.gbr")?)?;
    let mut bitmap = raster::Bitmap::new([0., 0.], [60., 40.], 0.05)?;
    raster::render_gerber(&film, &mut bitmap)?;
    Ok(centres.iter().map(|p| bitmap.at(*p)).collect())
}

fn centres(engine: &Engine, instance: Id) -> Vec<[f64; 2]> {
    let mut pads: Vec<_> = crate::layout::placed_pads(engine.project())
        .into_iter()
        .filter(|p| p.instance == instance)
        .collect();
    pads.sort_by_key(|p| p.pad.id);
    pads.iter().map(|p| p.center()).collect()
}

#[test]
fn typed_ties_in_released_copper_paste_cut_steps_and_assembly_csvs() -> Fallible<()> {
    let mut engine = Engine::profiled()?;
    let copper = tie_part(&mut engine, "Copper return", TieForm::Copper)?;
    let (nt, _) = place(&mut engine, &copper, "NT1", 1, [20., 10.])?;
    let jumper = |bridge| TieForm::SolderJumper {
        bridge,
        default: JumperState::Closed,
    };
    let trace = tie_part(&mut engine, "Cut link", jumper(JumperBridge::Trace))?;
    let (jp1, _) = place(&mut engine, &trace, "JP1", 1, [40., 20.])?;
    let solder = tie_part(&mut engine, "Solder link", jumper(JumperBridge::Solder))?;
    let (jp2, _) = place(&mut engine, &solder, "JP2", 1, [60., 30.])?;
    let chip = tie_part(&mut engine, "Jumper resistor", TieForm::ZeroOhm)?;
    let (link, _) = place(&mut engine, &chip, "R1", 1, [20., 30.])?;
    // The other resistor stays populated, so the open variant still emits
    // a placement CSV with a real row and a top stencil with real apertures.
    place(&mut engine, &chip, "R2", 1, [80., 30.])?;
    for (pin, name) in [(1, "SIG,RET"), (2, "GND")] {
        connect(&mut engine, ep(nt, pin), ep(link, pin))?;
        let net = net_of(&engine, ep(nt, pin))?;
        apply(
            &mut engine,
            Operation::Schematic(crate::schematic::SchematicCommand::RenameNet {
                net,
                name: name.into(),
            }),
        )?;
    }
    sync_onto_board(&mut engine)?;
    let closed = release(&engine)?;
    let variant = match apply(
        &mut engine,
        Operation::Variant(VariantCommand::Add {
            name: "Links open".into(),
            copy_of: None,
        }),
    )? {
        Outcome::VariantAdded { variant } => variant,
        _ => return Err("no variant".into()),
    };
    apply(
        &mut engine,
        Operation::Variant(VariantCommand::SetPopulation {
            instances: vec![jp1, jp2, link],
            variant: Some(variant),
            population: Some(Population::NotPopulated),
        }),
    )?;
    apply(
        &mut engine,
        Operation::Variant(VariantCommand::SetActive {
            variant: Some(variant),
        }),
    )?;
    let open = release(&engine)?;
    for set in [&closed, &open] {
        let film = reader::parse(text(set, "-copper-l1-top.gbr")?)?;
        let bridge = film
            .regions()
            .find(|r| {
                r.object_attributes
                    .get(".C")
                    .is_some_and(|ids| ids == &["NT1"])
            })
            .ok_or("copper tie region")?;
        let mut nets = bridge
            .object_attributes
            .get(".N")
            .cloned()
            .ok_or("bridge net attribute")?;
        nets.sort();
        assert_eq!(nets, ["GND", "SIG,RET"]);
        assert!(
            film.regions().any(|r| r
                .object_attributes
                .get(".C")
                .is_some_and(|ids| ids == &["JP1"])),
            "The cuttable trace is always fabricated"
        );
        assert!(
            !film.regions().any(|r| r
                .object_attributes
                .get(".C")
                .is_some_and(|ids| ids == &["JP2"])),
            "A solder jumper has bare lands"
        );
        let positions = text(set, "-pos-top.csv")?;
        let bought = text(set, "-bom.csv")?;
        assert!(positions.contains("R2") && bought.contains("R2"));
        for name in ["NT1", "JP1", "JP2"] {
            assert!(!positions.contains(name) && !bought.contains(name));
        }
        assert_eq!(opens(set, &centres(&engine, nt))?, [false, false]);
    }
    for (jp, closed_paste) in [
        (jp1, vec![false, true, true, true, true]),
        (jp2, vec![false, true, true]),
    ] {
        let pads = centres(&engine, jp);
        assert_eq!(opens(&closed, &pads)?, closed_paste);
        assert_eq!(opens(&open, &pads)?, vec![false; pads.len()]);
    }
    for suffix in ["-pos-top.csv", "-bom.csv"] {
        assert!(text(&closed, suffix)?.contains("R1"));
        assert!(!text(&open, suffix)?.contains("R1"));
    }
    let cuts = |set: &FabricationSet| {
        set.drawing
            .iter()
            .flat_map(|d| &d.sides)
            .flat_map(|s| &s.cuts)
            .map(|c| c.label.clone())
            .collect::<Vec<_>>()
    };
    assert!(cuts(&closed).is_empty());
    assert_eq!(cuts(&open), ["Cut JP1 pads 2–3"]);
    Ok(())
}

/// The copper of a tie, its lands and its bridge, is the document's, turned
/// and on either side; a bridge left out or moved is seen.
#[test]
fn a_bridge_is_plotted_where_the_design_has_it() -> Fallible<()> {
    use super::oracle::{self, Image};
    use crate::model::{CopperLayer, Layer};
    let mut engine = Engine::profiled()?;
    let copper = tie_part(&mut engine, "Copper return", TieForm::Copper)?;
    let (nt, _) = place(&mut engine, &copper, "NT1", 1, [20., 10.])?;
    let chip = tie_part(&mut engine, "Jumper resistor", TieForm::ZeroOhm)?;
    let (link, _) = place(&mut engine, &chip, "R1", 1, [20., 30.])?;
    for pin in [1, 2] {
        connect(&mut engine, ep(nt, pin), ep(link, pin))?;
    }
    sync_onto_board(&mut engine)?;
    for (side, layer, name) in [
        (Layer::Top, CopperLayer::Top, "-top.gbr"),
        (Layer::Bottom, CopperLayer::Bottom, "-bottom.gbr"),
    ] {
        let mut project = engine.project().clone();
        let placement = project
            .placements
            .iter_mut()
            .find(|p| p.instance == nt)
            .ok_or("NT1 on the board")?;
        placement.position = [30.0, 20.0];
        placement.rotation_deg = 25.0;
        placement.layer = side;
        let set = fabrication::generate(
            &ProductionSnapshot::resolve(&project, None)?,
            &Default::default(),
            &Default::default(),
        )?;
        let film = set
            .files
            .iter()
            .find(|f| f.name.contains("-copper-") && f.name.ends_with(name))
            .ok_or("the copper layer")?;
        let film = std::str::from_utf8(&film.bytes)?;
        let design = oracle::copper(&project, layer);
        assert!(design.iter().any(|p| p.what.contains("bridge")));
        let b = project.board().bounds();
        let differs = |text: &str| -> Fallible<usize> {
            let image = Image::read(text)?;
            Ok(oracle::differences(&image, &design, [b.min, b.max], 0.02).len())
        };
        assert_eq!(differs(film)?, 0, "{side:?}");
        // Seeded: the bridge's region left out, and moved by 0.02 mm.
        let start = film
            .find("%TA.AperFunction,EtchedComponent*%")
            .ok_or("the bridge")?;
        let end = start + film[start..].find("G37*\n").ok_or("its end")? + 5;
        let without = format!("{}{}", &film[..start], &film[end..]);
        assert!(differs(&without)? > 0, "{side:?}: the bridge left out");
        let moved: String = film[start..end]
            .lines()
            .map(|l| {
                let l = if l.starts_with('X') {
                    super::proof::shifted(l, 'X', 0.02, false)
                } else {
                    l.to_owned()
                };
                format!("{l}\n")
            })
            .collect();
        let moved = format!("{}{moved}{}", &film[..start], &film[end..]);
        assert!(differs(&moved)? > 0, "{side:?}: the bridge moved");
    }
    Ok(())
}

/// A net tie's bridge is copper like any other: a hole short of the track
/// clearance from it and fill copper short of its own, even fill of a net
/// it joins, are found.
#[test]
fn a_bridge_keeps_its_clearance_from_holes_and_fills() -> Fallible<()> {
    use crate::diagnostics::ObjectRef;
    use crate::model::CopperLayer;
    use crate::model::board::{BoardFeatureId, SolvedBoard, SolvedMountingHole};
    use crate::model::fill::{Fill, FilledCopper, FilledRegion};
    use crate::rules::{CopperObject, RuleKey, RuleOverride, RuleValue};
    let mut engine = Engine::profiled()?;
    let copper = tie_part(&mut engine, "Copper return", TieForm::Copper)?;
    let (nt, _) = place(&mut engine, &copper, "NT1", 1, [20., 10.])?;
    let chip = tie_part(&mut engine, "Jumper resistor", TieForm::ZeroOhm)?;
    let (link, _) = place(&mut engine, &chip, "R1", 1, [20., 30.])?;
    for pin in [1, 2] {
        connect(&mut engine, ep(nt, pin), ep(link, pin))?;
    }
    sync_onto_board(&mut engine)?;
    let net = net_of(&engine, ep(nt, 1))?;
    let mut project = engine.project().clone();
    let placement = project
        .placements
        .iter_mut()
        .find(|p| p.instance == nt)
        .ok_or("NT1 on the board")?;
    placement.position = [30.0, 20.0];
    placement.rotation_deg = 0.0;
    for b in [CopperObject::NpthHole, CopperObject::Fill] {
        project.rules.overrides.push(RuleOverride {
            key: RuleKey::Clearance {
                a: CopperObject::Track,
                b,
            }
            .normalized(),
            value: RuleValue::Length(0.3),
        });
    }
    // The bridge runs from (28.5, 19.9) to (30, 20.1): a 1 mm hole `gap`
    // above it, and a pour `gap` below.
    let instance = ObjectRef::Instance { id: nt };
    for (gap, found) in [(0.2, true), (0.35, false)] {
        let mut p = project.clone();
        let mut solved = SolvedBoard::rectangle(p.board_size);
        solved.holes.push(SolvedMountingHole {
            id: BoardFeatureId(3),
            at: [29.25, 20.1 + gap + 0.5],
            drill: 1.0,
            plated: false,
            pad_diameter: None,
        });
        p.board_shape = super::proof::sketch(solved);
        let hole = ObjectRef::BoardFeature { feature: 3 };
        let holes = crate::layout::checks(&p, &Default::default())
            .into_iter()
            .filter(|f| f.objects == vec![instance, hole])
            .count();
        assert_eq!(holes, usize::from(found), "hole {gap} mm off");
        let id = p.allocate()?;
        let (top, bottom) = (19.9 - gap, 18.0);
        let region = vec![[29.0, bottom], [29.5, bottom], [29.5, top], [29.0, top]];
        p.fills.push(Fill {
            id,
            name: "Pour".into(),
            net: Some(net),
            layer: CopperLayer::Top,
            region: Default::default(),
            outline: FootprintKeepout::rect("", [28.0, 17.0], [31.0, 19.95]).outline,
            priority: 0,
            settings: Default::default(),
            locked: false,
            computed: None,
        });
        let key = crate::layout::fill::inputs_key(&p, id)?;
        for f in p.fills.iter_mut().filter(|f| f.id == id) {
            f.computed = Some(FilledCopper {
                key: key.clone(),
                regions: vec![FilledRegion {
                    outer: region.clone(),
                    holes: Vec::new(),
                }],
                removed_islands: 0,
                removed_spokes: Vec::new(),
            });
        }
        let fills = crate::layout::fill::findings(&p)
            .into_iter()
            .filter(|f| f.objects == vec![ObjectRef::Fill { id }, instance])
            .count();
        assert_eq!(fills, usize::from(found), "pour {gap} mm off");
    }
    Ok(())
}

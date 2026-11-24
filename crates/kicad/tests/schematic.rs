//! A Rust-built schematic over a Rust-built board (slice K5): sheets,
//! symbols at KiCad's places with KiCad's pins, wires, a power symbol and
//! a label drawn through the engine, and the schematic's nets equal to the
//! board's. All data here is original.
mod support;

use sparksmith_kicad::report::Case;
use std::collections::BTreeMap;
use support::{Board, Footprint, Pad};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// Two resistors: R1.1 on GND, R1.2 and R2.1 on SIG, R2.2 on OUT, plus a
/// pin of type "not connected" on each that joins nothing.
fn board() -> Board {
    let pad = |number: &'static str, x: f64, net: (u32, &str)| {
        Pad::smd(number, "rect", [x, 0.0, 0.0], [0.8, 0.6])
            .with(&format!("(net {} \"{}\")", net.0, net.1))
    };
    let resistor = |reference: &str, x: f64, nets: [(u32, &str); 2]| {
        Footprint::new(
            reference,
            [x, 10.0, 0.0],
            false,
            vec![pad("1", -1.0, nets[0]), pad("2", 1.0, nets[1])],
        )
    };
    Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![
            resistor("R1", 10.0, [(1, "GND"), (2, "SIG")]),
            resistor("R2", 20.0, [(2, "SIG"), (3, "OUT")]),
        ],
        items: "  (net 1 \"GND\") (net 2 \"SIG\") (net 3 \"OUT\")
  (gr_rect (start 0 0) (end 30 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))\n"
            .into(),
    }
}

/// The root sheet: R1 at (50, 50), R2 at (60, 50) turned 180°, a wire
/// with three corners from R1.2 to R2.1, a GND power symbol on R1.1 and a
/// local label OUT on R2.2. Pins are 3.81 mm above and below each
/// symbol's origin (library frame, y up; the sheet's y is down, so R1.2
/// and the turned R2's pin 1 are at y = 53.81).
fn schematic() -> String {
    let resistor_lib = "(symbol \"Synthetic:R\" (in_bom yes) (on_board yes)
      (symbol \"R_0_1\" (rectangle (start -1 2.54) (end 1 -2.54)))
      (symbol \"R_1_1\"
        (pin passive line (at 0 3.81 270) (length 1.27) (name \"~\") (number \"1\"))
        (pin passive line (at 0 -3.81 90) (length 1.27) (name \"~\") (number \"2\"))
        (pin no_connect line (at 2.54 0 180) (length 1.27) hide (name \"NC\") (number \"3\"))))";
    let gnd_lib = "(symbol \"Synthetic:GND\" (power) (in_bom yes) (on_board yes)
      (symbol \"GND_1_1\" (pin power_in line (at 0 0 270) (length 0) hide (name \"GND\") (number \"1\"))))";
    let symbol = |lib: &str, reference: &str, value: &str, at: &str| {
        format!(
            "(symbol (lib_id \"{lib}\") (at {at}) (unit 1)
               (property \"Reference\" \"{reference}\" (at 0 0 0))
               (property \"Value\" \"{value}\" (at 0 0 0))
               (instances (project \"synthetic\" (path \"/00000000-0000-4000-8000-000000000001\" (reference \"{reference}\") (unit 1)))))"
        )
    };
    format!(
        "(kicad_sch (version 20231120) (generator \"sparksmith_test\")
  (uuid \"00000000-0000-4000-8000-000000000001\")
  (paper \"A4\")
  (lib_symbols {resistor_lib} {gnd_lib})
  {r1} {r2} {gnd}
  (wire (pts (xy 50 53.81) (xy 55 53.81)))
  (wire (pts (xy 55 53.81) (xy 55 58) (xy 60 58)))
  (wire (pts (xy 60 58) (xy 60 53.81)))
  (label \"OUT\" (at 60 46.19 0))
)",
        r1 = symbol("Synthetic:R", "R1", "10k", "50 50 0"),
        // Turned half round: its pin 1 is below, pin 2 above.
        r2 = symbol("Synthetic:R", "R2", "10k", "60 50 180"),
        gnd = symbol("Synthetic:GND", "#PWR01", "GND", "50 46.19 0"),
    )
}

#[test]
fn the_schematic_is_drawn_and_its_nets_equal_the_boards() -> Fallible<()> {
    let mut files = BTreeMap::new();
    files.insert("synthetic.kicad_sch".to_owned(), schematic().into_bytes());
    let imported = support::import_with_schematic(
        board().text().as_bytes(),
        None,
        Some(("synthetic.kicad_sch", &files)),
    )?;
    let project = imported.engine.project();
    let report = &imported.report;
    assert_eq!(report.counts.sheets, 1);
    assert_eq!(
        (
            report.counts.schematic_nets_equal,
            report.counts.schematic_nets_split,
            report.counts.schematic_nets_merged
        ),
        (3, 0, 0)
    );
    // The imported schematic's nets are the source schematic's.
    assert_eq!(
        (report.counts.source_nets, report.counts.source_nets_equal),
        (3, 3)
    );
    // The wire with its corners, the power port and the label; no
    // labels made up per pin.
    assert_eq!(report.count(Case::SchematicWire), 1);
    assert_eq!(report.count(Case::SchematicLabel), 2);
    assert_eq!(project.labels.len(), 2);
    assert_eq!(report.count(Case::NoConnectPin), 2);
    // R1 sits where KiCad has it: (50, 50) is (20, −20) on our sheet.
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let occurrence = project.occurrences_of(r1.id).next().ok_or("on no sheet")?;
    let at = occurrence.position.to_mm_f64();
    assert!((at[0] - 20.0).abs() < 1e-9 && (at[1] + 20.0).abs() < 1e-9);
    // Its symbol has KiCad's pins: pin 1 at (0, 3.81), and the "not
    // connected" pin drawn too.
    let symbol = project
        .library
        .symbol(sparksmith_core::SymbolRef::new(
            occurrence_symbol(project, r1.id)?,
            sparksmith_core::Revision::FIRST,
        ))
        .ok_or("symbol")?;
    let pins: Vec<_> = symbol
        .definition
        .units
        .iter()
        .flat_map(|u| &u.pins)
        .collect();
    assert_eq!(pins.len(), 2);
    let p = pins[0].position.to_mm_f64();
    assert!(p[0].abs() < 1e-9 && (p[1] - 3.81).abs() < 1e-9, "{p:?}");
    // Both resistors share the SIG net through the wire.
    let net = |d: &str, t: u32| -> Option<sparksmith_core::Id> {
        let i = project.instance_by_designator(d)?;
        project
            .net_of(sparksmith_core::Endpoint {
                instance: i.id,
                terminal: sparksmith_core::TerminalId(t),
            })
            .map(|n| n.id)
    };
    assert!(net("R1", 2).is_some());
    assert_eq!(net("R1", 2), net("R2", 1));
    assert_ne!(net("R1", 1), net("R1", 2));
    Ok(())
}

fn occurrence_symbol(
    project: &sparksmith_core::Project,
    instance: sparksmith_core::Id,
) -> Fallible<sparksmith_core::DefinitionId> {
    Ok(project.instance(instance).ok_or("instance")?.representation)
}

/// A two-pin resistor symbol, pins 1 (up) and 2 (down) 3.81 mm from its
/// origin, inside a body from (−1, −2.54) to (1, 2.54).
const TWO_PIN: &str = "(symbol \"Synthetic:R2\" (in_bom yes) (on_board yes)
      (symbol \"R2_0_1\" (rectangle (start -1 2.54) (end 1 -2.54)))
      (symbol \"R2_1_1\"
        (pin passive line (at 0 3.81 270) (length 1.27) (name \"~\") (number \"1\"))
        (pin passive line (at 0 -3.81 90) (length 1.27) (name \"~\") (number \"2\"))))";

/// A root sheet of `TWO_PIN` symbols (`(reference, "x y angle")`) and
/// further items.
fn sheet(symbols: &[(&str, &str)], items: &str) -> String {
    let placed: String = symbols
        .iter()
        .map(|(reference, at)| {
            format!(
                "(symbol (lib_id \"Synthetic:R2\") (at {at}) (unit 1)
                   (property \"Reference\" \"{reference}\" (at 0 0 0))
                   (property \"Value\" \"1k\" (at 0 0 0))
                   (instances (project \"synthetic\" (path \"/00000000-0000-4000-8000-000000000002\" (reference \"{reference}\") (unit 1)))))\n"
            )
        })
        .collect();
    format!(
        "(kicad_sch (version 20231120) (generator \"sparksmith_test\")
  (uuid \"00000000-0000-4000-8000-000000000002\")
  (paper \"A4\")
  (lib_symbols {TWO_PIN})
  {placed}
  {items}
)"
    )
}

/// One part of [`row_board`]: its reference and pads (number, net).
type Part<'a> = (&'a str, &'a [(&'static str, Option<(u32, &'a str)>)]);

/// Resistors in a row on the board, each pad on the net given (or none).
fn row_board(parts: &[Part<'_>]) -> Board {
    let mut nets = BTreeMap::new();
    let footprints = parts
        .iter()
        .enumerate()
        .map(|(k, (reference, pads))| {
            let pads = pads
                .iter()
                .enumerate()
                .map(|(j, (number, net))| {
                    let pad =
                        Pad::smd(number, "rect", [j as f64 * 2.0 - 1.0, 0.0, 0.0], [0.8, 0.6]);
                    match net {
                        Some((n, name)) => {
                            nets.insert(*n, *name);
                            pad.with(&format!("(net {n} \"{name}\")"))
                        }
                        None => pad,
                    }
                })
                .collect();
            Footprint::new(reference, [10.0 + 8.0 * k as f64, 10.0, 0.0], false, pads)
        })
        .collect();
    let mut items: String = nets
        .iter()
        .map(|(n, name)| format!("  (net {n} \"{name}\")"))
        .collect();
    items.push_str(
        "\n  (gr_rect (start 0 0) (end 60 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))\n",
    );
    Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints,
        items,
    }
}

fn import_sheet(board: &Board, sheet: String) -> Fallible<sparksmith_kicad::Imported> {
    let mut files = BTreeMap::new();
    files.insert("synthetic.kicad_sch".to_owned(), sheet.into_bytes());
    Ok(support::import_with_schematic(
        board.text().as_bytes(),
        None,
        Some(("synthetic.kicad_sch", &files)),
    )?)
}

/// The net of `designator`'s terminal named `number`.
fn net_of(
    project: &sparksmith_core::Project,
    designator: &str,
    number: &str,
) -> Option<sparksmith_core::Id> {
    let instance = project.instance_by_designator(designator)?;
    let terminal = project
        .library
        .component(instance.component)?
        .definition
        .terminals
        .iter()
        .find(|t| t.name == number)?
        .id;
    project
        .net_of(sparksmith_core::Endpoint {
            instance: instance.id,
            terminal,
        })
        .map(|n| n.id)
}

/// Each Issues row's code and title.
fn issue_rows(project: &sparksmith_core::Project) -> Fallible<Vec<(String, String)>> {
    let report = sparksmith_core::issues::report(
        project,
        &sparksmith_core::library::Drafts::default(),
        &sparksmith_core::issues::IssueScope::All,
        &Default::default(),
    )?;
    Ok(report
        .rows
        .iter()
        .map(|r| (r.finding.code.to_string(), r.title.clone()))
        .collect())
}

/// Four wire ends meeting at one point with a junction (left, right, up
/// and down, each to a pin) are one net, as in KiCad: our wires are drawn
/// as strokes, and the second stroke through the point must end on the
/// first there, never cross it (crossing wires do not join).
#[test]
fn four_wire_ends_at_one_point_are_one_net() -> Fallible<()> {
    let sig = Some((5, "SIG"));
    let board = row_board(&[
        ("R1", &[("1", Some((1, "A"))), ("2", sig)]),
        ("R2", &[("1", sig), ("2", Some((2, "B")))]),
        ("R3", &[("1", Some((3, "C"))), ("2", sig)]),
        ("R4", &[("1", sig), ("2", Some((4, "D")))]),
    ]);
    // Centre (70, 70): R1.2 above at (70, 63.81), R2.1 below at
    // (70, 76.19); R3 and R4 turned a quarter, R3.2 left at (63.81, 70)
    // and R4.1 right at (76.19, 70).
    let schematic = sheet(
        &[
            ("R1", "70 60 0"),
            ("R2", "70 80 0"),
            ("R3", "60 70 90"),
            ("R4", "80 70 90"),
        ],
        "(wire (pts (xy 63.81 70) (xy 70 70)))
  (wire (pts (xy 70 70) (xy 76.19 70)))
  (wire (pts (xy 70 63.81) (xy 70 70)))
  (wire (pts (xy 70 70) (xy 70 76.19)))
  (junction (at 70 70))",
    );
    let imported = import_sheet(&board, schematic)?;
    let project = imported.engine.project();
    let c = &imported.report.counts;
    assert_eq!(
        (
            c.schematic_nets_equal,
            c.schematic_nets_split,
            c.schematic_nets_merged
        ),
        (5, 0, 0)
    );
    let sig = net_of(project, "R1", "2");
    assert!(sig.is_some());
    for (d, n) in [("R2", "1"), ("R3", "2"), ("R4", "1")] {
        assert_eq!(net_of(project, d, n), sig, "{d}.{n}");
    }
    assert!(project.net_disagreements.is_empty());
    Ok(())
}

/// Where the board and its schematic disagree, the schematic wins and each
/// disagreement is a finding: the board joins R1.2 and R2.1 (SIG) that the
/// schematic leaves apart, and the schematic joins R1.1 and R2.2, which
/// the board has on two nets (GND and OUT).
#[test]
fn board_and_schematic_disagreements_are_findings() -> Fallible<()> {
    let sig = Some((2, "SIG"));
    let board = row_board(&[
        ("R1", &[("1", Some((1, "GND"))), ("2", sig)]),
        ("R2", &[("1", sig), ("2", Some((3, "OUT")))]),
    ]);
    // R1.1 at (50, 46.19) wired to R2.2 at (60, 53.81).
    let schematic = sheet(
        &[("R1", "50 50 0"), ("R2", "60 50 0")],
        "(wire (pts (xy 50 46.19) (xy 55 46.19) (xy 55 53.81) (xy 60 53.81)))",
    );
    let imported = import_sheet(&board, schematic)?;
    let project = imported.engine.project();
    let report = &imported.report;
    let c = &report.counts;
    assert_eq!(
        (
            c.schematic_nets_equal,
            c.schematic_nets_split,
            c.schematic_nets_merged
        ),
        (0, 1, 2)
    );
    // The schematic's nets are kept.
    assert!(net_of(project, "R1", "1").is_some());
    assert_eq!(net_of(project, "R1", "1"), net_of(project, "R2", "2"));
    assert_ne!(net_of(project, "R1", "2"), net_of(project, "R2", "1"));
    assert_eq!(report.count(Case::BoardJoinsNets), 1);
    assert_eq!(report.count(Case::BoardSplitsNet), 1);
    assert_eq!(project.net_disagreements.len(), 2);
    let rows = issue_rows(project)?;
    let joined = rows
        .iter()
        .find(|(code, _)| code == "import.board_joins_nets")
        .ok_or("no finding for the nets the board joins")?;
    assert!(
        joined.1.ends_with("joined on the imported board") && joined.1.contains(" and "),
        "{}",
        joined.1
    );
    let split = rows
        .iter()
        .find(|(code, _)| code == "import.board_splits_net")
        .ok_or("no finding for the net the board splits")?;
    assert!(
        split.1.ends_with("split on the imported board"),
        "{}",
        split.1
    );
    Ok(())
}

/// Run 7, finding 2: where the schematic and the board both give a value,
/// the part takes the schematic's. A project part of its lands (no
/// generator here) is named by its footprint and value, as an authored
/// part is, and selects no import-only field.
#[test]
fn the_schematics_value_is_the_parts_value() -> Fallible<()> {
    let mut board = row_board(&[("R1", &[("1", Some((1, "A"))), ("2", Some((2, "B")))])]);
    for fp in &mut board.footprints {
        fp.value = Some("2k".into());
    }
    let imported = import_sheet(&board, sheet(&[("R1", "50 50 0")], ""))?;
    let project = imported.engine.project();
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let name = &project
        .library
        .component(r1.component)
        .ok_or("component")?
        .meta
        .name;
    assert!(name.ends_with(" 1k"), "{name}");
    // The standard part value field holds the value; every field is a
    // standard one: no import-only field.
    let standard = imported
        .engine
        .context()
        .ok_or("standards")?
        .standards()
        .fields
        .library();
    let names: Vec<&str> = project
        .library
        .fields
        .iter()
        .map(|f| f.meta.name.as_str())
        .collect();
    assert!(names.contains(&"Value"), "{names:?}");
    assert!(
        project.library.fields.iter().all(|f| project
            .library
            .origins
            .iter()
            .any(|o| o.local.id == f.id && o.origin.library == standard)),
        "no import-only field"
    );
    assert_eq!(
        sparksmith_core::instances::value_text(&project.library, r1)?.as_deref(),
        Some("1k")
    );
    Ok(())
}

/// Run 7, finding 4: a SIG track the board runs from R1 pin 2 to R2 pin 1,
/// where the schematic leaves the two apart. The whole track is imported, as
/// the board has it; the pad the schematic keeps off its net is a short of two
/// nets, and the disagreement is its own finding.
#[test]
fn a_track_onto_a_pad_the_schematic_keeps_off_its_net_is_imported_and_reported() -> Fallible<()> {
    let sig = Some((2, "SIG"));
    let mut board = row_board(&[
        ("R1", &[("1", Some((1, "GND"))), ("2", sig)]),
        ("R2", &[("1", sig), ("2", Some((3, "OUT")))]),
    ]);
    // R1 pin 2 at (11, 10), R2 pin 1 at (17, 10), in two pieces.
    board.items.push_str(
        "  (segment (start 11 10) (end 14 10) (width 0.25) (layer \"F.Cu\") (net 2))
  (segment (start 14 10) (end 17 10) (width 0.25) (layer \"F.Cu\") (net 2))\n",
    );
    let schematic = sheet(
        &[("R1", "50 50 0"), ("R2", "60 50 0")],
        "(wire (pts (xy 50 46.19) (xy 55 46.19) (xy 55 53.81) (xy 60 53.81)))",
    );
    let imported = import_sheet(&board, schematic)?;
    let project = imported.engine.project();
    assert!(project.refused_import_tracks.is_empty());
    assert_eq!(imported.report.counts.tracks, 2);
    let report = sparksmith_core::issues::report(
        project,
        &sparksmith_core::library::Drafts::default(),
        &sparksmith_core::issues::IssueScope::All,
        &Default::default(),
    )?;
    for code in ["board.overlap", "import.board_joins_nets"] {
        assert!(
            report
                .rows
                .iter()
                .flat_map(|r| std::iter::once(&r.finding).chain(&r.also))
                .any(|f| f.code == code),
            "{code}: {:?}",
            report.rows
        );
    }
    Ok(())
}

/// A pad whose number matches no pin of its KiCad symbol gets a visible
/// pin outside the body, marked added by import, and an unmapped-pad
/// finding.
#[test]
fn an_unmapped_pad_gets_a_visible_pin_added_by_import() -> Fallible<()> {
    let board = row_board(&[(
        "R1",
        &[
            ("1", Some((1, "GND"))),
            ("2", Some((2, "SIG"))),
            ("3", Some((3, "EXTRA"))),
        ],
    )]);
    let schematic = sheet(&[("R1", "50 50 0")], "");
    let imported = import_sheet(&board, schematic)?;
    let project = imported.engine.project();
    assert_eq!(imported.report.count(Case::TerminalWithoutPin), 1);
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let terminal = project
        .library
        .component(r1.component)
        .ok_or("component")?
        .definition
        .terminals
        .iter()
        .find(|t| t.name == "3")
        .ok_or("terminal 3")?
        .id;
    let symbol = project
        .library
        .symbol(sparksmith_core::SymbolRef::new(
            occurrence_symbol(project, r1.id)?,
            sparksmith_core::Revision::FIRST,
        ))
        .ok_or("symbol")?;
    let pins: Vec<_> = symbol
        .definition
        .units
        .iter()
        .flat_map(|u| &u.pins)
        .collect();
    assert_eq!(pins.len(), 3);
    let added = pins
        .iter()
        .find(|p| p.terminal == terminal)
        .ok_or("no pin for pad 3")?;
    assert!(added.added_by_import);
    assert_eq!(pins.iter().filter(|p| p.added_by_import).count(), 1);
    // Outside the body's right side (x = 1), pointing into it, clear of
    // the other pins.
    let at = added.position.to_mm_f64();
    assert!((at[0] - 3.54).abs() < 1e-9 && at[1].abs() < 1e-9, "{at:?}");
    assert_eq!(
        added.direction,
        sparksmith_core::library::graphics::PinDirection::Left
    );
    let rows = issue_rows(project)?;
    assert!(
        rows.iter()
            .any(|(code, title)| code == "instance.unmapped_pad"
                && title == "R1 pin 3: pin added by import"),
        "{rows:?}"
    );
    Ok(())
}

/// Run 7, finding 5: a new QFN with an exposed pad placed on an imported,
/// routed sheet. Clear of the wires it places and the routed nets keep
/// their terminals; with a pin on the routed SIG wire it would join SIG
/// (a pin on a wire joins it), so it is refused, naming the net and pin.
#[test]
fn a_new_qfn_places_on_a_routed_imported_sheet() -> Fallible<()> {
    use sparksmith_core::library::fixtures::{self, SampleRefs};
    use sparksmith_core::library::{Definition, DefinitionMeta};
    use sparksmith_core::units::PointNm;
    use sparksmith_core::{
        BindingRef, DefinitionKind, FootprintRef, InstanceCommand, LibraryCommand, Operation,
        Outcome, PackageRef, Revision, SymbolRef,
    };
    let mut b = board();
    b.items
        .push_str("  (segment (start 11 10) (end 19 10) (width 0.25) (layer \"F.Cu\") (net 2))\n");
    let mut files = BTreeMap::new();
    files.insert("synthetic.kicad_sch".to_owned(), schematic().into_bytes());
    let imported = support::import_with_schematic(
        b.text().as_bytes(),
        None,
        Some(("synthetic.kicad_sch", &files)),
    )?;
    let mut engine = imported.engine;
    assert_eq!(engine.project().tracks.len(), 1, "SIG is routed");
    // The QFN-16 preset part (exposed pad), drafted and published.
    let mut ids = Vec::new();
    for kind in [
        DefinitionKind::Component,
        DefinitionKind::Symbol,
        DefinitionKind::Package,
        DefinitionKind::Footprint,
        DefinitionKind::Binding,
    ] {
        if let Outcome::DraftCreated { id } = engine.apply(
            engine.revision(),
            Operation::Library(LibraryCommand::NewDraft {
                meta: DefinitionMeta::default(),
                definition: Definition::empty(kind),
            }),
        )? {
            ids.push(id);
        }
    }
    let [component, symbol, package, footprint, binding] = ids[..] else {
        return Err("draft reservation failed".into());
    };
    let first = Revision::FIRST;
    let edits = fixtures::preset(
        "QFN-16",
        SampleRefs {
            component,
            symbol: SymbolRef::new(symbol, first),
            package: PackageRef::new(package, first),
            footprint: FootprintRef::new(footprint, first),
            binding: BindingRef::new(binding, first),
        },
    )?;
    engine.apply(
        engine.revision(),
        Operation::Library(LibraryCommand::Edit { edits }),
    )?;
    engine.apply(
        engine.revision(),
        Operation::Library(LibraryCommand::Publish {
            drafts: ids.clone(),
            close: true,
        }),
    )?;
    let component = engine
        .project()
        .library
        .components
        .iter()
        .find(|c| c.id == component)
        .ok_or("the QFN component")?;
    let id = component.id;
    let definition = component.definition.clone();
    let binding = *definition.implementations.first().ok_or("binding")?;
    let footprint = engine
        .project()
        .library
        .binding(binding)
        .and_then(|b| b.definition.footprints.first().copied())
        .ok_or("footprint")?;
    let symbol = *definition.representations.first().ok_or("symbol")?;
    let place = |at: [f64; 2]| -> Fallible<Operation> {
        Ok(Operation::Instance(InstanceCommand::Place(
            sparksmith_core::instances::NewInstance {
                sheet: None,
                component: sparksmith_core::ComponentRef::new(id, sparksmith_core::Revision::FIRST),
                designator: None,
                properties: vec![],
                representation: Some(symbol.id),
                unit: Some(sparksmith_core::UnitId(1)),
                position: Some(PointNm::from_mm_f64_rounded(at)?),
                physical: Some(sparksmith_core::PhysicalChoice {
                    binding: binding.id,
                    footprint: footprint.id,
                }),
            },
        )))
    };
    // A pin on the SIG wire, which runs from R1 pin 2 at (20, −23.81)
    // to (25, −23.81) on our sheet: refused, naming its net and the pin.
    let pin = engine
        .project()
        .library
        .symbol(symbol)
        .and_then(|s| s.definition.units.first())
        .and_then(|u| u.pins.first())
        .ok_or("a pin")?
        .position
        .to_mm_f64();
    let sig = net_of(engine.project(), "R1", "2").ok_or("SIG")?;
    let sig = engine.project().net_name(sig).to_owned();
    let revision = engine.revision();
    let refused = engine
        .apply(revision, place([22.0 - pin[0], -23.81 - pin[1]])?)
        .err()
        .ok_or("a pin on the routed wire joins SIG")?
        .to_string();
    assert!(
        refused.contains(&format!("routed copper on {sig} (U1 pin "))
            && refused.contains(&format!(" joins {sig})")),
        "{refused}"
    );
    assert_eq!(engine.revision(), revision);
    // Clear of the wires it places; no routed net changes.
    let before = engine.project().nets.clone();
    engine.apply(engine.revision(), place([100.0, -100.0])?)?;
    for net in &before {
        let after = engine.project().net(net.id).ok_or("a net went")?;
        assert_eq!(after.terminals, net.terminals, "{}", net.name);
    }
    Ok(())
}

/// A partly laid out project: R2 is on the schematic with no footprint on
/// the board. It comes in as a part on the sheet where KiCad draws it,
/// wired as KiCad wires it, with no footprint chosen; after the project is
/// saved and reopened, Issues still lists it as having no footprint. The
/// board and schematic do not disagree over it: it is not on the board.
#[test]
fn a_symbol_without_a_footprint_is_kept_as_a_part_without_one() -> Fallible<()> {
    let board = row_board(&[("R1", &[("1", Some((1, "A"))), ("2", Some((2, "SIG")))])]);
    // R1.2 at (50, 53.81) wired to R2.1 at (60, 46.19).
    let schematic = sheet(
        &[("R1", "50 50 0"), ("R2", "60 50 0")],
        "(wire (pts (xy 50 53.81) (xy 55 53.81) (xy 55 46.19) (xy 60 46.19)))",
    );
    let imported = import_sheet(&board, schematic)?;
    let report = &imported.report;
    assert_eq!(report.count(Case::SymbolWithoutFootprint), 1);
    assert_eq!(report.count(Case::SymbolNotDrawn), 0);
    let detail = report
        .details()
        .iter()
        .find(|d| d.case == Case::SymbolWithoutFootprint)
        .ok_or("no detail naming the part")?;
    assert_eq!(detail.footprint, "R2");
    let project = imported.engine.project();
    let r2 = project.instance_by_designator("R2").ok_or("R2 dropped")?;
    assert!(r2.physical.is_none());
    assert!(project.placement_of(r2.id).is_none());
    // On the sheet where KiCad has it: (60, 50) is (30, −20) on ours.
    let occurrence = project.occurrences_of(r2.id).next().ok_or("on no sheet")?;
    let at = occurrence.position.to_mm_f64();
    assert!(
        (at[0] - 30.0).abs() < 1e-9 && (at[1] + 20.0).abs() < 1e-9,
        "{at:?}"
    );
    // Wired as KiCad wires it, and the source's nets come in whole.
    assert!(net_of(project, "R1", "2").is_some());
    assert_eq!(net_of(project, "R1", "2"), net_of(project, "R2", "1"));
    assert_eq!(
        (report.counts.source_nets, report.counts.source_nets_equal),
        (3, 3)
    );
    assert!(project.net_disagreements.is_empty());
    assert_eq!(report.count(Case::BoardSplitsNet), 0);
    // Saved and reopened, Issues names it as having no footprint.
    let file = sparksmith_core::format::EditorFile::from_document(imported.engine.document());
    let bytes = sparksmith_core::format::encode(&file)?;
    let reopened = sparksmith_core::format::decode(&bytes)?;
    let rows = issue_rows(&reopened.project)?;
    let row = rows
        .iter()
        .find(|(code, _)| code == "instance.no_physical_choice")
        .ok_or("no Issues row for the part without a footprint")?;
    assert!(row.1.contains("R2"), "{}", row.1);
    Ok(())
}

/// The hierarchy is the files its sheets name: from the project's folder or
/// from the naming sheet's, never outside the project, never a file no
/// sheet names, and never one of two that could be meant.
#[test]
fn a_hierarchy_is_collected_by_reference() -> Fallible<()> {
    use sparksmith_kicad::schematic::{self, SchematicError, SchematicFiles};
    let sheet = |files: &[&str]| {
        let sheets: String = files
            .iter()
            .map(|f| {
                format!(
                    "(sheet (at 50 50) (size 20 10) (uuid \"u\") (property \"Sheetfile\" \"{f}\"))"
                )
            })
            .collect();
        format!("(kicad_sch (version 20231120) (uuid \"r\") {sheets})").into_bytes()
    };
    // What is read of `folder`, by name, and the sheets of the hierarchy.
    let read = |folder: &[(&str, Vec<u8>)]| {
        let mut asked = Vec::new();
        let files = schematic::collect("root.kicad_sch", |name| {
            asked.push(name.to_owned());
            let found = folder.iter().find(|(n, _)| *n == name);
            Ok::<_, std::convert::Infallible>(found.map(|(_, bytes)| bytes.clone()))
        })
        .unwrap_or_default();
        let sheets = schematic::read(SchematicFiles {
            root: "root.kicad_sch",
            files: &files,
        })
        .map(|s| s.sheets.into_iter().map(|s| s.file).collect::<Vec<_>>());
        (asked, sheets)
    };
    let folder = [
        (
            "root.kicad_sch",
            sheet(&["./a.kicad_sch", "sub\\\\b.kicad_sch"]),
        ),
        ("a.kicad_sch", sheet(&[])),
        ("sub/b.kicad_sch", sheet(&["c.kicad_sch", "../a.kicad_sch"])),
        ("sub/c.kicad_sch", sheet(&["sub/b2.kicad_sch"])),
        ("sub/b2.kicad_sch", sheet(&[])),
        ("other.kicad_sch", sheet(&["more.kicad_sch"])),
    ];
    let (asked, sheets) = read(&folder);
    assert_eq!(
        sheets,
        Ok([
            "root.kicad_sch",
            "a.kicad_sch",
            "sub/b.kicad_sch",
            "sub/c.kicad_sch",
            "sub/b2.kicad_sch",
            "a.kicad_sch"
        ]
        .map(str::to_owned)
        .to_vec())
    );
    assert!(
        !asked
            .iter()
            .any(|n| n.contains("other") || n.contains("more"))
    );

    for outside in [
        "../out.kicad_sch",
        "/out.kicad_sch",
        "C:\\\\out.kicad_sch",
        "",
    ] {
        let (asked, sheets) = read(&[("root.kicad_sch", sheet(&[outside]))]);
        assert_eq!(asked, ["root.kicad_sch"], "{outside}");
        assert_eq!(sheets, Err(SchematicError::Missing), "{outside}");
    }
    let (_, two) = read(&[
        ("root.kicad_sch", sheet(&["sub/b.kicad_sch"])),
        ("sub/b.kicad_sch", sheet(&["c.kicad_sch"])),
        ("c.kicad_sch", sheet(&[])),
        ("sub/c.kicad_sch", sheet(&[])),
    ]);
    assert!(matches!(two, Err(SchematicError::Value(_))), "{two:?}");
    // A sheet that names itself is read once and refused as too deep.
    let (asked, itself) = read(&[("root.kicad_sch", sheet(&["root.kicad_sch"]))]);
    assert_eq!(asked.len(), 1);
    assert!(
        matches!(itself, Err(SchematicError::Limit(_))),
        "{itself:?}"
    );
    Ok(())
}

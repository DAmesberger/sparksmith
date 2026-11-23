//! KiCad's not-fitted states import as the typed default population
//! (docs/assembly-variants.md, Import): the do-not-populate attribute and
//! a value that is or carries a not-fitted marker, which leaves the value.
//! Exclude-from-BOM (test points, mechanical items, logos) is not one.
//! Synthetic boards only.
mod support;

use sparksmith_core::model::Population;
use sparksmith_kicad::report::Case;
use std::collections::BTreeMap;
use support::{Board, Footprint, Pad};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// Two-pad parts in a row: (reference, value, extra footprint tokens).
fn board(parts: &[(&str, &str, &str)]) -> Board {
    let footprints = parts
        .iter()
        .enumerate()
        .map(|(k, (reference, value, extra))| {
            let pads = vec![
                Pad::smd("1", "rect", [-1.0, 0.0, 0.0], [0.8, 0.6]).with("(net 1 \"A\")"),
                Pad::smd("2", "rect", [1.0, 0.0, 0.0], [0.8, 0.6]).with("(net 2 \"B\")"),
            ];
            let mut fp = Footprint::new(reference, [10.0 + 8.0 * k as f64, 10.0, 0.0], false, pads);
            fp.value = Some((*value).to_owned());
            fp.extra = (*extra).to_owned();
            fp
        })
        .collect();
    Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints,
        items: "  (net 1 \"A\")\n  (net 2 \"B\")\n  (gr_rect (start 0 0) (end 80 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))\n".into(),
    }
}

/// Each designator's default population and value text.
fn states(
    imported: &sparksmith_kicad::Imported,
    designators: &[&str],
) -> Fallible<Vec<(Population, Option<String>)>> {
    let project = imported.engine.project();
    designators
        .iter()
        .map(|d| {
            let instance = project.instance_by_designator(d).ok_or(*d)?;
            Ok((
                instance.population,
                sparksmith_core::instances::value_text(&project.library, instance)?,
            ))
        })
        .collect()
}

/// The report's details of `case`, as (designator, reason).
fn details(imported: &sparksmith_kicad::Imported, case: Case) -> Vec<(String, String)> {
    imported
        .report
        .details()
        .iter()
        .filter(|d| d.case == case)
        .map(|d| (d.footprint.clone(), d.path.clone()))
        .collect()
}

#[test]
fn board_attributes_and_marker_values_import_as_not_populated() -> Fallible<()> {
    let source = board(&[
        ("R1", "10k", " (attr smd dnp)"),
        ("R2", "22k", " (attr smd exclude_from_bom)"),
        ("R3", "DNP", ""),
        ("R4", "10k DNP", ""),
        ("R5", "(dnf) 4k7", ""),
        ("R6", "NC", ""),
        ("R7", "1k", " (attr smd exclude_from_pos_files)"),
        ("R8", "N/F", ""),
    ]);
    let imported = support::import(source.text().as_bytes())?;
    let not = Population::NotPopulated;
    let fitted = Population::Populated;
    let some = |v: &str| Some(v.to_owned());
    assert_eq!(
        states(&imported, &["R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8"])?,
        vec![
            (not, some("10k")),
            (fitted, some("22k")),
            (not, None),
            (not, some("10k")),
            (not, some("4k7")),
            (fitted, some("NC")),
            (fitted, some("1k")),
            (not, None),
        ]
    );
    // No marker text survives in a component's name either.
    let project = imported.engine.project();
    assert!(
        project.library.components.iter().all(|c| {
            let name = c.meta.name.to_ascii_uppercase();
            !name.contains("DNP") && !name.contains("DNF") && !name.contains("N/F")
        }),
        "a marker in a part name"
    );
    // A part marked do-not-populate keeps its body (a variant may fit it);
    // KiCad's exclude-from-BOM and exclude-from-position-files attributes,
    // on a part with no model, say nothing is assembled there, so they
    // keep making no body, as before variants.
    let no_body: Vec<String> = details(&imported, Case::NoBody)
        .into_iter()
        .map(|(d, _)| d)
        .collect();
    assert_eq!(no_body, ["R2", "R7"]);
    assert_eq!(
        details(&imported, Case::NotPopulatedAttribute),
        vec![(
            "R1".to_owned(),
            "KiCad's do-not-populate attribute".to_owned()
        )]
    );
    let marker: Vec<String> = details(&imported, Case::NotPopulatedMarker)
        .into_iter()
        .map(|(d, _)| d)
        .collect();
    assert_eq!(marker, ["R3", "R4", "R5", "R8"]);
    assert_eq!(imported.report.count(Case::VariantsNotImported), 0);
    let rows = imported.report.rows();
    let row = rows
        .iter()
        .find(|r| r.case == Case::NotPopulatedMarker)
        .ok_or("marker row")?;
    assert_eq!(row.count, 4);
    Ok(())
}

/// A two-pin symbol and placed symbols with extra tokens each.
fn sheet(symbols: &[(&str, &str, &str)]) -> String {
    let placed: String = symbols
        .iter()
        .enumerate()
        .map(|(k, (reference, value, extra))| {
            format!(
                "(symbol (lib_id \"Synthetic:R2\") (at {} 50 0) (unit 1) {extra}
                   (property \"Reference\" \"{reference}\" (at 0 0 0))
                   (property \"Value\" \"{value}\" (at 0 0 0))
                   (instances (project \"synthetic\" (path \"/00000000-0000-4000-8000-000000000002\" (reference \"{reference}\") (unit 1)))))\n",
                20 + 20 * k
            )
        })
        .collect();
    format!(
        "(kicad_sch (version 20231120) (generator \"sparksmith_test\")
  (uuid \"00000000-0000-4000-8000-000000000002\")
  (paper \"A4\")
  (lib_symbols (symbol \"Synthetic:R2\" (in_bom yes) (on_board yes)
      (symbol \"R2_1_1\"
        (pin passive line (at 0 3.81 270) (length 1.27) (name \"~\") (number \"1\"))
        (pin passive line (at 0 -3.81 90) (length 1.27) (name \"~\") (number \"2\")))))
  {placed}
)"
    )
}

#[test]
fn schematic_attributes_import_as_not_populated() -> Fallible<()> {
    let source = board(&[
        ("R1", "1k", ""),
        ("R2", "1k", ""),
        ("R3", "1k", ""),
        ("R4", "1k", ""),
    ]);
    let mut files = BTreeMap::new();
    files.insert(
        "synthetic.kicad_sch".to_owned(),
        sheet(&[
            ("R1", "1k", "(in_bom yes) (on_board yes) (dnp yes)"),
            ("R2", "2k", "(in_bom no) (on_board yes) (dnp no)"),
            ("R3", "3k DNP", "(in_bom yes) (on_board yes) (dnp yes)"),
            ("R4", "4k", "(in_bom yes) (on_board yes) (dnp no)"),
        ])
        .into_bytes(),
    );
    let imported = support::import_with_schematic(
        source.text().as_bytes(),
        None,
        Some(("synthetic.kicad_sch", &files)),
    )?;
    let some = |v: &str| Some(v.to_owned());
    assert_eq!(
        states(&imported, &["R1", "R2", "R3", "R4"])?,
        vec![
            (Population::NotPopulated, some("1k")),
            // `(in_bom no)` marks a test point, a mechanical item or a
            // logo, not a part left off.
            (Population::Populated, some("2k")),
            (Population::NotPopulated, some("3k")),
            (Population::Populated, some("4k")),
        ]
    );
    // An attribute names the part once, before its value's marker.
    let attribute = "KiCad's do-not-populate attribute".to_owned();
    assert_eq!(
        details(&imported, Case::NotPopulatedAttribute),
        vec![
            ("R1".to_owned(), attribute.clone()),
            ("R3".to_owned(), attribute)
        ]
    );
    assert!(details(&imported, Case::NotPopulatedMarker).is_empty());
    Ok(())
}

#[test]
fn variant_data_is_reported_not_imported() -> Fallible<()> {
    let source = board(&[("R1", "1k", "")]);
    let project = br#"{"meta": {"version": 3}, "variants": [{"name": "Lite"}]}"#;
    let imported = support::import_with_project(source.text().as_bytes(), Some(project))?;
    assert_eq!(imported.report.count(Case::VariantsNotImported), 1);
    let plain = support::import(source.text().as_bytes())?;
    assert_eq!(plain.report.count(Case::VariantsNotImported), 0);
    // An empty list of variants holds none.
    let none = br#"{"meta": {"version": 3}, "schematic": {"variants": []}}"#;
    let empty = support::import_with_project(source.text().as_bytes(), Some(none))?;
    assert_eq!(empty.report.count(Case::VariantsNotImported), 0);
    Ok(())
}

/// Review §14: KiCad's exclude-from-BOM and exclude-from-position-files
/// are kept apart, per placed copy, whatever the source order: a copy kept
/// off the BOM is still placed, one kept out of the position files is
/// still bought, and whether a copy without a model gets a body is its
/// own (copies of one footprint never take another copy's).
#[test]
fn bom_and_placement_exclusions_are_each_copys_own_in_either_order() -> Fallible<()> {
    let parts = [
        ("U1", ""),
        ("U2", " (attr smd exclude_from_bom)"),
        ("U3", " (attr smd exclude_from_pos_files)"),
        ("U4", " (attr smd exclude_from_bom exclude_from_pos_files)"),
    ];
    for order in [parts.to_vec(), parts.iter().rev().copied().collect()] {
        let source = board(
            &order
                .iter()
                .map(|(r, extra)| (*r, "", *extra))
                .collect::<Vec<_>>(),
        );
        let imported = support::import(source.text().as_bytes())?;
        let project = imported.engine.project();
        let flags = |d: &str| -> Fallible<(bool, bool)> {
            let i = project.instance_by_designator(d).ok_or(d.to_owned())?;
            Ok((i.bom_excluded, i.placement_excluded))
        };
        assert_eq!(flags("U1")?, (false, false), "{order:?}");
        assert_eq!(flags("U2")?, (true, false), "{order:?}");
        assert_eq!(flags("U3")?, (false, true), "{order:?}");
        assert_eq!(flags("U4")?, (true, true), "{order:?}");
        let files =
            sparksmith_core::fabrication::assembly::files(project, &Default::default(), "t")?;
        let text = |suffix: &str| -> String {
            files
                .iter()
                .filter(|f| f.name.ends_with(suffix))
                .map(|f| String::from_utf8_lossy(&f.bytes).into_owned())
                .collect()
        };
        let placement = text("pos-top.csv");
        let bom = text("bom-parts.csv");
        for (d, in_bom, placed) in [
            ("U1", true, true),
            ("U2", false, true),
            ("U3", true, false),
            ("U4", false, false),
        ] {
            assert_eq!(
                placement.contains(&format!("\n{d},")),
                placed,
                "{d}: {placement}"
            );
            assert_eq!(bom.contains(&format!("\n{d},")), in_bom, "{d}: {bom}");
        }
        let mut no_body: Vec<String> = details(&imported, Case::NoBody)
            .into_iter()
            .map(|(d, _)| d)
            .collect();
        no_body.sort();
        assert_eq!(no_body, ["U2", "U3", "U4"], "{order:?}");
    }
    Ok(())
}

/// Review §13: a part on the schematic only keeps its do-not-populate
/// attribute and its value without the marker, as a placed part does, and
/// its pins keep their KiCad electrical types (a tri-state pin, which has
/// no equivalent, is imported passive and named).
#[test]
fn schematic_only_parts_keep_population_and_pin_types() -> Fallible<()> {
    let source = board(&[("U1", "10k DNP", " (attr smd dnp)")]);
    let placed: String = ["U1", "U2"]
        .iter()
        .enumerate()
        .map(|(k, reference)| {
            format!(
                "(symbol (lib_id \"Synthetic:Buffer\") (at {} 50 0) (unit 1) (in_bom yes) (on_board yes) (dnp yes)
                   (property \"Reference\" \"{reference}\" (at 0 0 0))
                   (property \"Value\" \"10k DNP\" (at 0 0 0))
                   (instances (project \"synthetic\" (path \"/00000000-0000-4000-8000-000000000002\" (reference \"{reference}\") (unit 1)))))\n",
                20 + 20 * k
            )
        })
        .collect();
    let sheet = format!(
        "(kicad_sch (version 20231120) (generator \"sparksmith_test\")
  (uuid \"00000000-0000-4000-8000-000000000002\")
  (paper \"A4\")
  (lib_symbols (symbol \"Synthetic:Buffer\" (in_bom yes) (on_board yes)
      (symbol \"Buffer_1_1\"
        (pin output line (at 0 3.81 270) (length 1.27) (name \"OUT\") (number \"1\"))
        (pin input line (at 0 -3.81 90) (length 1.27) (name \"IN\") (number \"2\"))
        (pin tri_state line (at 3.81 0 180) (length 1.27) (name \"OE\") (number \"3\")))))
  {placed}
)"
    );
    let mut files = BTreeMap::new();
    files.insert("synthetic.kicad_sch".to_owned(), sheet.into_bytes());
    let imported = support::import_with_schematic(
        source.text().as_bytes(),
        None,
        Some(("synthetic.kicad_sch", &files)),
    )?;
    let some = |v: &str| Some(v.to_owned());
    assert_eq!(
        states(&imported, &["U1", "U2"])?,
        vec![
            (Population::NotPopulated, some("10k")),
            (Population::NotPopulated, some("10k")),
        ]
    );
    let project = imported.engine.project();
    let u2 = project.instance_by_designator("U2").ok_or("U2")?;
    assert!(u2.physical.is_none(), "U2 is on the schematic only");
    let component = project.library.component(u2.component).ok_or("U2's part")?;
    use sparksmith_core::functions::Direction;
    let directions: Vec<(String, Option<Direction>)> = component
        .definition
        .terminals
        .iter()
        .map(|t| {
            (
                t.name.clone(),
                t.functions()
                    .and_then(|f| f.functions.first())
                    .map(|f| f.direction),
            )
        })
        .collect();
    assert_eq!(
        directions,
        [
            ("1".to_owned(), Some(Direction::Output)),
            ("2".to_owned(), Some(Direction::Input)),
            ("3".to_owned(), Some(Direction::Passive)),
        ]
    );
    let kept: Vec<(String, String)> = imported
        .report
        .details()
        .iter()
        .filter(|d| d.case == Case::PinTypeKept && d.footprint == "U2")
        .map(|d| (d.pad.clone(), d.path.clone()))
        .collect();
    assert_eq!(kept, [("3".to_owned(), "tri_state".to_owned())]);
    Ok(())
}

//! The importer's parts on original synthetic boards: generated parts
//! whose copper is the source's, typed values, part numbers, project parts
//! of source lands, and the findings.
use super::*;
use sparksmith_core::instances;
use sparksmith_core::layout::bodies;
use sparksmith_core::library::PackageModel;
use sparksmith_core::units::Nm;
use sparksmith_kicad::models::Reference;
type Fallible = Result<(), Box<dyn std::error::Error>>;

const VENDOR: &[u8] = b"Original synthetic vendor bytes; never parsed.";

fn reference(path: &str) -> Reference {
    Reference {
        path: path.into(),
        offset_nm: [Nm::ZERO; 3],
        rotation_deg: [0.0; 3],
        scale: [1.0; 3],
        hidden: false,
    }
}

/// A board of `footprints` (KiCad text) on two layers with nets A–D.
fn board(name: &str, footprints: &str) -> String {
    format!(
        r#"(kicad_pcb (version 20240108) (generator "{name}")
        (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C") (net 4 "D")
        (gr_rect (start 0 0) (end 120 40) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
        {footprints})"#
    )
}

fn import(dir: &Path, name: &str, text: &str) -> Result<crate::kicad_import::KicadImport, String> {
    let path = dir.join(format!("{name}.kicad_pcb"));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    crate::kicad_import::import_in_fixture_pool(&path, None)
}

/// A six-lead SOT land pattern written as KiCad stores it (y down): pins
/// 1–3 along +y, 4–6 along −y. A back-side footprint's pads are stored as
/// seen from the top; every pad's angle includes the footprint's.
fn sot_footprint(name: &str, at: [f64; 2], angle: f64, back: bool, extra: &str) -> String {
    let flip = if back { -1.0 } else { 1.0 };
    let (cu, paste, mask) = if back {
        ("B.Cu", "B.Paste", "B.Mask")
    } else {
        ("F.Cu", "F.Paste", "F.Mask")
    };
    let mut out = format!(
        r#"(footprint "Original:Six" (layer "{cu}") (at {} {} {angle})
        (property "Reference" "{name}") {extra}"#,
        at[0], at[1]
    );
    for (number, x, y, net) in [
        (1, -0.95, 1.3, 1),
        (2, 0.0, 1.3, 2),
        (3, 0.95, 1.3, 3),
        (4, 0.95, -1.3, 4),
        (5, 0.0, -1.3, 1),
        (6, -0.95, -1.3, 2),
    ] {
        out.push_str(&format!(
            r#"(pad "{number}" smd roundrect (at {x} {} {angle}) (size 0.6 1.0) (roundrect_rratio 0.25) (layers "{cu}" "{paste}" "{mask}") (net {net} "{}"))"#,
            flip * y,
            ["", "A", "B", "C", "D"][net]
        ));
    }
    out.push(')');
    out
}

/// Two chip lands 1.5 mm apart (a 0603 chip's), with `value`.
fn chip_footprint(name: &str, x: f64, value: &str) -> String {
    format!(
        r#"(footprint "Original:Chip" (layer "F.Cu") (at {x} 30)
        (property "Reference" "{name}") (property "Value" "{value}")
        (pad "1" smd roundrect (at -0.75 0) (size 0.85 0.95) (roundrect_rratio 0.25) (layers "F.Cu" "F.Paste" "F.Mask") (net 1 "A"))
        (pad "2" smd roundrect (at 0.75 0) (size 0.85 0.95) (roundrect_rratio 0.25) (layers "F.Cu" "F.Paste" "F.Mask") (net 2 "B")))"#
    )
}

fn generated_root(project: &sparksmith_core::Project, designator: &str) -> Option<String> {
    let instance = project.instance_by_designator(designator)?;
    project
        .library
        .generated
        .iter()
        .find(|g| g.root.id == instance.component.id)
        .map(|g| g.from.generator.clone())
}

/// The founder's rule: an import makes real generated parts, the same
/// way the palette does, its land pattern the source's lands, so the
/// board's copper is the source's on both sides at every rotation, and
/// each body's leads land their pads.
#[test]
fn source_lands_become_generated_parts_with_the_sources_copper() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    let mut footprints = String::new();
    let mut n = 0;
    for back in [false, true] {
        for angle in [0.0, 90.0, 180.0, 270.0, 30.0] {
            n += 1;
            let at = [10.0 * f64::from(n), if back { 25.0 } else { 10.0 }];
            footprints.push_str(&sot_footprint(&format!("U{n}"), at, angle, back, ""));
        }
    }
    let imported = import(
        dir.path(),
        "generated",
        &board("generated parts", &footprints),
    )?;
    let project = imported.engine.project();
    assert_eq!(imported.report.count(Case::GeneratedPart), 10);
    assert_eq!(imported.report.count(Case::SourcePart), 0);
    for k in 1..=n {
        let designator = format!("U{k}");
        assert_eq!(
            generated_root(project, &designator).as_deref(),
            Some("tabbed"),
            "{designator}"
        );
    }
    // One generated part serves every copy; no import-only definitions.
    assert_eq!(project.library.generated.len(), 1);
    // Its fields are the standard fields' copies, none of the import's.
    let standard = context.standards().fields.library();
    assert!(
        project.library.fields.iter().all(|f| {
            project
                .library
                .origins
                .iter()
                .any(|o| o.local.id == f.id && o.origin.library == standard)
        }),
        "no import-only field"
    );
    // The copper: on every layer exactly the copper of the source's lands
    // imported as they are.
    let path = dir.path().join("generated.kicad_pcb");
    let lands = crate::kicad_import::import_source_lands(&path)?;
    assert_eq!(lands.report.count(Case::GeneratedPart), 0);
    for layer in
        sparksmith_solidcraft::project::copper_agree::between(lands.engine.project(), project)
    {
        assert!(
            layer.objects_differ_mm2 < 1e-9 && layer.fills_differ_mm2 < 1e-9,
            "{layer:?}"
        );
    }
    // Every body's leads land their pads (the board check).
    let report = bodies::bodies(project);
    assert_eq!(report.parts.len(), 10, "{:?}", report.without_leads);
    for part in &report.parts {
        assert_eq!(part.off_pads, 0, "{}: {:?}", part.designator, part.leads);
        assert_eq!(part.pin_one, bodies::PinOne::Agrees, "{}", part.designator);
    }
    assert!(bodies::checks(project).is_empty());
    Ok(())
}

/// A generated part reopens: the check on load runs its generator with
/// the recorded parameters (the land pattern among them) and finds it.
#[test]
fn an_imported_generated_part_passes_the_check_on_load() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    let imported = import(
        dir.path(),
        "reopen",
        &board(
            "reopen",
            &sot_footprint("U1", [10.0, 10.0], 90.0, false, ""),
        ),
    )?;
    let path = dir.path().join("reopen.sparksmith");
    sparksmith_io::write_project(&path, &imported.engine.to_file())?;
    let reopened = sparksmith_io::read_project(&path)?;
    assert_eq!(reopened.project.library.generated.len(), 1);
    Ok(())
}

/// KiCad values become the generated part's typed fields, parsed as a
/// value typed into a palette part is: a resistor's Resistance, a
/// capacitor's Capacitance and Rated voltage.
#[test]
fn values_become_the_generated_parts_typed_fields() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    let footprints = [
        chip_footprint("R1", 10.0, "4k7"),
        chip_footprint("R2", 20.0, "10k"),
        chip_footprint("C1", 30.0, "100n 16V"),
        sot_footprint(
            "U1",
            [40.0, 10.0],
            0.0,
            false,
            r#"(property "Value" "ORIG-REG 3.3V")"#,
        ),
    ]
    .concat();
    let imported = import(dir.path(), "values", &board("values", &footprints))?;
    let project = imported.engine.project();
    let fields = |designator: &str| -> Result<Vec<(String, String)>, String> {
        let instance = project
            .instance_by_designator(designator)
            .ok_or(designator.to_owned())?;
        Ok(instances::effective_fields(&project.library, instance)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter_map(|f| Some((f.label, f.text?)))
            .collect())
    };
    assert_eq!(generated_root(project, "R1").as_deref(), Some("chip"));
    assert!(
        fields("R1")?.contains(&("Resistance".into(), "4.7 kΩ".into())),
        "{:?}",
        fields("R1")?
    );
    assert!(
        fields("R2")?.contains(&("Resistance".into(), "10 kΩ".into())),
        "{:?}",
        fields("R2")?
    );
    let c1 = fields("C1")?;
    assert!(c1.iter().any(|(f, _)| f == "Capacitance"), "{c1:?}");
    assert!(c1.iter().any(|(f, _)| f == "Rated voltage"), "{c1:?}");
    // R1 and R2 are one generated resistor, each with its own value.
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let r2 = project.instance_by_designator("R2").ok_or("R2")?;
    assert_eq!(r1.component, r2.component);
    // A generated part of no family holds its value whole in the standard
    // part value field: the schematic and the BOM show it.
    assert_eq!(
        fields("U1")?,
        vec![("Value".to_owned(), "ORIG-REG 3.3V".to_owned())]
    );
    let u1 = project.instance_by_designator("U1").ok_or("U1")?;
    assert_eq!(
        instances::value_text(&project.library, u1)?.as_deref(),
        Some("ORIG-REG 3.3V")
    );
    let bom = sparksmith_core::completeness::bom(project)?;
    assert!(
        bom.lines
            .iter()
            .any(|l| l.designators == ["U1"] && l.values.iter().any(|v| v.text == "ORIG-REG 3.3V")),
        "{bom:?}"
    );
    assert_eq!(imported.report.count(Case::ValueField), 4);
    assert_eq!(imported.report.count(Case::ValueWithoutField), 0);
    Ok(())
}

/// A part number with a manufacturer becomes the placed part's
/// manufacturer part; one naming another package than the lands are, or
/// a variant of the matched package's name, is a finding naming both, and
/// the lands stay as the source has them.
#[test]
fn a_part_number_naming_another_package_is_a_finding() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    let props = |mpn: &str| {
        format!(r#"(property "MPN" "{mpn}") (property "Manufacturer" "Original Devices")"#)
    };
    let footprints = [
        // Agrees: the matched package's alias, standing apart.
        sot_footprint("U1", [10.0, 10.0], 0.0, false, &props("SOT-23-6-TR")),
        // Another package's name.
        sot_footprint("U2", [20.0, 10.0], 0.0, false, &props("SOIC-8-TR")),
        // The matched package's name run on into a variant.
        sot_footprint("U3", [30.0, 10.0], 0.0, false, &props("SOT-23-6X")),
    ]
    .concat();
    let imported = import(dir.path(), "mpn", &board("mpn", &footprints))?;
    let found: Vec<_> = imported
        .report
        .details()
        .iter()
        .filter(|d| d.case == Case::PackageContradiction)
        .map(|d| (d.footprint.clone(), d.path.clone()))
        .collect();
    assert_eq!(found.len(), 2, "{found:?}");
    let u2 = found.iter().find(|(f, _)| f == "U2").ok_or("U2 finding")?;
    assert!(
        u2.1.contains("gull-wing/gull-wing-soic-8") && u2.1.contains("tabbed/tabbed-sot-23-6"),
        "{}",
        u2.1
    );
    let u3 = found.iter().find(|(f, _)| f == "U3").ok_or("U3 finding")?;
    assert!(
        u3.1.contains("variant") && u3.1.contains("tabbed/tabbed-sot-23-6"),
        "{}",
        u3.1
    );
    // The lands are kept: every copy is the generated SOT part.
    let project = imported.engine.project();
    for designator in ["U1", "U2", "U3"] {
        assert_eq!(
            generated_root(project, designator).as_deref(),
            Some("tabbed")
        );
        let instance = project
            .instance_by_designator(designator)
            .ok_or(designator)?;
        let part = instance
            .part
            .and_then(|p| project.library.part(p))
            .ok_or("part")?;
        assert_eq!(part.definition.manufacturer, "Original Devices");
    }
    Ok(())
}

/// A footprint no generator fits is a project part of its lands, the same
/// kind of part as an authored one: no import-only field, the package's
/// default model until a recreation, named by its value.
#[test]
fn a_footprint_no_generator_fits_is_a_project_part_of_its_lands() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("own.step"), VENDOR)?;
    let footprint = r#"(footprint "Original:Odd" (layer "F.Cu") (at 20 10)
        (property "Reference" "X1") (property "Value" "Odd part")
        (pad "1" smd rect (at 0 0) (size 1.3 0.7) (layers "F.Cu" "F.Mask" "F.Paste") (net 1 "A"))
        (pad "2" smd rect (at 3.1 0.9) (size 0.4 2.2) (layers "F.Cu" "F.Mask" "F.Paste") (net 2 "B"))
        (pad "3" smd rect (at -2.2 1.7) (size 0.9 0.9) (layers "F.Cu" "F.Mask" "F.Paste") (net 3 "C"))
        (model "own.step"))"#;
    let imported = import(dir.path(), "odd", &board("odd", footprint))?;
    let project = imported.engine.project();
    assert_eq!(imported.report.count(Case::SourcePart), 1);
    let instance = project.instance_by_designator("X1").ok_or("X1")?;
    let resolved = instances::resolve(&project.library, instance)?;
    assert_eq!(resolved.component.meta.name, "Odd Odd part");
    // The part selects the standard part value field, which holds its
    // value: no import-only field.
    let value = context
        .standards()
        .fields
        .part_value_selection(&project.library)
        .ok_or("the part value field is imported")?;
    assert_eq!(resolved.component.definition.fields, vec![value]);
    assert_eq!(
        instances::value_text(&project.library, instance)?.as_deref(),
        Some("Odd part")
    );
    assert_eq!(imported.report.count(Case::ValueField), 1);
    assert_eq!(imported.report.count(Case::ValueWithoutField), 0);
    let physical = resolved.physical.ok_or("physical")?;
    assert_eq!(physical.package.definition.model, PackageModel::default());
    assert!(
        project.library.models.is_empty(),
        "the vendor file is never the body"
    );
    let detail = imported
        .report
        .details()
        .iter()
        .find(|d| d.footprint == "X1" && d.case == Case::SourcePart)
        .ok_or("X1 detail")?;
    assert_eq!(detail.placeholder, Some(Placeholder::VendorReferenceFound));
    assert!(
        detail.path.contains("never used as the body"),
        "{}",
        detail.path
    );
    Ok(())
}

/// A generator terminal never joins pads the board keeps on different
/// nets: an exposed pad split in two numbered alike but on two nets stays
/// a project part of its lands.
#[test]
fn a_generator_never_joins_pads_on_different_nets() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    // Pads 1–6 of the SOT pattern, and pad 6 again beside it on net C.
    let footprint = sot_footprint("U1", [10.0, 10.0], 0.0, false, "").replace(
        "(pad \"6\"",
        "(pad \"6\" smd rect (at -0.95 -1.9) (size 0.2 0.2) (layers \"F.Cu\") (net 3 \"C\")) (pad \"6\"",
    );
    let imported = import(dir.path(), "nets", &board("nets", &footprint))?;
    assert_eq!(imported.report.count(Case::GeneratedPart), 0);
    let detail = imported
        .report
        .details()
        .iter()
        .find(|d| d.case == Case::SourcePart)
        .ok_or("detail")?;
    assert!(
        detail.path.contains("different nets") || detail.path.contains("no shipped preset"),
        "{}",
        detail.path
    );
    Ok(())
}

/// A six-pin 1.0 mm side-entry header (JST's SH layout: 0.6 × 1.55 mm
/// lands, 1.2 × 1.8 mm tab lands 3.8 mm out and 3.875 mm back) whose two
/// tab lands carry different numbers, each with its `tabs` net text.
fn tabbed_header(name: &str, tabs: [&str; 2]) -> String {
    let mut out = format!(
        r#"(footprint "Original:SixTabs" (layer "F.Cu") (at 60 20)
        (property "Reference" "{name}")"#
    );
    for k in 0..6_usize {
        out.push_str(&format!(
            r#"(pad "{}" smd rect (at {} 0) (size 0.6 1.55) (layers "F.Cu" "F.Paste" "F.Mask") (net {} "{}"))"#,
            k + 1,
            -2.5 + k as f64,
            k % 4 + 1,
            ["A", "B", "C", "D"][k % 4]
        ));
    }
    for (k, (x, net)) in [(-3.8, tabs[0]), (3.8, tabs[1])].into_iter().enumerate() {
        out.push_str(&format!(
            r#"(pad "T{}" smd rect (at {x} 3.875 90) (size 1.8 1.2) (layers "F.Cu" "F.Paste" "F.Mask") {net})"#,
            k + 1
        ));
    }
    out.push(')');
    out
}

/// Two tab lands of different numbers share the header's mechanical
/// terminal when the board leaves both on no net (they join nothing), and
/// never when the board has one on a net and the other on none.
#[test]
fn unconnected_tabs_of_different_numbers_share_a_terminal() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    let free = import(
        dir.path(),
        "free",
        &board("free", &tabbed_header("J1", ["", ""])),
    )?;
    assert_eq!(free.report.count(Case::GeneratedPart), 1);
    assert_eq!(
        generated_root(free.engine.project(), "J1").as_deref(),
        Some("wire-to-board")
    );
    let one = import(
        dir.path(),
        "one",
        &board("one", &tabbed_header("J1", [r#"(net 1 "A")"#, ""])),
    )?;
    assert_eq!(one.report.count(Case::GeneratedPart), 0);
    Ok(())
}

/// Footprints that are no part to draw get no body, as their source has
/// it: no model reference, and not assembled or bare copper.
#[test]
fn footprints_that_are_no_part_get_no_body_as_in_the_source() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    let pad = |n: &str, x: f64, net: &str| {
        format!(r#"(pad "{n}" smd rect (at {x} 0) (size 0.9 0.9) (layers "F.Cu" "F.Mask") {net})"#)
    };
    let net = r#"(net 1 "A")"#;
    let mut footprints = String::new();
    for (name, x, attr, pads) in [
        (
            "JP1",
            10,
            "(attr smd exclude_from_pos_files)",
            format!("{}{}", pad("1", -0.6, net), pad("2", 0.6, net)),
        ),
        ("TP1", 20, "(attr smd)", pad("1", 0.0, net)),
        ("FID1", 30, "(attr smd)", pad("1", 0.0, "")),
        ("G1", 40, "(attr board_only)", String::new()),
    ] {
        footprints.push_str(&format!(
            r#"(footprint "Own:{name}" (layer "F.Cu") (at {x} 10) {attr}
            (property "Reference" "{name}") {pads})"#
        ));
    }
    let imported = import(dir.path(), "no-body", &board("no body", &footprints))?;
    let project = imported.engine.project();
    for designator in ["JP1", "TP1", "FID1", "G1"] {
        let instance = project
            .instance_by_designator(designator)
            .ok_or(designator)?;
        let model = instances::resolve(&project.library, instance)?
            .physical
            .map(|p| p.package.definition.model.clone());
        assert_eq!(model, Some(PackageModel::NoBody), "{designator}");
    }
    assert_eq!(imported.report.count(Case::NoBody), 4);
    Ok(())
}

/// A KiCad library model reference is never resolved, even beside a
/// same-named project file; an expired import changes nothing.
#[test]
fn library_references_stay_unresolved_and_an_expired_import_refuses() -> Fallible {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("own.step"), VENDOR)?;
    let mut models = Models::new(
        dir.path(),
        crate::session::generators::fixture_batch_runtime,
    )?;
    let mut engine = Engine::demo()?;
    let facts = sparksmith_kicad::models::Facts::default();
    let refs = [reference("${KICAD9_3DMODEL_DIR}/own.step")];
    let outcome = models.part(
        &mut engine,
        &PartRequest {
            pads: &[],
            prefix: "U",
            models: &refs,
            facts: &facts,
            nets: &[],
            mpn: None,
        },
    )?;
    assert!(matches!(outcome.made, Made::Source { .. }));
    assert!(models.sources.is_empty(), "never looked up");
    models.deadline = std::time::Instant::now();
    let before = engine.revision();
    let error = models
        .part(
            &mut engine,
            &PartRequest {
                pads: &[],
                prefix: "U",
                models: &[reference("own.step")],
                facts: &facts,
                nets: &[],
                mpn: None,
            },
        )
        .err()
        .ok_or("expired import succeeded")?;
    assert!(error.contains("time limit"), "{error}");
    assert_eq!(engine.revision(), before);
    Ok(())
}

/// The copper of two saved designs (`SPARKSMITH_COPPER_FILES=a:b`),
/// compared layer by layer, as evidence that a re-import or a body change
/// left the copper as it was: each layer's differing area is printed and
/// must be zero.
#[test]
#[ignore = "compares the designs named by SPARKSMITH_COPPER_FILES"]
fn two_saved_designs_have_the_same_copper() -> Fallible {
    let Ok(files) = std::env::var("SPARKSMITH_COPPER_FILES") else {
        return Err("set SPARKSMITH_COPPER_FILES=a.sparksmith:b.sparksmith".into());
    };
    let (a, b) = files.split_once(':').ok_or("two paths, `:`-separated")?;
    let context = sparksmith_io::standards::shipped_context()?;
    let _context = context.enter();
    let open = |path: &str| -> Result<sparksmith_core::Engine, Box<dyn std::error::Error>> {
        let mut engine = sparksmith_core::Engine::default();
        engine.replace_document(sparksmith_io::read_project(Path::new(path))?)?;
        Ok(engine)
    };
    let (a, b) = (open(a)?, open(b)?);
    let layers = sparksmith_solidcraft::project::copper_agree::between(a.project(), b.project());
    for layer in &layers {
        println!("{layer:?}");
    }
    assert!(
        layers
            .iter()
            .all(|l| l.objects_differ_mm2 == 0.0 && l.fills_differ_mm2 == 0.0)
    );
    Ok(())
}

/// Two imports of one board have the same copper on every layer; a moved
/// pad does not.
#[test]
fn the_copper_of_two_imports_is_compared_layer_by_layer() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    let text = board("twice", &sot_footprint("U1", [10.0, 10.0], 30.0, true, ""));
    let a = import(dir.path(), "a", &text)?;
    let b = import(dir.path(), "b", &text)?;
    for layer in sparksmith_solidcraft::project::copper_agree::between(
        a.engine.project(),
        b.engine.project(),
    ) {
        assert_eq!(layer.objects_differ_mm2, 0.0, "{layer:?}");
        assert_eq!(layer.fills_differ_mm2, 0.0, "{layer:?}");
    }
    let moved = import(dir.path(), "c", &text.replace("(at 0 1.3", "(at 0.1 1.3"))?;
    let differs: f64 = sparksmith_solidcraft::project::copper_agree::between(
        a.engine.project(),
        moved.engine.project(),
    )
    .iter()
    .map(|l| l.objects_differ_mm2)
    .sum();
    assert!(differs > 0.01, "{differs}");
    Ok(())
}

/// Each placed pad's net name by (designator, pad number, pad place).
fn pad_nets(project: &sparksmith_core::Project) -> BTreeMap<(String, String, [i64; 2]), String> {
    sparksmith_core::layout::placed_pads(project)
        .iter()
        .filter_map(|p| {
            let instance = project.instance(p.instance)?;
            let at = p
                .transform
                .apply(p.pad.position())
                .map(|v| (v * 1e4).round() as i64);
            Some((
                (instance.designator.clone(), p.pad.number.clone(), at),
                p.net
                    .map(|n| project.net_name(n).to_owned())
                    .unwrap_or_default(),
            ))
        })
        .collect()
}

/// The acceptance measure on a real design, run by hand with
/// `SPARKSMITH_KICAD_DESIGN` (a board, project or folder) and optionally
/// `SPARKSMITH_KICAD_PREVIOUS` (an earlier import's project file):
/// imports the design with generated parts and with every footprint a
/// project part of its lands, and prints counts and areas only (never a
/// name from the design): parts by generator, project parts by reason,
/// each layer's copper difference, the body check, pad nets and values.
#[test]
#[ignore = "needs a design named by SPARKSMITH_KICAD_DESIGN"]
fn acceptance_on_a_design() -> Fallible {
    let Some(design) = std::env::var_os("SPARKSMITH_KICAD_DESIGN") else {
        return Ok(());
    };
    let context = sparksmith_io::standards::shipped_context()?;
    let _context = context.enter();
    let design = std::path::PathBuf::from(design);
    let made = crate::kicad_import::import(&design, None)?;
    let lands = crate::kicad_import::import_source_lands(&design)?;
    let project = made.engine.project();
    let reference = lands.engine.project();
    // Parts by generator family (placed instances) and project parts by reason.
    let mut by_generator: BTreeMap<String, usize> = BTreeMap::new();
    for instance in &project.instances {
        let family = project
            .library
            .generated
            .iter()
            .find(|g| g.root.id == instance.component.id)
            .map(|g| g.from.generator.clone());
        *by_generator
            .entry(family.unwrap_or_else(|| "(project part of its lands)".into()))
            .or_default() += 1;
    }
    println!("instances by generator: {by_generator:?}");
    println!(
        "generated: {}, project parts of their lands: {}, no body: {}",
        made.report.count(Case::GeneratedPart),
        made.report.count(Case::SourcePart),
        made.report.count(Case::NoBody)
    );
    println!("project parts by reason: {:?}", made.report.placeholders());
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for detail in made
        .report
        .details()
        .iter()
        .filter(|d| d.case == Case::SourcePart)
    {
        // The reason's fixed words only (after the preset name).
        let why = detail
            .path
            .rsplit_once(", but ")
            .map(|(_, why)| why.split(['(', '"']).next().unwrap_or("").trim().to_owned())
            .unwrap_or_else(|| "no preset".into());
        *refused.entry(why).or_default() += 1;
    }
    println!("project-part reasons in words: {refused:?}");
    println!(
        "package contradictions: {}",
        made.report.count(Case::PackageContradiction)
    );
    let show =
        |what: &str, layers: Vec<sparksmith_solidcraft::project::copper_agree::LayerDifference>| {
            for l in layers {
                println!(
                    "{what} {:?}: objects differ {:.3e} mm², fills differ {:.3e} mm² (copper {:.3} / {:.3} mm², flattening allows {:.3e})",
                    l.layer,
                    l.objects_differ_mm2,
                    l.fills_differ_mm2,
                    l.a_mm2,
                    l.b_mm2,
                    l.allowed_mm2
                );
            }
        };
    show(
        "vs the source lands",
        sparksmith_solidcraft::project::copper_agree::between(reference, project),
    );
    if let Some(previous) = std::env::var_os("SPARKSMITH_KICAD_PREVIOUS") {
        match sparksmith_io::read_project(std::path::Path::new(&previous)) {
            Ok(file) => show(
                "vs the previous import",
                sparksmith_solidcraft::project::copper_agree::between(&file.project, project),
            ),
            Err(error) => println!(
                "the previous import does not open: {}",
                error.to_string().len()
            ),
        }
    }
    let body = bodies::bodies(project);
    println!(
        "body check: {} bodies with leads, {} leads off their pads, {} parts without leads, {} findings",
        body.parts.len(),
        body.parts.iter().map(|p| p.off_pads).sum::<usize>(),
        body.without_leads.len(),
        bodies::checks(project).len()
    );
    // Nets compared as groups of pads (an unnamed net's generated name may
    // differ between two imports): the partition of the pads into nets.
    type PadKey = (String, String, [i64; 2]);
    let groups = |nets: &BTreeMap<PadKey, String>| {
        let mut out: BTreeMap<String, std::collections::BTreeSet<PadKey>> = BTreeMap::new();
        for (k, v) in nets.iter().filter(|(_, v)| !v.is_empty()) {
            out.entry(v.clone()).or_default().insert(k.clone());
        }
        out.into_values().collect::<std::collections::BTreeSet<_>>()
    };
    let (before, after) = (pad_nets(reference), pad_nets(project));
    let (a, b) = (groups(&before), groups(&after));
    let equal = a.intersection(&b).count();
    println!(
        "pad nets: {} nets before, {} now, {} equal (same pads), {} pads before, {} now",
        a.len(),
        b.len(),
        equal,
        before.len(),
        after.len()
    );
    println!(
        "source schematic nets: {}, equal in the import {}, split by the import {}, joined by the import {}",
        made.report.counts.source_nets,
        made.report.counts.source_nets_equal,
        made.report.count(Case::SchematicNetSplit),
        made.report.count(Case::SchematicNetJoined)
    );
    let mut resistors = 0;
    let mut valued = 0;
    for instance in project
        .instances
        .iter()
        .filter(|i| i.designator.starts_with('R'))
    {
        resistors += 1;
        let fields = instances::effective_fields(&project.library, instance)?;
        if fields
            .iter()
            .any(|f| f.label == "Resistance" && f.value.is_some())
        {
            valued += 1;
        }
    }
    println!("resistors: {resistors}, with a Resistance value: {valued}");
    println!(
        "values: {} set fields, {} with unread words, {} without a field",
        made.report.count(Case::ValueField),
        made.report.count(Case::ValueWordUnread),
        made.report.count(Case::ValueWithoutField)
    );
    Ok(())
}

/// A six-pin KiCad symbol for the SOT pattern: pins 1–3 down the left
/// side, 4–6 up the right (library frame, y up). Pin 2 is off the grid,
/// where a power port on the generated symbol's pin 1 stands (that symbol
/// is drawn at KiCad's place, its pin 1 at (−8.77, 1.23) from the origin).
const SIX_LIB: &str = r#"(symbol "Original:Six" (in_bom yes) (on_board yes)
  (symbol "Six_0_1" (rectangle (start -3.81 3.81) (end 3.81 -3.81)))
  (symbol "Six_1_1"
    (pin passive line (at -6.35 2.54 0) (length 2.54) (name "P1") (number "1"))
    (pin passive line (at -8.77 2.23 0) (length 2.54) (name "P2") (number "2"))
    (pin passive line (at -6.35 -2.54 0) (length 2.54) (name "P3") (number "3"))
    (pin passive line (at 6.35 -2.54 180) (length 2.54) (name "P4") (number "4"))
    (pin passive line (at 6.35 0 180) (length 2.54) (name "P5") (number "5"))
    (pin passive line (at 6.35 2.54 180) (length 2.54) (name "P6") (number "6"))))"#;

/// A power symbol: one hidden pin at its origin.
const POWER_LIB: &str = r#"(symbol "Original:Power" (power) (in_bom yes) (on_board yes)
  (symbol "Power_1_1" (pin power_in line (at 0 0 270) (length 0) hide (name "P") (number "1"))))"#;

/// A KiCad schematic of `items` with the six-pin symbol at each place
/// and a power symbol of each value at each point.
fn six_schematic(places: &[(&str, [f64; 2])], power: &[(&str, [f64; 2])], items: &str) -> String {
    let symbol = |lib: &str, reference: &str, value: &str, at: [f64; 2]| {
        format!(
            r#"(symbol (lib_id "{lib}") (at {} {} 0) (unit 1)
              (property "Reference" "{reference}" (at 0 0 0))
              (property "Value" "{value}" (at 0 0 0))
              (instances (project "nets" (path "/00000000-0000-4000-8000-000000000001" (reference "{reference}") (unit 1)))))"#,
            at[0], at[1]
        )
    };
    let symbols: String = places
        .iter()
        .map(|(reference, at)| symbol("Original:Six", reference, "Six", *at))
        .chain(
            power
                .iter()
                .enumerate()
                .map(|(k, (value, at))| symbol("Original:Power", &format!("#PWR{k}"), value, *at)),
        )
        .collect();
    format!(
        r#"(kicad_sch (version 20231120) (generator "sparksmith_test")
  (uuid "00000000-0000-4000-8000-000000000001") (paper "A4")
  (lib_symbols {SIX_LIB} {POWER_LIB}) {symbols} {items})"#
    )
}

fn global_label(name: &str, at: [f64; 2]) -> String {
    format!(
        r#"(global_label "{name}" (shape input) (at {} {} 180))"#,
        at[0], at[1]
    )
}

fn wires(points: &[[f64; 2]]) -> String {
    points
        .windows(2)
        .map(|w| {
            format!(
                "(wire (pts (xy {} {}) (xy {} {})))",
                w[0][0], w[0][1], w[1][0], w[1][1]
            )
        })
        .collect()
}

/// The imported schematic's nets equal the source schematic's for
/// generated parts, whose own symbols are drawn apart from KiCad's wiring
/// and joined back to it: power ports and labels standing on the source's
/// pins (a wire meeting one of them there too, KiCad's wires ending at the
/// next pins), wires whose rejoining paths cross, and the board's pad nets
/// kept.
#[test]
fn generated_parts_keep_the_source_schematics_nets() -> Fallible {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    // Board nets of each SOT: 1 A, 2 B, 3 C, 4 D, 5 A, 6 B.
    let footprints = format!(
        "{}{}",
        sot_footprint("U1", [20.0, 10.0], 0.0, false, ""),
        sot_footprint("U2", [40.0, 10.0], 0.0, false, "")
    );
    // U1 at (50, 50) and U2 at (80, 50) on the sheet (y down): pin 1 at
    // x − 6.35, y − 2.54; pin 2 at x − 8.77, y − 2.23; pin 3 at x − 6.35,
    // y + 2.54; pins 4–6 at x + 6.35, y + 2.54, y, y − 2.54.
    let items = [
        // U1: power ports on pins 1 (A) and 4 (D), KiCad's wires ending
        // at the other pins (pin 2's point under the port on the generated
        // symbol's pin 1).
        wires(&[[41.23, 47.77], [35.0, 47.77]]),
        global_label("B", [35.0, 47.77]),
        wires(&[[43.65, 52.54], [35.0, 52.54]]),
        global_label("C", [35.0, 52.54]),
        // U1.5 (A) to U2.1 (A), and U1.6 (B) to U2.2 (B), crossing it;
        // a power port on U1.5 and a label on U1.6 too.
        global_label("B", [56.35, 47.46]),
        wires(&[[56.35, 50.0], [65.0, 50.0], [65.0, 47.46], [73.65, 47.46]]),
        wires(&[
            [56.35, 47.46],
            [63.0, 47.46],
            [63.0, 51.0],
            [68.0, 51.0],
            [68.0, 47.77],
            [71.23, 47.77],
        ]),
        // U2's other pins: labels and power ports at the pins.
        global_label("C", [73.65, 52.54]),
        global_label("B", [86.35, 47.46]),
    ]
    .concat();
    std::fs::write(
        dir.path().join("nets.kicad_sch"),
        six_schematic(
            &[("U1", [50.0, 50.0]), ("U2", [80.0, 50.0])],
            &[
                ("A", [43.65, 47.46]),
                ("A", [56.35, 50.0]),
                ("D", [56.35, 52.54]),
                ("A", [86.35, 50.0]),
                ("D", [86.35, 52.54]),
            ],
            &items,
        ),
    )?;
    let imported = import(dir.path(), "nets", &board("nets", &footprints))?;
    let report = &imported.report;
    assert_eq!(report.count(Case::GeneratedPart), 2);
    // Four source nets (A, B, C, D), each equal; no import fault.
    assert_eq!(
        (report.counts.source_nets, report.counts.source_nets_equal),
        (4, 4),
        "{:?}",
        report.details()
    );
    assert_eq!(report.count(Case::SchematicNetSplit), 0);
    assert_eq!(report.count(Case::SchematicNetJoined), 0);
    // The pads' nets are the board's.
    let project = imported.engine.project();
    for (key, net) in pad_nets(project) {
        let want = match key.1.as_str() {
            "1" | "5" => "A",
            "2" | "6" => "B",
            "3" => "C",
            _ => "D",
        };
        assert!(net.eq_ignore_ascii_case(want), "{}.{}: {net}", key.0, key.1);
    }
    Ok(())
}

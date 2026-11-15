//! The Place palette's generated parts (slice D1): generator presets below
//! the components, generic parts a text reads as, and choosing one makes
//! the component with its generator (off the UI thread, as
//! `library.generate_part` does) and arms placement. Driven through the
//! engine; the scripted part of the VM run (place a resistor, a QFN and a
//! header from the palette) is `placing_a_resistor_a_qfn_and_a_header`.
use super::tests::{key, parts_engine, shown, workspace_frame};
use super::*;
use egui::vec2;
use sparksmith_core::Evaluators;
use sparksmith_ui::fonts;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    ctx
}

/// An empty project on the test fixture fab profile, with the host's
/// generator runtime: the parts it makes estimate their lands with the
/// board's margins.
fn empty_engine() -> Engine {
    let mut engine = Engine::profiled().unwrap_or_default();
    engine.set_generators(sparksmith_automation::session::generators::runtime());
    engine
}

/// An editor whose palette offers the shipped generators.
fn editor() -> SchematicEditor {
    let mut editor = SchematicEditor::default();
    editor
        .palette_mut()
        .set_shelf(&crate::generators::test_shelf());
    editor
}

/// The palette open with `text` typed; the texts it drew, top to bottom.
fn typed(
    editor: &mut SchematicEditor,
    engine: &Engine,
    ctx: &egui::Context,
    text: &str,
) -> Vec<(String, Pos2)> {
    editor.on_tool("place", engine);
    workspace_frame(editor, engine, ctx, vec![]);
    let (output, _) = workspace_frame(editor, engine, ctx, vec![egui::Event::Text(text.into())]);
    let mut rows = shown(&output);
    rows.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
    rows
}

fn apply_hosted(engine: &mut Engine, operation: Operation) -> TestResult {
    engine.apply_hosted(
        engine.revision(),
        operation,
        Evaluators {
            footprint: Some(&sparksmith_solidcraft::footprint::SketchEvaluator),
            board: None,
        },
    )?;
    Ok(())
}

/// Enter on the highlighted row, then frames until the part is made: the
/// one command the editor sends.
fn enter_and_make(
    editor: &mut SchematicEditor,
    engine: &Engine,
    ctx: &egui::Context,
) -> Result<Operation, Box<dyn std::error::Error>> {
    let (_, mut operations) = workspace_frame(
        editor,
        engine,
        ctx,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    for _ in 0..600 {
        if let Some(operation) = operations.pop() {
            assert!(operations.is_empty(), "one command: {operations:?}");
            return Ok(operation);
        }
        if editor.making.is_none() {
            return Err("nothing is being made".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        operations = workspace_frame(editor, engine, ctx, vec![]).1;
    }
    Err("the part was not made in 30 s".into())
}

/// A click on the sheet while placement is armed: the one placement sent,
/// applied, joined to the create as the app joins it.
fn place_at(
    editor: &mut SchematicEditor,
    engine: &mut Engine,
    ctx: &egui::Context,
    at: Pos2,
) -> TestResult {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut sent = Vec::new();
    for events in [
        vec![egui::Event::PointerMoved(at)],
        vec![button(true)],
        vec![button(false)],
    ] {
        sent.extend(workspace_frame(editor, engine, ctx, events).1);
    }
    let [operation @ Operation::Instance(InstanceCommand::Place(_))] = &sent[..] else {
        return Err(format!("one component.place, got {sent:?}").into());
    };
    apply_hosted(engine, operation.clone())?;
    if let Some(revision) = editor.take_template_join() {
        engine.join_last_step(revision)?;
    }
    Ok(())
}

/// The scripted part of the VM run: a resistor (a generic part the text
/// reads as), a QFN (a generator preset) and a header (a generic part of
/// counts) placed from the palette, each made by its generator with its 3D
/// model, created and placed as one undo step.
#[test]
fn placing_a_resistor_a_qfn_and_a_header() -> TestResult {
    let mut engine = empty_engine();
    let ctx = context();
    for (n, (text, name, generator)) in [
        ("resistor 0603", "Chip resistor 0603", "chip"),
        ("qfn-16 3x3", "QFN-16 3×3 0.5 EP 1.7", "no-lead"),
        ("header 2x5", "Pin header 2×5 2.54 mm", "header"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut editor = editor();
        let rows = typed(&mut editor, &engine, &ctx, text);
        assert!(
            rows.iter().any(|(t, _)| t == "not yet created"),
            "{text}: {rows:?}"
        );
        let operation = enter_and_make(&mut editor, &engine, &ctx)?;
        assert!(
            matches!(
                operation,
                Operation::Library(sparksmith_core::LibraryCommand::Generate(_))
            ),
            "{text}: {operation:?}"
        );
        apply_hosted(&mut engine, operation)?;
        workspace_frame(&mut editor, &engine, &ctx, vec![]);
        assert_eq!(editor.tool, Tool::Place, "{text}");
        let component = editor.component.ok_or("armed")?;
        let library = &engine.project().library;
        let published = library.component(component).ok_or("published")?;
        assert_eq!(published.meta.name, name, "{text}");
        let record = sparksmith_core::library::generated::record_of(
            library,
            sparksmith_core::ids::DefinitionRef {
                kind: sparksmith_core::ids::DefinitionKind::Component,
                id: component.id,
                revision: component.revision,
            },
        )
        .ok_or("made by a generator")?;
        assert_eq!(record.from.generator, generator);
        let x = 120.0 + 200.0 * n as f32;
        place_at(&mut editor, &mut engine, &ctx, pos2(x, 680.0))?;
        assert_eq!(engine.project().instances.len(), n + 1, "{text}");
    }
    // Each create went with its first placement: three steps undo all.
    for _ in 0..3 {
        engine.undo()?;
    }
    assert!(engine.project().instances.is_empty());
    assert!(engine.project().library.components.is_empty());
    assert!(!engine.can_undo());
    Ok(())
}

/// The same text again finds the part the project holds: listed once,
/// nothing made, placement armed at once; Esc keeps it.
#[test]
fn a_part_the_project_holds_is_what_the_text_finds() -> TestResult {
    let mut engine = empty_engine();
    let ctx = context();
    let mut editor = editor();
    typed(&mut editor, &engine, &ctx, "resistor 0603");
    let operation = enter_and_make(&mut editor, &engine, &ctx)?;
    apply_hosted(&mut engine, operation)?;
    workspace_frame(&mut editor, &engine, &ctx, vec![]);
    let component = editor.component.ok_or("armed")?;

    let mut editor = self::editor();
    let rows = typed(&mut editor, &engine, &ctx, "resistor 0603");
    assert!(
        rows.iter().any(|(t, _)| t.starts_with("IN THIS PROJECT")),
        "{rows:?}"
    );
    let listed = rows
        .iter()
        .filter(|(t, p)| t == "Chip resistor 0603" && p.x < 800.0);
    assert_eq!(listed.count(), 1, "once: {rows:?}");
    let (_, operations) = workspace_frame(
        &mut editor,
        &engine,
        &ctx,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert!(operations.is_empty(), "{operations:?}");
    assert!(editor.making.is_none(), "nothing to make");
    assert_eq!(editor.tool, Tool::Place);
    assert_eq!(editor.component, Some(component));
    workspace_frame(
        &mut editor,
        &engine,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert_eq!(editor.take_template_undo(), None);
    assert_eq!(engine.project().library.latest_components().len(), 1);
    Ok(())
}

/// Esc once armed undoes the create: the project is as it was.
#[test]
fn cancelling_a_generic_part_changes_nothing() -> TestResult {
    let mut engine = empty_engine();
    let ctx = context();
    let mut editor = editor();
    let before = engine.project().clone();
    typed(&mut editor, &engine, &ctx, "led 0603");
    let operation = enter_and_make(&mut editor, &engine, &ctx)?;
    apply_hosted(&mut engine, operation)?;
    workspace_frame(&mut editor, &engine, &ctx, vec![]);
    assert_eq!(editor.tool, Tool::Place);
    workspace_frame(
        &mut editor,
        &engine,
        &ctx,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert_eq!(editor.tool, Tool::Select);
    assert_eq!(editor.take_template_undo(), Some(engine.revision()));
    engine.undo()?;
    assert_eq!(*engine.project(), before);
    Ok(())
}

/// A colour the text named reaches each instance placed.
#[test]
fn the_named_colour_reaches_the_placed_instance() -> TestResult {
    let mut engine = empty_engine();
    let ctx = context();
    let mut editor = editor();
    let rows = typed(&mut editor, &engine, &ctx, "led 0603 red");
    assert!(rows.iter().any(|(t, _)| t == "Red"), "{rows:?}");
    let operation = enter_and_make(&mut editor, &engine, &ctx)?;
    apply_hosted(&mut engine, operation)?;
    workspace_frame(&mut editor, &engine, &ctx, vec![]);
    assert_eq!(editor.place_values.len(), 1, "{:?}", editor.place_values);
    Ok(())
}

/// A size no preset has lists why; Enter says so and makes nothing.
#[test]
fn a_refused_part_says_why_and_sends_nothing() -> TestResult {
    let engine = empty_engine();
    let ctx = context();
    let mut editor = editor();
    let rows = typed(&mut editor, &engine, &ctx, "resistor 0604");
    assert!(rows.iter().any(|(t, _)| t == "cannot be made"), "{rows:?}");
    let (_, operations) = workspace_frame(
        &mut editor,
        &engine,
        &ctx,
        vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
    );
    assert!(operations.is_empty());
    assert!(editor.making.is_none());
    assert!(editor.palette.is_open());
    assert_eq!(editor.component, None);
    Ok(())
}

/// The founder's bar for this list: components, generic parts and
/// presets in one neat sheet, a cut row showing its full text on hover.
#[test]
fn the_palette_rows_show_in_full_on_hover() -> TestResult {
    let engine = parts_engine()?;
    let screen = crate::kit::text::tests::Screen::new(vec2(1000.0, 700.0));
    let run = |editor: &mut SchematicEditor, events: Vec<egui::Event>| {
        screen.run(events, |ui| {
            editor.canvas(ui, &engine);
        })
    };
    for (text, picture) in [
        ("0603", "place-palette-generators"),
        ("100n 0603 16V", "place-palette-generic-values"),
        ("header 5x2", "place-palette-generic-header"),
        ("qfn", "place-palette-presets"),
    ] {
        let mut editor = editor();
        editor.on_tool("place", &engine);
        run(&mut editor, vec![]);
        run(&mut editor, vec![egui::Event::Text(text.into())]);
        let drawn = run(&mut editor, vec![]);
        screen.picture(picture, &drawn, 1.0)?;
        let (_, missing) = crate::kit::text::tests::cuts_without_hover(&screen.ctx, |events| {
            run(&mut editor, events)
        });
        assert!(
            missing.is_empty(),
            "{text}: cut without a hover: {missing:?}"
        );
    }
    Ok(())
}

/// VM check #4 (run 3 #10): on the placed USB-C receptacle no pin number's
/// box runs past its pin end, touches the end's mark, the body outline,
/// another number or a pin name, measured at the drawn fonts' width; stacked
/// pads read in full (run 4 #12: the short form was cut too small to read),
/// at the pin text height. A connector without a value shows none, never
/// its component name (run 4 #11).
#[test]
fn usb_c_pin_numbers_sit_clear_and_no_name_is_its_value() -> TestResult {
    use sparksmith_core::geometry::Bounds;
    use sparksmith_core::schematic::{self, PIN_RING_MM};
    let mut engine = empty_engine();
    // The shipped receptacle (library/parts/gct-usb4105), placed as the
    // palette places it.
    let bundle = sparksmith_automation::part_files::builtin_parts()?
        .require("gct-usb4105")?
        .component
        .clone()
        .ok_or("a component part")?;
    apply_hosted(
        &mut engine,
        Operation::Library(sparksmith_core::LibraryCommand::PlaceFrom {
            source: sparksmith_core::library::import::PlaceSource::Bundle {
                bundle: Box::new(sparksmith_core::library::Bundle::clone(&bundle)),
            },
            position: sparksmith_core::units::PointNm::ORIGIN,
            designator: None,
            other_revision: Default::default(),
        }),
    )?;
    let project = engine.project();
    let occurrence = project.occurrences.first().ok_or("placed")?.id;
    let unit = schematic::placed_unit(project, occurrence)?;
    let m = TextMetric::default();
    // Capitals and digits draw at up to 0.9 of their cap height.
    let wide = TextMetric { advance: 0.9 };
    let numbers: Vec<(&schematic::PlacedPin, Bounds)> = unit
        .pins
        .iter()
        .filter_map(|p| Some((p, p.number.as_ref()?.bounds(wide))))
        .collect();
    assert_eq!(numbers.len(), 9, "every pin numbered");
    let names: Vec<Bounds> = unit
        .pins
        .iter()
        .filter_map(|p| Some(p.name.as_ref()?.bounds(m)))
        .collect();
    for (i, (pin, b)) in numbers.iter().enumerate() {
        let text = pin.number.as_ref().map_or("", |t| t.text.as_str());
        let ring = Bounds::point(pin.ring_centre(PIN_RING_MM)).inflated(PIN_RING_MM);
        assert!(!b.intersects(&ring), "{text}: on the pin end's mark");
        // Along the pin, the box keeps clear of the body outline (its
        // stroke and a gap) and of the pin end's ring.
        let o = pin.outward();
        let along = |p: [f64; 2]| p[0] * o[0] + p[1] * o[1];
        let (lo, hi) = (along(pin.body_end), along(pin.end));
        for corner in [b.min, b.max, [b.min[0], b.max[1]], [b.max[0], b.min[1]]] {
            let a = along(corner);
            assert!(a >= lo + 0.25 - 1e-9, "{text}: on the body outline");
            assert!(a <= hi - PIN_RING_MM + 1e-9, "{text}: over the pin end");
        }
        for (other, c) in &numbers[i + 1..] {
            let them = other.number.as_ref().map_or("", |t| t.text.as_str());
            assert!(!b.intersects(c), "{text} runs into {them}");
        }
        assert!(
            !names.iter().any(|n| b.intersects(n)),
            "{text} runs into a name"
        );
    }
    let shown: Vec<&str> = numbers
        .iter()
        .filter_map(|(p, _)| p.number.as_ref().map(|t| t.text.as_str()))
        .collect();
    for full in ["A4B9,B4A9", "A1B12,B1A12"] {
        assert!(shown.contains(&full), "{full} in {shown:?}");
    }
    for (pin, _) in &numbers {
        let number = pin.number.as_ref().ok_or("a number")?;
        assert!(
            (number.height - sparksmith_core::library::graphics::PIN_TEXT_HEIGHT_MM).abs() < 1e-9,
            "{} at {} mm",
            number.text,
            number.height
        );
    }
    // The component name ("USB-C receptacle …, GCT USB4105 …") is no value;
    // the assigned GCT Part shows its part number, USB4105-GF-A.
    assert!(
        unit.texts
            .iter()
            .all(|t| !t.text.contains("receptacle") && !t.text.contains("GCT")),
        "{:?}",
        unit.texts
    );
    Ok(())
}

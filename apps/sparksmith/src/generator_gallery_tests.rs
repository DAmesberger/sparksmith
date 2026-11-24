//! The generator gallery (slice D1): rows from `library.generators`, one
//! form for every generator, the regenerate rule (G15) visible where it
//! applies, and a working set's package taken from a generator in one
//! command.
use super::*;
use crate::generators::test_shelf;
use egui::{Pos2, Rect};
use sparksmith_core::library::generated::GeneratorScope;
use sparksmith_core::{Evaluators, LibraryCommand};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
    ctx
}

fn engine() -> Engine {
    let mut engine = Engine::profiled().unwrap_or_default();
    engine.set_generators(sparksmith_automation::session::generators::fixture_runtime());
    engine
}

/// One frame of the sheet: its texts and the command it sent.
fn frame(
    ctx: &egui::Context,
    gallery: &mut GeneratorGallery,
    engine: &Engine,
    events: Vec<egui::Event>,
) -> (Vec<String>, Option<Operation>) {
    frame_of(ctx, gallery, engine, events, 900.0)
}

/// [`frame`] on a screen `height` points tall.
fn frame_of(
    ctx: &egui::Context,
    gallery: &mut GeneratorGallery,
    engine: &Engine,
    events: Vec<egui::Event>,
    height: f32,
) -> (Vec<String>, Option<Operation>) {
    let mut sent = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, height))),
            events,
            ..Default::default()
        },
        |ui| sent = gallery.show(ui.ctx(), engine),
    );
    output.textures_delta.clear();
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in &output.shapes {
        walk(&shape.shape, &mut texts);
    }
    (texts, sent)
}

/// One frame `width` x `height` points: each text with whether it was cut
/// short, and whether it painted a refusal's error fill.
fn painted(
    ctx: &egui::Context,
    gallery: &mut GeneratorGallery,
    engine: &Engine,
    width: f32,
    height: f32,
) -> (Vec<(String, bool)>, bool) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, height))),
            ..Default::default()
        },
        |ui| {
            gallery.show(ui.ctx(), engine);
        },
    );
    output.textures_delta.clear();
    fn walk(shape: &egui::Shape, texts: &mut Vec<(String, bool)>, fill: &mut bool) {
        match shape {
            egui::Shape::Text(text) => {
                texts.push((text.galley.job.text.clone(), text.galley.elided));
            }
            egui::Shape::Rect(rect) => *fill |= rect.fill == Tokens::get().form_error_fill,
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, texts, fill)),
            _ => {}
        }
    }
    let (mut texts, mut fill) = (Vec::new(), false);
    for shape in &output.shapes {
        walk(&shape.shape, &mut texts, &mut fill);
    }
    (texts, fill)
}

fn enter() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Frames until the part is made: the command the sheet sends.
fn made(
    ctx: &egui::Context,
    gallery: &mut GeneratorGallery,
    engine: &Engine,
) -> Result<Operation, Box<dyn std::error::Error>> {
    for _ in 0..600 {
        if let (_, Some(operation)) = frame(ctx, gallery, engine, vec![]) {
            return Ok(operation);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("nothing was made in 30 s".into())
}

fn apply(engine: &mut Engine, operation: Operation) -> TestResult {
    engine.apply_hosted(engine.revision(), operation, Evaluators::default())?;
    Ok(())
}

#[test]
fn every_generator_lists_its_values_and_its_presets() -> TestResult {
    let offers = test_shelf().offers().ok_or("the shipped generators")?;
    let ids: Vec<&str> = offers.iter().map(|o| o.description.id.as_str()).collect();
    for id in [
        "chip",
        "grid",
        "gull-wing",
        "header",
        "module",
        "molded",
        "no-lead",
        "tabbed",
        "tht",
    ] {
        assert!(ids.contains(&id), "{id} ships: {ids:?}");
    }
    let all = rows(&offers, None, "");
    let custom = all.iter().filter(|r| r.preset.is_none()).count();
    assert_eq!(custom, offers.len(), "a custom row per generator");
    let presets: usize = offers.iter().map(|o| o.summary.presets.len()).sum();
    assert_eq!(all.len(), (custom + presets).min(ROWS));
    let qfn = rows(&offers, None, "qfn-16");
    assert!(
        !qfn.is_empty() && qfn.iter().all(|r| r.generator == "no-lead"),
        "{qfn:?}"
    );
    // VM run 8: the module generator's presets are offered, not only its
    // custom values.
    for text in ["wroom", "module"] {
        let found = rows(&offers, None, text);
        assert!(
            found
                .iter()
                .any(|r| r.preset.is_some() && r.name.contains("WROOM-3")),
            "{text}: {found:?}"
        );
    }
    let chip = rows(&offers, Some("chip"), "0603");
    assert!(
        chip.iter().all(|r| r.generator == "chip") && chip.len() >= 4,
        "{chip:?}"
    );
    Ok(())
}

/// GUI check 1, F8: a size code reads as imperial. The parts of that size
/// come first and the first is selected; a part that has the code only as
/// its metric name is still listed, after them.
#[test]
fn a_size_code_ranks_the_imperial_size_first() -> TestResult {
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let offers = test_shelf().offers().ok_or("the shipped generators")?;
    for (code, metric_name_of) in [("0603", "0201"), ("0402", "01005")] {
        for text in [
            code.to_owned(),
            format!("{code} resistor"),
            format!("chip {code}"),
        ] {
            let found = rows(&offers, None, &text);
            let at = |key: &str| {
                found
                    .iter()
                    .position(|r| r.preset.as_deref() == Some(key))
                    .ok_or(format!("{text}: no {key} in {found:?}"))
            };
            let imperial = at(&format!("resistor-{code}"))?;
            let metric = at(&format!("resistor-{metric_name_of}"))?;
            assert!(imperial < metric, "{text}: {found:?}");
            assert!(!found[0].metric_only, "{text}: {found:?}");
            let first_metric = found.iter().position(|r| r.metric_only);
            let last_imperial = found.iter().rposition(|r| !r.metric_only);
            assert!(first_metric > last_imperial, "{text}: {found:?}");
        }
    }
    // Codes that are no part's metric name, and a text that says metric.
    for (text, key) in [
        ("0805", "resistor-0805"),
        ("1206", "resistor-1206"),
        ("01005", "resistor-01005"),
    ] {
        let found = rows(&offers, None, text);
        assert!(
            found.iter().any(|r| r.preset.as_deref() == Some(key)),
            "{text}: {found:?}"
        );
        assert!(!found.iter().any(|r| r.metric_only), "{text}: {found:?}");
    }
    let metric = rows(&offers, None, "0603 metric");
    assert!(
        metric
            .iter()
            .any(|r| r.preset.as_deref() == Some("resistor-0201") && !r.metric_only),
        "{metric:?}"
    );
    Ok(())
}

/// Typed a digit at a time, the code ends on the imperial size, though
/// "06" put the 0201 first; a row a person chose stays.
#[test]
fn typing_a_size_code_selects_the_imperial_size() -> TestResult {
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let offers = test_shelf().offers().ok_or("the shipped generators")?;
    let mut gallery = GeneratorGallery::default();
    gallery.set_shelf(&test_shelf());
    gallery.open_new();
    let type_code = |gallery: &mut GeneratorGallery, code: &str| {
        for n in 1..=code.len() {
            if let Some(open) = &mut gallery.open {
                open.query = code[..n].to_owned();
            }
            gallery.refresh(&offers);
        }
        let row = gallery.open.as_ref().and_then(|o| o.row().cloned());
        row.map(|r| (r.name, r.metric_only))
    };
    for code in ["0603", "0402", "0805"] {
        let (name, metric) = type_code(&mut gallery, code).ok_or("a row")?;
        assert!(!metric && name.contains(code), "{code}: {name}");
        assert!(
            !name.contains(&format!("({code} metric)")),
            "{code}: {name}"
        );
    }
    // The 0201, chosen by hand, stays through the same search.
    if let Some(open) = &mut gallery.open {
        open.query = "0603".to_owned();
    }
    gallery.refresh(&offers);
    let open = gallery.open.as_mut().ok_or("open")?;
    open.highlighted = open
        .rows
        .iter()
        .position(|r| r.metric_only)
        .ok_or("a metric row")?;
    open.chosen = true;
    open.moved(&offers);
    let held = open.kept.clone();
    open.query = "0603 ".to_owned();
    gallery.refresh(&offers);
    assert_eq!(gallery.open.as_ref().and_then(|o| o.kept.clone()), held);
    Ok(())
}

/// The form is the description's: every parameter of every generator
/// has a row, by its label, and a preset fills its values.
#[test]
fn one_form_shows_every_generators_parameters() -> TestResult {
    let ctx = context();
    let engine = engine();
    let shelf = test_shelf();
    let offers = shelf.offers().ok_or("the shipped generators")?;
    for offer in offers.iter() {
        let mut gallery = GeneratorGallery::default();
        gallery.set_shelf(&shelf);
        let preset = offer.summary.presets.first().map(|p| p.key.clone());
        let request = MakeRequest {
            generator: offer.description.id.clone(),
            preset: preset.clone(),
            params: Default::default(),
            name: None,
            values: Vec::new(),
        };
        gallery.open_on(&request, String::new(), false);
        frame_of(&ctx, &mut gallery, &engine, vec![], 6000.0);
        let (texts, _) = frame_of(&ctx, &mut gallery, &engine, vec![], 6000.0);
        // On a tall enough sheet every parameter's label shows.
        let shown = offer
            .summary
            .params
            .iter()
            .filter(|p| texts.contains(&p.label))
            .count();
        assert!(
            shown == offer.summary.params.len(),
            "{}: {shown} of {} labels: {texts:?}",
            offer.description.id,
            offer.summary.params.len()
        );
        let values = gallery
            .open
            .as_ref()
            .and_then(|o| o.values.clone())
            .ok_or("values")?;
        assert_eq!(values.preset, preset, "{}", offer.description.id);
    }
    Ok(())
}

/// G15 in the form: the part a version made is held (nothing changes); a
/// newer version offers making its next revision; an older one is refused
/// with the reason, its primary disabled.
#[test]
fn the_regenerate_rule_shows_where_it_applies() -> TestResult {
    let ctx = context();
    let mut engine = engine();
    let shelf = test_shelf();
    let chip = shelf.offer("chip").ok_or("the chip generator")?;
    let request = MakeRequest {
        generator: "chip".into(),
        preset: Some("resistor-0603".into()),
        params: Default::default(),
        name: Some("Chip resistor 0603 (1608 metric)".into()),
        values: Vec::new(),
    };
    let operation = generators::command(&engine, &request, Into::Part(GeneratorScope::Component))?;
    apply(&mut engine, operation)?;

    let with_version = |version: u32| {
        let offers: Vec<Offer> = shelf
            .offers()
            .map(|o| o.to_vec())
            .unwrap_or_default()
            .into_iter()
            .map(|mut o| {
                if o.description.id == "chip" {
                    o.summary.version = version;
                    o.description = o.summary.description();
                }
                o
            })
            .collect();
        Shelf::with(offers)
    };
    let shown = |shelf: &Shelf| -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut gallery = GeneratorGallery::default();
        gallery.set_shelf(shelf);
        gallery.open_on(&request, String::new(), false);
        frame(&ctx, &mut gallery, &engine, vec![]);
        Ok(frame(&ctx, &mut gallery, &engine, vec![]).0)
    };
    let version = chip.summary.version;
    let same = shown(&shelf)?;
    assert!(same.iter().any(|t| t == "Open component"), "{same:?}");
    assert!(
        same.iter().any(|t| t.contains("nothing changes")),
        "{same:?}"
    );

    let newer = shown(&with_version(version + 1))?;
    let again = format!("Make again (version {})", version + 1);
    assert!(newer.contains(&again), "{newer:?}");
    assert!(
        newer.iter().any(|t| t.contains("makes its next revision")),
        "{newer:?}"
    );

    let older = shown(&with_version(version - 1))?;
    assert!(
        older.iter().any(|t| t.contains("never downgrades")),
        "{older:?}"
    );
    let mut gallery = GeneratorGallery::default();
    gallery.set_shelf(&with_version(version - 1));
    gallery.open_on(&request, String::new(), false);
    frame(&ctx, &mut gallery, &engine, vec![]);
    frame(&ctx, &mut gallery, &engine, vec![enter()]);
    assert!(
        gallery.open.as_ref().is_some_and(|o| o.making.is_none()),
        "refused: nothing is made"
    );
    Ok(())
}

/// A new component from a preset: Enter makes it, the sheet sends one
/// `library.generate`, and the made component is handed to the host.
#[test]
fn create_makes_the_part_and_hands_it_over() -> TestResult {
    let ctx = context();
    let mut engine = engine();
    let mut gallery = GeneratorGallery::default();
    gallery.set_shelf(&test_shelf());
    gallery.open_new();
    frame(&ctx, &mut gallery, &engine, vec![]);
    frame(
        &ctx,
        &mut gallery,
        &engine,
        vec![egui::Event::Text("sot-23-5".into())],
    );
    let (texts, _) = frame(&ctx, &mut gallery, &engine, vec![]);
    assert!(texts.iter().any(|t| t == "Create component"), "{texts:?}");
    frame(&ctx, &mut gallery, &engine, vec![enter()]);
    let operation = made(&ctx, &mut gallery, &engine)?;
    assert!(matches!(
        operation,
        Operation::Library(LibraryCommand::Generate(_))
    ));
    // E2E run 10, finding 5: while the part is made and applied the form
    // is busy, not refused: a neutral row, never the red refusal box.
    let (texts, error_fill) = painted(&ctx, &mut gallery, &engine, 1400.0, 900.0);
    assert!(
        texts.iter().any(|(t, _)| t == "Waiting for the engine…"),
        "{texts:?}"
    );
    assert!(!error_fill, "a busy form paints no refusal");
    apply(&mut engine, operation)?;
    frame(&ctx, &mut gallery, &engine, vec![]);
    let created = gallery.take_created().ok_or("handed over")?;
    let component = engine
        .project()
        .library
        .latest_components()
        .into_iter()
        .find(|c| c.id == created)
        .ok_or("published")?;
    assert!(
        component.meta.name.contains("SOT-23-5"),
        "{}",
        component.meta.name
    );
    assert!(!gallery.is_open());
    Ok(())
}

/// E2E run 9, finding 2: a value the module refuses (the ground array
/// turned 30°) is marked in the module's words while the form is open,
/// before Create, and Create makes nothing while it stands.
#[test]
fn a_value_the_module_refuses_is_marked_before_create() -> TestResult {
    let ctx = context();
    let engine = engine();
    let mut gallery = GeneratorGallery::default();
    gallery.set_shelf(&test_shelf());
    let mut params = sparksmith_core::library::generated::GeneratorParams::new();
    params.insert(
        "ground_rotation".into(),
        sparksmith_core::library::generated::ParamValue::Int(30_000),
    );
    let request = MakeRequest {
        generator: "module".into(),
        preset: Some("module-esp32-s31-wroom-3".into()),
        params,
        name: None,
        values: Vec::new(),
    };
    gallery.open_on(&request, String::new(), false);
    let message =
        "The ground array is turned 30°; the module's part files turn it by a multiple of 45°.";
    let mut marked = false;
    for _ in 0..600 {
        let (texts, _) = frame(&ctx, &mut gallery, &engine, vec![]);
        if texts.iter().any(|t| t == message) {
            marked = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(marked, "the module's refusal is marked before Create");
    // E2E run 10, finding 6: at 1024 x 560 the sentence is read whole,
    // under the field and in the banner, not cut.
    painted(&ctx, &mut gallery, &engine, 1024.0, 560.0);
    let (texts, _) = painted(&ctx, &mut gallery, &engine, 1024.0, 560.0);
    let shown: Vec<bool> = texts
        .iter()
        .filter(|(t, _)| t == message)
        .map(|(_, cut)| *cut)
        .collect();
    assert_eq!(shown, [false, false], "{texts:?}");
    frame(&ctx, &mut gallery, &engine, vec![enter()]);
    for _ in 0..5 {
        let (_, sent) = frame(&ctx, &mut gallery, &engine, vec![]);
        assert!(sent.is_none(), "Create made nothing");
    }
    assert!(gallery.open.as_ref().is_some_and(|o| o.making.is_none()));
    Ok(())
}

/// E2E run 11, finding 3: a list of lengths reads in mm with its unit,
/// like every single length (never the stored nanometres), and each
/// section is captioned with the module's whole group name.
#[test]
fn list_lengths_read_in_mm_under_whole_group_captions() -> TestResult {
    let ctx = context();
    let engine = engine();
    let shelf = test_shelf();
    let offer = shelf.offer("module").ok_or("the module generator")?;
    let mut gallery = GeneratorGallery::default();
    gallery.set_shelf(&shelf);
    let request = MakeRequest {
        generator: "module".into(),
        preset: None,
        params: Default::default(),
        name: None,
        values: Vec::new(),
    };
    gallery.open_on(&request, String::new(), false);
    frame_of(&ctx, &mut gallery, &engine, vec![], 6000.0);
    let (texts, _) = frame_of(&ctx, &mut gallery, &engine, vec![], 6000.0);
    for row in ["-8.4 mm, -5.715 mm, 8.4 mm", "5.5 mm, -11.9 mm, -11.01 mm"] {
        assert!(texts.iter().any(|t| t == row), "{row}: {texts:?}");
    }
    assert!(
        !texts.iter().any(|t| t.contains("8400000")),
        "nanometres shown: {texts:?}"
    );
    let mut groups: Vec<&str> = offer
        .summary
        .params
        .iter()
        .filter_map(|p| p.group.as_deref())
        .collect();
    groups.dedup();
    for group in groups {
        let caption = group.to_uppercase();
        assert!(texts.contains(&caption), "{caption}: {texts:?}");
    }
    Ok(())
}

#[test]
fn a_package_edit_during_generation_requires_a_fresh_replacement() -> TestResult {
    use sparksmith_core::library::{Definition, DraftEdit};
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _entered = context.enter();
    let ctx = self::context();
    let mut engine = Engine::demo()?;
    engine.set_generators(sparksmith_automation::session::generators::fixture_runtime());
    let footprint = engine
        .drafts()
        .items
        .iter()
        .find(|draft| matches!(draft.definition, Definition::Footprint(_)))
        .cloned()
        .ok_or("footprint draft")?;
    let Definition::Footprint(mut body) = footprint.definition else {
        return Err("expected a footprint".into());
    };
    let package = body.package.id;
    assert!(body.pads.len() >= 2);
    let mut gallery = GeneratorGallery::default();
    gallery.set_shelf(&test_shelf());
    gallery.open_package(&engine, package, footprint.id, "resistor 0603".into());
    frame(&ctx, &mut gallery, &engine, vec![]);
    gallery.submit(&ctx, &engine);
    assert!(gallery.open.as_ref().is_some_and(|o| o.making.is_some()));

    body.pads.pop();
    let remaining = body.pads.len();
    apply(
        &mut engine,
        Operation::Library(LibraryCommand::Edit {
            edits: vec![DraftEdit {
                id: footprint.id,
                meta: footprint.meta,
                definition: Definition::Footprint(body),
            }],
        }),
    )?;
    let edited = engine.document().clone();
    let revision = engine.revision();
    // Enter in the frame that first learns of the edit cannot approve it.
    let (_, stale) = frame(&ctx, &mut gallery, &engine, vec![enter()]);
    assert!(stale.is_none());
    assert!(gallery.open.as_ref().is_some_and(|o| o.making.is_none()));
    assert_eq!(
        gallery.scope(),
        Some(&Scope::Package {
            draft: package,
            pads: remaining
        })
    );
    assert_eq!(engine.document(), &edited);
    assert_eq!(engine.revision(), revision);
    // An explicit retry from the refreshed preview uses the edited draft.
    gallery.submit(&ctx, &engine);
    let replacement = made(&ctx, &mut gallery, &engine)?;
    assert!(
        matches!(&replacement, Operation::Library(LibraryCommand::PackageFromGenerator(p))
        if p.draft == package && p.replace)
    );
    apply(&mut engine, replacement)?;
    engine.undo()?;
    assert_eq!(engine.document(), &edited);
    Ok(())
}

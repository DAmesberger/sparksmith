//! Slice A1: the workspace opens one subject explicitly, names it in the
//! identity bar, walks its history, and shows library-folder items
//! read-only until Edit a copy.
use super::folder_preview;
use super::identity::Subject;
use super::*;
use sparksmith_core::ids::LibraryId;
use sparksmith_ui::fonts;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Texts one 900 × 600 canvas frame drew, the identity bar first.
fn frame(editor: &mut PartEditor, engine: &Engine, ctx: &egui::Context) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0))),
            ..Default::default()
        },
        |ui| {
            editor.identity_strip(ui, engine);
            editor.canvas(ui, engine);
        },
    );
    output.textures_delta.clear();
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &output.shapes {
        texts(&shape.shape, &mut out);
    }
    out
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    ctx
}

/// A New request from the editor, carried out as the app does.
fn create(
    editor: &mut PartEditor,
    engine: &mut Engine,
    ctx: &egui::Context,
    template: Template,
) -> TestResult {
    editor.create_new(template);
    let request = editor.take_request().ok_or("a request")?;
    run(engine, request)?;
    editor.commit(ctx, engine);
    Ok(())
}

/// A project with two component drafts opens the start page, not either.
#[test]
fn a_project_with_drafts_opens_the_start_page() -> TestResult {
    let ctx = context();
    let mut engine = Engine::default();
    let mut author = PartEditor::default();
    create(&mut author, &mut engine, &ctx, Template::TwoTerminal)?;
    create(&mut author, &mut engine, &ctx, Template::Header)?;
    let components = engine
        .drafts()
        .items
        .iter()
        .filter(|d| d.definition.kind() == sparksmith_core::DefinitionKind::Component)
        .count();
    assert_eq!(components, 2);
    let mut editor = PartEditor::default();
    editor.commit(&ctx, &engine);
    assert!(editor.draft.is_none() && editor.lineage().is_none());
    let shown = frame(&mut editor, &engine, &ctx);
    assert!(shown.iter().any(|t| t == "Open a component"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "No component open"), "{shown:?}");
    // Both drafts are listed, not shown.
    assert!(shown.iter().any(|t| t == "Resistor"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Connector"), "{shown:?}");
    Ok(())
}

/// New opens the component it created; the bar names it and its state,
/// and publishing changes the state chip.
#[test]
fn the_bar_names_the_subject_and_its_state() -> TestResult {
    let ctx = context();
    let mut engine = Engine::default();
    let mut editor = PartEditor::default();
    editor.commit(&ctx, &engine);
    create(&mut editor, &mut engine, &ctx, Template::TwoTerminal)?;
    let lineage = editor.lineage().ok_or("the new component is open")?;
    assert_eq!(
        editor.draft.as_ref().map(|s| s.ids.component),
        Some(lineage)
    );
    assert_eq!(
        editor.identity_text(&engine),
        Some(("Project › Resistor".into(), "new · never published".into()))
    );
    let shown = frame(&mut editor, &engine, &ctx);
    assert!(shown.iter().any(|t| t == "Project › Resistor"), "{shown:?}");
    assert!(
        shown.iter().any(|t| t == "new · never published"),
        "{shown:?}"
    );
    let publish = editor.on_tool("publish", &engine).ok_or("publish")?;
    engine.apply(engine.revision(), publish)?;
    editor.commit(&ctx, &engine);
    assert_eq!(
        editor.identity_text(&engine).map(|(_, state)| state),
        Some("published r1".into())
    );
    if let Some(set) = &mut editor.draft {
        set.metas.component.name = "Resistor 0603".into();
    }
    let edit = editor.commit(&ctx, &engine).ok_or("the rename commits")?;
    engine.apply(engine.revision(), edit)?;
    editor.commit(&ctx, &engine);
    assert_eq!(
        editor.identity_text(&engine),
        Some(("Project › Resistor 0603".into(), "r1 → draft".into()))
    );
    Ok(())
}

/// The Details card's Revision row agrees with the bar: right after
/// Publish it reads published, and an edit makes it a draft.
#[test]
fn the_revision_row_agrees_with_the_bar_after_publish() -> TestResult {
    let ctx = context();
    let mut engine = Engine::default();
    let mut editor = PartEditor::default();
    editor.commit(&ctx, &engine);
    create(&mut editor, &mut engine, &ctx, Template::TwoTerminal)?;
    let publish = editor.on_tool("publish", &engine).ok_or("publish")?;
    engine.apply(engine.revision(), publish)?;
    editor.commit(&ctx, &engine);
    let revision_row = |editor: &mut PartEditor, engine: &Engine| {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let unpublished = editor.unpublished(engine);
            editor.definition_section(ui, engine, unpublished);
        });
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text.starts_with("r1") => {
                    Some(t.galley.job.text.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        editor.identity_text(&engine).map(|(_, state)| state),
        Some("published r1".into())
    );
    assert_eq!(revision_row(&mut editor, &engine), ["r1 · published"]);
    if let Some(set) = &mut editor.draft {
        set.metas.component.name = "Resistor 0603".into();
    }
    let edit = editor.commit(&ctx, &engine).ok_or("the rename commits")?;
    engine.apply(engine.revision(), edit)?;
    editor.commit(&ctx, &engine);
    assert_eq!(
        revision_row(&mut editor, &engine),
        ["r1 · unpublished edits"]
    );
    Ok(())
}

/// The ribbon's Component group is New, Open and Publish.
#[test]
fn the_component_group_is_new_open_and_publish() -> TestResult {
    let ctx = context();
    let mut engine = Engine::default();
    let mut editor = PartEditor::default();
    create(&mut editor, &mut engine, &ctx, Template::TwoTerminal)?;
    for tab in ["details", "symbol", "footprint", "mapping", "model"] {
        editor.on_tab(tab);
        let ribbon = editor.ribbon(&engine);
        let group = ribbon
            .groups
            .iter()
            .find(|g| g.label == "Component")
            .ok_or("a Component group")?;
        let labels: Vec<_> = group.tools.iter().map(|t| t.label).collect();
        assert_eq!(labels, ["New", "Open", "Publish"], "{tab}");
    }
    // New opens the template gallery, Open the start page; nothing is
    // edited.
    let before = engine.revision();
    assert!(editor.on_tool("component-new", &engine).is_none());
    assert_eq!(
        editor.gallery.scope(),
        Some(&crate::generator_gallery::Scope::NewComponent)
    );
    assert!(editor.on_tool("component-open", &engine).is_none());
    assert!(editor.lineage().is_none() && editor.draft.is_none());
    assert_eq!(engine.revision(), before);
    Ok(())
}

/// Back and forward walk the opened subjects; the start page returns to
/// the one left.
#[test]
fn back_and_forward_walk_the_open_history() -> TestResult {
    let ctx = context();
    let mut engine = Engine::default();
    let mut editor = PartEditor::default();
    create(&mut editor, &mut engine, &ctx, Template::TwoTerminal)?;
    let first = editor.lineage().ok_or("first")?;
    editor.show_start();
    create(&mut editor, &mut engine, &ctx, Template::Header)?;
    let second = editor.lineage().ok_or("second")?;
    assert_ne!(first, second);
    editor.go_back(&engine);
    editor.commit(&ctx, &engine);
    assert_eq!(editor.lineage(), Some(first));
    assert_eq!(editor.draft.as_ref().map(|s| s.ids.component), Some(first));
    editor.go_forward(&engine);
    editor.commit(&ctx, &engine);
    assert_eq!(editor.lineage(), Some(second));
    editor.show_start();
    assert_eq!(editor.lineage(), None);
    editor.go_back(&engine);
    assert_eq!(editor.lineage(), Some(second), "back from the start page");
    // Recents list the newest first.
    assert_eq!(
        editor.identity.recents()[..2],
        [Subject::Project(second), Subject::Project(first)]
    );
    Ok(())
}

use folder_preview::sample_item as folder_item;

/// A folder item opens read-only: the bar says so, the project stays
/// unchanged and nothing the views do reaches it. Edit a copy imports and
/// drafts it, and the copy's origin chip names the folder.
#[test]
fn a_folder_item_opens_read_only_and_edit_a_copy_drafts_it() -> TestResult {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let ctx = context();
    let (pick, bundle) = folder_item()?;
    let mut project = Engine::default();
    let kept = (project.project().clone(), project.drafts().clone());
    let mut editor = PartEditor::default();
    editor.set_sources(
        vec![(LibraryId(7), "Sample Parts".into(), false)],
        1,
        Default::default,
    );
    editor.open_folder(pick.clone());
    assert_eq!(
        editor.take_folder_actions(),
        vec![identity::FolderAction::Preview(pick.clone())]
    );
    let preview = folder_preview::build(pick.clone(), bundle.clone())?;
    editor.show_preview(Box::new(preview));
    assert!(editor.commit(&ctx, &project).is_none());
    assert!(editor.is_read_only());
    assert_eq!(editor.lineage(), None, "no project lineage is open");
    let name = editor
        .draft
        .as_ref()
        .map(|s| s.metas.component.name.clone());
    assert_eq!(name.as_deref(), Some(pick.name.as_str()));
    let shown = frame(&mut editor, &project, &ctx);
    let state = format!("read-only · Sample Parts {}", pick.revision);
    assert!(shown.contains(&state), "{shown:?}");
    assert!(
        shown.iter().any(|t| t == "Edit a copy in this project"),
        "{shown:?}"
    );
    // Edits are refused or dropped; the project never changes.
    assert!(editor.on_tool("terminal", &project).is_none());
    if let Some(set) = &mut editor.draft {
        set.metas.component.name = "Changed".into();
    }
    assert!(editor.commit(&ctx, &project).is_none());
    assert_eq!(
        editor
            .draft
            .as_ref()
            .map(|s| s.metas.component.name.clone()),
        name
    );
    let labels: Vec<_> = editor
        .ribbon(&project)
        .groups
        .iter()
        .flat_map(|g| g.tools.iter().map(|t| (t.label, t.enabled)))
        .collect();
    assert_eq!(
        labels,
        [
            ("New", true),
            ("Open", true),
            ("Edit a copy", true),
            ("Publish", false)
        ]
    );
    assert_eq!((project.project().clone(), project.drafts().clone()), kept);
    // Edit a copy: the app imports and drafts the bundle, then opens it.
    assert!(editor.on_tool("component-edit-copy", &project).is_none());
    assert_eq!(
        editor.take_folder_actions(),
        vec![identity::FolderAction::EditCopy]
    );
    let copied = edit_copy(&mut project, editor.preview_bundle().ok_or("bundle")?)?;
    assert!(copied.changed, "an import and its drafts");
    editor.open(copied.lineage, &project);
    editor.commit(&ctx, &project);
    assert!(!editor.is_read_only());
    assert_eq!(editor.lineage(), Some(copied.lineage));
    assert_eq!(
        editor.identity_text(&project),
        Some((
            format!("Project › {}", pick.name),
            format!("published {}", sparksmith_core::Revision::FIRST)
        ))
    );
    assert_eq!(
        editor.origin_text(&project),
        Some(format!("from Sample Parts {}", pick.revision))
    );
    // One undo (one `library.edit_copy`) restores the project exactly.
    project.undo()?;
    assert_eq!((project.project().clone(), project.drafts().clone()), kept);
    Ok(())
}

/// A preview lent for the 3D view comes back only under its own subject:
/// after a subject change it is dropped, never shown for another item.
#[test]
fn a_lent_preview_is_restored_only_under_its_own_subject() -> TestResult {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let (pick, bundle) = folder_item()?;
    let mut editor = PartEditor::default();
    editor.set_sources(
        vec![(LibraryId(7), "Sample Parts".into(), false)],
        1,
        Default::default,
    );
    editor.open_folder(pick.clone());
    editor.show_preview(Box::new(folder_preview::build(
        pick.clone(),
        bundle.clone(),
    )?));
    let lent = editor.lend_preview();
    assert!(lent.is_some());
    editor.restore_preview(lent);
    assert!(editor.preview_bundle().is_some(), "same subject: restored");
    let lent = editor.lend_preview();
    editor.show_start();
    let mut other = pick.clone();
    other.revision = sparksmith_core::Revision(other.revision.0 + 1);
    editor.open_folder(other);
    editor.restore_preview(lent);
    assert!(
        editor.preview_bundle().is_none(),
        "another subject: dropped"
    );
    Ok(())
}

/// N3: a state chip reads at the WCAG AA ratio on its fill, and never takes
/// the saturated selection fill it looked selected on.
#[test]
fn state_chips_are_legible_and_do_not_look_selected() {
    for dark in [false, true] {
        sparksmith_ui::theme::set_dark(dark);
        let t = sparksmith_ui::theme::Tokens::get();
        for strong in [false, true] {
            let (fill, ink) = super::identity::chip_colours(&t, strong);
            assert_ne!(fill, t.chip, "the selection chip's fill");
            let ratio = crate::kit::contrast(ink, fill);
            assert!(ratio >= 4.5, "dark {dark}, strong {strong}: {ratio}");
        }
    }
}

/// The Open page lists template rows below the components; choosing one
/// opens the New from template form on it, whose Esc changes nothing and
/// whose primary makes the drafts and opens them.
#[test]
fn the_open_page_finds_presets_and_opens_the_form() -> TestResult {
    let ctx = context();
    let mut engine = Engine::default();
    let mut editor = PartEditor::default();
    editor.set_shelf(&crate::generators::test_shelf());
    create(&mut editor, &mut engine, &ctx, Template::TwoTerminal)?;
    let lineage = editor.lineage().ok_or("a component")?;
    editor.show_start();
    assert_eq!(
        engine.drafts().get(lineage).ok_or("draft")?.meta.name,
        "Resistor"
    );

    let run = |editor: &mut PartEditor, engine: &Engine, events: Vec<egui::Event>| {
        let mut sent = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1400.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| sent = editor.canvas(ui, engine),
        );
        output.textures_delta.clear();
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
            match shape {
                egui::Shape::Text(text) => out.push((
                    text.galley.job.text.clone(),
                    text.pos + text.galley.size() * 0.5,
                )),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for shape in &output.shapes {
            walk(&shape.shape, &mut texts);
        }
        (texts, sent)
    };
    let typed = |text: &str| vec![egui::Event::Text(text.to_owned())];

    run(&mut editor, &engine, vec![]);
    run(&mut editor, &engine, vec![]);
    let (texts, _) = run(&mut editor, &engine, typed("led"));
    let chip = |texts: &[(String, Pos2)]| {
        texts
            .iter()
            .find(|(t, _)| t.starts_with("Chip LED 0603"))
            .map(|(_, at)| *at)
    };
    assert!(chip(&texts).is_some(), "led finds the row: {texts:?}");
    assert!(texts.iter().any(|(t, _)| t.starts_with("FROM GENERATOR")));
    // Nothing else is called "led" here, so the group stands alone.
    assert!(!texts.iter().any(|(t, _)| t.starts_with("COMPONENTS")));

    // "resistor" matches the component and chip rows: the component first.
    let erase = vec![egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }];
    run(&mut editor, &engine, erase);
    let (texts, _) = run(&mut editor, &engine, typed("resistor"));
    let y = |texts: &[(String, Pos2)], text: &str| {
        texts
            .iter()
            .find(|(t, _)| t.starts_with(text))
            .map(|(_, at)| at.y)
    };
    let group = y(&texts, "FROM GENERATOR").ok_or("the generator group")?;
    let components = y(&texts, "COMPONENTS").ok_or("the component group")?;
    let component = y(&texts, "Resistor").ok_or("the component row")?;
    assert!(components < component && component < group, "{texts:?}");

    // "led" again, and choosing its row.
    let erase = vec![egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }];
    run(&mut editor, &engine, erase);
    let (texts, _) = run(&mut editor, &engine, typed("led"));
    let row = texts
        .iter()
        .find(|(t, _)| t.starts_with("Chip LED 0603"))
        .map(|(_, at)| *at)
        .ok_or_else(|| format!("led finds the row: {texts:?}"))?;
    let click = |pressed| egui::Event::PointerButton {
        pos: row,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    run(&mut editor, &engine, vec![egui::Event::PointerMoved(row)]);
    run(&mut editor, &engine, vec![click(true)]);
    run(&mut editor, &engine, vec![click(false)]);
    run(&mut editor, &engine, vec![]);
    assert_eq!(
        editor.gallery.scope(),
        Some(&crate::generator_gallery::Scope::NewComponent)
    );
    let (texts, _) = run(&mut editor, &engine, vec![]);
    assert!(
        texts.iter().any(|(t, _)| t == "Create component"),
        "{texts:?}"
    );

    // Esc: nothing was made.
    let before = engine.revision();
    let esc = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    run(&mut editor, &engine, vec![esc]);
    assert!(!editor.gallery.is_open());
    assert_eq!(engine.revision(), before);
    Ok(())
}

/// The Open page's template rows, in one picture, show in full on hover.
#[test]
fn the_open_page_rows_show_in_full_on_hover() -> TestResult {
    let ctx = context();
    let mut engine = Engine::default();
    let mut editor = PartEditor::default();
    editor.set_shelf(&crate::generators::test_shelf());
    create(&mut editor, &mut engine, &ctx, Template::TwoTerminal)?;
    editor.show_start();
    let screen = crate::kit::text::tests::Screen::new(egui::vec2(1000.0, 800.0));
    let run = |editor: &mut PartEditor, events: Vec<egui::Event>| {
        screen.run(events, |ui| {
            editor.canvas(ui, &engine);
        })
    };
    run(&mut editor, vec![]);
    run(&mut editor, vec![]);
    run(&mut editor, vec![egui::Event::Text("resistor".into())]);
    let drawn = run(&mut editor, vec![]);
    screen.picture("open-page-templates", &drawn, 1.0)?;
    let (_, missing) =
        crate::kit::text::tests::cuts_without_hover(&screen.ctx, |events| run(&mut editor, events));
    assert!(missing.is_empty(), "cut without a hover: {missing:?}");
    Ok(())
}

/// Demo run 2, finding 8: the Open page lists each component once, though
/// the project holds both the component and a part that implements it
/// (both open the component).
#[test]
fn the_open_page_lists_each_component_once() -> TestResult {
    use sparksmith_core::DefinitionKind;
    use sparksmith_core::catalog::Source;
    let ctx = context();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith"
    );
    let mut engine = Engine::demo()?;
    engine.replace_document(sparksmith_io::read_project(std::path::Path::new(path))?)?;
    let catalog = sparksmith_core::catalog::entries(engine.project(), engine.drafts());
    let named = |kind: DefinitionKind| {
        catalog
            .iter()
            .any(|e| e.name == "APT1608SGC" && e.kind == kind && e.source == Source::Project)
    };
    // The example holds the component and its part under one name.
    assert!(named(DefinitionKind::Component), "the component");
    assert!(named(DefinitionKind::Part), "its part");
    let mut editor = PartEditor::default();
    editor.show_start();
    frame(&mut editor, &engine, &ctx);
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0))),
            events: vec![egui::Event::Text("APT1608".to_owned())],
            ..Default::default()
        },
        |ui| {
            editor.identity_strip(ui, &engine);
            editor.canvas(ui, &engine);
        },
    )
    .textures_delta
    .clear();
    let shown = frame(&mut editor, &engine, &ctx);
    let rows = shown.iter().filter(|t| *t == "APT1608SGC").count();
    assert_eq!(rows, 1, "{shown:?}");
    Ok(())
}

/// R7-6: a bounded result group on the Open page reads "8 of n" with
/// "Show all ›", as the cards' bounded lists do, not "Show all n"; Show
/// all lists them all.
#[test]
fn the_open_page_bounds_results_as_the_cards_do() -> TestResult {
    let ctx = context();
    // The ESP32 example holds 11 components, more than the page shows.
    let engine = Engine::from_file(sparksmith_io::read_project(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith"
    )))?)?;
    let mut editor = PartEditor::default();
    editor.show_start();
    let run = |editor: &mut PartEditor, events: Vec<egui::Event>| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1400.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                editor.canvas(ui, &engine);
            },
        );
        output.textures_delta.clear();
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
            match shape {
                egui::Shape::Text(text) => out.push((
                    text.galley.job.text.clone(),
                    text.pos + text.galley.size() * 0.5,
                )),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for shape in &output.shapes {
            walk(&shape.shape, &mut texts);
        }
        texts
    };
    run(&mut editor, vec![]);
    let texts = run(&mut editor, vec![]);
    assert!(
        !texts
            .iter()
            .any(|(t, _)| t.starts_with("Show all") && t != "Show all \u{203a}"),
        "{texts:?}"
    );
    let count = texts
        .iter()
        .find(|(t, _)| {
            t.split_once(" of ")
                .is_some_and(|(a, b)| a.parse::<usize>().is_ok() && b.parse::<usize>().is_ok())
        })
        .map(|(t, _)| t.clone())
        .ok_or_else(|| format!("no \"n of m\": {texts:?}"))?;
    let all = texts
        .iter()
        .find(|(t, _)| t == "Show all \u{203a}")
        .map(|(_, at)| *at)
        .ok_or_else(|| format!("no Show all: {texts:?}"))?;
    let click = |pressed| egui::Event::PointerButton {
        pos: all,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    run(&mut editor, vec![egui::Event::PointerMoved(all)]);
    run(&mut editor, vec![click(true)]);
    run(&mut editor, vec![click(false)]);
    run(&mut editor, vec![]);
    let texts = run(&mut editor, vec![]);
    assert!(!texts.iter().any(|(t, _)| *t == count), "{texts:?}");
    Ok(())
}

/// Change package… (slice D1): a generator's package becomes the working
/// set's own in one command and one undo step: its part file is the model,
/// its lands the footprint's pads, and the terminals keep their identities
/// by number, so the binding's mappings stand.
#[test]
fn change_package_takes_a_generators_package_in_one_step() -> TestResult {
    use crate::generators::{self, Into, MakeRequest};
    use sparksmith_core::library::{Definition, PackageModel};
    let ctx = context();
    let mut engine = Engine::profiled()?;
    engine.set_generators(sparksmith_automation::session::generators::fixture_runtime());
    let mut editor = PartEditor::default();
    editor.set_shelf(&generators::test_shelf());
    create(&mut editor, &mut engine, &ctx, Template::TwoTerminal)?;
    let set = editor.draft.as_ref().ok_or("a working set")?;
    let ids = set.ids;
    let terminals: Vec<_> = set.package.terminals.iter().map(|t| t.id).collect();
    let mappings = set.binding.mappings.clone();
    assert_eq!(terminals.len(), 2);
    let before = engine.project().clone();
    let request = MakeRequest {
        generator: "chip".into(),
        preset: Some("resistor-0603".into()),
        params: Default::default(),
        name: None,
        values: Vec::new(),
    };
    let operation = generators::command(
        &engine,
        &request,
        Into::WorkingPackage {
            draft: ids.package,
            replace: true,
        },
    )?;
    assert!(matches!(
        operation,
        Operation::Library(sparksmith_core::LibraryCommand::PackageFromGenerator(_))
    ));
    engine.apply_hosted(
        engine.revision(),
        operation,
        sparksmith_core::Evaluators::default(),
    )?;
    let draft = |id| engine.drafts().get(id).map(|d| d.definition.clone());
    let Some(Definition::Package(package)) = draft(ids.package) else {
        return Err("the package draft".into());
    };
    assert!(
        matches!(package.model, PackageModel::PartFile(_)),
        "{:?}",
        package.model
    );
    let kept: Vec<_> = package.terminals.iter().map(|t| t.id).collect();
    assert_eq!(kept, terminals, "terminals kept by number");
    let Some(Definition::Footprint(footprint)) = draft(ids.footprint) else {
        return Err("the footprint draft".into());
    };
    assert_eq!(footprint.pads.len(), 2);
    assert!(
        footprint
            .pads
            .iter()
            .all(|p| p.terminal.is_some_and(|t| kept.contains(&t)))
    );
    let Some(Definition::Binding(binding)) = draft(ids.binding) else {
        return Err("the binding draft".into());
    };
    assert_eq!(binding.mappings, mappings, "mappings stand");
    // The generated package stays in the library with its record.
    assert_eq!(engine.project().library.generated.len(), 1);
    engine.undo()?;
    assert_eq!(*engine.project(), before);
    Ok(())
}

/// E2E run 9 finding 1: Change package… on a drafted generated part (its
/// drafts pin the stored revisions they were drafted from). After each
/// replacement, also twice in a row and to a package with fewer
/// terminals, the package's terminals, the footprint's pads and the
/// binding agree: every pad lands a terminal of the package draft, every
/// terminal has a pad, and new terminals are reported unmapped, never
/// filled.
#[test]
fn change_package_on_a_drafted_part_keeps_package_footprint_and_mapping_agreeing() -> TestResult {
    use crate::generators::{self, Into, MakeRequest};
    use sparksmith_core::library::Definition;
    let mut engine = Engine::profiled()?;
    engine.set_generators(sparksmith_automation::session::generators::fixture_runtime());
    let request = |preset: &str| MakeRequest {
        generator: "no-lead".into(),
        preset: Some(preset.into()),
        params: Default::default(),
        name: None,
        values: Vec::new(),
    };
    let make = generators::command(
        &engine,
        &request("no-lead-qfn-16-3x3-0.5"),
        Into::Part(sparksmith_core::library::generated::GeneratorScope::Component),
    )?;
    let sparksmith_core::Outcome::Generated { report } = engine.apply_hosted(
        engine.revision(),
        make,
        sparksmith_core::Evaluators::default(),
    )?
    else {
        return Err("a generated part".into());
    };
    let component = ComponentRef::new(report.root.id, report.root.revision);
    engine.apply(
        engine.revision(),
        Operation::Library(sparksmith_core::LibraryCommand::DraftClosure { component }),
    )?;
    let package_draft = engine
        .drafts()
        .items
        .iter()
        .find(|d| matches!(d.definition, Definition::Package(_)))
        .map(|d| d.id)
        .ok_or("a package draft")?;
    let agree = |engine: &Engine, terminals: usize, unmapped: bool, removed: bool| -> TestResult {
        let drafts = engine.drafts();
        let mut package = None;
        let mut footprint = None;
        let mut binding = None;
        for d in &drafts.items {
            match &d.definition {
                Definition::Package(p) => package = Some((d, p)),
                Definition::Footprint(f) => footprint = Some(f),
                Definition::Binding(b) => binding = Some(b),
                _ => {}
            }
        }
        let (Some((draft, package)), Some(footprint), Some(binding)) =
            (package, footprint, binding)
        else {
            return Err("the working set's drafts".into());
        };
        assert_eq!(package.terminals.len(), terminals);
        let reports = sparksmith_core::completeness::drafts(engine.project(), drafts)?;
        let codes: Vec<&str> = reports
            .iter()
            .flat_map(|r| r.findings.iter().map(|f| f.code.as_str()))
            .collect();
        for wrong in [
            "footprint.pad_terminal_unknown",
            "footprint.terminal_without_pad",
        ] {
            assert!(!codes.contains(&wrong), "{wrong}: {codes:?}");
        }
        // New terminals are to-dos to map; mappings of removed terminals
        // are flagged, never dropped or moved by name.
        assert_eq!(
            codes.contains(&"binding.mapping_missing"),
            unmapped,
            "{codes:?}"
        );
        assert_eq!(
            codes.contains(&"binding.mapping_unknown"),
            removed,
            "{codes:?}"
        );
        // The drafts name the package draft's pending revision, freshly
        // drafted (`library.draft_closure` pins them) and after each change.
        let pinned = draft.pending();
        assert_eq!(
            Some(footprint.package.revision),
            pinned,
            "footprint lands the package"
        );
        assert_eq!(
            Some(binding.package.revision),
            pinned,
            "binding names the package"
        );
        for pad in &footprint.pads {
            let terminal = pad.terminal.ok_or("a pad without a terminal")?;
            assert!(package.terminal(terminal).is_some(), "{pad:?}");
        }
        Ok(())
    };
    agree(&engine, 17, false, false)?;
    for (preset, terminals, unmapped, removed) in [
        ("no-lead-qfn-20-3x3-0.4", 21, true, false),
        ("no-lead-qfn-20-3x3-0.4", 21, true, false),
        ("no-lead-qfn-12-3x3-0.5", 13, false, true),
    ] {
        let operation = generators::command(
            &engine,
            &request(preset),
            Into::WorkingPackage {
                draft: package_draft,
                replace: true,
            },
        )?;
        engine.apply_hosted(
            engine.revision(),
            operation,
            sparksmith_core::Evaluators::default(),
        )?;
        agree(&engine, terminals, unmapped, removed)?;
    }
    Ok(())
}

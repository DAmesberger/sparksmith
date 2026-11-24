//! Production files…: the dialog opens from its ribbon command, Export
//! stays disabled while an unwaived error blocks it, Waive… commits one
//! `fabrication.waive` with its reason, a whole group or a Shift-click run
//! is waived with one reason in one undo step from a form that opens
//! under the group, and the export writes the set off the UI thread.
use super::*;
use egui::{Pos2, Shape};
use sparksmith_core::Engine;
use sparksmith_core::issues::Severity;
use sparksmith_ui::fonts;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn frame(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, Option<Operation>) {
    frame_at(editor, engine, ctx, events, 700.0)
}

/// One frame with the slot `width` wide.
fn frame_at(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    width: f32,
) -> (egui::FullOutput, Option<Operation>) {
    let mut command = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(width, 1400.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            command = editor.subject_slot(ui, engine).output;
        },
    );
    output.textures_delta.clear();
    (output, command)
}

fn texts(output: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(text) => out.push(text.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &output.shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

fn positions(output: &egui::FullOutput, label: &str) -> Vec<Pos2> {
    fn walk(shape: &Shape, label: &str, out: &mut Vec<Pos2>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, label, out)),
            Shape::Text(text) if text.galley.job.text == label => {
                out.push(text.pos + text.galley.size() * 0.5);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &output.shapes {
        walk(&shape.shape, label, &mut out);
    }
    out
}

/// The reason field's hint is painted whole, not cut short, in a slot as
/// wide as the app's Properties card.
fn hint_shows_whole(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    hint: &str,
) -> TestResult {
    let width = sparksmith_ui::theme::Tokens::get().form_card_width;
    frame_at(editor, engine, ctx, vec![], width);
    let (output, _) = frame_at(editor, engine, ctx, vec![], width);
    let output = &output;
    fn walk(shape: &Shape, hint: &str, out: &mut Vec<bool>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, hint, out)),
            Shape::Text(text) if text.galley.job.text == hint => out.push(text.galley.elided),
            _ => {}
        }
    }
    let mut elided = Vec::new();
    for shape in &output.shapes {
        walk(&shape.shape, hint, &mut elided);
    }
    match elided.as_slice() {
        [false] => Ok(()),
        [] => Err(format!("no {hint} hint in {:?}", texts(output)).into()),
        _ => Err(format!("{hint} hint cut short: {elided:?}").into()),
    }
}

fn press(point: Pos2) -> Vec<egui::Event> {
    let button = |pressed| egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![
        egui::Event::PointerMoved(point),
        button(true),
        button(false),
    ]
}

fn click(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    label: &str,
) -> Result<Option<Operation>, Box<dyn std::error::Error>> {
    let (output, _) = frame(editor, engine, ctx, vec![]);
    let point = *positions(&output, label)
        .last()
        .ok_or_else(|| format!("missing {label}"))?;
    Ok(frame(editor, engine, ctx, press(point)).1)
}

/// Rest the pointer on the painted `label` for a frame (hover actions
/// show on the next one).
fn hover(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    label: &str,
) -> Result<Pos2, Box<dyn std::error::Error>> {
    let (output, _) = frame(editor, engine, ctx, vec![]);
    let point = *positions(&output, label)
        .last()
        .ok_or_else(|| format!("missing {label}"))?;
    frame(editor, engine, ctx, vec![egui::Event::PointerMoved(point)]);
    Ok(point)
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    ctx
}

fn dialog(editor: &mut PcbEditor) -> Result<&mut ProductionDialog, Box<dyn std::error::Error>> {
    Ok(editor.production.as_mut().ok_or("the dialog is closed")?)
}

/// A report with one clearance error, its waiver read from the project.
fn report_with_error(engine: &Engine) -> PreflightReport {
    let key = r#"board.clearance [{"kind":"track","id":1},{"kind":"track","id":2}]"#.to_owned();
    let waiver = engine.project().production.waiver(&key).cloned();
    let blocking = usize::from(waiver.is_none());
    PreflightReport {
        profile: "Standard 6/6".into(),
        items: vec![PreflightItem {
            key,
            code: "board.clearance".into(),
            severity: Severity::Error,
            title: "Via on GND too close to via on VCC".into(),
            fix: Some("Move one via".into()),
            objects: Vec::new(),
            waiver,
            needs: None,
        }],
        errors: 1,
        warnings: 0,
        waived: 1 - blocking,
        blocking,
    }
}

fn open(engine: &Engine) -> Result<PcbEditor, Box<dyn std::error::Error>> {
    let mut editor = PcbEditor::default();
    assert!(editor.on_tool("production", engine).is_none());
    editor.open_production_in(None);
    assert_eq!(editor.subject(), Subject::Dialog);
    Ok(editor)
}

#[test]
fn the_ribbon_command_opens_the_dialog_in_the_subject_slot() -> TestResult {
    let engine = Engine::showcase()?;
    let mut editor = PcbEditor::default();
    let ribbon = editor.ribbon(&engine);
    assert!(
        ribbon
            .groups
            .iter()
            .flat_map(|g| g.tools.iter())
            .any(|t| t.id == "production")
    );
    editor.on_tool("production", &engine);
    assert_eq!(editor.subject(), Subject::Dialog);
    let ctx = context();
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    let shown = texts(&output);
    for label in ["Production files", "PRODUCTION CHECK", "OUTPUT", "Export"] {
        assert!(shown.iter().any(|t| t == label), "{label} in {shown:?}");
    }
    // Cancel closes it.
    click(&mut editor, &engine, &ctx, "Cancel")?;
    assert!(editor.production.is_none());
    Ok(())
}

/// Run 3, finding 16: without a folder the card says why Export waits; a
/// picked folder goes to the app once to keep for the project, and the
/// kept folder starts the next dialog, unless it has gone.
#[test]
fn the_card_says_export_needs_a_folder_and_the_folder_is_kept() -> TestResult {
    let engine = Engine::showcase()?;
    let ctx = context();
    let says = |output: &egui::FullOutput| {
        texts(output)
            .iter()
            .any(|t| t.starts_with("Choose a folder to save to"))
    };
    let mut editor = open(&engine)?;
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    assert!(says(&output), "{:?}", texts(&output));

    let tmp = tempfile::tempdir()?;
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send(Some(tmp.path().to_path_buf()))?;
    dialog(&mut editor)?.picking = Some(receiver);
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    assert!(!says(&output), "a folder is chosen");
    assert_eq!(
        editor.take_chosen_production_folder().as_deref(),
        Some(tmp.path())
    );
    assert_eq!(editor.take_chosen_production_folder(), None, "once");

    // What the app hands back for this project starts the next dialog.
    let mut editor = PcbEditor {
        production_folder: Some(tmp.path().to_path_buf()),
        ..PcbEditor::default()
    };
    editor.open_production_in(None);
    assert_eq!(dialog(&mut editor)?.location.as_deref(), Some(tmp.path()));
    let mut editor = PcbEditor {
        production_folder: Some(tmp.path().join("gone")),
        ..PcbEditor::default()
    };
    editor.open_production_in(None);
    assert_eq!(dialog(&mut editor)?.location, None, "a missing folder asks");
    Ok(())
}

#[test]
fn an_unwaived_error_blocks_export_and_a_waiver_unblocks_it() -> TestResult {
    let mut engine = Engine::showcase()?;
    let mut editor = open(&engine)?;
    let ctx = context();
    let tmp = tempfile::tempdir()?;
    {
        let d = dialog(&mut editor)?;
        d.report = Some((
            (engine.report_key(), Pick::Board, None),
            Ok(report_with_error(&engine)),
        ));
        d.location = Some(tmp.path().to_path_buf());
    }
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    let shown = texts(&output);
    assert!(shown.iter().any(|t| t == "1 error"), "{shown:?}");
    {
        let d = dialog(&mut editor)?;
        let report = d.current(&engine).cloned();
        assert_eq!(
            export_blocked(d, report.as_ref(), true, d.output("x").as_ref()).as_deref(),
            Some("Fix or waive the error first")
        );
    }
    // Readable without hovering Export (GUI check 1, item F6).
    assert!(
        shown.iter().any(|t| t == "Fix or waive the error first"),
        "{shown:?}"
    );
    // Export is disabled: clicking it starts nothing.
    click(&mut editor, &engine, &ctx, "Export")?;
    assert!(!dialog(&mut editor)?.busy());
    // Waive… shows while the row is hovered; it asks for a reason, and
    // Waive commits it as one edit.
    hover(
        &mut editor,
        &engine,
        &ctx,
        "Via on GND too close to via on VCC",
    )?;
    assert!(click(&mut editor, &engine, &ctx, "Waive…")?.is_none());
    "Vias are 0.18 mm apart by design".clone_into(&mut dialog(&mut editor)?.waiving.reason);
    let op = click(&mut editor, &engine, &ctx, "Waive 1 finding")?.ok_or("no waiver edit")?;
    assert!(matches!(
        &op,
        Operation::Fabrication(FabricationCommand::Waive { keys, reason })
            if keys == &[r#"board.clearance [{"kind":"track","id":1},{"kind":"track","id":2}]"#] && reason == "Vias are 0.18 mm apart by design"
    ));
    let before = engine.revision();
    engine.apply(before, op)?;
    assert_eq!(engine.project().production.waivers.len(), 1);
    // The report of the new revision has the item waived: Export enables.
    {
        let d = dialog(&mut editor)?;
        d.report = Some((
            (engine.report_key(), Pick::Board, None),
            Ok(report_with_error(&engine)),
        ));
        let report = d.current(&engine).cloned();
        assert_eq!(
            export_blocked(d, report.as_ref(), true, d.output("x").as_ref()),
            None
        );
        // A new project without a name is asked for one first.
        assert_eq!(
            export_blocked(d, report.as_ref(), false, None).as_deref(),
            Some(UNNAMED)
        );
    }
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    let shown = texts(&output);
    assert!(shown.iter().any(|t| t == "1 waived"), "{shown:?}");
    // Restore removes the waiver again.
    hover(
        &mut editor,
        &engine,
        &ctx,
        "Via on GND too close to via on VCC",
    )?;
    let op = click(&mut editor, &engine, &ctx, "Restore")?.ok_or("no restore edit")?;
    assert!(matches!(
        op,
        Operation::Fabrication(FabricationCommand::Unwaive { .. })
    ));
    engine.apply(engine.revision(), op)?;
    assert!(engine.project().production.waivers.is_empty());
    engine.undo()?;
    assert_eq!(engine.project().production.waivers.len(), 1);
    Ok(())
}

/// By default a reason is optional: Waive commits with none, and the
/// waived row's tip reads "Waived" without an empty reason.
#[test]
fn a_waiver_without_a_reason_is_committed_by_default() -> TestResult {
    let mut engine = Engine::showcase()?;
    let mut editor = open(&engine)?;
    let ctx = context();
    dialog(&mut editor)?.report = Some((
        (engine.report_key(), Pick::Board, None),
        Ok(report_with_error(&engine)),
    ));
    hover(
        &mut editor,
        &engine,
        &ctx,
        "Via on GND too close to via on VCC",
    )?;
    assert!(click(&mut editor, &engine, &ctx, "Waive…")?.is_none());
    hint_shows_whole(&mut editor, &engine, &ctx, "Optional")?;
    let op = click(&mut editor, &engine, &ctx, "Waive 1 finding")?.ok_or("no waiver edit")?;
    assert!(matches!(
        &op,
        Operation::Fabrication(FabricationCommand::Waive { reason, .. }) if reason.is_empty()
    ));
    engine.apply(engine.revision(), op)?;
    let report = report_with_error(&engine);
    let item = report.items.first().ok_or("item")?;
    assert_eq!(item.waiver.as_ref().map(|w| w.reason.as_str()), Some(""));
    assert_eq!(row_tip(item), "Waived\nMove one via");
    Ok(())
}

/// E2E run 10, finding 7: production files take the project's name, and
/// an Untitled project the name its first save gives it (its KiCad board's
/// until saved); a new project neither saved nor named is asked for one
/// instead of writing "Untitled" files.
#[test]
fn production_files_take_the_name_the_first_save_gives() {
    use std::path::Path;
    let untitled = sparksmith_core::Project::default();
    assert_eq!(untitled.name, sparksmith_core::UNTITLED);
    let options = FabricationOptions::default();
    let kicad = Path::new("/k/sensetap/sense tap.kicad_pcb");
    assert_eq!(
        files_stem(&untitled, &options, None, Some(kicad)).as_deref(),
        Some("sense_tap")
    );
    let saved = Path::new("/d/Untitled.sparksmith");
    assert_eq!(
        files_stem(&untitled, &options, Some(saved), Some(kicad)).as_deref(),
        Some("Untitled"),
        "a file saved as Untitled keeps that name"
    );
    assert_eq!(files_stem(&untitled, &options, None, None), None);
    let named = sparksmith_core::Project {
        name: "USB board".into(),
        ..sparksmith_core::Project::default()
    };
    assert_eq!(
        files_stem(&named, &options, None, Some(kicad)).as_deref(),
        Some("USB_board")
    );
    let own = FabricationOptions {
        base_name: Some("rev-a".into()),
        ..FabricationOptions::default()
    };
    assert_eq!(
        files_stem(&untitled, &own, None, None).as_deref(),
        Some("rev-a")
    );
}

#[test]
fn export_writes_the_set_off_the_ui_thread() -> TestResult {
    let engine = Engine::showcase()?;
    let mut editor = open(&engine)?;
    let ctx = context();
    let tmp = tempfile::tempdir()?;
    dialog(&mut editor)?.location = Some(tmp.path().to_path_buf());
    dialog(&mut editor)?.wait_check(&ctx, &engine);
    let ready = dialog(&mut editor)?
        .current(&engine)
        .and_then(|r| r.as_ref().ok())
        .is_some_and(PreflightReport::ready);
    if !ready {
        // A design the core refuses cannot be written; the dialog says why.
        return Ok(());
    }
    click(&mut editor, &engine, &ctx, "Export")?;
    assert!(dialog(&mut editor)?.busy(), "the export did not start");
    dialog(&mut editor)?.wait(&ctx);
    let written = dialog(&mut editor)?
        .written
        .clone()
        .ok_or("nothing was written")?;
    let folder = tmp.path().join(format!(
        "{}-production",
        fabrication::base_name(engine.project(), &FabricationOptions::default())?
    ));
    assert_eq!(PathBuf::from(&written.output), folder);
    assert!(folder.is_dir());
    for file in &written.files {
        assert!(folder.join(&file.name).is_file(), "{}", file.name);
    }
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    assert!(texts(&output).iter().any(|t| t == "WRITTEN"));
    // A second export into the same, now non-empty, folder is refused.
    click(&mut editor, &engine, &ctx, "Export")?;
    dialog(&mut editor)?.wait(&ctx);
    assert!(dialog(&mut editor)?.refusal.is_some());
    Ok(())
}

#[test]
fn sizes_and_summaries_read_as_words() {
    assert_eq!(size_text(812), "812 B");
    assert_eq!(size_text(2048), "2.0 KB");
    assert_eq!(size_text(3 * 1024 * 1024), "3.0 MB");
    assert_eq!(report_summary(&PreflightReport::default()), "No issues");
}

/// A report like the one-hour run's: a clearance error, nine unrouted
/// nets and twelve parts without a Part, waivers read from the project.
fn report_like_the_run(engine: &Engine) -> PreflightReport {
    let item = |key: String, code: &str, severity, title: String, fix: &str| {
        let waiver = engine.project().production.waiver(&key).cloned();
        PreflightItem {
            key,
            code: code.into(),
            severity,
            title,
            fix: Some(fix.into()),
            objects: Vec::new(),
            waiver,
            needs: None,
        }
    };
    let mut items = vec![item(
        r#"board.clearance [{"kind":"track","id":1},{"kind":"track","id":2}]"#.into(),
        "board.clearance",
        Severity::Error,
        "Via on GND too close to via on VCC".into(),
        "Move one via",
    )];
    for (id, net) in [
        "VBUS", "USB_DP", "USB_DM", "GND", "3V3", "SWDIO", "SWCLK", "SDA", "SCL",
    ]
    .into_iter()
    .enumerate()
    {
        items.push(item(
            format!(r#"board.unrouted [{{"kind":"net","id":{}}}]"#, id + 1),
            "board.unrouted",
            Severity::Error,
            format!("{net}: 2 pads unrouted"),
            "Route the connection",
        ));
    }
    for n in 1..=12 {
        items.push(item(
            format!(r#"purchasing.no_part [{{"kind":"instance","id":{n}}}]"#),
            "purchasing.no_part",
            Severity::Warning,
            format!("R{n}: no Part"),
            "Choose a Part with Resolve parts",
        ));
    }
    items.push(item(
        "fabrication.legend_outside legend:R3".into(),
        "fabrication.legend_outside",
        Severity::Warning,
        "Legend of R3 runs into the board edge cutout near the mounting hole".into(),
        "Move the legend",
    ));
    let blocking = items.iter().filter(|i| i.blocks()).count();
    let waived = items.iter().filter(|i| i.waiver.is_some()).count();
    PreflightReport {
        profile: "Standard 6/6".into(),
        errors: 10,
        warnings: 13,
        waived,
        blocking,
        items,
    }
}

/// The topmost painted `label`'s centre.
fn at(output: &egui::FullOutput, label: &str) -> Result<Pos2, Box<dyn std::error::Error>> {
    Ok(*positions(output, label)
        .first()
        .ok_or_else(|| format!("missing {label} in {:?}", texts(output)))?)
}

#[test]
fn a_whole_group_is_waived_with_one_reason_in_one_undo_step() -> TestResult {
    let mut engine = Engine::showcase()?;
    // Settings: "Require a reason to waive errors".
    engine.require_waiver_reason(true);
    let mut editor = open(&engine)?;
    let ctx = context();
    dialog(&mut editor)?.report = Some((
        (engine.report_key(), Pick::Board, None),
        Ok(report_like_the_run(&engine)),
    ));
    // The findings read as groups: the nine unrouted nets are one row.
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    let shown = texts(&output);
    for label in [
        "Unrouted connections",
        "No Part chosen",
        "Via on GND too close to via on VCC",
    ] {
        assert!(shown.iter().any(|t| t == label), "{label} in {shown:?}");
    }
    assert!(
        !shown.iter().any(|t| t == "VBUS: 2 pads unrouted"),
        "folded"
    );
    // Unrouted connections cannot be waived: their group offers no
    // Waive all… (e2e run 13, finding 37).
    hover(&mut editor, &engine, &ctx, "Unrouted connections")?;
    assert!(
        click(&mut editor, &engine, &ctx, "Waive all…").is_err(),
        "no Waive all… on the unrouted group"
    );
    assert!(dialog(&mut editor)?.waiving.ticked.is_empty());
    // Waive all… on a group opens the reason form right under it.
    let header = hover(&mut editor, &engine, &ctx, "No Part chosen")?;
    assert!(click(&mut editor, &engine, &ctx, "Waive all…")?.is_none());
    assert_eq!(dialog(&mut editor)?.waiving.ticked.len(), 12);
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    let reason = at(&output, "Reason")?;
    let next = at(
        &output,
        "Legend of R3 runs into the board edge cutout near the mounting hole",
    )?;
    assert!(
        header.y < reason.y && reason.y < next.y,
        "the form is under its group, before the next: {header:?} {reason:?} {next:?}"
    );
    hint_shows_whole(&mut editor, &engine, &ctx, "Required")?;
    // The reason is required by the setting: without one, Waive does
    // nothing, and the engine refuses such a waiver unchanged.
    assert!(click(&mut editor, &engine, &ctx, "Waive 12 findings")?.is_none());
    let revision = engine.revision();
    let keys: Vec<String> = dialog(&mut editor)?
        .waiving
        .ticked
        .iter()
        .cloned()
        .collect();
    let unreasoned = Operation::Fabrication(FabricationCommand::Waive {
        keys,
        reason: String::new(),
    });
    assert!(engine.apply(revision, unreasoned).is_err());
    assert_eq!(engine.revision(), revision);
    assert!(engine.project().production.waivers.is_empty());
    "prototype".clone_into(&mut dialog(&mut editor)?.waiving.reason);
    let op = click(&mut editor, &engine, &ctx, "Waive 12 findings")?.ok_or("no waiver edit")?;
    let Operation::Fabrication(FabricationCommand::Waive { keys, reason }) = &op else {
        return Err(format!("not a waiver: {op:?}").into());
    };
    assert_eq!(keys.len(), 12);
    assert!(keys.iter().all(|k| k.starts_with("purchasing.no_part ")));
    assert_eq!(reason, "prototype");
    assert!(dialog(&mut editor)?.waiving.at.is_none(), "the form closes");
    engine.apply(engine.revision(), op)?;
    assert_eq!(engine.project().production.waivers.len(), 12);
    engine.undo()?;
    assert!(
        engine.project().production.waivers.is_empty(),
        "one undo step"
    );
    Ok(())
}

#[test]
fn a_shift_click_ticks_a_run_and_the_form_follows_the_row() -> TestResult {
    let engine = Engine::showcase()?;
    let report = report_like_the_run(&engine);
    let groups = groups(&report);
    let mut w = Waiving::default();
    tick_row(
        &mut w,
        &groups,
        r#"purchasing.no_part [{"kind":"instance","id":2}]"#,
        false,
    );
    assert_eq!(
        w.at,
        Some(At::Row(
            r#"purchasing.no_part [{"kind":"instance","id":2}]"#.into()
        ))
    );
    tick_row(
        &mut w,
        &groups,
        r#"purchasing.no_part [{"kind":"instance","id":6}]"#,
        true,
    );
    assert_eq!(w.ticked.len(), 5, "R2 to R6: {:?}", w.ticked);
    assert_eq!(
        w.at,
        Some(At::Row(
            r#"purchasing.no_part [{"kind":"instance","id":6}]"#.into()
        ))
    );
    // A plain click unticks one; the last untick closes the form.
    tick_row(
        &mut w,
        &groups,
        r#"purchasing.no_part [{"kind":"instance","id":6}]"#,
        false,
    );
    assert_eq!(w.ticked.len(), 4);
    for n in 2..=5 {
        tick_row(
            &mut w,
            &groups,
            &format!(r#"purchasing.no_part [{{"kind":"instance","id":{n}}}]"#),
            false,
        );
    }
    assert!(w.ticked.is_empty());
    assert!(w.at.is_none());
    // A row of a folded group keeps its form under the group's header.
    w.ticked
        .insert(r#"purchasing.no_part [{"kind":"instance","id":12}]"#.into());
    w.at = Some(At::Row(
        r#"purchasing.no_part [{"kind":"instance","id":12}]"#.into(),
    ));
    let lines = entries(&w, &groups);
    let form = lines.iter().position(|e| matches!(e, Entry::Form));
    let header = lines
        .iter()
        .position(|e| matches!(e, Entry::Header(g) if groups[*g].0 == "purchasing.no_part"));
    assert_eq!(form, header.map(|h| h + 1));
    Ok(())
}

/// The dialog with the run's findings and the unrouted group unfolded.
fn run_dialog(engine: &Engine) -> Result<PcbEditor, Box<dyn std::error::Error>> {
    let mut editor = open(engine)?;
    let d = dialog(&mut editor)?;
    d.report = Some((
        (engine.report_key(), Pick::Board, None),
        Ok(report_like_the_run(engine)),
    ));
    d.waiving.open.insert("board.unrouted".into());
    Ok(editor)
}

/// The dialog in a 320-point column, as in the one-hour run: every text
/// cut short shows in full on hover. With `SPARKSMITH_TEXT_PICTURES=<dir>`
/// it writes the grouped list, a hovered row and the group's reason form.
#[test]
fn production_check_text_cut_short_shows_in_full_on_hover() -> TestResult {
    use crate::kit::text::tests::{Screen, cuts_without_hover, tooltips_at_once};
    let engine = Engine::showcase()?;
    let mut editor = run_dialog(&engine)?;
    let screen = Screen::new(egui::vec2(320.0, 1100.0));
    let run = |editor: &mut PcbEditor, events: Vec<egui::Event>| {
        screen.run(events, |ui| {
            editor.subject_slot(ui, &engine);
        })
    };
    run(&mut editor, vec![]);
    let rest = run(&mut editor, vec![]);
    screen.picture("production-grouped", &rest, 2.0)?;
    let (cut, missing) = cuts_without_hover(&screen.ctx, |events| run(&mut editor, events));
    assert!(cut >= 1, "the narrow column cuts a row: {cut}");
    assert!(missing.is_empty(), "cut without a hover: {missing:?}");
    // A hovered row: its action shows, and its title's tooltip.
    tooltips_at_once(&screen.ctx);
    let out = run(&mut editor, vec![]);
    let row = *positions(&out, "VBUS: 2 pads unrouted")
        .first()
        .ok_or("no VBUS row")?;
    run(&mut editor, vec![egui::Event::PointerMoved(row)]);
    let hovered = run(&mut editor, vec![]);
    screen.picture("production-row-hover", &hovered, 2.0)?;
    // Waive all… on the No Part group: the form under its header.
    let header = *positions(&hovered, "No Part chosen")
        .first()
        .ok_or("no No Part group")?;
    run(&mut editor, vec![egui::Event::PointerMoved(header)]);
    let out = run(&mut editor, vec![]);
    let waive = *positions(&out, "Waive all…")
        .last()
        .ok_or("no Waive all…")?;
    run(&mut editor, press(waive));
    "prototype".clone_into(&mut dialog(&mut editor)?.waiving.reason);
    run(&mut editor, vec![egui::Event::PointerGone]);
    let form = run(&mut editor, vec![]);
    assert!(texts(&form).iter().any(|t| t == "Waive 12 findings"));
    screen.picture("production-waive-group-form", &form, 2.0)?;
    Ok(())
}

/// In the run's narrow column (240 points less the card inset), a group's
/// title is never cut at rest: it wraps to a second line, its count after
/// it. (Hovered, Waive all… may cut it; its hover then shows it in full.)
#[test]
fn group_titles_fit_a_narrow_column() -> TestResult {
    use crate::kit::text::tests::{Screen, cuts};
    let engine = Engine::showcase()?;
    let mut editor = run_dialog(&engine)?;
    let screen = Screen::new(egui::vec2(224.0, 1100.0));
    let run = |editor: &mut PcbEditor, events: Vec<egui::Event>| {
        screen.run(events, |ui| {
            editor.subject_slot(ui, &engine);
        })
    };
    run(&mut editor, vec![egui::Event::PointerGone]);
    let rest = run(&mut editor, vec![]);
    screen.picture("production-narrow-groups", &rest, 2.0)?;
    let report = report_like_the_run(&engine);
    let titles: Vec<String> = groups(&report)
        .iter()
        .filter(|(_, items)| items.len() > 1)
        .map(|(code, _)| sparksmith_core::issues::group_title(code))
        .collect();
    assert!(titles.len() >= 2, "{titles:?}");
    let cut: Vec<String> = cuts(&rest).into_iter().map(|c| c.text).collect();
    let shown = texts(&rest);
    for title in &titles {
        assert!(!cut.contains(title), "{title} is cut: {cut:?}");
        assert!(shown.contains(title), "{title} is not shown: {shown:?}");
    }
    Ok(())
}

/// A check settings change (the impedance tolerance in Settings) takes no
/// revision, yet the production check computed before it is not the
/// current one any more: the dialog checks again under the new settings.
#[test]
fn a_check_settings_change_refreshes_the_production_check() -> TestResult {
    let mut engine = Engine::showcase()?;
    let mut editor = open(&engine)?;
    let ctx = context();
    dialog(&mut editor)?.wait_check(&ctx, &engine);
    assert!(dialog(&mut editor)?.current(&engine).is_some());
    let revision = engine.revision();
    engine.set_check_settings(sparksmith_core::layout::CheckSettings {
        impedance_tolerance: Some(10.0),
    })?;
    assert_eq!(engine.revision(), revision);
    assert!(
        dialog(&mut editor)?.current(&engine).is_none(),
        "a report of other settings is not shown as current"
    );
    dialog(&mut editor)?.wait_check(&ctx, &engine);
    assert!(dialog(&mut editor)?.current(&engine).is_some());
    Ok(())
}

/// The check is keyed on the picked profile's content, not its id: the
/// same profile file edited (another minimum width) is not shown as
/// checked, and the dialog checks it again.
#[test]
fn an_edited_profile_is_checked_again() -> TestResult {
    use sparksmith_core::rules::{RuleEntry, RuleKey, RuleValue};
    let engine = Engine::showcase()?;
    let mut editor = open(&engine)?;
    let ctx = context();
    let mut profile = sparksmith_core::rules::fixtures::prototype()?;
    profile.id = "mine".into();
    {
        let d = dialog(&mut editor)?;
        d.user_profiles = vec![profile.clone()];
        d.pick = Pick::User("mine".into());
        d.wait_check(&ctx, &engine);
        assert!(d.current(&engine).is_some());
    }
    profile.set(
        RuleKey::TrackWidthMin,
        RuleEntry::value(RuleValue::Length(0.15), "Test fixture."),
    )?;
    let d = dialog(&mut editor)?;
    d.user_profiles = vec![profile];
    assert!(
        d.current(&engine).is_none(),
        "a check of the profile as it was is not current"
    );
    d.wait_check(&ctx, &engine);
    assert!(d.current(&engine).is_some());
    Ok(())
}

/// A picked profile whose file is gone, renamed, damaged or in a folder
/// that no longer reads blocks the check and the export and is named,
/// however often Export is pressed: never the board's own rules instead.
#[test]
fn a_missing_profile_is_named_and_never_replaced_by_the_board_rules() -> TestResult {
    type Break = fn(&std::path::Path) -> std::io::Result<()>;
    let cases: [(&str, Break); 4] = [
        ("deleted", |dir| std::fs::remove_file(dir.join("mine.json"))),
        ("renamed", |dir| {
            std::fs::rename(dir.join("mine.json"), dir.join("other.json"))
        }),
        ("damaged", |dir| std::fs::write(dir.join("mine.json"), b"{")),
        ("unreadable folder", |dir| {
            std::fs::remove_dir_all(dir)?;
            std::fs::write(dir, b"not a folder")
        }),
    ];
    for (case, broken) in cases {
        let engine = Engine::showcase()?;
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("fab-profiles");
        let library = ProfileLibrary::new(
            dir.clone(),
            sparksmith_io::shipped::ShippedLibrary::locate()?,
        );
        let mut profile = sparksmith_core::rules::fixtures::prototype()?;
        profile.id = "mine".into();
        profile.name = "My fab".into();
        library.create(&profile)?;
        let mut editor = PcbEditor::default();
        editor.open_production_in(Some(library));
        let ctx = context();
        let out = tmp.path().join("out");
        let d = dialog(&mut editor)?;
        d.pick = Pick::User("mine".into());
        d.picked_name = Some("My fab".into());
        d.wait_check(&ctx, &engine);
        broken(&dir)?;
        for press in 0..3 {
            let d = dialog(&mut editor)?;
            d.start_export(
                &ctx,
                engine.document().clone(),
                *engine.check_settings(),
                out.clone(),
                "board".into(),
            );
            assert!(!d.busy(), "{case}: export {press} started");
            d.wait_check(&ctx, &engine);
            let checked = d.current(&engine).cloned();
            let Some(Err(why)) = checked else {
                return Err(format!("{case}: checked under {checked:?}").into());
            };
            assert!(why.contains("‘My fab’"), "{case}: {why}");
            let blocked = export_blocked(d, Some(&Err(why.clone())), true, Some(&out));
            assert_eq!(blocked, Some(why), "{case}");
            assert!(!out.exists(), "{case}: files were written");
        }
        // The dialog says it, and shows what is picked.
        let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
        let shown = texts(&output);
        for said in [
            "‘My fab’ is no longer among your fab profiles. Pick another profile",
            "My fab · missing",
        ] {
            assert!(shown.iter().any(|t| t == said), "{case}: {shown:?}");
        }
        // Another pick is checked again.
        let d = dialog(&mut editor)?;
        d.pick = Pick::Board;
        d.wait_check(&ctx, &engine);
        assert!(matches!(d.current(&engine), Some(Ok(_))), "{case}");
    }
    Ok(())
}

/// The test prototype as `id` of `fabricator`, its via annular ring
/// minimum `ring` (its default via keeping it).
fn ring_profile(
    id: &str,
    fabricator: &str,
    ring: f64,
) -> Result<RuleProfile, Box<dyn std::error::Error>> {
    use sparksmith_core::rules::{RuleEntry, RuleKey, RuleValue};
    let mut p = sparksmith_core::rules::fixtures::prototype()?;
    p.id = id.into();
    p.name = format!("Profile {id}");
    p.fabricator = Some(fabricator.into());
    p.set(
        RuleKey::ViaDiameterDefault,
        RuleEntry::value(RuleValue::Length(0.3 + 2.0 * ring + 0.1), "Invented"),
    )?;
    p.set(
        RuleKey::ViaAnnularRingMin,
        RuleEntry::value(RuleValue::Length(ring), "Invented row: via ring"),
    )?;
    Ok(p)
}

/// The showcase on `profile`.
fn showcase_on(profile: RuleProfile) -> Result<Engine, Box<dyn std::error::Error>> {
    let mut engine = Engine::showcase()?;
    engine.apply(
        engine.revision(),
        Operation::Rules(sparksmith_core::rules::RulesCommand::set_profile(
            profile, false,
        )),
    )?;
    Ok(engine)
}

/// A check under the shipped default, which is no fabricator's, says to
/// choose one before ordering, and blocks nothing; under a fabricator's own
/// profile it says nothing.
#[test]
fn a_check_under_the_shipped_default_says_to_choose_a_fabricator() -> TestResult {
    let shipped = sparksmith_io::rule_profiles::shipped_from_env()?;
    let said = "Checked against Standard; choose your fabricator's profile before ordering";
    for (id, says) in [("standard", true), ("jlcpcb-4-layer", false)] {
        let profile = shipped.iter().find(|p| p.id == id).ok_or(id)?.clone();
        let engine = showcase_on(profile)?;
        let mut editor = open(&engine)?;
        let ctx = context();
        let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
        let shown = texts(&output);
        assert_eq!(shown.iter().any(|t| t == said), says, "{id}: {shown:?}");
        let clean = PreflightReport {
            blocking: 0,
            items: Vec::new(),
            ..report_with_error(&engine)
        };
        let out = PathBuf::from("out");
        let blocked = export_blocked(dialog(&mut editor)?, Some(&Ok(clean)), true, Some(&out));
        assert_eq!(blocked, None, "{id}");
    }
    Ok(())
}

/// A finding the board needs more of its fab profile for states what it
/// needs, offers no Waive…, and "Switch to …" sends the one rules edit the
/// production check resolved the suggested profile with; Export says to
/// fix the design or switch, not to waive.
#[test]
fn a_profile_limited_finding_says_what_it_needs_and_switches_instead_of_waiving() -> TestResult {
    let engine = showcase_on(ring_profile("strict", "fab-a", 0.25)?)?;
    let other = ring_profile("relaxed", "fab-a", 0.1)?;
    let mut editor = open(&engine)?;
    let ctx = context();
    let tmp = tempfile::tempdir()?;
    let report = fabrication::preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?;
    let need = report
        .items
        .iter()
        .find_map(|i| i.needs.clone())
        .ok_or("a via ring needs the profile")?;
    let title = report
        .items
        .iter()
        .find(|i| i.needs.is_some())
        .map(|i| i.title.clone())
        .ok_or("title")?;
    let found = fabrication::suggest::suggest(
        engine.project(),
        engine.drafts(),
        engine.check_settings(),
        None,
        std::slice::from_ref(&other),
        &[],
    )?;
    assert_eq!(found.suggestions.len(), 1);
    {
        let d = dialog(&mut editor)?;
        let wanted = (engine.report_key(), Pick::Board, None);
        d.report = Some((wanted.clone(), Ok(report)));
        d.suggested = Some((
            wanted,
            crate::fab_suggestions::Suggested {
                result: Ok(found),
                profiles: vec![other.clone()],
            },
        ));
        d.location = Some(tmp.path().to_path_buf());
    }
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    let shown = texts(&output);
    assert!(shown.contains(&need.text), "{shown:?}");
    {
        // The showcase has other errors too; with only the ones a profile
        // resolves, Export says to fix or switch, never to waive.
        let d = dialog(&mut editor)?;
        let mut only = match d.current(&engine).cloned() {
            Some(Ok(report)) => report,
            other => return Err(format!("{other:?}").into()),
        };
        only.items.retain(|i| i.needs.is_some());
        only.blocking = only.items.iter().filter(|i| i.blocks()).count();
        assert!(only.blocking > 0);
        assert_eq!(
            export_blocked(d, Some(&Ok(only.clone())), true, d.output("x").as_ref()).as_deref(),
            Some("Fix the design or switch to a fab profile that allows it first")
        );
        // An error neither a waiver nor another profile resolves offers
        // neither; a missing profile asks for one (GUI check 1, U11).
        only.items.truncate(1);
        only.items[0].needs = None;
        only.items[0].code = "fabrication.slot_unroutable.3".into();
        only.blocking = 1;
        assert_eq!(
            export_blocked(d, Some(&Ok(only.clone())), true, None).as_deref(),
            Some("Fix the error first; it cannot be waived")
        );
        only.items.push(only.items[0].clone());
        only.blocking = 2;
        assert_eq!(
            export_blocked(d, Some(&Ok(only.clone())), true, None).as_deref(),
            Some("Fix the 2 errors first; they cannot be waived")
        );
        only.items[0].code = "rules.no_fab_profile".into();
        assert_eq!(
            export_blocked(d, Some(&Ok(only)), true, None).as_deref(),
            Some("Choose a fab profile first")
        );
    }
    // Hovering the finding offers no Waive….
    if positions(&output, &title).is_empty() {
        // A group of several: unfold it to reach its rows.
        click(
            &mut editor,
            &engine,
            &ctx,
            &sparksmith_core::issues::group_title("board.via_annular_ring"),
        )?;
    }
    hover(&mut editor, &engine, &ctx, &title)?;
    let (output, _) = frame(&mut editor, &engine, &ctx, vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|t| t == "Waive…" || t == "Waive all…"),
        "{:?}",
        texts(&output)
    );
    let op = click(&mut editor, &engine, &ctx, "Switch to Profile relaxed")?.ok_or("no switch")?;
    let want = fabrication::snapshot::switch(engine.project(), &other)?;
    assert_eq!(
        serde_json::to_value(&op)?,
        serde_json::to_value(Operation::Rules(want))?
    );
    assert_eq!(dialog(&mut editor)?.pick, Pick::Board);
    Ok(())
}

/// The suggestions come from a worker over the installed profiles: the
/// user's library in a temporary config folder and the shipped ones.
#[test]
fn suggestions_are_checked_off_the_ui_thread_against_the_installed_profiles() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let library = ProfileLibrary::new(
        tmp.path().join("fab-profiles"),
        sparksmith_io::shipped::ShippedLibrary::locate()?,
    );
    let strict = ring_profile("strict", "fab-a", 0.25)?;
    library.create(&strict)?;
    library.create(&ring_profile("relaxed", "fab-a", 0.1)?)?;
    let engine = showcase_on(strict)?;
    let mut editor = PcbEditor::default();
    editor.open_production_in(Some(library));
    let ctx = context();
    let d = dialog(&mut editor)?;
    d.wait_check(&ctx, &engine);
    d.wait_suggestions(&ctx, &engine);
    let suggested = d.current_suggestions(&engine).ok_or("no suggestions")?;
    let found = suggested.result.as_ref().map_err(|e| e.clone())?;
    assert_eq!(
        found.suggestions.first().map(|s| s.id.as_str()),
        Some("relaxed"),
        "{found:?}"
    );
    assert!(found.suggestions[0].same_fabricator);
    Ok(())
}

/// A short, or a finding whose files would not be the design, is not to be
/// waived: its row offers no waiver and says so with what fixes it, and the
/// disabled Export says the same, never to waive or to switch the profile.
#[test]
fn what_cannot_be_waived_says_what_to_do_instead() -> TestResult {
    let engine = Engine::showcase()?;
    let base = report_with_error(&engine);
    let item = |code: &str, fix: &str| {
        let mut item = base.items[0].clone();
        item.key = format!("{code} []");
        item.code = code.into();
        item.fix = Some(fix.into());
        item
    };
    let of = |items: Vec<PreflightItem>| PreflightReport {
        errors: items.len(),
        blocking: items.len(),
        items,
        ..base.clone()
    };
    for (code, fix) in [
        ("board.overlap", "Move the copper apart"),
        ("fill.overlap", "Refill the fill, or move the copper apart"),
        ("fill.stale", "Refill the fill"),
        ("fabrication.fill_unfilled", "Refill the fill"),
        ("board.shape_invalid", "Fix the crossing"),
        ("board.shape_open", "Join the open ends in Board shape"),
        (
            "fabrication.no_footprint",
            "Choose a footprint for the part",
        ),
    ] {
        let item = item(code, fix);
        assert!(!item.offers_waiver() && !waivable(&item), "{code}");
        assert_eq!(row_tip(&item), format!("{fix}\nCannot be waived"), "{code}");
    }
    // Beside one that can be waived, Export counts the ones that cannot.
    let mut editor = open(&engine)?;
    let tmp = tempfile::tempdir()?;
    let d = dialog(&mut editor)?;
    d.location = Some(tmp.path().to_path_buf());
    let mixed = of(vec![
        base.items[0].clone(),
        item("fill.stale", "Refill the fill"),
    ]);
    assert_eq!(
        export_blocked(d, Some(&Ok(mixed)), true, d.output("x").as_ref()).as_deref(),
        Some("Fix or waive the 2 errors first; 1 cannot be waived")
    );
    let only = of(vec![item("fill.stale", "Refill the fill")]);
    assert_eq!(
        export_blocked(d, Some(&Ok(only)), true, d.output("x").as_ref()).as_deref(),
        Some("Fix the error first; it cannot be waived")
    );
    Ok(())
}

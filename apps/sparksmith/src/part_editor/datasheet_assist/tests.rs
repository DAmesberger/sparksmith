//! The start-page entry opens the form; progress and the result
//! arrive from the background thread.
use super::*;

/// Run one headless frame drawing `add`; the painted texts.
fn frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    mut add: impl FnMut(&mut Ui),
) -> Vec<(String, egui::Rect)> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ui| add(ui),
    );
    output.textures_delta.clear();
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => out.push((
                t.galley.text().to_owned(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &output.shapes {
        walk(&s.shape, &mut out);
    }
    out
}

fn draw(ui: &mut Ui, assist: &mut DatasheetAssist) {
    super::super::start::start_new_for_tests(ui, assist);
    assist.ui(ui);
}

#[test]
fn the_start_page_button_opens_the_form() -> Result<(), String> {
    let ctx = egui::Context::default();
    sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
    let mut assist = DatasheetAssist::default();
    let painted = frame(&ctx, vec![], |ui| draw(ui, &mut assist));
    let (_, rect) = painted
        .iter()
        .find(|(t, _)| t == "Create from datasheet…")
        .ok_or_else(|| format!("the button: {painted:?}"))?;
    assert!(
        !painted.iter().any(|(t, _)| t == "Datasheets"),
        "closed at first"
    );
    let at = rect.center();
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [
        vec![egui::Event::PointerMoved(at)],
        vec![press(true)],
        vec![press(false)],
    ] {
        frame(&ctx, events, |ui| draw(ui, &mut assist));
    }
    assert!(assist.open);
    let painted = frame(&ctx, vec![], |ui| draw(ui, &mut assist));
    for label in ["Datasheets", "Part name", "Save as", "Model", "Start"] {
        assert!(
            painted.iter().any(|(t, _)| t == label),
            "{label}: {painted:?}"
        );
    }
    // The model and its key are set in Settings › AI, not here.
    assert!(
        painted.iter().any(|(t, _)| t.contains("Settings › AI")),
        "{painted:?}"
    );
    for gone in ["Provider", "Endpoint", "API key"] {
        assert!(!painted.iter().any(|(t, _)| t == gone), "{gone}");
    }
    // No prices in the product: no cost cap, no spend.
    assert!(
        !painted
            .iter()
            .any(|(t, _)| t.contains("Cost") || t.contains('$') || t.contains("USD")),
        "{painted:?}"
    );
    // The MCP workflow is offered beside the built-in run.
    assert!(
        painted
            .iter()
            .any(|(t, _)| t.contains("datasheet_to_component")
                && t.contains("Connect AI assistants")),
        "{painted:?}"
    );
    Ok(())
}

/// Demo run 3: the form opens below a long start page; opening it scrolls
/// it into view, its heading carries no count, and its last rows clear the
/// canvas's floating navigation bar.
#[test]
fn the_form_opens_in_view() -> Result<(), String> {
    let ctx = egui::Context::default();
    sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
    let mut assist = DatasheetAssist::default();
    let view = 500.0;
    let page = |ui: &mut Ui, assist: &mut DatasheetAssist| {
        egui::ScrollArea::vertical()
            .id_salt("start-page-test")
            .max_height(view)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // The search and recent lists above it.
                ui.add_space(380.0);
                draw(ui, assist);
                ui.add_space(crate::navigation::bar_band());
            });
    };
    let painted = frame(&ctx, vec![], |ui| page(ui, &mut assist));
    let button = painted
        .iter()
        .find(|(t, r)| t == "Create from datasheet…" && r.bottom() < view)
        .map(|(_, r)| r.center());
    let at = button.ok_or("the button is in view, near the bottom")?;
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [
        vec![egui::Event::PointerMoved(at)],
        vec![press(true)],
        vec![press(false)],
    ] {
        frame(&ctx, events, |ui| page(ui, &mut assist));
    }
    assert!(assist.open);
    let mut painted = Vec::new();
    // The scroll eases in over a few frames.
    for _ in 0..60 {
        painted = frame(&ctx, vec![], |ui| page(ui, &mut assist));
    }
    let heading = painted
        .iter()
        .find(|(t, _)| t.starts_with("CREATE FROM DATASHEET"))
        .ok_or_else(|| format!("the heading: {painted:?}"))?;
    assert_eq!(heading.0, "CREATE FROM DATASHEET", "no count");
    assert!(
        // Unscrolled it would sit near the bottom, past 400.
        heading.1.top() >= 0.0 && heading.1.top() < view / 2.0,
        "scrolled to the form's heading: {heading:?}"
    );
    let key = painted
        .iter()
        .find(|(t, _)| t == "Model")
        .ok_or_else(|| format!("the model row: {painted:?}"))?;
    assert!(key.1.bottom() <= view, "the model row is in view: {key:?}");
    Ok(())
}

#[test]
fn progress_and_the_result_arrive_from_the_thread() {
    let (tx, rx) = channel();
    let mut assist = DatasheetAssist {
        open: true,
        job: Some(Job {
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
        }),
        ..DatasheetAssist::default()
    };
    let sent = [
        Message::Line("Request 1".into()),
        Message::Done(Ok(Finished {
            stop: "review report submitted".into(),
            output_tokens: 1000,
            project: PathBuf::from("x.sparksmith"),
            review: PathBuf::from("x.review.md"),
        })),
    ];
    for message in sent {
        assert!(tx.send(message).is_ok());
    }
    assist.poll();
    assert!(assist.job.is_none(), "the run is over");
    assert_eq!(assist.log, ["Request 1"]);
    assert!(
        matches!(&assist.result, Some(Ok(f)) if f.review.as_path() == std::path::Path::new("x.review.md"))
    );
}

fn local_endpoint() -> ai_settings::EndpointSetting {
    ai_settings::EndpointSetting {
        name: "Local".into(),
        provider: ai_settings::Wire::Openai,
        base_url: "http://localhost:11434/v1".into(),
        model: "qwen3.8:27b".into(),
        timeout_minutes: 90,
        capabilities: Some(ai_settings::Declared {
            tool_calling: true,
            vision: true,
            native_pdf: false,
            context_tokens: 65_536,
            output_tokens: None,
        }),
    }
}

/// The run takes the model, as declared, from Settings › AI, and the
/// datasheet workflow's own budget; the person's limits, when set, hold it.
#[test]
fn the_run_takes_the_endpoint_and_the_workflows_budget() -> Result<(), String> {
    let ctx = egui::Context::default();
    sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
    let library = sparksmith_io::shipped::ShippedLibrary::locate().map_err(|e| e.to_string())?;
    let own = ai_settings::workflow_budget(&library, WORKFLOW).map_err(|e| e.to_string())?;
    let mut assist = DatasheetAssist {
        open: true,
        chosen: Some(Ok(Chosen {
            endpoint: local_endpoint(),
            limits: None,
        })),
        datasheets: "a.pdf".into(),
        name: "X".into(),
        ..DatasheetAssist::default()
    };
    let painted = frame(&ctx, vec![], |ui| assist.ui(ui));
    assert!(
        painted
            .iter()
            .any(|(t, _)| t.starts_with("Local: qwen3.8:27b") && t.contains("this computer")),
        "{painted:?}"
    );
    let plan = assist.plan()?;
    let p = &plan.config.profile;
    assert_eq!(p.provider, Provider::OpenAiCompatible);
    assert_eq!(p.capabilities.context_tokens, 65_536);
    assert!(p.capabilities.vision && !p.capabilities.native_pdf);
    assert!(plan.endpoint.is_loopback());
    assert_eq!(plan.config.budget.max_requests, own.requests);
    assert_eq!(
        plan.config.budget.max_duration,
        std::time::Duration::from_secs(u64::from(own.minutes) * 60)
    );
    // The person's tighter limit holds the workflow's budget.
    let mut limits = ai_settings::Shipped::locate()
        .map_err(|e| e.to_string())?
        .limits_start;
    limits.minutes = 5;
    if let Some(Ok(chosen)) = &mut assist.chosen {
        chosen.limits = Some(limits);
    }
    let plan = assist.plan()?;
    assert_eq!(
        plan.config.budget.max_duration,
        std::time::Duration::from_secs(5 * 60)
    );
    Ok(())
}

/// Without an endpoint in Settings › AI, Start is off and says where to
/// add one; nothing starts.
#[test]
fn without_an_endpoint_nothing_starts_and_settings_is_named() -> Result<(), String> {
    let ctx = egui::Context::default();
    sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
    let why = "No model endpoint: add one in Settings › AI".to_owned();
    let mut assist = DatasheetAssist {
        open: true,
        chosen: Some(Err(why.clone())),
        datasheets: "a.pdf".into(),
        name: "X".into(),
        out: "x.sparksmith".into(),
        ..DatasheetAssist::default()
    };
    let painted = frame(&ctx, vec![], |ui| assist.ui(ui));
    assert!(
        painted.iter().any(|(t, _)| t == "None set up"),
        "{painted:?}"
    );
    assist.start();
    assert!(assist.job.is_none(), "nothing started");
    assert_eq!(assist.result, Some(Err(why)));
    Ok(())
}

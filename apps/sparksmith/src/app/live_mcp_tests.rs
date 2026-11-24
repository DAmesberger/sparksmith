//! Live MCP against a whole app, in process and without a window: an
//! assistant's request over the loopback socket runs on the app's engine
//! between frames, joins the person's undo history and is refused while the
//! person holds the window.
use super::super::*;
use super::Place;
use crate::commands::Registry;
use serde_json::{Value, json};
use sparksmith_automation::live::{self, Client, ClientToken};
use sparksmith_ui::fonts;

#[path = "live_mcp_selection_tests.rs"]
mod selection;

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// No token or discovery files: the test never touches the user's folders.
const TEST_APP: live::App = live::App {
    name: sparksmith_core::product::NAME,
    data_dir: || None,
    env: |_| None,
    utc: |secs| format!("{secs}"),
    default_port: live::DEFAULT_PORT,
};

struct Harness {
    app: App,
    ctx: egui::Context,
    frame: eframe::Frame,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// The demo project in a headless app, with live MCP turned on.
    fn new() -> Fallible<Self> {
        let dir = tempfile::tempdir()?;
        let ctx = egui::Context::default();
        fonts::setup_bundled_fonts(&ctx);
        let engine = Engine::demo()?;
        let files = FileUi::with_directory(
            &ctx,
            rfd::AsyncFileDialog::new(),
            &engine,
            dir.path().to_path_buf(),
        );
        let mut app = App::with_parts(
            &ctx,
            viewport::Viewport::detached(),
            Parts {
                engine,
                files,
                sources: LibrarySources::new(None),
                registry: Arc::new(Registry::standard()),
                keymap_problems: Vec::new(),
                messages: Messages::default(),
                browser_width: chrome::BROWSER_WIDTH,
                #[cfg(target_os = "macos")]
                native_menu: None,
            },
        );
        crate::keys::publish(&ctx, &app.registry);
        app.live.use_place(Place {
            app: TEST_APP,
            listen: live::Listen::Exact(0),
            user_sources: false,
        });
        app.live.set_on(&ctx, true);
        let mut harness = Self {
            app,
            ctx,
            frame: eframe::Frame::_new_kittest(),
            _dir: dir,
        };
        harness.frame(Vec::new());
        Ok(harness)
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.frame_output(events).textures_delta.clear();
    }

    fn frame_output(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1380.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let Self {
            app, ctx, frame, ..
        } = self;
        ctx.run_ui(input, |ui| eframe::App::ui(app, ui, frame))
    }

    fn client(&self) -> Fallible<Client> {
        let endpoint = self.app.live.endpoint().ok_or("live MCP is off")?;
        Ok(Client::with_token(
            endpoint.port(),
            ClientToken {
                token: endpoint.token().to_owned(),
                file: None,
                app: TEST_APP,
            },
        ))
    }

    /// Send one request from another thread, as an MCP bridge does, and run
    /// frames until its reply arrives.
    fn call(&mut self, method: &str, params: Value) -> Fallible<Value> {
        let mut client = self.client()?;
        let method = method.to_owned();
        let asked = std::thread::spawn(move || client.call(&method, params));
        for _ in 0..5000 {
            if asked.is_finished() {
                break;
            }
            self.frame(Vec::new());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(asked.join().map_err(|_| "client thread")??)
    }

    /// `layout.add_via` at `revision`.
    fn add_via(&self, revision: u64) -> Value {
        let net = self.app.engine.project().nets.first().map_or(0, |n| n.id);
        json!({"entry": "layout.add_via", "expected_revision": revision, "params": {
            "via": {"net": net, "position": [1.0, 1.0], "drill": 0.3, "diameter": 0.6, "span": "through", "tented": true}
        }})
    }

    fn said(&self, text: &str) -> bool {
        self.app.messages.entries().any(|m| m.text.contains(text))
    }
}

fn code(reply: &Value) -> &str {
    reply["error"]["code"].as_str().unwrap_or_default()
}

/// The assistant's via lands in the open project with a small reply; the
/// person's Undo takes it back, says it was the assistant's, and the
/// assistant's own undo needs the revision it saw.
#[test]
fn an_assistant_edits_the_open_project_with_shared_undo() -> Fallible {
    let mut h = Harness::new()?;
    let revision = h.app.engine.revision();
    let before = h.app.engine.document().clone();
    let reply = h.call(live::CALL, h.add_via(revision))?;
    assert_eq!(reply["ok"], true, "{reply}");
    let bytes = reply.to_string().len();
    assert!(bytes < 4 * 1024, "the reply is {bytes} bytes: {reply}");
    assert_eq!(h.app.engine.revision(), revision + 1);
    assert_eq!(
        h.app.engine.project().vias.len(),
        before.project.vias.len() + 1
    );
    // Its title, never its wire name (night round 5).
    assert!(h.said("AI assistant edit: Add a via"));

    // The person undoes it in the window: the same history.
    h.app.history(false);
    assert_eq!(h.app.engine.document(), &before);
    assert!(h.said("Undid AI assistant edit: Add a via"));

    // The assistant redoes at the revision it last saw: refused, stale.
    let stale = h.call(
        live::CALL,
        json!({"entry": "project.redo", "params": {}, "expected_revision": revision + 1}),
    )?;
    assert_eq!(code(&stale), "stale_revision", "{stale}");
    let now = h.call(live::REVISION, json!({}))?;
    let current = now["result"]["revision"].as_u64().ok_or("revision")?;
    assert_eq!(current, h.app.engine.revision());
    let redone = h.call(
        live::CALL,
        json!({"entry": "project.redo", "params": {}, "expected_revision": current}),
    )?;
    assert_eq!(redone["ok"], true, "{redone}");
    assert_eq!(
        h.app.engine.project().vias.len(),
        before.project.vias.len() + 1
    );
    // The window says what the assistant's redo applied, by its title.
    assert!(h.said("AI assistant redid: Add a via · PCB"));
    assert_eq!(redone["result"]["step"]["title"], "Add a via", "{redone}");
    Ok(())
}

/// While the person holds the window (the command palette open, a pointer
/// held down), a change is refused with the reason and nothing changes; a
/// read still answers. The window's own file requests are refused: the
/// files module is outside its grant.
#[test]
fn changes_wait_while_the_person_holds_the_window() -> Fallible {
    let mut h = Harness::new()?;
    let revision = h.app.engine.revision();
    let before = h.app.engine.document().clone();

    h.app.palette.open();
    h.frame(Vec::new());
    let refused = h.call(live::CALL, h.add_via(revision))?;
    assert_eq!(code(&refused), "live.busy", "{refused}");
    let read = h.call(
        live::CALL,
        json!({"entry": "project.inspect", "params": {}}),
    )?;
    assert_eq!(read["ok"], true, "{read}");
    h.app.palette.close();
    h.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);

    let at = egui::pos2(700.0, 450.0);
    h.frame(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let dragging = h.call(live::CALL, h.add_via(revision))?;
    assert_eq!(code(&dragging), "live.busy", "{dragging}");
    assert!(
        dragging["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("dragging")),
        "{dragging}"
    );
    h.frame(vec![egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert_eq!(h.app.engine.document(), &before);
    assert_eq!(h.app.engine.revision(), revision);

    let open = h.call(
        live::CALL,
        json!({"entry": "project.open", "params": {"path": "/tmp/x.sparksmith"}, "expected_revision": revision}),
    )?;
    assert_eq!(code(&open), "gate.module", "{open}");
    let missing = h.call(live::CALL, json!({"entry": "project.undo", "params": {}}))?;
    assert_eq!(code(&missing), "malformed_request", "{missing}");
    assert_eq!(h.app.engine.revision(), revision);
    Ok(())
}

/// An Ask-class request waits for the person: Deny answers the assistant
/// and writes nothing; Allow runs it.
#[test]
fn an_ask_request_waits_for_the_person() -> Fallible {
    let mut h = Harness::new()?;
    let revision = h.app.engine.revision();
    let dir = tempfile::tempdir_in(std::env::temp_dir())?;
    let path = dir.path().join("board.solidcraft");
    let export = json!({"entry": "project.export_native",
        "params": {"path": path.display().to_string(), "tier": "full", "write_anyway": true}});
    let sparksmith_core::QueryResult::Preflight(report) = h
        .app
        .engine
        .query(sparksmith_core::Query::Preflight { profile: None })?
    else {
        return Err("expected the demo board's preflight report".into());
    };
    assert!(
        report.warnings > 0,
        "the export card must review a real warning"
    );

    let mut client = h.client()?;
    let sent = export.clone();
    let asked = std::thread::spawn(move || client.call(live::CALL, sent));
    for _ in 0..5000 {
        if h.app.live_held().is_some() {
            break;
        }
        h.frame(Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(h.app.live_held(), Some("project.export_native"));
    let output = h.frame_output(Vec::new());
    let labels: Vec<&str> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    let revision_label = revision.to_string();
    let summary = crate::pcb_editor::report_summary(&report);
    assert!(summary.contains("warning"), "{summary}");
    for text in [
        "Board revision",
        revision_label.as_str(),
        "Geometry tier",
        "Full",
        "Write even if geometry differs",
        "Preflight",
        summary.as_str(),
    ] {
        assert!(
            labels.contains(&text),
            "missing {text} on the approval card"
        );
    }
    let finding = report.items.first().ok_or("preflight finding")?;
    assert!(
        labels.contains(&finding.title.as_str()),
        "missing {}",
        finding.title
    );
    let warning = report
        .items
        .iter()
        .find(|item| {
            item.severity == sparksmith_core::issues::Severity::Warning && item.waiver.is_none()
        })
        .ok_or("a warning to review")?;
    let position = |output: &egui::FullOutput, text: &str| {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            // Drawn where its clip shows some of it: a cell's text starts
            // on the cell's own edge.
            egui::Shape::Text(t)
                if t.galley.job.text == text
                    && shape
                        .clip_rect
                        .intersect(egui::Rect::from_min_size(t.pos, t.galley.size()))
                        .is_positive() =>
            {
                Some(t.pos)
            }
            _ => None,
        })
    };
    let over_table =
        position(&output, &finding.title).ok_or("a visible preflight row")? + egui::vec2(2.0, 2.0);
    let mut visible = output;
    // Scroll the card's report if errors precede the first warning.
    for _ in 0..report.items.len() {
        if position(&visible, &warning.title).is_some() {
            break;
        }
        visible = h.frame_output(vec![
            egui::Event::PointerMoved(over_table),
            egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -2.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
    }
    let row = position(&visible, &warning.title).ok_or("the warning title")?;
    assert!(
        visible
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(t)
                if t.galley.job.text == "Warning" && t.pos.y == row.y && t.pos.x < row.x
            )),
        "the card must label the warning on its own row"
    );
    assert!(!path.exists());
    // Meanwhile another change is refused: the window waits for an answer.
    let waiting = h.call(live::CALL, h.add_via(revision))?;
    assert_eq!(code(&waiting), "live.busy", "{waiting}");
    h.app.live_answer(false);
    let denied = asked.join().map_err(|_| "client thread")??;
    assert_eq!(code(&denied), "gate.denied", "{denied}");
    assert!(!path.exists());

    let mut client = h.client()?;
    let asked = std::thread::spawn(move || client.call(live::CALL, export));
    for _ in 0..5000 {
        if h.app.live_held().is_some() {
            break;
        }
        h.frame(Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    h.app.live_answer(true);
    let allowed = asked.join().map_err(|_| "client thread")??;
    assert_eq!(allowed["ok"], true, "{allowed}");
    assert_eq!(allowed["result"]["tier"], "full", "{allowed}");
    assert_eq!(allowed["result"]["revision"], revision, "{allowed}");
    assert!(path.exists());
    Ok(())
}

/// Turning access off closes the socket: a bridge gets a clear failure.
#[test]
fn turning_access_off_closes_the_channel() -> Fallible {
    let mut h = Harness::new()?;
    let mut client = h.client()?;
    assert!(h.app.live.is_on());
    h.app.live.set_on(&h.ctx, false);
    assert!(!h.app.live.is_on());
    h.frame(Vec::new());
    assert!(client.call(live::REVISION, json!({})).is_err());
    Ok(())
}

/// Over live MCP, `library.source.add` changes the person's settings, so
/// it waits for the person; Deny answers the assistant and leaves the
/// sources as they were.
#[test]
fn live_mcp_asks_before_adding_a_library_source() -> Fallible {
    let mut h = Harness::new()?;
    let dir = tempfile::tempdir_in(std::env::temp_dir())?;
    let add = json!({"entry": "library.source.add",
        "params": {"path": dir.path().display().to_string()}});
    let mut client = h.client()?;
    let asked = std::thread::spawn(move || client.call(live::CALL, add));
    for _ in 0..5000 {
        if h.app.live_held().is_some() {
            break;
        }
        h.frame(Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(h.app.live_held(), Some("library.source.add"));
    h.app.live_answer(false);
    let denied = asked.join().map_err(|_| "client thread")??;
    assert_eq!(code(&denied), "gate.denied", "{denied}");
    assert!(h.app.live.session.sources().list().is_empty());
    Ok(())
}

/// An export the person was asked about for one project is refused once
/// another is open: it never writes the other one's files.
#[test]
fn a_held_export_ends_with_the_project_it_was_asked_for() -> Fallible {
    let mut h = Harness::new()?;
    let dir = tempfile::tempdir_in(std::env::temp_dir())?;
    let output = dir.path().join("fab");
    let export = json!({"entry": "project.export_fabrication",
        "params": {"output": output.display().to_string()}});
    let mut client = h.client()?;
    let asked = std::thread::spawn(move || client.call(live::CALL, export));
    for _ in 0..5000 {
        if h.app.live_held().is_some() {
            break;
        }
        h.frame(Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(h.app.live_held(), Some("project.export_fabrication"));
    h.app
        .replace(Engine::showcase()?.to_file(), None, None, false);
    h.frame(Vec::new());
    assert_eq!(h.app.live_held(), None);
    // Allow, had it been pressed, finds nothing to run.
    h.app.live_answer(true);
    let refused = asked.join().map_err(|_| "client thread")??;
    assert_eq!(code(&refused), "live.stale_approval", "{refused}");
    assert!(!output.exists());
    Ok(())
}

/// I-A2: a read that names a path asks the person and, on Allow, runs: it
/// is not refused for a revision it has no field for.
#[test]
fn live_mcp_runs_a_path_read_on_allow() -> Fallible {
    let mut h = Harness::new()?;
    let dir = tempfile::tempdir_in(std::env::temp_dir())?;
    let part = dir.path().join("smd-nut.solidcraft");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../library/parts/smd-nut.solidcraft"
        ),
        &part,
    )?;
    let check = json!({"entry": "library.check_part_file",
        "params": {"path": part.display().to_string()}});
    let mut client = h.client()?;
    let asked = std::thread::spawn(move || client.call(live::CALL, check));
    for _ in 0..5000 {
        if h.app.live_held().is_some() {
            break;
        }
        h.frame(Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(h.app.live_held(), Some("library.check_part_file"));
    h.app.live_answer(true);
    let reply = asked.join().map_err(|_| "client thread")??;
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["report"]["ok"], true, "{reply}");
    Ok(())
}

/// The access checkbox starts at the window's left edge, under the state
/// note, not after an empty label column.
#[test]
fn the_access_checkbox_starts_at_the_left() -> Fallible {
    let mut h = Harness::new()?;
    h.app.live.open = true;
    h.frame(Vec::new());
    let output = h.frame_output(Vec::new());
    let left = |pick: &dyn Fn(&str) -> bool| {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if pick(&t.galley.job.text) => Some(t.pos.x),
            _ => None,
        })
    };
    let check = left(&|t| t == "Let AI assistants edit this project").ok_or("no checkbox")?;
    let note = left(&|t| t.starts_with("On, on this computer only")).ok_or("no state note")?;
    let t = sparksmith_ui::theme::Tokens::get();
    assert!(
        check - note < t.form_label_width / 2.0,
        "checkbox text at {check}, note at {note}"
    );
    Ok(())
}

/// S5a over the live channel: the assistant selects a placed part by id and
/// the PCB inspects it, opens the Layout and 3D views and a sheet, and
/// reads what the window shows. None of it changes the revision, adds an
/// undo step or asks for a model; steering is refused while the person
/// drags, and reading what the window shows still answers.
#[test]
fn an_assistant_steers_the_window_and_the_project_stays() -> Fallible {
    let mut h = Harness::new()?;
    // A step of the assistant's own first: the one undo below takes it
    // back, so no view call added a step after it.
    let before = h.app.engine.document().clone();
    let reply = h.call(live::CALL, h.add_via(h.app.engine.revision()))?;
    assert_eq!(reply["ok"], true, "{reply}");
    let revision = h.app.engine.revision();
    let (id, designator) = {
        let project = h.app.engine.project();
        let placement = project.placements.first().ok_or("no placed part")?;
        let instance = project.instance(placement.instance).ok_or("no part")?;
        (instance.id, instance.designator.clone())
    };
    // The person shows the 3D view once, so its model is asked for now.
    h.app.window().show_pcb(true);
    for _ in 0..3 {
        h.frame(Vec::new());
    }
    let requests = h.app.scene.requests();

    let opened = h.call(
        live::CALL,
        json!({"entry": "view.open", "params": {"workspace": "pcb", "view": "layout"}}),
    )?;
    assert_eq!(opened["result"]["view"], "layout", "{opened}");
    assert!(!h.app.pcb.is_3d());

    let selected = h.call(
        live::CALL,
        json!({"entry": "view.select", "params": {"ids": [id]}}),
    )?;
    assert_eq!(selected["result"]["workspace"], "pcb", "{selected}");
    assert_eq!(h.app.workspace, Workspace::Pcb);
    assert_eq!(h.app.pcb.selected_instance(), Some(id));

    let opened = h.call(
        live::CALL,
        json!({"entry": "view.open", "params": {"workspace": "pcb", "view": "3d"}}),
    )?;
    assert_eq!(opened["result"]["view"], "3d", "{opened}");
    assert!(h.app.pcb.is_3d());
    // The selection is the design's, the same in 3D.
    assert_eq!(
        opened["result"]["selection"],
        json!([{"kind": "instance", "id": id, "name": designator}]),
        "{opened}"
    );

    let sheet = h.app.engine.project().sheets.first();
    let opened = h.call(
        live::CALL,
        json!({"entry": "view.open", "params": {"workspace": "schematic", "sheet": sheet}}),
    )?;
    assert_eq!(opened["result"]["sheet"]["id"], json!(sheet), "{opened}");
    assert_eq!(h.app.workspace, Workspace::Schematic);
    assert_eq!(h.app.schematic.active_sheet(), sheet);

    let context = h.call(live::CALL, json!({"entry": "view.context", "params": {}}))?;
    assert_eq!(context["result"]["workspace"], "schematic", "{context}");
    assert_eq!(context["result"]["revision"], revision, "{context}");

    // While the person drags, steering waits; reading answers.
    let at = egui::pos2(700.0, 450.0);
    h.frame(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let dragging = h.call(
        live::CALL,
        json!({"entry": "view.open", "params": {"workspace": "pcb"}}),
    )?;
    assert_eq!(code(&dragging), "live.busy", "{dragging}");
    assert_eq!(h.app.workspace, Workspace::Schematic);
    let read = h.call(live::CALL, json!({"entry": "view.context", "params": {}}))?;
    assert_eq!(read["ok"], true, "{read}");
    h.frame(vec![egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);

    // Back in 3D (the function the person's own commands run), frames ask
    // for no model: nothing was rebuilt.
    h.app.window().show_pcb(true);
    for _ in 0..3 {
        h.frame(Vec::new());
    }
    assert_eq!(
        h.app.scene.requests(),
        requests,
        "a view call rebuilt the model"
    );
    assert_eq!(h.app.engine.revision(), revision);
    // One undo takes back the via: the view calls added no step.
    h.app.history(false);
    assert_eq!(h.app.engine.document(), &before);
    Ok(())
}

#[test]
fn a_connection_dialog_holds_the_live_edit_until_closed() -> Fallible {
    let mut h = Harness::new()?;
    let revision = h.app.engine.revision();
    let before = h.app.engine.document().clone();
    h.app.live.toggle_window();
    h.frame(Vec::new());
    assert!(h.app.live.is_open());
    let batch = json!({"entry":"project.apply_all", "expected_revision":revision, "params":{
        "operations": [{"editor":"Layout", "operation":{"command":"layout.add_via",
            "params":h.add_via(revision)["params"]}}]
    }});
    assert_eq!(code(&h.call(live::CALL, batch.clone())?), "live.busy");
    assert_eq!(h.app.engine.document(), &before);
    assert_eq!(
        h.call(live::REVISION, json!({}))?["result"]["revision"],
        revision
    );
    h.app.live.close();
    assert_eq!(h.call(live::CALL, batch)?["ok"], true);
    assert_eq!(h.app.engine.revision(), revision + 1);
    Ok(())
}

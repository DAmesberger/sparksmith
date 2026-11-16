//! The datasheet workflow in the whole app's chat, driven through its
//! accessibility tree on a scripted model (no network, no paid calls): a
//! message with an `@` file and the part number, or a dropped PDF, offers
//! the workflow on one compact card filled from the conversation (an inline
//! field only for the part name nothing gave), Accept and continue (Enter)
//! starts the run, the run block follows the steps with Stop, the
//! generator choice waits for Continue or Change, the pinout's impact card
//! for Apply, and the result card opens the drafts in the editor and
//! undoes the whole run as one step. Stop mid-run leaves the document as it
//! was; Just chat goes on as chat; the start page's Create from datasheet…
//! opens the chat on the card. At 1024×640 and 1920×1080.
use super::super::ax_harness::{Ax, Fallible};
use crate::assistant::State;
use crate::assistant::worker::ChatModel;
use crate::shell::Workspace;
use egui::accesskit::Role;
use serde_json::{Value, json};
use sparksmith_assist::fixtures;
use sparksmith_assist::model::{
    ModelBackend, ModelProfile, ModelReply, ModelRequest, Part, Role as Speaker, StopReason,
};
use sparksmith_assist::usage::Usage;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const SMALL: egui::Vec2 = egui::vec2(1024.0, 640.0);
const LARGE: egui::Vec2 = egui::vec2(1920.0, 1080.0);

/// A model answering from a script shared with the test.
struct Script(Arc<Mutex<VecDeque<ModelReply>>>);

impl ModelBackend for Script {
    fn label(&self) -> String {
        "script".into()
    }
    fn send(&mut self, _: &ModelRequest<'_>) -> sparksmith_assist::Result<ModelReply> {
        self.0
            .lock()
            .ok()
            .and_then(|mut replies| replies.pop_front())
            .ok_or_else(|| sparksmith_assist::Error::Protocol("the script ended".into()))
    }
    fn count_prompt_tokens(
        &mut self,
        _: &ModelRequest<'_>,
    ) -> sparksmith_assist::Result<Option<u64>> {
        Ok(None)
    }
}

fn reply(content: Vec<Part>, stop: StopReason) -> ModelReply {
    ModelReply {
        model: "scripted".into(),
        content,
        stop,
        usage: Usage {
            input_tokens: 2_000,
            output_tokens: 300,
            ..Usage::default()
        },
        usage_estimated: false,
    }
}

/// The model's answer to a judgement step.
fn answer(step: &str, result: Value) -> ModelReply {
    reply(
        vec![Part::ToolCall {
            id: format!("answer-{step}"),
            name: "query".into(),
            input: json!({"name": "assist.step_result", "params": {"step": step, "result": result}}),
            malformed: None,
            replay: None,
        }],
        StopReason::ToolCalls,
    )
}

fn text(text: &str) -> ModelReply {
    reply(vec![Part::text(text)], StopReason::EndTurn)
}

/// The three judgement steps' answers for EXR-22, then `after`.
fn script(after: Vec<ModelReply>) -> Vec<ModelReply> {
    fixtures::exr22_answers()
        .into_iter()
        .map(|(step, result)| answer(step, result))
        .chain(after)
        .collect()
}

/// The demo project on the Component workspace at `window`, its chat on
/// `replies`.
fn app(window: egui::Vec2, replies: Vec<ModelReply>) -> Fallible<Ax> {
    let mut ax = Ax::new(Workspace::Part)?;
    ax.window = window;
    let script = Arc::new(Mutex::new(VecDeque::from(replies)));
    ax.app.assistant.model = Some(ChatModel {
        profile: ModelProfile::openai_compatible("scripted", 131_072),
        connect: Arc::new(move || {
            Ok(Box::new(Script(Arc::clone(&script))) as Box<dyn ModelBackend>)
        }),
    });
    ax.settle();
    Ok(ax)
}

/// A file dropped on the window from this computer.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// Frames until `done`: a run's steps take real work (the PDF read, the
/// generator), so this waits longer than the harness's own `until`.
fn wait(ax: &mut Ax, what: &str, mut done: impl FnMut(&Ax) -> bool) -> Fallible {
    for _ in 0..6_000 {
        ax.frame(vec![]);
        if done(ax) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Err(format!(
        "{what} did not happen; the chat is {:?}:\n{}",
        ax.app.assistant.conversation.state,
        ax.dump()
    )
    .into())
}

/// Drop EXR-22's datasheet on the window; the workflow's compact card
/// shows with the file read and an inline field for the part name only.
fn drop_datasheet(ax: &mut Ax) -> Fallible {
    let pdf = ax.dir.path().join("exr22.pdf");
    fixtures::exr22_datasheet(&pdf)?;
    ax.drop_files(vec![Arc::new(Dropped(pdf))]);
    wait(ax, "the workflow card", |ax| {
        ax.has(Role::Button, "Accept and continue")
            && !ax.all_prefix(Role::Button, "exr22.pdf · 2 pages").is_empty()
    })?;
    ax.find(Role::Label, "Datasheet → component")?;
    ax.find(Role::TextInput, "Part name")?;
    assert!(!ax.has(Role::TextInput, "Notes"), "optional and empty");
    Ok(())
}

/// The part name in its inline field, then Enter: the run reads the
/// datasheet and plans the package, and waits on the generator choice.
fn start(ax: &mut Ax) -> Fallible {
    // The transcript scrolls the new card into view first.
    ax.settle();
    ax.click(Role::TextInput, "Part name")?;
    ax.type_text("EXR-22");
    ax.settle();
    ax.key(egui::Key::Enter);
    generator_choice(ax)
}

fn generator_choice(ax: &mut Ax) -> Fallible {
    wait(ax, "the generator choice", |ax| {
        ax.has(Role::Button, "Choose another…") && ax.has(Role::Button, "Continue")
    })?;
    ax.settle();
    Ok(())
}

fn drafts(ax: &Ax) -> usize {
    ax.app.engine.drafts().items.len()
}

fn undo_line(ax: &Ax) -> Option<String> {
    ax.app.engine.undo_step().map(|s| s.line())
}

#[test]
fn a_dropped_datasheet_runs_in_the_chat_and_undoes_as_one_step() -> Fallible {
    for size in [SMALL, LARGE] {
        let mut ax = app(size, script(vec![text("I made EXR-22's drafts.")]))?;
        let before = (drafts(&ax), undo_line(&ax));
        drop_datasheet(&mut ax)?;
        start(&mut ax)?;
        // The run block: the workflow, its steps so far and Stop.
        ax.find(Role::Label, "Datasheet → component: EXR-22")?;
        assert!(ax.has(Role::Button, "Stop"));
        assert_eq!(ax.app.assistant.conversation.state, State::Waiting);
        // Nothing is made before the person lets the plan go on.
        assert_eq!(drafts(&ax), before.0);
        ax.button("Continue")?;
        wait(&mut ax, "the pinout's impact card", |ax| {
            ax.has(Role::Button, "Apply")
        })?;
        ax.settle();
        ax.button("Apply")?;
        wait(&mut ax, "the run's end", |ax| {
            ax.app.assistant.run.is_none()
        })?;
        ax.settle();
        assert_eq!(
            ax.app.assistant.conversation.state,
            State::Idle,
            "{:?}",
            ax.app.assistant.notes
        );
        // The drafts, with the pins the import brought, as one undo step.
        assert!(drafts(&ax) >= before.0 + 4, "{}", drafts(&ax));
        let made =
            sparksmith_automation::Session::lending().lend(&mut ax.app.engine, None, |session| {
                sparksmith_assist::report::drafts(&session.snapshot())
            });
        assert!(
            made.iter()
                .any(|d| d["kind"] == "component" && d["name"] == "EXR-22"),
            "{made:?}"
        );
        assert_eq!(
            undo_line(&ax).as_deref(),
            Some("Datasheet → component: EXR-22")
        );
        // The result card opens the component in the editor.
        ax.find(Role::Button, "Undo run")?;
        ax.button("Open in editor")?;
        ax.settle();
        assert_eq!(ax.app.workspace, Workspace::Part);
        assert!(ax.app.part.lineage().is_some());
        // The model is told how the run ended with the next message, and
        // goes on with it.
        assert!(
            ax.app
                .assistant
                .notes
                .iter()
                .any(|n| n.contains("ended: done")),
            "{:?}",
            ax.app.assistant.notes
        );
        ax.ctx
            .memory_mut(|m| m.request_focus(crate::assistant::Assistant::composer_id()));
        ax.frame(vec![]);
        ax.type_text("What did you make?");
        ax.key(egui::Key::Enter);
        wait(&mut ax, "the reply", |ax| {
            ax.app.assistant.job.is_none() && !ax.app.assistant.conversation.state.busy()
        })?;
        let told = ax.app.assistant.history.iter().any(|m| {
            m.role == Speaker::User
                && m.content.iter().any(|p| {
                    matches!(p, Part::Text { text, .. } if text.contains("Datasheet → component: EXR-22") && text.contains("Its result"))
                })
        });
        assert!(told, "{:?}", ax.app.assistant.history);
        assert!(ax.app.assistant.notes.is_empty());
        // One undo takes the whole run back: drafts, pins and all.
        ax.button("Undo run")?;
        ax.settle();
        assert_eq!((drafts(&ax), undo_line(&ax)), before);
    }
    Ok(())
}

/// Stop while the run waits on the generator choice, before anything is
/// made: the run ends stopped and the document is as it was. At 1024×640 by
/// the composer's Stop, at 1920×1080 by the run block's.
#[test]
fn stop_mid_run_leaves_the_document_unchanged() -> Fallible {
    for (size, which) in [(SMALL, 1), (LARGE, 0)] {
        let mut ax = app(size, script(Vec::new()))?;
        let revision = ax.app.engine.revision();
        let before = (drafts(&ax), undo_line(&ax));
        drop_datasheet(&mut ax)?;
        start(&mut ax)?;
        // The run block's Stop first, the composer's below it.
        let stops: Vec<egui::Pos2> = ax
            .all(Role::Button, "Stop")
            .iter()
            .map(|n| n.center())
            .collect();
        assert_eq!(stops.len(), 2);
        ax.click_at(stops[which]);
        wait(&mut ax, "the run's end", |ax| {
            ax.app.assistant.run.is_none()
        })?;
        ax.settle();
        assert_eq!(ax.app.assistant.conversation.state, State::Stopped);
        assert_eq!(ax.app.engine.revision(), revision);
        assert_eq!((drafts(&ax), undo_line(&ax)), before);
        assert!(!ax.has(Role::Button, "Change"));
        assert!(
            ax.app.assistant.notes.iter().any(|n| n.contains("stopped")),
            "{:?}",
            ax.app.assistant.notes
        );
    }
    Ok(())
}

#[test]
fn just_chat_goes_on_as_chat() -> Fallible {
    let mut ax = app(LARGE, vec![text("Here is what the datasheet says.")])?;
    let revision = ax.app.engine.revision();
    drop_datasheet(&mut ax)?;
    ax.button("Just chat")?;
    ax.settle();
    assert!(ax.app.assistant.run.is_none());
    assert!(!ax.has(Role::Button, "Accept and continue"));
    ax.ctx
        .memory_mut(|m| m.request_focus(crate::assistant::Assistant::composer_id()));
    ax.frame(vec![]);
    ax.type_text("What is its pitch?");
    ax.key(egui::Key::Enter);
    wait(&mut ax, "the reply", |ax| {
        ax.app.assistant.job.is_none() && !ax.app.assistant.conversation.state.busy()
    })?;
    assert_eq!(ax.app.assistant.conversation.state, State::Idle);
    let first = ax.app.assistant.history.first().ok_or("a message")?;
    assert!(
        first
            .content
            .iter()
            .any(|p| matches!(p, Part::Text { text, .. }
            if text.contains("Just chat") && text.contains("What is its pitch?"))),
        "{first:?}"
    );
    assert!(
        ax.texts
            .iter()
            .any(|t| t.contains("what the datasheet says")),
        "{:?}",
        ax.texts
    );
    assert_eq!(ax.app.engine.revision(), revision);
    Ok(())
}

#[test]
fn create_from_datasheet_opens_the_chat_on_the_workflow_card() -> Fallible {
    for size in [SMALL, LARGE] {
        let mut ax = app(size, Vec::new())?;
        assert!(!ax.app.assistant.open);
        ax.button("Create from datasheet…")?;
        ax.settle();
        assert!(ax.app.assistant.shown());
        assert_eq!(ax.app.workflow_cards.offers(), 1);
        ax.find(Role::Label, "Datasheet → component")?;
        ax.find(Role::Button, "Accept and continue")?;
        // Pressed again while the card waits, it offers no second card.
        ax.button("Create from datasheet…")?;
        ax.settle();
        assert_eq!(ax.app.workflow_cards.offers(), 1);
    }
    Ok(())
}

/// The model's `assist.offer` for EXR-22 with the part number it read in
/// the message; the datasheet it leaves to the message's `@` file.
fn offer_reply() -> ModelReply {
    reply(
        vec![Part::ToolCall {
            id: "offer".into(),
            name: "show".into(),
            input: json!({"name": "assist.offer", "params": {
                "workflow": "datasheet-to-component",
                "inputs": {"part": "EXR-22"},
                "why": "You named the part and attached its datasheet."}}),
            malformed: None,
            replay: None,
        }],
        StopReason::ToolCalls,
    )
}

/// `@` in the composer lists the project folder's files by a fuzzy match;
/// Enter puts the one chosen in the message as a chip. The message "make a
/// part from @exr22.pdf, it's the EXR-22" gets one compact card with both
/// filled (the file from the chip, the part from the model's offer), and
/// Enter on it runs the workflow.
#[test]
fn a_message_with_an_at_file_fills_one_card_and_enter_runs_it() -> Fallible {
    for size in [SMALL, LARGE] {
        let replies: Vec<ModelReply> = [offer_reply(), text("I offered the datasheet workflow.")]
            .into_iter()
            .chain(script(Vec::new()))
            .collect();
        let mut ax = app(size, replies)?;
        let folder = ax.dir.path().join("datasheets");
        std::fs::create_dir_all(&folder)?;
        let pdf = folder.join("exr22.pdf");
        fixtures::exr22_datasheet(&pdf)?;
        ax.app.toggle_assistant(&ax.ctx.clone());
        ax.settle();
        ax.ctx
            .memory_mut(|m| m.request_focus(crate::assistant::Assistant::composer_id()));
        ax.frame(vec![]);
        ax.type_text("make a part from @e2p");
        ax.settle();
        // The list: the project folder's file, where it is.
        ax.find(Role::Button, "exr22.pdf")?;
        ax.key(egui::Key::Enter);
        assert_eq!(ax.app.assistant.composer, "make a part from @exr22.pdf ");
        assert_eq!(ax.app.assistant.files, vec![pdf.clone()]);
        ax.find(Role::Label, "@exr22.pdf")?;
        ax.find(Role::Button, "Remove exr22.pdf")?;
        ax.type_text("it's the EXR-22");
        ax.key(egui::Key::Enter);
        wait(&mut ax, "the filled card", |ax| {
            ax.has(Role::Button, "Accept and continue")
                && !ax.all_prefix(Role::Button, "exr22.pdf · 2 pages").is_empty()
                && ax.app.assistant.job.is_none()
        })?;
        ax.settle();
        // One card, both filled, no field left to fill.
        assert_eq!(ax.app.workflow_cards.offers(), 1);
        ax.find(Role::TextInput, "Part name")?;
        // The model was told the file and its path with the message.
        let told = ax.app.assistant.history.iter().any(|m| {
            m.role == Speaker::User
                && m.content.iter().any(|p| matches!(p, Part::Text { text, .. }
                    if text.contains("@exr22.pdf: ") && text.contains("it's the EXR-22")))
        });
        assert!(told, "{:?}", ax.app.assistant.history);
        ax.key(egui::Key::Enter);
        generator_choice(&mut ax)?;
        // The run is labelled with the part the model filled in.
        ax.find(Role::Label, "Datasheet → component: EXR-22")?;
        assert!(ax.app.assistant.run.is_some(), "the run goes on");
    }
    Ok(())
}

/// A path typed into the composer becomes a chip once a space follows it.
#[test]
fn a_typed_path_becomes_a_chip() -> Fallible {
    let mut ax = app(SMALL, Vec::new())?;
    let pdf = ax.dir.path().join("exr22.pdf");
    fixtures::exr22_datasheet(&pdf)?;
    ax.app.toggle_assistant(&ax.ctx.clone());
    ax.settle();
    ax.ctx
        .memory_mut(|m| m.request_focus(crate::assistant::Assistant::composer_id()));
    ax.frame(vec![]);
    ax.type_text(&format!("read {} please", pdf.display()));
    ax.settle();
    assert_eq!(ax.app.assistant.composer, "read @exr22.pdf please");
    assert_eq!(ax.app.assistant.files, vec![pdf]);
    ax.button("Remove exr22.pdf")?;
    assert!(ax.app.assistant.files.is_empty());
    assert_eq!(ax.app.assistant.composer, "read please");
    Ok(())
}

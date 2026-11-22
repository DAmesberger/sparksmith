//! The assistant chat in the whole app, driven through its accessibility
//! tree on a scripted model (no network): the column opens by its key and
//! its application-bar button, folds, docks left and keeps the canvas room
//! at 1024×640; a message runs one turn whose tool rows, reply and summary
//! card show, whose edits undo as one step from the card; an Ask waits for
//! the person; Stop ends a turn.
use super::super::ax_harness::{Ax, Fallible};
use crate::assistant::worker::ChatModel;
use crate::assistant::{Item, RowState, State};
use crate::shell::Workspace;
use egui::accesskit::Role;
use serde_json::{Value, json};
use sparksmith_assist::model::{
    ModelBackend, ModelProfile, ModelReply, ModelRequest, Part, StopReason,
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
            input_tokens: 1_000,
            output_tokens: 50,
            ..Usage::default()
        },
        usage_estimated: false,
    }
}

fn text(text: &str) -> ModelReply {
    reply(vec![Part::text(text)], StopReason::EndTurn)
}

fn tool(verb: &str, input: Value) -> ModelReply {
    reply(
        vec![Part::ToolCall {
            id: "c1".into(),
            name: verb.into(),
            input,
            malformed: None,
            replay: None,
        }],
        StopReason::ToolCalls,
    )
}

/// The app on the demo board at `window`, its chat on `replies`.
fn chat(window: egui::Vec2, replies: Vec<ModelReply>) -> Fallible<Ax> {
    let mut ax = Ax::new(Workspace::Pcb)?;
    ax.window = window;
    let script = Arc::new(Mutex::new(VecDeque::from(replies)));
    ax.app.assistant.model = Some(ChatModel {
        profile: ModelProfile::openai_compatible("scripted", 32_768),
        connect: Arc::new(move || {
            Ok(Box::new(Script(Arc::clone(&script))) as Box<dyn ModelBackend>)
        }),
    });
    ax.settle();
    Ok(ax)
}

fn shift_command() -> egui::Modifiers {
    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
}

/// Open the column with ⇧⌘A, type `message` into the composer (focused on
/// opening) and press Enter.
fn send(ax: &mut Ax, message: &str) -> Fallible {
    if !ax.app.assistant.shown() {
        ax.key_with(egui::Key::A, shift_command());
    }
    // A click on a card's button took the keyboard: back to the composer.
    ax.ctx
        .memory_mut(|m| m.request_focus(crate::assistant::Assistant::composer_id()));
    ax.frame(vec![]);
    ax.type_text(message);
    ax.key(egui::Key::Enter);
    Ok(())
}

fn idle(ax: &Ax) -> bool {
    !ax.app.assistant.conversation.state.busy() && ax.app.assistant.job.is_none()
}

/// A via on the demo board's first net at `x`, as `apply` takes it.
fn add_via(ax: &Ax, x: f64) -> Value {
    let net = ax.app.engine.project().nets.first().map_or(0, |n| n.id);
    json!({"expected_revision": ax.app.engine.revision(), "operations": [{"op": "layout.add_via",
        "params": {"via": {"net": net, "position": [x, 1.0], "drill": 0.3, "diameter": 0.6,
        "span": "through", "tented": true}}}]})
}

#[test]
fn the_column_opens_by_key_and_button_folds_docks_and_leaves_the_canvas_room() -> Fallible {
    let mut ax = chat(SMALL, vec![])?;
    assert!(!ax.has(Role::Button, "Close the assistant"));
    ax.key_with(egui::Key::A, shift_command());
    let close = ax.find(Role::Button, "Close the assistant")?;
    // Docked at the right edge, within its widths.
    assert!(close.rect.right() > SMALL.x - 60.0, "{:?}", close.rect);
    let send = ax.find(Role::Button, "Send")?;
    assert!(send.disabled, "nothing typed yet");
    let left = send.rect.left().min(close.rect.left());
    assert!(
        SMALL.x - left <= crate::assistant::MAX_WIDTH,
        "{}",
        ax.dump()
    );
    // The canvas keeps the rest: the application bar and ribbon still show.
    ax.find_prefix(Role::Button, "Command palette")?;
    // Fold to the strip, and back.
    ax.button("Fold the assistant")?;
    let unfold = ax.find(Role::Button, "Unfold the assistant")?;
    assert!(unfold.rect.left() > SMALL.x - crate::assistant::STRIP_WIDTH - 1.0);
    assert!(!ax.has(Role::Button, "Send"));
    ax.button("Unfold the assistant")?;
    // Dock left.
    ax.button("Dock the assistant left")?;
    ax.settle();
    let close = ax.find(Role::Button, "Close the assistant")?;
    assert!(
        close.rect.right() < crate::assistant::MAX_WIDTH + 20.0,
        "{:?}",
        close.rect
    );
    ax.button("Close the assistant")?;
    assert!(!ax.has(Role::Button, "Send"));
    // The application bar's button opens it again; so does the key.
    ax.click(Role::Button, "Show Assistant (⇧⌘A)")
        .or_else(|_| ax.click(Role::Button, "Show Assistant (Ctrl+Shift+A)"))?;
    ax.find(Role::Button, "Send")?;
    ax.key_with(egui::Key::A, shift_command());
    assert!(!ax.has(Role::Button, "Send"));
    Ok(())
}

#[test]
fn a_message_is_one_turn_whose_edits_undo_as_one_step_from_its_card() -> Fallible {
    let mut ax = chat(LARGE, vec![])?;
    let start = ax.app.engine.revision();
    let vias = ax.app.engine.project().vias.len();
    let first = add_via(&ax, 1.0);
    let mut second = add_via(&ax, 2.0);
    second["expected_revision"] = json!(start + 1);
    if let Some(model) = &ax.app.assistant.model {
        let script = vec![
            tool("query", json!({"name": "view.context"})),
            tool("apply", first),
            tool("apply", second),
            text("Added **two vias** on the first net."),
        ];
        let script = Arc::new(Mutex::new(VecDeque::from(script)));
        ax.app.assistant.model = Some(ChatModel {
            profile: model.profile.clone(),
            connect: Arc::new(move || {
                Ok(Box::new(Script(Arc::clone(&script))) as Box<dyn ModelBackend>)
            }),
        });
    }
    send(&mut ax, "Add two vias")?;
    ax.until("the turn's end", idle)?;
    ax.settle();
    let assistant = &ax.app.assistant;
    assert_eq!(
        assistant.conversation.state,
        State::Idle,
        "{:?}",
        assistant.conversation.items
    );
    let items = &assistant.conversation.items;
    assert!(matches!(&items[0], Item::You { text, context, .. }
        if text == "Add two vias" && context.starts_with("PCB · Layout")));
    let rows: Vec<(&str, &RowState)> = items
        .iter()
        .filter_map(|i| match i {
            Item::Call(row) => Some((row.line.as_str(), &row.state)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("query view.context", &RowState::Done),
            ("apply layout.add_via", &RowState::Done),
            ("apply layout.add_via", &RowState::Done),
        ]
    );
    // The reply, the card and the rows show.
    assert!(
        ax.texts.iter().any(|t| t.contains("two vias")),
        "{:?}",
        ax.texts
    );
    assert!(
        ax.texts.iter().any(|t| t == "Done · 1 undo step"),
        "{:?}",
        ax.texts
    );
    assert!(
        ax.texts.iter().any(|t| t == "Vias: 2 added"),
        "{:?}",
        ax.texts
    );
    assert_eq!(ax.app.engine.project().vias.len(), vias + 2);
    assert_eq!(
        ax.app.engine.undo_step().map(|s| s.line()).as_deref(),
        Some("AI assistant: Add two vias")
    );
    // Show selects what it made; Undo turn takes both back as one step.
    ax.button("Show")?;
    ax.settle();
    ax.button("Undo turn")?;
    ax.settle();
    assert_eq!(ax.app.engine.project().vias.len(), vias);
    assert!(ax.find(Role::Button, "Undo turn")?.disabled);
    // The model's history keeps the conversation for the next message.
    assert_eq!(ax.app.assistant.history.len(), 8);
    Ok(())
}

#[test]
fn an_ask_waits_for_the_person_and_deny_tells_the_model() -> Fallible {
    let mut ax = chat(
        SMALL,
        vec![
            tool("act", json!({"name": "project.save", "params": {}})),
            text("I left the file as it was."),
        ],
    )?;
    send(&mut ax, "Save it")?;
    ax.until("the Ask card", |ax| ax.has(Role::Button, "Deny"))?;
    assert_eq!(ax.app.assistant.conversation.state, State::Waiting);
    assert!(
        ax.texts.iter().any(|t| t == "Allow: Save the project?"),
        "{:?}",
        ax.texts
    );
    ax.button("Deny")?;
    ax.until("the turn's end", idle)?;
    let refused = ax.app.assistant.conversation.items.iter().any(|i| {
        matches!(i, Item::Call(row) if row.state == RowState::Refused && row.reply.contains("gate.denied"))
    });
    assert!(refused, "{:?}", ax.app.assistant.conversation.items);
    assert!(!ax.path().exists() || ax.app.assistant.conversation.state == State::Idle);
    Ok(())
}

#[test]
fn stop_ends_a_turn_waiting_on_the_person_and_the_next_message_runs() -> Fallible {
    let mut ax = chat(
        LARGE,
        vec![
            tool("act", json!({"name": "project.save", "params": {}})),
            text("unused"),
            text("Still here."),
        ],
    )?;
    send(&mut ax, "Save it")?;
    ax.until("the Ask card", |ax| ax.has(Role::Button, "Stop"))?;
    ax.until("the Ask card", |ax| ax.has(Role::Button, "Deny"))?;
    ax.button("Stop")?;
    ax.until("the turn's end", idle)?;
    assert_eq!(ax.app.assistant.conversation.state, State::Stopped);
    assert!(
        ax.texts.iter().any(|t| t == "Stopped by you."),
        "{:?}",
        ax.texts
    );
    send(&mut ax, "Are you there?")?;
    ax.until("the second turn's end", idle)?;
    assert!(
        ax.app
            .assistant
            .conversation
            .items
            .iter()
            .any(|i| matches!(i, Item::Reply(t) if t == "unused" || t == "Still here.")),
        "{:?}",
        ax.app.assistant.conversation.items
    );
    Ok(())
}

#[test]
fn a_new_project_starts_an_empty_conversation() -> Fallible {
    let mut ax = chat(SMALL, vec![text("Hello.")])?;
    send(&mut ax, "Hi")?;
    ax.until("the turn's end", idle)?;
    assert!(!ax.app.assistant.conversation.items.is_empty());
    ax.app.document_replaced();
    assert!(ax.app.assistant.conversation.items.is_empty());
    assert!(ax.app.assistant.history.is_empty());
    Ok(())
}

#[test]
fn allow_for_this_session_lets_the_entry_run_without_asking_again() -> Fallible {
    let save = || tool("act", json!({"name": "project.save", "params": {}}));
    let mut ax = chat(
        LARGE,
        vec![save(), text("Saved."), save(), text("Saved again.")],
    )?;
    send(&mut ax, "Save it")?;
    ax.until("the Ask card", |ax| {
        ax.has(Role::Button, "Allow for this session")
    })?;
    ax.button("Allow for this session")?;
    ax.until("the turn's end", idle)?;
    ax.settle();
    assert!(
        ax.texts.iter().any(|t| t == "Allowed for this session."),
        "{:?}",
        ax.texts
    );
    send(&mut ax, "Save again")?;
    ax.until("the second turn's end", idle)?;
    let items = &ax.app.assistant.conversation.items;
    let cards = items.iter().filter(|i| matches!(i, Item::Ask(_))).count();
    assert_eq!(cards, 1, "the second save asked nobody: {items:?}");
    let saved = items
        .iter()
        .filter(|i| matches!(i, Item::Call(row) if row.state == RowState::Done))
        .count();
    assert_eq!(saved, 2, "{items:?}");
    Ok(())
}

#[test]
fn the_chat_publishes_into_the_project_unasked_and_asks_to_publish_to_a_library() -> Fallible {
    let reach = sparksmith_assist::chat::ChatLibrary::locate()?.reach;
    assert!(reach.unasked("library.publish"));
    assert!(!reach.unasked("library.promote"));
    assert!(!reach.unasked("library.publish_to_folder"));
    // The chat's turns are limited by default (Settings › AI's limits).
    let limits = sparksmith_assist::settings::Shipped::locate()?.limits;
    assert!(limits.requests < u32::MAX && limits.total_tokens < u64::MAX);
    Ok(())
}

#[test]
fn the_chat_moves_the_view_without_asking_by_default() -> Fallible {
    let mut ax = chat(
        SMALL,
        vec![
            tool(
                "show",
                json!({"name": "view.open", "params": {"workspace": "schematic"}}),
            ),
            text("Opened."),
        ],
    )?;
    send(&mut ax, "Show me the schematic")?;
    ax.until("the turn's end", idle)?;
    assert!(
        !ax.has(Role::Button, "Allow once"),
        "no Ask card for moving the view"
    );
    assert_eq!(ax.app.workspace, Workspace::Schematic);
    Ok(())
}

/// Put `replies` next in the chat's script.
fn script(ax: &mut Ax, replies: Vec<ModelReply>) {
    if let Some(model) = &ax.app.assistant.model {
        let script = Arc::new(Mutex::new(VecDeque::from(replies)));
        ax.app.assistant.model = Some(ChatModel {
            profile: model.profile.clone(),
            connect: Arc::new(move || {
                Ok(Box::new(Script(Arc::clone(&script))) as Box<dyn ModelBackend>)
            }),
        });
    }
}

fn rename(ax: &mut Ax, name: &str) -> Fallible {
    ax.app.engine.apply(
        ax.app.engine.revision(),
        sparksmith_core::Operation::Schematic(
            sparksmith_core::schematic::SchematicCommand::RenameProject { name: name.into() },
        ),
    )?;
    ax.settle();
    Ok(())
}

fn save_in_window(ax: &mut Ax) -> Fallible {
    ax.app.files.request(crate::file_ui::Action::Save);
    ax.until("the save", |ax| {
        !ax.app.files.busy() && !ax.app.files.dirty()
    })
}

/// Review finding 8, through the real chat approval: with project A open
/// and unsaved, an approved chat open of B is refused and nothing moves.
/// Once A is saved, the approved open moves the window to B, its file,
/// saved state and recovery included, and the conversation carries on;
/// the next desktop Save writes B, never A.
#[test]
fn an_approved_chat_open_moves_the_window_to_the_opened_file() -> Fallible {
    let mut ax = chat(LARGE, vec![])?;
    let a = ax.path();
    let b = a.with_file_name("other.sparksmith");
    let mut other = sparksmith_core::Engine::demo()?;
    other.apply(
        other.revision(),
        sparksmith_core::Operation::Schematic(
            sparksmith_core::schematic::SchematicCommand::RenameProject {
                name: "Board B".into(),
            },
        ),
    )?;
    sparksmith_io::write_project(&b, &other.to_file())?;
    rename(&mut ax, "Board A")?;
    assert!(ax.app.files.dirty());

    let open = |ax: &Ax| {
        tool(
            "act",
            json!({"name": "project.open", "params": {
                "path": b.display().to_string(),
                "expected_revision": ax.app.engine.revision()}}),
        )
    };
    let first = open(&ax);
    script(&mut ax, vec![first, text("It has unsaved changes.")]);
    send(&mut ax, "Open the other board")?;
    ax.until("the Ask card", |ax| ax.has(Role::Button, "Allow once"))?;
    ax.button("Allow once")?;
    ax.until("the turn's end", idle)?;
    let refused = ax.app.assistant.conversation.items.iter().any(|i| {
        matches!(i, Item::Call(row) if row.state == RowState::Refused && row.reply.contains("unsaved changes"))
    });
    assert!(refused, "{:?}", ax.app.assistant.conversation.items);
    assert_eq!(ax.app.engine.project().name, "Board A");
    assert_eq!(ax.app.files.path(), Some(a.as_path()));
    assert!(ax.app.files.dirty());

    save_in_window(&mut ax)?;
    let second = open(&ax);
    script(&mut ax, vec![second, text("Opened.")]);
    send(&mut ax, "Open the other board now")?;
    ax.until("the Ask card", |ax| ax.has(Role::Button, "Allow once"))?;
    ax.button("Allow once")?;
    ax.until("the turn's end", idle)?;
    assert_eq!(ax.app.engine.project().name, "Board B");
    assert_eq!(ax.app.files.path(), Some(b.as_path()));
    assert!(!ax.app.files.dirty());
    // The assistant's own open keeps its conversation.
    assert!(
        ax.app
            .assistant
            .conversation
            .items
            .iter()
            .any(|i| matches!(i, Item::Reply(t) if t == "Opened.")),
        "{:?}",
        ax.app.assistant.conversation.items
    );

    rename(&mut ax, "Board B, edited")?;
    save_in_window(&mut ax)?;
    assert_eq!(
        sparksmith_io::read_project(&b)?.project.name,
        "Board B, edited"
    );
    assert_eq!(sparksmith_io::read_project(&a)?.project.name, "Board A");
    Ok(())
}

/// A person's edit made while a turn waits on them is not the assistant's:
/// the summary lists the turn's own changes, and the person's beside them.
#[test]
fn an_edit_made_during_a_turn_is_not_counted_as_the_assistants() -> Fallible {
    let mut ax = chat(LARGE, vec![])?;
    let start = ax.app.engine.revision();
    let vias = ax.app.engine.project().vias.len();
    let first = add_via(&ax, 1.0);
    let mut second = add_via(&ax, 2.0);
    // After the assistant's first via and the person's.
    second["expected_revision"] = json!(start + 2);
    script(
        &mut ax,
        vec![
            tool("apply", first),
            tool("act", json!({"name": "project.save", "params": {}})),
            tool("apply", second),
            text("Added two vias."),
        ],
    );
    send(&mut ax, "Add two vias")?;
    ax.until("the Ask card", |ax| ax.has(Role::Button, "Deny"))?;
    // The person adds a via of their own while the card waits.
    let net = ax.app.engine.project().nets.first().map_or(0, |n| n.id);
    let rules = ax.app.engine.project().rules.effective();
    let via = sparksmith_core::layout::via::ViaRequest::through(&rules, net, [3.0, 1.0]);
    let theirs =
        sparksmith_core::Operation::Layout(sparksmith_core::layout::LayoutCommand::AddVia { via });
    ax.app.engine.apply(ax.app.engine.revision(), theirs)?;
    ax.button("Deny")?;
    ax.until("the turn's end", idle)?;
    ax.settle();
    assert_eq!(ax.app.engine.project().vias.len(), vias + 3);
    let summary = ax
        .app
        .assistant
        .conversation
        .items
        .iter()
        .find_map(|i| match i {
            Item::Summary(card) => Some(card.clone()),
            _ => None,
        })
        .ok_or("no summary card")?;
    assert_eq!(summary.changes, ["Vias: 2 added"]);
    assert_eq!(summary.others, ["Vias: 1 added"]);
    assert!(
        ax.texts.iter().any(|t| t == "Changed outside this turn:"),
        "{:?}",
        ax.texts
    );
    // Show selects the assistant's two vias, not the person's.
    assert_eq!(summary.show.len(), 2);
    Ok(())
}

/// The chat's turn stops at its request limit with a card that names the
/// limit, says what was done so far and its cost, and offers Continue,
/// which runs one more turn's worth on the person's click.
#[test]
fn a_turn_at_its_limit_says_which_and_continues_on_the_persons_click() -> Fallible {
    let mut ax = chat(LARGE, vec![])?;
    let start = ax.app.engine.revision();
    let vias = ax.app.engine.project().vias.len();
    let first = add_via(&ax, 1.0);
    let mut second = add_via(&ax, 2.0);
    second["expected_revision"] = json!(start + 1);
    let script = Arc::new(Mutex::new(VecDeque::from(vec![
        tool("apply", first),
        tool("apply", second),
        text("Both vias are in."),
    ])));
    if let Some(model) = &ax.app.assistant.model {
        ax.app.assistant.model = Some(ChatModel {
            profile: model.profile.clone(),
            connect: Arc::new(move || {
                Ok(Box::new(Script(Arc::clone(&script))) as Box<dyn ModelBackend>)
            }),
        });
    }
    let mut limits = sparksmith_assist::settings::Shipped::locate()?.limits;
    limits.requests = 2;
    ax.app.assistant.model_limits = Some(limits);
    ax.app.assistant.model_price = Some(sparksmith_assist::price::Price {
        provider: sparksmith_assist::settings::Wire::Openai,
        base_url: "http://localhost".into(),
        model: "scripted".into(),
        currency: "USD".into(),
        per_million: sparksmith_assist::price::Rates {
            input: 10.0,
            cache_write: 12.5,
            cache_read: 1.0,
            output: 20.0,
        },
        source: "https://example/pricing".into(),
        as_of: "2026-11-03".into(),
    });
    send(&mut ax, "Add two vias")?;
    ax.until("the turn's end", idle)?;
    ax.settle();
    assert_eq!(ax.app.engine.project().vias.len(), vias + 2);
    for line in [
        "Stopped at a limit · 1 undo step",
        "This turn's limit of 2 model requests was reached.",
        "Done so far:",
        "Vias: 2 added",
        // Two requests of 1 000 tokens in and 50 out at 10 and 20 USD a
        // million.
        "This turn: 2000 in, 100 out · about 0.02 USD",
        "This conversation: 2000 in, 100 out · about 0.02 USD",
    ] {
        assert!(ax.texts.iter().any(|t| t == line), "{line}: {:?}", ax.texts);
    }
    let go = ax.find(Role::Button, crate::assistant::CONTINUE)?;
    assert!(!go.disabled);
    // Nothing runs until the person clicks.
    let messages = |ax: &Ax| {
        ax.app
            .assistant
            .conversation
            .items
            .iter()
            .filter(|i| matches!(i, Item::You { .. }))
            .count()
    };
    ax.settle();
    assert_eq!(messages(&ax), 1);
    ax.button(crate::assistant::CONTINUE)?;
    ax.until("the continued turn's end", idle)?;
    ax.settle();
    let conversation = &ax.app.assistant.conversation;
    assert_eq!(conversation.state, State::Idle, "{:?}", conversation.items);
    assert!(matches!(
        conversation.items.iter().rev().find(|i| matches!(i, Item::You { .. })),
        Some(Item::You { text, .. }) if text == crate::assistant::CONTINUE_MESSAGE
    ));
    assert!(
        ax.texts
            .iter()
            .any(|t| t == "This conversation: 3000 in, 150 out · about 0.03 USD"),
        "{:?}",
        ax.texts
    );
    // The first card's Continue is used up.
    let buttons = ax.all(Role::Button, crate::assistant::CONTINUE);
    assert!(buttons.iter().all(|b| b.disabled), "{buttons:?}");
    Ok(())
}

/// A conversation that has used its tokens stops before it sends, says
/// it was the conversation's limit, and Continue raises it.
#[test]
fn a_conversation_at_its_limit_stops_before_sending_and_continue_raises_it() -> Fallible {
    let mut ax = chat(LARGE, vec![text("Here."), text("Here again.")])?;
    let mut limits = sparksmith_assist::settings::Shipped::locate()?.limits;
    limits.conversation_tokens = 1;
    ax.app.assistant.model_limits = Some(limits);
    send(&mut ax, "Hello")?;
    ax.until("the turn's end", idle)?;
    ax.settle();
    assert!(
        ax.texts
            .iter()
            .any(|t| t == "This conversation's limit of 1 tokens was reached."),
        "{:?}",
        ax.texts
    );
    assert!(
        ax.texts
            .iter()
            .any(|t| t == "Done so far: nothing in the design."),
        "{:?}",
        ax.texts
    );
    // No price for this model: tokens only.
    assert!(
        !ax.texts.iter().any(|t| t.contains("USD")),
        "{:?}",
        ax.texts
    );
    ax.button(crate::assistant::CONTINUE)?;
    ax.until("the continued turn's end", idle)?;
    let conversation = &ax.app.assistant.conversation;
    assert_eq!(conversation.state, State::Idle, "{:?}", conversation.items);
    assert_eq!(conversation.raised, limits.total_tokens);
    Ok(())
}

//! The model's question card (`assist.ask`, `docs/assistant-panel.md`
//! §3.4) in the whole app, on a scripted model (no network, no paid
//! calls), driven through the accessibility tree at 1024×640 and
//! 1920×1080: a vague request gets one card of three questions with the
//! recommended answers chosen; the keys answer it (↑↓, Enter goes on and
//! sends after the last) and the answers are the call's reply; typing in
//! the composer answers it in the person's own words; an answered card is a
//! one-line summary; an engineer's specific request goes straight on
//! without a card, with the asking guidance in the prompt the model got.
use super::super::ax_harness::{Ax, Fallible};
use crate::assistant::questions::Answered;
use crate::assistant::worker::ChatModel;
use crate::assistant::{Item, State};
use crate::shell::Workspace;
use egui::accesskit::Role;
use serde_json::{Value, json};
use sparksmith_assist::model::{
    Message, ModelBackend, ModelProfile, ModelReply, ModelRequest, Part, StopReason,
};
use sparksmith_assist::usage::Usage;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const SMALL: egui::Vec2 = egui::vec2(1024.0, 640.0);
const LARGE: egui::Vec2 = egui::vec2(1920.0, 1080.0);

/// What the model was sent: each request's system text and messages.
type Sent = Arc<Mutex<Vec<(String, Vec<Message>)>>>;

/// A model answering from a script, keeping what it was sent.
struct Script {
    replies: Arc<Mutex<VecDeque<ModelReply>>>,
    sent: Sent,
}

impl ModelBackend for Script {
    fn label(&self) -> String {
        "script".into()
    }
    fn send(&mut self, request: &ModelRequest<'_>) -> sparksmith_assist::Result<ModelReply> {
        if let Ok(mut sent) = self.sent.lock() {
            sent.push((request.system.to_owned(), request.messages.to_vec()));
        }
        self.replies
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

/// The model asks three questions on one card.
fn ask() -> ModelReply {
    reply(
        vec![Part::ToolCall {
            id: "ask".into(),
            name: "show".into(),
            input: json!({"name": "assist.ask", "params": {
            "title": "A few things first",
            "why": "These change which parts I pick.",
            "questions": [
                {"id": "power", "title": "Power", "text": "How will it get its power?",
                 "type": "single", "recommended": "usb", "options": [
                    {"value": "usb", "title": "USB", "about": "plugs into a phone charger"},
                    {"value": "battery", "title": "Batteries", "about": "no cable, change them now and then"},
                    {"value": "mains", "title": "Mains", "about": "a wall socket"}]},
                {"id": "place", "title": "Where", "text": "Where will it sit?",
                 "type": "single", "recommended": "indoor", "options": [
                    {"value": "indoor", "title": "Indoors"},
                    {"value": "outdoor", "title": "Outdoors", "about": "rain and sun"}]},
                {"id": "every", "title": "How often", "text": "How often should it report?",
                 "type": "number", "unit": "minutes", "min": 1, "max": 1440, "recommended": 10}
            ]}}),
            malformed: None,
            replay: None,
        }],
        StopReason::ToolCalls,
    )
}

/// The app on the demo board at `window`, its chat on `replies`.
fn chat(window: egui::Vec2, replies: Vec<ModelReply>) -> Fallible<(Ax, Sent)> {
    let mut ax = Ax::new(Workspace::Pcb)?;
    ax.window = window;
    let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
    let sent: Sent = Arc::default();
    let kept = Arc::clone(&sent);
    ax.app.assistant.model = Some(ChatModel {
        profile: ModelProfile::openai_compatible("scripted", 32_768),
        connect: Arc::new(move || {
            Ok(Box::new(Script {
                replies: Arc::clone(&replies),
                sent: Arc::clone(&kept),
            }) as Box<dyn ModelBackend>)
        }),
    });
    ax.settle();
    Ok((ax, sent))
}

/// Open the column, type `message` and press Enter.
fn send(ax: &mut Ax, message: &str) {
    if !ax.app.assistant.shown() {
        ax.key_with(
            egui::Key::A,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        );
    }
    composer(ax);
    ax.type_text(message);
    ax.key(egui::Key::Enter);
}

fn composer(ax: &mut Ax) {
    ax.ctx
        .memory_mut(|m| m.request_focus(crate::assistant::Assistant::composer_id()));
    ax.frame(vec![]);
}

fn idle(ax: &Ax) -> bool {
    !ax.app.assistant.conversation.state.busy() && ax.app.assistant.job.is_none()
}

fn asking(ax: &Ax) -> bool {
    ax.app.assistant.asking.is_some() && ax.app.assistant.conversation.state == State::Waiting
}

/// The question card's answer, as the transcript holds it.
fn answer(ax: &Ax) -> Option<Answered> {
    ax.app
        .assistant
        .conversation
        .items
        .iter()
        .find_map(|item| match item {
            Item::Question(card) => Some(card.answer.clone()),
            _ => None,
        })
        .flatten()
}

/// The reply the model got for its `assist.ask` call.
fn ask_result(sent: &Sent) -> Fallible<Value> {
    let sent = sent.lock().map_err(|e| e.to_string())?;
    let text = sent
        .iter()
        .flat_map(|(_, messages)| messages.iter())
        .flat_map(|m| m.content.iter())
        .find_map(|part| match part {
            Part::ToolResult { call_id, text, .. } if call_id == "ask" => Some(text.clone()),
            _ => None,
        })
        .ok_or("the model got no reply to its question card")?;
    Ok(serde_json::from_str(&text)?)
}

const VAGUE: &str = "I want a little thing that tells my phone how warm my greenhouse is";

#[test]
fn a_vague_request_gets_one_card_of_three_questions_answered_with_the_keys() -> Fallible {
    for size in [SMALL, LARGE] {
        let (mut ax, sent) = chat(size, vec![ask(), text("Got it: batteries, outdoors.")])?;
        send(&mut ax, VAGUE);
        ax.until("the question card", asking)?;
        ax.settle();
        // One card, its three questions, the recommended answers chosen.
        ax.find(Role::Label, "A few things first")?;
        ax.find(Role::Label, "How will it get its power?")?;
        let usb = ax.find(Role::RadioButton, "USB")?;
        let indoors = ax.find(Role::RadioButton, "Indoors")?;
        ax.find(Role::RadioButton, "Batteries")?;
        ax.find(Role::RadioButton, "Outdoors")?;
        ax.find(Role::TextInput, "How often")?;
        let answer_button = ax.find(Role::Button, "Answer")?;
        let skip = ax.find(Role::Button, "Skip")?;
        assert!(usb.rect.right() <= size.x && answer_button.rect.right() <= size.x);
        assert!(
            answer_button.rect.bottom() <= size.y,
            "{:?}",
            answer_button.rect
        );
        assert!(!skip.rect.intersects(answer_button.rect));
        assert!(indoors.rect.top() > usb.rect.top());
        // ↓ Enter: Batteries, on to Where; ↓ Enter: Outdoors, on to How
        // often, whose field takes the keyboard; Enter there sends.
        for key in [
            egui::Key::ArrowDown,
            egui::Key::Enter,
            egui::Key::ArrowDown,
            egui::Key::Enter,
        ] {
            ax.key(key);
        }
        ax.frame(vec![]);
        ax.key(egui::Key::Enter);
        ax.until("the turn's end", idle)?;
        ax.settle();
        let reply = ask_result(&sent)?;
        assert_eq!(
            reply["answers"],
            json!({"power": "battery", "place": "outdoor", "every": 10}),
            "{reply}"
        );
        // The answered card is one summary line.
        let line = "Power: Batteries · Where: Outdoors · How often: 10 minutes";
        assert_eq!(answer(&ax), Some(Answered::Card(line.into())));
        ax.find(Role::Label, line)?;
        assert!(!ax.has(Role::Button, "Answer"));
        assert!(!ax.has(Role::RadioButton, "USB"));
    }
    Ok(())
}

#[test]
fn typing_in_the_composer_answers_the_card_in_the_persons_words() -> Fallible {
    for size in [SMALL, LARGE] {
        let (mut ax, sent) = chat(size, vec![ask(), text("USB and outdoors it is.")])?;
        send(&mut ax, VAGUE);
        ax.until("the question card", asking)?;
        ax.settle();
        // The composer still sends while the card waits.
        composer(&mut ax);
        let said = "USB, it sits outside, every 10 minutes is fine";
        ax.type_text(said);
        ax.key(egui::Key::Enter);
        ax.until("the turn's end", idle)?;
        ax.settle();
        assert_eq!(ask_result(&sent)?["said"], json!(said));
        assert_eq!(answer(&ax), Some(Answered::Said(said.into())));
        ax.find(Role::Label, said)?;
        assert!(ax.app.assistant.composer.is_empty());
        // The words answered the card: they started no second turn.
        let messages = ax
            .app
            .assistant
            .conversation
            .items
            .iter()
            .filter(|i| matches!(i, Item::You { .. }))
            .count();
        assert_eq!(messages, 1);
    }
    Ok(())
}

#[test]
fn esc_skips_the_card_and_leaves_the_choice_to_the_model() -> Fallible {
    let (mut ax, sent) = chat(SMALL, vec![ask(), text("I chose USB power.")])?;
    send(&mut ax, VAGUE);
    ax.until("the question card", asking)?;
    ax.settle();
    ax.key(egui::Key::Escape);
    ax.until("the turn's end", idle)?;
    assert_eq!(ask_result(&sent)?["skipped"], json!(true));
    assert_eq!(answer(&ax), Some(Answered::Skipped));
    Ok(())
}

#[test]
fn an_engineers_specific_request_goes_on_without_a_card() -> Fallible {
    for size in [SMALL, LARGE] {
        let plan = "Plan: R1 10k 0603 pull-up, C1 100n 0402, board 20 × 15 mm. Building it now.";
        let (mut ax, sent) = chat(size, vec![text(plan)])?;
        send(
            &mut ax,
            "Add a 10k 0603 pull-up and a 100n 0402 decoupling cap, board 20x15 mm",
        );
        ax.until("the turn's end", idle)?;
        ax.settle();
        let cards = ax
            .app
            .assistant
            .conversation
            .items
            .iter()
            .filter(|i| matches!(i, Item::Question(_)))
            .count();
        assert_eq!(cards, 0);
        assert!(!ax.has(Role::Button, "Answer"));
        // The model was told when to ask and when not to: the shipped data.
        let sent = sent.lock().map_err(|e| e.to_string())?;
        let (system, _) = sent.first().ok_or("no request")?;
        for words in [
            "## Asking before you design",
            "show assist.ask",
            "do not ask what it gives",
            "Talk the way the person does",
        ] {
            assert!(system.contains(words), "the prompt lacks {words:?}");
        }
    }
    Ok(())
}

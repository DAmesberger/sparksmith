//! The workflow cards driven as the person does: through the accessibility
//! tree at 1024×640 and 1920×1080, in a chat-wide column, with the runner's
//! events and cards fed in as it sends them.
use super::*;
use egui::accesskit::{NodeId, Role, TreeUpdate};
use serde_json::json;
use sparksmith_assist::workflow::control::{
    Action, Count, Field, Finding, Gauge, Item as ResultItem, Row,
};
use sparksmith_assist::workflow::event::StepRow;
use sparksmith_assist::workflow::{HostCatalog, load};
use std::collections::HashMap;

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

const SIZES: [egui::Vec2; 2] = [egui::vec2(1024.0, 640.0), egui::vec2(1920.0, 1080.0)];
/// The chat column's default width.
const COLUMN: f32 = 360.0;

/// The shipped workflows, checked against this program's catalog.
pub(crate) fn shipped() -> Vec<Listed> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/assist/workflows");
    load::list(Some(&folder), None, &HostCatalog)
}

#[derive(Clone, Debug)]
struct Node {
    role: Role,
    name: String,
    rect: egui::Rect,
    disabled: bool,
}

struct Chat {
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    nodes: Vec<Node>,
    cards: WorkflowCards,
    actions: Vec<CardAction>,
    dir: tempfile::TempDir,
}

impl Chat {
    fn new(size: egui::Vec2) -> Fallible<Self> {
        let dir = tempfile::tempdir()?;
        let ctx = egui::Context::default();
        sparksmith_ui::fonts::setup_bundled_fonts(&ctx);
        let cards = WorkflowCards::with(shipped(), Some(dir.path().join("conversation")));
        Ok(Self {
            ctx,
            size,
            time: 0.0,
            nodes: Vec::new(),
            cards,
            actions: Vec::new(),
            dir,
        })
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.25;
        self.ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let column =
            egui::Rect::from_min_max(egui::pos2(self.size.x - COLUMN, 0.0), self.size.to_pos2());
        let (cards, actions) = (&mut self.cards, &mut self.actions);
        let mut out = self.ctx.run_ui(input, |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(column.shrink(8.0)), |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| actions.extend(cards.show(ui)));
            });
        });
        if let Some(update) = out.platform_output.accesskit_update.take() {
            self.nodes = nodes(&update);
        }
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(vec![]);
        }
    }

    fn find(&self, role: Role, name: &str) -> Fallible<Node> {
        let found: Vec<&Node> = self
            .nodes
            .iter()
            .filter(|n| n.role == role && n.name == name)
            .collect();
        match found.as_slice() {
            [one] => Ok((*one).clone()),
            _ => Err(format!("{} {role:?} {name:?} in {:?}", found.len(), self.names()).into()),
        }
    }

    fn has(&self, name: &str) -> bool {
        self.nodes.iter().any(|n| n.name == name)
    }

    fn names(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|n| !n.name.is_empty())
            .map(|n| n.name.clone())
            .collect()
    }

    fn click(&mut self, role: Role, name: &str) -> Fallible {
        let node = self.find(role, name)?;
        if node.disabled {
            return Err(format!("{name} is disabled").into());
        }
        let window = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size);
        if !window.contains_rect(node.rect) {
            return Err(format!(
                "{name} at {:?} is outside the {:?} window",
                node.rect, self.size
            )
            .into());
        }
        let at = node.rect.center();
        self.frame(vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        self.frame(vec![]);
        Ok(())
    }

    fn type_into(&mut self, name: &str, text: &str) -> Fallible {
        self.click(Role::TextInput, name)?;
        self.frame(vec![egui::Event::Text(text.into())]);
        // A field's mark goes once it changes: the cards below move up.
        self.frame(vec![]);
        Ok(())
    }

    fn key(&mut self, key: egui::Key) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        self.frame(vec![]);
    }

    /// Frames until the documents dropped have been read.
    fn read(&mut self) -> Fallible {
        let deadline = std::time::Instant::now() + crate::app::ax_harness::PATIENCE;
        while std::time::Instant::now() < deadline {
            self.frame(vec![]);
            if self.cards.probes.is_empty() {
                self.frame(vec![]);
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err("the documents were not read".into())
    }

    /// A one-page PDF named `name` in the test's folder.
    fn pdf(&self, name: &str) -> Fallible<PathBuf> {
        let path = self.dir.path().join(name);
        std::fs::write(
            &path,
            b"%PDF-1.7\n1 0 obj << /Type /Page >> endobj\n%%EOF\n",
        )?;
        Ok(path)
    }
}

fn nodes(update: &TreeUpdate) -> Vec<Node> {
    let index: HashMap<NodeId, usize> = update
        .nodes
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();
    let own = |node: &egui::accesskit::Node| -> Option<String> {
        node.label()
            .or_else(|| (node.role() == Role::Label).then(|| node.value()).flatten())
            .map(str::to_owned)
    };
    update
        .nodes
        .iter()
        .map(|(_, node)| {
            let name = own(node)
                .or_else(|| {
                    let names: Vec<String> = node
                        .labelled_by()
                        .iter()
                        .filter_map(|id| index.get(id))
                        .filter_map(|i| own(&update.nodes[*i].1))
                        .collect();
                    (!names.is_empty()).then(|| names.join(" "))
                })
                .unwrap_or_default();
            let rect = node.bounds().map_or(egui::Rect::NOTHING, |b| {
                egui::Rect::from_min_max(
                    egui::pos2(b.x0 as f32, b.y0 as f32),
                    egui::pos2(b.x1 as f32, b.y1 as f32),
                )
            });
            Node {
                role: node.role(),
                name,
                rect,
                disabled: node.is_disabled(),
            }
        })
        .collect()
}

fn field(input: &str, title: &str, optional: bool) -> Field {
    Field {
        input: input.into(),
        title: title.into(),
        help: String::new(),
        optional,
    }
}

fn card(chat: &mut Chat, step: &str, card: ControlCard) {
    chat.cards.event(&WorkflowEvent::Card {
        run: "run-1".into(),
        step: step.into(),
        card,
    });
}

fn started(chat: &mut Chat) {
    chat.cards.event(&WorkflowEvent::Started {
        run: "run-1".into(),
        workflow: "datasheet-to-component".into(),
        version: "1.0".into(),
        digest: "sha256:0".into(),
        label: "Datasheet → component: TLV75733PDBVR".into(),
        steps: ["ask", "read", "import", "done"]
            .into_iter()
            .map(|id| StepRow {
                id: id.into(),
                title: format!("Step {id}"),
            })
            .collect(),
    });
}

/// Dropping a PDF offers the datasheet workflow on one compact card: the
/// file filled in, read for its page count and copied into the
/// conversation's folder, an inline field only for the part name nothing
/// gave, the optional notes left out. Accept and continue starts the run
/// with the copy. Any model, no words.
#[test]
fn a_dropped_pdf_offers_the_datasheet_workflow() -> Fallible {
    for size in SIZES {
        let mut chat = Chat::new(size)?;
        let pdf = chat.pdf("tlv757p.pdf")?;
        let refused = chat.cards.drop_files(vec![pdf.clone()]);
        assert!(refused.is_empty(), "{refused:?}");
        assert!(chat.cards.open_requested);
        chat.read()?;
        chat.settle();
        assert!(chat.has("Datasheet → component"), "{:?}", chat.names());
        assert!(
            chat.has("tlv757p.pdf · 1 page · 1 KB ⌄"),
            "{:?}",
            chat.names()
        );
        chat.find(Role::Button, "Change…")?;
        chat.find(Role::TextInput, "Part name")?;
        assert!(
            chat.find(Role::TextInput, "Notes").is_err(),
            "optional and empty"
        );
        chat.click(Role::Button, "Accept and continue")?;
        assert!(chat.actions.is_empty(), "the part name is required");
        assert!(chat.has("required"), "{:?}", chat.names());
        chat.type_into("Part name", "TLV75733PDBVR")?;
        chat.click(Role::Button, "Accept and continue")?;
        let [
            CardAction::Start {
                workflow, inputs, ..
            },
        ] = chat.actions.as_slice()
        else {
            return Err(format!("{:?}", chat.actions).into());
        };
        assert_eq!(workflow, "datasheet-to-component");
        let copy = chat.dir.path().join("conversation").join("tlv757p.pdf");
        assert_eq!(
            Value::Object(inputs.clone()),
            json!({"datasheets": [copy.display().to_string()], "part": "TLV75733PDBVR"})
        );
        assert!(copy.exists());
        // The card stays, answered with what it gave.
        chat.settle();
        assert!(chat.find(Role::Button, "Accept and continue").is_err());
        assert!(
            chat.has("Datasheets: tlv757p.pdf · 1 page · 1 KB · Part name: TLV75733PDBVR"),
            "{:?}",
            chat.names()
        );
    }
    Ok(())
}

/// A message's files fill the files input of a workflow offered next,
/// and the model's offer fills the waiting card instead of adding one:
/// one card with both filled, Enter accepts.
#[test]
fn the_conversation_fills_one_card() -> Fallible {
    let mut chat = Chat::new(SIZES[1])?;
    let pdf = chat.pdf("tlv757p.pdf")?;
    chat.cards.attach(std::slice::from_ref(&pdf));
    chat.read()?;
    let card = chat.cards.offer("datasheet-to-component", Map::new(), "")?;
    let mut part = Map::new();
    part.insert("part".into(), json!("TLV75733PDBVR"));
    let again = chat
        .cards
        .offer("datasheet-to-component", part, "You named the part.")?;
    assert_eq!(card, again);
    assert_eq!(chat.cards.offers(), 1);
    chat.settle();
    assert!(chat.has("You named the part."), "{:?}", chat.names());
    chat.key(egui::Key::Enter);
    let [CardAction::Start { inputs, .. }] = chat.actions.as_slice() else {
        return Err(format!("{:?}", chat.actions).into());
    };
    let copy = chat.dir.path().join("conversation").join("tlv757p.pdf");
    assert_eq!(inputs["datasheets"], json!([copy.display().to_string()]));
    assert_eq!(inputs["part"], "TLV75733PDBVR");
    Ok(())
}

/// A file chosen with Change… that the conversation read before shows at
/// once, not "Reading…" (run 12, finding 9).
#[test]
fn a_file_read_before_is_ready_when_chosen_again() -> Fallible {
    let mut chat = Chat::new(SIZES[0])?;
    let pdf = chat.pdf("tlv757p.pdf")?;
    chat.cards.attach(std::slice::from_ref(&pdf));
    chat.read()?;
    let card = chat.cards.offer("datasheet-to-component", Map::new(), "")?;
    // The attached file filled it already; a pick of the same file keeps it.
    chat.cards.picked(&card, vec![pdf]);
    chat.settle();
    assert!(
        chat.has("tlv757p.pdf · 1 page · 1 KB ⌄"),
        "{:?}",
        chat.names()
    );
    assert!(!chat.names().iter().any(|n| n.contains("Reading…")));
    Ok(())
}

/// A file no workflow takes is said, by name; nothing is offered.
#[test]
fn a_file_no_workflow_takes_is_named() -> Fallible {
    let mut chat = Chat::new(SIZES[0])?;
    let notes = chat.dir.path().join("notes.txt");
    std::fs::write(&notes, "hello")?;
    assert_eq!(
        chat.cards.drop_files(vec![notes]),
        ["No workflow takes notes.txt"]
    );
    assert!(chat.cards.items.is_empty());
    Ok(())
}

/// Just chat answers the card and is reported for the model; Esc does the
/// same on the card the conversation waits on.
#[test]
fn just_chat_declines_the_offer() -> Fallible {
    let mut chat = Chat::new(SIZES[1])?;
    let card = chat.cards.offer(
        "datasheet-to-component",
        Map::new(),
        "You asked for a part.",
    )?;
    chat.settle();
    assert!(chat.has("You asked for a part."));
    chat.key(egui::Key::Escape);
    assert_eq!(chat.actions, [CardAction::JustChat { card: card.clone() }]);
    let Some(WorkflowEvent::Offered { by, missing, .. }) =
        chat.cards.offered_event(&card, OfferedBy::Model)
    else {
        return Err("no offered event".into());
    };
    assert_eq!(by, OfferedBy::Model);
    assert_eq!(missing, ["datasheets", "part"]);
    assert!(chat.cards.offer("make-coffee", Map::new(), "").is_err());
    Ok(())
}

/// An input step's cards are one compact confirmation: the person types,
/// Accept and continue answers with the values as the inputs take them; a
/// required field left empty is marked and nothing is answered.
#[test]
fn an_input_card_answers_with_typed_values() -> Fallible {
    for size in SIZES {
        let mut chat = Chat::new(size)?;
        started(&mut chat);
        card(
            &mut chat,
            "ask",
            ControlCard::Text {
                field: field("part", "Part name", false),
                multiline: false,
                max_bytes: 64,
                value: None,
            },
        );
        card(
            &mut chat,
            "ask",
            ControlCard::Number {
                field: field("pitch", "Pitch", false),
                quantity: Quantity::Length,
                min: None,
                max: Some(2_540_000.into()),
                value: None,
            },
        );
        card(
            &mut chat,
            "ask",
            ControlCard::PageRanges {
                field: field("pages", "Pages to read first", true),
                pages: Some(38),
                value: Some("3, 31-39".into()),
            },
        );
        chat.settle();
        chat.click(Role::Button, "Accept and continue")?;
        assert!(chat.actions.is_empty(), "{:?}", chat.actions);
        assert!(chat.has("required"), "{:?}", chat.names());
        chat.type_into("Part name", "TLV75733PDBVR")?;
        chat.type_into("Pitch", "10mil")?;
        chat.click(Role::Button, "Accept and continue")?;
        assert!(
            chat.actions.is_empty(),
            "page 39 is past 38: {:?}",
            chat.actions
        );
        assert!(
            chat.has("page 39 is past the last, 38"),
            "{:?}",
            chat.names()
        );
        chat.click(Role::TextInput, "Pages to read first")?;
        chat.key(egui::Key::End);
        for _ in 0.."3, 31-39".len() {
            chat.key(egui::Key::Backspace);
        }
        chat.frame(vec![egui::Event::Text("3, 31-32".into())]);
        chat.frame(vec![]);
        chat.click(Role::Button, "Accept and continue")?;
        let [
            CardAction::Answer {
                run,
                step,
                answer: ControlAnswer::Inputs { values },
            },
        ] = chat.actions.as_slice()
        else {
            let texts: Vec<(String, Option<String>)> = chat
                .cards
                .items
                .iter()
                .flat_map(|i| match i {
                    Item::Inputs { fields, .. } => fields
                        .iter()
                        .map(|f| (f.text.clone(), f.error.clone()))
                        .collect(),
                    _ => Vec::new(),
                })
                .collect();
            return Err(format!("{:?} {texts:?}", chat.actions).into());
        };
        assert_eq!((run.as_str(), step.as_str()), ("run-1", "ask"));
        assert_eq!(
            Value::Object(values.clone()),
            json!({"part": "TLV75733PDBVR", "pitch": 254000, "pages": "3, 31-32"})
        );
    }
    Ok(())
}

/// A datasheet value takes dropped and picked files while it waits:
/// Choose… asks the host's picker, each file is read for its pages.
#[test]
fn a_datasheet_field_takes_dropped_and_picked_files() -> Fallible {
    let mut chat = Chat::new(SIZES[0])?;
    started(&mut chat);
    card(
        &mut chat,
        "ask",
        ControlCard::Datasheet {
            field: field("datasheets", "Datasheets", false),
            accept: vec!["application/pdf".into()],
            min: 1,
            max: 2,
            files: Vec::new(),
        },
    );
    chat.settle();
    chat.click(Role::Button, "Choose…")?;
    let [CardAction::PickFiles { card }] = chat.actions.as_slice() else {
        return Err(format!("{:?}", chat.actions).into());
    };
    let card = card.clone();
    chat.actions.clear();
    let (a, b) = (chat.pdf("a.pdf")?, chat.pdf("b.pdf")?);
    chat.cards.picked(&card, vec![a]);
    assert!(chat.cards.drop_files(vec![b]).is_empty());
    chat.read()?;
    assert!(chat.has("a.pdf, b.pdf ⌄"), "{:?}", chat.names());
    chat.find(Role::Button, "Change…")?;
    chat.click(Role::Button, "Accept and continue")?;
    let [
        CardAction::Answer {
            answer: ControlAnswer::Inputs { values },
            ..
        },
    ] = chat.actions.as_slice()
    else {
        return Err(format!("{:?}", chat.actions).into());
    };
    let copies: Vec<String> = ["a.pdf", "b.pdf"]
        .iter()
        .map(|n| {
            chat.dir
                .path()
                .join("conversation")
                .join(n)
                .display()
                .to_string()
        })
        .collect();
    assert_eq!(values["datasheets"], json!(copies));
    Ok(())
}

/// The run block follows the run: steps done and running, the meter of
/// requests, tokens and minutes, Stop; how it ended once it has.
#[test]
fn the_run_block_follows_the_run_and_stops_it() -> Fallible {
    for size in SIZES {
        let mut chat = Chat::new(size)?;
        started(&mut chat);
        chat.cards.event(&WorkflowEvent::Step {
            run: "run-1".into(),
            step: "ask".into(),
            done: 0,
            total: 4,
        });
        chat.cards.event(&WorkflowEvent::StepDone {
            run: "run-1".into(),
            step: "ask".into(),
            summary: "tlv757p.pdf".into(),
        });
        chat.cards.event(&WorkflowEvent::Step {
            run: "run-1".into(),
            step: "read".into(),
            done: 1,
            total: 4,
        });
        chat.cards.event(&WorkflowEvent::Meter {
            run: "run-1".into(),
            requests: Gauge {
                used: 14,
                limit: Some(40),
            },
            output_tokens: Gauge {
                used: 61_000,
                limit: Some(200_000),
            },
            minutes: Gauge {
                used: 6,
                limit: Some(45),
            },
        });
        chat.settle();
        assert!(
            chat.has("14 of 40 requests · 61 k of 200 k tokens · 6 of 45 min"),
            "{:?}",
            chat.names()
        );
        assert!(chat.has("Step read · 1 of 4"), "{:?}", chat.names());
        chat.click(Role::Button, "Stop")?;
        assert_eq!(
            chat.actions,
            [CardAction::Stop {
                run: "run-1".into()
            }]
        );
        chat.cards.event(&WorkflowEvent::Ended {
            run: "run-1".into(),
            end: RunEnd::Stopped,
        });
        chat.settle();
        assert!(chat.find(Role::Button, "Stop").is_err());
        assert!(chat.has("Stopped; what completed stays as one undo step"));
    }
    Ok(())
}

/// A confirm gate's impact card: Apply with rows left out answers the run;
/// the result card's Open in editor is its `show` call, Undo run the run's.
#[test]
fn impact_and_result_cards_answer_the_run() -> Fallible {
    for size in SIZES {
        let mut chat = Chat::new(size)?;
        started(&mut chat);
        card(
            &mut chat,
            "import",
            ControlCard::Impact {
                title: "New part: 5 pins, SOT-23-5".into(),
                counts: vec![Count {
                    label: "pins".into(),
                    n: 5,
                }],
                findings: vec![Finding {
                    blocking: false,
                    message: "2 pins without a function".into(),
                }],
                rows: vec![Row {
                    id: "4".into(),
                    text: "Pin 4 NC".into(),
                }],
            },
        );
        chat.settle();
        // The card says what the import makes, as the run titles it.
        assert!(chat.has("New part: 5 pins, SOT-23-5"), "{:?}", chat.names());
        chat.click(Role::Button, "Leave out rows…")?;
        chat.click(Role::CheckBox, "Pin 4 NC")?;
        chat.click(Role::Button, "Apply")?;
        assert_eq!(
            chat.actions,
            [CardAction::Answer {
                run: "run-1".into(),
                step: "import".into(),
                answer: ControlAnswer::Apply {
                    leave_out: vec!["4".into()]
                }
            }]
        );
        chat.actions.clear();
        let open = json!({"workspace": "component", "definition": 7});
        card(
            &mut chat,
            "done",
            ControlCard::Result {
                title: "Done · 1 undo step".into(),
                items: vec![
                    ResultItem {
                        title: "Drafts".into(),
                        value: json!(["component", "symbol"]),
                    },
                    ResultItem {
                        title: "Review".into(),
                        value: json!("# Review of TLV75733PDBVR\n\n- pitch 0.95 mm, page 31"),
                    },
                ],
                findings: Vec::new(),
                actions: vec![Action {
                    title: "Open in editor".into(),
                    call: "view.open".into(),
                    params: open.clone(),
                }],
                undo: true,
            },
        );
        chat.settle();
        assert!(chat.has("component, symbol"), "{:?}", chat.names());
        assert!(chat.has("Review of TLV75733PDBVR"), "{:?}", chat.names());
        chat.click(Role::Button, "Show review")?;
        assert!(
            chat.names()
                .iter()
                .any(|n| n.contains("pitch 0.95 mm, page 31")),
            "{:?}",
            chat.names()
        );
        chat.click(Role::Button, "Open in editor")?;
        chat.click(Role::Button, "Undo run")?;
        assert_eq!(
            chat.actions,
            [
                CardAction::Show {
                    call: "view.open".into(),
                    params: open
                },
                CardAction::Answer {
                    run: "run-1".into(),
                    step: String::new(),
                    answer: ControlAnswer::UndoRun
                },
            ]
        );
        assert!(
            chat.find(Role::Button, "Undo run").is_err(),
            "Undo run is offered once"
        );
        assert!(
            chat.find(Role::Button, "Open in editor").is_err(),
            "what the run made is gone"
        );
    }
    Ok(())
}

/// Continue on a notify card keeps the model's choice: the run goes on.
#[test]
fn a_notify_card_continues_with_the_models_choice() -> Fallible {
    for size in SIZES {
        let mut chat = Chat::new(size)?;
        started(&mut chat);
        card(
            &mut chat,
            "pick",
            ControlCard::Choice {
                field: field("pick", "Generator", false),
                multiple: false,
                options: vec![sparksmith_assist::workflow::control::Choice {
                    value: "sot-23".into(),
                    title: "SOT-23".into(),
                }],
                value: vec!["sot-23".into()],
            },
        );
        chat.settle();
        chat.click(Role::Button, "Continue")?;
        assert_eq!(
            chat.actions,
            [CardAction::Answer {
                run: "run-1".into(),
                step: "pick".into(),
                answer: ControlAnswer::Change {
                    value: json!("sot-23")
                }
            }]
        );
        chat.settle();
        assert!(chat.find(Role::Button, "Continue").is_err());
        assert!(chat.has("Kept"), "{:?}", chat.names());
    }
    Ok(())
}

/// A notify card shows the model's choice; Change answers with another.
#[test]
fn a_notify_card_changes_the_choice() -> Fallible {
    let mut chat = Chat::new(SIZES[0])?;
    started(&mut chat);
    let mut why = field("pick", "Generator", false);
    why.help = "SOT-23-5, TI lands".into();
    card(
        &mut chat,
        "pick",
        ControlCard::Choice {
            field: why,
            multiple: false,
            options: vec![
                sparksmith_assist::workflow::control::Choice {
                    value: "sot-23".into(),
                    title: "SOT-23".into(),
                },
                sparksmith_assist::workflow::control::Choice {
                    value: "qfn".into(),
                    title: "QFN".into(),
                },
            ],
            value: vec!["sot-23".into()],
        },
    );
    chat.settle();
    // The choice first, on one line; the others once asked for.
    assert!(
        chat.has("SOT-23 · SOT-23-5, TI lands"),
        "{:?}",
        chat.names()
    );
    assert!(chat.find(Role::CheckBox, "QFN").is_err());
    chat.click(Role::Button, "Choose another…")?;
    assert!(chat.find(Role::Button, "Change")?.disabled);
    chat.click(Role::CheckBox, "QFN")?;
    chat.click(Role::Button, "Change")?;
    assert_eq!(
        chat.actions,
        [CardAction::Answer {
            run: "run-1".into(),
            step: "pick".into(),
            answer: ControlAnswer::Change {
                value: json!("qfn")
            }
        }]
    );
    Ok(())
}

#[test]
fn numbers_read_as_the_command_wire_does() -> Fallible {
    assert_eq!(number_value(Quantity::Length, "0.25")?, json!(250_000));
    assert_eq!(number_value(Quantity::Length, "0.25mm")?, json!(250_000));
    assert_eq!(number_value(Quantity::Length, "10mil")?, json!(254_000));
    assert!(number_value(Quantity::Length, "ten").is_err());
    assert_eq!(number_value(Quantity::Angle, "45")?, json!(45_000));
    assert_eq!(number_value(Quantity::Angle, "22.5°")?, json!(22_500));
    assert_eq!(number_value(Quantity::Integer, "3")?, json!(3));
    assert!(number_value(Quantity::Integer, "3.5").is_err());
    assert_eq!(number_value(Quantity::Number, "1.5")?, json!(1.5));
    assert_eq!(number_text(Quantity::Length, &250_000.into()), "0.25");
    assert_eq!(number_text(Quantity::Angle, &22_500.into()), "22.5");
    Ok(())
}

/// The palette and the `/` menu list workflows from their data, by words
/// of the title, summary or intents.
#[test]
fn palette_and_slash_rows_come_from_the_workflows() {
    let listed = shipped();
    assert_eq!(
        palette_rows(&listed, "datasheet").len(),
        2,
        "both workflows"
    );
    let rows = palette_rows(&listed, "component");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].title, "Assistant: Datasheet → component");
    assert_eq!(
        rows[0].id,
        format!("{PALETTE_PREFIX}datasheet-to-component")
    );
    assert!(rows[0].enabled);
    assert_eq!(
        palette_rows(&listed, "assistant: chip").len(),
        1,
        "an intent's words"
    );
    assert!(palette_rows(&listed, "route the usb pair").is_empty());
    assert_eq!(menu_rows(&listed, "").len(), listed.len());
}

/// The `/` menu: Enter on the highlighted workflow shows its card.
#[test]
fn the_slash_menu_shows_the_chosen_workflows_card() -> Fallible {
    let ctx = egui::Context::default();
    let mut cards = WorkflowCards::with(shipped(), None);
    let mut chosen = None;
    for pressed in [true, false] {
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            if let Some(id) = cards.slash_menu(ui, "component", true) {
                chosen = Some(id);
            }
        });
    }
    assert_eq!(chosen.as_deref(), Some("datasheet-to-component"));
    assert_eq!(cards.items.len(), 1);
    Ok(())
}

/// A workspace signal offers only the workflows that name it, once per
/// change; the shipped workflow names none.
#[test]
fn a_workspace_signal_offers_what_names_it() -> Fallible {
    let mut chat = Chat::new(SIZES[0])?;
    assert!(chat.cards.workspace("pcb").is_empty());
    assert!(chat.cards.workspace("component").is_empty());
    let folder = chat.dir.path().join("workflows");
    let ours = folder.join("datasheet-to-component");
    std::fs::create_dir_all(&ours)?;
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../library/assist/workflows/datasheet-to-component");
    let text = std::fs::read_to_string(shipped.join("workflow.json"))?.replace(
        r#""offer_when": [{"attachment": "application/pdf"}]"#,
        r#""offer_when": [{"attachment": "application/pdf"}, {"workspace": "component"}]"#,
    );
    std::fs::write(ours.join("workflow.json"), text)?;
    std::fs::copy(shipped.join("method.md"), ours.join("method.md"))?;
    let mut cards = WorkflowCards::with(load::list(Some(&folder), None, &HostCatalog), None);
    assert!(cards.workspace("pcb").is_empty());
    assert_eq!(cards.workspace("component").len(), 1);
    assert!(
        cards.workspace("component").is_empty(),
        "no change, no offer"
    );
    cards.workspace("pcb");
    assert!(cards.workspace("component").is_empty(), "one waits already");
    Ok(())
}

/// Folder searches load off-thread, discard another project's choices, and
/// feed the existing datasheet picker without changing its attachment behavior.
#[test]
fn a_datasheet_picker_loads_the_current_project_folder_and_selects_its_file() -> Fallible {
    for size in SIZES {
        let mut chat = Chat::new(size)?;
        let first = chat.dir.path().join("first");
        let second = chat.dir.path().join("second");
        std::fs::create_dir(&first)?;
        std::fs::create_dir(&second)?;
        let alpha = chat.pdf("alpha.pdf")?;
        let beta = chat.pdf("beta.pdf")?;
        std::fs::rename(alpha, first.join("alpha.pdf"))?;
        std::fs::rename(beta, second.join("beta.pdf"))?;
        std::fs::write(second.join("not-a-datasheet.txt"), b"original text")?;
        chat.cards.set_project(Some(first.clone()));
        started(&mut chat);
        card(
            &mut chat,
            "ask",
            ControlCard::Datasheet {
                field: field("datasheets", "Datasheets", false),
                accept: vec!["application/pdf".into()],
                min: 1,
                max: 1,
                files: Vec::new(),
            },
        );
        let wait_for_choices = |chat: &mut Chat| -> Fallible {
            let deadline = std::time::Instant::now() + crate::app::ax_harness::PATIENCE;
            while std::time::Instant::now() < deadline {
                chat.frame(vec![]);
                if chat.has("None ⌄") {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(format!("project choices did not load: {:?}", chat.names()).into())
        };
        chat.frame(vec![]);
        assert!(chat.has("Searching project files…"));
        let loading_field = chat.find(Role::Button, "Choose…")?.rect;
        wait_for_choices(&mut chat)?;
        let ready_field = chat.find(Role::Button, "Choose…")?.rect;
        assert!(
            (loading_field.top() - ready_field.top()).abs() <= 1.0,
            "the file field moved from {loading_field:?} to {ready_field:?}"
        );
        chat.click(Role::Button, "None ⌄")?;
        chat.find(Role::CheckBox, &format!("alpha.pdf · {}", first.display()))?;
        chat.click(Role::Button, "None ⌄")?;
        chat.cards.set_project(Some(second.clone()));
        wait_for_choices(&mut chat)?;
        chat.click(Role::Button, "None ⌄")?;
        assert!(
            !chat
                .names()
                .iter()
                .any(|name| name.starts_with("alpha.pdf ·"))
        );
        assert!(
            !chat
                .names()
                .iter()
                .any(|name| name.starts_with("not-a-datasheet"))
        );
        chat.click(Role::CheckBox, &format!("beta.pdf · {}", second.display()))?;
        chat.read()?;
        chat.click(Role::Button, "Accept and continue")?;
        let [
            CardAction::Answer {
                answer: ControlAnswer::Inputs { values },
                ..
            },
        ] = chat.actions.as_slice()
        else {
            return Err(format!("{:?}", chat.actions).into());
        };
        assert_eq!(
            values["datasheets"],
            json!([chat
                .dir
                .path()
                .join("conversation")
                .join("beta.pdf")
                .display()
                .to_string()])
        );
    }
    Ok(())
}

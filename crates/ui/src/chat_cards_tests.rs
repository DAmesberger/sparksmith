//! The cards drawn headless and driven through the accessibility tree, as a screen reader or the
//! VM driver finds them: at the smallest supported window (1024×640) and a large one
//! (1920×1080), in a chat column at the window's right edge.
use super::*;
use egui::accesskit::{NodeId, Role, TreeUpdate};
use std::cell::Cell;
use std::collections::HashMap;

/// The window sizes every card is checked at.
const SIZES: [egui::Vec2; 2] = [egui::vec2(1024.0, 640.0), egui::vec2(1920.0, 1080.0)];
/// The chat column's width (the narrowest it docks at, then a wide one).
const COLUMNS: [f32; 2] = [300.0, 560.0];

#[derive(Clone, Debug)]
struct Node {
    role: Role,
    name: String,
    rect: egui::Rect,
    disabled: bool,
}

struct Harness {
    ctx: egui::Context,
    size: egui::Vec2,
    column: f32,
    time: f64,
    nodes: Vec<Node>,
}

impl Harness {
    fn new(size: egui::Vec2, column: f32) -> Self {
        let ctx = egui::Context::default();
        crate::fonts::setup_bundled_fonts(&ctx);
        Self { ctx, size, column, time: 0.0, nodes: Vec::new() }
    }

    fn frame(&mut self, events: Vec<egui::Event>, draw: &mut dyn FnMut(&mut Ui)) {
        self.time += 0.25;
        self.ctx.enable_accesskit();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let column = egui::Rect::from_min_max(egui::pos2(self.size.x - self.column, 0.0), egui::pos2(self.size.x, self.size.y));
        let mut out = self.ctx.run_ui(input, |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(column.shrink(8.0)), |ui| draw(ui));
        });
        if let Some(update) = out.platform_output.accesskit_update.take() {
            self.nodes = nodes(&update);
        }
    }

    fn settle(&mut self, draw: &mut dyn FnMut(&mut Ui)) {
        for _ in 0..3 {
            self.frame(vec![], draw);
        }
    }

    fn find(&self, role: Role, name: &str) -> Node {
        let found: Vec<&Node> = self.nodes.iter().filter(|n| n.role == role && n.name == name).collect();
        assert_eq!(found.len(), 1, "one {role:?} {name:?} in {:#?}", self.names());
        found[0].clone()
    }

    fn names(&self) -> Vec<(Role, String)> {
        self.nodes.iter().filter(|n| !n.name.is_empty()).map(|n| (n.role, n.name.clone())).collect()
    }

    fn click(&mut self, role: Role, name: &str, draw: &mut dyn FnMut(&mut Ui)) {
        let node = self.find(role, name);
        assert!(!node.disabled, "{name} is disabled");
        let at = node.rect.center();
        self.frame(vec![egui::Event::PointerMoved(at)], draw);
        for pressed in [true, false] {
            self.frame(
                vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }],
                draw,
            );
        }
    }

    fn key(&mut self, key: egui::Key, draw: &mut dyn FnMut(&mut Ui)) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }], draw);
        }
    }

    /// Every named button lies inside the window and the column, and no two overlap.
    fn buttons_fit(&self) {
        let window = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size);
        let left = self.size.x - self.column;
        let buttons: Vec<&Node> = self.nodes.iter().filter(|n| n.role == Role::Button && !n.name.is_empty()).collect();
        assert!(!buttons.is_empty(), "no buttons in {:#?}", self.names());
        for b in &buttons {
            assert!(window.contains_rect(b.rect), "{} at {:?} leaves the {:?} window", b.name, b.rect, self.size);
            assert!(b.rect.left() >= left - 0.5, "{} at {:?} leaves the column at {left}", b.name, b.rect);
        }
        for (i, a) in buttons.iter().enumerate() {
            for b in &buttons[i + 1..] {
                assert!(!a.rect.shrink(0.5).intersects(b.rect.shrink(0.5)), "{} and {} overlap", a.name, b.name);
            }
        }
    }
}

fn nodes(update: &TreeUpdate) -> Vec<Node> {
    let index: HashMap<NodeId, usize> = update.nodes.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
    let own = |node: &egui::accesskit::Node| -> Option<String> {
        node.label().or_else(|| (node.role() == Role::Label).then(|| node.value()).flatten()).map(str::to_owned)
    };
    update
        .nodes
        .iter()
        .map(|(_, node)| {
            let name = own(node)
                .or_else(|| {
                    let names: Vec<String> =
                        node.labelled_by().iter().filter_map(|id| index.get(id)).filter_map(|i| own(&update.nodes[*i].1)).collect();
                    (!names.is_empty()).then(|| names.join(" "))
                })
                .unwrap_or_default();
            let rect = node.bounds().map_or(egui::Rect::NOTHING, |b| {
                egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))
            });
            Node { role: node.role(), name, rect, disabled: node.is_disabled() }
        })
        .collect()
}

fn every_size(mut check: impl FnMut(&mut Harness)) {
    for size in SIZES {
        for column in COLUMNS {
            check(&mut Harness::new(size, column));
        }
    }
}

const OPTIONS: [FileOption<'static>; 2] = [
    FileOption { title: "tlv757p.pdf", detail: "38 pages · 1.2 MB", chosen: true },
    FileOption { title: "tlv757p-errata.pdf", detail: "Attached", chosen: false },
];

/// The compact confirmation: one line per input (a file with its ⌄ list and Change…, a part
/// name to edit in place), Accept and continue, and the decline its caller names.
fn draw_confirm<'a>(
    part: &'a mut String,
    chosen: &'a Cell<Option<ConfirmChoice>>,
    file: &'a Cell<Option<FileChoice>>,
    active: bool,
) -> impl FnMut(&mut Ui) + 'a {
    move |ui: &mut Ui| {
        let view = Confirm { title: "Datasheet → component", version: "1.1", why: "You asked for a part.", decline: "Just chat" };
        let choice = confirm(ui, &view, active, |ui| {
            if let Some(e) = compact_row(ui, "Datasheets", None, |ui| file_value(ui, egui::Id::unique("files"), "tlv757p.pdf · 38 pages", &OPTIONS)) {
                file.set(Some(e));
            }
            let response = compact_row(ui, "Part name", None, |ui| text_input(ui, egui::Id::unique("part"), part, false, 64, false));
            entered(ui, &response)
        });
        if let Some(c) = choice {
            chosen.set(Some(c));
        }
    }
}

#[test]
fn the_compact_confirmation_changes_values_in_place_and_accepts() {
    every_size(|h| {
        let mut part = "TLV75733PDBVR".to_owned();
        let (chosen, file) = (Cell::new(None), Cell::new(None));
        let mut draw = draw_confirm(&mut part, &chosen, &file, true);
        h.settle(&mut draw);
        h.buttons_fit();
        h.find(Role::TextInput, "Part name");
        assert!(h.nodes.iter().any(|n| n.name == "You asked for a part."), "{:#?}", h.names());
        h.click(Role::Button, "Change…", &mut draw);
        assert_eq!(file.take(), Some(FileChoice::Change));
        h.click(Role::Button, "tlv757p.pdf · 38 pages ⌄", &mut draw);
        h.settle(&mut draw);
        h.click(Role::CheckBox, "tlv757p-errata.pdf · Attached", &mut draw);
        assert_eq!(file.take(), Some(FileChoice::Toggle(1)));
        h.click(Role::Button, "Accept and continue", &mut draw);
        assert_eq!(chosen.take(), Some(ConfirmChoice::Accept));
        h.click(Role::Button, "Just chat", &mut draw);
        assert_eq!(chosen.take(), Some(ConfirmChoice::Decline));
    });
}

#[test]
fn enter_accepts_an_active_confirmation_and_esc_declines_it() {
    let mut h = Harness::new(SIZES[0], COLUMNS[0]);
    let mut part = "TLV75733PDBVR".to_owned();
    let (chosen, file) = (Cell::new(None), Cell::new(None));
    let mut draw = draw_confirm(&mut part, &chosen, &file, true);
    h.settle(&mut draw);
    h.key(egui::Key::Enter, &mut draw);
    assert_eq!(chosen.take(), Some(ConfirmChoice::Accept));
    h.key(egui::Key::Escape, &mut draw);
    assert_eq!(chosen.take(), Some(ConfirmChoice::Decline));
    // Enter in the part's field accepts too.
    h.click(Role::TextInput, "Part name", &mut draw);
    h.key(egui::Key::Enter, &mut draw);
    assert_eq!(chosen.take(), Some(ConfirmChoice::Accept));
    // An inactive card reads no keys: only the card the conversation waits on does.
    drop(draw);
    let mut draw = draw_confirm(&mut part, &chosen, &file, false);
    h.settle(&mut draw);
    h.key(egui::Key::Enter, &mut draw);
    assert_eq!(chosen.take(), None);
}

#[test]
fn a_file_chip_is_named_and_removed() {
    every_size(|h| {
        let removed = Cell::new(false);
        let mut draw = |ui: &mut Ui| {
            if file_chip(ui, "tlv757p.pdf", true) {
                removed.set(true);
            }
        };
        h.settle(&mut draw);
        assert!(h.nodes.iter().any(|n| n.name == "@tlv757p.pdf"), "{:#?}", h.names());
        h.click(Role::Button, "Remove tlv757p.pdf", &mut draw);
        assert!(removed.get());
    });
}

#[test]
fn a_menu_moves_with_the_keys_it_is_given() {
    let rows = [
        MenuRow { title: "tlv757p.pdf", summary: "Attached", enabled: true },
        MenuRow { title: "tlv757p-errata.pdf", summary: "datasheets", enabled: true },
    ];
    let mut highlighted = 0;
    let mut h = Harness::new(SIZES[1], COLUMNS[1]);
    let mut chosen = Vec::new();
    let mut nav = MenuNav { down: true, ..MenuNav::default() };
    let mut draw = |ui: &mut Ui| {
        if let Some(i) = menu(ui, &rows, &mut highlighted, Some(nav)) {
            chosen.push(i);
        }
        nav = MenuNav { enter: true, ..MenuNav::default() };
    };
    h.frame(vec![], &mut draw);
    h.frame(vec![], &mut draw);
    assert_eq!(chosen, vec![1], "↓ then Enter chooses the second row");
}

#[test]
fn impact_review_applies_leaving_rows_out_and_a_blocking_finding_stops_apply() {
    let counts = [CountRow { label: "pins", n: 5 }, CountRow { label: "functions", n: 5 }];
    let rows = [ChangeRow { id: "pin-1", text: "Pin 1 IN" }, ChangeRow { id: "pin-2", text: "Pin 2 GND" }];
    every_size(|h| {
        let mut leave_out = Vec::new();
        let mut chosen = None;
        let mut draw = |ui: &mut Ui| {
            let view = Impact { title: "Import pinout into the drafts?", gate: Gate::Review, counts: &counts, findings: &[], rows: &rows };
            if let Some(c) = impact(ui, egui::Id::unique("impact"), &view, &mut leave_out, true) {
                chosen = Some(c);
            }
        };
        h.settle(&mut draw);
        h.buttons_fit();
        h.click(Role::Button, "Leave out rows…", &mut draw);
        h.click(Role::CheckBox, "Pin 2 GND", &mut draw);
        h.click(Role::Button, "Apply", &mut draw);
        assert_eq!(chosen, Some(GateChoice::Apply));
        assert_eq!(leave_out, vec!["pin-2".to_owned()]);
    });
    let blocking = [FindingRow { blocking: true, message: "Pin 3 has no function" }];
    let mut h = Harness::new(SIZES[0], COLUMNS[0]);
    let chosen = Cell::new(None);
    let mut leave_out = Vec::new();
    let mut draw = |ui: &mut Ui| {
        let view = Impact { title: "Import pinout into the drafts?", gate: Gate::Review, counts: &counts, findings: &blocking, rows: &[] };
        if let Some(c) = impact(ui, egui::Id::unique("impact"), &view, &mut leave_out, true) {
            chosen.set(Some(c));
        }
    };
    h.settle(&mut draw);
    assert!(h.find(Role::Button, "Apply").disabled);
    h.key(egui::Key::Enter, &mut draw);
    assert_eq!(chosen.get(), None, "Enter does not apply past a blocking finding");
    h.click(Role::Button, "Decline", &mut draw);
    assert_eq!(chosen.get(), Some(GateChoice::Decline));
}

#[test]
fn impact_ask_offers_the_four_answers() {
    for (button, expected) in [
        ("Allow once", GateChoice::AllowOnce),
        ("Allow for this session", GateChoice::AllowForSession),
        ("Always allow", GateChoice::AlwaysAllow),
        ("Deny", GateChoice::Deny),
    ] {
        every_size(|h| {
            let mut chosen = None;
            let mut leave_out = Vec::new();
            let mut draw = |ui: &mut Ui| {
                let view = Impact { title: "Export fabrication files?", gate: Gate::Ask, counts: &[], findings: &[], rows: &[] };
                if let Some(c) = impact(ui, egui::Id::unique("ask"), &view, &mut leave_out, true) {
                    chosen = Some(c);
                }
            };
            h.settle(&mut draw);
            h.buttons_fit();
            h.click(Role::Button, button, &mut draw);
            assert_eq!(chosen, Some(expected), "{button}");
        });
    }
}

#[test]
fn run_block_shows_steps_meter_and_stop() {
    let steps = [
        StepLine { title: "Inputs", state: StepState::Done, summary: "tlv757p.pdf" },
        StepLine { title: "Read", state: StepState::Running, summary: "" },
        StepLine { title: "Package", state: StepState::Waiting, summary: "" },
    ];
    let gauges = [
        GaugeRow { unit: "requests", used: 14, limit: 40 },
        GaugeRow { unit: "tokens", used: 61_000, limit: 200_000 },
        GaugeRow { unit: "min", used: 6, limit: 45 },
    ];
    every_size(|h| {
        let mut stopped = false;
        let mut draw = |ui: &mut Ui| {
            let view =
                Run { title: "Datasheet → component · TLV75733PDBVR", version: "1.0", steps: &steps, gauges: &gauges, running: true, ended: "" };
            stopped |= run_block(ui, &view);
        };
        h.settle(&mut draw);
        h.buttons_fit();
        let meter = "14 of 40 requests · 61 k of 200 k tokens · 6 of 45 min";
        assert!(h.nodes.iter().any(|n| n.name == meter), "{:#?}", h.names());
        assert!(h.nodes.iter().any(|n| n.name == "Read · 1 of 3"), "{:#?}", h.names());
        h.click(Role::Button, "Stop", &mut draw);
        assert!(stopped);
    });
}

#[test]
fn the_meter_shows_no_prices() {
    let line = meter_line(&[GaugeRow { unit: "requests", used: 3, limit: 40 }, GaugeRow { unit: "tokens", used: 12_345_678, limit: 100_000_000 }]);
    assert_eq!(line, "3 of 40 requests · 12 M of 100 M tokens");
    assert!(!line.contains('$') && !line.contains('€'));
}

#[test]
fn result_card_opens_in_the_editor_and_undoes_the_run() {
    let items = [ItemRow { title: "Drafts", value: "component, symbol, package, footprint" }];
    let findings = [FindingRow { blocking: false, message: "5 pins unreviewed" }];
    every_size(|h| {
        let mut chosen = Vec::new();
        let mut draw = |ui: &mut Ui| {
            let view = Outcome { title: "Done · 1 undo step", items: &items, findings: &findings, actions: &["Show", "Open in editor"], undo: true };
            if let Some(c) = outcome(ui, &view, |_| {}) {
                chosen.push(c);
            }
        };
        h.settle(&mut draw);
        h.buttons_fit();
        h.click(Role::Button, "Open in editor", &mut draw);
        h.click(Role::Button, "Undo run", &mut draw);
        assert_eq!(chosen, vec![OutcomeChoice::Action(1), OutcomeChoice::UndoRun]);
    });
}

#[test]
fn input_fields_are_named_by_their_titles_and_take_text() {
    every_size(|h| {
        let mut part = String::new();
        let mut pages = String::new();
        let mut size = String::new();
        let mut chosen = vec!["sot-23".to_owned()];
        let mut files_event = None;
        let mut accepted = false;
        let options = [ChoiceOption { value: "sot-23", title: "SOT-23" }, ChoiceOption { value: "qfn", title: "QFN" }];
        let files = [FileRow { title: "tlv757p.pdf", detail: "38 pages · 1.2 MB" }];
        let mut draw = |ui: &mut Ui| {
            card(ui, "Inputs", "", |ui| {
                let response = field(ui, &Field { title: "Part name", help: "The manufacturer part number", optional: false }, None, |ui| {
                    text_input(ui, egui::Id::unique("part"), &mut part, false, 64, false)
                });
                accepted |= entered(ui, &response);
                field(ui, &Field { title: "Pages to read first", help: "", optional: true }, None, |ui| {
                    pages_input(ui, egui::Id::unique("pages"), &mut pages, Some(38), false)
                });
                field(ui, &Field { title: "Pitch", help: "", optional: false }, Some("give a length such as 0.5mm"), |ui| {
                    number_input(ui, egui::Id::unique("pitch"), &mut size, "mm", true)
                });
                field(ui, &Field { title: "Package", help: "", optional: false }, None, |ui| {
                    choice_input(ui, egui::Id::unique("package"), &options, &mut chosen, false)
                });
                if let Some(e) =
                    field(ui, &Field { title: "Datasheets", help: "", optional: false }, None, |ui| files_input(ui, &files, "PDFs", true))
                {
                    files_event = Some(e);
                }
            });
        };
        h.settle(&mut draw);
        h.buttons_fit();
        h.click(Role::TextInput, "Part name", &mut draw);
        h.frame(vec![egui::Event::Text("TLV75733PDBVR".into())], &mut draw);
        h.key(egui::Key::Enter, &mut draw);
        h.click(Role::CheckBox, "QFN", &mut draw);
        h.click(Role::Button, "Choose…", &mut draw);
        assert_eq!(part, "TLV75733PDBVR");
        assert!(accepted, "Enter in a field accepts the card");
        assert_eq!(chosen, vec!["qfn".to_owned()], "a single choice replaces");
        assert_eq!(files_event, Some(FilesEvent::Choose));
        h.find(Role::TextInput, "Pages to read first");
        h.find(Role::TextInput, "Pitch");
        assert!(h.nodes.iter().any(|n| n.name == "of 38"));
        assert!(h.nodes.iter().any(|n| n.name == "give a length such as 0.5mm"));
    });
}

#[test]
fn the_slash_menu_lists_workflows_and_skips_those_that_cannot_run() {
    every_size(|h| {
        let rows = [
            MenuRow { title: "Datasheet → component", summary: "Draft a component from datasheets", enabled: true },
            MenuRow { title: "Review a schematic", summary: "Needs the checks module 2.0", enabled: false },
        ];
        let chosen = Cell::new(None);
        let mut highlighted = 1;
        let mut draw = |ui: &mut Ui| {
            if let Some(i) = workflow_menu(ui, &rows, &mut highlighted, true) {
                chosen.set(Some(i));
            }
        };
        h.settle(&mut draw);
        assert!(h.find(Role::Button, "Review a schematic").disabled);
        h.key(egui::Key::Enter, &mut draw);
        assert_eq!(chosen.take(), Some(0), "the highlight skips a row that cannot run");
        h.click(Role::Button, "Datasheet → component", &mut draw);
        assert_eq!(chosen.take(), Some(0));
    });
}

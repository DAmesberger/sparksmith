//! Board text in Layout: the Text tool places the text its card holds,
//! centred on each click (on the 0.5 mm grid), as one `layout.text_add`;
//! a click on a text selects it, and its card changes the words, height,
//! turn, side and position (`layout.text_edit`) or deletes it
//! (`layout.text_delete`). Each is one undo step. The text prints through
//! the board's one legend, so it is drawn with the silkscreen.
use super::{PcbEditor, focus_within, snap};
use crate::kit::{self, ToolCard};
use egui::Ui;
use sparksmith_core::layout::{self, LayoutCommand};
use sparksmith_core::library::graphics::{Rotation, TEXT_HEIGHT_MM, TextMetric};
use sparksmith_core::model::{BoardText, BoardTextSpec, Layer};
use sparksmith_core::units::{Nm, PointNm};
use sparksmith_core::{Id, Operation, Point, Project};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{self, Button};

/// The Text tool's settings: what each click places.
#[derive(Clone, Debug)]
pub(super) struct TextDraw {
    pub text: String,
    pub height_mm: f64,
    pub side: Layer,
}

impl Default for TextDraw {
    fn default() -> Self {
        Self {
            text: String::new(),
            height_mm: 0.0,
            side: Layer::Top,
        }
    }
}

/// The selected text's card while its fields are edited: the text as last
/// seen, and the words, height and position being typed.
#[derive(Clone, Debug)]
pub(super) struct TextCard {
    seen: BoardText,
    text: String,
    height_mm: f64,
    position: [f64; 2],
}

impl TextCard {
    fn of(text: &BoardText) -> Self {
        Self {
            seen: text.clone(),
            text: text.text.clone(),
            height_mm: text.height_nm.to_mm_f64(),
            position: text.position.to_mm_f64(),
        }
    }
}

fn side_options() -> [(Layer, String); 2] {
    [
        (Layer::Top, "Top silkscreen".to_owned()),
        (Layer::Bottom, "Bottom silkscreen (mirrored)".to_owned()),
    ]
}

fn turn_options() -> [(Rotation, String); 4] {
    [
        (Rotation::R0, "0°"),
        (Rotation::R90, "90°"),
        (Rotation::R180, "180°"),
        (Rotation::R270, "270°"),
    ]
    .map(|(turn, name)| (turn, name.to_owned()))
}

/// Board coordinates in the core's range (±2 m).
const POSITION_MM: std::ops::RangeInclusive<f64> = -2000.0..=2000.0;

impl PcbEditor {
    /// Select board text `id`.
    pub fn select_board_text(&mut self, id: Id) {
        self.set_selection(None, None, None);
        self.board_text = Some(id);
    }

    /// The board text under `at` on a shown silkscreen: the last placed
    /// first.
    pub(super) fn pick_board_text(
        &self,
        at: Point,
        tolerance: f64,
        metric: TextMetric,
    ) -> Option<Id> {
        let layers = self.seen();
        layout::pick_graphic(&self.graphics, at, tolerance, metric, |g| {
            g.board_text().is_some() && layers.drawing(g.layer)
        })
        .and_then(|g| g.board_text())
    }

    /// The tool's text at its height when none is set yet: the rules'
    /// minimum silkscreen text height, else the library's smallest.
    fn text_height_default(project: &Project) -> f64 {
        let min = project.rules.effective().silkscreen.text_height_min;
        min.max(*TEXT_HEIGHT_MM.start())
    }

    /// The `layout.text_add` of the tool's text centred on `at`, on the
    /// grid; `None` while the card holds no text.
    pub(super) fn text_click(&mut self, project: &Project, at: Point) -> Option<Operation> {
        if self.text_draw.text.trim().is_empty() {
            self.text_draw_needs_text = true;
            return None;
        }
        if self.text_draw.height_mm <= 0.0 {
            self.text_draw.height_mm = Self::text_height_default(project);
        }
        let spec = BoardTextSpec {
            text: self.text_draw.text.clone(),
            position: PointNm::from_mm_f64_rounded(snap(at)).ok()?,
            height_nm: Nm::from_mm_f64_rounded(self.text_draw.height_mm).ok()?,
            rotation: Rotation::R0,
            side: self.text_draw.side,
        };
        Some(Operation::Layout(LayoutCommand::AddText {
            text: Box::new(spec),
        }))
    }

    /// The Text tool card: the text, height and side of what a click
    /// places. Tool settings are editor state, so it issues no operation.
    pub(super) fn text_tool_card(&mut self, ui: &mut Ui, project: &Project) {
        if self.text_draw.height_mm <= 0.0 {
            self.text_draw.height_mm = Self::text_height_default(project);
        }
        let card = ToolCard {
            icon: "list",
            name: "Text",
            key: "",
            hint: if self.text_draw.text.trim().is_empty() {
                "Type the text first, then click where its centre goes"
            } else {
                "Click where the text's centre goes; each click places one"
            },
            tip: "Writes text on the board's silkscreen, such as the board's name and \
                  revision. Each click places the text centred on the 0.5 mm grid, on the \
                  top or bottom silkscreen; the bottom's reads mirrored from the top, so it \
                  reads correctly on the turned board. A placed text's card changes it.",
            finish: None,
        };
        let t = Tokens::get();
        let margin = egui::Margin::symmetric(t.space_8 as i8, 0);
        let response = egui::Frame::NONE
            .inner_margin(margin)
            .show(ui, |ui| {
                kit::tool_card(ui, &card, |ui| {
                    let field = kit::multiline_row(
                        ui,
                        "Text",
                        &mut self.text_draw.text,
                        self.text_draw_needs_text,
                    );
                    if self.text_draw_needs_text {
                        kit::field_error(ui, &field, "Type the text to place first", true);
                    }
                    if field.changed() {
                        self.text_draw_needs_text = false;
                    }
                    kit::number_row(
                        ui,
                        "Height",
                        &mut self.text_draw.height_mm,
                        "mm",
                        TEXT_HEIGHT_MM,
                    );
                    kit::choice_row(ui, "Side", &mut self.text_draw.side, &side_options());
                })
            })
            .inner;
        if response.done {
            self.end_route();
        }
    }

    /// TEXT: the words, height, turn, side and position of a board text.
    /// A turn or side commits at once; the typed fields when the card
    /// loses focus. Each commit is one `layout.text_edit`.
    pub(super) fn board_text_section(
        &mut self,
        ui: &mut Ui,
        text: &BoardText,
    ) -> Option<Operation> {
        let card = match self.text_card.take() {
            Some(card) if card.seen == *text => card,
            _ => TextCard::of(text),
        };
        let mut card = card;
        let mut next = text.clone();
        let mut chosen = false;
        let mut focused = false;
        kit::section(ui, "pcb-board-text", "Text", true, None, |ui| {
            let fields = ui.scope(|ui| {
                let blank = card.text.trim().is_empty();
                kit::multiline_row(ui, "Text", &mut card.text, blank);
                kit::number_row(ui, "Height", &mut card.height_mm, "mm", TEXT_HEIGHT_MM);
                kit::pair_row(ui, "Position", &mut card.position, "mm", POSITION_MM);
            });
            focused = focus_within(ui, fields.response.rect);
            chosen |= kit::choice_row(ui, "Turn", &mut next.rotation, &turn_options());
            chosen |= kit::choice_row(ui, "Side", &mut next.side, &side_options());
        });
        if self.card_escape && focused {
            card = TextCard::of(text);
        }
        let typed = !focused
            && (card.text != text.text
                || card.height_mm != text.height_nm.to_mm_f64()
                || card.position != text.position.to_mm_f64());
        if typed {
            next.text = card.text.clone();
            next.height_nm = Nm::from_mm_f64_rounded(card.height_mm).ok()?;
            next.position = PointNm::from_mm_f64_rounded(card.position).ok()?;
        }
        let changed = (chosen || typed) && next != *text;
        // A refused edit shows the stored text again next frame.
        self.text_card = (!changed).then_some(card);
        changed.then(|| {
            crate::refusal::mark(ui.ctx(), crate::refusal::Origin::Command);
            Operation::Layout(LayoutCommand::EditText {
                text: Box::new(next),
            })
        })
    }

    /// The text card's footer: Delete.
    pub(super) fn board_text_footer(ui: &mut Ui, text: &BoardText) -> Option<Operation> {
        ui_kit::button(ui, "Delete", Button::Normal)
            .on_hover_text("Delete the text")
            .clicked()
            .then_some(Operation::Layout(LayoutCommand::DeleteText { id: text.id }))
    }
}

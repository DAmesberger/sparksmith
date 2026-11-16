//! The assistant's in-chat cards (`docs/assistant-panel.md` §3.1, §4, §7): the ten controls of
//! a workflow, the workflow card ("Just chat" / "Use it"), a running workflow's block and the
//! `/` menu's rows. Sparksmith's own code, built once here on the kit's container, buttons,
//! fields and tokens; the chat transcript (the desktop app) supplies the content and acts on
//! what each card returns.
//!
//! Every card is drawn from plain borrowed data and returns what the person chose. A card that
//! takes an answer is `active` while it is the one the run waits on: only then do Enter (accept)
//! and Esc (decline) reach it, and its first field takes the keyboard when it appears.
use crate::kit::frame::Side;
use crate::kit::layout::{LABEL, Text, scope, text};
use crate::kit::metrics::{dimension, row_height};
use crate::kit::{BackingTooltipExt as _, Button, Container, Surface, button, checkbox, type_face};
use crate::tokens::Tokens;
use egui::{Response, RichText, Ui, vec2};

/// A field's heading: its title, help and whether it may stay empty.
#[derive(Clone, Copy, Debug, Default)]
pub struct Field<'a> {
    pub title: &'a str,
    pub help: &'a str,
    pub optional: bool,
}

/// One button of a card's footer, right-aligned with the primary last.
#[derive(Clone, Copy, Debug)]
pub struct FooterButton<'a> {
    pub label: &'a str,
    pub primary: bool,
    pub enabled: bool,
    /// Why it is disabled, shown on hover.
    pub why_not: &'a str,
}

impl<'a> FooterButton<'a> {
    pub const fn new(label: &'a str) -> Self {
        Self { label, primary: false, enabled: true, why_not: "" }
    }
    pub const fn primary(label: &'a str) -> Self {
        Self { label, primary: true, enabled: true, why_not: "" }
    }
    /// Disabled, saying why on hover, when `why_not` is not empty.
    pub const fn unless(mut self, why_not: &'a str) -> Self {
        self.enabled = why_not.is_empty();
        self.why_not = why_not;
        self
    }
}

/// The keys a card that takes an answer reads while it is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKey {
    /// Enter, outside a multiline field.
    Accept,
    /// Esc.
    Decline,
}

/// Enter or Esc for the active card: Esc always, Enter when no field holds the keyboard (a
/// single-line field's own Enter is read through [`entered`]).
pub fn card_key(ui: &Ui, active: bool) -> Option<CardKey> {
    if !active {
        return None;
    }
    let ctx = ui.ctx();
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        Some(CardKey::Decline)
    } else if ctx.memory(|m| m.focused().is_none()) && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        Some(CardKey::Accept)
    } else {
        None
    }
}

/// Whether a single-line field gave up the keyboard to Enter this frame.
pub fn entered(ui: &Ui, response: &Response) -> bool {
    response.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter))
}

/// The card's outer chrome: the kit container's panel frame without its shadow, holding a header
/// (title, and `aside` on the right, such as the workflow's version) and the body.
pub fn card<R>(ui: &mut Ui, title: &str, aside: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    let frame = Container::floating_card(Surface::Panel)
        .frame(ui.ctx(), t.panel, Side::Right)
        .shadow(egui::Shadow::NONE)
        .corner_radius(t.radius_md)
        .inner_margin(egui::Margin::same(t.space_8 as i8));
    let width = dimension(ui.available_width() - 2.0 * t.space_8 - 2.0);
    Container::show_backing(ui, frame, |ui| {
        ui.set_width(width);
        scope(ui, |ui| {
            ui.spacing_mut().item_spacing.y = t.space_6;
            header(ui, title, aside);
            body(ui)
        })
    })
    .inner
}

fn header(ui: &mut Ui, title: &str, aside: &str) {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(title).font(type_face::semibold(t.font_button)).color(t.form_ink)).truncate());
        if !aside.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(aside).size(t.font_small).color(t.form_dim)));
            });
        }
    });
}

/// A dim line of text that wraps.
pub fn note(ui: &mut Ui, line: &str) -> Response {
    text(ui, line, Text::Hint(line))
}

/// A line of text in the form's ink that wraps.
pub fn line(ui: &mut Ui, line: &str) -> Response {
    let t = Tokens::get();
    ui.add(egui::Label::new(RichText::new(line).size(t.font_label).color(t.form_ink)).wrap())
}

/// The footer: `buttons` right-aligned, in order, the primary last. Answers the index of the one
/// clicked.
pub fn footer(ui: &mut Ui, buttons: &[FooterButton<'_>]) -> Option<usize> {
    let mut clicked = None;
    // Right-aligned, wrapping to another row where the column is narrow.
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min).with_main_wrap(true), |ui| {
        for (index, b) in buttons.iter().enumerate().rev() {
            let style = match (b.primary, b.enabled) {
                (_, false) => Button::Enabled(false),
                (true, true) => Button::Primary,
                (false, true) => Button::Normal,
            };
            let response = button(ui, b.label, style);
            let response = if b.enabled || b.why_not.is_empty() { response } else { response.backing_on_disabled_hover_text(b.why_not) };
            if b.enabled && response.clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// A field: its title (with "optional"), the control under it, its help and its error. The
/// control is labelled with the title for assistive technology and the scenario driver.
pub fn field<R>(ui: &mut Ui, field: &Field<'_>, error: Option<&str>, control: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(field.title).font(type_face::medium(t.font_label)).color(t.form_ink)));
        if field.optional {
            ui.add(egui::Label::new(RichText::new("optional").size(t.font_small).color(t.form_dim)));
        }
    });
    let info = egui::UiStackInfo::default().with_tag_value(LABEL, field.title.to_owned());
    let inner = ui.scope_builder(egui::UiBuilder::new().ui_stack_info(info), control).inner;
    if !field.help.is_empty() {
        note(ui, field.help);
    }
    if let Some(error) = error {
        text(ui, error, Text::Error(error));
    }
    inner
}

/// `text`: one line or several, at most `max_chars` characters (the byte limit is the caller's
/// check).
pub fn text_input(ui: &mut Ui, id: egui::Id, value: &mut String, multiline: bool, max_chars: usize, invalid: bool) -> Response {
    if !multiline {
        return crate::kit::fields::bounded_cell_editor(ui, id, value, "", invalid, max_chars);
    }
    let t = Tokens::get();
    let editor = egui::TextEdit::multiline(value)
        .id(id)
        .char_limit(max_chars)
        .desired_rows(3)
        .desired_width(dimension(ui.available_width()))
        .font(egui::FontId::proportional(t.font_label))
        .background_color(if invalid { t.form_error_fill } else { t.input_fill() })
        .text_color(if invalid { t.form_error } else { t.form_ink })
        .margin(vec2(t.form_inset, t.space_4));
    let response = ui.add(editor);
    let title = crate::kit::layout::label(ui).to_owned();
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, ui.is_enabled(), &title);
        info.current_text_value = Some(value.clone());
        info
    });
    t.input_bevel(ui, &response);
    response
}

/// `number`: what the person types (`0.25`, `0.25mm`, `10mil`), with the unit the bare number is
/// read in beside it.
pub fn number_input(ui: &mut Ui, id: egui::Id, value: &mut String, unit: &str, invalid: bool) -> Response {
    let t = Tokens::get();
    ui.horizontal(|ui| {
        let unit_width = if unit.is_empty() { 0.0 } else { t.space_24 * 1.5 };
        ui.set_width(dimension(ui.available_width()));
        let field = dimension(ui.available_width() - unit_width - t.space_4);
        let response = ui.allocate_ui(vec2(field, row_height()), |ui| crate::kit::fields::bounded_cell_editor(ui, id, value, "", invalid, 64)).inner;
        if !unit.is_empty() {
            ui.add(egui::Label::new(RichText::new(unit).size(t.font_label).color(t.form_dim)));
        }
        response
    })
    .inner
}

/// A titled option of a `choice`.
#[derive(Clone, Copy, Debug)]
pub struct ChoiceOption<'a> {
    pub value: &'a str,
    pub title: &'a str,
}

/// The most option rows a choice shows before it scrolls.
pub const CHOICE_ROWS: usize = 8;

/// `choice`: single or multiple. A single choice replaces `chosen`; a multiple one toggles.
/// Answers whether `chosen` changed. More than [`CHOICE_ROWS`] options scroll.
pub fn choice_input(ui: &mut Ui, id: egui::Id, options: &[ChoiceOption<'_>], chosen: &mut Vec<String>, multiple: bool) -> bool {
    let mut changed = false;
    let mut rows = |ui: &mut Ui| {
        for option in options {
            let mut on = chosen.iter().any(|c| c == option.value);
            if checkbox(ui, option.title, &mut on).changed() {
                changed = true;
                if multiple {
                    if on {
                        chosen.push(option.value.to_owned());
                    } else {
                        chosen.retain(|c| c != option.value);
                    }
                } else {
                    chosen.clear();
                    if on {
                        chosen.push(option.value.to_owned());
                    }
                }
            }
        }
    };
    if options.len() > CHOICE_ROWS {
        let height = CHOICE_ROWS as f32 * (Tokens::get().form_checkbox_height + ui.spacing().item_spacing.y);
        egui::ScrollArea::vertical().id_salt(id).max_height(height).auto_shrink([false, true]).show(ui, rows);
    } else {
        rows(ui);
    }
    changed
}

/// A file a `datasheet` field holds, as it shows it.
#[derive(Clone, Copy, Debug)]
pub struct FileRow<'a> {
    pub title: &'a str,
    /// "38 pages · 1.2 MB", or "Reading…".
    pub detail: &'a str,
}

/// What the person did on a `datasheet` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesEvent {
    /// Choose… : the host opens its file picker.
    Choose,
    /// Remove the file at this index.
    Remove(usize),
}

/// `datasheet`: the files given, each removable, and a drop target with Choose… while more may
/// be added. The target lights up while files are dragged over the window; the host reads the
/// drop (it knows which field is active).
pub fn files_input(ui: &mut Ui, files: &[FileRow<'_>], accepts: &str, can_add: bool) -> Option<FilesEvent> {
    let t = Tokens::get();
    let mut event = None;
    for (index, file) in files.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.set_width(dimension(ui.available_width()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, "Remove", Button::Small).on_hover_text(format!("Remove {}", file.title)).clicked() {
                    event = Some(FilesEvent::Remove(index));
                }
                ui.add(egui::Label::new(RichText::new(file.detail).size(t.font_small).color(t.form_dim)));
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(file.title).size(t.font_label).color(t.form_ink)).truncate());
                });
            });
        });
    }
    if can_add {
        let dragging = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let (fill, ink) = if dragging { (t.selected_bg, t.selected_ink) } else { (t.input_fill(), t.form_dim) };
        let frame = egui::Frame::NONE
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, if dragging { t.form_primary } else { t.control_border }))
            .corner_radius(t.control_radius)
            .inner_margin(egui::Margin::symmetric(t.form_inset as i8, t.space_6 as i8));
        Container::show_backing(ui, frame, |ui| {
            ui.set_width(dimension(ui.available_width()));
            ui.horizontal(|ui| {
                let drop = if dragging { format!("Drop to add {accepts}") } else { format!("Drop {accepts} here") };
                ui.add(egui::Label::new(RichText::new(drop).size(t.font_label).color(ink)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if button(ui, "Choose…", Button::Small).clicked() {
                        event = Some(FilesEvent::Choose);
                    }
                });
            });
        });
    }
    event
}

/// `url`: an `https://` URL; the note says it is fetched only once allowed.
pub fn url_input(ui: &mut Ui, id: egui::Id, value: &mut String, invalid: bool) -> Response {
    let response = crate::kit::fields::bounded_cell_editor(ui, id, value, "https://", invalid, 2048);
    note(ui, "Fetched only after you allow it.");
    response
}

/// `page_ranges`: pages such as `1-5, 31, 32`, against the file's page count when known.
pub fn pages_input(ui: &mut Ui, id: egui::Id, value: &mut String, pages: Option<u32>, invalid: bool) -> Response {
    let t = Tokens::get();
    let of = pages.map(|n| format!("of {n}")).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.set_width(dimension(ui.available_width()));
        let width = if of.is_empty() { 0.0 } else { t.space_24 * 2.0 };
        let field = dimension(ui.available_width() - width - t.space_4);
        let response =
            ui.allocate_ui(vec2(field, row_height()), |ui| crate::kit::fields::bounded_cell_editor(ui, id, value, "1-5, 31", invalid, 256)).inner;
        if !of.is_empty() {
            ui.add(egui::Label::new(RichText::new(of).size(t.font_label).color(t.form_dim)));
        }
        response
    })
    .inner
}

/// A count on an impact card.
#[derive(Clone, Copy, Debug)]
pub struct CountRow<'a> {
    pub label: &'a str,
    pub n: u64,
}

/// A finding on an impact or result card.
#[derive(Clone, Copy, Debug)]
pub struct FindingRow<'a> {
    pub blocking: bool,
    pub message: &'a str,
}

/// A changed row the person may leave out.
#[derive(Clone, Copy, Debug)]
pub struct ChangeRow<'a> {
    pub id: &'a str,
    pub text: &'a str,
}

/// What an impact card asks: a plan to apply (a workflow's confirm gate), or a call that needs
/// a person's Allow (the surface's Ask).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Review,
    Ask,
}

/// The person's answer on an impact card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateChoice {
    Apply,
    Decline,
    AllowOnce,
    AllowForSession,
    AlwaysAllow,
    Deny,
}

/// What an impact card shows.
#[derive(Clone, Copy, Debug)]
pub struct Impact<'a> {
    pub title: &'a str,
    pub gate: Gate,
    pub counts: &'a [CountRow<'a>],
    pub findings: &'a [FindingRow<'a>],
    pub rows: &'a [ChangeRow<'a>],
}

/// The most changed rows listed before the rest are counted.
pub const IMPACT_ROWS: usize = 8;

/// `impact`: counts, findings and the rows the person may leave out (`leave_out`, by row id),
/// with Apply and Decline for a plan, or Allow once, Allow for this session, Always allow and
/// Deny for a call that asks. Enter applies or allows once, Esc declines or denies.
pub fn impact(ui: &mut Ui, id: egui::Id, view: &Impact<'_>, leave_out: &mut Vec<String>, active: bool) -> Option<GateChoice> {
    let t = Tokens::get();
    card(ui, view.title, "", |ui| {
        if !view.counts.is_empty() {
            let counts: Vec<String> = view.counts.iter().map(|c| format!("{} {}", c.n, c.label)).collect();
            line(ui, &counts.join(" · "));
        }
        let blocking = view.findings.iter().filter(|f| f.blocking).count();
        if view.findings.is_empty() {
            if view.gate == Gate::Review {
                note(ui, "Findings: none");
            }
        } else {
            for finding in view.findings.iter().take(IMPACT_ROWS) {
                if finding.blocking {
                    text(ui, finding.message, Text::Error(finding.message));
                } else {
                    note(ui, finding.message);
                }
            }
            if view.findings.len() > IMPACT_ROWS {
                note(ui, &format!("{} more findings", view.findings.len() - IMPACT_ROWS));
            }
        }
        if !view.rows.is_empty() {
            let open_id = id.with("leave_out");
            let mut open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
            let label =
                if leave_out.is_empty() { "Leave out rows…".to_owned() } else { format!("Leave out rows… ({} left out)", leave_out.len()) };
            if button(ui, label, if open { Button::SmallSelected } else { Button::Small }).clicked() {
                open = !open;
                ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
            }
            if open {
                egui::ScrollArea::vertical()
                    .id_salt(id.with("rows"))
                    .max_height(IMPACT_ROWS as f32 * (t.form_checkbox_height + ui.spacing().item_spacing.y))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for row in view.rows {
                            let mut out = leave_out.iter().any(|r| r == row.id);
                            if checkbox(ui, row.text, &mut out).changed() {
                                if out {
                                    leave_out.push(row.id.to_owned());
                                } else {
                                    leave_out.retain(|r| r != row.id);
                                }
                            }
                        }
                    });
            }
        }
        let key = card_key(ui, active);
        match view.gate {
            Gate::Review => {
                let apply_why = if blocking > 0 { "A blocking finding stops Apply" } else { "" };
                let buttons = [FooterButton::new("Decline"), FooterButton::primary("Apply").unless(apply_why)];
                match (footer(ui, &buttons), key) {
                    (Some(1), _) | (None, Some(CardKey::Accept)) if blocking == 0 => Some(GateChoice::Apply),
                    (Some(0), _) | (None, Some(CardKey::Decline)) => Some(GateChoice::Decline),
                    _ => None,
                }
            }
            Gate::Ask => {
                let buttons = [
                    FooterButton::new("Deny"),
                    FooterButton::new("Always allow"),
                    FooterButton::new("Allow for this session"),
                    FooterButton::primary("Allow once"),
                ];
                match (footer(ui, &buttons), key) {
                    (Some(3), _) | (None, Some(CardKey::Accept)) => Some(GateChoice::AllowOnce),
                    (Some(2), _) => Some(GateChoice::AllowForSession),
                    (Some(1), _) => Some(GateChoice::AlwaysAllow),
                    (Some(0), _) | (None, Some(CardKey::Decline)) => Some(GateChoice::Deny),
                    _ => None,
                }
            }
        }
    })
}

/// `progress`: the current step, done of total, and Stop. Answers whether Stop was pressed.
pub fn progress(ui: &mut Ui, step: &str, done: u32, total: u32, can_stop: bool) -> bool {
    let t = Tokens::get();
    let mut stop = false;
    ui.horizontal(|ui| {
        ui.set_width(dimension(ui.available_width()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if can_stop && button(ui, "Stop", Button::Normal).clicked() {
                stop = true;
            }
            let fraction = if total == 0 { 0.0 } else { (done.min(total) as f32) / (total as f32) };
            ui.add(egui::ProgressBar::new(fraction).desired_width(t.space_24 * 3.0).desired_height(t.space_6).fill(t.form_primary));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(format!("{step} · {done} of {total}")).size(t.font_label).color(t.form_ink)).truncate());
            });
        });
    });
    stop
}

/// One limit of the meter and how much of it is used.
#[derive(Clone, Copy, Debug)]
pub struct GaugeRow<'a> {
    /// "requests", "tokens", "min".
    pub unit: &'a str,
    pub used: u64,
    pub limit: u64,
}

/// A count the way the meter shows it: `61 k` past ten thousand, else whole.
pub fn short_count(n: u64) -> String {
    if n >= 10_000_000 {
        format!("{} M", n / 1_000_000)
    } else if n >= 10_000 {
        format!("{} k", n / 1_000)
    } else {
        n.to_string()
    }
}

/// The meter's line: "14 of 40 requests · 61 k of 200 k tokens · 6 of 45 min". No prices.
pub fn meter_line(gauges: &[GaugeRow<'_>]) -> String {
    gauges.iter().map(|g| format!("{} of {} {}", short_count(g.used), short_count(g.limit), g.unit)).collect::<Vec<_>>().join(" · ")
}

/// `meter`: requests, tokens and minutes against the run's limits; a limit reached is in the
/// error ink.
pub fn meter(ui: &mut Ui, gauges: &[GaugeRow<'_>]) -> Response {
    let t = Tokens::get();
    let full = gauges.iter().any(|g| g.limit > 0 && g.used >= g.limit);
    let ink = if full { t.form_error } else { t.form_dim };
    ui.add(egui::Label::new(RichText::new(meter_line(gauges)).size(t.font_small).color(ink)).wrap())
}

/// A titled value on the result card.
#[derive(Clone, Copy, Debug)]
pub struct ItemRow<'a> {
    pub title: &'a str,
    pub value: &'a str,
}

/// What a result card shows.
#[derive(Clone, Copy, Debug)]
pub struct Outcome<'a> {
    pub title: &'a str,
    pub items: &'a [ItemRow<'a>],
    pub findings: &'a [FindingRow<'a>],
    /// The buttons that make a `show` call: Show, Open in editor.
    pub actions: &'a [&'a str],
    /// Whether Undo run is offered.
    pub undo: bool,
}

/// What the person chose on a result card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeChoice {
    /// The action at this index.
    Action(usize),
    UndoRun,
}

/// `result`: what was made, the open findings, its actions and Undo run.
pub fn outcome(ui: &mut Ui, view: &Outcome<'_>) -> Option<OutcomeChoice> {
    let t = Tokens::get();
    card(ui, view.title, "", |ui| {
        for item in view.items {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(item.title).size(t.font_label).color(t.form_dim)));
                ui.add(egui::Label::new(RichText::new(item.value).size(t.font_label).color(t.form_ink)).wrap());
            });
        }
        for finding in view.findings.iter().take(IMPACT_ROWS) {
            if finding.blocking {
                text(ui, finding.message, Text::Error(finding.message));
            } else {
                note(ui, finding.message);
            }
        }
        if view.findings.len() > IMPACT_ROWS {
            note(ui, &format!("{} more findings", view.findings.len() - IMPACT_ROWS));
        }
        let mut buttons: Vec<FooterButton<'_>> = Vec::with_capacity(view.actions.len() + 1);
        if view.undo {
            buttons.push(FooterButton::new("Undo run"));
        }
        let first = buttons.len();
        for (index, label) in view.actions.iter().enumerate() {
            let last = index + 1 == view.actions.len();
            buttons.push(if last { FooterButton::primary(label) } else { FooterButton::new(label) });
        }
        match footer(ui, &buttons) {
            Some(i) if i >= first => Some(OutcomeChoice::Action(i - first)),
            Some(_) => Some(OutcomeChoice::UndoRun),
            None => None,
        }
    })
}

/// A given input on the workflow card ("Datasheet  tlv757p.pdf (38 pages)").
#[derive(Clone, Copy, Debug)]
pub struct GivenRow<'a> {
    pub title: &'a str,
    pub value: &'a str,
}

/// What a workflow card shows.
#[derive(Clone, Copy, Debug)]
pub struct Offer<'a> {
    pub title: &'a str,
    pub version: &'a str,
    pub summary: &'a str,
    /// Why it fits, as the model or the signal said; may be empty.
    pub why: &'a str,
    pub given: &'a [GivenRow<'a>],
    /// The titles of the required inputs the run will still ask for.
    pub missing: &'a [&'a str],
}

/// The person's answer on a workflow card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferChoice {
    JustChat,
    UseIt,
}

/// The workflow card (§3.1): "I can do this with a workflow", the workflow, what it was given and
/// [Just chat] [Use it]. Enter uses it, Esc just chats.
pub fn offer(ui: &mut Ui, view: &Offer<'_>, active: bool) -> Option<OfferChoice> {
    let t = Tokens::get();
    card(ui, "I can do this with a workflow", "", |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(view.title).font(type_face::semibold(t.font_label)).color(t.form_ink)).truncate());
            ui.add(egui::Label::new(RichText::new(format!("v{}", view.version)).size(t.font_small).color(t.form_dim)));
        });
        line(ui, view.summary);
        if !view.why.is_empty() {
            note(ui, view.why);
        }
        for given in view.given {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(given.title).size(t.font_label).color(t.form_dim)));
                ui.add(egui::Label::new(RichText::new(given.value).size(t.font_label).color(t.form_ink)).truncate());
            });
        }
        if !view.missing.is_empty() {
            note(ui, &format!("It will ask for: {}", view.missing.join(", ")));
        }
        let key = card_key(ui, active);
        match (footer(ui, &[FooterButton::new("Just chat"), FooterButton::primary("Use it")]), key) {
            (Some(1), _) | (None, Some(CardKey::Accept)) => Some(OfferChoice::UseIt),
            (Some(0), _) | (None, Some(CardKey::Decline)) => Some(OfferChoice::JustChat),
            _ => None,
        }
    })
}

/// A step row's state in a running workflow's block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Waiting,
    Running,
    Done,
    Failed,
}

/// One row of a running workflow's step list.
#[derive(Clone, Copy, Debug)]
pub struct StepLine<'a> {
    pub title: &'a str,
    pub state: StepState,
    /// What it did ("38 pages, drawings on 31, 32") or is doing.
    pub summary: &'a str,
}

/// A running workflow's block (§7): its title, the step list, the progress, the meter and Stop.
#[derive(Clone, Copy, Debug)]
pub struct Run<'a> {
    pub title: &'a str,
    pub version: &'a str,
    pub steps: &'a [StepLine<'a>],
    pub gauges: &'a [GaugeRow<'a>],
    /// Whether the run still goes (Stop is offered); else `ended` says how it ended.
    pub running: bool,
    pub ended: &'a str,
}

/// The run block. Answers whether Stop was pressed.
pub fn run_block(ui: &mut Ui, view: &Run<'_>) -> bool {
    let t = Tokens::get();
    card(ui, view.title, &format!("v{}", view.version), |ui| {
        let mut current = None;
        let mut done = 0u32;
        for (index, step) in view.steps.iter().enumerate() {
            let (mark, ink) = match step.state {
                StepState::Done => ("✓", t.form_success),
                StepState::Running => ("◌", t.form_primary),
                StepState::Failed => ("✕", t.form_error),
                StepState::Waiting => ("·", t.form_dim),
            };
            if step.state == StepState::Done {
                done = done.saturating_add(1);
            }
            if step.state == StepState::Running {
                current = Some(index);
            }
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(mark).size(t.font_label).color(ink)));
                let title_ink = if step.state == StepState::Waiting { t.form_dim } else { t.form_ink };
                ui.add(egui::Label::new(RichText::new(step.title).size(t.font_label).color(title_ink)));
                if !step.summary.is_empty() {
                    ui.add(egui::Label::new(RichText::new(step.summary).size(t.font_small).color(t.form_dim)).truncate());
                }
            });
        }
        if !view.gauges.is_empty() {
            meter(ui, view.gauges);
        }
        if view.running {
            let step = current.and_then(|i| view.steps.get(i)).map_or("Starting", |s| s.title);
            progress(ui, step, done, u32::try_from(view.steps.len()).unwrap_or(u32::MAX), true)
        } else {
            if !view.ended.is_empty() {
                note(ui, view.ended);
            }
            false
        }
    })
}

/// A row of the `/` menu: a workflow's title and summary, or why it cannot run here.
#[derive(Clone, Copy, Debug)]
pub struct MenuRow<'a> {
    pub title: &'a str,
    pub summary: &'a str,
    pub enabled: bool,
}

/// The `/` menu's list: one row per workflow, `highlighted` lit. While `active` (the composer
/// holds the keyboard on a `/`), ↑↓ move the highlight over the rows that can run and Enter
/// chooses it. Answers the row chosen.
pub fn workflow_menu(ui: &mut Ui, rows: &[MenuRow<'_>], highlighted: &mut usize, active: bool) -> Option<usize> {
    let t = Tokens::get();
    let mut chosen = None;
    if rows.is_empty() {
        note(ui, "No workflow matches");
        return None;
    }
    let enabled = |i: usize| rows.get(i).is_some_and(|r| r.enabled);
    if active {
        let (up, down, enter) =
            ui.ctx().input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter)));
        let len = rows.len();
        *highlighted = (*highlighted).min(len - 1);
        if up || down {
            let step = if down { 1 } else { len - 1 };
            for _ in 0..len {
                *highlighted = (*highlighted + step) % len;
                if enabled(*highlighted) {
                    break;
                }
            }
        }
        if !enabled(*highlighted) {
            *highlighted = (0..len).find(|i| enabled(*i)).unwrap_or(0);
        }
        if enter && enabled(*highlighted) {
            chosen = Some(*highlighted);
        }
    }
    for (index, row) in rows.iter().enumerate() {
        let height = row_height() + t.font_small + t.space_4;
        let (rect, response) = ui.allocate_exact_size(
            vec2(dimension(ui.available_width()), height),
            if row.enabled { egui::Sense::click() } else { egui::Sense::hover() },
        );
        let lit = index == *highlighted || response.hovered();
        if lit && row.enabled {
            ui.painter().rect_filled(rect, t.radius_sm, t.selected_bg);
        }
        let ink = if row.enabled { if lit { t.selected_ink } else { t.form_ink } } else { t.form_dim };
        let inner = rect.shrink2(vec2(t.space_8, t.space_2));
        crate::kit::text::paint_text(
            ui,
            egui::Rect::from_min_size(inner.min, vec2(inner.width(), row_height() - t.space_4)),
            row.title,
            type_face::medium(t.font_label),
            ink,
            false,
        );
        let below = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.top() + row_height() - t.space_4), inner.max);
        crate::kit::text::paint_text(ui, below, row.summary, egui::FontId::proportional(t.font_small), t.form_dim, false);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, row.enabled, row.title));
        if row.enabled && response.clicked() {
            *highlighted = index;
            chosen = Some(index);
        }
    }
    chosen
}

#[cfg(test)]
#[path = "chat_cards_tests.rs"]
mod tests;

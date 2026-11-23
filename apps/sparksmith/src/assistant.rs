//! The assistant chat (`docs/assistant-panel.md` §7, slice C1): one
//! conversation per open project, in a docked column at the window's edge
//! (resizable from [`MIN_WIDTH`] to [`MAX_WIDTH`], on the right or the
//! left), which folds to a [`STRIP_WIDTH`] strip with a state dot.
//!
//! This module holds the conversation and draws the column; the window
//! (`app/assistant_host.rs`) runs what it asks for. Each message is one
//! turn of the built-in agent (`sparksmith_assist::chat`) off the UI thread
//! ([`worker`]): its tool calls come back to the window, run through the
//! front door on the window's engine in the message's turn, and undo as one
//! step. The transcript shows the person's message with the window's
//! context when it was sent, the reply in bounded Markdown, folded
//! reasoning, one row per tool call, Ask cards, and a summary card at the
//! end of each turn with what changed, Show and Undo turn.
//!
//! The composer completes `@` to a file the conversation knows
//! ([`mentions`]): the files attached in this session, then the project
//! folder's, fuzzily, with ↑↓ and Enter. The file goes with the message as
//! a chip, as does a path typed or pasted; the chat attaches it, and a
//! workflow's files input is filled from it.
mod conversation;
mod markdown;
pub mod mentions;
pub mod questions;
pub mod worker;

pub use conversation::{
    Allow, AskCard, Conversation, Item, RowState, State, TurnSummary, context_line,
};

use crate::kit;
use egui::{Ui, vec2};
use sparksmith_assist::chat::ChatLibrary;
use sparksmith_assist::model::Message;
use sparksmith_automation::Session;
use sparksmith_core::Document;
use sparksmith_ui::chat_cards;
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{self, Button, type_face};
use std::collections::HashSet;
use worker::{ChatModel, Job, RunJob, ToolRequest};

/// The column's narrowest and widest, and its width before the person drags
/// it (points).
pub const MIN_WIDTH: f32 = 300.0;
pub const MAX_WIDTH: f32 = 560.0;
pub const DEFAULT_WIDTH: f32 = 360.0;
/// The folded column: a strip with the state dot.
pub const STRIP_WIDTH: f32 = 30.0;
/// The composer's lines before it scrolls.
const COMPOSER_ROWS: usize = 3;
/// The longest message the composer sends, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// The most files one message carries.
pub const MAX_MESSAGE_FILES: usize = 16;

/// The button of the chat's not-connected state.
pub const CONNECT: &str = "Connect a model…";

/// Which edge the column docks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dock {
    #[default]
    Right,
    Left,
}

/// What the person asked of the column this frame; the window does it.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Send,
    Stop,
    /// The person's answer on the Ask card `id`.
    Answer {
        id: u64,
        allow: Allow,
    },
    /// Select what the turn of summary `id` changed.
    Show(u64),
    /// Undo the turn of summary `id`.
    UndoTurn(u64),
    /// Go on after the person's limit the turn of summary `id` stopped at:
    /// one more turn within the same limits.
    Continue(u64),
    Fold(bool),
    Dock(Dock),
    Close,
    /// What the person chose on a workflow card (C2).
    Card(crate::workflow_cards::CardAction),
    /// The `/` menu chose a workflow; its card is in the transcript.
    Offered(String),
    /// Connect a model…: Settings › AI › Connection.
    ConnectModel,
    /// What the person did on the question card `card`.
    Question {
        card: String,
        choice: questions::Choice,
    },
}

/// The turn a message opened: its handle, its undo label, the revision
/// before it, and, for its summary, what its own calls changed apart from
/// what others changed meanwhile (the person's edit, an undo, a workflow
/// run's calls).
pub struct OpenTurn {
    pub id: String,
    pub label: String,
    pub revision: u64,
    /// Each stretch of the turn's own edits with no one else's revision in
    /// between, as the documents it started and ended at.
    pub own: Vec<(Document, Document)>,
    /// Each stretch of others' revisions during the turn, likewise.
    pub others: Vec<(Document, Document)>,
    /// The revision and document where the turn's last call left off.
    at: (u64, Document),
    /// The revision the last of [`Self::own`] ended at: a call from there
    /// carries it on. A read after someone else's edit moves [`Self::at`]
    /// on, never this, so the turn never claims that edit (review,
    /// turn ownership).
    own_end: Option<u64>,
}

impl OpenTurn {
    pub fn new(id: String, label: String, engine: &sparksmith_core::Engine) -> Self {
        Self {
            id,
            label,
            revision: engine.revision(),
            own: Vec::new(),
            others: Vec::new(),
            at: (engine.revision(), engine.document().clone()),
            own_end: None,
        }
    }

    /// One of the turn's calls ran from `from` (revision and document) to
    /// where `engine` stands now.
    pub fn called(&mut self, from: (u64, Document), engine: &sparksmith_core::Engine) {
        let joined = self.own_end == Some(from.0);
        self.caught_up(from.0, &from.1);
        if engine.revision() == from.0 {
            return;
        }
        let now = engine.document().clone();
        match self.own.last_mut() {
            Some(last) if joined => last.1 = now.clone(),
            _ => self.own.push((from.1, now.clone())),
        }
        self.own_end = Some(engine.revision());
        self.at = (engine.revision(), now);
    }

    /// Others' revisions since the turn's last call, up to `revision`.
    pub fn caught_up(&mut self, revision: u64, document: &Document) {
        if revision != self.at.0 {
            self.others.push((self.at.1.clone(), document.clone()));
            self.at = (revision, document.clone());
        }
    }
}

/// A tool call waiting for the person's Allow or Deny on card `ask`.
pub struct Held {
    pub request: ToolRequest,
    pub ask: u64,
    pub question: sparksmith_automation::Question,
}

/// A tool call (`assist.ask`) waiting for the person's answers on the
/// question card `card`, which the front door checked and the window
/// showed.
pub struct Asking {
    pub request: ToolRequest,
    pub card: String,
}

/// A `library.search` with `src:online` waiting for the registry, which
/// answers on a thread of its own so the window never waits on the
/// network; when `report` arrives the call runs with it.
pub struct OnlineWait {
    pub request: ToolRequest,
    /// The person allowed the call on its card.
    pub allowed: bool,
    pub report: std::sync::mpsc::Receiver<
        Result<sparksmith_automation::session::online::OnlineReport, String>,
    >,
}

/// A workflow run that ended: Undo run takes it back while the document
/// still stands at `after` and its step is the one Undo would take.
pub struct RunDone {
    pub run: String,
    pub label: String,
    pub after: u64,
}

/// What the column shows that the window knows.
pub struct Now {
    /// The project's revision.
    pub revision: u64,
    /// The label of the step Undo would take back.
    pub undo_label: Option<String>,
    /// What the window shows now, in the person's words.
    pub context: String,
    /// The open project's folder, whose files `@` completes.
    pub folder: Option<std::path::PathBuf>,
}

/// The chat: its column and its one conversation.
pub struct Assistant {
    pub open: bool,
    pub folded: bool,
    pub dock: Dock,
    pub composer: String,
    /// The files the message being written carries, as chips.
    pub files: Vec<std::path::PathBuf>,
    /// The `@` list's highlighted row, and its candidates while it shows.
    mention: usize,
    mention_search: mentions::search::Search,
    pub conversation: Conversation,
    /// What the model has been sent and answered, for the next turn.
    pub history: Vec<Message>,
    pub job: Option<Job>,
    /// The workflow run going on in the conversation.
    pub run: Option<RunJob>,
    /// Runs that ended, for their result cards' Undo run.
    pub runs: Vec<RunDone>,
    /// What the model is told with the person's next message: a workflow
    /// card declined (Just chat), a run's result.
    pub notes: Vec<String>,
    pub turn: Option<OpenTurn>,
    pub held: Option<Held>,
    /// A question card the turn waits on.
    pub asking: Option<Asking>,
    /// Online searches waiting for the registry.
    pub online_waits: Vec<OnlineWait>,
    /// A waited-for online search is running with its answer installed.
    pub prefetched: bool,
    /// The session the chat's calls run in, over the window's engine; it
    /// keeps the turns open between calls.
    pub session: Session,
    /// The model the chat uses instead of the one set up (tests).
    pub model: Option<ChatModel>,
    /// With `model`: the limits its turns run within, as the person's
    /// would be (none: unbounded) (tests).
    pub model_limits: Option<sparksmith_assist::settings::Limits>,
    library: Option<Result<ChatLibrary, String>>,
    /// Transcript items shown expanded (tool rows, reasoning).
    expanded: HashSet<usize>,
    /// Entries the person allowed for this session on an Ask card: they
    /// run without asking until the app quits, in any project.
    pub allowed_session: HashSet<String>,
    /// Settings › AI's rules per kind of action, read when a message is
    /// sent; Always allow on a card changes them there.
    pub permissions: Option<sparksmith_assist::settings::Permissions>,
    /// Whether the chat has looked, since it opened or Settings › AI
    /// saved a connection, that a model is connected.
    pub setup_checked: bool,
}

impl Default for Assistant {
    fn default() -> Self {
        Self {
            open: false,
            folded: false,
            dock: Dock::Right,
            composer: String::new(),
            files: Vec::new(),
            mention: 0,
            mention_search: mentions::search::Search::default(),
            conversation: Conversation::default(),
            history: Vec::new(),
            job: None,
            run: None,
            runs: Vec::new(),
            notes: Vec::new(),
            turn: None,
            held: None,
            asking: None,
            online_waits: Vec::new(),
            prefetched: false,
            session: Session::lending(),
            model: None,
            model_limits: None,
            library: None,
            expanded: HashSet::new(),
            allowed_session: HashSet::new(),
            permissions: None,
            setup_checked: false,
        }
    }
}

impl Assistant {
    /// ⇧⌘A: open the column (unfolded), or close it when it shows.
    pub fn toggle(&mut self) {
        if self.open && !self.folded {
            self.open = false;
        } else {
            self.open = true;
            self.folded = false;
        }
    }

    /// Whether the column shows, unfolded.
    pub fn shown(&self) -> bool {
        self.open && !self.folded
    }

    /// The shipped chat data, read once.
    pub fn library(&mut self) -> Result<&ChatLibrary, String> {
        self.library
            .get_or_insert_with(|| ChatLibrary::locate().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The composer's id, which the busy rule leaves out: typing a message
    /// is not working in the window.
    pub fn composer_id() -> egui::Id {
        egui::Id::unique("assistant-composer")
    }

    /// Stop the turn or run going on: the model stops between chunks or
    /// calls, a run at its next step boundary, and a call waiting for the
    /// person is given up.
    pub fn stop(&mut self) {
        if let Some(job) = &self.job {
            job.stop();
        }
        if let Some(run) = &self.run {
            run.stop();
        }
        if let Some(held) = self.held.take()
            && let Some(card) = self.conversation.ask_mut(held.ask)
        {
            card.answer = Some(Allow::Deny);
        }
        if let Some(asking) = self.asking.take()
            && let Some(card) = self.conversation.question_mut(&asking.card)
        {
            card.answer = Some(questions::Answered::Stopped);
        }
    }

    /// Another project opened: its conversation starts empty, and a turn
    /// of the last one ends without touching the new project.
    pub fn document_opened(&mut self) {
        self.stop();
        self.job = None;
        self.run = None;
        self.runs.clear();
        self.notes.clear();
        self.turn = None;
        self.conversation = Conversation::default();
        self.history.clear();
        self.expanded.clear();
        self.files.clear();
    }

    /// The folded column: the state dot and the button that unfolds it.
    pub fn strip(&mut self, ui: &mut Ui) -> Vec<Action> {
        let t = Tokens::get();
        let mut actions = Vec::new();
        ui.vertical_centered(|ui| {
            ui.add_space(t.space_8);
            if icon_button(ui, "assistant", "Unfold the assistant").clicked() {
                actions.push(Action::Fold(false));
            }
            ui.add_space(t.space_8);
            state_dot(ui, &self.conversation.state);
        });
        actions
    }

    /// The column: header, transcript and composer.
    pub fn column(
        &mut self,
        ui: &mut Ui,
        now: &Now,
        cards: &mut crate::workflow_cards::WorkflowCards,
    ) -> Vec<Action> {
        let t = Tokens::get();
        let mut actions = Vec::new();
        self.header(ui, &mut actions);
        kit::rule(ui);
        self.refresh_mentions(ui.ctx(), now, cards.attached());
        // The composer keeps its room at the bottom; the transcript takes
        // the rest and follows the newest item.
        let composer = composer_height(ui, !self.files.is_empty())
            + self.slash_height()
            + self.mention_height();
        let transcript = (ui.available_height() - composer).max(t.control_height * 2.0);
        egui::ScrollArea::vertical()
            .id_salt("assistant-transcript")
            .max_height(transcript)
            .min_scrolled_height(transcript)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.conversation.items.is_empty() {
                    kit::note(
                        ui,
                        "Ask about the open design or ask for a change. Each message is one undo step.",
                    );
                }
                self.transcript(ui, now, cards, &mut actions);
            });
        kit::rule(ui);
        self.slash_menu(ui, cards, &mut actions);
        self.mention_menu(ui);
        self.composer(ui, now, &mut actions);
        actions
    }

    fn header(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let t = Tokens::get();
        ui.horizontal(|ui| {
            ui.set_min_height(t.control_height);
            state_dot(ui, &self.conversation.state);
            ui.label(
                egui::RichText::new("Assistant")
                    .font(type_face::semibold(t.font_heading))
                    .color(t.text),
            );
            kit::note(ui, self.conversation.state.label());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, "close", "Close the assistant").clicked() {
                    actions.push(Action::Close);
                }
                let fold = match self.dock {
                    Dock::Right => "chevron_right",
                    Dock::Left => "chevron",
                };
                if icon_button(ui, fold, "Fold the assistant").clicked() {
                    actions.push(Action::Fold(true));
                }
                let (other, tip) = match self.dock {
                    Dock::Right => (Dock::Left, "Dock the assistant left"),
                    Dock::Left => (Dock::Right, "Dock the assistant right"),
                };
                if icon_button(ui, "display", tip).clicked() {
                    actions.push(Action::Dock(other));
                }
            });
        });
        match &self.conversation.state {
            State::NotSetUp(why) => {
                // One way on: the button that opens where the key goes.
                kit::note(ui, why);
                kit::actions(ui, |ui| {
                    if kit::primary(ui, CONNECT, true)
                        .on_hover_text("Choose a model and paste its API key in Settings › AI")
                        .clicked()
                    {
                        actions.push(Action::ConnectModel);
                    }
                });
            }
            State::Error(why) => {
                kit::refusal_wrapped(ui, why);
            }
            _ => {}
        }
    }

    fn transcript(
        &mut self,
        ui: &mut Ui,
        now: &Now,
        cards: &mut crate::workflow_cards::WorkflowCards,
        actions: &mut Vec<Action>,
    ) {
        let t = Tokens::get();
        let mut toggled = None;
        let asking = self.asking.as_ref().map(|a| a.card.clone());
        let last = (!self.conversation.state.busy())
            .then(|| self.conversation.last_summary())
            .flatten();
        for (index, item) in self.conversation.items.iter_mut().enumerate() {
            ui.add_space(t.space_6);
            let open = self.expanded.contains(&index);
            ui.push_id(("assistant-item", index), |ui| match item {
                Item::You {
                    text,
                    context,
                    attachments,
                } => you(ui, text, context, attachments),
                Item::Reply(text) => markdown(ui, text),
                Item::Reasoning(text) => {
                    let first = text.lines().next().unwrap_or_default();
                    if disclosure(ui, open, "Reasoning", first, None).clicked() {
                        toggled = Some(index);
                    }
                    if open {
                        detail(ui, text);
                    }
                }
                Item::Call(row) => {
                    let (word, level) = match &row.state {
                        RowState::Writing => ("writing".to_owned(), None),
                        RowState::Running => ("running".to_owned(), None),
                        RowState::Done => ("done".to_owned(), Some(kit::severity::Level::Ok)),
                        RowState::Refused => {
                            ("refused".to_owned(), Some(kit::severity::Level::Warning))
                        }
                        RowState::NotRun(why) => {
                            (format!("not run: {why}"), Some(kit::severity::Level::ToDo))
                        }
                    };
                    if disclosure(ui, open, &row.line, &word, level).clicked() {
                        toggled = Some(index);
                    }
                    if open {
                        kit::note(ui, "Input");
                        detail(ui, &row.input);
                        if !row.reply.is_empty() {
                            kit::note(ui, "Reply");
                            detail(ui, &row.reply);
                        }
                    }
                }
                Item::Ask(card) => ask(ui, card, actions),
                Item::Question(card) => {
                    let active = asking.as_deref() == Some(card.card.as_str());
                    if let Some(choice) = questions::show(ui, card, active) {
                        actions.push(Action::Question {
                            card: card.card.clone(),
                            choice,
                        });
                    }
                }
                Item::Summary(card) => summary(ui, card, now, last == Some(card.id), actions),
                Item::Workflow(key) => {
                    if let Some(action) = cards.show_key(ui, *key) {
                        actions.push(Action::Card(action));
                    }
                }
                Item::Condensed { line, record } | Item::Nudged { line, note: record } => {
                    kit::rule(ui);
                    if disclosure(ui, open, line, "", None).clicked() {
                        toggled = Some(index);
                    }
                    if open {
                        detail(ui, record);
                    }
                }
            });
        }
        if let Some(index) = toggled
            && !self.expanded.remove(&index)
        {
            self.expanded.insert(index);
        }
    }

    /// The `/` menu while the composer starts with `/`: the workflows by
    /// the words after it; choosing one shows its card and clears the
    /// composer.
    fn slash_menu(
        &mut self,
        ui: &mut Ui,
        cards: &mut crate::workflow_cards::WorkflowCards,
        actions: &mut Vec<Action>,
    ) {
        if !self.slash() {
            return;
        }
        let query = self
            .composer
            .strip_prefix('/')
            .unwrap_or_default()
            .trim()
            .to_owned();
        let focused = ui.memory(|m| m.has_focus(Self::composer_id()));
        let chosen = egui::ScrollArea::vertical()
            .id_salt("assistant-slash-menu")
            .max_height(self.slash_height())
            .show(ui, |ui| cards.slash_menu(ui, &query, focused))
            .inner;
        if let Some(id) = chosen {
            self.composer.clear();
            actions.push(Action::Offered(id));
        }
        kit::rule(ui);
    }

    /// Whether the `/` menu shows: the composer starts with `/` and a word
    /// that is not a path.
    fn slash(&self) -> bool {
        self.composer.strip_prefix('/').is_some_and(|rest| {
            !rest
                .split_whitespace()
                .next()
                .is_some_and(|word| word.contains('/'))
        })
    }

    /// The room the `/` menu takes: four rows while it shows.
    fn slash_height(&self) -> f32 {
        if !self.slash() {
            return 0.0;
        }
        let t = Tokens::get();
        4.0 * (t.control_height + t.font_small + t.space_4)
    }

    /// The room the `@` list takes: four rows while it shows.
    fn mention_height(&self) -> f32 {
        if !self.mention_search.visible() {
            return 0.0;
        }
        let t = Tokens::get();
        4.0 * (t.control_height + t.font_small + t.space_4)
    }

    /// The composer's cursor, as a character index (its end when unknown).
    fn cursor(&self, ctx: &egui::Context) -> usize {
        egui::TextEdit::load_state(ctx, Self::composer_id())
            .and_then(|state| state.cursor.char_range())
            .map_or_else(
                || self.composer.chars().count(),
                |range| range.primary.index.into(),
            )
    }

    fn set_cursor(ctx: &egui::Context, at: usize) {
        let id = Self::composer_id();
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(at),
            )));
        state.store(ctx, id);
    }

    /// The `@` list while an `@` is typed before the cursor: the files that
    /// match what follows it, best first; ↑↓ and Enter (or a click) put the
    /// one chosen in the message as a chip. `known`: the files attached in
    /// this session.
    fn refresh_mentions(&mut self, ctx: &egui::Context, now: &Now, known: &[std::path::PathBuf]) {
        let focused = ctx.memory(|m| m.has_focus(Self::composer_id()));
        let typed = mentions::active(&self.composer, self.cursor(ctx));
        let Some((_, query)) = typed.filter(|_| focused) else {
            self.mention_search.clear();
            self.mention = 0;
            return;
        };
        if self.mention_search.refresh(
            mentions::search::Request {
                query,
                revision: now.revision,
                folder: now.folder.clone(),
                known: known.to_vec(),
            },
            ctx,
        ) {
            self.mention = 0;
        }
    }

    fn mention_menu(&mut self, ui: &mut Ui) {
        if !self.mention_search.visible() {
            return;
        }
        let ctx = ui.ctx().clone();
        let Some((start, query)) = mentions::active(&self.composer, self.cursor(&ctx)) else {
            return;
        };
        let height = self.mention_height();
        let ready = self.mention_search.result();
        let ranked = ready
            .and_then(|found| found.as_ref().ok())
            .map(|found| &found.ranked);
        let nav = ranked
            .filter(|rows| !rows.is_empty())
            .map(|_| chat_cards::MenuNav::take(ui));
        // Keep the composer's place while the search changes from loading
        // to results, including empty or truncated results.
        let chosen = egui::ScrollArea::vertical()
            .id_salt("assistant-mention-menu")
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| match ready {
                None => {
                    kit::note(ui, "Searching files…");
                    None
                }
                Some(Err(error)) => {
                    kit::note(ui, error);
                    None
                }
                Some(Ok(found)) => {
                    if found.limited {
                        kit::note(
                            ui,
                            "Folder search limited; type a path to attach another file",
                        );
                    }
                    if found.ranked.is_empty() {
                        kit::note(ui, "No file matches");
                    }
                    let rows: Vec<chat_cards::MenuRow<'_>> = found
                        .ranked
                        .iter()
                        .map(|candidate| chat_cards::MenuRow {
                            title: &candidate.name,
                            summary: &candidate.detail,
                            enabled: true,
                        })
                        .collect();
                    chat_cards::menu(ui, &rows, &mut self.mention, nav)
                }
            })
            .inner;
        kit::rule(ui);
        if let Some(candidate) = chosen
            .and_then(|i| ranked.and_then(|rows| rows.get(i)))
            .cloned()
        {
            let end = start + 1 + query.len();
            let name = format!("@{} ", candidate.name);
            let at = self.composer.get(..start).map_or(0, |b| b.chars().count());
            if end <= self.composer.len() && self.composer.len() + name.len() <= MAX_MESSAGE_BYTES {
                self.composer.replace_range(start..end, &name);
                Self::set_cursor(&ctx, at + name.chars().count());
                self.add_file(candidate.path);
            }
            self.mention_search.clear();
            self.mention = 0;
        }
    }

    fn add_file(&mut self, path: std::path::PathBuf) {
        if !self.files.contains(&path) && self.files.len() < MAX_MESSAGE_FILES {
            self.files.push(path);
        }
    }

    /// Paths typed in the composer that name a file become chips (`@` and
    /// the file's name); with `all`, the one still being typed too. A chip
    /// whose `@` name left the text goes.
    pub fn chip_paths(&mut self, ctx: &egui::Context, all: bool) {
        let found = mentions::typed_paths(&self.composer, all);
        if !found.is_empty() {
            let mut cursor = self.cursor(ctx);
            for (range, path) in found.into_iter().rev() {
                let name = format!("@{}", mentions::file_name(&path));
                let Some(old) = self.composer.get(range.clone()) else {
                    continue;
                };
                let removed = old.chars().count();
                let at = self
                    .composer
                    .get(..range.start)
                    .map_or(0, |b| b.chars().count());
                self.composer.replace_range(range, &name);
                if cursor > at {
                    cursor = (cursor + name.chars().count()).saturating_sub(removed);
                }
                self.add_file(path);
            }
            Self::set_cursor(ctx, cursor);
        }
        let text = &self.composer;
        self.files
            .retain(|p| text.contains(&format!("@{}", mentions::file_name(p))));
    }

    fn composer(&mut self, ui: &mut Ui, now: &Now, actions: &mut Vec<Action>) {
        let t = Tokens::get();
        kit::note(ui, &now.context);
        let id = Self::composer_id();
        // Enter sends, Shift+Enter breaks the line: taken before the field
        // sees the key.
        let focused = ui.memory(|m| m.has_focus(id));
        let busy = self.conversation.state.busy();
        // While a question card waits, a message answers it.
        let answering = self.asking.is_some();
        let can_send = (!busy || answering)
            && !self.composer.trim().is_empty()
            && !matches!(self.conversation.state, State::NotSetUp(_));
        // While the `/` menu shows, Enter is its own (the `@` list took
        // its keys already).
        let slash = self.slash();
        if focused
            && !slash
            && ui.input_mut(|i| {
                !i.modifiers.any() && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            })
            && can_send
        {
            actions.push(Action::Send);
        }
        if !self.files.is_empty() {
            let mut removed = None;
            ui.horizontal_wrapped(|ui| {
                for (index, path) in self.files.iter().enumerate() {
                    if chat_cards::file_chip(ui, &mentions::file_name(path), true) {
                        removed = Some(index);
                    }
                }
            });
            if let Some(index) = removed {
                let path = self.files.remove(index);
                let name = format!("@{}", mentions::file_name(&path));
                if let Some(at) = self.composer.find(&name) {
                    let mut end = at + name.len();
                    if self
                        .composer
                        .get(end..)
                        .is_some_and(|rest| rest.starts_with(' '))
                    {
                        end += 1;
                    }
                    self.composer.replace_range(at..end, "");
                }
            }
        }
        let edit = egui::TextEdit::multiline(&mut self.composer)
            .id(id)
            .hint_text(if answering {
                "Answer the card, or in your own words here"
            } else {
                "Ask about this design… @ adds a file"
            })
            .desired_rows(COMPOSER_ROWS)
            .desired_width(ui.available_width())
            .char_limit(MAX_MESSAGE_BYTES);
        if ui.add(edit).changed() {
            self.chip_paths(ui.ctx(), false);
        }
        ui.add_space(t.space_2);
        kit::actions(ui, |ui| {
            if busy {
                if ui_kit::button(ui, "Stop", Button::Enabled(true)).clicked() {
                    actions.push(Action::Stop);
                }
                if answering && kit::primary(ui, "Send", can_send).clicked() {
                    actions.push(Action::Send);
                }
            } else if kit::primary(ui, "Send", can_send).clicked() {
                actions.push(Action::Send);
            }
            if self.conversation.state == State::Running {
                let usage = &self.conversation.usage;
                kit::note(
                    ui,
                    &usage_line(
                        usage.requests,
                        usage.tokens,
                        usage.seconds,
                        usage.usage_estimated,
                    ),
                );
            }
        });
    }
}

/// The limit card's action.
pub const CONTINUE: &str = "Continue (raise this turn's limit)";
/// What Continue tells the model.
pub const CONTINUE_MESSAGE: &str =
    "Continue where you stopped: the person raised the limit for this.";

/// The height the composer, its chips and its context line take.
fn composer_height(ui: &Ui, chips: bool) -> f32 {
    let t = Tokens::get();
    let line = ui.text_style_height(&egui::TextStyle::Body);
    let chips = if chips { t.control_height } else { 0.0 };
    chips
        + t.font_small * 1.6
        + line * COMPOSER_ROWS as f32
        + t.space_8 * 2.0
        + t.space_4
        + t.control_height
        + t.space_8 * 2.0
}

/// "3 requests · 12 k tokens · 14 s".
pub fn usage_line(requests: u32, tokens: u64, seconds: f64, estimated: bool) -> String {
    let tokens = if tokens >= 10_000 {
        format!("{} k tokens", tokens / 1_000)
    } else {
        format!("{tokens} tokens")
    };
    let estimate = if estimated { " (estimated)" } else { "" };
    let time = if seconds >= 90.0 {
        format!("{:.0} min", seconds / 60.0)
    } else {
        format!("{seconds:.0} s")
    };
    format!(
        "{} · {tokens}{estimate} · {time}",
        kit::counted(requests as usize, "request", "requests")
    )
}

/// The state dot's ink.
fn state_level(state: &State) -> kit::severity::Level {
    use kit::severity::Level;
    match state {
        State::NotSetUp(_) | State::Stopped => Level::ToDo,
        State::Idle => Level::Ok,
        State::Running => Level::Info,
        State::Waiting => Level::Warning,
        State::Error(_) => Level::Error,
    }
}

fn state_dot(ui: &mut Ui, state: &State) {
    let t = Tokens::get();
    let size = t.space_8;
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    ui.painter().circle_filled(
        rect.center(),
        size / 2.0,
        kit::severity::ink(state_level(state)),
    );
    response.on_hover_text(state.label());
}

/// A square icon button with its accessible name and tooltip.
fn icon_button(ui: &mut Ui, icon: &str, label: &str) -> egui::Response {
    let t = Tokens::get();
    let (rect, response) = ui.allocate_exact_size(
        vec2(t.control_height, t.control_height),
        egui::Sense::click(),
    );
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
    }
    sparksmith_ui::icons::paint_on(ui.painter(), rect.shrink(t.space_4), icon, None, 1.0);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

/// A one-line row that expands: its text, and a state at its end.
fn disclosure(
    ui: &mut Ui,
    open: bool,
    text: &str,
    state: &str,
    level: Option<kit::severity::Level>,
) -> egui::Response {
    let t = Tokens::get();
    let width = ui.available_width().max(0.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, t.control_height), egui::Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(0.0, 1.0)), t.radius_sm, t.hover);
    }
    let icon = egui::pos2(rect.left() + t.space_8, rect.center().y);
    kit::disclosure(ui.painter(), icon, open, t.form_dim);
    let mut left = icon.x + t.space_8;
    if let Some(level) = level {
        let glyph = egui::pos2(left + t.space_6, rect.center().y);
        kit::severity_glyph(ui.painter(), glyph, t.font_label, level);
        left = glyph.x + t.space_8;
    }
    let state_galley = kit::text::one_line(
        ui,
        state,
        egui::FontId::proportional(t.font_small),
        t.form_dim,
        (width * 0.4).max(0.0),
    );
    let state_left = rect.right() - t.space_4 - state_galley.size().x;
    let galley = kit::text::one_line(
        ui,
        text,
        egui::FontId::monospace(t.font_small),
        t.text,
        (state_left - t.space_8 - left).max(0.0),
    );
    let cut = galley.elided;
    ui.painter().galley(
        egui::pos2(left, rect.center().y - galley.size().y / 2.0),
        galley,
        t.text,
    );
    ui.painter().galley(
        egui::pos2(state_left, rect.center().y - state_galley.size().y / 2.0),
        state_galley,
        t.form_dim,
    );
    let name = format!("{text}: {state}");
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, &name)
    });
    if cut {
        response.on_hover_text(text)
    } else {
        response
    }
}

/// An expanded row's text: monospace, small, selectable.
fn detail(ui: &mut Ui, text: &str) {
    let t = Tokens::get();
    egui::Frame::NONE
        .fill(t.input_fill())
        .corner_radius(t.radius_sm)
        .inner_margin(egui::Margin::same(t.space_4 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .monospace()
                        .size(t.font_small)
                        .color(t.form_ink),
                )
                .wrap()
                .selectable(true),
            );
        });
}

/// The person's message: the context line, attachment chips and the text.
fn you(ui: &mut Ui, text: &str, context: &str, attachments: &[String]) {
    let t = Tokens::get();
    egui::Frame::NONE
        .fill(t.panel_header)
        .corner_radius(t.radius_lg)
        .inner_margin(egui::Margin::same(t.space_8 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            kit::note(ui, context);
            if !attachments.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for name in attachments {
                        chat_cards::file_chip(ui, name, false);
                    }
                });
            }
            ui.add(
                egui::Label::new(egui::RichText::new(text).size(t.font_body).color(t.text))
                    .wrap()
                    .selectable(true),
            );
        });
}

/// A reply in bounded Markdown.
pub(crate) fn markdown(ui: &mut Ui, text: &str) {
    use markdown::Block;
    let t = Tokens::get();
    let (blocks, cut) = markdown::parse(text);
    for block in blocks {
        match block {
            Block::Heading(level, spans) => {
                let size = if level <= 2 {
                    t.font_heading
                } else {
                    t.font_body
                };
                rich(ui, &spans, size, true, 0.0, None);
            }
            Block::Paragraph(spans) => rich(ui, &spans, t.font_body, false, 0.0, None),
            Block::Item {
                depth,
                marker,
                spans,
            } => rich(
                ui,
                &spans,
                t.font_body,
                false,
                t.space_12 * f32::from(depth),
                Some(&marker),
            ),
            Block::Code(code) => detail(ui, &code),
            Block::Rule => kit::rule(ui),
        }
    }
    if cut {
        kit::note(ui, "The rest of this reply is not shown: it is too long.");
    }
}

/// One block of inline spans, wrapped, with an optional list marker.
fn rich(
    ui: &mut Ui,
    spans: &[markdown::Span],
    size: f32,
    heading: bool,
    indent: f32,
    marker: Option<&str>,
) {
    let t = Tokens::get();
    let mut job = egui::text::LayoutJob::default();
    if let Some(marker) = marker {
        job.append(
            &format!("{marker} "),
            indent,
            egui::TextFormat::simple(egui::FontId::proportional(size), t.text_dim),
        );
    }
    for (i, span) in spans.iter().enumerate() {
        let font = if span.code {
            egui::FontId::monospace(size * 0.92)
        } else if span.bold || heading {
            type_face::semibold(size)
        } else {
            egui::FontId::proportional(size)
        };
        let mut format = egui::TextFormat::simple(font, t.text);
        format.italics = span.italic;
        if span.code {
            format.background = t.input_fill();
        }
        let leading = if marker.is_none() && i == 0 {
            indent
        } else {
            0.0
        };
        job.append(&span.text, leading, format);
    }
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap().selectable(true));
}

/// An Ask card: what the assistant wants to do, drawn with the impact
/// card's ask mode (one look for every question); Allow once, for this
/// session or always, or Deny. Enter allows once and Esc denies while it
/// waits.
fn ask(ui: &mut Ui, card: &AskCard, actions: &mut Vec<Action>) {
    use sparksmith_ui::chat_cards as cards;
    let title = format!("Allow: {}?", card.command);
    if let Some(answer) = card.answer {
        cards::card(ui, &title, "", |ui| cards::note(ui, answer.said()));
        return;
    }
    let lines: Vec<String> = card
        .paths
        .iter()
        .map(|path| format!("Path: {path}"))
        .chain(std::iter::once(card.why.clone()))
        .collect();
    let findings: Vec<cards::FindingRow<'_>> = lines
        .iter()
        .map(|message| cards::FindingRow {
            blocking: false,
            message,
        })
        .collect();
    let view = cards::Impact {
        title: &title,
        gate: cards::Gate::Ask,
        counts: &[],
        findings: &findings,
        rows: &[],
    };
    let id = egui::Id::unique(("assistant-ask", card.id));
    let allow = match cards::impact(ui, id, &view, &mut Vec::new(), true) {
        Some(cards::GateChoice::AllowOnce) => Allow::Once,
        Some(cards::GateChoice::AllowForSession) => Allow::Session,
        Some(cards::GateChoice::AlwaysAllow) => Allow::Always,
        Some(cards::GateChoice::Deny) => Allow::Deny,
        _ => return,
    };
    actions.push(Action::Answer { id: card.id, allow });
}

/// The card's title: how the turn ended and how many undo steps its own
/// changes are (more than one when someone else's edit came between them).
pub fn summary_title(card: &TurnSummary) -> String {
    let changed = card.after != card.before;
    let steps = match (changed, card.steps > 1) {
        (false, _) => "nothing changed".to_owned(),
        (true, false) => "1 undo step".to_owned(),
        (true, true) => format!("{} undo steps", card.steps),
    };
    let ending = match (&card.limit, &card.end) {
        (Some(_), _) => "Stopped at a limit",
        (None, None) => "Done",
        (None, Some(_)) => "Ended early",
    };
    format!("{ending} · {steps}")
}

/// The card at the end of a turn. A turn stopped at a limit says which,
/// what it did so far and, on the last card, offers Continue.
fn summary(ui: &mut Ui, card: &TurnSummary, now: &Now, last: bool, actions: &mut Vec<Action>) {
    let t = Tokens::get();
    let changed = card.after != card.before;
    // The turn's own changes split around someone else's edit are that
    // many undo steps, with the other edit between them (review, Undo
    // turn): the card says so instead of offering to undo them as one.
    let split = card.steps > 1;
    card_frame(ui, |ui| {
        ui.label(
            egui::RichText::new(summary_title(card))
                .font(type_face::semibold(t.font_body))
                .color(t.text),
        );
        if let Some(limit) = &card.limit {
            kit::note(ui, &format!("{} was reached.", limit.which));
        }
        if let Some(end) = &card.end {
            kit::note(ui, end);
        }
        if card.limit.is_some() {
            kit::note(
                ui,
                if card.changes.is_empty() {
                    "Done so far: nothing in the design."
                } else {
                    "Done so far:"
                },
            );
        }
        for line in &card.changes {
            kit::note(ui, line);
        }
        if !card.others.is_empty() {
            kit::note(ui, "Changed outside this turn:");
            for line in &card.others {
                kit::note(ui, line);
            }
        }
        if card.issues_new > 0 || card.issues_resolved > 0 {
            kit::note(
                ui,
                &format!(
                    "Issues: {} new, {} resolved",
                    card.issues_new, card.issues_resolved
                ),
            );
            for message in &card.new_issues {
                kit::note(ui, message);
            }
        }
        kit::note(
            ui,
            &usage_line(
                card.usage.requests,
                card.usage.tokens,
                card.usage.seconds,
                card.usage.usage_estimated,
            ),
        );
        for line in &card.spend {
            kit::note(ui, line);
        }
        if split && changed {
            kit::note(
                ui,
                "You edited between the assistant's changes, so Undo turn is off. Undo steps back one change at a time, yours included.",
            );
        }
        let undoable = changed
            && !split
            && now.revision == card.after
            && now.undo_label.as_deref() == Some(card.label.as_str());
        kit::actions(ui, |ui| {
            if ui_kit::button(ui, "Undo turn", Button::Enabled(undoable)).clicked() {
                actions.push(Action::UndoTurn(card.id));
            }
            if ui_kit::button(ui, "Show", Button::Enabled(!card.show.is_empty())).clicked() {
                actions.push(Action::Show(card.id));
            }
            if let Some(limit) = &card.limit
                && ui_kit::button(ui, CONTINUE, Button::Enabled(last && !limit.continued)).clicked()
            {
                actions.push(Action::Continue(card.id));
            }
        });
    });
}

fn card_frame(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(t.radius_lg)
        .inner_margin(egui::Margin::same(t.space_8 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usage_line_counts_requests_tokens_and_time() {
        assert_eq!(
            usage_line(1, 950, 4.2, true),
            "1 request · 950 tokens (estimated) · 4 s"
        );
        assert_eq!(
            usage_line(1, 950, 4.2, false),
            "1 request · 950 tokens · 4 s"
        );
        assert_eq!(
            usage_line(3, 61_400, 360.0, false),
            "3 requests · 61 k tokens · 6 min"
        );
    }

    #[test]
    fn the_toggle_opens_unfolded_and_closes_what_shows() {
        let mut a = Assistant::default();
        a.toggle();
        assert!(a.shown());
        a.folded = true;
        a.toggle();
        assert!(a.shown(), "a folded column unfolds");
        a.toggle();
        assert!(!a.open);
    }
}

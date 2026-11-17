//! The chat's workflow cards (assistant slices C2, W2, `docs/assistant-panel.md`
//! §3, §4): the compact confirmation a model, a signal, the `/` menu or the
//! palette puts in the conversation, its inputs filled from the
//! conversation; a running workflow's block; and the cards of its steps,
//! drawn with `sparksmith_ui::chat_cards` from the runner's
//! [`ControlCard`]s, answered with [`ControlAnswer`]s.
//!
//! This is the thin adapter between the workflow contract and the widgets:
//! it keeps each card's edits, turns what the person typed into the values
//! an input takes (lengths in nanometres from `0.25mm` or `10mil`, angles in
//! millidegrees), and reports what the person chose as a [`CardAction`] for
//! the host (the chat, the runner) to carry out. It holds no workflow
//! logic: which inputs, cards and gates there are is the workflow's data.
//!
//! **Filled from the conversation.** The files the person attached (`@`,
//! a typed path, a drop, a pick) are kept here; a workflow's start inputs
//! that take their type are filled from the latest of them
//! ([`workflow::Workflow::fill_files`]), and the model fills the rest in
//! its offer. The confirmation shows each filled input as a value to change
//! in place and an inline field only for a required input nothing filled;
//! Enter accepts and continues.
use serde_json::{Map, Value};
use sparksmith_assist::workflow::control::{ControlAnswer, ControlCard, Quantity};
use sparksmith_assist::workflow::event::{OfferedBy, RunEnd, WorkflowEvent};
use sparksmith_assist::workflow::{self, Listed, Offer, Signal, Workflow, offer};
use sparksmith_core::units::{LengthUnit, Nm, format_number, parse_length};
use sparksmith_ui::chat_cards as cards;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

/// The id prefix of a palette row that offers the workflow named after it.
pub(crate) const PALETTE_PREFIX: &str = "assistant.workflow:";
/// The most cards a conversation keeps; the oldest answered go first.
const MAX_ITEMS: usize = 256;
/// The most bytes of one document a datasheet field takes (the runner's
/// own bound on attached datasheets).
const MAX_FILE_BYTES: u64 = sparksmith_assist::MAX_DATASHEET_BYTES;
/// The most files the conversation keeps as attached, the oldest going
/// first.
const MAX_ATTACHED: usize = 64;

/// What the person chose on a card, for the host to carry out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CardAction {
    /// Accept and continue: start the workflow with these inputs.
    Start {
        card: String,
        workflow: String,
        inputs: Map<String, Value>,
    },
    /// Just chat: the model is told and the conversation goes on.
    JustChat { card: String },
    /// The person's answer to the cards a run's step waits on.
    Answer {
        run: String,
        step: String,
        answer: ControlAnswer,
    },
    /// A result card's button: a `show` call (Open in editor).
    Show { call: String, params: Value },
    /// Choose…: the host's file picker for this card's documents.
    PickFiles { card: String },
    /// Stop: the run ends at the next step boundary; what completed stays.
    Stop { run: String },
}

/// A document a datasheet field holds: read off the UI thread for its
/// page count and size, and copied into the conversation's folder.
#[derive(Clone, Debug, PartialEq)]
struct Document {
    given: PathBuf,
    title: String,
    /// "38 pages · 1.2 MB", "Reading…" or why it cannot be used.
    detail: String,
    /// Where the run reads it: the copy in the conversation's folder.
    stored: Option<PathBuf>,
    failed: bool,
}

/// One input of a confirmation, with what the person typed.
struct FieldEdit {
    card: ControlCard,
    text: String,
    chosen: Vec<String>,
    documents: Vec<Document>,
    error: Option<String>,
    /// Whether its row shows: it holds a value, or it is required. An
    /// optional input nothing filled stays out of the way.
    shown: bool,
    /// The person changed it: a later offer leaves it as it is.
    edited: bool,
}

enum State {
    Waiting,
    Answered(&'static str),
}

enum Item {
    /// A workflow offered, its start inputs filled from the conversation.
    Offer {
        card: String,
        offer: Offer,
        fields: Vec<FieldEdit>,
        state: State,
    },
    Run {
        run: String,
        title: String,
        version: String,
        steps: Vec<(String, String, cards::StepState, String)>,
        meter: Option<[(u64, u64); 3]>,
        ended: Option<RunEnd>,
    },
    Inputs {
        card: String,
        run: String,
        step: String,
        title: String,
        fields: Vec<FieldEdit>,
        state: State,
    },
    Gate {
        card: String,
        run: String,
        step: String,
        title: String,
        counts: Vec<(String, u64)>,
        findings: Vec<(bool, String)>,
        rows: Vec<(String, String)>,
        leave_out: Vec<String>,
        state: State,
    },
    Notify {
        card: String,
        run: String,
        step: String,
        field: FieldEdit,
        state: State,
    },
    Result {
        run: String,
        card: ControlCard,
        undone: bool,
    },
}

impl Item {
    fn waiting(&self) -> bool {
        matches!(
            self,
            Self::Offer {
                state: State::Waiting,
                ..
            } | Self::Inputs {
                state: State::Waiting,
                ..
            } | Self::Gate {
                state: State::Waiting,
                ..
            } | Self::Notify {
                state: State::Waiting,
                ..
            }
        )
    }
}

/// A document read in the background.
struct Probe {
    given: PathBuf,
    receiver: Receiver<Result<Document, String>>,
}

/// The conversation's workflow cards.
#[derive(Default)]
pub(crate) struct WorkflowCards {
    /// The host's workflows, read when first needed.
    listed: Option<Vec<Listed>>,
    items: Vec<Item>,
    next: u64,
    /// The conversation's folder documents are copied into (in the chat's
    /// grant); without one they are read where they are.
    folder: Option<PathBuf>,
    probes: Vec<Probe>,
    /// Documents read so far, by the path given.
    documents: Vec<Document>,
    /// The `/` menu's highlighted row.
    menu: usize,
    /// The workspace shown when last told, for `offer_when` workspace
    /// signals.
    workspace: Option<&'static str>,
    /// A card was added that the person should see: the host opens the
    /// chat.
    pub open_requested: bool,
    /// Each item's stable key, beside it.
    keys: Vec<u64>,
    next_key: u64,
    /// Keys added since [`Self::take_new`].
    fresh: Vec<u64>,
    /// What the person chose that the chat's run host carries out (W2):
    /// Accept and continue, Just chat, a run's answers, Stop.
    pub outbox: Vec<CardAction>,
    /// Files attached in this session (`@`, typed paths, drops, picks), the
    /// latest last: the composer's `@` and a datasheet value's ⌄ offer them.
    attached: Vec<PathBuf>,
    /// The latest message's (or drop's) files and their media types: what
    /// an offer's files inputs are filled from.
    latest: Vec<(String, String)>,
    /// The open project's folder, and its files once listed.
    project: Option<PathBuf>,
    project_files: crate::assistant::mentions::search::Search,
}

impl WorkflowCards {
    /// Cards over `listed` workflows, documents copied into `folder`.
    #[cfg(test)]
    pub(crate) fn with(listed: Vec<Listed>, folder: Option<PathBuf>) -> Self {
        Self {
            listed: Some(listed),
            folder,
            ..Self::default()
        }
    }

    /// The host's workflows: the shipped library's and the person's own.
    pub(crate) fn listed(&mut self) -> &[Listed] {
        self.listed.get_or_insert_with(workflow::load::host)
    }

    fn card_id(&mut self) -> String {
        self.next += 1;
        format!("card-{}", self.next)
    }

    fn push(&mut self, item: Item) {
        if self.items.len() >= MAX_ITEMS
            && let Some(old) = self.items.iter().position(|i| !i.waiting())
        {
            self.items.remove(old);
            self.keys.remove(old);
        }
        self.items.push(item);
        self.next_key += 1;
        self.keys.push(self.next_key);
        self.fresh.push(self.next_key);
        self.open_requested = true;
    }

    /// The keys of the cards added since last asked, in order: the chat
    /// puts each in its transcript ([`Self::show_key`] draws it).
    pub(crate) fn take_new(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.fresh)
    }

    /// Draw the card `key` where the transcript holds it; answers what the
    /// person chose. A card dropped from the conversation draws nothing.
    pub(crate) fn show_key(&mut self, ui: &mut egui::Ui, key: u64) -> Option<CardAction> {
        let index = self.keys.iter().position(|k| *k == key)?;
        let active = self.items.iter().rposition(Item::waiting) == Some(index);
        self.show_item(ui, index, active)
    }

    /// Take in what happened since the last frame (documents read).
    pub(crate) fn frame(&mut self) {
        self.poll();
    }

    /// Put a checked workflow card in the conversation (`assist.offer` from
    /// the window, a signal, the `/` menu, the palette), its files inputs
    /// filled from the latest attached files; answers its id. While a card
    /// of the same workflow waits, the offer fills that one instead: each
    /// value it gives replaces one the person has not changed.
    pub(crate) fn show_offer(&mut self, offer: Offer) -> String {
        let offer = self.filled(offer);
        let fields = self.offer_fields(&offer);
        let waiting = self.items.iter_mut().rev().find_map(|item| match item {
            Item::Offer {
                card,
                offer: old,
                fields: own,
                state: State::Waiting,
            } if old.workflow == offer.workflow => Some((card.clone(), old, own)),
            _ => None,
        });
        if let Some((card, old, own)) = waiting {
            for field in fields {
                let name = input_name(&field.card);
                if !offer.inputs.contains_key(&name) {
                    continue;
                }
                if let Some(mine) = own.iter_mut().find(|f| input_name(&f.card) == name)
                    && !mine.edited
                {
                    *mine = field;
                }
            }
            old.inputs.extend(offer.inputs);
            old.missing = offer.missing;
            if !offer.why.is_empty() {
                old.why = offer.why;
            }
            self.open_requested = true;
            return card;
        }
        let card = self.card_id();
        self.push(Item::Offer {
            card: card.clone(),
            offer,
            fields,
            state: State::Waiting,
        });
        card
    }

    /// Offer workflow `id` with `inputs`, checked against the host's
    /// workflows; the reason when it cannot be offered.
    pub(crate) fn offer(
        &mut self,
        id: &str,
        inputs: Map<String, Value>,
        why: &str,
    ) -> Result<String, String> {
        let offer = Offer::check(self.listed(), id, inputs, why)?;
        Ok(self.show_offer(offer))
    }

    /// The workflow `id` as the host lists it.
    fn workflow(&mut self, id: &str) -> Option<Workflow> {
        self.listed()
            .iter()
            .filter_map(|l| l.loaded.as_ref())
            .find(|l| l.workflow.id == id)
            .map(|l| l.workflow.clone())
    }

    /// `offer` with its files inputs that nothing gave filled from the
    /// latest attached files, checked again.
    fn filled(&mut self, offer: Offer) -> Offer {
        let Some(workflow) = self.workflow(&offer.workflow) else {
            return offer;
        };
        let mut inputs = offer.inputs.clone();
        workflow.fill_files(&mut inputs, &self.latest);
        if inputs.len() == offer.inputs.len() {
            return offer;
        }
        Offer::check(self.listed(), &offer.workflow, inputs, &offer.why).unwrap_or(offer)
    }

    /// The confirmation's fields: the workflow's start inputs with what the
    /// offer gives.
    fn offer_fields(&mut self, offer: &Offer) -> Vec<FieldEdit> {
        let Some(workflow) = self.workflow(&offer.workflow) else {
            return Vec::new();
        };
        let cards: Vec<ControlCard> = workflow
            .start_inputs()
            .into_iter()
            .filter_map(|name| {
                let input = workflow.input(name)?;
                Some(ControlCard::for_input(
                    name,
                    input,
                    offer.inputs.get(name),
                    Vec::new(),
                ))
            })
            .collect();
        cards
            .into_iter()
            .map(|card| {
                let mut field = FieldEdit::new(card);
                // A file given by path is read like a dropped one: its page
                // count, and its copy in the conversation's folder.
                for document in &mut field.documents {
                    *document = self.document(document.given.clone());
                }
                field
            })
            .collect()
    }

    /// The document read from `path`, or a placeholder while it is read.
    fn document(&mut self, path: PathBuf) -> Document {
        if let Some(read) = self.documents.iter().find(|d| d.given == path) {
            return read.clone();
        }
        self.probe(path.clone());
        Document {
            title: file_name(&path),
            detail: "Reading…".into(),
            given: path,
            stored: None,
            failed: false,
        }
    }

    /// Files the person attached to a message, dropped or picked: they are
    /// the latest the next offer is filled from, and stay attached for the
    /// composer's `@` and a datasheet value's ⌄. A PDF is read for its
    /// pages and copied into the conversation's folder.
    pub(crate) fn attach(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        self.latest.clear();
        for path in paths {
            if let Some(media) = media_type_of(path) {
                self.latest
                    .push((path.display().to_string(), media.to_owned()));
                if media == offer::PDF {
                    self.probe(path.clone());
                }
            }
            self.attached.retain(|p| p != path);
            self.attached.push(path.clone());
        }
        let over = self.attached.len().saturating_sub(MAX_ATTACHED);
        self.attached.drain(..over);
    }

    /// The files attached in this session, the latest last.
    pub(crate) fn attached(&self) -> &[PathBuf] {
        &self.attached
    }

    /// The open project's folder, for the files a datasheet value offers.
    pub(crate) fn set_project(&mut self, folder: Option<PathBuf>) {
        if self.project != folder {
            self.project = folder;
            self.project_files.clear();
        }
    }

    /// The files a datasheet value may be: the PDFs attached (the latest
    /// first), then those in the project's folder. Listed by name only;
    /// the one chosen is read for its type like any other.
    fn file_candidates(&mut self, ui: &mut egui::Ui) -> Vec<PathBuf> {
        let pdf = |p: &PathBuf| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        let mut out: Vec<PathBuf> = self
            .attached
            .iter()
            .rev()
            .filter(|p| pdf(p))
            .cloned()
            .collect();
        if self.project.is_some() {
            self.project_files.refresh(
                crate::assistant::mentions::search::Request {
                    query: String::new(),
                    revision: 0,
                    folder: self.project.clone(),
                    known: Vec::new(),
                },
                ui.ctx(),
            );
            let status = match self.project_files.result() {
                None => "Searching project files…",
                Some(Err(error)) => error,
                Some(Ok(found)) => {
                    for candidate in &found.candidates {
                        if pdf(&candidate.path) && !out.contains(&candidate.path) {
                            out.push(candidate.path.clone());
                        }
                    }
                    if found.limited {
                        "Folder search limited; attach another file by its path"
                    } else {
                        ""
                    }
                }
            };
            // Reserve one kit row, including when ready: a background result
            // must not move the card's field while the person is editing it.
            let t = sparksmith_ui::theme::Tokens::get();
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), t.control_height),
                egui::Sense::hover(),
            );
            let cut = crate::kit::text::paint_line(
                ui,
                rect,
                status,
                egui::FontId::proportional(t.font_small),
                t.form_dim,
            );
            crate::kit::text::hover(response, status, "", cut);
        }
        out
    }

    /// The event the chat sends the model for card `card`, when it is a
    /// workflow card.
    pub(crate) fn offered_event(&self, card: &str, by: OfferedBy) -> Option<WorkflowEvent> {
        self.items.iter().find_map(|item| match item {
            Item::Offer { card: c, offer, .. } if c == card => Some(offer.event(c.clone(), by)),
            _ => None,
        })
    }

    /// A structured signal (`offer_when`): offer each workflow it names that
    /// is not already waiting for an answer. Answers the cards added.
    pub(crate) fn signal(&mut self, signal: &Signal, why: &str) -> Vec<String> {
        let ids: Vec<String> = offer::offered_on(self.listed(), signal)
            .into_iter()
            .map(|l| l.workflow.id.clone())
            .collect();
        let mut added = Vec::new();
        for id in ids {
            if !self.waiting_offer(&id)
                && let Ok(card) = self.offer(&id, Map::new(), why)
            {
                added.push(card);
            }
        }
        added
    }

    /// How many workflow cards the conversation holds.
    #[cfg(test)]
    pub(crate) fn offers(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Offer { .. }))
            .count()
    }

    /// The window shows `workspace` (`component`, `schematic`, `pcb`):
    /// on a change, the workflows whose `offer_when` names it are offered.
    pub(crate) fn workspace(&mut self, workspace: &'static str) -> Vec<String> {
        if self.workspace.replace(workspace) == Some(workspace) {
            return Vec::new();
        }
        self.signal(&Signal::Workspace(workspace.to_owned()), "")
    }

    fn waiting_offer(&self, id: &str) -> bool {
        self.items.iter().any(|item| {
            matches!(item, Item::Offer { offer, state: State::Waiting, .. } if offer.workflow == id)
        })
    }

    /// Files dropped on the chat. They are attached to the conversation;
    /// the waiting confirmation with a datasheet value takes them;
    /// otherwise each file's type is a signal, and a workflow that names it
    /// is offered with the files as its input. Answers a sentence for each
    /// file nothing took.
    pub(crate) fn drop_files(&mut self, paths: Vec<PathBuf>) -> Vec<String> {
        self.attach(&paths);
        if let Some(index) = self.datasheet_field() {
            for path in paths {
                self.add_document(index, path, false);
            }
            return Vec::new();
        }
        let mut by_type: Vec<(&'static str, Vec<PathBuf>)> = Vec::new();
        let mut refused = Vec::new();
        for path in paths {
            match media_type_of(&path) {
                Some(media) => match by_type.iter_mut().find(|(m, _)| *m == media) {
                    Some((_, list)) => list.push(path),
                    None => by_type.push((media, vec![path])),
                },
                None => refused.push(format!("No workflow takes {}", file_name(&path))),
            }
        }
        for (media, files) in by_type {
            let signal = Signal::Attachment(media.to_owned());
            let found: Vec<(String, String, usize)> = offer::offered_on(self.listed(), &signal)
                .into_iter()
                .filter_map(|l| {
                    let input = l.workflow.input_for_attachment(media)?;
                    let max = l
                        .workflow
                        .input(input)
                        .and_then(|i| i.max.as_ref())
                        .and_then(serde_json::Number::as_u64)
                        .unwrap_or(1);
                    Some((
                        l.workflow.id.clone(),
                        input.to_owned(),
                        usize::try_from(max).unwrap_or(1),
                    ))
                })
                .collect();
            if found.is_empty() {
                refused.extend(
                    files
                        .iter()
                        .map(|f| format!("No workflow takes {}", file_name(f))),
                );
                continue;
            }
            for path in &files {
                self.probe(path.clone());
            }
            let why = if files.len() == 1 {
                format!("You dropped {}.", file_name(&files[0]))
            } else {
                format!("You dropped {} files.", files.len())
            };
            for (id, input, max) in found {
                let given: Vec<Value> = files
                    .iter()
                    .take(max)
                    .map(|p| Value::String(p.display().to_string()))
                    .collect();
                let mut inputs = Map::new();
                inputs.insert(input, Value::Array(given));
                if let Err(why) = self.offer(&id, inputs, &why) {
                    refused.push(why);
                }
            }
        }
        refused
    }

    /// Files the host's picker returned for card `card`: they are attached,
    /// and its datasheet value takes them (one replaces a single file).
    pub(crate) fn picked(&mut self, card: &str, paths: Vec<PathBuf>) {
        self.attach(&paths);
        let index = self.items.iter().position(|item| match item {
            Item::Inputs {
                card: c,
                state: State::Waiting,
                ..
            }
            | Item::Offer {
                card: c,
                state: State::Waiting,
                ..
            } => c == card,
            _ => false,
        });
        if let Some(index) = index {
            for path in paths {
                self.add_document(index, path, true);
            }
        }
    }

    /// The fields of the item at `index` that take input.
    fn fields_mut(&mut self, index: usize) -> Option<&mut Vec<FieldEdit>> {
        match self.items.get_mut(index)? {
            Item::Inputs { fields, .. } | Item::Offer { fields, .. } => Some(fields),
            _ => None,
        }
    }

    /// The waiting confirmation with a datasheet value that takes more.
    fn datasheet_field(&self) -> Option<usize> {
        self.items.iter().rposition(|item| match item {
            Item::Inputs {
                fields,
                state: State::Waiting,
                ..
            }
            | Item::Offer {
                fields,
                state: State::Waiting,
                ..
            } => fields.iter().any(|f| matches!(f.card, ControlCard::Datasheet { max, .. } if (f.documents.len() as u64) < max)),
            _ => false,
        })
    }

    /// Put `path` in the datasheet value of the item at `index`: added
    /// while it takes more, or, with `replace`, in place of a single file.
    fn add_document(&mut self, index: usize, path: PathBuf, replace: bool) {
        let document = self.document(path);
        let Some(fields) = self.fields_mut(index) else {
            return;
        };
        let Some(field) = fields
            .iter_mut()
            .find(|f| matches!(f.card, ControlCard::Datasheet { .. }))
        else {
            return;
        };
        let ControlCard::Datasheet { max, .. } = field.card else {
            return;
        };
        if field.documents.iter().any(|d| d.given == document.given) {
            return;
        }
        if replace && max == 1 {
            field.documents.clear();
        }
        if (field.documents.len() as u64) < max {
            field.documents.push(document);
            field.error = None;
            field.edited = true;
            field.shown = true;
        }
    }

    /// The person's ⌄ choice on the datasheet value of the item at
    /// `index`: a single file is replaced, several toggle.
    fn toggle_document(&mut self, index: usize, path: PathBuf) {
        let had = self.fields_mut(index).is_some_and(|fields| {
            fields.iter_mut().any(|f| {
                let before = f.documents.len();
                if matches!(f.card, ControlCard::Datasheet { max, .. } if max > 1) {
                    f.documents.retain(|d| d.given != path);
                }
                f.edited |= f.documents.len() != before;
                f.documents.len() != before
            })
        });
        if !had {
            self.add_document(index, path, true);
        }
    }

    /// Read `path` off the UI thread: its kind, page count and size, then
    /// a copy into the conversation's folder.
    fn probe(&mut self, path: PathBuf) {
        if self.documents.iter().any(|d| d.given == path)
            || self.probes.iter().any(|p| p.given == path)
        {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let folder = self.folder.clone();
        let given = path.clone();
        std::thread::spawn(move || {
            let _ = sender.send(read_document(&given, folder.as_deref()));
        });
        self.probes.push(Probe {
            given: path,
            receiver,
        });
    }

    /// Take in the documents read since the last frame.
    fn poll(&mut self) {
        let mut done = Vec::new();
        self.probes.retain(|probe| match probe.receiver.try_recv() {
            Ok(result) => {
                done.push(result.unwrap_or_else(|why| Document {
                    title: file_name(&probe.given),
                    detail: why,
                    given: probe.given.clone(),
                    stored: None,
                    failed: true,
                }));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        for document in done {
            for item in &mut self.items {
                if let Item::Inputs { fields, .. } | Item::Offer { fields, .. } = item {
                    for field in fields.iter_mut() {
                        for d in &mut field.documents {
                            if d.given == document.given {
                                *d = document.clone();
                            }
                        }
                    }
                }
            }
            self.documents.push(document);
        }
    }

    /// A workflow event from the runner: the run block, its steps and its
    /// cards.
    pub(crate) fn event(&mut self, event: &WorkflowEvent) {
        match event {
            WorkflowEvent::Offered { .. } | WorkflowEvent::Declined { .. } => {}
            WorkflowEvent::Started {
                run,
                version,
                label,
                steps,
                ..
            } => self.push(Item::Run {
                run: run.clone(),
                title: label.clone(),
                version: version.clone(),
                steps: steps
                    .iter()
                    .map(|s| {
                        (
                            s.id.clone(),
                            s.title.clone(),
                            cards::StepState::Waiting,
                            String::new(),
                        )
                    })
                    .collect(),
                meter: None,
                ended: None,
            }),
            WorkflowEvent::Step { run, step, .. } => {
                self.step_row(run, step, cards::StepState::Running, None);
            }
            WorkflowEvent::StepDone { run, step, summary } => {
                self.step_row(run, step, cards::StepState::Done, Some(summary.clone()));
            }
            WorkflowEvent::Retry {
                run,
                step,
                attempt,
                of,
                ..
            } => {
                let note = format!("again, {attempt} of {of}");
                self.step_row(run, step, cards::StepState::Running, Some(note));
            }
            WorkflowEvent::Meter {
                run,
                requests,
                output_tokens,
                minutes,
            } => {
                if let Some(Item::Run { meter, .. }) = self.run_mut(run) {
                    *meter = Some([
                        (requests.used, requests.limit),
                        (output_tokens.used, output_tokens.limit),
                        (minutes.used, minutes.limit),
                    ]);
                }
            }
            WorkflowEvent::Ended { run, end } => {
                if let Some(Item::Run { ended, steps, .. }) = self.run_mut(run) {
                    *ended = Some(end.clone());
                    if let RunEnd::Failed { step, .. } = end
                        && let Some(row) = steps.iter_mut().find(|r| &r.0 == step)
                    {
                        row.2 = cards::StepState::Failed;
                    }
                }
                // What still waits on this run is answered by its end.
                for item in &mut self.items {
                    match item {
                        Item::Inputs { run: r, state, .. }
                        | Item::Gate { run: r, state, .. }
                        | Item::Notify { run: r, state, .. }
                            if r == run && matches!(state, State::Waiting) =>
                        {
                            *state = State::Answered("Ended");
                        }
                        _ => {}
                    }
                }
            }
            WorkflowEvent::Card { run, step, card } => self.card(run, step, card),
        }
    }

    fn run_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items
            .iter_mut()
            .rev()
            .find(|i| matches!(i, Item::Run { run, .. } if run == id))
    }

    fn step_row(&mut self, run: &str, step: &str, to: cards::StepState, summary: Option<String>) {
        if let Some(Item::Run { steps, .. }) = self.run_mut(run)
            && let Some(row) = steps.iter_mut().find(|r| r.0 == step)
        {
            row.2 = to;
            if let Some(summary) = summary {
                row.3 = summary;
            }
        }
    }

    fn card(&mut self, run: &str, step: &str, card: &ControlCard) {
        match card {
            ControlCard::Progress { .. } => {}
            ControlCard::Meter {
                requests,
                output_tokens,
                minutes,
                ..
            } => {
                if let Some(Item::Run { meter, .. }) = self.run_mut(run) {
                    *meter = Some([
                        (requests.used, requests.limit),
                        (output_tokens.used, output_tokens.limit),
                        (minutes.used, minutes.limit),
                    ]);
                }
            }
            ControlCard::Impact {
                title,
                counts,
                findings,
                rows,
            } => {
                let id = self.card_id();
                self.push(Item::Gate {
                    card: id,
                    run: run.to_owned(),
                    step: step.to_owned(),
                    title: title.clone(),
                    counts: counts.iter().map(|c| (c.label.clone(), c.n)).collect(),
                    findings: findings
                        .iter()
                        .map(|f| (f.blocking, f.message.clone()))
                        .collect(),
                    rows: rows
                        .iter()
                        .map(|r| (r.id.clone(), r.text.clone()))
                        .collect(),
                    leave_out: Vec::new(),
                    state: State::Waiting,
                });
            }
            ControlCard::Result { .. } => self.push(Item::Result {
                run: run.to_owned(),
                card: card.clone(),
                undone: false,
            }),
            // A review step's notify card names the step as its field.
            ControlCard::Choice { field, .. } if field.input == step => {
                let id = self.card_id();
                self.push(Item::Notify {
                    card: id,
                    run: run.to_owned(),
                    step: step.to_owned(),
                    field: FieldEdit::new(card.clone()),
                    state: State::Waiting,
                });
            }
            _ => {
                let joins = matches!(self.items.last(), Some(Item::Inputs { run: r, step: s, state: State::Waiting, .. }) if r == run && s == step);
                if joins {
                    if let Some(Item::Inputs { fields, .. }) = self.items.last_mut() {
                        fields.push(FieldEdit::new(card.clone()));
                    }
                } else {
                    let id = self.card_id();
                    let title = match self.run_mut(run) {
                        Some(Item::Run { steps, .. }) => steps
                            .iter()
                            .find(|r| r.0 == step)
                            .map_or_else(|| step.to_owned(), |r| r.1.clone()),
                        _ => step.to_owned(),
                    };
                    self.push(Item::Inputs {
                        card: id,
                        run: run.to_owned(),
                        step: step.to_owned(),
                        title,
                        fields: vec![FieldEdit::new(card.clone())],
                        state: State::Waiting,
                    });
                }
            }
        }
    }

    /// The `/` menu over the composer's text after `/`: the workflows whose
    /// title or summary hold every word, those that cannot run here dimmed.
    /// Answers the workflow chosen; its card is shown.
    pub(crate) fn slash_menu(
        &mut self,
        ui: &mut egui::Ui,
        query: &str,
        active: bool,
    ) -> Option<String> {
        let rows = menu_rows(self.listed(), query);
        let shown: Vec<cards::MenuRow<'_>> = rows
            .iter()
            .map(|r| cards::MenuRow {
                title: &r.title,
                summary: &r.detail,
                enabled: r.enabled,
            })
            .collect();
        let chosen = cards::workflow_menu(ui, &shown, &mut self.menu, active)?;
        let id = rows.get(chosen)?.id.clone();
        self.offer(&id, Map::new(), "").ok()?;
        Some(id)
    }

    /// Draw every card in conversation order; answers what the person chose.
    #[cfg(test)]
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) -> Vec<CardAction> {
        self.poll();
        let active = self.items.iter().rposition(Item::waiting);
        let mut actions = Vec::new();
        for index in 0..self.items.len() {
            if let Some(action) = self.show_item(ui, index, active == Some(index)) {
                actions.push(action);
            }
            ui.add_space(sparksmith_ui::theme::Tokens::get().space_8);
        }
        actions
    }

    fn show_item(&mut self, ui: &mut egui::Ui, index: usize, active: bool) -> Option<CardAction> {
        let key = *self.keys.get(index)?;
        let candidates = match self.items.get(index)? {
            Item::Offer {
                state: State::Waiting,
                fields,
                ..
            }
            | Item::Inputs {
                state: State::Waiting,
                fields,
                ..
            } if fields
                .iter()
                .any(|f| matches!(f.card, ControlCard::Datasheet { .. })) =>
            {
                self.file_candidates(ui)
            }
            _ => Vec::new(),
        };
        let mut toggle = None;
        let item = self.items.get_mut(index)?;
        let action = ui
            .push_id(("workflow-card", key), |ui| match item {
                Item::Offer {
                    card,
                    offer,
                    fields,
                    state,
                } => {
                    if let State::Answered(how) = state {
                        answered(ui, &offer.title, how, fields);
                        return None;
                    }
                    let view = cards::Confirm {
                        title: &offer.title,
                        version: &offer.version,
                        why: &offer.why,
                        decline: "Just chat",
                    };
                    let mut inputs = offer.inputs.clone();
                    let shown =
                        show_compact(ui, card, &view, fields, active, &candidates, &mut toggle);
                    match shown? {
                        Compact::Accept(values) => {
                            *state = State::Answered("Accepted");
                            for field in fields.iter() {
                                inputs.remove(&input_name(&field.card));
                            }
                            inputs.extend(values);
                            Some(CardAction::Start {
                                card: card.clone(),
                                workflow: offer.workflow.clone(),
                                inputs,
                            })
                        }
                        Compact::Decline => {
                            *state = State::Answered("Just chat");
                            Some(CardAction::JustChat { card: card.clone() })
                        }
                        Compact::Pick => Some(CardAction::PickFiles { card: card.clone() }),
                    }
                }
                Item::Run {
                    title,
                    version,
                    steps,
                    meter,
                    ended,
                    run,
                } => {
                    let lines: Vec<cards::StepLine<'_>> = steps
                        .iter()
                        .map(|(_, title, state, summary)| cards::StepLine {
                            title,
                            state: *state,
                            summary,
                        })
                        .collect();
                    let gauges: Vec<cards::GaugeRow<'_>> = meter
                        .iter()
                        .flat_map(|m| {
                            [("requests", m[0]), ("tokens", m[1]), ("min", m[2])]
                                .map(|(unit, (used, limit))| cards::GaugeRow { unit, used, limit })
                        })
                        .collect();
                    let end = ended.as_ref().map(end_line).unwrap_or_default();
                    let view = cards::Run {
                        title,
                        version,
                        steps: &lines,
                        gauges: &gauges,
                        running: ended.is_none(),
                        ended: &end,
                    };
                    cards::run_block(ui, &view).then(|| CardAction::Stop { run: run.clone() })
                }
                Item::Inputs {
                    card,
                    run,
                    step,
                    title,
                    fields,
                    state,
                } => {
                    if let State::Answered(how) = state {
                        answered(ui, title, how, fields);
                        return None;
                    }
                    let view = cards::Confirm {
                        title,
                        version: "",
                        why: "",
                        decline: "Decline",
                    };
                    let (label, answer) = match show_compact(
                        ui,
                        card,
                        &view,
                        fields,
                        active,
                        &candidates,
                        &mut toggle,
                    )? {
                        Compact::Accept(values) => ("Accepted", ControlAnswer::Inputs { values }),
                        Compact::Decline => ("Declined", ControlAnswer::Decline),
                        Compact::Pick => {
                            return Some(CardAction::PickFiles { card: card.clone() });
                        }
                    };
                    *state = State::Answered(label);
                    Some(CardAction::Answer {
                        run: run.clone(),
                        step: step.clone(),
                        answer,
                    })
                }
                Item::Gate {
                    card,
                    run,
                    step,
                    title,
                    counts,
                    findings,
                    rows,
                    leave_out,
                    state,
                } => {
                    if let State::Answered(how) = state {
                        cards::card(ui, title, how, |_| {});
                        return None;
                    }
                    let counts: Vec<cards::CountRow<'_>> = counts
                        .iter()
                        .map(|(label, n)| cards::CountRow { label, n: *n })
                        .collect();
                    let findings: Vec<cards::FindingRow<'_>> = findings
                        .iter()
                        .map(|(blocking, message)| cards::FindingRow {
                            blocking: *blocking,
                            message,
                        })
                        .collect();
                    let rows: Vec<cards::ChangeRow<'_>> = rows
                        .iter()
                        .map(|(id, text)| cards::ChangeRow { id, text })
                        .collect();
                    let view = cards::Impact {
                        title,
                        gate: cards::Gate::Review,
                        counts: &counts,
                        findings: &findings,
                        rows: &rows,
                    };
                    let choice = cards::impact(
                        ui,
                        egui::Id::unique(("gate", card.as_str())),
                        &view,
                        leave_out,
                        active,
                    )?;
                    // A confirm gate's card: an Ask is the chat's own card (C1).
                    let (label, answer) = match choice {
                        cards::GateChoice::Apply => (
                            "Applied",
                            ControlAnswer::Apply {
                                leave_out: leave_out.clone(),
                            },
                        ),
                        _ => ("Declined", ControlAnswer::Decline),
                    };
                    let action = CardAction::Answer {
                        run: run.clone(),
                        step: step.clone(),
                        answer,
                    };
                    *state = State::Answered(label);
                    Some(action)
                }
                Item::Notify {
                    card,
                    run,
                    step,
                    field,
                    state,
                } => {
                    let ControlCard::Choice {
                        field: f,
                        options,
                        value: shown,
                        ..
                    } = &field.card
                    else {
                        return None;
                    };
                    let options: Vec<cards::ChoiceOption<'_>> = options
                        .iter()
                        .map(|o| cards::ChoiceOption {
                            value: &o.value,
                            title: &o.title,
                        })
                        .collect();
                    let aside = match state {
                        State::Answered(how) => *how,
                        State::Waiting => "",
                    };
                    let waiting = matches!(state, State::Waiting);
                    // The choice first, on one line with why; the other options
                    // only once the person asks for them.
                    let title_of = |value: Option<&String>| {
                        value
                            .and_then(|v| options.iter().find(|o| o.value == v.as_str()))
                            .map_or_else(
                                || value.cloned().unwrap_or_else(|| "None".to_owned()),
                                |o| o.title.to_owned(),
                            )
                    };
                    let open_id = egui::Id::unique(("notify-open", card.as_str()));
                    let mut open = ui
                        .ctx()
                        .data(|d| d.get_temp::<bool>(open_id))
                        .unwrap_or(false);
                    cards::card(ui, &f.title, aside, |ui| {
                        let chosen = title_of(field.chosen.first());
                        let line = if f.help.is_empty() {
                            chosen
                        } else {
                            format!("{chosen} · {}", f.help)
                        };
                        cards::line(ui, &line);
                        if waiting && !open && options.len() > 1 {
                            let other =
                                cards::footer(ui, &[cards::FooterButton::new("Choose another…")]);
                            if other.is_some() {
                                open = true;
                                ui.ctx().data_mut(|d| d.insert_temp(open_id, true));
                            }
                        }
                        if waiting && open {
                            cards::choice_input(
                                ui,
                                egui::Id::unique(("notify", card.as_str())),
                                &options,
                                &mut field.chosen,
                                false,
                            );
                        }
                        if !waiting {
                            return None;
                        }
                        let differs = field.chosen != *shown;
                        let why = if differs || field.chosen.is_empty() {
                            ""
                        } else {
                            "Choose another option to change it"
                        };
                        // Continue keeps the model's choice; Change sends the run
                        // back with the person's. Enter continues.
                        let pressed = cards::footer(
                            ui,
                            &[
                                cards::FooterButton::new("Change").unless(why),
                                cards::FooterButton::primary("Continue"),
                            ],
                        );
                        let key = cards::card_key(ui, active);
                        let as_value = |v: Option<&String>| {
                            v.map_or(Value::Null, |v| Value::String(v.clone()))
                        };
                        let (how, value) = match (pressed, key) {
                            (Some(0), _) => ("Changed", as_value(field.chosen.first())),
                            (Some(1), _) | (None, Some(cards::CardKey::Accept)) => {
                                ("Kept", as_value(shown.first()))
                            }
                            _ => return None,
                        };
                        *state = State::Answered(how);
                        Some(CardAction::Answer {
                            run: run.clone(),
                            step: step.clone(),
                            answer: ControlAnswer::Change { value },
                        })
                    })
                }
                Item::Result { run, card, undone } => {
                    let ControlCard::Result {
                        title,
                        items,
                        findings,
                        actions,
                        undo,
                    } = card
                    else {
                        return None;
                    };
                    // A report's Markdown shows by its first line; Show report
                    // opens it under the items.
                    let values: Vec<String> = items
                        .iter()
                        .map(|i| workflow::control::readable(&i.value))
                        .collect();
                    let long: Vec<(String, String)> = items
                        .iter()
                        .filter_map(|i| {
                            let text = i.value.as_str().filter(|t| t.contains('\n'))?;
                            Some((i.title.clone(), text.to_owned()))
                        })
                        .collect();
                    let rows: Vec<cards::ItemRow<'_>> = items
                        .iter()
                        .zip(&values)
                        .map(|(i, v)| cards::ItemRow {
                            title: &i.title,
                            value: v,
                        })
                        .collect();
                    let findings: Vec<cards::FindingRow<'_>> = findings
                        .iter()
                        .map(|f| cards::FindingRow {
                            blocking: f.blocking,
                            message: &f.message,
                        })
                        .collect();
                    // After Undo run, what it made is gone: nothing to open.
                    let labels: Vec<&str> = if *undone {
                        Vec::new()
                    } else {
                        actions.iter().map(|a| a.title.as_str()).collect()
                    };
                    let view = cards::Outcome {
                        title,
                        items: &rows,
                        findings: &findings,
                        actions: &labels,
                        undo: *undo && !*undone,
                    };
                    let shown_id = egui::Id::unique(("result-long", run.as_str()));
                    let more = |ui: &mut egui::Ui| {
                        for (title, text) in &long {
                            let open = ui
                                .ctx()
                                .data(|d| d.get_temp::<bool>(shown_id.with(title)))
                                .unwrap_or(false);
                            let label = if open {
                                format!("Hide {}", title.to_lowercase())
                            } else {
                                format!("Show {}", title.to_lowercase())
                            };
                            if sparksmith_ui::ui_kit::button(
                                ui,
                                label,
                                sparksmith_ui::ui_kit::Button::Small,
                            )
                            .clicked()
                            {
                                ui.ctx()
                                    .data_mut(|d| d.insert_temp(shown_id.with(title), !open));
                            }
                            if open {
                                crate::assistant::markdown(ui, text);
                            }
                        }
                    };
                    match cards::outcome(ui, &view, more)? {
                        cards::OutcomeChoice::Action(i) => {
                            actions.get(i).map(|a| CardAction::Show {
                                call: a.call.clone(),
                                params: a.params.clone(),
                            })
                        }
                        cards::OutcomeChoice::UndoRun => {
                            *undone = true;
                            Some(CardAction::Answer {
                                run: run.clone(),
                                step: String::new(),
                                answer: ControlAnswer::UndoRun,
                            })
                        }
                    }
                }
            })
            .inner;
        if let Some(path) = toggle {
            self.toggle_document(index, path);
        }
        action
    }
}

impl FieldEdit {
    fn new(card: ControlCard) -> Self {
        let (text, chosen) = match &card {
            ControlCard::Text { value, .. }
            | ControlCard::Url { value, .. }
            | ControlCard::PageRanges { value, .. } => {
                (value.clone().unwrap_or_default(), Vec::new())
            }
            ControlCard::Number {
                quantity, value, ..
            } => (
                value
                    .as_ref()
                    .map(|n| number_text(*quantity, n))
                    .unwrap_or_default(),
                Vec::new(),
            ),
            ControlCard::Choice { value, .. } => (String::new(), value.clone()),
            _ => (String::new(), Vec::new()),
        };
        let documents = match &card {
            ControlCard::Datasheet { files, .. } => files
                .iter()
                .map(|f| Document {
                    given: PathBuf::from(&f.path),
                    title: f.title.clone(),
                    detail: if f.bytes == 0 && f.pages.is_none() {
                        String::new()
                    } else {
                        document_detail(f.pages.map(u64::from), f.bytes)
                    },
                    stored: Some(PathBuf::from(&f.path)),
                    failed: false,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut field = Self {
            card,
            text,
            chosen,
            documents,
            error: None,
            shown: true,
            edited: false,
        };
        field.shown = !field.optional() || !field.empty();
        field
    }

    fn optional(&self) -> bool {
        heading(&self.card).is_some_and(|h| h.optional)
    }

    /// Whether it holds nothing yet.
    fn empty(&self) -> bool {
        match &self.card {
            ControlCard::Choice { .. } => self.chosen.is_empty(),
            ControlCard::Datasheet { .. } => self.documents.is_empty(),
            _ => self.text.trim().is_empty(),
        }
    }

    /// The value this field gives, or why it cannot be taken; `None` for
    /// an optional field left empty.
    fn value(&self) -> Result<Option<Value>, String> {
        let empty = self.text.trim().is_empty();
        match &self.card {
            ControlCard::Text {
                field, max_bytes, ..
            } => {
                if empty {
                    return required(field.optional);
                }
                if self.text.len() as u64 > *max_bytes {
                    return Err(format!("{} bytes; at most {max_bytes}", self.text.len()));
                }
                Ok(Some(Value::String(self.text.clone())))
            }
            ControlCard::Number {
                field,
                quantity,
                min,
                max,
                ..
            } => {
                if empty {
                    return required(field.optional);
                }
                let value = number_value(*quantity, &self.text)?;
                let n = value.as_f64().unwrap_or_default();
                let bound = |b: &Option<serde_json::Number>| {
                    b.as_ref().and_then(serde_json::Number::as_f64)
                };
                if let Some(least) = bound(min)
                    && n < least
                {
                    return Err(format!(
                        "at least {}",
                        number_text(*quantity, &number(least))
                    ));
                }
                if let Some(most) = bound(max)
                    && n > most
                {
                    return Err(format!("at most {}", number_text(*quantity, &number(most))));
                }
                Ok(Some(value))
            }
            ControlCard::Choice {
                field, multiple, ..
            } => match (self.chosen.as_slice(), multiple) {
                ([], _) => required(field.optional),
                (chosen, true) => Ok(Some(Value::Array(
                    chosen.iter().cloned().map(Value::String).collect(),
                ))),
                ([one, ..], false) => Ok(Some(Value::String(one.clone()))),
            },
            ControlCard::Datasheet {
                field, min, max, ..
            } => {
                if self.documents.is_empty() {
                    return if field.optional && *min == 0 {
                        Ok(None)
                    } else {
                        Err(format!("give {min} to {max} documents"))
                    };
                }
                if let Some(bad) = self.documents.iter().find(|d| d.failed) {
                    return Err(format!("{}: {}", bad.title, bad.detail));
                }
                if self.documents.iter().any(|d| d.stored.is_none()) {
                    return Err("still reading the documents".into());
                }
                let count = self.documents.len() as u64;
                if count < *min || count > *max {
                    return Err(format!("{count} documents; give {min} to {max}"));
                }
                Ok(Some(Value::Array(
                    self.documents
                        .iter()
                        .filter_map(|d| d.stored.as_ref())
                        .map(|p| Value::String(p.display().to_string()))
                        .collect(),
                )))
            }
            ControlCard::Url { field, .. } => {
                if empty {
                    return required(field.optional);
                }
                let url = self.text.trim();
                if !url.starts_with("https://") || url.len() > 2048 {
                    return Err("give an https:// address".into());
                }
                Ok(Some(Value::String(url.to_owned())))
            }
            ControlCard::PageRanges { field, pages, .. } => {
                if empty {
                    return required(field.optional);
                }
                let ranges = workflow::parse_page_ranges(&self.text)?;
                if let Some(count) = pages
                    && let Some((_, last)) = ranges.iter().find(|(_, b)| b > count)
                {
                    return Err(format!("page {last} is past the last, {count}"));
                }
                Ok(Some(Value::String(self.text.trim().to_owned())))
            }
            _ => Ok(None),
        }
    }
}

fn required(optional: bool) -> Result<Option<Value>, String> {
    if optional {
        Ok(None)
    } else {
        Err("required".into())
    }
}

/// The files a datasheet value's ⌄ lists at most.
const MAX_FILE_CHOICES: usize = 12;

/// What the person did on a compact confirmation.
enum Compact {
    /// Accept and continue, with the values of its inputs.
    Accept(Map<String, Value>),
    Decline,
    /// Change… on a datasheet value: the host's picker.
    Pick,
}

/// A compact confirmation: one line per input that shows (a value given,
/// or a required one to fill in), each changed in place; then the decline
/// and Accept and continue. Enter accepts; a required input left empty, or
/// a value that does not pass, is marked and nothing is answered. When it
/// first waits, the keyboard goes to the first input to fill in, or else
/// leaves the composer, so Enter accepts.
fn show_compact(
    ui: &mut egui::Ui,
    card: &str,
    view: &cards::Confirm<'_>,
    fields: &mut [FieldEdit],
    active: bool,
    candidates: &[PathBuf],
    toggle: &mut Option<PathBuf>,
) -> Option<Compact> {
    let mut pick = false;
    let focus_once = egui::Id::unique(("workflow-confirm-focus", card));
    let first_wait = active
        && !ui
            .ctx()
            .data(|d| d.get_temp::<bool>(focus_once))
            .unwrap_or(false);
    let mut to_fill = None;
    let choice = cards::confirm(ui, view, active, |ui| {
        let mut entered = false;
        for (index, field) in fields.iter_mut().enumerate() {
            if !field.shown {
                continue;
            }
            let Some(title) = heading(&field.card).map(|h| h.title.to_owned()) else {
                continue;
            };
            let id = egui::Id::unique(("workflow-field", card, index));
            let error = field.error.clone();
            let invalid = error.is_some();
            let missing = !field.optional() && field.empty();
            let FieldEdit {
                card: control,
                text,
                chosen,
                documents,
                edited,
                ..
            } = field;
            let response = cards::compact_row(ui, &title, error.as_deref(), |ui| match &*control {
                ControlCard::Text {
                    multiline,
                    max_bytes,
                    ..
                } => Some(cards::text_input(
                    ui,
                    id,
                    text,
                    *multiline,
                    usize::try_from(*max_bytes).unwrap_or(usize::MAX),
                    invalid,
                )),
                ControlCard::Number { quantity, .. } => Some(cards::number_input(
                    ui,
                    id,
                    text,
                    unit_of(*quantity),
                    invalid,
                )),
                ControlCard::Choice {
                    options, multiple, ..
                } => {
                    let options: Vec<cards::ChoiceOption<'_>> = options
                        .iter()
                        .map(|o| cards::ChoiceOption {
                            value: &o.value,
                            title: &o.title,
                        })
                        .collect();
                    *edited |= cards::choice_input(ui, id, &options, chosen, *multiple);
                    None
                }
                ControlCard::Datasheet { .. } => {
                    let shown = documents_line(documents);
                    let rows: Vec<(String, String, bool)> = candidates
                        .iter()
                        .take(MAX_FILE_CHOICES)
                        .map(|path| {
                            let held = documents
                                .iter()
                                .find(|d| d.given == *path || d.stored.as_ref() == Some(path));
                            let detail = held.map_or_else(
                                || {
                                    path.parent()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_default()
                                },
                                |d| d.detail.clone(),
                            );
                            (file_name(path), detail, held.is_some())
                        })
                        .collect();
                    let options: Vec<cards::FileOption<'_>> = rows
                        .iter()
                        .map(|(title, detail, chosen)| cards::FileOption {
                            title,
                            detail,
                            chosen: *chosen,
                        })
                        .collect();
                    match cards::file_value(ui, id, &shown, &options) {
                        Some(cards::FileChoice::Change) => pick = true,
                        Some(cards::FileChoice::Toggle(i)) => {
                            *toggle = candidates.get(i).cloned();
                        }
                        None => {}
                    }
                    None
                }
                ControlCard::Url { .. } => Some(cards::url_input(ui, id, text, invalid)),
                ControlCard::PageRanges { pages, .. } => {
                    Some(cards::pages_input(ui, id, text, *pages, invalid))
                }
                _ => None,
            });
            if let Some(response) = response {
                if missing && to_fill.is_none() {
                    to_fill = Some(response.id);
                }
                entered |= active && cards::entered(ui, &response);
                if response.changed() {
                    field.error = None;
                    field.edited = true;
                }
            }
        }
        entered
    });
    if first_wait {
        let ctx = ui.ctx();
        ctx.data_mut(|d| d.insert_temp(focus_once, true));
        let composer = crate::assistant::Assistant::composer_id();
        match to_fill {
            Some(id) => ctx.memory_mut(|m| m.request_focus(id)),
            None if ctx.memory(|m| m.has_focus(composer)) => {
                ctx.memory_mut(|m| m.surrender_focus(composer));
            }
            None => {}
        }
    }
    if pick {
        return Some(Compact::Pick);
    }
    match choice? {
        cards::ConfirmChoice::Decline => Some(Compact::Decline),
        cards::ConfirmChoice::Accept => {
            let mut values = Map::new();
            let mut ok = true;
            for field in fields.iter_mut() {
                let name = input_name(&field.card);
                match field.value() {
                    Ok(Some(value)) => {
                        field.error = None;
                        values.insert(name, value);
                    }
                    Ok(None) => field.error = None,
                    Err(why) => {
                        // A hidden field is optional and empty: it cannot
                        // refuse, but an error must show where it can.
                        field.shown = true;
                        field.error = Some(why);
                        ok = false;
                    }
                }
            }
            ok.then_some(Compact::Accept(values))
        }
    }
}

/// A datasheet value on one line: one file with its pages and size, or
/// the names of several.
fn documents_line(documents: &[Document]) -> String {
    match documents {
        [] => String::new(),
        [one] if one.detail.is_empty() => one.title.clone(),
        [one] => format!("{} · {}", one.title, one.detail),
        many => many
            .iter()
            .map(|d| d.title.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// An answered confirmation: what it gave, one line.
fn answered(ui: &mut egui::Ui, title: &str, how: &str, fields: &[FieldEdit]) {
    cards::card(ui, title, how, |ui| {
        let given: Vec<String> = fields
            .iter()
            .filter_map(|field| {
                let heading = heading(&field.card)?;
                let shown = match &field.card {
                    ControlCard::Datasheet { .. } => documents_line(&field.documents),
                    _ => field.value().ok().flatten().map(|v| value_text(&v))?,
                };
                (!shown.is_empty()).then(|| format!("{}: {shown}", heading.title))
            })
            .collect();
        if !given.is_empty() {
            cards::note(ui, &given.join(" · "));
        }
    });
}

fn heading(card: &ControlCard) -> Option<cards::Field<'_>> {
    let field = match card {
        ControlCard::Text { field, .. }
        | ControlCard::Number { field, .. }
        | ControlCard::Choice { field, .. }
        | ControlCard::Datasheet { field, .. }
        | ControlCard::Url { field, .. }
        | ControlCard::PageRanges { field, .. } => field,
        _ => return None,
    };
    Some(cards::Field {
        title: &field.title,
        help: &field.help,
        optional: field.optional,
    })
}

fn input_name(card: &ControlCard) -> String {
    match card {
        ControlCard::Text { field, .. }
        | ControlCard::Number { field, .. }
        | ControlCard::Choice { field, .. }
        | ControlCard::Datasheet { field, .. }
        | ControlCard::Url { field, .. }
        | ControlCard::PageRanges { field, .. } => field.input.clone(),
        _ => String::new(),
    }
}

/// The unit a bare number is read in.
fn unit_of(quantity: Quantity) -> &'static str {
    match quantity {
        Quantity::Length => "mm",
        Quantity::Angle => "°",
        Quantity::Number | Quantity::Integer => "",
    }
}

/// What the person typed as the value an input of `quantity` takes:
/// lengths in whole nanometres (`0.25`, `0.25mm`, `10mil`), angles in
/// whole millidegrees (`45`, `45°`), numbers and whole numbers as such.
pub(crate) fn number_value(quantity: Quantity, text: &str) -> Result<Value, String> {
    let text = text.trim();
    match quantity {
        Quantity::Length => parse_length(text, LengthUnit::Mm)
            .map(|nm| Value::from(nm.get()))
            .map_err(|_| "give a length such as 0.25mm or 10mil".to_owned()),
        Quantity::Angle => {
            let degrees: f64 = text
                .trim_end_matches('°')
                .trim_end_matches("deg")
                .trim()
                .parse()
                .map_err(|_| "give an angle in degrees, such as 45".to_owned())?;
            if !degrees.is_finite() || degrees.abs() > 1e9 {
                return Err("give an angle in degrees, such as 45".into());
            }
            // Within ±1e12 millidegrees: the cast is lossless.
            Ok(Value::from((degrees * 1_000.0).round() as i64))
        }
        Quantity::Integer => text
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| "give a whole number".to_owned()),
        Quantity::Number => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| "give a number".to_owned()),
    }
}

/// A stored value as the person types it: millimetres for a length,
/// degrees for an angle.
fn number_text(quantity: Quantity, n: &serde_json::Number) -> String {
    match quantity {
        Quantity::Length => n
            .as_i64()
            .and_then(Nm::try_new)
            .map_or_else(|| n.to_string(), |nm| format_number(nm, LengthUnit::Mm)),
        Quantity::Angle => n.as_i64().map_or_else(
            || n.to_string(),
            |m| {
                let text = format!("{}.{:03}", m / 1000, (m % 1000).abs());
                text.trim_end_matches('0').trim_end_matches('.').to_owned()
            },
        ),
        Quantity::Number | Quantity::Integer => n.to_string(),
    }
}

fn number(n: f64) -> serde_json::Number {
    serde_json::Number::from_f64(n).unwrap_or_else(|| serde_json::Number::from(0))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The media type of the file at `path`, from its first bytes.
fn media_type_of(path: &Path) -> Option<&'static str> {
    use std::io::Read as _;
    let mut head = [0u8; 8];
    let mut file = std::fs::File::open(path).ok()?;
    let n = file.read(&mut head).ok()?;
    offer::media_type(head.get(..n)?)
}

/// Read a document: a PDF within the size bound, its page count, and a
/// copy in `folder` when given.
fn read_document(path: &Path, folder: Option<&Path>) -> Result<Document, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("cannot be read: {e}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!("larger than {} MiB", MAX_FILE_BYTES >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot be read: {e}"))?;
    if offer::media_type(&bytes) != Some("application/pdf") {
        return Err("not a PDF".into());
    }
    let pages = sparksmith_assist::pdf_pages(&bytes);
    let stored = match folder {
        Some(folder) => {
            std::fs::create_dir_all(folder).map_err(|e| format!("cannot be copied: {e}"))?;
            let to = free_name(folder, &file_name(path));
            sparksmith_io::persistence::atomic_create(&to, &bytes)
                .map_err(|e| format!("cannot be copied: {e}"))?;
            to
        }
        None => path.to_path_buf(),
    };
    Ok(Document {
        given: path.to_path_buf(),
        title: file_name(path),
        detail: document_detail((pages > 0).then_some(pages), size),
        stored: Some(stored),
        failed: false,
    })
}

/// `name` in `folder`, numbered when taken.
fn free_name(folder: &Path, name: &str) -> PathBuf {
    let first = folder.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..1000)
        .map(|n| {
            folder.join(if ext.is_empty() {
                format!("{stem} {n}")
            } else {
                format!("{stem} {n}.{ext}")
            })
        })
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// "38 pages · 1.2 MB".
fn document_detail(pages: Option<u64>, bytes: u64) -> String {
    let size = if bytes >= 1024 * 1024 {
        format!(
            "{}.{} MB",
            bytes / (1024 * 1024),
            (bytes % (1024 * 1024)) * 10 / (1024 * 1024)
        )
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    };
    match pages {
        Some(1) => format!("1 page · {size}"),
        Some(n) => format!("{n} pages · {size}"),
        None => size,
    }
}

/// A value on a card: text as itself, a list joined, anything else as JSON.
fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(list) => list.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Value::Null => "—".into(),
        other => other.to_string(),
    }
}

/// How a run ended, in one line.
fn end_line(end: &RunEnd) -> String {
    match end {
        RunEnd::Done => "Done".into(),
        RunEnd::Stopped => "Stopped; what completed stays as one undo step".into(),
        RunEnd::Declined { .. } => "Declined; what completed stays as one undo step".into(),
        RunEnd::Limit { why } => format!("Limit reached: {why}"),
        RunEnd::Failed { why, .. } => format!("Failed: {why}"),
    }
}

/// A row of the `/` menu or the palette.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MenuRow {
    pub id: String,
    pub title: String,
    /// The summary, or why it cannot run here.
    pub detail: String,
    pub enabled: bool,
}

/// The workflows whose title, summary or intents hold every word of
/// `query`, those that run here first, then by title.
pub(crate) fn menu_rows(listed: &[Listed], query: &str) -> Vec<MenuRow> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut rows: Vec<MenuRow> = listed
        .iter()
        .filter_map(|l| {
            let loaded = l.loaded.as_ref()?;
            let w = &loaded.workflow;
            let text = format!("{} {} {}", w.title, w.summary, w.intents.join(" ")).to_lowercase();
            words
                .iter()
                .all(|word| text.contains(word.as_str()))
                .then(|| MenuRow {
                    id: w.id.clone(),
                    title: w.title.clone(),
                    detail: if l.available() {
                        w.summary.clone()
                    } else {
                        l.why_not.join("; ")
                    },
                    enabled: l.available(),
                })
        })
        .collect();
    rows.sort_by(|a, b| {
        b.enabled
            .cmp(&a.enabled)
            .then_with(|| a.title.cmp(&b.title))
    });
    rows
}

/// The palette's rows: "Assistant: <title>" for each workflow the query
/// names, generated from the workflows' data; the id is
/// [`PALETTE_PREFIX`] and the workflow's id, the detail "Assistant" or why
/// it cannot run here.
pub(crate) fn palette_rows(listed: &[Listed], query: &str) -> Vec<MenuRow> {
    let query = query.trim().to_lowercase();
    let query = query
        .strip_prefix("assistant")
        .map_or(query.as_str(), |rest| rest.trim_start_matches(':').trim());
    menu_rows(listed, query)
        .into_iter()
        .map(|row| MenuRow {
            id: format!("{PALETTE_PREFIX}{}", row.id),
            title: format!("Assistant: {}", row.title),
            detail: if row.enabled {
                "Assistant".into()
            } else {
                row.detail
            },
            enabled: row.enabled,
        })
        .collect()
}

#[cfg(test)]
#[path = "workflow_cards_tests.rs"]
pub(crate) mod tests;

//! The assistant chat in the window (`docs/assistant-panel.md` §7, slice
//! C1): its column at the window's edge, and what its turns ask of the
//! window.
//!
//! A message opens a turn (`project.turn_begin`) and reads what the window
//! shows (`view.context`), both through the front door as the built-in
//! agent; the turn's model runs off the UI thread
//! ([`crate::assistant::worker`]). Its tool calls are served here between
//! frames: each runs the verb on the window's engine as
//! `Principal::LocalAgent` under the chat's reach (`library/assist/reach/
//! chat.json`), in the message's turn, with this window as its
//! `WindowHost`, so its edits undo as one step and `show` steers this
//! window. A call the gate asks about waits as an Ask card for the person's
//! Allow or Deny. When the turn ends it is closed (`project.turn_end`) and
//! a summary card says what changed between the document before and after
//! it (`sparksmith_automation::session::difference`).
//!
//! A workflow run's calls come here the same way
//! (`app/assistant_run.rs`): under the run's grant and in the run's turn
//! instead of the message's.
use super::App;
use crate::assistant::worker::{self, ChatModel, FromTurn, ToolRequest};
use crate::assistant::{
    self, Action, Allow, AskCard, Asking, Dock, Held, Item, Now, OpenTurn, State, TurnSummary,
};
use crate::kit::severity::Level;
use serde_json::{Value, json};
use sparksmith_assist::chat::{self, ChatConfig, End, Outcome, ReachProfile};
use sparksmith_assist::desk::{self, Called};
use sparksmith_assist::model::{Budget, Connection, KeySource, Message, Part, Provider};
use sparksmith_assist::settings::{self as ai, AssistSettings, Permissions, Rule, Shipped};
use sparksmith_automation::front::{self, Answer, Asker, Call, CallCx, NoOne, Surface};
use sparksmith_automation::workflow::ask::AskReply;
use sparksmith_automation::{Grant, Principal, Question, WindowHost};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{Container, Edge, Surface as Host};
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;

/// Messages from a turn handled per frame, so a flood cannot starve drawing.
const PER_FRAME: usize = 64;
/// New issues a summary card names.
const NAMED_ISSUES: usize = 3;
/// Names a summary line lists before it counts.
const NAMED_OBJECTS: usize = 4;
/// The longest undo label, in characters, before "…".
const LABEL_CHARS: usize = 60;
/// The entry that asks the person on a question card: it shows no view, so
/// the rule for moving the view does not hold it.
const ASK: &str = "assist.ask";

/// The kind of action a question is about, as Settings › AI's permissions
/// name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Edits,
    Files,
    PublishLibrary,
    View,
}

impl Kind {
    /// The kind of the entries a question names: an edit, a library
    /// action that asks (publishing to a library), or anything else that
    /// undo cannot take back (files, fabrication output, the 3D model).
    fn of(question: &Question) -> Self {
        let entries = || {
            question
                .command
                .split(", ")
                .filter_map(sparksmith_automation::session::metadata::find)
        };
        if entries().all(|e| e.class == sparksmith_core::Class::Show) {
            Self::View
        } else if entries().all(|e| e.class == sparksmith_core::Class::Edit) {
            Self::Edits
        } else if entries().any(|e| e.module == "library") {
            Self::PublishLibrary
        } else {
            Self::Files
        }
    }

    fn rule(self, permissions: &Permissions) -> Rule {
        match self {
            Self::Edits => permissions.edits,
            Self::Files => permissions.files,
            Self::PublishLibrary => permissions.publish_library,
            Self::View => permissions.view,
        }
    }

    fn set(self, permissions: &mut Permissions, rule: Rule) {
        match self {
            Self::Edits => permissions.edits = rule,
            Self::Files => permissions.files = rule,
            Self::PublishLibrary => permissions.publish_library = rule,
            Self::View => permissions.view = rule,
        }
    }
}

/// How the chat asks: a question the gate asks follows Settings › AI's
/// rule for its kind (always allowed, denied, or a card); a card is not
/// shown when the person already allowed this call, or every entry it
/// names runs without asking (the reach profile's `unasked`, or the
/// person's Allow for this session or always).
struct ChatAsker {
    allowed: bool,
    unasked: Vec<String>,
    permissions: Permissions,
    question: Option<Question>,
}

impl Asker for ChatAsker {
    fn ask(&mut self, question: &Question) -> Answer {
        let unasked = question
            .command
            .split(", ")
            .all(|entry| self.unasked.iter().any(|u| u == entry));
        if self.allowed || unasked {
            return Answer::Allow;
        }
        match Kind::of(question).rule(&self.permissions) {
            Rule::Always => Answer::Allow,
            Rule::Deny => Answer::Deny,
            Rule::Ask | Rule::Session => {
                self.question = Some(question.clone());
                Answer::Unavailable
            }
        }
    }
}

/// Settings › AI as the person keeps it in `config`, the shipped starting
/// values and its file.
fn ai_settings(
    config: Option<std::path::PathBuf>,
) -> Result<(AssistSettings, Shipped, std::path::PathBuf), String> {
    let config = config.ok_or("No config folder: set SPARKSMITH_CONFIG_DIR or a home folder")?;
    let path = ai::user_file(&config);
    let settings = ai::read(&path).map_err(|e| e.to_string())?;
    let shipped = Shipped::locate().map_err(|e| e.to_string())?;
    Ok((settings, shipped, path))
}

/// A call of `entry` with `params`, outside any turn.
fn call(entry: &str, params: Value) -> Call {
    Call {
        entry: entry.to_owned(),
        params,
        project: None,
        expected_revision: None,
        dry_run: false,
        confirm: None,
        turn: None,
        job: false,
    }
}

/// The undo label of a message's turn: "AI assistant: " and the message's
/// start.
pub(super) fn turn_label(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let mut label: String = line.chars().take(LABEL_CHARS).collect();
    if line.chars().count() > LABEL_CHARS {
        label.push('…');
    }
    format!("AI assistant: {label}")
}

impl App {
    /// The chat's grant and reach: the shipped reach profile, with the
    /// open project's folder the only one a path may name.
    pub(super) fn chat_grant(&mut self) -> Result<(Grant, ReachProfile), String> {
        let reach = self.assistant.library()?.reach.clone();
        let mut grant = reach.grant();
        grant.folders = self.project_folder().into_iter().collect();
        Ok((grant, reach))
    }

    /// The model the chat uses, its budget and the person's permissions:
    /// Settings › AI's endpoint for the assistant, its limits when the
    /// person set any (none by default: no per-turn caps), and its rules
    /// per kind of action. A key comes from the keychain for the endpoint,
    /// else for Anthropic `ANTHROPIC_API_KEY` when it is set; a hosted
    /// model without either is not connected
    /// ([`super::settings_ai::readiness`]).
    pub(super) fn chat_model(&mut self) -> Result<(ChatModel, Budget, Permissions), String> {
        if let Some(model) = &self.assistant.model {
            let shipped = Shipped::locate().map_err(|e| e.to_string())?;
            return Ok((model.clone(), chat::unbounded(), shipped.permissions));
        }
        let page = &self.settings_window.ai;
        let keyring = page.keyring();
        let (settings, shipped, _) = ai_settings(page.config_dir())?;
        super::settings_ai::readiness(&settings, keyring)?;
        let setting = settings
            .run_endpoint()
            .cloned()
            .ok_or(super::settings_ai::NOT_CONNECTED)?;
        let profile = setting.model_profile().map_err(|e| e.to_string())?;
        let endpoint = setting.endpoint().map_err(|e| e.to_string())?;
        let key = if endpoint.check_key_allowed().is_ok() && keyring.has(&endpoint) {
            KeySource::Keychain
        } else if endpoint.is_loopback() {
            KeySource::None
        } else {
            match profile.provider {
                Provider::Anthropic => KeySource::Env("ANTHROPIC_API_KEY".into()),
                Provider::OpenAiCompatible => KeySource::None,
            }
        };
        let connection = Connection { endpoint, key };
        let declared = profile.clone();
        let budget = ai::run_budget(settings.limits.as_ref(), None).unwrap_or_else(chat::unbounded);
        Ok((
            ChatModel {
                profile,
                connect: Arc::new(move || {
                    sparksmith_assist::model::connect(&declared, &connection)
                        .map_err(|e| e.to_string())
                }),
            },
            budget,
            settings.permissions_or(&shipped),
        ))
    }

    /// Always allow on a card: Settings › AI's rule for the kind becomes
    /// Always, kept in the person's settings file.
    fn always_allow(&mut self, kind: Kind) {
        if let Some(permissions) = self.assistant.permissions.as_mut() {
            kind.set(permissions, Rule::Always);
        }
        if self.assistant.model.is_some() {
            // A test's model: no settings file is touched.
            return;
        }
        let kept = ai_settings(self.settings_window.ai.config_dir()).and_then(
            |(mut settings, shipped, path)| {
                let mut permissions = settings.permissions_or(&shipped);
                kind.set(&mut permissions, Rule::Always);
                settings.permissions = Some(permissions);
                ai::write(&path, &settings).map_err(|e| e.to_string())
            },
        );
        if let Err(why) = kept {
            self.say(
                Level::Warning,
                format!("Could not keep Always allow: {why}"),
            );
        }
    }

    /// The column, or its folded strip, at the window's edge: before the
    /// workspace, which takes the room left.
    pub(super) fn assistant_column(&mut self, ui: &mut egui::Ui) {
        if !self.assistant.open {
            return;
        }
        self.check_setup();
        let ctx = ui.ctx().clone();
        let t = Tokens::get();
        let host = Container::new(&ctx, Host::Panel);
        let edge = match self.assistant.dock {
            Dock::Right => Edge::Right,
            Dock::Left => Edge::Left,
        };
        let frame = host
            .edge_frame(t.panel)
            .inner_margin(egui::Margin::same(t.space_8 as i8));
        let actions = if self.assistant.folded {
            let panel = host
                .edge("assistant-strip", edge)
                .resizable(false)
                .exact_size(assistant::STRIP_WIDTH);
            let frame = host.edge_frame(t.panel).inner_margin(egui::Margin::same(0));
            host.show_edge(ui, panel, frame, |ui| self.assistant.strip(ui))
                .inner
        } else {
            let (window, engine) = self.window_and_engine();
            let context = serde_json::to_value(window.context(engine)).unwrap_or_default();
            let now = Now {
                revision: self.engine.revision(),
                undo_label: self.engine.undo_step().map(|step| step.line()),
                context: assistant::context_line(&context),
                folder: self.project_folder(),
            };
            let panel = host
                .edge("assistant-column", edge)
                .resizable(true)
                .default_size(assistant::DEFAULT_WIDTH)
                .size_range(assistant::MIN_WIDTH..=assistant::MAX_WIDTH);
            let cards = &mut self.workflow_cards;
            host.show_edge(ui, panel, frame, |ui| {
                self.assistant.column(ui, &now, cards)
            })
            .inner
        };
        for action in actions {
            self.assistant_action(&ctx, action);
        }
    }

    /// The open project's folder, when it is saved.
    pub(super) fn project_folder(&self) -> Option<std::path::PathBuf> {
        self.files
            .path()
            .and_then(std::path::Path::parent)
            .filter(|folder| folder.is_absolute())
            .map(std::path::Path::to_path_buf)
    }

    /// Whether a model is connected, looked at once when the chat opens and
    /// again when Settings › AI saves a connection: the chat says "No model
    /// connected" with Connect a model… before a message is typed, and is
    /// ready again once one is saved.
    fn check_setup(&mut self) {
        if self.settings_window.ai.take_connected() {
            self.assistant.setup_checked = false;
        }
        if self.assistant.setup_checked || self.assistant.model.is_some() {
            return;
        }
        self.assistant.setup_checked = true;
        let state = &mut self.assistant.conversation.state;
        if !matches!(state, State::Idle | State::NotSetUp(_)) {
            return;
        }
        let page = &self.settings_window.ai;
        let ready = ai_settings(page.config_dir())
            .and_then(|(settings, ..)| super::settings_ai::readiness(&settings, page.keyring()));
        *state = match ready {
            Ok(()) => State::Idle,
            Err(why) => State::NotSetUp(why),
        };
    }

    /// ⇧⌘A, View › Assistant, the palette and the application bar's button.
    pub(super) fn toggle_assistant(&mut self, ctx: &egui::Context) {
        self.assistant.toggle();
        if self.assistant.shown() {
            self.assistant.setup_checked = false;
            ctx.memory_mut(|m| m.request_focus(assistant::Assistant::composer_id()));
        }
    }

    fn assistant_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Send => self.assistant_send(ctx),
            Action::Stop => self.assistant.stop(),
            Action::Answer { id, allow } => self.assistant_answer(ctx, id, allow),
            Action::Show(id) => self.assistant_show(id),
            Action::UndoTurn(id) => {
                let undoable = self.assistant.conversation.summary(id).is_some_and(|card| {
                    card.after != card.before
                        && card.after == self.engine.revision()
                        && self.engine.undo_step().map(|s| s.line()).as_deref()
                            == Some(card.label.as_str())
                });
                if undoable {
                    self.history(false);
                }
            }
            Action::Fold(folded) => self.assistant.folded = folded,
            Action::Dock(dock) => self.assistant.dock = dock,
            Action::Close => self.assistant.open = false,
            Action::Card(action) => self.workflow_card_action(action),
            // The card is in the cards already; the next frame shows it.
            Action::Offered(_) => {}
            Action::Question { card, choice } => {
                let reply = match choice {
                    assistant::questions::Choice::Send(answers) => AskReply {
                        card,
                        answers,
                        ..AskReply::default()
                    },
                    assistant::questions::Choice::Skip => AskReply {
                        card,
                        skipped: true,
                        ..AskReply::default()
                    },
                };
                self.answer_question(reply);
            }
            Action::ConnectModel => {
                self.settings_window.ai.focus_connection();
                if !self.settings_window.is_open() {
                    self.settings_window.toggle();
                }
            }
        }
    }

    /// The person's answers to the question card the turn waits on: they
    /// are the reply of its `assist.ask` call, and the turn goes on.
    fn answer_question(&mut self, reply: AskReply) {
        let Some(asking) = self
            .assistant
            .asking
            .take_if(|asking| asking.card == reply.card)
        else {
            return;
        };
        let revision = self.engine.revision();
        let value = serde_json::to_value(&reply).unwrap_or_default();
        let _ = asking.request.reply.send(Called {
            envelope: front::envelope(Ok(value)),
            before: revision,
            after: revision,
        });
        self.assistant.conversation.state = State::Running;
    }

    /// Send the composer's message: read what the window shows, open the
    /// message's turn and start the model on it. While a question card
    /// waits, the message answers it in the person's own words instead.
    fn assistant_send(&mut self, ctx: &egui::Context) {
        if let Some(card) = self.assistant.asking.as_ref().map(|a| a.card.clone()) {
            let said = self.assistant.composer.trim().to_owned();
            if said.is_empty() {
                return;
            }
            if let Some(question) = self.assistant.conversation.question_mut(&card) {
                question.answer = Some(assistant::questions::Answered::Said(said.clone()));
            }
            self.assistant.composer.clear();
            self.answer_question(AskReply {
                card,
                said: Some(said),
                ..AskReply::default()
            });
            return;
        }
        if self.assistant.job.is_some()
            || self.assistant.run.is_some()
            || self.assistant.conversation.state.busy()
        {
            return;
        }
        // A path still being typed becomes a chip too.
        self.assistant.chip_paths(ctx, true);
        let text = self.assistant.composer.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let (model, budget, permissions) = match self.chat_model() {
            Ok(chosen) => chosen,
            Err(why) => {
                self.assistant.conversation.state = State::NotSetUp(why);
                return;
            }
        };
        self.assistant.permissions = Some(permissions);
        let (grant, _) = match self.chat_grant() {
            Ok(both) => both,
            Err(why) => {
                self.assistant.conversation.state = State::Error(why);
                return;
            }
        };
        let library = match self.assistant.library() {
            Ok(library) => library.clone(),
            Err(why) => {
                self.assistant.conversation.state = State::Error(why);
                return;
            }
        };
        // The chat may offer a workflow (`assist.offer`) among these.
        let workflows = self.workflow_cards.listed().to_vec();
        self.assistant.session.lend_workflows(Some(workflows));
        let label = turn_label(&text);
        let path = self.files.path().map(std::path::Path::to_path_buf);
        let (mut window, engine, assistant) = self.window_for_assistant(None);
        let (context, begun) = assistant.session.lend(engine, path.as_deref(), |session| {
            let context = front::envelope(session.call(
                CallCx {
                    who: Principal::LocalAgent { grant: &grant },
                    asker: &mut NoOne,
                    verbose: false,
                    window: Some(&mut window),
                },
                call("view.context", json!({})),
            ));
            let begun = front::envelope(session.call(
                CallCx {
                    who: Principal::LocalAgent { grant: &grant },
                    asker: &mut NoOne,
                    verbose: false,
                    window: None,
                },
                call("project.turn_begin", json!({"label": label})),
            ));
            (context, begun)
        });
        let Some(turn) = begun["result"]["turn"].as_str().map(str::to_owned) else {
            let why = begun["error"]["message"]
                .as_str()
                .unwrap_or("the turn could not be opened")
                .to_owned();
            self.assistant.conversation.state = State::Error(why);
            return;
        };
        let context = context["result"].clone();
        self.assistant.turn = Some(OpenTurn {
            id: turn,
            label,
            before: self.engine.document().clone(),
            revision: self.engine.revision(),
        });
        // The message's files: attached to the conversation (a workflow's
        // files input is filled from them) and named to the model.
        let files = std::mem::take(&mut self.assistant.files);
        self.workflow_cards.attach(&files);
        let names: Vec<String> = files
            .iter()
            .map(|p| assistant::mentions::file_name(p))
            .collect();
        self.assistant.conversation.sent(
            text.clone(),
            assistant::context_line(&context),
            names.clone(),
        );
        self.assistant.composer.clear();
        let attached: String = files
            .iter()
            .zip(&names)
            .map(|(path, name)| format!("\n- @{name}: {}", path.display()))
            .collect();
        let text = if attached.is_empty() {
            text
        } else {
            format!("{text}\n\nFiles attached to this message:{attached}")
        };
        // What happened in the chat since the last message: a workflow card
        // declined, a run's result.
        let mut said = std::mem::take(&mut self.assistant.notes);
        said.push(format!("The window shows: {context}\n\n{text}"));
        let message = vec![Part::text(said.join("\n\n"))];
        let config = ChatConfig::new(&library, model.profile.clone(), budget);
        let history = std::mem::take(&mut self.assistant.history);
        let wake = ctx.clone();
        match worker::start(
            config,
            model.connect,
            history,
            message,
            Arc::new(move || wake.request_repaint()),
        ) {
            Ok(job) => self.assistant.job = Some(job),
            Err(why) => self.finish_turn(
                Outcome {
                    end: End::Failed(why),
                    usage: Default::default(),
                },
                Vec::new(),
            ),
        }
    }

    /// What the turn running sent since the last frame: its events, its
    /// tool calls and its end.
    pub(super) fn serve_assistant(&mut self, ctx: &egui::Context) {
        self.serve_online_waits(ctx);
        for _ in 0..PER_FRAME {
            if self.assistant.held.is_some() {
                return;
            }
            let Some(job) = &self.assistant.job else {
                return;
            };
            let message = match job.rx.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => FromTurn::Done {
                    outcome: Outcome {
                        end: End::Failed("the turn ended without an answer".into()),
                        usage: Default::default(),
                    },
                    history: Vec::new(),
                },
            };
            match message {
                FromTurn::Event(event) => self.assistant.conversation.apply(event),
                FromTurn::Tool(request) => self.run_chat_call(ctx, request, false),
                FromTurn::Done { outcome, history } => self.finish_turn(outcome, history),
            }
        }
        // More are waiting: answer them next frame.
        ctx.request_repaint();
    }

    /// Run one of the turn's (or a workflow run's) tool calls on the
    /// window's engine, in its turn, and answer it, or hold it as an Ask
    /// card when the gate asks. `allowed`: the person allowed it on its
    /// card.
    pub(super) fn run_chat_call(
        &mut self,
        ctx: &egui::Context,
        request: ToolRequest,
        allowed: bool,
    ) {
        // A search that asks the registry (`src:online`) waits for it on a
        // thread of its own; the call runs when the answer is in.
        let request = match self.fetch_online(ctx, request, allowed) {
            Some(request) => request,
            None => return,
        };
        let (chat_grant, reach) = match self.chat_grant() {
            Ok(both) => both,
            Err(why) => {
                let _ = request.reply.send(Called::refused(
                    json!({"ok": false, "error": {"code": "chat.unavailable", "message": why}}),
                ));
                return;
            }
        };
        // A run's call runs under the run's grant, in the run's turn; the
        // chat's in the message's turn.
        let (turn, mut grant) = match &request.run {
            Some(run) => (run.turn.clone(), run.grant.clone()),
            None => match self.assistant.turn.as_ref() {
                Some(turn) => (Some(turn.id.clone()), chat_grant),
                None => return,
            },
        };
        let permissions = self.assistant.permissions.unwrap_or(Permissions {
            edits: Rule::Always,
            files: Rule::Ask,
            publish_library: Rule::Ask,
            view: Rule::Ask,
        });
        // Moving the person's view follows its own rule: the gate never
        // asks about it.
        let shown = request.input["name"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if request.verb == sparksmith_assist::verbs::SHOW
            && shown != ASK
            && !allowed
            && !self.assistant.allowed_session.contains(&shown)
        {
            match permissions.view {
                Rule::Always => {}
                Rule::Deny => {
                    let _ = request.reply.send(Called::refused(json!({"ok": false, "error": {
                        "code": "gate.denied",
                        "message": "The person does not let the assistant move their view (Settings › AI). Nothing was changed."}})));
                    return;
                }
                Rule::Ask | Rule::Session => {
                    let question = Question {
                        command: shown,
                        class: sparksmith_core::Class::Show,
                        paths: Vec::new(),
                        why: "It moves your view.".to_owned(),
                    };
                    self.hold(request, question);
                    return;
                }
            }
        }
        let busy = if allowed { None } else { self.live_busy(ctx) };
        let host = self.files.host_file();
        let mut unasked: Vec<String> = reach.unasked.clone();
        unasked.extend(self.assistant.allowed_session.iter().cloned());
        let mut asker = ChatAsker {
            allowed,
            unasked,
            permissions,
            question: None,
        };
        if permissions.edits != Rule::Always {
            // Edits follow the person's rule too: the gate asks about each.
            grant.mode = sparksmith_automation::Mode::AskEveryTime;
        }
        let (mut window, engine, assistant) = self.window_for_assistant(busy);
        let entry = request.run.as_ref().is_some_and(|run| run.entry);
        let (reply, changes) = assistant.session.lend_file(engine, &host, |session| {
            let input = request.input.clone();
            if entry {
                let before = session.engine().revision();
                let envelope = desk::call_on(
                    session,
                    &grant,
                    turn.as_deref(),
                    Some(&mut window),
                    &mut asker,
                    &request.verb,
                    input,
                );
                Called {
                    envelope,
                    before,
                    after: session.engine().revision(),
                }
            } else {
                desk::verb_on(
                    session,
                    &grant,
                    turn.as_deref(),
                    Some(&mut window),
                    &mut asker,
                    &request.verb,
                    input,
                )
            }
        });
        // An open, new project or save the call made: the file workflow
        // takes it on with the engine (review finding 8).
        self.file_changes(changes);
        let asked = self.asks.shown.take();
        match (asker.question, asked) {
            (Some(question), _) if !allowed => self.hold(request, question),
            // A question card shown: the call waits for the person's answers.
            (_, Some((card, params))) if reply.envelope["ok"] == true => {
                let question = assistant::questions::QuestionCard::new(card.clone(), &params);
                self.assistant.conversation.push(Item::Question(question));
                self.assistant.conversation.state = State::Waiting;
                self.assistant.asking = Some(Asking { request, card });
            }
            _ => {
                let _ = request.reply.send(reply);
            }
        }
    }

    /// F1: a `library.search` with `src:online` asks the registry on a
    /// thread of its own and waits in [`crate::assistant::OnlineWait`], so
    /// the window never waits on the network; `None` then. Any other call,
    /// a search while no registry runs, or one whose answer is in
    /// (`prefetched`), comes back to run now.
    fn fetch_online(
        &mut self,
        ctx: &egui::Context,
        request: ToolRequest,
        allowed: bool,
    ) -> Option<ToolRequest> {
        use sparksmith_automation::session::online::{self as reg, OnlineSource};
        if self.assistant.prefetched {
            return Some(request);
        }
        let Some(text) = reg::online_search(&request.verb, &request.input) else {
            return Some(request);
        };
        let Some(source) = self
            .sources
            .online()
            .map(|o| o.client() as Arc<dyn OnlineSource>)
        else {
            return Some(request);
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        // A thread that cannot start drops `tx`, so the wait ends at once
        // and the call says the registry did not answer.
        let _ = std::thread::Builder::new()
            .name("chat-online-search".into())
            .spawn(move || {
                let _ = tx.send(source.search(&text, reg::MAX_ONLINE_ROWS));
                wake.request_repaint();
            });
        self.assistant.online_waits.push(assistant::OnlineWait {
            request,
            allowed,
            report: rx,
        });
        None
    }

    /// Run each online search whose registry answer came in, with that
    /// answer installed for its one call (`online::Prefetched`).
    fn serve_online_waits(&mut self, ctx: &egui::Context) {
        use sparksmith_automation::session::online::{self as reg, OnlineSource};
        let mut k = 0;
        while k < self.assistant.online_waits.len() {
            let report = match self.assistant.online_waits[k].report.try_recv() {
                Ok(report) => report,
                Err(TryRecvError::Empty) => {
                    k += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => {
                    Err("the registry search stopped without an answer".into())
                }
            };
            let wait = self.assistant.online_waits.remove(k);
            let real = self
                .sources
                .online()
                .map(|o| o.client() as Arc<dyn OnlineSource>);
            if let (Some(source), Some(text)) = (
                real.clone(),
                reg::online_search(&wait.request.verb, &wait.request.input),
            ) {
                self.assistant
                    .session
                    .set_online(Some(Arc::new(reg::Prefetched {
                        source,
                        text,
                        report,
                    })));
            }
            self.assistant.prefetched = true;
            self.run_chat_call(ctx, wait.request, wait.allowed);
            self.assistant.prefetched = false;
            self.assistant.session.set_online(real);
        }
    }

    /// Hold `request` as an Ask card about `question`.
    fn hold(&mut self, request: ToolRequest, question: Question) {
        let conversation = &mut self.assistant.conversation;
        let id = conversation.next_id();
        let command = sparksmith_automation::session::metadata::find(&question.command)
            .map_or_else(|| question.command.clone(), |entry| entry.title.to_owned());
        conversation.push(Item::Ask(AskCard {
            id,
            command,
            why: question.why.clone(),
            paths: question.paths.clone(),
            answer: None,
        }));
        conversation.state = State::Waiting;
        self.assistant.held = Some(Held {
            request,
            ask: id,
            question,
        });
    }

    /// The person's answer on Ask card `id`: Allow once, for this session
    /// or always (the entries asked about then run without asking), or Deny.
    fn assistant_answer(&mut self, ctx: &egui::Context, id: u64, allow: Allow) {
        let Some(held) = self.assistant.held.take_if(|held| held.ask == id) else {
            return;
        };
        if let Some(card) = self.assistant.conversation.ask_mut(id) {
            card.answer = Some(allow);
        }
        self.assistant.conversation.state = State::Running;
        let entries = held.question.command.split(", ").map(str::to_owned);
        let kind = Kind::of(&held.question);
        // Settings › AI's "once a session": the first Allow covers the rest.
        let session_rule = self
            .assistant
            .permissions
            .is_some_and(|p| kind.rule(&p) == Rule::Session);
        match allow {
            Allow::Session => self.assistant.allowed_session.extend(entries),
            Allow::Once if session_rule => self.assistant.allowed_session.extend(entries),
            Allow::Always => self.always_allow(kind),
            Allow::Once | Allow::Deny => {}
        }
        if allow != Allow::Deny {
            self.run_chat_call(ctx, held.request, true);
        } else {
            let _ = held
                .request
                .reply
                .send(Called::refused(sparksmith_automation::gate::failure(
                    &held.question.denied(),
                )));
        }
    }

    /// The turn ended: close it, and add its summary card.
    fn finish_turn(&mut self, outcome: Outcome, history: Vec<Message>) {
        self.assistant.job = None;
        self.assistant.held = None;
        if let Some(asking) = self.assistant.asking.take()
            && let Some(card) = self.assistant.conversation.question_mut(&asking.card)
        {
            card.answer = Some(assistant::questions::Answered::Stopped);
        }
        self.assistant.history = history;
        let Some(turn) = self.assistant.turn.take() else {
            return;
        };
        if let Ok((grant, _)) = self.chat_grant() {
            let path = self.files.path().map(std::path::Path::to_path_buf);
            let (_, engine, assistant) = self.window_for_assistant(None);
            assistant.session.lend(engine, path.as_deref(), |session| {
                let _ = session.call(
                    CallCx {
                        who: Principal::LocalAgent { grant: &grant },
                        asker: &mut NoOne,
                        verbose: false,
                        window: None,
                    },
                    call("project.turn_end", json!({"turn": turn.id})),
                );
            });
        }
        let difference = sparksmith_automation::session::difference(
            &turn.before,
            self.engine.document(),
            self.engine.check_settings(),
        )
        .unwrap_or_default();
        let id = self.assistant.conversation.next_id();
        let card = summary_card(id, &turn, &difference, &outcome, &self.engine);
        self.assistant.conversation.ended(card, &outcome.end);
    }

    /// Select what the turn of summary `id` changed, as the person.
    fn assistant_show(&mut self, id: u64) {
        let Some(ids) = self
            .assistant
            .conversation
            .summary(id)
            .map(|card| card.show.clone())
        else {
            return;
        };
        let path = self.files.path().map(std::path::Path::to_path_buf);
        let (mut window, engine, assistant) = self.window_for_assistant(None);
        let shown = assistant.session.lend(engine, path.as_deref(), |session| {
            session.call(
                CallCx {
                    who: Principal::Person,
                    asker: &mut NoOne,
                    verbose: false,
                    window: Some(&mut window),
                },
                call("view.select", json!({"ids": ids})),
            )
        });
        if let Err(error) = shown {
            self.say(Level::Warning, error.message);
        }
    }
}

/// The summary card of `turn`, which ended with `outcome`, from what
/// `difference` found between the document before it and now.
fn summary_card(
    id: u64,
    turn: &OpenTurn,
    difference: &Value,
    outcome: &Outcome,
    engine: &sparksmith_core::Engine,
) -> TurnSummary {
    let changes = &difference["changes"];
    let names = &difference["names"];
    let mut lines = Vec::new();
    let mut show = Vec::new();
    for (kind, word) in [
        ("added", "added"),
        ("changed", "changed"),
        ("removed", "removed"),
    ] {
        let Some(lists) = changes[kind].as_object() else {
            continue;
        };
        for (path, ids) in lists {
            let ids = ids.as_array().map(Vec::as_slice).unwrap_or_default();
            let named: Vec<&str> = ids
                .iter()
                .filter_map(|id| names[id.to_string()].as_str())
                .collect();
            let what =
                if !named.is_empty() && named.len() == ids.len() && ids.len() <= NAMED_OBJECTS {
                    named.join(", ")
                } else {
                    ids.len().to_string()
                };
            lines.push(format!("{}: {what} {word}", collection(path)));
            if kind != "removed" {
                show.extend(ids.iter().filter_map(Value::as_u64).filter(|id| {
                    sparksmith_automation::view::object_of(engine.project(), *id).is_some()
                }));
            }
        }
    }
    if changes["fields"].as_array().is_some_and(|f| !f.is_empty()) {
        lines.push("Project settings changed".to_owned());
    }
    show.truncate(sparksmith_automation::view::MAX_SELECT);
    let findings = &difference["findings"];
    let new: Vec<&Value> = findings["new"].as_array().into_iter().flatten().collect();
    let issues_new = new.len() + findings["omitted"].as_u64().unwrap_or(0) as usize;
    let issues_resolved = findings["resolved"].as_object().map_or(0, |r| {
        r.values().filter_map(Value::as_u64).sum::<u64>() as usize
    });
    let end = match &outcome.end {
        End::Answered => None,
        End::Stopped => Some("Stopped by you.".to_owned()),
        other => Some(format!("{}.", capital(&other.to_string()))),
    };
    TurnSummary {
        id,
        label: turn.label.clone(),
        changes: lines,
        issues_new,
        issues_resolved,
        new_issues: new
            .iter()
            .filter_map(|f| f["message"].as_str().map(str::to_owned))
            .take(NAMED_ISSUES)
            .collect(),
        usage: outcome.usage,
        before: turn.revision,
        after: engine.revision(),
        show,
        end,
    }
}

/// A collection's path as a person reads it: "layout.tracks" is "Tracks".
fn collection(path: &str) -> String {
    let last = path.rsplit('.').next().unwrap_or(path);
    let words = match last {
        "instances" => "Parts".to_owned(),
        "items" if path.starts_with("drafts") => "Drafts".to_owned(),
        other => other.replace('_', " "),
    };
    capital(&words)
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_is_labelled_by_its_message() {
        assert_eq!(
            turn_label("Add two vias\nnear U1"),
            "AI assistant: Add two vias"
        );
        let long = "x".repeat(LABEL_CHARS + 5);
        assert_eq!(
            turn_label(&long),
            format!("AI assistant: {}…", "x".repeat(LABEL_CHARS))
        );
    }

    #[test]
    fn collections_read_as_words() {
        assert_eq!(collection("vias"), "Vias");
        assert_eq!(collection("instances"), "Parts");
        assert_eq!(collection("drafts.items"), "Drafts");
        assert_eq!(collection("part_dimensions"), "Part dimensions");
    }
}

#[cfg(test)]
#[path = "assistant_tests.rs"]
mod assistant_tests;

#[cfg(test)]
#[path = "assistant_ask_tests.rs"]
mod assistant_ask_tests;

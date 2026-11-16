//! Settings › AI (`docs/assistant-panel.md` §6.4). First, **Connection**:
//! where the chat's model comes from (Anthropic, OpenAI, a model server on
//! this computer, or an agent installed on this computer), and for a
//! hosted model one API key field, the model and Test connection. Then,
//! folded: **Advanced** (every connection, with server addresses,
//! timeouts, declared capabilities and agent commands), the permissions per
//! kind of action, a model run's limits, the run profiles and the
//! workflows. There are no per-turn caps on edits or calls.
//!
//! What is set here is the person's `<config>/assist/settings.json`
//! (`sparksmith_assist::settings`), written at once and checked first; the
//! values it starts with, and Connection's choices, ship in the library. A
//! key is typed or pasted into a masked field (shown only while typing,
//! when asked), goes to the system keychain for the connection's base URL
//! when it is saved, and is never shown again, written to a file or kept in
//! the form; a key for plain HTTP to another computer is refused before it
//! is stored. Test connection sends one short request off the UI thread
//! (`sparksmith_assist::model::check_connection`). An agent is
//! configuration only: nothing here starts it. Profiles are files: a
//! built-in one is copied to edit, and the text is checked when it is
//! saved. Workflows are listed read-only, with why one can't run.
use crate::kit;
use sparksmith_assist::AssistLibrary;
use sparksmith_assist::model::{Endpoint, KeySource, Secret, keys};
use sparksmith_assist::profile as run_profile;
use sparksmith_assist::settings::{
    self as stored, AgentSetting, AssistSettings, CONTEXT_RANGE, EndpointSetting, OUTPUT_RANGE,
    ProfileEntry, ProfileSource, Rule, Shipped, TIMEOUT_RANGE, Wire,
};
use sparksmith_assist::workflow::{self, HostCatalog, Listed, Source, load};
use sparksmith_ui::{ui_kit, ui_kit::Button};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The system keychain, or a stand-in that never reaches it (tests).
#[derive(Clone, Copy)]
pub(crate) struct Keyring {
    has: fn(&Endpoint) -> bool,
    store: fn(&Endpoint, &Secret) -> Result<(), String>,
    delete: fn(&Endpoint) -> Result<bool, String>,
}

#[cfg_attr(test, allow(dead_code))]
fn system_store(endpoint: &Endpoint, key: &Secret) -> Result<(), String> {
    keys::keychain_store(endpoint, key).map_err(|e| e.to_string())
}

#[cfg_attr(test, allow(dead_code))]
fn system_delete(endpoint: &Endpoint) -> Result<bool, String> {
    keys::keychain_delete(endpoint).map_err(|e| e.to_string())
}

impl Keyring {
    /// Whether it holds a key for `endpoint`.
    pub(crate) fn has(&self, endpoint: &Endpoint) -> bool {
        (self.has)(endpoint)
    }

    #[cfg_attr(test, allow(dead_code))]
    const SYSTEM: Self = Self {
        has: keys::keychain_has,
        store: system_store,
        delete: system_delete,
    };
}

#[cfg(test)]
pub(crate) mod test_keyring {
    //! A keychain in memory, per test thread.
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    thread_local! {
        pub(crate) static KEYS: RefCell<BTreeMap<String, String>> = RefCell::default();
    }

    fn has(e: &Endpoint) -> bool {
        KEYS.with(|k| k.borrow().contains_key(e.base()))
    }
    fn store(e: &Endpoint, key: &Secret) -> Result<(), String> {
        // The stand-in keeps that a key was stored, never its text.
        let _ = key;
        KEYS.with(|k| k.borrow_mut().insert(e.base().to_owned(), "stored".into()));
        Ok(())
    }
    fn delete(e: &Endpoint) -> Result<bool, String> {
        Ok(KEYS.with(|k| k.borrow_mut().remove(e.base()).is_some()))
    }

    pub(crate) const MEMORY: Keyring = Keyring { has, store, delete };

    /// Test connection without a network: the model answers when a key
    /// came with the request or the server is on this computer.
    pub(crate) fn check(setting: &EndpointSetting, key: Option<Secret>) -> Result<String, String> {
        let local = setting.endpoint().is_ok_and(|e| e.is_loopback());
        if key.is_some() || local {
            Ok(setting.model.trim().to_owned())
        } else {
            Err("401: invalid x-api-key".into())
        }
    }
}

/// Test connection's one request, run off the UI thread: the setting, and
/// the key typed (none: the keychain's, when there is one). The model that
/// answered, or why not.
pub(crate) type Checker = fn(&EndpointSetting, Option<Secret>) -> Result<String, String>;

#[cfg_attr(test, allow(dead_code))]
fn system_check(setting: &EndpointSetting, typed: Option<Secret>) -> Result<String, String> {
    let profile = setting.model_profile().map_err(|e| e.to_string())?;
    let endpoint = setting.endpoint().map_err(|e| e.to_string())?;
    let key = match typed {
        Some(key) => Some(key),
        None if endpoint.check_key_allowed().is_ok() => {
            KeySource::Keychain.resolve(&endpoint).ok().flatten()
        }
        None => None,
    };
    sparksmith_assist::model::check_connection(&profile, endpoint, key).map_err(|e| e.to_string())
}

/// Whether the chat can use what is set up: a model connection with its
/// key (in the keychain, or for Anthropic in `ANTHROPIC_API_KEY`), or a
/// server on this computer. Why not, in the person's words.
pub(crate) fn readiness(settings: &AssistSettings, keyring: Keyring) -> Result<(), String> {
    let Some(setting) = settings.run_endpoint() else {
        return Err(NOT_CONNECTED.into());
    };
    setting
        .model_profile()
        .map_err(|_| NOT_CONNECTED.to_owned())?;
    let endpoint = setting.endpoint().map_err(|_| NOT_CONNECTED.to_owned())?;
    let env = setting.provider == Wire::Anthropic
        && cfg!(not(test))
        && std::env::var("ANTHROPIC_API_KEY").is_ok_and(|v| !v.trim().is_empty());
    if endpoint.is_loopback() || has_key(keyring, setting) || env {
        Ok(())
    } else {
        Err(NOT_CONNECTED.into())
    }
}

/// What the chat says when no model is connected.
pub(crate) const NOT_CONNECTED: &str = "No model connected";

/// What Connection uses: a shipped choice of model, an agent, or one of
/// the person's own model connections that no choice matches (set up
/// under Advanced).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Model(usize),
    Agent(usize),
    Saved(usize),
}

/// Test connection's state.
enum Check {
    Idle,
    Running(std::sync::mpsc::Receiver<Result<String, String>>),
    Passed(String),
    Failed(String),
}

/// Connection's form: the model connection being set up, its key as typed
/// (never stored here past Save) and the test's state.
struct Quick {
    choice: Choice,
    /// The connection's place among the saved ones; none: a new one.
    at: Option<usize>,
    draft: EndpointSetting,
    key: String,
    /// The typed key shows (while typing only).
    show: bool,
    /// The keychain holds a key for the draft's URL.
    stored: bool,
    /// Replace was chosen: the key field shows over a stored key.
    replacing: bool,
    check: Check,
}

/// Whether the shipped choice `preset` is the saved connection `e`: the
/// same kind of server at the same address, or both on this computer.
fn same_server(preset: &EndpointSetting, e: &EndpointSetting) -> bool {
    let (Ok(a), Ok(b)) = (preset.endpoint(), e.endpoint()) else {
        return false;
    };
    preset.provider == e.provider && (a.base() == b.base() || (a.is_loopback() && b.is_loopback()))
}

impl Quick {
    /// The form for what the chat uses now; the first choice when nothing
    /// is set up.
    fn read(state: &State, settings: &AssistSettings, shipped: &Shipped, keyring: Keyring) -> Self {
        let agent = settings
            .connection
            .as_deref()
            .and_then(|name| settings.agents.iter().find(|a| a.name == name))
            .and_then(|a| {
                shipped
                    .agent_presets
                    .iter()
                    .position(|p| p.agent.command == a.command)
            });
        if let Some(index) = agent {
            return Self::open(Choice::Agent(index), state, settings, shipped, keyring);
        }
        let saved = settings.run_endpoint().and_then(|e| {
            let at = settings.endpoints.iter().position(|s| s == e)?;
            Some(
                match shipped
                    .endpoint_presets
                    .iter()
                    .position(|p| same_server(&p.endpoint, e))
                {
                    Some(preset) => Choice::Model(preset),
                    None => Choice::Saved(at),
                },
            )
        });
        Self::open(
            saved.unwrap_or(Choice::Model(0)),
            state,
            settings,
            shipped,
            keyring,
        )
    }

    /// The form for `choice`: a saved connection it matches, else a new
    /// one from the shipped choice with its default model.
    fn open(
        choice: Choice,
        state: &State,
        settings: &AssistSettings,
        shipped: &Shipped,
        keyring: Keyring,
    ) -> Self {
        let preset = match choice {
            Choice::Model(i) => shipped.endpoint_presets.get(i).map(|p| &p.endpoint),
            _ => None,
        };
        let (at, draft) = match (choice, preset) {
            (Choice::Saved(i), _) => (Some(i), settings.endpoints.get(i).cloned()),
            (_, Some(preset)) => {
                // The connection the chat uses first, then any other.
                let active = settings.run_endpoint();
                let found = active
                    .filter(|e| same_server(preset, e))
                    .or_else(|| settings.endpoints.iter().find(|e| same_server(preset, e)));
                match found {
                    Some(e) => (
                        settings.endpoints.iter().position(|s| s == e),
                        Some(e.clone()),
                    ),
                    None => {
                        let mut draft = preset.clone();
                        draft.name = settings.free_name(&draft.name);
                        if draft.model.trim().is_empty()
                            && draft.provider == Wire::Anthropic
                            && let Ok(library) = &state.library
                        {
                            draft.model = library.default_model.clone();
                        }
                        (None, Some(draft))
                    }
                }
            }
            _ => (None, None),
        };
        let draft = draft
            .or_else(|| shipped.endpoint_presets.first().map(|p| p.endpoint.clone()))
            .unwrap_or_else(|| EndpointSetting {
                name: String::new(),
                provider: Wire::Anthropic,
                base_url: String::new(),
                model: String::new(),
                timeout_minutes: *TIMEOUT_RANGE.start(),
                capabilities: None,
            });
        Self {
            choice,
            at,
            stored: has_key(keyring, &draft),
            draft,
            key: String::new(),
            show: false,
            replacing: false,
            check: Check::Idle,
        }
    }

    /// The draft's server is on this computer.
    fn local(&self) -> bool {
        self.draft.endpoint().is_ok_and(|e| e.is_loopback())
    }
}

/// The choices for one kind of action. An ask itself offers Allow once,
/// Allow for this session, Always allow and Deny.
fn rules() -> [(Rule, String); 4] {
    [
        (Rule::Ask, "Ask every time".to_owned()),
        (Rule::Session, "Ask once a session".to_owned()),
        (Rule::Always, "Always allow".to_owned()),
        (Rule::Deny, "Deny".to_owned()),
    ]
}

/// An endpoint being added or changed.
struct EndpointEdit {
    /// Its place in the list; none for a new one.
    at: Option<usize>,
    draft: EndpointSetting,
    /// Typed once; taken out of the form when the endpoint is saved.
    key: String,
    /// Whether the keychain holds a key for the draft's URL as opened.
    stored: bool,
}

/// An agent being added or changed; arguments one per line, variables
/// `NAME=value` one per line.
struct AgentEdit {
    at: Option<usize>,
    draft: AgentSetting,
    args: String,
    env: String,
}

enum Editing {
    Endpoint(Box<EndpointEdit>),
    Agent(Box<AgentEdit>),
}

/// The page as read when the window opened.
struct State {
    config: Option<PathBuf>,
    settings: Result<AssistSettings, String>,
    shipped: Result<Shipped, String>,
    library: Result<AssistLibrary, String>,
    profiles: Vec<ProfileEntry>,
    workflows: Vec<Listed>,
    /// Connection's form; read again after Advanced changes a connection.
    quick: Option<Quick>,
    editing: Option<Editing>,
    /// The user profile being edited: its name and text.
    profile: Option<(String, String)>,
    /// The last refusal, shown once at the top of the page.
    refusal: Option<String>,
    /// Lists showing every row.
    show_all: BTreeSet<&'static str>,
}

impl State {
    fn read(config: Option<PathBuf>) -> Self {
        let settings = match &config {
            Some(c) => stored::read(&stored::user_file(c)).map_err(|e| e.to_string()),
            None => Err("No config folder: set SPARKSMITH_CONFIG_DIR or a home folder".into()),
        };
        let shipped_library = sparksmith_io::shipped::ShippedLibrary::locate().ok();
        let library = AssistLibrary::locate().map_err(|e| e.to_string());
        let shipped_workflows = shipped_library.as_ref().map(load::shipped_folder);
        let user_workflows = config.as_deref().map(load::user_folder);
        let workflows = load::list(
            shipped_workflows.as_deref(),
            user_workflows.as_deref(),
            &HostCatalog,
        );
        let mut state = Self {
            settings,
            shipped: Shipped::locate().map_err(|e| e.to_string()),
            profiles: Vec::new(),
            workflows,
            library,
            config,
            quick: None,
            editing: None,
            profile: None,
            refusal: None,
            show_all: BTreeSet::new(),
        };
        state.reload_profiles();
        state
    }

    fn profile_folder(&self) -> Option<PathBuf> {
        self.config.as_deref().map(run_profile::user_folder)
    }

    fn reload_profiles(&mut self) {
        let folder = self.profile_folder();
        self.profiles = match &self.library {
            Ok(library) => stored::profiles(library, folder.as_deref()),
            Err(_) => Vec::new(),
        };
    }

    /// Check and write `next`; the refusal is kept when it fails.
    fn commit(&mut self, next: AssistSettings) -> bool {
        let Some(config) = &self.config else {
            self.refusal = Some("No config folder to save to".into());
            return false;
        };
        match stored::write(&stored::user_file(config), &next) {
            Ok(()) => {
                // Connections may have changed: Connection reads them again.
                if self.settings.as_ref().ok().is_none_or(|s| {
                    s.endpoints != next.endpoints
                        || s.agents != next.agents
                        || s.connection != next.connection
                }) {
                    self.quick = None;
                }
                self.settings = Ok(next);
                self.refusal = None;
                true
            }
            Err(e) => {
                self.refusal = Some(e.to_string());
                false
            }
        }
    }
}

/// Settings › AI's state while the Settings window is open.
pub(crate) struct AiPage {
    /// The config folder; none: the user's, found when the page is read.
    config: Option<PathBuf>,
    keyring: Keyring,
    checker: Checker,
    state: Option<State>,
    /// Connection is to show, opened and in view, with the key field
    /// focused (the chat's Connect a model…).
    focus: u8,
    /// Connection saved what the chat uses since the app last asked.
    connected: bool,
}

impl Default for AiPage {
    fn default() -> Self {
        Self {
            config: None,
            #[cfg(not(test))]
            keyring: Keyring::SYSTEM,
            #[cfg(test)]
            keyring: test_keyring::MEMORY,
            #[cfg(not(test))]
            checker: system_check,
            #[cfg(test)]
            checker: test_keyring::check,
            state: None,
            focus: 0,
            connected: false,
        }
    }
}

impl AiPage {
    /// The page over the config folder `config` (tests).
    #[cfg(test)]
    pub fn at(config: PathBuf) -> Self {
        Self {
            config: Some(config),
            ..Self::default()
        }
    }

    /// Read again when next shown: the window opened or closed. A key
    /// typed and not saved goes with the form.
    pub fn reset(&mut self) {
        self.state = None;
    }

    /// The config folder Settings › AI reads and writes: the page's own
    /// (tests), else the user's.
    pub fn config_dir(&self) -> Option<PathBuf> {
        self.config.clone().or_else(|| {
            sparksmith_io::settings::UserDirs::from_env()
                .ok()
                .map(|d| d.config)
        })
    }

    pub fn keyring(&self) -> Keyring {
        self.keyring
    }

    /// Show Connection next: opened, in view, the key field focused.
    pub fn focus_connection(&mut self) {
        self.focus = FOCUS_FRAMES;
    }

    /// Whether Connection is to be brought into view this frame.
    pub fn wants_focus(&self) -> bool {
        self.focus > 0
    }

    /// Whether Connection saved what the chat uses since the last ask.
    pub fn take_connected(&mut self) -> bool {
        std::mem::take(&mut self.connected)
    }

    fn state(&mut self) -> &mut State {
        let config = self.config_dir();
        self.state.get_or_insert_with(|| State::read(config))
    }

    /// The page, in the Settings window's AI section.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let keyring = self.keyring;
        let checker = self.checker;
        let focus = self.focus > 0;
        self.focus = self.focus.saturating_sub(1);
        let mut connected = false;
        let state = self.state();
        if let Some(why) = &state.refusal {
            kit::refusal_wrapped(ui, why);
        }
        let settings = match &state.settings {
            Ok(settings) => settings.clone(),
            Err(why) => {
                kit::refusal_wrapped(ui, why);
                kit::hint(
                    ui,
                    "Fix or remove that file to change these settings",
                    "Nothing is saved over a file that can't be read.",
                );
                workflows(ui, state);
                return;
            }
        };
        let shipped = match &state.shipped {
            Ok(shipped) => shipped.clone(),
            Err(why) => {
                kit::refusal_wrapped(ui, why);
                return;
            }
        };
        if focus {
            kit::open_section(ui, CONNECTION);
        }
        kit::section(ui, CONNECTION, "Connection", true, None, |ui| {
            connected = connection(ui, state, &settings, &shipped, keyring, checker, focus);
        });
        // Everything else is folded until the person opens it.
        let count = settings.endpoints.len() + settings.agents.len();
        kit::section(
            ui,
            "settings-ai-connections",
            "Advanced",
            false,
            Some(count),
            |ui| connections(ui, state, &settings, &shipped, keyring),
        );
        kit::section(
            ui,
            "settings-ai-permissions",
            "Permissions",
            false,
            None,
            |ui| permissions(ui, state, &settings, &shipped),
        );
        kit::section(
            ui,
            "settings-ai-run-limits",
            "Run limits",
            false,
            None,
            |ui| limits(ui, state, &settings, &shipped),
        );
        let profile_count = state.profiles.len();
        kit::section(
            ui,
            "settings-ai-profiles",
            "Profiles",
            false,
            Some(profile_count),
            |ui| profiles(ui, state),
        );
        workflows(ui, state);
        self.connected |= connected;
    }
}

/// Frames Connect a model… keeps Connection in view and the key field
/// focused: a window that just opened lays itself out first.
const FOCUS_FRAMES: u8 = 3;

/// Connection's section id.
const CONNECTION: &str = "settings-ai-connection";

/// What Connection says of a key in the keychain.
const STORED: &str = "Key stored in the system keychain";

/// The key field's id, for Paste and Connect a model….
fn key_field_id() -> egui::Id {
    egui::Id::unique("settings-ai-key")
}

/// Connection: what the chat uses, in as few fields as it takes. Whether
/// it saved what the chat uses.
fn connection(
    ui: &mut egui::Ui,
    state: &mut State,
    settings: &AssistSettings,
    shipped: &Shipped,
    keyring: Keyring,
    checker: Checker,
    focus: bool,
) -> bool {
    let mut quick = match state.quick.take() {
        Some(quick) => quick,
        None => Quick::read(state, settings, shipped, keyring),
    };
    // The choices: the shipped models, then the agents, then a connection
    // of the person's own that none of them is.
    let mut options: Vec<(Choice, String)> = shipped
        .endpoint_presets
        .iter()
        .enumerate()
        .map(|(i, p)| (Choice::Model(i), p.title.clone()))
        .chain(
            shipped
                .agent_presets
                .iter()
                .enumerate()
                .map(|(i, p)| (Choice::Agent(i), format!("{} on this computer", p.title))),
        )
        .collect();
    if let Choice::Saved(i) = quick.choice
        && let Some(e) = settings.endpoints.get(i)
    {
        options.push((quick.choice, format!("{} (from Advanced)", e.name)));
    }
    let mut choice = quick.choice;
    if kit::choice_row(ui, "Use", &mut choice, &options) {
        quick = Quick::open(choice, state, settings, shipped, keyring);
    }
    let saved = match quick.choice {
        Choice::Agent(index) => agent_choice(ui, state, settings, shipped, index),
        Choice::Model(_) | Choice::Saved(_) => model_choice(
            ui, state, settings, shipped, keyring, checker, &mut quick, focus,
        ),
    };
    if saved {
        // Read again from what was saved; the test's outcome stays.
        let check = std::mem::replace(&mut quick.check, Check::Idle);
        if let Ok(now) = state.settings.clone() {
            quick = Quick::read(state, &now, shipped, keyring);
        }
        quick.check = check;
    }
    state.quick = Some(quick);
    saved
}

/// An agent as the connection: one click, no form. Whether it was saved.
fn agent_choice(
    ui: &mut egui::Ui,
    state: &mut State,
    settings: &AssistSettings,
    shipped: &Shipped,
    index: usize,
) -> bool {
    let Some(preset) = shipped.agent_presets.get(index) else {
        return false;
    };
    let title = &preset.title;
    let saved = settings
        .agents
        .iter()
        .position(|a| a.command == preset.agent.command);
    let active = saved
        .and_then(|i| settings.agents.get(i))
        .is_some_and(|a| settings.connection.as_deref() == Some(a.name.as_str()));
    kit::hint(
        ui,
        &format!("Uses {title} as installed on this computer; it signs in itself"),
        "Nothing to paste here: the agent keeps its own sign-in, and its key never comes to this app. Its command is under Advanced.",
    );
    kit::hint(
        ui,
        "The chat can't start agents yet; it uses a model until then",
        "Agents in the chat come in a later version. Choose a model above to chat today.",
    );
    let mut clicked = false;
    kit::actions(ui, |ui| {
        clicked = kit::primary(ui, &format!("Use {title}"), !active).clicked();
    });
    if active {
        kit::note(ui, &format!("Set: {title} is the connection."));
    }
    if !clicked {
        return false;
    }
    let mut next = settings.clone();
    let name = match saved.and_then(|i| next.agents.get(i)) {
        Some(a) => a.name.clone(),
        None => {
            let mut agent = preset.agent.clone();
            agent.name = settings.free_name(&agent.name);
            let name = agent.name.clone();
            next.agents.push(agent);
            name
        }
    };
    next.connection = Some(name);
    state.commit(next)
}

/// A model as the connection: the server's address only for a server on
/// this computer, one API key field for a hosted one, the model, Test
/// connection and Save. Whether it was saved.
#[allow(clippy::too_many_arguments)]
fn model_choice(
    ui: &mut egui::Ui,
    state: &mut State,
    settings: &AssistSettings,
    shipped: &Shipped,
    keyring: Keyring,
    checker: Checker,
    quick: &mut Quick,
    focus: bool,
) -> bool {
    let preset_local = match quick.choice {
        Choice::Model(i) => shipped
            .endpoint_presets
            .get(i)
            .is_some_and(|p| p.endpoint.endpoint().is_ok_and(|e| e.is_loopback())),
        _ => true,
    };
    if preset_local {
        let parsed = quick.draft.endpoint();
        let field = kit::checked_text_row(
            ui,
            "Server address",
            &mut quick.draft.base_url,
            "http://localhost:11434/v1",
            parsed.is_err(),
        )
        .on_hover_text("The server's base URL, as its documentation gives it");
        if let Err(why) = &parsed {
            kit::field_error(ui, &field, &why.to_string(), false);
        }
    }
    let local = quick.local();
    let mut focus_key = focus;
    let mut changed = false;
    if !local {
        if quick.stored && !quick.replacing {
            let mut replace = false;
            let mut remove = false;
            // The key's state, then its two actions under it: one line is
            // too narrow in a small window.
            kit::value_row(ui, "API key", STORED);
            kit::row(ui, "", |ui| {
                ui.horizontal(|ui| {
                    replace = ui_kit::button(ui, "Replace", Button::Small)
                        .on_hover_text("Paste a new key in its place")
                        .clicked();
                    remove = ui_kit::button(ui, "Remove", Button::Small)
                        .on_hover_text("Remove the key from the system keychain")
                        .clicked();
                });
            });
            if replace {
                quick.replacing = true;
                focus_key = true;
            }
            if remove {
                match quick.draft.endpoint().map_err(|e| e.to_string()) {
                    Ok(e) => match (keyring.delete)(&e) {
                        Ok(_) => {
                            quick.stored = false;
                            changed = true;
                            quick.check = Check::Idle;
                            state.refusal = None;
                        }
                        Err(why) => state.refusal = Some(why),
                    },
                    Err(why) => state.refusal = Some(why),
                }
            }
        }
        if !quick.stored || quick.replacing {
            key_row(ui, quick, focus_key);
        }
    }
    kit::checked_text_row(
        ui,
        "Model",
        &mut quick.draft.model,
        "The model's name, as your provider lists it",
        false,
    )
    .on_hover_text("The model's id, exactly as the provider or server names it");
    // What Test and Save need, and why not.
    let needs_key = !local && !quick.stored && quick.key.trim().is_empty();
    let missing = if quick.draft.model.trim().is_empty() {
        Some("Type the model's name first")
    } else if needs_key {
        Some("Paste your API key first")
    } else if quick.draft.endpoint().is_err() {
        Some("Correct the server address first")
    } else {
        None
    };
    // Saved as it is and what the chat uses: nothing to save.
    let active = quick.key.trim().is_empty()
        && quick
            .at
            .and_then(|at| settings.endpoints.get(at))
            .is_some_and(|e| *e == quick.draft)
        && settings.run_endpoint().map(|r| &r.name) == Some(&quick.draft.name)
        && settings
            .connection
            .as_deref()
            .is_none_or(|c| c == quick.draft.name);
    let running = matches!(quick.check, Check::Running(_));
    let mut save = false;
    let mut test = false;
    let mut keep = false;
    kit::actions(ui, |ui| {
        let response = kit::primary(ui, "Save", missing.is_none() && !active);
        save = response.clicked();
        if let Some(why) = missing {
            response.on_disabled_hover_text(why);
        }
        let response = ui_kit::button(
            ui,
            "Test connection",
            Button::Enabled(missing.is_none() && !running),
        )
        .on_hover_text(TEST_TIP);
        test = response.clicked();
        if quick.replacing && ui_kit::button(ui, "Keep the stored key", Button::Normal).clicked() {
            keep = true;
        }
    });
    kit::hint(
        ui,
        "Test sends one short message to check the key and model",
        TEST_TIP,
    );
    if keep {
        quick.replacing = false;
        quick.key.clear();
        quick.show = false;
    }
    if test {
        quick.check = start_check(ui.ctx(), checker, &quick.draft, &quick.key);
    }
    if let Check::Running(rx) = &quick.check {
        match rx.try_recv() {
            Ok(Ok(model)) => quick.check = Check::Passed(model),
            Ok(Err(why)) => quick.check = Check::Failed(why),
            Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                quick.check = Check::Failed("The test ended without an answer".into());
            }
        }
    }
    match &quick.check {
        Check::Idle => {}
        Check::Running(_) => {
            kit::note(ui, "Testing…");
        }
        Check::Passed(model) => {
            kit::status(
                ui,
                &format!("Connected: {model} answered."),
                kit::Tone::Note,
            );
        }
        Check::Failed(why) => {
            kit::refusal_wrapped(ui, &format!("Test failed: {why}"));
        }
    }
    if active && (local || quick.stored) {
        kit::note(
            ui,
            &format!("Set: the chat uses {}.", quick.draft.model.trim()),
        );
    }
    if !save {
        return changed;
    }
    let mut edit = EndpointEdit {
        at: quick.at,
        draft: quick.draft.clone(),
        key: std::mem::take(&mut quick.key),
        stored: quick.stored,
    };
    quick.show = false;
    let saved = save_endpoint(state, settings, keyring, &mut edit, true);
    if saved {
        quick.stored = edit.stored;
        quick.replacing = false;
    }
    saved
}

const TEST_TIP: &str = "One request of a few tokens (at most 16 in reply), with no tools and no reasoning, so it costs next to nothing. A model on this computer costs nothing.";

/// The masked API key field with Paste and Show (the typed key only).
fn key_row(ui: &mut egui::Ui, quick: &mut Quick, focus: bool) {
    let id = key_field_id();
    let mut paste = false;
    kit::row(ui, "API key", |ui| {
        ui.horizontal(|ui| {
            let t = sparksmith_ui::theme::Tokens::get();
            let shown = if quick.show { "Hide" } else { "Show" };
            // Room for the two buttons after the field.
            let buttons = 2.0 * (t.control_height * 2.2) + t.space_8;
            let response = ui.add(
                egui::TextEdit::singleline(&mut quick.key)
                    .id(id)
                    .password(!quick.show)
                    .hint_text("Paste your API key")
                    .desired_width((ui.available_width() - buttons).max(t.control_height)),
            );
            let response = kit::named_by_row(ui, response);
            if focus {
                response.request_focus();
            }
            paste = ui_kit::button(ui, "Paste", Button::Small)
                .on_hover_text("Paste the key from the clipboard")
                .clicked();
            if ui_kit::button(ui, shown, Button::Small)
                .on_hover_text(
                    "Show or hide the key while you type it; it is never shown once saved",
                )
                .clicked()
            {
                quick.show = !quick.show;
            }
        });
    });
    if paste {
        // The clipboard's text comes as a paste into the focused field.
        ui.memory_mut(|m| m.request_focus(id));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }
    kit::hint(
        ui,
        "Kept in the system keychain when you save; never in a file",
        "From your provider's account page. It is sent only to this provider, over HTTPS, and is never shown again once saved.",
    );
}

/// Start Test connection off the UI thread.
fn start_check(ctx: &egui::Context, checker: Checker, draft: &EndpointSetting, key: &str) -> Check {
    let (tx, rx) = std::sync::mpsc::channel();
    let setting = draft.clone();
    let key = Secret::new(key);
    let wake = ctx.clone();
    match std::thread::Builder::new()
        .name("settings-ai-check".into())
        .spawn(move || {
            let _ = tx.send(checker(&setting, key));
            wake.request_repaint();
        }) {
        Ok(_) => Check::Running(rx),
        Err(why) => Check::Failed(why.to_string()),
    }
}

fn connections(
    ui: &mut egui::Ui,
    state: &mut State,
    settings: &AssistSettings,
    shipped: &Shipped,
    keyring: Keyring,
) {
    match state.editing.take() {
        Some(Editing::Endpoint(mut edit)) => {
            if !endpoint_form(ui, state, settings, shipped, keyring, &mut edit) {
                state.editing = Some(Editing::Endpoint(edit));
            }
            return;
        }
        Some(Editing::Agent(mut edit)) => {
            if !agent_form(ui, state, settings, &mut edit) {
                state.editing = Some(Editing::Agent(edit));
            }
            return;
        }
        None => {}
    }
    let rows: Vec<(String, String)> = settings
        .endpoints
        .iter()
        .map(|e| (e.name.clone(), e.summary()))
        .chain(
            settings
                .agents
                .iter()
                .map(|a| (a.name.clone(), format!("agent · {}", a.summary()))),
        )
        .collect();
    if rows.is_empty() {
        kit::note(
            ui,
            "No connection yet. Choose one under Connection, or add one here.",
        );
    }
    let max = if state.show_all.contains("connections") {
        rows.len()
    } else {
        kit::LIST_MAX_ROWS
    };
    let mut open = None;
    let shown = kit::list_frame(ui, rows.len(), max, |ui, index| {
        if let Some((name, summary)) = rows.get(index)
            && kit::summary_row(ui, &format!("{name} — {summary}"), Some("Edit"))
        {
            open = Some(index);
        }
    });
    if shown.show_all {
        state.show_all.insert("connections");
    }
    if let Some(index) = open {
        state.editing = Some(match settings.endpoints.get(index) {
            Some(e) => Editing::Endpoint(Box::new(EndpointEdit {
                at: Some(index),
                stored: has_key(keyring, e),
                draft: e.clone(),
                key: String::new(),
            })),
            None => {
                let at = index - settings.endpoints.len();
                match settings.agents.get(at) {
                    Some(a) => Editing::Agent(Box::new(agent_edit(Some(at), a.clone()))),
                    None => return,
                }
            }
        });
        return;
    }
    let mut choice = settings.connection.clone();
    let options: Vec<(Option<String>, String)> = std::iter::once((None, "None".to_owned()))
        .chain(settings.names().map(|n| (Some(n.to_owned()), n.to_owned())))
        .collect();
    if kit::choice_row(ui, "The chat uses", &mut choice, &options) {
        let mut next = settings.clone();
        next.connection = choice;
        state.commit(next);
    }
    let mut add_endpoint = None;
    kit::row(ui, "Add a model", |ui| {
        kit::dropdown_menu(ui, "settings-ai-add-endpoint", "Choose…", |ui| {
            for (index, preset) in shipped.endpoint_presets.iter().enumerate() {
                if ui.selectable_label(false, &preset.title).clicked() {
                    add_endpoint = Some(index);
                }
            }
        })
    });
    let mut add_agent = None;
    kit::row(ui, "Add an agent", |ui| {
        kit::dropdown_menu(ui, "settings-ai-add-agent", "Choose…", |ui| {
            for (index, preset) in shipped.agent_presets.iter().enumerate() {
                if ui.selectable_label(false, &preset.title).clicked() {
                    add_agent = Some(index);
                }
            }
        })
    });
    kit::hint(
        ui,
        "Each starts from a Connection choice you then edit; nothing is detected",
        "An endpoint's key is kept in the system keychain. An agent signs in itself and is started by the chat, not here.",
    );
    if let Some(preset) = add_endpoint.and_then(|i| shipped.endpoint_presets.get(i)) {
        let mut draft = preset.endpoint.clone();
        draft.name = settings.free_name(&draft.name);
        if draft.model.trim().is_empty()
            && draft.provider == Wire::Anthropic
            && let Ok(library) = &state.library
        {
            draft.model = library.default_model.clone();
        }
        state.editing = Some(Editing::Endpoint(Box::new(EndpointEdit {
            at: None,
            stored: has_key(keyring, &draft),
            draft,
            key: String::new(),
        })));
    }
    if let Some(preset) = add_agent.and_then(|i| shipped.agent_presets.get(i)) {
        let mut draft = preset.agent.clone();
        draft.name = settings.free_name(&draft.name);
        state.editing = Some(Editing::Agent(Box::new(agent_edit(None, draft))));
    }
}

fn has_key(keyring: Keyring, endpoint: &EndpointSetting) -> bool {
    endpoint
        .endpoint()
        .is_ok_and(|e| e.check_key_allowed().is_ok() && (keyring.has)(&e))
}

fn agent_edit(at: Option<usize>, draft: AgentSetting) -> AgentEdit {
    AgentEdit {
        at,
        args: draft.args.join("\n"),
        env: draft
            .env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n"),
        draft,
    }
}

/// A whole number from a number row whose range keeps it in bounds.
fn whole(value: f64) -> u64 {
    // The rows' ranges are within u64; a negative or NaN value is 0.
    value.round().max(0.0) as u64
}

fn number_u32(ui: &mut egui::Ui, label: &str, value: &mut u32, unit: &str, range: (u32, u32)) {
    let mut v = f64::from(*value);
    if kit::number_row(
        ui,
        label,
        &mut v,
        unit,
        f64::from(range.0)..=f64::from(range.1),
    ) {
        *value = u32::try_from(whole(v)).unwrap_or(range.1);
    }
}

fn number_u64(ui: &mut egui::Ui, label: &str, value: &mut u64, unit: &str, range: (u64, u64)) {
    // Token counts stay far below 2^53, so f64 holds them exactly.
    let mut v = *value as f64;
    if kit::number_row(ui, label, &mut v, unit, range.0 as f64..=range.1 as f64) {
        *value = whole(v);
    }
}

/// The endpoint form; whether it closed.
fn endpoint_form(
    ui: &mut egui::Ui,
    state: &mut State,
    settings: &AssistSettings,
    shipped: &Shipped,
    keyring: Keyring,
    edit: &mut EndpointEdit,
) -> bool {
    kit::form_heading(
        ui,
        if edit.at.is_some() {
            "Model connection"
        } else {
            "New model connection"
        },
    );
    let d = &mut edit.draft;
    kit::text_row(ui, "Name", &mut d.name);
    let before = d.provider;
    kit::choice_row(
        ui,
        "Provider",
        &mut d.provider,
        &[
            (Wire::Anthropic, "Anthropic".to_owned()),
            (Wire::Openai, "OpenAI or compatible".to_owned()),
        ],
    );
    if d.provider != before {
        // What an OpenAI-compatible model can do is declared, starting
        // from the shipped preset's values; the Messages API declares its
        // own.
        d.capabilities = match d.provider {
            Wire::Anthropic => None,
            Wire::Openai => shipped
                .endpoint_presets
                .iter()
                .find_map(|p| p.endpoint.capabilities.clone()),
        };
    }
    let parsed = d.endpoint();
    let url = kit::checked_text_row(
        ui,
        "Server address",
        &mut d.base_url,
        "https://…",
        parsed.is_err(),
    );
    match &parsed {
        Err(why) => {
            kit::field_error(ui, &url, &why.to_string(), false);
        }
        Ok(e) => {
            kit::hint(
                ui,
                &format!("Requests go to {}", e.location()),
                "Your prompts, the board and datasheets go there.",
            );
        }
    }
    kit::text_row(ui, "Model", &mut d.model).on_hover_text("The model id as the server knows it");
    number_u32(
        ui,
        "Wait for a reply",
        &mut d.timeout_minutes,
        "min",
        (*TIMEOUT_RANGE.start(), *TIMEOUT_RANGE.end()),
    );
    if let Some(c) = &mut d.capabilities {
        number_u64(
            ui,
            "Context size",
            &mut c.context_tokens,
            "tokens",
            (*CONTEXT_RANGE.start(), *CONTEXT_RANGE.end()),
        );
        kit::check_row(ui, "Can use tools", &mut c.tool_calling);
        kit::check_row(ui, "Reads images", &mut c.vision);
        kit::check_row(ui, "Reads PDFs itself", &mut c.native_pdf);
        let mut own = c.output_tokens.is_some();
        if kit::check_row(ui, "Set the output limit", &mut own) {
            c.output_tokens = own.then_some(32_000.min(*OUTPUT_RANGE.end()));
        }
        if let Some(out) = &mut c.output_tokens {
            number_u32(
                ui,
                "Output",
                out,
                "tokens",
                (*OUTPUT_RANGE.start(), *OUTPUT_RANGE.end()),
            );
        }
        kit::hint(
            ui,
            "Declare what this model can do; nothing is detected",
            "Without PDF reading the datasheets' text is sent; pages go as images only to a model that takes them. A model that can't call tools can't be used by the chat.",
        );
    }
    let key_state = if edit.stored {
        "Stored in the system keychain"
    } else {
        "None stored"
    };
    kit::value_row(ui, "API key", key_state);
    kit::row(ui, "New API key", |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut edit.key)
                .password(true)
                .hint_text("Type to store or replace")
                .desired_width(ui.available_width()),
        )
    })
    .on_hover_text(
        "Kept in the system keychain for this base URL when you save, and never shown again. Leave empty for a server that takes none.",
    );
    let mut close = false;
    kit::actions(ui, |ui| {
        if kit::primary(ui, "Save", true).clicked() {
            close = save_endpoint(state, settings, keyring, edit, false);
        }
        if ui_kit::button(ui, "Cancel", Button::Normal).clicked() {
            close = true;
        }
        if edit.stored
            && ui_kit::button(ui, "Remove key", Button::Normal)
                .on_hover_text("Remove this URL's key from the system keychain")
                .clicked()
        {
            match edit.draft.endpoint().map_err(|e| e.to_string()) {
                Ok(e) => match (keyring.delete)(&e) {
                    Ok(_) => edit.stored = false,
                    Err(why) => state.refusal = Some(why),
                },
                Err(why) => state.refusal = Some(why),
            }
        }
        if let Some(at) = edit.at
            && ui_kit::button(ui, "Remove", Button::Normal)
                .on_hover_text(
                    "Remove this connection; its key goes when no other connection uses the address",
                )
                .clicked()
        {
            close = remove_endpoint(state, settings, keyring, at);
        }
    });
    if close {
        edit.key.clear();
    }
    close
}

/// Save the endpoint and then its key, and with `use_it` make it what the
/// chat uses; whether it was saved. The typed key leaves the form whatever
/// happens.
fn save_endpoint(
    state: &mut State,
    settings: &AssistSettings,
    keyring: Keyring,
    edit: &mut EndpointEdit,
    use_it: bool,
) -> bool {
    let key = Secret::new(&std::mem::take(&mut edit.key));
    let mut next = settings.clone();
    match edit.at.and_then(|at| next.endpoints.get_mut(at)) {
        Some(slot) => {
            let before = slot.name.clone();
            *slot = edit.draft.clone();
            if next.connection.as_deref() == Some(before.as_str()) {
                next.connection = Some(edit.draft.name.clone());
            }
        }
        None => next.endpoints.push(edit.draft.clone()),
    }
    if use_it {
        next.connection = Some(edit.draft.name.clone());
    }
    if let Err(why) = next.check() {
        state.refusal = Some(why);
        return false;
    }
    let endpoint = match edit.draft.endpoint() {
        Ok(e) => e,
        Err(why) => {
            state.refusal = Some(why.to_string());
            return false;
        }
    };
    if key.is_some()
        && let Err(why) = endpoint.check_key_allowed()
    {
        state.refusal = Some(why.to_string());
        return false;
    }
    if !state.commit(next) {
        return false;
    }
    if let Some(key) = key {
        if let Err(why) = (keyring.store)(&endpoint, &key) {
            state.refusal = Some(why);
            return false;
        }
        edit.stored = true;
    }
    true
}

fn remove_endpoint(
    state: &mut State,
    settings: &AssistSettings,
    keyring: Keyring,
    at: usize,
) -> bool {
    let mut next = settings.clone();
    if at >= next.endpoints.len() {
        return true;
    }
    let gone = next.endpoints.remove(at);
    if next.connection.as_deref() == Some(gone.name.as_str()) {
        next.connection = None;
    }
    if !state.commit(next.clone()) {
        return false;
    }
    let shared = next.endpoints.iter().any(|e| e.base_url == gone.base_url);
    if !shared
        && let Ok(e) = gone.endpoint()
        && let Err(why) = (keyring.delete)(&e)
    {
        state.refusal = Some(why);
    }
    true
}

/// The agent form; whether it closed.
fn agent_form(
    ui: &mut egui::Ui,
    state: &mut State,
    settings: &AssistSettings,
    edit: &mut AgentEdit,
) -> bool {
    kit::form_heading(
        ui,
        if edit.at.is_some() {
            "Agent"
        } else {
            "New agent"
        },
    );
    kit::text_row(ui, "Name", &mut edit.draft.name);
    kit::text_row(ui, "Command", &mut edit.draft.command)
        .on_hover_text("The program, run without a shell");
    kit::multiline_row(ui, "Arguments", &mut edit.args, false)
        .on_hover_text("One argument per line");
    kit::multiline_row(ui, "Environment", &mut edit.env, false)
        .on_hover_text("NAME=value, one per line. No keys: the agent signs in itself");
    kit::hint(
        ui,
        "Nothing starts here: the chat starts the agent when you use it",
        "Sign in with the agent itself; its key never comes to this app.",
    );
    let mut close = false;
    kit::actions(ui, |ui| {
        if kit::primary(ui, "Save", true).clicked() {
            close = save_agent(state, settings, edit);
        }
        if ui_kit::button(ui, "Cancel", Button::Normal).clicked() {
            close = true;
        }
        if let Some(at) = edit.at
            && ui_kit::button(ui, "Remove", Button::Normal).clicked()
        {
            let mut next = settings.clone();
            if at < next.agents.len() {
                let gone = next.agents.remove(at);
                if next.connection.as_deref() == Some(gone.name.as_str()) {
                    next.connection = None;
                }
                close = state.commit(next);
            }
        }
    });
    close
}

fn save_agent(state: &mut State, settings: &AssistSettings, edit: &mut AgentEdit) -> bool {
    let mut draft = edit.draft.clone();
    draft.args = edit
        .args
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    draft.env.clear();
    for line in edit.env.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some((name, value)) = line.split_once('=') else {
            state.refusal = Some(format!("Environment: {line:?} is not NAME=value"));
            return false;
        };
        draft.env.insert(name.trim().to_owned(), value.to_owned());
    }
    let mut next = settings.clone();
    match edit.at.and_then(|at| next.agents.get_mut(at)) {
        Some(slot) => {
            if next.connection.as_deref() == Some(slot.name.as_str()) {
                next.connection = Some(draft.name.clone());
            }
            *slot = draft;
        }
        None => next.agents.push(draft),
    }
    state.commit(next)
}

fn permissions(ui: &mut egui::Ui, state: &mut State, settings: &AssistSettings, shipped: &Shipped) {
    let mut p = settings.permissions_or(shipped);
    let before = p;
    let choices = rules();
    kit::choice_row(ui, "Edit the design", &mut p.edits, &choices);
    kit::choice_row(ui, "Read or write files", &mut p.files, &choices);
    kit::choice_row(ui, "Publish to a library", &mut p.publish_library, &choices);
    kit::choice_row(ui, "Move my view", &mut p.view, &choices);
    kit::hint(
        ui,
        "When asked: Allow once, Allow for this session, Always allow or Deny",
        "Always allow sets the choice here. Publishing a part into the open project never asks. Keys, endpoints and these permissions are yours alone: no assistant can read or change them.",
    );
    if p != before {
        let mut next = settings.clone();
        next.permissions = (p != shipped.permissions).then_some(p);
        state.commit(next);
    }
}

/// The person's own limits on model runs: off by default, so the chat can
/// build a whole board; a workflow brings its own budget, held within
/// these when they are on.
fn limits(ui: &mut egui::Ui, state: &mut State, settings: &AssistSettings, shipped: &Shipped) {
    use workflow::{MAX_MINUTES, MAX_OUTPUT_TOKENS, MAX_REQUESTS, MAX_TOTAL_TOKENS};
    let mut on = settings.limits.is_some();
    if kit::check_row(ui, "Limit model runs", &mut on) {
        let mut next = settings.clone();
        next.limits = on.then_some(shipped.limits_start);
        state.commit(next);
        return;
    }
    let Some(mut l) = settings.limits else {
        kit::hint(
            ui,
            "No limit: the chat runs until it is done or you stop it",
            "A workflow brings its own budget (the datasheet workflow: its requests and minutes). Turn this on to hold every run, the chat's and the workflows', within your own limits.",
        );
        return;
    };
    let before = l;
    number_u32(ui, "Model requests", &mut l.requests, "", (1, MAX_REQUESTS));
    number_u64(
        ui,
        "Output tokens",
        &mut l.output_tokens,
        "",
        (1, MAX_OUTPUT_TOKENS),
    );
    number_u64(
        ui,
        "All tokens",
        &mut l.total_tokens,
        "",
        (1, MAX_TOTAL_TOKENS),
    );
    number_u32(ui, "Time", &mut l.minutes, "min", (1, MAX_MINUTES));
    kit::hint(
        ui,
        "A model run stops at the first of these it reaches",
        "All tokens counts prompt and output. A workflow's budget is held within these, never above. An agent's tokens are its own; its time still holds.",
    );
    if l != before {
        let mut next = settings.clone();
        next.limits = Some(l);
        state.commit(next);
    }
}

fn profiles(ui: &mut egui::Ui, state: &mut State) {
    if let Err(why) = &state.library {
        kit::refusal_wrapped(ui, why);
        return;
    }
    let folder = state.profile_folder();
    if let Some((name, mut text)) = state.profile.take() {
        kit::form_heading(ui, &format!("Profile {name}"));
        kit::multiline_row(ui, "Profile", &mut text, false);
        let mut close = false;
        kit::actions(ui, |ui| {
            if kit::primary(ui, "Save", folder.is_some()).clicked()
                && let Some(folder) = &folder
            {
                match stored::save_profile(folder, &name, &text) {
                    Ok(_) => {
                        state.refusal = None;
                        close = true;
                    }
                    Err(why) => state.refusal = Some(why.to_string()),
                }
            }
            if ui_kit::button(ui, "Cancel", Button::Normal).clicked() {
                close = true;
            }
            if ui_kit::button(ui, "Remove", Button::Normal)
                .on_hover_text("Delete your copy; a built-in profile of this name is used again")
                .clicked()
                && let Some(folder) = &folder
            {
                match stored::remove_profile(folder, &name) {
                    Ok(()) => close = true,
                    Err(why) => state.refusal = Some(why.to_string()),
                }
            }
        });
        kit::hint(
            ui,
            "Checked when you save; a profile that fails is never used",
            "A run uses the first of your profiles, then the built-in ones, that lists its model; else default.",
        );
        if close {
            state.reload_profiles();
        } else {
            state.profile = Some((name, text));
        }
        return;
    }
    let rows: Vec<(String, String, Option<&'static str>)> = state
        .profiles
        .iter()
        .map(|p| {
            let user = state
                .profiles
                .iter()
                .any(|q| q.name == p.name && q.source == ProfileSource::User);
            let (text, action) = match (&p.source, &p.profile) {
                (ProfileSource::BuiltIn, Ok(profile)) => (
                    format!("{} · built in{}", p.name, models(profile)),
                    (!user && folder.is_some()).then_some("Copy to edit"),
                ),
                (ProfileSource::User, Ok(profile)) => (
                    format!("{} · yours{}", p.name, models(profile)),
                    Some("Edit"),
                ),
                (_, Err(why)) => (format!("{} · not used: {why}", p.name), Some("Edit")),
            };
            (p.name.clone(), text, action)
        })
        .collect();
    let max = if state.show_all.contains("profiles") {
        rows.len()
    } else {
        kit::LIST_MAX_ROWS
    };
    let mut clicked = None;
    let shown = kit::list_frame(ui, rows.len(), max, |ui, index| {
        if let Some((name, text, action)) = rows.get(index)
            && kit::summary_row(ui, text, *action)
        {
            clicked = Some((name.clone(), *action));
        }
    });
    if shown.show_all {
        state.show_all.insert("profiles");
    }
    let (Some((name, action)), Some(folder)) = (clicked, folder) else {
        return;
    };
    let opened = match (action, &state.library) {
        (Some("Copy to edit"), Ok(library)) => library
            .copy_to_user(&name, &folder)
            .map_err(|e| e.to_string())
            .and_then(|path| std::fs::read_to_string(path).map_err(|e| e.to_string())),
        _ => {
            std::fs::read_to_string(folder.join(format!("{name}.json"))).map_err(|e| e.to_string())
        }
    };
    match opened {
        Ok(text) => {
            state.profile = Some((name, text));
            state.reload_profiles();
        }
        Err(why) => state.refusal = Some(why),
    }
}

fn models(profile: &sparksmith_assist::profile::RunProfile) -> String {
    if profile.models.is_empty() {
        String::new()
    } else {
        format!(" · for {}", profile.models.join(", "))
    }
}

fn workflows(ui: &mut egui::Ui, state: &mut State) {
    let count = state.workflows.len();
    let show_all = state.show_all.contains("workflows");
    let mut more = false;
    kit::section(
        ui,
        "settings-ai-workflows",
        "Workflows",
        false,
        Some(count),
        |ui| {
            if count == 0 {
                kit::note(ui, "No workflow is installed.");
            }
            let max = if show_all { count } else { kit::LIST_MAX_ROWS };
            more = kit::list_frame(ui, count, max, |ui, index| {
                if let Some(listed) = state.workflows.get(index) {
                    let (title, status) = workflow_row(listed);
                    kit::value_row(ui, &title, &status);
                }
            })
            .show_all;
            kit::hint(
                ui,
                "Read only: workflows are files in the library or your config folder",
                "Yours are in <config>/assist/workflows/<id>/ and take the place of a shipped one of the same id.",
            );
        },
    );
    if more {
        state.show_all.insert("workflows");
    }
}

/// A workflow's title with its version, and where it comes from and
/// whether it can run here, or why not.
fn workflow_row(listed: &Listed) -> (String, String) {
    let source = match listed.source {
        Source::Shipped => "Shipped",
        Source::User => "Yours",
    };
    let title = match &listed.loaded {
        Some(l) => format!("{} v{}", l.workflow.title, l.workflow.version),
        None => listed.id.clone(),
    };
    let status = if listed.available() {
        format!("{source} · available")
    } else {
        format!("{source} · unavailable: {}", listed.why_not.join("; "))
    };
    (title, status)
}

#[cfg(test)]
#[path = "settings_ai_tests.rs"]
mod tests;

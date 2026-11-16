//! Settings › AI (`docs/assistant-panel.md` §6.4): the connections (model
//! endpoints and ACP agents), the permissions per kind of action, a model
//! run's limits, the run profiles and the workflows, on one page of the
//! Settings window. There are no per-turn caps on edits or calls.
//!
//! What is set here is the person's `<config>/assist/settings.json`
//! (`sparksmith_assist::settings`), written at once and checked first; the
//! values it starts with ship in the library. A key is typed into a masked
//! field, goes to the system keychain for the endpoint's base URL when the
//! endpoint is saved, and is never shown again, written to a file or kept
//! in the form; a key for plain HTTP to another computer is refused before
//! it is stored. An agent is configuration only: nothing here starts it.
//! Profiles are files: a built-in one is copied to edit, and the text is
//! checked when it is saved. Workflows are listed read-only, with why one
//! can't run.
use crate::kit;
use sparksmith_assist::AssistLibrary;
use sparksmith_assist::model::{Endpoint, Secret, keys};
use sparksmith_assist::profile as run_profile;
use sparksmith_assist::settings::{
    self as stored, AgentSetting, AssistSettings, CONTEXT_RANGE, EndpointSetting, Limits,
    OUTPUT_RANGE, ProfileEntry, ProfileSource, Rule, Shipped, TIMEOUT_RANGE, Wire,
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
    state: Option<State>,
}

impl Default for AiPage {
    fn default() -> Self {
        Self {
            config: None,
            #[cfg(not(test))]
            keyring: Keyring::SYSTEM,
            #[cfg(test)]
            keyring: test_keyring::MEMORY,
            state: None,
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

    fn state(&mut self) -> &mut State {
        let config = self.config.clone();
        self.state.get_or_insert_with(|| {
            State::read(config.or_else(|| {
                sparksmith_io::settings::UserDirs::from_env()
                    .ok()
                    .map(|d| d.config)
            }))
        })
    }

    /// The page, in the Settings window's AI section.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let keyring = self.keyring;
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
        let count = settings.endpoints.len() + settings.agents.len();
        kit::section(
            ui,
            "settings-ai-connections",
            "Connections",
            true,
            Some(count),
            |ui| connections(ui, state, &settings, &shipped, keyring),
        );
        kit::section(
            ui,
            "settings-ai-permissions",
            "Permissions",
            true,
            None,
            |ui| permissions(ui, state, &settings, &shipped),
        );
        kit::section(
            ui,
            "settings-ai-run-limits",
            "Run limits",
            true,
            None,
            |ui| limits(ui, state, &settings, &shipped),
        );
        let profile_count = state.profiles.len();
        kit::section(
            ui,
            "settings-ai-profiles",
            "Profiles",
            true,
            Some(profile_count),
            |ui| profiles(ui, state),
        );
        workflows(ui, state);
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
        kit::note(ui, "No connection yet. Add a model endpoint or an agent.");
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
    if kit::choice_row(ui, "Assistant uses", &mut choice, &options) {
        let mut next = settings.clone();
        next.connection = choice;
        state.commit(next);
    }
    let mut add_endpoint = None;
    kit::row(ui, "Add endpoint", |ui| {
        kit::dropdown_menu(ui, "settings-ai-add-endpoint", "Choose a preset…", |ui| {
            for (index, preset) in shipped.endpoint_presets.iter().enumerate() {
                if ui.selectable_label(false, &preset.title).clicked() {
                    add_endpoint = Some(index);
                }
            }
        })
    });
    let mut add_agent = None;
    kit::row(ui, "Add agent", |ui| {
        kit::dropdown_menu(ui, "settings-ai-add-agent", "Choose a preset…", |ui| {
            for (index, preset) in shipped.agent_presets.iter().enumerate() {
                if ui.selectable_label(false, &preset.title).clicked() {
                    add_agent = Some(index);
                }
            }
        })
    });
    kit::hint(
        ui,
        "Presets are templates you edit; nothing is detected",
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
            "Endpoint"
        } else {
            "New endpoint"
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
            (Wire::Anthropic, "Anthropic Messages".to_owned()),
            (Wire::Openai, "OpenAI-compatible".to_owned()),
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
        "Base URL",
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
        "Timeout",
        &mut d.timeout_minutes,
        "min",
        (*TIMEOUT_RANGE.start(), *TIMEOUT_RANGE.end()),
    );
    if let Some(c) = &mut d.capabilities {
        number_u64(
            ui,
            "Context",
            &mut c.context_tokens,
            "tokens",
            (*CONTEXT_RANGE.start(), *CONTEXT_RANGE.end()),
        );
        kit::check_row(ui, "Calls tools", &mut c.tool_calling);
        kit::check_row(ui, "Takes images (vision)", &mut c.vision);
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
    kit::value_row(ui, "Key", key_state);
    kit::row(ui, "New key", |ui| {
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
            close = save_endpoint(state, settings, keyring, edit);
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
                    "Remove this endpoint; its key goes when no other endpoint uses the URL",
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

/// Save the endpoint and then its key; whether it was saved. The typed
/// key leaves the form whatever happens.
fn save_endpoint(
    state: &mut State,
    settings: &AssistSettings,
    keyring: Keyring,
    edit: &mut EndpointEdit,
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

fn limits(ui: &mut egui::Ui, state: &mut State, settings: &AssistSettings, shipped: &Shipped) {
    use workflow::{MAX_MINUTES, MAX_OUTPUT_TOKENS, MAX_REQUESTS, MAX_TOTAL_TOKENS};
    let mut l: Limits = settings.limits_or(shipped);
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
        "All tokens counts prompt and output. A workflow's budget sets its run's limits within these, never above. An agent's tokens are its own; its time still holds.",
    );
    if settings.limits.is_some() {
        kit::actions(ui, |ui| {
            if ui_kit::button(ui, "Reset to the defaults", Button::Normal).clicked() {
                let mut next = settings.clone();
                next.limits = None;
                state.commit(next);
            }
        });
    }
    if l != before {
        let mut next = settings.clone();
        next.limits = (l != shipped.limits).then_some(l);
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
        true,
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

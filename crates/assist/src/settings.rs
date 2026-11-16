//! Settings › AI as stored: the connections (model endpoints and ACP
//! agents), the permissions per kind of action, a model run's limits and
//! which connection the assistant uses (`docs/assistant-panel.md` §6).
//! There are no per-turn caps on edits or calls (founder, 16 November).
//!
//! A person's settings are `<config>/assist/settings.json`, apart from the
//! app's `settings.json`, so no entry that reads or writes general settings
//! reaches them: keys, endpoints, permissions and grants are Never-class.
//! What Settings › AI starts with (permissions, limits and the endpoint and
//! agent presets) ships in the library's `assist/settings.json`; the code
//! assumes none of it. A key is never in either file: it lives in the
//! system keychain under the endpoint's base URL ([`crate::model::keys`]).
//!
//! An agent is configuration only here: a command, its arguments and its
//! environment, run without a shell when the chat starts it. Its sign-in
//! is its own, so a variable that would carry a key is refused.
//!
//! Run profiles stay files in `<config>/assist-profiles/`
//! ([`crate::profile`]); [`profiles`] lists them beside the built-in ones
//! and [`save_profile`] checks one before it replaces the file.
use crate::model::{Budget, Endpoint, ModelProfile, OutputLimits, Provider};
use crate::profile::{AssistLibrary, RunProfile};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sparksmith_io::shipped::{Shelf, ShippedLibrary};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The one format of both settings files until the first release.
pub const SETTINGS_FORMAT: u32 = 1;
/// The file's name in the user's `<config>/assist/` and the library's
/// `assist/`.
pub const SETTINGS_FILE: &str = "settings.json";
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Endpoints, agents and presets of each kind kept.
pub const MAX_CONNECTIONS: usize = 32;
/// Characters of a connection's name.
pub const MAX_NAME: usize = 64;
/// Bytes of a model id.
pub const MAX_MODEL: usize = 200;
/// Bytes of a command, an argument or a variable's value.
pub const MAX_TEXT: usize = 4096;
/// Arguments of one agent.
pub const MAX_ARGS: usize = 64;
/// Environment variables of one agent.
pub const MAX_ENV: usize = 64;
/// The context a model may declare, in tokens.
pub const CONTEXT_RANGE: std::ops::RangeInclusive<u64> = 2_048..=10_000_000;
/// Output tokens one request may be declared to take.
pub const OUTPUT_RANGE: std::ops::RangeInclusive<u32> = 256..=1_000_000;
/// Minutes one request may wait.
pub const TIMEOUT_RANGE: std::ops::RangeInclusive<u32> = 1..=24 * 60;
/// Parts of a variable name that mean it carries a key.
const SECRET_WORDS: &[&str] = &["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];

/// Which wire format an endpoint speaks, as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wire {
    /// Anthropic Messages.
    Anthropic,
    /// OpenAI-compatible chat completions.
    Openai,
}

impl Wire {
    pub fn provider(self) -> Provider {
        match self {
            Self::Anthropic => Provider::Anthropic,
            Self::Openai => Provider::OpenAiCompatible,
        }
    }
}

/// What a model on an OpenAI-compatible endpoint can do, as declared.
/// The Messages API declares its own ([`ModelProfile::anthropic`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declared {
    pub tool_calling: bool,
    pub vision: bool,
    pub native_pdf: bool,
    /// What the server gives this model, not what the weights could take.
    pub context_tokens: u64,
    /// Output tokens a request; none: from the context
    /// ([`OutputLimits::for_context`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
}

/// One model endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointSetting {
    pub name: String,
    pub provider: Wire,
    /// The API's base URL; the keychain keeps its key under it.
    pub base_url: String,
    pub model: String,
    /// Longest wait for one response.
    pub timeout_minutes: u32,
    /// What the model can do, declared; an OpenAI-compatible endpoint
    /// needs it, and the Messages API declares its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Declared>,
}

impl EndpointSetting {
    /// The checked base URL.
    pub fn endpoint(&self) -> Result<Endpoint> {
        Endpoint::parse(&self.base_url)
    }

    /// The model as a run takes it: the Messages API's own capabilities,
    /// or what was declared for an OpenAI-compatible one.
    pub fn model_profile(&self) -> Result<ModelProfile> {
        let model = self.model.trim();
        let mut profile = match self.provider {
            Wire::Anthropic => ModelProfile::anthropic(model),
            Wire::Openai => {
                let Some(c) = &self.capabilities else {
                    return Err(Error::Config(format!(
                        "endpoint {}: declare what the model can do",
                        self.name
                    )));
                };
                let mut p = ModelProfile::openai_compatible(model, c.context_tokens);
                p.capabilities.tool_calling = c.tool_calling;
                p.capabilities.vision = c.vision;
                p.capabilities.native_pdf = c.native_pdf;
                if let Some(out) = c.output_tokens {
                    let from_context = OutputLimits::for_context(c.context_tokens);
                    p.output = OutputLimits {
                        per_request: out,
                        after_cut: from_context.after_cut.max(out),
                    };
                }
                p
            }
        };
        profile.request_timeout = Duration::from_secs(u64::from(self.timeout_minutes) * 60);
        Ok(profile)
    }

    /// One line: the model and where its requests go.
    pub fn summary(&self) -> String {
        let place = self.endpoint().map_or("an invalid URL", |e| e.location());
        format!("{} · {}", self.model.trim(), place)
    }

    fn check(&self) -> std::result::Result<(), String> {
        check_name(&self.name)?;
        self.endpoint().map_err(|e| e.to_string())?;
        let model = self.model.trim();
        if model.is_empty() || model.len() > MAX_MODEL || model.chars().any(char::is_control) {
            return Err(format!(
                "endpoint {}: give the model id, at most {MAX_MODEL} bytes",
                self.name
            ));
        }
        if !TIMEOUT_RANGE.contains(&self.timeout_minutes) {
            return Err(format!(
                "endpoint {}: the timeout must be {} to {} minutes",
                self.name,
                TIMEOUT_RANGE.start(),
                TIMEOUT_RANGE.end()
            ));
        }
        let Some(c) = &self.capabilities else {
            return match self.provider {
                Wire::Anthropic => Ok(()),
                Wire::Openai => Err(format!(
                    "endpoint {}: declare what the model can do",
                    self.name
                )),
            };
        };
        if !CONTEXT_RANGE.contains(&c.context_tokens) {
            return Err(format!(
                "endpoint {}: the context must be {} to {} tokens",
                self.name,
                CONTEXT_RANGE.start(),
                CONTEXT_RANGE.end()
            ));
        }
        if c.output_tokens.is_some_and(|o| !OUTPUT_RANGE.contains(&o)) {
            return Err(format!(
                "endpoint {}: the output limit must be {} to {} tokens",
                self.name,
                OUTPUT_RANGE.start(),
                OUTPUT_RANGE.end()
            ));
        }
        Ok(())
    }
}

/// One ACP agent: what the chat runs, without a shell, when it starts it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSetting {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl AgentSetting {
    /// One line: the command as it runs.
    pub fn summary(&self) -> String {
        std::iter::once(self.command.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn check(&self) -> std::result::Result<(), String> {
        check_name(&self.name)?;
        let plain = |text: &str| text.len() <= MAX_TEXT && !text.chars().any(char::is_control);
        if self.command.trim().is_empty() || !plain(&self.command) {
            return Err(format!(
                "agent {}: give the command, at most {MAX_TEXT} bytes",
                self.name
            ));
        }
        if self.args.len() > MAX_ARGS || !self.args.iter().all(|a| plain(a)) {
            return Err(format!(
                "agent {}: at most {MAX_ARGS} arguments of at most {MAX_TEXT} bytes",
                self.name
            ));
        }
        if self.env.len() > MAX_ENV {
            return Err(format!(
                "agent {}: at most {MAX_ENV} environment variables",
                self.name
            ));
        }
        for (name, value) in &self.env {
            if !env_name(name) || !plain(value) {
                return Err(format!(
                    "agent {}: {name:?} is not a variable name (A–Z, 0–9, _) with a value of at most {MAX_TEXT} bytes",
                    self.name
                ));
            }
            let upper = name.to_ascii_uppercase();
            if SECRET_WORDS.iter().any(|w| upper.contains(w)) {
                return Err(format!(
                    "agent {}: {name} would keep a key in a file; sign in with the agent itself",
                    self.name
                ));
            }
        }
        Ok(())
    }
}

fn env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 128
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn check_name(name: &str) -> std::result::Result<(), String> {
    if name.trim().is_empty()
        || name.trim() != name
        || name.chars().count() > MAX_NAME
        || name.chars().any(char::is_control)
    {
        return Err(format!(
            "name {name:?}: 1–{MAX_NAME} characters, without spaces at either end"
        ));
    }
    Ok(())
}

/// What happens when an assistant wants to do one kind of action. The
/// ask itself offers Allow once, Allow for this session, Always allow
/// (which sets [`Rule::Always`] here) and Deny.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// Ask each time.
    Ask,
    /// Ask the first time in a session; allowed after that.
    Session,
    /// Allowed without asking.
    Always,
    /// Refused without asking.
    Deny,
}

/// The rule for each kind of action an assistant may take. Publishing a
/// part into the open project needs no asking; publishing to a library
/// follows [`Self::publish_library`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Changing the open design.
    pub edits: Rule,
    /// Reading or writing files outside the project.
    pub files: Rule,
    /// Publishing to a library.
    pub publish_library: Rule,
    /// Moving the person's view (selection, camera, opening an editor).
    pub view: Rule,
}

/// Limits the person may set on every model run, hosted or local. Unset
/// by default, so the chat can build a whole board; a workflow brings its
/// own budget ([`crate::workflow::Budget`]), held within these when set
/// ([`run_budget`]). There are no per-turn caps on edits or calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub requests: u32,
    pub output_tokens: u64,
    /// Prompt plus output tokens.
    pub total_tokens: u64,
    pub minutes: u32,
}

impl Limits {
    pub fn check(&self) -> std::result::Result<(), String> {
        use crate::workflow::{MAX_MINUTES, MAX_OUTPUT_TOKENS, MAX_REQUESTS, MAX_TOTAL_TOKENS};
        let bad = |what: &str, top: String| Err(format!("limits: {what} must be 1 to {top}"));
        if !(1..=MAX_REQUESTS).contains(&self.requests) {
            return bad("model requests", MAX_REQUESTS.to_string());
        }
        if !(1..=MAX_OUTPUT_TOKENS).contains(&self.output_tokens) {
            return bad("output tokens", MAX_OUTPUT_TOKENS.to_string());
        }
        if !(1..=MAX_TOTAL_TOKENS).contains(&self.total_tokens) {
            return bad("prompt plus output tokens", MAX_TOTAL_TOKENS.to_string());
        }
        if !(1..=MAX_MINUTES).contains(&self.minutes) {
            return bad("minutes", MAX_MINUTES.to_string());
        }
        Ok(())
    }

    /// `budget` held within these limits.
    pub fn cap(&self, budget: Budget) -> Budget {
        Budget {
            max_requests: budget.max_requests.min(self.requests),
            max_output_tokens: budget.max_output_tokens.min(self.output_tokens),
            max_total_tokens: budget.max_total_tokens.min(self.total_tokens),
            max_duration: budget
                .max_duration
                .min(Duration::from_secs(u64::from(self.minutes) * 60)),
        }
    }
}

/// A model run's budget: a workflow's own, held within the person's
/// limits when they are set; the person's limits alone for a run with no
/// budget of its own (the chat); none when neither is set, so the chat
/// runs until it is done or stopped. A workflow budget without a total
/// token figure takes the format's bound.
pub fn run_budget(
    limits: Option<&Limits>,
    workflow: Option<&crate::workflow::Budget>,
) -> Option<Budget> {
    let own = workflow.map(|w| Budget {
        max_requests: w.requests,
        max_output_tokens: w.output_tokens,
        max_total_tokens: w.total_tokens.unwrap_or(crate::workflow::MAX_TOTAL_TOKENS),
        max_duration: Duration::from_secs(u64::from(w.minutes) * 60),
    });
    match (own, limits) {
        (Some(own), Some(limits)) => Some(limits.cap(own)),
        (Some(own), None) => Some(own),
        (None, Some(limits)) => Some(Budget {
            max_requests: limits.requests,
            max_output_tokens: limits.output_tokens,
            max_total_tokens: limits.total_tokens,
            max_duration: Duration::from_secs(u64::from(limits.minutes) * 60),
        }),
        (None, None) => None,
    }
}

/// The budget the shipped workflow `id` declares, read from its
/// `workflow.json` and checked by the format's rules.
pub fn workflow_budget(shipped: &ShippedLibrary, id: &str) -> Result<crate::workflow::Budget> {
    let name = format!(
        "{}/{id}/{}",
        crate::workflow::load::FOLDER,
        crate::workflow::WORKFLOW_FILE
    );
    let origin = shipped.shelf(Shelf::Assist).join(&name);
    let bytes = shipped
        .read(Shelf::Assist, &name, crate::workflow::load::MAX_FILE_BYTES)
        .map_err(|e| Error::Config(format!("{}: {e}", origin.display())))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Config(format!("{}: not UTF-8 text", origin.display())))?;
    let workflow = crate::workflow::Workflow::parse(&text, &origin.display().to_string())
        .map_err(Error::Config)?;
    Ok(workflow.budget)
}

/// A person's Settings › AI, as stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistSettings {
    pub format: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<EndpointSetting>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentSetting>,
    /// The connection the assistant uses, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// None: the shipped ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    /// None: no limits of the person's own ([`run_budget`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
}

impl Default for AssistSettings {
    fn default() -> Self {
        Self {
            format: SETTINGS_FORMAT,
            endpoints: Vec::new(),
            agents: Vec::new(),
            connection: None,
            permissions: None,
            limits: None,
        }
    }
}

impl AssistSettings {
    /// Every rule the file keeps; the first broken one, named.
    pub fn check(&self) -> std::result::Result<(), String> {
        if self.format != SETTINGS_FORMAT {
            return Err(format!(
                "format {} is not supported (this version reads format {SETTINGS_FORMAT})",
                self.format
            ));
        }
        if self.endpoints.len() > MAX_CONNECTIONS || self.agents.len() > MAX_CONNECTIONS {
            return Err(format!(
                "at most {MAX_CONNECTIONS} endpoints and {MAX_CONNECTIONS} agents"
            ));
        }
        let mut names = BTreeSet::new();
        for name in self.names() {
            if !names.insert(name) {
                return Err(format!("two connections are named {name}"));
            }
        }
        for e in &self.endpoints {
            e.check()?;
        }
        for a in &self.agents {
            a.check()?;
        }
        if let Some(name) = &self.connection
            && !names.contains(name.as_str())
        {
            return Err(format!("the assistant uses {name}, which is not set up"));
        }
        if let Some(limits) = &self.limits {
            limits.check()?;
        }
        Ok(())
    }

    /// Every connection's name, endpoints first.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.endpoints
            .iter()
            .map(|e| e.name.as_str())
            .chain(self.agents.iter().map(|a| a.name.as_str()))
    }

    /// The endpoint a model run uses: the assistant's connection when it
    /// is an endpoint, else the first endpoint.
    pub fn run_endpoint(&self) -> Option<&EndpointSetting> {
        self.connection
            .as_deref()
            .and_then(|name| self.endpoints.iter().find(|e| e.name == name))
            .or_else(|| self.endpoints.first())
    }

    pub fn permissions_or(&self, shipped: &Shipped) -> Permissions {
        self.permissions.unwrap_or(shipped.permissions)
    }

    /// A name for a new connection from `base` not taken yet.
    pub fn free_name(&self, base: &str) -> String {
        let taken: BTreeSet<&str> = self.names().collect();
        if !taken.contains(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|name| !taken.contains(name.as_str()))
            .unwrap_or_else(|| base.to_owned())
    }
}

/// The user's settings file under their config folder.
pub fn user_file(config: &Path) -> PathBuf {
    config.join("assist").join(SETTINGS_FILE)
}

/// Read the settings at `path`; a file that is not there is the empty
/// settings. One that can't be read or checked is refused, naming it, so
/// nothing saves over it by mistake.
pub fn read(path: &Path) -> Result<AssistSettings> {
    let origin = path.display();
    let text = match read_bounded(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AssistSettings::default()),
        Err(e) => return Err(Error::Config(format!("{origin}: {e}"))),
    };
    let settings: AssistSettings =
        serde_json::from_str(&text).map_err(|e| Error::Config(format!("{origin}: {e}")))?;
    settings
        .check()
        .map_err(|why| Error::Config(format!("{origin}: {why}")))?;
    Ok(settings)
}

/// Check `settings` and replace the file at `path` atomically.
pub fn write(path: &Path, settings: &AssistSettings) -> Result<()> {
    settings.check().map_err(Error::Config)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings)? + "\n";
    sparksmith_io::persistence::atomic_write(path, text.as_bytes())
        .map_err(|e| Error::Config(format!("{}: {e}", path.display())))
}

fn read_bounded(path: &Path) -> std::io::Result<String> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(std::io::Error::other(format!(
            "larger than {MAX_FILE_BYTES} bytes"
        )));
    }
    Ok(text)
}

/// A preset for a new endpoint: a template the person edits.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointPreset {
    pub title: String,
    /// An empty model is the shipped default model.
    pub endpoint: EndpointSetting,
}

/// A preset for a new agent: a template the person edits; nothing is
/// probed or detected.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPreset {
    pub title: String,
    pub agent: AgentSetting,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShippedFile {
    format: u32,
    #[serde(default, rename = "about")]
    _about: String,
    permissions: Permissions,
    limits_start: Limits,
    endpoint_presets: Vec<EndpointPreset>,
    agent_presets: Vec<AgentPreset>,
    #[serde(default, rename = "sources")]
    _sources: serde_json::Map<String, serde_json::Value>,
}

/// What Settings › AI starts with, from the library's
/// `assist/settings.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Shipped {
    pub permissions: Permissions,
    /// The values the limits start from when the person turns them on.
    pub limits_start: Limits,
    pub endpoint_presets: Vec<EndpointPreset>,
    pub agent_presets: Vec<AgentPreset>,
}

impl Shipped {
    pub fn locate() -> Result<Self> {
        let shipped = ShippedLibrary::locate().map_err(|e| Error::Config(e.to_string()))?;
        Self::load(&shipped)
    }

    /// Read and check the shipped file; a missing or broken one is
    /// refused with its path.
    pub fn load(shipped: &ShippedLibrary) -> Result<Self> {
        let path = shipped.shelf(Shelf::Assist).join(SETTINGS_FILE);
        let bad = |why: &dyn std::fmt::Display| Error::Config(format!("{}: {why}", path.display()));
        let bytes = shipped
            .read(Shelf::Assist, SETTINGS_FILE, MAX_FILE_BYTES)
            .map_err(|e| bad(&e))?;
        let file: ShippedFile = serde_json::from_slice(&bytes).map_err(|e| bad(&e))?;
        if file.format != SETTINGS_FORMAT {
            return Err(bad(&format!(
                "format {} is not supported (this version reads format {SETTINGS_FORMAT})",
                file.format
            )));
        }
        file.limits_start.check().map_err(|e| bad(&e))?;
        if file.endpoint_presets.len() > MAX_CONNECTIONS
            || file.agent_presets.len() > MAX_CONNECTIONS
        {
            return Err(bad(&format!(
                "at most {MAX_CONNECTIONS} presets of each kind"
            )));
        }
        for preset in &file.endpoint_presets {
            let mut e = preset.endpoint.clone();
            if e.model.trim().is_empty() {
                // Checked with a stand-in for the default model it takes.
                e.model = "default".into();
            }
            e.check()
                .map_err(|why| bad(&format!("preset {}: {why}", preset.title)))?;
        }
        for preset in &file.agent_presets {
            preset
                .agent
                .check()
                .map_err(|why| bad(&format!("preset {}: {why}", preset.title)))?;
        }
        Ok(Self {
            permissions: file.permissions,
            limits_start: file.limits_start,
            endpoint_presets: file.endpoint_presets,
            agent_presets: file.agent_presets,
        })
    }
}

/// Where a run profile comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileSource {
    BuiltIn,
    User,
}

/// One run profile as Settings › AI lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileEntry {
    pub name: String,
    pub source: ProfileSource,
    /// The user's file, for a user profile.
    pub path: Option<PathBuf>,
    /// The profile, or why it is never used.
    pub profile: std::result::Result<RunProfile, String>,
}

/// The user's profiles in `folder` (each read and checked, a broken one
/// with its reason), then the built-in ones; a user file of a built-in's
/// name takes its place, as a run selects it.
pub fn profiles(shipped: &AssistLibrary, folder: Option<&Path>) -> Vec<ProfileEntry> {
    let mut out = Vec::new();
    if let Some(folder) = folder
        && let Ok(entries) = std::fs::read_dir(folder)
    {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        paths.truncate(MAX_CONNECTIONS * 2);
        for path in paths {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(ProfileEntry {
                name,
                source: ProfileSource::User,
                profile: crate::profile::read_file(&path).map_err(|e| e.to_string()),
                path: Some(path),
            });
        }
    }
    for name in shipped.built_in_names() {
        if out.iter().any(|p| p.name == name) {
            continue;
        }
        if let Some(profile) = shipped.built_in(name) {
            out.push(ProfileEntry {
                name: name.to_owned(),
                source: ProfileSource::BuiltIn,
                path: None,
                profile: Ok(profile.clone()),
            });
        }
    }
    out
}

/// Check `text` as the user's profile `name` and replace its file in
/// `folder`. A text that fails the check is refused and the file is
/// left as it was.
pub fn save_profile(folder: &Path, name: &str, text: &str) -> Result<RunProfile> {
    let path = folder.join(format!("{name}.json"));
    let origin = path.display().to_string();
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(Error::Config(format!(
            "run profile {origin}: larger than {MAX_FILE_BYTES} bytes"
        )));
    }
    let profile = RunProfile::parse(text, &origin)?;
    if profile.name != name {
        return Err(Error::Config(format!(
            "run profile {origin}: its name is {:?}; keep {name:?}, the file's name",
            profile.name
        )));
    }
    std::fs::create_dir_all(folder)?;
    sparksmith_io::persistence::atomic_write(&path, text.as_bytes())
        .map_err(|e| Error::Config(format!("{origin}: {e}")))?;
    Ok(profile)
}

/// Remove the user's profile `name` from `folder`; a built-in of that name
/// is used again.
pub fn remove_profile(folder: &Path, name: &str) -> Result<()> {
    let path = folder.join(format!("{name}.json"));
    std::fs::remove_file(&path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests;

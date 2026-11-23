//! Library-selected native strategies for presenting contracts and keeping
//! conversation history. Profiles describe presentation only.
use crate::{
    Error, Result,
    chat::{Condensed, Policy, context},
    model::Message,
};
use serde::Deserialize;
use serde_json::Value;
use sparksmith_automation::session::SchemaDetail;
use sparksmith_io::shipped::{Shelf, ShippedLibrary};
use std::path::Path;

use crate::profile::{MAX_FILE_BYTES, PROFILE_FORMAT, valid_name};

#[derive(Clone, Copy, Debug, Default)]
pub struct HarnessSelection<'a> {
    pub model: Option<&'a str>,
    pub workflow: Option<&'a str>,
    pub step: Option<&'a str>,
    pub connection: Option<&'a str>,
}

impl<'a> HarnessSelection<'a> {
    pub fn choose(self, fallback: &'a str) -> &'a str {
        self.connection
            .or(self.step)
            .or(self.workflow)
            .or(self.model)
            .unwrap_or(fallback)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessProfile {
    format: u32,
    pub name: String,
    pub surface_view: SurfaceView,
    reply_shape: ReplyShape,
    condense: Condense,
    nudge: Nudge,
    pub recover: Recover,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceView {
    pub strategy: SchemaDetail,
    full_on_request: bool,
}

/// The native reply and history strategy registries. Serde resolves each
/// stable name strictly; a profile can configure only registered strategies.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReplyStrategy {
    Shorten,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyShape {
    strategy: ReplyStrategy,
    reply_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HistoryStrategy {
    StubsThenSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Condense {
    strategy: HistoryStrategy,
    keep_rounds: usize,
    bulky_chars: usize,
    batch_chars: usize,
    trigger: f64,
    target: f64,
    window_tokens: u64,
}

/// Present a surface contract from the catalog through the describe verb.
pub(crate) trait SurfaceStrategy {
    fn apply(self, verb: &str, args: &mut Value);
}

/// Shape the reply that a caller reads. Document state and grants stay
/// outside the strategy interface.
pub(crate) trait ShapeReply {
    fn render(self, verb: &str, args: &Value, value: Value, budget: usize) -> String;
}
impl ShapeReply for ReplyStrategy {
    fn render(self, verb: &str, args: &Value, value: Value, budget: usize) -> String {
        match self {
            Self::Shorten => context::reply_text(verb, args, value, budget),
        }
    }
}

/// Reduce an existing history and return what changed for the transcript.
pub(crate) trait CondenseHistory {
    fn condense(
        self,
        history: &mut [Message],
        policy: &Policy,
        context_tokens: u64,
        estimate: &dyn Fn(&[Message]) -> u64,
    ) -> Option<Condensed>;
    fn clear_settled(self, history: &mut [Message], policy: &Policy) -> usize;
}
impl CondenseHistory for HistoryStrategy {
    fn condense(
        self,
        history: &mut [Message],
        policy: &Policy,
        context_tokens: u64,
        estimate: &dyn Fn(&[Message]) -> u64,
    ) -> Option<Condensed> {
        match self {
            Self::StubsThenSummary => context::condense(history, policy, context_tokens, estimate),
        }
    }
    fn clear_settled(self, history: &mut [Message], policy: &Policy) -> usize {
        match self {
            Self::StubsThenSummary => context::clear_settled(history, policy),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nudge {
    after_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recover {
    pub repeat_limit: u32,
}

impl SurfaceStrategy for SurfaceView {
    /// An explicit detail belongs to the caller. Omitted detail uses the
    /// server default unless this connection selected the full contract.
    fn apply(self, verb: &str, args: &mut serde_json::Value) {
        if let Some(fields) = args.as_object_mut() {
            sparksmith_automation::mcp::tools::set_default_detail(verb, fields, self.strategy);
        }
    }
}

/// Paths and the fallback named in the installed library.
#[derive(Clone, Debug, PartialEq)]
pub struct HarnessLibrary {
    pub(crate) shipped: ShippedLibrary,
    pub(crate) default: String,
    pub(crate) user_folder: Option<std::path::PathBuf>,
}

impl HarnessLibrary {
    pub(crate) fn select(&self, names: HarnessSelection<'_>) -> Result<HarnessProfile> {
        HarnessProfile::load(
            &self.shipped,
            self.user_folder.as_deref(),
            names.choose(&self.default),
        )
    }
}

impl HarnessProfile {
    fn parse(text: &str, origin: &str) -> Result<Self> {
        let problem = |why: &str| Error::Config(format!("harness profile {origin}: {why}"));
        let mut de = serde_json::Deserializer::from_str(text);
        let profile: Self =
            serde_path_to_error::deserialize(&mut de).map_err(|e| problem(&e.to_string()))?;
        de.end().map_err(|e| problem(&e.to_string()))?;
        if profile.format != PROFILE_FORMAT {
            return Err(problem(&format!(
                "format {} is unsupported",
                profile.format
            )));
        }
        if !valid_name(&profile.name) {
            return Err(problem("name: expected a run profile name"));
        }
        if !profile.surface_view.full_on_request {
            return Err(problem(
                "surface_view.full_on_request must allow the full contract",
            ));
        }
        let policy = profile.policy();
        if policy.keep_rounds == 0 {
            return Err(problem(
                "condense.keep_rounds: keep at least one round so the model can read its latest tool replies",
            ));
        }
        for (field, valid) in [
            ("reply_shape.reply_chars", policy.reply_chars > 0),
            ("condense.bulky_chars", policy.bulky_chars > 0),
            ("condense.batch_chars", policy.batch_chars > 0),
            ("condense.window_tokens", policy.window_tokens > 0),
            (
                "condense.trigger",
                policy.trigger > 0.0 && policy.trigger <= 1.0,
            ),
            (
                "condense.target",
                policy.target > 0.0 && policy.target < policy.trigger,
            ),
            (
                "recover.repeat_limit",
                (2..=20).contains(&profile.recover.repeat_limit),
            ),
        ] {
            if !valid {
                return Err(problem(&format!("{field}: invalid value")));
            }
        }
        Ok(profile)
    }

    pub fn policy(&self) -> Policy {
        Policy {
            reply_chars: self.reply_shape.reply_chars,
            keep_rounds: self.condense.keep_rounds,
            bulky_chars: self.condense.bulky_chars,
            batch_chars: self.condense.batch_chars,
            trigger: self.condense.trigger,
            target: self.condense.target,
            window_tokens: self.condense.window_tokens,
            nudge_tokens: self.nudge.after_tokens,
        }
    }

    pub(crate) fn reply_strategy(&self) -> ReplyStrategy {
        self.reply_shape.strategy
    }
    pub(crate) fn history_strategy(&self) -> HistoryStrategy {
        self.condense.strategy
    }

    /// Resolve one file, preferring the user's copy of the named profile.
    pub fn load(shipped: &ShippedLibrary, user: Option<&Path>, name: &str) -> Result<Self> {
        if !valid_name(name) {
            return Err(Error::Config(format!(
                "invalid harness profile name {name:?}"
            )));
        }
        let filename = format!("{name}.json");
        let rel = format!("harness/{filename}");
        let mut path = shipped.shelf(Shelf::Assist).join(&rel);
        if let Some(folder) = user {
            let candidate = folder.join(&filename);
            match candidate.try_exists() {
                Ok(true) => path = candidate,
                Ok(false) => (),
                Err(e) => return Err(Error::Config(format!("{}: {e}", candidate.display()))),
            }
        }
        let origin = path.display().to_string();
        let text = crate::profile::bounded_text(&path, MAX_FILE_BYTES)?;
        let profile = Self::parse(&text, &origin)?;
        if profile.name != name {
            return Err(Error::Config(format!(
                "harness profile {origin}: name must be {name:?}"
            )));
        }
        Ok(profile)
    }
}

#[cfg(test)]
mod tests;

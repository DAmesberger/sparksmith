//! The chat's loop (`docs/assistant-panel.md` §1, §7): one message of the
//! person is one turn of the model with the six verbs, on any backend,
//! until the model answers without a tool call, the person stops it, a
//! limit is reached or the backend fails.
//!
//! The loop knows no session: every tool call goes to the host's [`Tools`],
//! which runs it through the front door as the built-in agent (the desktop
//! app runs it on the window's engine, in the turn the message opened). The
//! conversation's history is the host's and only grows, so earlier turns
//! reach the model unchanged.
//!
//! What the chat draws arrives as [`Event`]s in the order it happens: per
//! request [`Event::Request`], the reply's reasoning and text, then each
//! tool call's row from `Running` to how it ended, and [`Event::Usage`]
//! against the turn's [`Budget`].
use crate::harness::{
    CondenseHistory, HistoryStrategy, ReplyStrategy, ShapeReply, SurfaceStrategy,
};
use crate::model::{
    Budget, Message, ModelBackend, ModelProfile, ModelRequest, Part, Role, StopReason, ToolSpec,
};
use crate::usage::Tally;
use crate::{Error, Result};
use serde_json::Value;
use sparksmith_io::shipped::{Shelf, ShippedLibrary};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub use crate::profile::ReachProfile;
pub use context::{Condensed, Policy};

pub mod context;

/// The chat's base system prompt, in the library's `assist/` folder.
pub const PROMPT_FILE: &str = "chat-prompt.md";
/// The playbook for building a board, after the prompt.
pub const PLAYBOOK_FILE: &str = "chat-playbook.md";
/// The summariser's instructions for condensing earlier turns.
pub const CONDENSE_FILE: &str = "condense-prompt.md";
/// The note a turn that reads without changing anything is sent.
pub const NUDGE_FILE: &str = "nudge-prompt.md";
/// The chat's reach profile, `assist/reach/chat.json`.
pub const REACH: &str = "chat";
/// Largest shipped file read.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// `{product}` in the prompt stands for the program's name.
const PRODUCT_PLACEHOLDER: &str = "{product}";

/// What ships for the chat: its system prompt and its reach.
#[derive(Clone, Debug)]
pub struct ChatLibrary {
    run_profiles: crate::AssistLibrary,
    /// The prompt, then the playbook.
    pub system: String,
    pub reach: ReachProfile,
    /// The summariser's instructions ([`context::summarise`]).
    pub condense: String,
    /// The harness's note after a long read-only stretch.
    pub nudge: String,
}

impl ChatLibrary {
    /// Locate the shipped library and read the chat's data from it.
    pub fn locate() -> Result<Self> {
        let shipped = ShippedLibrary::locate().map_err(|e| Error::Config(e.to_string()))?;
        Self::load(&shipped)
    }

    /// Read the chat's data from `shipped`; a missing or broken file is
    /// refused with its path.
    pub fn load(shipped: &ShippedLibrary) -> Result<Self> {
        let folder = shipped.shelf(Shelf::Assist);
        let read = |name: &str| -> Result<String> {
            let path = folder.join(name);
            let bytes = shipped
                .read(Shelf::Assist, name, MAX_FILE_BYTES)
                .map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
            String::from_utf8(bytes)
                .map_err(|_| Error::Config(format!("{}: not UTF-8 text", path.display())))
        };
        let nonempty = |name: &str| -> Result<String> {
            let text = read(name)?;
            if text.trim().is_empty() {
                return Err(Error::Config(format!(
                    "{}: the file is empty",
                    folder.join(name).display()
                )));
            }
            Ok(text)
        };
        let system = format!(
            "{}\n\n{}\n\n{}",
            sparksmith_automation::mcp::agent_method(shipped).map_err(Error::Config)?,
            nonempty(PROMPT_FILE)?.trim_end(),
            nonempty(PLAYBOOK_FILE)?.trim_end()
        );
        let condense = nonempty(CONDENSE_FILE)?;
        let nudge = nonempty(NUDGE_FILE)?;
        let file = format!("{}/{REACH}.json", crate::profile::REACH_FOLDER);
        let origin = folder.join(&file).display().to_string();
        let reach = ReachProfile::parse(&read(&file)?, &origin)?;
        if reach.name != REACH {
            return Err(Error::Config(format!(
                "reach profile {origin}: its name is {:?}; the file must be {}.json",
                reach.name, reach.name
            )));
        }
        let product = |text: &str| {
            text.trim_end()
                .replace(PRODUCT_PLACEHOLDER, sparksmith_core::product_name!())
        };
        Ok(Self {
            run_profiles: crate::AssistLibrary::load(shipped)?,
            system: product(&system),
            reach,
            condense: product(&condense),
            nudge: product(&nudge),
        })
    }
}

/// What a turn may use, on which model.
#[derive(Clone, Debug)]
pub struct ChatConfig {
    pub profile: ModelProfile,
    pub system: String,
    pub budget: Budget,
    /// The six verbs, as MCP's `tools/list` defines them.
    pub tools: Vec<ToolSpec>,
    /// How much of the conversation the model is sent.
    pub policy: Policy,
    pub surface_view: crate::harness::SurfaceView,
    reply_strategy: ReplyStrategy,
    history_strategy: HistoryStrategy,
    /// The summariser's instructions.
    pub condense: String,
    /// The note after a long read-only stretch ([`Policy::nudge_tokens`]).
    pub nudge: String,
}

impl ChatConfig {
    /// A chat on `profile` with the shipped prompt, the six verbs and
    /// `budget` per turn (the person's limits, or none).
    pub fn new(library: &ChatLibrary, profile: ModelProfile, budget: Budget) -> Result<Self> {
        Self::with_profiles(library, profile, budget, None)
    }

    pub fn with_profiles(
        library: &ChatLibrary,
        profile: ModelProfile,
        budget: Budget,
        user_profiles: Option<&std::path::Path>,
    ) -> Result<Self> {
        let run =
            crate::RunConfig::new(&library.run_profiles, profile.clone(), None, user_profiles)?;
        let harness = run.harness_for(None, None)?;
        let policy = harness.policy();
        Ok(Self {
            profile,
            system: library.system.clone(),
            budget,
            tools: crate::verbs::tools(),
            policy,
            surface_view: harness.surface_view,
            history_strategy: harness.history_strategy(),
            reply_strategy: harness.reply_strategy(),
            condense: library.condense.clone(),
            nudge: library.nudge.clone(),
        })
    }

    /// Why this model can't chat, if it can't: the chat works through
    /// tools.
    pub fn check(&self) -> Result<()> {
        self.profile.check()?;
        if !self.profile.capabilities.tool_calling {
            return Err(Error::Unsupported(format!(
                "the chat works through tools, and {} is declared without tool calling",
                self.profile.model
            )));
        }
        Ok(())
    }
}

/// Where the model's tool calls run: the host's front door, as the
/// built-in agent. Answers in the transport envelope (`{"ok", "result"}`
/// or `{"ok": false, "error"}`).
pub trait Tools {
    fn call(&mut self, verb: &str, input: &Value) -> Value;
}

/// What a turn's events are: the loop's own (`crate::Event`), the same the
/// datasheet run sends, so the chat draws both alike.
pub use crate::{CallState, Event, Meter};

/// A turn's use.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Use {
    pub requests: u32,
    pub output_tokens: u64,
    /// Prompt plus output tokens, every request counted.
    pub tokens: u64,
    pub seconds: f64,
    pub usage_estimated: bool,
    /// Accounted tokens: uncached, cache written,
    /// cache read and output.
    pub reported: crate::usage::Usage,
}

/// Which limit ended a turn or a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Requests,
    OutputTokens,
    /// Prompt plus output tokens.
    Tokens,
    Time,
}

/// How a turn ended. What its calls changed stays, as one undo step.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// The model answered without a tool call.
    Answered,
    /// The person pressed Stop.
    Stopped,
    /// A limit of the turn was reached: which, and the numbers in words.
    Limit { hit: Hit, why: String },
    /// The reply stopped at the output limit before it was complete.
    CutOff(String),
    /// The model declined.
    Declined(String),
    /// The backend or the transport failed.
    Failed(String),
}

impl From<crate::request::Stopped> for End {
    fn from(stop: crate::request::Stopped) -> Self {
        match stop {
            crate::request::Stopped::Cancelled => Self::Stopped,
            crate::request::Stopped::Limit(hit, why) => Self::Limit { hit, why },
            crate::request::Stopped::Failed(why) => Self::Failed(why),
        }
    }
}

impl std::fmt::Display for End {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Answered => write!(f, "answered"),
            Self::Stopped => write!(f, "stopped"),
            Self::Limit { why, .. } => write!(f, "limit reached: {why}"),
            Self::CutOff(why) => write!(f, "cut off: {why}"),
            Self::Declined(why) => write!(f, "the model declined: {why}"),
            Self::Failed(why) => write!(f, "failed: {why}"),
        }
    }
}

/// A turn's end and its use.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub end: End,
    pub usage: Use,
}

/// Add the person's `message` to `history` and run the model with the
/// verbs until it answers, `cancel` is set, the budget is reached or the
/// backend fails. `history` ends ready for the next message: every tool
/// call it holds has its result.
pub fn turn(
    config: &ChatConfig,
    backend: &mut dyn ModelBackend,
    history: &mut Vec<Message>,
    message: Vec<Part>,
    tools: &mut dyn Tools,
    events: &mut dyn FnMut(Event),
    cancel: &AtomicBool,
) -> Outcome {
    push_user(history, message);
    let started = Instant::now();
    let mut tally = Tally::default();
    let usage = |tally: &Tally| Use {
        requests: tally.requests,
        output_tokens: tally.usage.output_tokens,
        tokens: tally
            .usage
            .prompt()
            .saturating_add(tally.usage.output_tokens),
        seconds: started.elapsed().as_secs_f64(),
        reported: tally.usage,
        usage_estimated: tally.usage_estimated,
    };
    if let Err(error) = config.check() {
        return Outcome {
            end: End::Failed(error.to_string()),
            usage: usage(&tally),
        };
    }
    let budget = &config.budget;
    let profile = &config.profile;
    let policy = &config.policy;
    let fixed =
        (config.system.len() + serde_json::to_string(&config.tools).map_or(0, |t| t.len())) as u64;
    let estimate = |history: &[Message]| -> u64 {
        crate::usage::estimated_tokens(
            history
                .iter()
                .map(|m| crate::model::appended_chars(&m.content))
                .fold(fixed, u64::saturating_add),
        )
    };
    let context = profile.capabilities.context_tokens;
    let window = policy.window(context);
    // At the turn's start, a boundary: condense, and summarise the earlier
    // turns if even that leaves too much.
    let share = |history: &[Message]| estimate(history) as f64 / window.max(1) as f64;
    let before = estimate(history);
    let over = share(history) > policy.trigger;
    let mut condensed = config
        .history_strategy
        .condense(history, policy, context, &estimate);
    if (over || condensed.is_some())
        && share(history) > policy.target
        && let Some((turns, record)) = context::summarise(
            history,
            &config.condense,
            crate::request::Allowance {
                output: budget.max_output_tokens,
                duration: None,
            }
            .output_within(profile.output.per_request),
            profile.request_timeout,
            |request| {
                let prompt = crate::usage::estimated_tokens(
                    request
                        .messages
                        .iter()
                        .map(|message| crate::model::appended_chars(&message.content))
                        .fold(request.system.len() as u64, u64::saturating_add),
                );
                crate::request::Driver {
                    backend,
                    budget,
                    started,
                    tally: &mut tally,
                    cancel,
                    events,
                }
                .send_chat(*request, prompt, context, false)
                .ok()
                .filter(|received| received.terminal.is_none())
                .map(|received| received.reply)
            },
        )
    {
        let shown = condensed.get_or_insert(Condensed {
            cleared: 0,
            turns: 0,
            before_tokens: before,
            after_tokens: 0,
            record: None,
        });
        shown.turns = turns;
        shown.record = Some(record);
        shown.after_tokens = estimate(history);
    }
    if let Some(condensed) = condensed {
        events(Event::Condensed(condensed));
    }
    // Tokens at the turn's last change (or nudge): reading past the
    // policy's allowance with nothing changed sends the model the nudge,
    // with the tool results it reads next; the turn goes on.
    let mut changed_at = 0u64;
    let end = loop {
        let spent = usage(&tally).tokens;
        if spent.saturating_sub(changed_at) > policy.nudge_tokens
            && let Some(last) = history.last_mut()
            && last.role == Role::User
        {
            last.content.push(Part::text(config.nudge.clone()));
            events(Event::Nudged {
                tokens: spent.saturating_sub(changed_at),
                note: config.nudge.clone(),
            });
            changed_at = spent;
        }
        let allowance = match (crate::request::Driver {
            backend,
            budget,
            started,
            tally: &mut tally,
            cancel,
            events,
        })
        .allowance()
        {
            Ok(allowance) => allowance,
            Err(stop) => break stop.into(),
        };
        // Between requests: replies read and moved past go, together.
        let before = estimate(history);
        let cleared = config.history_strategy.clear_settled(history, policy);
        if let Some(condensed) = config
            .history_strategy
            .condense(history, policy, context, &estimate)
        {
            events(Event::Condensed(Condensed {
                cleared: condensed.cleared + cleared,
                before_tokens: before,
                ..condensed
            }));
        } else if cleared > 0 {
            events(Event::Condensed(Condensed {
                cleared,
                turns: 0,
                before_tokens: before,
                after_tokens: estimate(history),
                record: None,
            }));
        }
        let prompt = estimate(history);
        let request = ModelRequest {
            system: &config.system,
            tools: &config.tools,
            messages: history,
            max_output_tokens: allowance.output_within(profile.output.per_request),
            timeout: allowance.timeout_within(profile.request_timeout),
        };
        let received = match (crate::request::Driver {
            backend,
            budget,
            started,
            tally: &mut tally,
            cancel,
            events,
        })
        .send_chat(request, prompt, context, true)
        {
            Ok(received) => received,
            Err(stop) => break stop.into(),
        };
        if let Some(stop) = received.terminal {
            break stop.into();
        }
        let n = received.n;
        let limit = received.output_limit;
        let reply = received.reply;
        if reply.stop == StopReason::Stopped || cancel.load(Ordering::Relaxed) {
            // Stopped while the reply arrived: it is never used.
            break End::Stopped;
        }
        let text: Vec<&str> = reply
            .content
            .iter()
            .filter_map(|part| match part {
                Part::Text { text, .. } if !text.trim().is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect();
        if !text.is_empty() {
            events(Event::Text(text.join("\n\n")));
        }
        if let StopReason::Refusal { category } = &reply.stop {
            break End::Declined(category.clone());
        }
        let cut = reply.stop == StopReason::CutOff;
        let calls: Vec<(String, String, Value, Option<String>)> = reply
            .content
            .iter()
            .filter_map(|part| match part {
                Part::ToolCall {
                    id,
                    name,
                    input,
                    malformed,
                    ..
                } => Some((id.clone(), name.clone(), input.clone(), malformed.clone())),
                _ => None,
            })
            .collect();
        history.push(Message {
            role: Role::Assistant,
            content: reply.content,
        });
        if calls.is_empty() {
            break if cut {
                End::CutOff(format!(
                    "the reply stopped at the output limit of {limit} tokens"
                ))
            } else {
                End::Answered
            };
        }
        let mut results: Vec<Part> = Vec::with_capacity(calls.len());
        let mut left = calls.len();
        for (call, (id, name, mut input, malformed)) in (0u32..).zip(calls) {
            left -= 1;
            let row = |state| Event::Call {
                request: n,
                call,
                name: name.clone(),
                state,
            };
            let not_run = if cut {
                Some((
                    "cut off at the output limit",
                    "Cut off at the output limit before this call was complete; send it again in smaller pieces.",
                ))
            } else if cancel.load(Ordering::Relaxed) {
                Some(("stopped", "Not run: the person stopped the turn."))
            } else {
                None
            };
            if let Some((why, text)) = not_run {
                events(row(CallState::NotRun { why: why.into() }));
                results.push(result(id, text.to_owned(), true));
                continue;
            }
            if let Some(why) = malformed {
                let text =
                    format!("This call was not run: {why}. Send it again with valid arguments.");
                events(row(CallState::Refused {
                    reply: text.clone(),
                }));
                results.push(result(id, text, true));
                continue;
            }
            config.surface_view.apply(&name, &mut input);
            events(row(CallState::Running {
                input: input.clone(),
            }));
            let answer = tools.call(&name, &input);
            let ok = answer["ok"] == true;
            if ok && context::made_progress(&name, &input, &answer["result"]) {
                changed_at = usage(&tally).tokens;
            }
            let mut body = if ok {
                answer["result"].clone()
            } else {
                answer["error"].clone()
            };
            crate::run::integers_as_text(&mut body);
            let used = estimate(history)
                .saturating_add(crate::usage::estimated_tokens(
                    crate::model::appended_chars(&results),
                ))
                .saturating_add(crate::run::ESTIMATE_MARGIN)
                .saturating_add(Policy::notes_tokens(left));
            let text = config.reply_strategy.render(
                &name,
                &input,
                body,
                policy.reply_limit(context, used),
            );
            events(row(if ok {
                CallState::Done {
                    reply: text.clone(),
                    step: None,
                }
            } else {
                CallState::Refused {
                    reply: text.clone(),
                }
            }));
            results.push(result(id, text, !ok));
        }
        history.push(Message {
            role: Role::User,
            content: results,
        });
        if cut {
            break End::CutOff(format!(
                "the reply stopped at the output limit of {limit} tokens"
            ));
        }
    };
    Outcome {
        end,
        usage: usage(&tally),
    }
}

/// The person's message after the history: joined to the last message when
/// that is the person's too (tool results of a stopped turn), so roles
/// alternate as every provider requires.
fn push_user(history: &mut Vec<Message>, message: Vec<Part>) {
    match history.last_mut() {
        Some(last) if last.role == Role::User => last.content.extend(message),
        _ => history.push(Message {
            role: Role::User,
            content: message,
        }),
    }
}

fn result(call_id: String, text: String, error: bool) -> Part {
    Part::ToolResult {
        call_id,
        text,
        error,
        images: Vec::new(),
    }
}

#[cfg(test)]
#[path = "chat/tests.rs"]
mod tests;

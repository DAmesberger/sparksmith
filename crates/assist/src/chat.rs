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

/// The chat's base system prompt, in the library's `assist/` folder.
pub const PROMPT_FILE: &str = "chat-prompt.md";
/// The chat's reach profile, `assist/reach/chat.json`.
pub const REACH: &str = "chat";
/// Largest shipped file read.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Characters of one tool reply the model reads; longer replies are cut
/// with a note (the model can ask for a narrower view).
pub const MAX_REPLY_CHARS: usize = crate::MAX_RESULT_CHARS;
/// Characters per token of the history, for the estimate of the next
/// prompt.
const CHARS_PER_TOKEN: u64 = 3;
/// `{product}` in the prompt stands for the program's name.
const PRODUCT_PLACEHOLDER: &str = "{product}";

/// What ships for the chat: its system prompt and its reach.
#[derive(Clone, Debug)]
pub struct ChatLibrary {
    pub system: String,
    pub reach: ReachProfile,
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
        let system = read(PROMPT_FILE)?;
        if system.trim().is_empty() {
            return Err(Error::Config(format!(
                "{}: the chat's prompt is empty",
                folder.join(PROMPT_FILE).display()
            )));
        }
        let file = format!("{}/{REACH}.json", crate::profile::REACH_FOLDER);
        let origin = folder.join(&file).display().to_string();
        let reach = ReachProfile::parse(&read(&file)?, &origin)?;
        if reach.name != REACH {
            return Err(Error::Config(format!(
                "reach profile {origin}: its name is {:?}; the file must be {}.json",
                reach.name, reach.name
            )));
        }
        Ok(Self {
            system: system
                .trim_end()
                .replace(PRODUCT_PLACEHOLDER, sparksmith_core::product_name!()),
            reach,
        })
    }
}

/// The chat's budget: no per-turn caps (founder, 16 November 2026); a long
/// conversation is compacted instead (a later slice). Stop still ends a
/// turn.
pub fn unbounded() -> Budget {
    Budget {
        max_requests: u32::MAX,
        max_output_tokens: u64::MAX,
        max_total_tokens: u64::MAX,
        max_duration: std::time::Duration::MAX,
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
}

impl ChatConfig {
    /// A chat on `profile` with the shipped prompt, the six verbs and
    /// `budget` per turn.
    pub fn new(library: &ChatLibrary, profile: ModelProfile, budget: Budget) -> Self {
        Self {
            profile,
            system: library.system.clone(),
            budget,
            tools: crate::verbs::tools(),
        }
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

/// Where a tool call is.
#[derive(Clone, Debug, PartialEq)]
pub enum CallState {
    /// It runs, with this input.
    Running { input: Value },
    /// It ran: the reply the model reads.
    Done { reply: String },
    /// It was refused or failed; the model reads why.
    Refused { reply: String },
    /// It never ran, and why: cut off at the output limit, or stopped.
    NotRun { why: String },
}

/// Use against the turn's budget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Use {
    pub requests: u32,
    pub output_tokens: u64,
    /// Prompt plus output tokens, every request counted.
    pub tokens: u64,
    pub seconds: f64,
}

/// One thing that happened in a turn, for the chat to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Request `n` of the turn is about to go out.
    Request { n: u32 },
    /// Answer text as it arrives (a streaming backend).
    TextDelta(String),
    /// Reasoning as it arrives; the chat folds it.
    ReasoningDelta(String),
    /// What the model reasoned in this reply, whole.
    Reasoning(String),
    /// The reply's answer text, whole: it replaces what the deltas showed.
    Text(String),
    /// A tool-call row. `call` counts request `request`'s calls from 0, so
    /// the two name the row; each new state replaces the last.
    Call {
        request: u32,
        call: u32,
        name: String,
        state: CallState,
    },
    /// Use so far, after each response.
    Usage(Use),
}

/// How a turn ended. What its calls changed stays, as one undo step.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// The model answered without a tool call.
    Answered,
    /// The person pressed Stop.
    Stopped,
    /// A limit of the turn was reached; which.
    Limit(String),
    /// The reply stopped at the output limit before it was complete.
    CutOff(String),
    /// The model declined.
    Declined(String),
    /// The backend or the transport failed.
    Failed(String),
}

impl std::fmt::Display for End {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Answered => write!(f, "answered"),
            Self::Stopped => write!(f, "stopped"),
            Self::Limit(why) => write!(f, "limit reached: {why}"),
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
        tokens: tally.usage.prompt().saturating_add(tally.usage.output_tokens),
        seconds: started.elapsed().as_secs_f64(),
    };
    if let Err(error) = config.check() {
        return Outcome {
            end: End::Failed(error.to_string()),
            usage: usage(&tally),
        };
    }
    let budget = &config.budget;
    let profile = &config.profile;
    let fixed = (config.system.len() + serde_json::to_string(&config.tools).map_or(0, |t| t.len()))
        as u64;
    let end = loop {
        if cancel.load(Ordering::Relaxed) {
            break End::Stopped;
        }
        if tally.requests >= budget.max_requests {
            break End::Limit(format!("{} requests", budget.max_requests));
        }
        let left = budget.max_duration.saturating_sub(started.elapsed());
        if left.is_zero() {
            break End::Limit(format!(
                "{} minutes",
                budget.max_duration.as_secs().div_ceil(60)
            ));
        }
        let output_left = budget
            .max_output_tokens
            .saturating_sub(tally.usage.output_tokens);
        let limit = u64::from(profile.output.per_request).min(output_left);
        if limit == 0 {
            break End::Limit(format!("{} output tokens", budget.max_output_tokens));
        }
        let prompt = history
            .iter()
            .map(|m| crate::model::appended_chars(&m.content))
            .fold(fixed, u64::saturating_add)
            .div_ceil(CHARS_PER_TOKEN);
        let used = tally
            .usage
            .prompt()
            .saturating_add(tally.usage.output_tokens);
        if used.saturating_add(prompt).saturating_add(limit) > budget.max_total_tokens {
            break End::Limit(format!(
                "{} tokens: {used} used, and the next request could take {}",
                budget.max_total_tokens,
                prompt + limit
            ));
        }
        let request = ModelRequest {
            system: &config.system,
            tools: &config.tools,
            messages: history,
            max_output_tokens: u32::try_from(limit).unwrap_or(profile.output.per_request),
            timeout: profile.request_timeout.min(left),
        };
        let n = tally.requests.saturating_add(1);
        events(Event::Request { n });
        let reply = match backend.send(&request) {
            Ok(reply) => reply,
            Err(error) => break End::Failed(error.to_string()),
        };
        tally.book(&reply.usage);
        events(Event::Usage(usage(&tally)));
        if cancel.load(Ordering::Relaxed) {
            // Stopped while the reply arrived: it is never used.
            break End::Stopped;
        }
        for part in &reply.content {
            match part {
                Part::Reasoning { text, .. } if !text.trim().is_empty() => {
                    events(Event::Reasoning(text.clone()));
                }
                Part::Text { text, .. } if !text.trim().is_empty() => {
                    events(Event::Text(text.clone()));
                }
                _ => {}
            }
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
        let mut results = Vec::with_capacity(calls.len());
        for (call, (id, name, input, malformed)) in (0u32..).zip(calls) {
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
            events(row(CallState::Running {
                input: input.clone(),
            }));
            let answer = tools.call(&name, &input);
            let ok = answer["ok"] == true;
            let mut body = if ok {
                answer["result"].clone()
            } else {
                answer["error"].clone()
            };
            crate::run::integers_as_text(&mut body);
            let text = cut_reply(body.to_string());
            events(row(if ok {
                CallState::Done {
                    reply: text.clone(),
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

/// `text` cut to [`MAX_REPLY_CHARS`], saying so.
fn cut_reply(mut text: String) -> String {
    if text.len() > MAX_REPLY_CHARS {
        let mut end = MAX_REPLY_CHARS;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("… [cut: the result exceeded the tool reply limit; ask for a narrower view]");
    }
    text
}

#[cfg(test)]
#[path = "chat/tests.rs"]
mod tests;

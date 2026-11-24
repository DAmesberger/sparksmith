//! What the model is sent of a conversation (`docs/assistant-panel.md`
//! §6.5, slice K1). The person's transcript never changes; only the
//! history the model reads shrinks, deterministically and without a model
//! request except for the last resort.
//!
//! - **Small replies.** A tool reply over [`Policy::reply_chars`] is
//!   shortened as JSON ([`shrink`]): long lists keep their first items and
//!   say how many more, long texts are cut, deep objects are counted. The
//!   reply says it was shortened and how to ask for less.
//! - **No re-sending of bulky replies.** A reply over
//!   [`Policy::bulky_chars`] that the model has read and moved past (older
//!   than the last [`Policy::keep_rounds`] rounds of calls) is replaced by
//!   a stub naming the call that made it, so the model asks again if it
//!   needs it. They are cleared together once [`Policy::batch_chars`] have
//!   piled up, so the prompt cache is rebuilt rarely.
//! - **Condensing (K1).** When the estimated prompt passes
//!   [`Policy::trigger`] of the model's context, every earlier reply is
//!   cleared the same way and a kept record ([`record`]) of what was asked
//!   and built (ids, designators, revisions) is put in front. At a turn
//!   boundary, if the prompt is still above [`Policy::target`], the earlier
//!   turns' prose is summarised by the model into that record
//!   ([`summarise`]).
use crate::model::{Message, ModelReply, ModelRequest, Part, Role, StopReason};
use serde_json::{Map, Value};
use std::time::Duration;

/// Marks a cleared reply; never cleared again.
pub const STUB: &str = "[Cleared";
/// Marks the kept record at the head of the history.
pub const RECORD: &str = "[Kept record";

/// How much of the conversation the model is sent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    /// Characters of one tool reply the model reads, at most.
    pub reply_chars: usize,
    /// Within [`Policy::reply_chars`], what the model's context leaves room
    /// for; none: the characters alone.
    pub reply_context: Option<ReplyContext>,
    /// The last rounds of calls (a request's calls and their replies) whose
    /// replies always stay.
    pub keep_rounds: usize,
    /// A reply at least this long is cleared once it is older.
    pub bulky_chars: usize,
    /// Older bulky replies are cleared together once their JSON strings occupy this many bytes.
    pub batch_chars: usize,
    /// Condense when the prompt passes this share of the context.
    pub trigger: f64,
    /// Summarise earlier prose when, cleared, it is still above this share.
    pub target: f64,
    /// The context the shares are of, at most: every request re-sends the
    /// conversation, so on a model with a very large context the trigger
    /// is of this working size, not of the whole context.
    pub window_tokens: u64,
    /// Tokens a turn may spend after its last change (or its start, or the
    /// last nudge) with no edit or action succeeding before the harness
    /// nudges the model to act or ask the person. The turn goes on.
    pub nudge_tokens: u64,
}

/// How much of the context's free room one tool reply may take, so several
/// large replies in a row still leave room to answer.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyContext {
    /// Share of the context kept free of replies for the model's answer.
    pub answer_share: f64,
    /// Share of the room left beside it that one reply may take.
    pub reply_share: f64,
    /// A reply may always take this many characters (a datasheet page),
    /// while that leaves the next request its least room to answer and the
    /// margin for text denser than the estimate.
    pub least_chars: usize,
}

impl Policy {
    /// Characters the next tool reply may have on a model with
    /// `context_tokens`, when the conversation already takes `used_tokens`.
    /// A reply over it is shortened by the profile's strategy; the note
    /// that says so always fits.
    pub fn reply_limit(&self, context_tokens: u64, used_tokens: u64) -> usize {
        let Some(share) = self.reply_context else {
            return self.reply_chars;
        };
        let room = context_tokens.saturating_sub(used_tokens);
        let answer = (context_tokens as f64 * share.answer_share) as u64;
        let part = (room.saturating_sub(answer) as f64 * share.reply_share) as u64;
        let chars =
            |tokens| usize::try_from(crate::usage::estimated_bytes(tokens)).unwrap_or(usize::MAX);
        let kept = crate::run::MIN_OUTPUT_ROOM.saturating_add(crate::run::ESTIMATE_MARGIN);
        let most = chars(room.saturating_sub(kept));
        self.reply_chars
            .min(chars(part).max(share.least_chars))
            .min(most.max(CUT_NOTE.len()))
    }

    /// The tokens the notes of `calls` more replies take at least.
    pub(crate) fn notes_tokens(calls: usize) -> u64 {
        crate::usage::estimated_tokens((CUT_NOTE.len() as u64).saturating_mul(calls as u64))
    }

    /// Nothing shortened or cleared: the conversation as it grew (what the
    /// chat did before K1), for comparisons.
    pub const KEEP_ALL: Self = Self {
        reply_chars: crate::MAX_RESULT_CHARS,
        reply_context: None,
        keep_rounds: usize::MAX,
        bulky_chars: usize::MAX,
        batch_chars: usize::MAX,
        trigger: f64::INFINITY,
        target: f64::INFINITY,
        window_tokens: u64::MAX,
        nudge_tokens: u64::MAX,
    };

    /// The context the trigger and the target are shares of.
    pub fn window(&self, context_tokens: u64) -> u64 {
        context_tokens.min(self.window_tokens)
    }
}

/// What one condensing did, for the chat's divider row.
#[derive(Clone, Debug, PartialEq)]
pub struct Condensed {
    /// Tool replies cleared.
    pub cleared: usize,
    /// Earlier person turns summarised (0 when only replies were cleared).
    pub turns: usize,
    /// Estimated prompt tokens before and after.
    pub before_tokens: u64,
    pub after_tokens: u64,
    /// The kept record, when one was written.
    pub record: Option<String>,
}

// ---------------------------------------------------------------------------
// Shortening one reply

/// Catalog pages already have server-side pagination. Keep their contracts
/// and approvals whole when a profile shortens other tool replies.
pub(crate) fn reply_text(verb: &str, args: &Value, mut value: Value, budget: usize) -> String {
    match verb {
        crate::verbs::DESCRIBE => complete_permissions(&mut value),
        crate::verbs::FIND => {}
        crate::verbs::QUERY if args["name"] == "project.schema" => {
            if let Some(entries) = value.get_mut("matches").and_then(Value::as_array_mut) {
                for entry in entries {
                    complete_permissions(entry);
                }
            }
        }
        _ => return shrink(&value, budget),
    }
    value.to_string()
}

/// Older full contracts omit some gate metadata. Show the catalog's own
/// declarations on those model-facing views, leaving definition pages and
/// ordinary tool results alone.
fn complete_permissions(entry: &mut Value) {
    if entry.get("schema").is_none() || entry.get("class").is_none() || entry.get("reach").is_some()
    {
        return;
    }
    let Some(meta) = entry["name"]
        .as_str()
        .and_then(sparksmith_automation::session::metadata::find)
    else {
        return;
    };
    entry["person"] = serde_json::json!(meta.person);
    entry["reach"] = serde_json::json!(meta.reach.iter().collect::<Vec<_>>());
    if meta.takes_path {
        entry["asks_with_path"] = Value::Bool(true);
    }
}

/// Ends a reply no shortening made fit.
const CUT_NOTE: &str =
    "… [cut: the reply exceeded the chat's reply limit; ask for a narrower view]";

/// `value` as text of at most `budget` characters: as it is when it fits,
/// else shortened as JSON, saying so.
pub fn shrink(value: &Value, budget: usize) -> String {
    let whole = value.to_string();
    if whole.len() <= budget {
        return whole;
    }
    // Ever tighter: items per list, characters per text, depth.
    const STEPS: [(usize, usize, usize); 6] = [
        (12, 400, 8),
        (6, 200, 6),
        (4, 120, 5),
        (3, 80, 4),
        (2, 60, 3),
        (1, 40, 2),
    ];
    let note = |shown: usize| {
        format!(
            "shortened from {} to about {shown} characters: lists keep their first items, long texts and deep objects are cut. Ask a narrower query (a filter, a limit, project.inspect {{\"at\": \"/path\"}}) for the rest",
            whole.len()
        )
    };
    for (items, chars, depth) in STEPS {
        let mut cut = cut_value(value, items, chars, depth);
        let shown = cut.to_string().len();
        mark(&mut cut, note(shown));
        let text = cut.to_string();
        if text.len() <= budget {
            return text;
        }
    }
    let (prefix_bytes, suffix) = if budget >= CUT_NOTE.len() {
        (budget.saturating_sub(200), CUT_NOTE)
    } else {
        // Preserve a prefix even when the profile leaves room only for an ellipsis.
        let marker = match budget {
            0 => "",
            1 | 2 => ".",
            _ => "…",
        };
        (budget - marker.len(), marker)
    };
    let mut text = whole;
    text.truncate(text.floor_char_boundary(prefix_bytes));
    text.push_str(suffix);
    text
}

fn mark(value: &mut Value, note: String) {
    match value {
        Value::Object(map) => {
            map.insert("_shortened".into(), Value::String(note));
        }
        other => {
            let inner = std::mem::take(other);
            let mut map = Map::new();
            map.insert("_shortened".into(), Value::String(note));
            map.insert("value".into(), inner);
            *other = Value::Object(map);
        }
    }
}

fn cut_value(value: &Value, items: usize, chars: usize, depth: usize) -> Value {
    match value {
        Value::String(text) if text.len() > chars => {
            let mut end = chars;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(format!("{}… ({} characters)", &text[..end], text.len()))
        }
        Value::Array(list) => {
            if depth == 0 {
                return Value::String(format!("[{} items]", list.len()));
            }
            let mut out: Vec<Value> = list
                .iter()
                .take(items)
                .map(|v| cut_value(v, items, chars, depth - 1))
                .collect();
            if list.len() > items {
                out.push(Value::String(format!("… {} more", list.len() - items)));
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            if depth == 0 {
                return Value::String(format!("{{{} keys}}", map.len()));
            }
            Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), cut_value(v, items, chars, depth - 1)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Clearing older replies

/// The index from which replies always stay: the start of the last
/// `keep_rounds` rounds of calls.
fn kept_from(history: &[Message], keep_rounds: usize) -> usize {
    if keep_rounds == usize::MAX {
        return 0;
    }
    let mut rounds = 0;
    for (i, message) in history.iter().enumerate().rev() {
        let calls = message.role == Role::Assistant
            && message
                .content
                .iter()
                .any(|p| matches!(p, Part::ToolCall { .. }));
        if calls {
            rounds += 1;
            if rounds >= keep_rounds {
                return i;
            }
        }
    }
    0
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Effect {
    Rehearsed,
    Committed,
    Performed,
    Unconfirmed,
}

impl Effect {
    fn label(self) -> &'static str {
        match self {
            Self::Rehearsed => "Rehearsed",
            Self::Committed => "Committed",
            Self::Performed => "Performed",
            Self::Unconfirmed => "No change confirmed",
        }
    }
}

fn revision(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

#[derive(Clone, Copy)]
struct Intent {
    edit_or_action: bool,
    dry_run: bool,
    expected: Option<u64>,
    asked: bool,
}

impl Intent {
    fn new(verb: &str, input: &Value) -> Self {
        Self {
            edit_or_action: matches!(verb, "apply" | "act"),
            dry_run: input["dry_run"] == true,
            expected: revision(&input["expected_revision"]),
            asked: verb == "show"
                && matches!(input["name"].as_str(), Some("assist.ask" | "assist.offer")),
        }
    }

    fn effect(self, result: &Value) -> Effect {
        if self.dry_run || result["dry_run"] == true {
            return Effect::Rehearsed;
        }
        if self.asked {
            return Effect::Performed;
        }
        if !self.edit_or_action {
            return Effect::Unconfirmed;
        }
        let changes = &result["changes"];
        let changed_objects = ["added", "removed", "changed"].iter().any(|key| {
            changes[*key].as_object().is_some_and(|map| {
                map.values()
                    .any(|ids| ids.as_array().is_some_and(|list| !list.is_empty()))
            })
        });
        if changed_objects
            || changes["fields"]
                .as_array()
                .is_some_and(|fields| !fields.is_empty())
            || self
                .expected
                .zip(revision(&result["revision"]))
                .is_some_and(|(before, after)| after > before)
        {
            Effect::Committed
        } else if result["saved_path"].is_string()
            || result["exported_path"].is_string()
            || result["records_written"]
                .as_u64()
                .is_some_and(|count| count > 0)
            || (result["output"].is_string()
                && result["files"]
                    .as_array()
                    .is_some_and(|files| !files.is_empty()))
        {
            Effect::Performed
        } else {
            Effect::Unconfirmed
        }
    }
}

pub(super) fn made_progress(verb: &str, input: &Value, result: &Value) -> bool {
    matches!(
        Intent::new(verb, input).effect(result),
        Effect::Committed | Effect::Performed
    )
}

struct RecordedCall {
    label: String,
    intent: Intent,
}

fn calls_by_id(history: &[Message]) -> std::collections::HashMap<String, RecordedCall> {
    let mut calls = std::collections::HashMap::new();
    for message in history {
        for part in &message.content {
            if let Part::ToolCall {
                id, name, input, ..
            } = part
            {
                calls.insert(
                    id.clone(),
                    RecordedCall {
                        label: format!("{name} {}", clip(input.to_string(), 240)),
                        intent: Intent::new(name, input),
                    },
                );
            }
        }
    }
    calls
}

const KEPT: &str = "It gave:";

fn stub(call: Option<&RecordedCall>, reply: &str) -> String {
    let chars = reply.len();
    let mut kept = String::new();
    if let Some(call) = call.filter(|c| c.intent.edit_or_action) {
        let value = serde_json::from_str(reply).unwrap_or(Value::Null);
        let effect = call.intent.effect(&value);
        let mut facts = Vec::new();
        facts_of(&value, 0, &mut facts);
        kept = format!(" {}. {KEPT} {}.", effect.label(), facts.join(", "));
    }
    let label = call.map_or("a call", |c| c.label.as_str());
    format!(
        "{STUB}: the reply to {label} ({chars} characters) was read and cleared to save context.{kept} Call it again if you need it.]"
    )
}

/// Clear the replies before `from` that `clear` picks; how many, and the
/// characters saved.
fn clear_before(
    history: &mut [Message],
    from: usize,
    clear: impl Fn(&str) -> bool,
) -> (usize, usize) {
    let calls = calls_by_id(history);
    let (mut count, mut saved) = (0, 0);
    for message in history.iter_mut().take(from) {
        for part in &mut message.content {
            if let Part::ToolResult {
                call_id,
                text,
                images,
                ..
            } = part
                && !text.starts_with(STUB)
                && clear(text)
            {
                let replaced = stub(calls.get(call_id.as_str()), text);
                saved += text.len().saturating_sub(replaced.len());
                *text = replaced;
                images.clear();
                count += 1;
            }
        }
    }
    (count, saved)
}

/// Bytes that a plain UTF-8 string occupies in the request JSON,
/// including its quotes. Count escapes without allocating another string.
fn json_string_bytes(text: &str) -> usize {
    text.bytes().fold(2usize, |bytes, byte| {
        let encoded = match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        };
        bytes.saturating_add(encoded)
    })
}

/// Clear older bulky replies once enough have piled up: how many were
/// cleared (0 when they were left).
pub fn clear_settled(history: &mut [Message], policy: &Policy) -> usize {
    let from = kept_from(history, policy.keep_rounds);
    let mut piled = 0usize;
    let batch_ready = history
        .iter()
        .take(from)
        .flat_map(|m| &m.content)
        .any(|part| {
            let Part::ToolResult { text, .. } = part else {
                return false;
            };
            if text.len() < policy.bulky_chars || text.starts_with(STUB) {
                return false;
            }
            piled = piled.saturating_add(json_string_bytes(text));
            piled >= policy.batch_chars
        });
    if !batch_ready {
        return 0;
    }
    clear_before(history, from, |text| text.len() >= policy.bulky_chars).0
}

/// Replies this short are kept even when condensing: an edit's ids.
const SMALL_REPLY: usize = 400;

/// Condense when the estimated prompt passes the trigger: clear every
/// earlier reply that is not small (an edit's stub keeps its ids). The
/// kept record is what the divider shows; it goes into the history only
/// with a summary ([`summarise`]), since until then the person's words and
/// every edit's ids are still there.
pub fn condense(
    history: &mut [Message],
    policy: &Policy,
    context_tokens: u64,
    estimate: &dyn Fn(&[Message]) -> u64,
) -> Option<Condensed> {
    let before = estimate(history);
    if (before as f64) <= policy.trigger * policy.window(context_tokens) as f64 {
        return None;
    }
    let record = record(history);
    let from = kept_from(history, policy.keep_rounds);
    let (cleared, _) = clear_before(history, from, |text| text.len() > SMALL_REPLY);
    if cleared == 0 {
        // Nothing left to clear: only a summary can help.
        return None;
    }
    Some(Condensed {
        cleared,
        turns: 0,
        before_tokens: before,
        after_tokens: estimate(history),
        record: Some(record),
    })
}

/// Put `record` at the head of the first message, replacing an earlier one.
fn put_record(history: &mut [Message], record: &str) {
    let Some(first) = history.first_mut() else {
        return;
    };
    first
        .content
        .retain(|p| !matches!(p, Part::Text { text, .. } if text.starts_with(RECORD)));
    first.content.insert(0, Part::text(record));
}

// ---------------------------------------------------------------------------
// The kept record

/// Keys of an edit's reply worth keeping: what names what was built.
const KEPT_KEYS: [&str; 12] = [
    "instance",
    "occurrence",
    "designator",
    "component",
    "net",
    "track",
    "via",
    "fill",
    "id",
    "ids",
    "name",
    "drafts",
];
/// Facts kept from one reply.
const MAX_FACTS: usize = 12;
/// Characters kept of one of the person's messages.
const MAX_ASKED: usize = 400;

/// The kept record of `history`: what the person asked, every edit that
/// was made with the ids and names its reply gave, and how many calls were
/// refused. Built from the history alone, the same on every model.
pub fn record(history: &[Message]) -> String {
    let calls = calls_by_id(history);
    let mut asked = Vec::new();
    let mut made = Vec::new();
    let mut refused = 0usize;
    let mut revision = None;
    for message in history {
        for part in &message.content {
            match part {
                Part::Text { text, .. }
                    if message.role == Role::User && !text.starts_with(RECORD) =>
                {
                    asked.push(clip(person_words(text), MAX_ASKED));
                }
                Part::Text { text, .. } if text.starts_with(RECORD) => {
                    // An earlier summary's record carries on: its turns
                    // are gone from the history.
                    for line in text.lines() {
                        if let Some(line) = line.strip_prefix("- ") {
                            made.push(line.to_owned());
                        } else if let Some(line) = line.strip_prefix("* ") {
                            asked.push(line.to_owned());
                        }
                    }
                }
                Part::ToolResult {
                    call_id,
                    text,
                    error,
                    ..
                } => {
                    if *error {
                        refused += 1;
                        continue;
                    }
                    let Some(call) = calls.get(call_id) else {
                        continue;
                    };
                    if !call.intent.edit_or_action {
                        continue;
                    }
                    let (effect, facts) = if text.starts_with(STUB) {
                        let (effect, _) = text.split_once(KEPT).unwrap_or(("", ""));
                        let effect = [Effect::Committed, Effect::Performed, Effect::Rehearsed]
                            .into_iter()
                            .find(|e| {
                                effect
                                    .trim_end()
                                    .strip_suffix('.')
                                    .is_some_and(|s| s.ends_with(e.label()))
                            })
                            .unwrap_or(Effect::Unconfirmed);
                        (
                            effect,
                            text.split_once(KEPT)
                                .and_then(|(_, rest)| rest.split_once(". Call it again"))
                                .map(|(facts, _)| facts.trim().to_owned())
                                .unwrap_or_default(),
                        )
                    } else if let Ok(reply) = serde_json::from_str::<Value>(text) {
                        if let Some(r) = reply.get("revision") {
                            revision = Some(r.to_string());
                        }
                        let mut facts = Vec::new();
                        facts_of(&reply, 0, &mut facts);
                        (call.intent.effect(&reply), facts.join(", "))
                    } else {
                        (call.intent.effect(&Value::Null), String::new())
                    };
                    let detail = if facts.is_empty() {
                        String::new()
                    } else {
                        format!(" → {facts}")
                    };
                    made.push(format!(
                        "{} · {}{detail}",
                        effect.label(),
                        clip(call.label.clone(), 160)
                    ));
                }
                _ => {}
            }
        }
    }
    let mut out = format!(
        "{RECORD} of the earlier conversation: older tool replies were cleared to save context; query again for anything not here.]"
    );
    if let Some(revision) = revision {
        out.push_str(&format!("\nLast revision seen: {revision}."));
    }
    out.push_str("\nThe person asked:");
    for line in &asked {
        out.push_str(&format!("\n* {line}"));
    }
    out.push_str("\nEarlier edit and action calls:");
    for line in &made {
        out.push_str(&format!("\n- {line}"));
    }
    if refused > 0 {
        out.push_str(&format!("\nRefused calls: {refused}."));
    }
    out
}

/// The person's own words of a message: after the window's context line.
fn person_words(text: &str) -> String {
    text.split_once("\n\n")
        .filter(|(head, _)| head.starts_with("The window shows:"))
        .map_or(text, |(_, rest)| rest)
        .trim()
        .to_owned()
}

fn clip(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

fn facts_of(value: &Value, depth: usize, facts: &mut Vec<String>) {
    if depth > 4 || facts.len() >= MAX_FACTS {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if facts.len() >= MAX_FACTS {
                    return;
                }
                let scalar = matches!(inner, Value::Number(_) | Value::String(_));
                if KEPT_KEYS.contains(&key.as_str()) && scalar {
                    facts.push(format!("{key} {}", clip(inner.to_string(), 60)));
                } else if KEPT_KEYS.contains(&key.as_str())
                    && inner.as_object().is_some_and(|o| o.contains_key("id"))
                {
                    facts.push(format!("{key} {}", clip(inner.to_string(), 80)));
                } else if matches!(key.as_str(), "outcome" | "outcomes" | "result" | "results")
                    || inner.is_object()
                    || inner.is_array()
                {
                    facts_of(inner, depth + 1, facts);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter().take(MAX_FACTS) {
                facts_of(item, depth + 1, facts);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Summarising earlier prose (the last resort)

/// Where the latest person turn starts: the last message of the person's
/// own words that carries no tool replies.
fn latest_turn(history: &[Message]) -> Option<usize> {
    history.iter().rposition(|m| {
        m.role == Role::User
            && m.content
                .iter()
                .any(|p| matches!(p, Part::Text { text, .. } if !text.starts_with(RECORD)))
            && !m
                .content
                .iter()
                .any(|p| matches!(p, Part::ToolResult { .. }))
    })
}

/// At a turn boundary, when even condensed the prompt is above the
/// target: have the model summarise the turns before the latest, and put
/// the summary with the kept record in their place. `prompt` is the
/// summariser's instructions (`library/assist/condense-prompt.md`). `None`
/// when nothing earlier is left to summarise, or the model failed (the
/// history is then unchanged).
pub fn summarise(
    history: &mut Vec<Message>,
    prompt: &str,
    max_output_tokens: u32,
    timeout: Duration,
    send: impl FnOnce(&ModelRequest<'_>) -> Option<ModelReply>,
) -> Option<(usize, String)> {
    let cut = latest_turn(history).filter(|cut| *cut > 0)?;
    let record = record(&history[..cut]);
    let turns = history[..cut]
        .iter()
        .filter(|m| {
            m.role == Role::User
                && !m
                    .content
                    .iter()
                    .any(|p| matches!(p, Part::ToolResult { .. }))
        })
        .count();
    // The earlier turns as text: the summariser reads, never calls.
    let mut transcript = String::new();
    for message in &history[..cut] {
        for part in &message.content {
            let line = match part {
                Part::Text { text, .. } => text.clone(),
                Part::ToolCall { name, input, .. } => format!("(called {name} {input})"),
                Part::ToolResult { text, .. } => format!("(reply: {})", clip(text.clone(), 300)),
                _ => continue,
            };
            let who = match message.role {
                Role::User => "Person",
                Role::Assistant => "Assistant",
            };
            transcript.push_str(&format!("{who}: {line}\n"));
        }
    }
    let messages = [Message {
        role: Role::User,
        content: vec![Part::text(format!(
            "{record}\n\nThe conversation so far:\n{transcript}"
        ))],
    }];
    let request = ModelRequest {
        system: prompt,
        tools: &[],
        messages: &messages,
        max_output_tokens,
        timeout,
    };
    let reply = send(&request)?;
    if reply.stop != StopReason::EndTurn {
        return None;
    }
    let summary: String = reply
        .content
        .iter()
        .filter_map(|p| match p {
            Part::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if summary.trim().is_empty() {
        return None;
    }
    let kept = format!(
        "{record}\nSummary of the earlier turns:\n{}",
        summary.trim()
    );
    let rest = history.split_off(cut);
    *history = rest;
    put_record(history, &kept);
    Some((turns, kept))
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;

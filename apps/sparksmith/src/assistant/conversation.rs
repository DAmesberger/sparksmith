//! One conversation as the chat shows it (`docs/assistant-panel.md` §7):
//! the transcript's items, folded from the loop's events, and the state
//! the column's header names. No egui here.
use serde_json::Value;
use sparksmith_assist::chat::{self, CallState, Event, Hit, Use};
use sparksmith_assist::price::Price;
use sparksmith_assist::settings::{Bound, Limits};
use sparksmith_assist::usage::Usage;

/// Characters of a call's input or reply shown when its row is expanded.
pub const MAX_DETAIL_CHARS: usize = 4_000;
/// Items one conversation keeps; the oldest go first.
pub const MAX_ITEMS: usize = 2_000;

/// Where the conversation is.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// No model is set up, and why.
    NotSetUp(String),
    Idle,
    /// A turn runs.
    Running,
    /// A turn waits for the person's Allow or Deny.
    Waiting,
    /// The person stopped the last turn.
    Stopped,
    /// The last turn ended early, and why (a limit, a failed request).
    Error(String),
}

impl State {
    /// The header's words for it.
    pub fn label(&self) -> &str {
        match self {
            Self::NotSetUp(_) => "Not connected",
            Self::Idle => "Ready",
            Self::Running => "Working…",
            Self::Waiting => "Waiting for you",
            Self::Stopped => "Stopped",
            Self::Error(_) => "Stopped early",
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
}

/// How a tool-call row ended, or that it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowState {
    /// The model is writing it.
    Writing,
    Running,
    Done,
    Refused,
    NotRun(String),
}

/// One tool call: the verb and the entries it names, its state, and what
/// it sent and got (shown when expanded).
#[derive(Clone, Debug, PartialEq)]
pub struct CallRow {
    pub request: u32,
    pub call: u32,
    /// "query project.issues", "apply layout.add_via".
    pub line: String,
    pub state: RowState,
    pub input: String,
    pub reply: String,
}

/// A question for the person: the gate's Ask.
#[derive(Clone, Debug, PartialEq)]
pub struct AskCard {
    pub id: u64,
    /// The entry's title ("Save the project").
    pub command: String,
    pub why: String,
    pub paths: Vec<String>,
    /// The person's answer; `None` while it waits.
    pub answer: Option<Allow>,
}

/// The answers an Ask card offers, as a coding harness does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allow {
    /// This call only.
    Once,
    /// This entry, until the app quits.
    Session,
    /// This entry from now on.
    Always,
    Deny,
}

impl Allow {
    /// The card's note once answered.
    pub fn said(self) -> &'static str {
        match self {
            Self::Once => "Allowed once.",
            Self::Session => "Allowed for this session.",
            Self::Always => "Always allowed.",
            Self::Deny => "Denied.",
        }
    }
}

/// The card at the end of each turn.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnSummary {
    pub id: u64,
    /// The turn's undo label.
    pub label: String,
    /// "Vias: 2 added", "Parts: U1 changed", one line per collection.
    pub changes: Vec<String>,
    /// What others changed during the turn (the person's edit, an undo),
    /// as `changes` words it: not the assistant's.
    pub others: Vec<String>,
    pub issues_new: usize,
    pub issues_resolved: usize,
    /// The new issues' messages, as core words them (first few).
    pub new_issues: Vec<String>,
    pub usage: Use,
    /// The revision before and after the turn: Undo turn is offered while
    /// the document still stands at `after` and the turn changed it.
    pub before: u64,
    pub after: u64,
    /// Project object ids Show selects.
    pub show: Vec<u64>,
    /// How the turn ended, when not with an answer.
    pub end: Option<String>,
    /// The turn's and the conversation's tokens, and their cost where the
    /// model has a price: one line each.
    pub spend: Vec<String>,
    /// The limit the turn stopped at, in words, when it stopped at one:
    /// the card offers Continue.
    pub limit: Option<LimitStop>,
}

/// A turn stopped at a limit.
#[derive(Clone, Debug, PartialEq)]
pub struct LimitStop {
    /// "This turn's limit of 400 000 tokens".
    pub which: String,
    /// The conversation's limit was reached: Continue raises it by a
    /// turn's tokens.
    pub conversation: bool,
    /// The person pressed Continue.
    pub continued: bool,
}

/// The limits the turn running was given, and which bounded its tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnLimits {
    pub limits: Limits,
    pub bound: Bound,
}

/// One item of the transcript.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// The person's message, the window's context when it was sent, and its
    /// attachments' names.
    You {
        text: String,
        context: String,
        attachments: Vec<String>,
    },
    /// Reply text, in bounded Markdown.
    Reply(String),
    /// What the model reasoned, folded.
    Reasoning(String),
    Call(CallRow),
    Ask(AskCard),
    /// The model's questions to the person (`assist.ask`).
    Question(super::questions::QuestionCard),
    Summary(TurnSummary),
    /// A workflow's card or a run's (C2), by its key in the window's
    /// workflow cards (`crate::workflow_cards`).
    Workflow(u64),
    /// The divider where the chat condensed what the model is sent
    /// (`docs/assistant-panel.md` §6.5): its line, and the kept record it
    /// unfolds to. The transcript above is unchanged.
    Condensed {
        line: String,
        record: String,
    },
}

/// The transcript, its state and the use of the turn running.
#[derive(Debug)]
pub struct Conversation {
    pub items: Vec<Item>,
    pub state: State,
    /// Use of the turn running so far.
    pub usage: Use,
    /// The tokens of every turn that ended, as the provider reported them.
    pub spent: Usage,
    /// What those turns cost: `None` once a turn ran on a model with no
    /// price (or another currency), so no partial sum is shown.
    pub cost: Option<(f64, String)>,
    /// Tokens Continue allowed past the conversation's limit.
    pub raised: u64,
    /// The price of the model the turn running uses, if it has one.
    pub price: Option<Price>,
    /// The turn running's limits (none for a test model).
    pub limits: Option<TurnLimits>,
    /// The reply and reasoning items the current request's deltas grow.
    reply_at: Option<usize>,
    reasoning_at: Option<usize>,
    next_id: u64,
}

impl Default for Conversation {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            state: State::Idle,
            usage: Use::default(),
            spent: Usage::default(),
            cost: Some((0.0, String::new())),
            raised: 0,
            price: None,
            limits: None,
            reply_at: None,
            reasoning_at: None,
            next_id: 1,
        }
    }
}

impl Conversation {
    /// A fresh id for a card.
    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn push(&mut self, item: Item) {
        self.items.push(item);
        if self.items.len() > MAX_ITEMS {
            let drop = self.items.len() - MAX_ITEMS;
            self.items.drain(..drop);
            self.reply_at = self.reply_at.and_then(|i| i.checked_sub(drop));
            self.reasoning_at = self.reasoning_at.and_then(|i| i.checked_sub(drop));
        }
    }

    /// A message sent: a turn starts.
    pub fn sent(&mut self, text: String, context: String, attachments: Vec<String>) {
        self.push(Item::You {
            text,
            context,
            attachments,
        });
        self.state = State::Running;
        self.usage = Use::default();
        self.reply_at = None;
        self.reasoning_at = None;
    }

    /// Fold one of the loop's events into the transcript.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Request { .. } => {
                self.reply_at = None;
                self.reasoning_at = None;
            }
            Event::TextDelta(text) => self.grow(text, false),
            Event::ReasoningDelta(text) => self.grow(text, true),
            Event::Text(text) => match self.reply_at.and_then(|i| self.items.get_mut(i)) {
                // The whole text replaces what its deltas showed.
                Some(Item::Reply(shown)) => *shown = text,
                _ => {
                    self.push(Item::Reply(text));
                    self.reply_at = Some(self.items.len() - 1);
                }
            },
            Event::Call {
                request,
                call,
                name,
                state,
            } => self.call(request, call, &name, state),
            Event::Usage(meter) => {
                self.usage = Use {
                    requests: u32::try_from(meter.requests.used).unwrap_or(u32::MAX),
                    output_tokens: meter.output_tokens.used,
                    tokens: meter.tokens.used,
                    seconds: meter.minutes.used as f64 * 60.0,
                    reported: meter.reported,
                };
            }
            Event::Condensed(condensed) => self.push(Item::Condensed {
                line: condensed_line(&condensed),
                record: condensed.record.unwrap_or_else(|| {
                    "Older tool replies the model had read were cleared; it asks again when it needs them.".into()
                }),
            }),
            Event::Response { .. } | Event::Warning(_) => {}
        }
    }

    /// A delta: this reply's item of its kind grows, or starts.
    fn grow(&mut self, text: String, reasoning: bool) {
        let at = if reasoning {
            self.reasoning_at
        } else {
            self.reply_at
        };
        match at.and_then(|i| self.items.get_mut(i)) {
            Some(Item::Reasoning(shown)) if reasoning => shown.push_str(&text),
            Some(Item::Reply(shown)) if !reasoning => shown.push_str(&text),
            _ => {
                self.push(if reasoning {
                    Item::Reasoning(text)
                } else {
                    Item::Reply(text)
                });
                let at = Some(self.items.len() - 1);
                if reasoning {
                    self.reasoning_at = at;
                } else {
                    self.reply_at = at;
                }
            }
        }
    }

    fn call(&mut self, request: u32, call: u32, name: &str, state: CallState) {
        // Rows are named within their turn: since the person's last message.
        let turn = self
            .items
            .iter()
            .rposition(|item| matches!(item, Item::You { .. }))
            .map_or(0, |i| i + 1);
        let index = self.items[turn..]
            .iter()
            .rposition(
                |item| matches!(item, Item::Call(row) if row.request == request && row.call == call),
            )
            .map(|i| i + turn);
        let index = match index {
            Some(index) => index,
            None => {
                self.push(Item::Call(CallRow {
                    request,
                    call,
                    line: name.to_owned(),
                    state: RowState::Running,
                    input: String::new(),
                    reply: String::new(),
                }));
                self.items.len() - 1
            }
        };
        let Some(Item::Call(row)) = self.items.get_mut(index) else {
            return;
        };
        match state {
            CallState::Writing => row.state = RowState::Writing,
            CallState::Running { input } => {
                row.line = call_line(name, &input);
                row.input = detail(&serde_json::to_string_pretty(&input).unwrap_or_default());
                row.state = RowState::Running;
            }
            CallState::Done { reply, .. } => {
                row.reply = detail(&reply);
                row.state = RowState::Done;
            }
            CallState::Refused { reply } => {
                row.reply = detail(&reply);
                row.state = RowState::Refused;
            }
            CallState::NotRun { why } => row.state = RowState::NotRun(why),
        }
    }

    /// The Ask card `id`, if it is in the transcript.
    pub fn ask_mut(&mut self, id: u64) -> Option<&mut AskCard> {
        self.items.iter_mut().find_map(|item| match item {
            Item::Ask(card) if card.id == id => Some(card),
            _ => None,
        })
    }

    /// The question card `card`, if it is in the transcript.
    pub fn question_mut(&mut self, card: &str) -> Option<&mut super::questions::QuestionCard> {
        self.items.iter_mut().rev().find_map(|item| match item {
            Item::Question(question) if question.card == card => Some(question),
            _ => None,
        })
    }

    /// The summary card `id`.
    pub fn summary(&self, id: u64) -> Option<&TurnSummary> {
        self.items.iter().find_map(|item| match item {
            Item::Summary(card) if card.id == id => Some(card),
            _ => None,
        })
    }

    /// Tokens every ended turn used, prompt and output.
    pub fn spent_tokens(&self) -> u64 {
        self.spent.prompt().saturating_add(self.spent.output_tokens)
    }

    /// A turn ended: its card, with what the turn and the conversation
    /// used and cost and the limit it stopped at, and the state its end
    /// leaves.
    pub fn ended(&mut self, mut summary: TurnSummary, end: &chat::End) {
        let turn = summary.usage.reported;
        self.spent.add(&turn);
        self.cost = match (self.cost.take(), &self.price) {
            (Some((sum, currency)), Some(price))
                if currency.is_empty() || currency == price.currency =>
            {
                Some((sum + price.cost(&turn), price.currency.clone()))
            }
            // A turn that used nothing costs nothing on any model.
            (cost, _) if turn == Usage::default() => cost,
            _ => None,
        };
        summary.spend = spend_lines(&turn, self.price.as_ref(), &self.spent, self.cost.as_ref());
        if let chat::End::Limit { hit, .. } = end {
            summary.limit = Some(LimitStop {
                which: limit_words(*hit, self.limits.as_ref()),
                conversation: *hit == Hit::Tokens
                    && self.limits.is_some_and(|l| l.bound == Bound::Conversation),
                continued: false,
            });
        }
        self.state = match end {
            chat::End::Answered => State::Idle,
            chat::End::Stopped => State::Stopped,
            other => State::Error(other.to_string()),
        };
        self.push(Item::Summary(summary));
    }

    /// The id of the last turn's card, which alone offers Continue.
    pub fn last_summary(&self) -> Option<u64> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Summary(card) => Some(card.id),
            _ => None,
        })
    }

    /// The person pressed Continue on card `id`: it is marked, and the
    /// conversation's limit is raised by a turn's tokens when that was the
    /// limit reached. False when the card offers no Continue.
    pub fn continue_from(&mut self, id: u64) -> bool {
        if self.state.busy() || self.last_summary() != Some(id) {
            return false;
        }
        let tokens = self.limits.map(|l| l.limits.total_tokens).unwrap_or(0);
        let Some(Item::Summary(card)) = self
            .items
            .iter_mut()
            .rev()
            .find(|item| matches!(item, Item::Summary(card) if card.id == id))
        else {
            return false;
        };
        let Some(limit) = card.limit.as_mut().filter(|l| !l.continued) else {
            return false;
        };
        limit.continued = true;
        if limit.conversation {
            self.raised = self.raised.saturating_add(tokens);
        }
        true
    }
}

/// Tokens in the card's words: "950", "61 k", "1.9 M".
pub fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{} k", n / 1_000)
    } else {
        n.to_string()
    }
}

/// One line of use: "61 k in (40 k from the cache, 2 k written to it),
/// 2100 out". The cache is named only when the provider reported it.
fn use_words(usage: &Usage) -> String {
    let mut cache = Vec::new();
    if usage.cache_read_tokens > 0 {
        cache.push(format!(
            "{} from the cache",
            tokens(usage.cache_read_tokens)
        ));
    }
    if usage.cache_write_tokens > 0 {
        cache.push(format!(
            "{} written to it",
            tokens(usage.cache_write_tokens)
        ));
    }
    let cache = if cache.is_empty() {
        String::new()
    } else {
        format!(" ({})", cache.join(", "))
    };
    format!(
        "{} in{cache}, {} out",
        tokens(usage.prompt()),
        tokens(usage.output_tokens)
    )
}

/// The card's lines for a turn's and the conversation's use, each with
/// its estimate when the model is priced; with no price, tokens only.
pub fn spend_lines(
    turn: &Usage,
    price: Option<&Price>,
    conversation: &Usage,
    cost: Option<&(f64, String)>,
) -> Vec<String> {
    let turn_line = match price {
        Some(price) => format!("This turn: {} · {}", use_words(turn), price.estimate(turn)),
        None => format!("This turn: {}", use_words(turn)),
    };
    let money = |amount: f64, currency: &str| {
        if amount < 0.005 {
            format!("under 0.01 {currency}")
        } else {
            format!("about {amount:.2} {currency}")
        }
    };
    let conversation_line = match cost {
        Some((amount, currency)) if !currency.is_empty() => format!(
            "This conversation: {} · {}",
            use_words(conversation),
            money(*amount, currency)
        ),
        _ => format!("This conversation: {}", use_words(conversation)),
    };
    vec![turn_line, conversation_line]
}

/// Which limit a turn stopped at, in words.
pub fn limit_words(hit: Hit, limits: Option<&TurnLimits>) -> String {
    let Some(TurnLimits { limits, bound }) = limits else {
        return "A limit of this turn".into();
    };
    let n = |v: u64| group(v);
    match hit {
        Hit::Requests => format!(
            "This turn's limit of {} model requests",
            n(limits.requests.into())
        ),
        Hit::OutputTokens => format!(
            "This turn's limit of {} output tokens",
            n(limits.output_tokens)
        ),
        Hit::Tokens if *bound == Bound::Conversation => format!(
            "This conversation's limit of {} tokens",
            n(limits.conversation_tokens)
        ),
        Hit::Tokens => format!("This turn's limit of {} tokens", n(limits.total_tokens)),
        Hit::Time => format!("This turn's limit of {} minutes", limits.minutes),
        Hit::Idle => "The stop for reading without changing anything".into(),
    }
}

/// 400000 as "400 000".
fn group(v: u64) -> String {
    let digits = v.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// How a row names a call: the verb and the entries it names.
pub fn call_line(verb: &str, input: &Value) -> String {
    let names: Vec<&str> = match input["operations"].as_array() {
        Some(operations) => operations
            .iter()
            .filter_map(|op| op["op"].as_str())
            .collect(),
        None => input["name"]
            .as_str()
            .or_else(|| input["text"].as_str())
            .into_iter()
            .collect(),
    };
    if names.is_empty() {
        verb.to_owned()
    } else {
        format!("{verb} {}", names.join(", "))
    }
}

/// `text` within [`MAX_DETAIL_CHARS`], saying when it was cut.
fn detail(text: &str) -> String {
    if text.len() <= MAX_DETAIL_CHARS {
        return text.to_owned();
    }
    let mut end = MAX_DETAIL_CHARS;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The divider's line: "Condensed 14 earlier tool replies · 182 k → 61 k
/// tokens".
fn condensed_line(condensed: &sparksmith_assist::chat::Condensed) -> String {
    let k = |tokens: u64| {
        if tokens >= 10_000 {
            format!("{} k", tokens / 1_000)
        } else {
            tokens.to_string()
        }
    };
    let replies = match condensed.cleared {
        1 => "1 earlier tool reply".to_owned(),
        n => format!("{n} earlier tool replies"),
    };
    let what = match (condensed.turns, condensed.cleared) {
        (0, _) => replies,
        (1, 0) => "1 earlier turn".to_owned(),
        (t, 0) => format!("{t} earlier turns"),
        (1, _) => format!("1 earlier turn and {replies}"),
        (t, _) => format!("{t} earlier turns and {replies}"),
    };
    format!(
        "Condensed {what} · {} → {} tokens",
        k(condensed.before_tokens),
        k(condensed.after_tokens)
    )
}

/// The context line of a message: what `view.context` answered, in the
/// person's words ("PCB · Layout · U1, R3").
pub fn context_line(context: &Value) -> String {
    let mut parts = Vec::new();
    match context["workspace"].as_str() {
        Some("pcb") => {
            parts.push("PCB".to_owned());
            match context["view"].as_str() {
                Some("3d") => parts.push("3D".to_owned()),
                Some(_) => parts.push("Layout".to_owned()),
                None => {}
            }
        }
        Some("schematic") => {
            parts.push("Schematic".to_owned());
            if let Some(sheet) = context["sheet"]["name"].as_str() {
                parts.push(sheet.to_owned());
            }
        }
        Some("component") => parts.push("Component".to_owned()),
        _ => {}
    }
    const SHOWN: usize = 4;
    let selection: Vec<String> = context["selection"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| match s["name"].as_str() {
            Some(name) => name.to_owned(),
            None => format!(
                "{} {}",
                s["kind"].as_str().unwrap_or("object"),
                s["id"]
                    .as_u64()
                    .map(|id| id.to_string())
                    .unwrap_or_default()
            )
            .trim()
            .to_owned(),
        })
        .collect();
    parts.push(match selection.len() {
        0 => "nothing selected".to_owned(),
        n if n <= SHOWN => selection.join(", "),
        n => format!("{}, +{} more", selection[..SHOWN].join(", "), n - SHOWN),
    });
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deltas_grow_one_item_and_the_whole_text_replaces_them() {
        let mut c = Conversation::default();
        c.sent("Hi".into(), "PCB".into(), vec![]);
        c.apply(Event::Request {
            n: 1,
            prompt_tokens: 0,
        });
        c.apply(Event::ReasoningDelta("Let me ".into()));
        c.apply(Event::ReasoningDelta("think".into()));
        c.apply(Event::TextDelta("Two ".into()));
        c.apply(Event::TextDelta("vias".into()));
        // A call the model starts writing does not split the reply.
        c.apply(Event::Call {
            request: 1,
            call: 0,
            name: "query".into(),
            state: CallState::Writing,
        });
        c.apply(Event::Text("Two vias.".into()));
        assert_eq!(c.items.len(), 4, "{:?}", c.items);
        assert_eq!(
            c.items[1..3],
            [
                Item::Reasoning("Let me think".into()),
                Item::Reply("Two vias.".into())
            ]
        );
        assert_eq!(c.state, State::Running);
    }

    #[test]
    fn a_call_row_moves_through_its_states_in_place() -> Result<(), String> {
        let mut c = Conversation::default();
        let input = json!({"expected_revision": 3, "operations": [{"op": "layout.add_via"}, {"op": "layout.add_via"}]});
        c.apply(Event::Call {
            request: 1,
            call: 0,
            name: "apply".into(),
            state: CallState::Running { input },
        });
        c.apply(Event::Call {
            request: 1,
            call: 0,
            name: "apply".into(),
            state: CallState::Refused {
                reply: "stale".into(),
            },
        });
        assert_eq!(c.items.len(), 1);
        let Item::Call(row) = &c.items[0] else {
            return Err(format!("not a call row: {:?}", c.items[0]));
        };
        assert_eq!(row.line, "apply layout.add_via, layout.add_via");
        assert_eq!(row.state, RowState::Refused);
        assert_eq!(row.reply, "stale");
        assert!(row.input.contains("expected_revision"));
        Ok(())
    }

    fn price() -> Price {
        Price {
            provider: sparksmith_assist::settings::Wire::Anthropic,
            base_url: "https://api.example".into(),
            model: "m".into(),
            currency: "USD".into(),
            per_million: sparksmith_assist::price::Rates {
                input: 10.0,
                cache_write: 12.5,
                cache_read: 1.0,
                output: 20.0,
            },
            source: "https://example/pricing".into(),
            as_of: "2026-11-03".into(),
        }
    }

    fn card(id: u64, reported: Usage) -> TurnSummary {
        TurnSummary {
            id,
            label: "AI assistant: x".into(),
            changes: vec![],
            others: vec![],
            issues_new: 0,
            issues_resolved: 0,
            new_issues: vec![],
            usage: Use {
                reported,
                ..Use::default()
            },
            before: 1,
            after: 1,
            show: vec![],
            end: None,
            spend: vec![],
            limit: None,
        }
    }

    fn limits() -> TurnLimits {
        TurnLimits {
            limits: Limits {
                requests: 20,
                output_tokens: 100_000,
                total_tokens: 400_000,
                minutes: 45,
                conversation_tokens: 2_000_000,
            },
            bound: Bound::Turn,
        }
    }

    /// Each card counts the turn's tokens in, cached and out and the
    /// conversation's, with an estimate on a priced model.
    #[test]
    fn a_card_shows_the_turns_and_the_conversations_tokens_and_cost() -> Result<(), String> {
        let mut c = Conversation {
            price: Some(price()),
            ..Conversation::default()
        };
        let first = Usage {
            input_tokens: 20_000,
            cache_write_tokens: 40_000,
            cache_read_tokens: 0,
            output_tokens: 1_000,
        };
        c.ended(card(1, first), &chat::End::Answered);
        let second = Usage {
            input_tokens: 2_000,
            cache_write_tokens: 0,
            cache_read_tokens: 60_000,
            output_tokens: 2_000,
        };
        c.ended(card(2, second), &chat::End::Answered);
        let Some(Item::Summary(last)) = c.items.last() else {
            return Err(format!("{:?}", c.items));
        };
        // 2 000 × 10 + 60 000 × 1 + 2 000 × 20 per million: 0.12 USD; with
        // the first turn's 0.20 + 0.50 + 0.02, 0.84 USD.
        assert_eq!(
            last.spend,
            [
                "This turn: 62 k in (60 k from the cache), 2000 out · about 0.12 USD",
                "This conversation: 122 k in (60 k from the cache, 40 k written to it), 3000 out · about 0.84 USD",
            ]
        );
        assert_eq!(c.spent_tokens(), 125_000);
        Ok(())
    }

    /// No price: tokens only, never a guessed cost; and once a turn ran
    /// unpriced the conversation shows no partial sum.
    #[test]
    fn without_a_price_a_card_shows_tokens_only() -> Result<(), String> {
        let mut c = Conversation {
            price: Some(price()),
            ..Conversation::default()
        };
        let used = Usage {
            input_tokens: 5_000,
            output_tokens: 100,
            ..Usage::default()
        };
        c.ended(card(1, used), &chat::End::Answered);
        c.price = None;
        c.ended(card(2, used), &chat::End::Answered);
        let Some(Item::Summary(last)) = c.items.last() else {
            return Err(format!("{:?}", c.items));
        };
        assert_eq!(
            last.spend,
            [
                "This turn: 5000 in, 100 out",
                "This conversation: 10 k in, 200 out"
            ]
        );
        assert!(last.spend.iter().all(|l| !l.contains("USD")));
        Ok(())
    }

    /// A limit stop names the limit; Continue is offered once, on the last
    /// card, and past the conversation's limit raises it by a turn's tokens.
    #[test]
    fn a_limit_stop_names_its_limit_and_continue_raises_the_conversations() {
        let mut c = Conversation {
            limits: Some(limits()),
            ..Conversation::default()
        };
        let stop = |hit| chat::End::Limit {
            hit,
            why: "numbers".into(),
        };
        c.ended(card(1, Usage::default()), &stop(Hit::Requests));
        let which = |c: &Conversation, id| {
            c.summary(id)
                .and_then(|card| card.limit.as_ref())
                .map(|l| l.which.clone())
        };
        assert_eq!(
            which(&c, 1).as_deref(),
            Some("This turn's limit of 20 model requests")
        );
        assert!(!c.continue_from(7), "no such card");
        assert!(c.continue_from(1));
        assert!(!c.continue_from(1), "Continue is offered once");
        assert_eq!(c.raised, 0, "a turn's own limit raises nothing kept");
        c.limits = Some(TurnLimits {
            bound: Bound::Conversation,
            ..limits()
        });
        c.ended(card(2, Usage::default()), &stop(Hit::Tokens));
        assert_eq!(
            which(&c, 2).as_deref(),
            Some("This conversation's limit of 2 000 000 tokens")
        );
        c.ended(card(3, Usage::default()), &chat::End::Answered);
        assert!(!c.continue_from(2), "only the last card continues");
        c.ended(card(4, Usage::default()), &stop(Hit::Tokens));
        assert!(c.continue_from(4));
        assert_eq!(c.raised, 400_000);
        assert_eq!(
            limit_words(Hit::Idle, Some(&limits())),
            "The stop for reading without changing anything"
        );
        assert_eq!(
            limit_words(Hit::Tokens, Some(&limits())),
            "This turn's limit of 400 000 tokens"
        );
    }

    #[test]
    fn the_context_line_names_what_the_window_shows() {
        let pcb = json!({"revision": 4, "workspace": "pcb", "view": "layout",
            "selection": [{"kind": "instance", "id": 7, "name": "U1"}, {"kind": "track", "id": 9}]});
        assert_eq!(context_line(&pcb), "PCB · Layout · U1, track 9");
        let sheet =
            json!({"workspace": "schematic", "sheet": {"id": 1, "name": "Power"}, "selection": []});
        assert_eq!(context_line(&sheet), "Schematic · Power · nothing selected");
        let many: Vec<Value> = (0..6)
            .map(|i| json!({"kind": "instance", "id": i, "name": format!("R{i}")}))
            .collect();
        let crowded = json!({"workspace": "component", "selection": many});
        assert_eq!(
            context_line(&crowded),
            "Component · R0, R1, R2, R3, +2 more"
        );
    }
}
